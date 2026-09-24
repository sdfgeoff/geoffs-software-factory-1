use std::path::{Path, PathBuf};

use sqlx::{Sqlite, Transaction};

use crate::{domain::Attachment, error::AppError};

use super::Store;

/// Upload limits are deliberately enforced again when attachments are linked.
pub const MAX_ATTACHMENTS_PER_CONTENT: usize = 5;
pub const MAX_ATTACHMENT_BYTES: usize = 10 * 1024 * 1024;
pub const MAX_ATTACHMENTS_TOTAL_BYTES: i64 = 25 * 1024 * 1024;
pub const ORPHAN_MAX_AGE_SECONDS: i64 = 24 * 60 * 60;
const CLEANUP_CLAIM_MAX_AGE_SECONDS: i64 = 60 * 60;
const CLEANUP_BATCH_SIZE: usize = 100;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AttachmentOwner {
    Human,
    Agent(i64),
}

impl AttachmentOwner {
    fn agent_id(self) -> Option<i64> {
        match self {
            Self::Human => None,
            Self::Agent(id) => Some(id),
        }
    }

    fn is_human(self) -> bool {
        matches!(self, Self::Human)
    }
}

pub struct AttachmentUpload {
    pub filename: String,
    pub declared_media_type: String,
    pub bytes: Vec<u8>,
}

pub struct AttachmentDownload {
    pub attachment: Attachment,
    pub path: PathBuf,
}

#[derive(Debug)]
struct ClaimedAttachment {
    id: String,
    storage_name: String,
    claimed_at: i64,
}

impl Store {
    pub async fn upload_attachment(
        &self,
        owner: AttachmentOwner,
        upload: AttachmentUpload,
    ) -> Result<Attachment, AppError> {
        if upload.bytes.is_empty() || upload.bytes.len() > MAX_ATTACHMENT_BYTES {
            return Err(AppError::bad_request(format!(
                "image must contain between 1 and {MAX_ATTACHMENT_BYTES} bytes"
            )));
        }
        let original_filename = safe_filename(&upload.filename)?;
        let (media_type, extension) = inspect_image(
            &upload.bytes,
            &upload.declared_media_type,
            &original_filename,
        )?;
        let id = uuid::Uuid::new_v4().simple().to_string();
        let storage_name = format!("{id}.{extension}");
        let final_path = self.attachment_root.join(&storage_name);
        let temporary_path = self.attachment_root.join(format!(".{id}.upload"));
        tokio::fs::write(&temporary_path, &upload.bytes)
            .await
            .map_err(|error| AppError::internal(format!("failed to store attachment: {error}")))?;
        tokio::fs::rename(&temporary_path, &final_path)
            .await
            .map_err(|error| AppError::internal(format!("failed to commit attachment: {error}")))?;

        let result = sqlx::query(
            "INSERT INTO attachments (id, owner_agent_id, owner_is_human, original_filename, media_type, byte_size, storage_name) \
             VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(owner.agent_id())
        .bind(owner.is_human())
        .bind(&original_filename)
        .bind(media_type)
        .bind(upload.bytes.len() as i64)
        .bind(&storage_name)
        .execute(&self.pool)
        .await;
        if let Err(error) = result {
            let _ = tokio::fs::remove_file(&final_path).await;
            return Err(error.into());
        }
        Ok(Attachment {
            id,
            original_filename,
            media_type: media_type.to_owned(),
            byte_size: upload.bytes.len() as i64,
        })
    }

    pub async fn attachment_download(
        &self,
        id: &str,
    ) -> Result<Option<AttachmentDownload>, AppError> {
        if !valid_id(id) {
            return Ok(None);
        }
        let row: Option<(String, String, String, i64, String)> = sqlx::query_as(
            "SELECT id, original_filename, media_type, byte_size, storage_name FROM attachments \
             WHERE id = ? AND content_kind IS NOT NULL",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(
            |(id, original_filename, media_type, byte_size, storage_name)| AttachmentDownload {
                attachment: Attachment {
                    id,
                    original_filename,
                    media_type,
                    byte_size,
                },
                path: self.attachment_root.join(storage_name),
            },
        ))
    }

    pub async fn cleanup_orphaned_attachments(&self, now: i64) -> Result<usize, AppError> {
        let rows = self.claim_orphaned_attachment_batch(now).await?;
        self.remove_claimed_attachment_files(rows).await
    }

    pub async fn cleanup_expired_attachment_batches(&self, now: i64) -> Result<usize, AppError> {
        let mut total = 0;
        loop {
            let removed = self.cleanup_orphaned_attachments(now).await?;
            total += removed;
            if removed < CLEANUP_BATCH_SIZE {
                return Ok(total);
            }
        }
    }

    pub async fn run_attachment_cleanup(self, interval: std::time::Duration) {
        loop {
            if let Err(error) = self
                .cleanup_expired_attachment_batches(jiff::Timestamp::now().as_second())
                .await
            {
                eprintln!("Attachment cleanup failed: {error}");
            }
            tokio::time::sleep(interval).await;
        }
    }

    async fn claim_orphaned_attachment_batch(
        &self,
        now: i64,
    ) -> Result<Vec<ClaimedAttachment>, AppError> {
        let cutoff = now - ORPHAN_MAX_AGE_SECONDS;
        let abandoned_claim_cutoff = now - CLEANUP_CLAIM_MAX_AGE_SECONDS;
        let mut transaction = self.pool.begin().await?;
        let rows: Vec<(String, String)> = sqlx::query_as(
            "UPDATE attachments SET cleanup_claimed_at = ? WHERE id IN (\
                 SELECT id FROM attachments WHERE content_kind IS NULL AND created_at < ? \
                 AND (cleanup_claimed_at IS NULL OR cleanup_claimed_at < ?) \
                 ORDER BY created_at, id LIMIT ?\
             ) AND content_kind IS NULL \
             AND (cleanup_claimed_at IS NULL OR cleanup_claimed_at < ?) \
             RETURNING id, storage_name",
        )
        .bind(now)
        .bind(cutoff)
        .bind(abandoned_claim_cutoff)
        .bind(CLEANUP_BATCH_SIZE as i64)
        .bind(abandoned_claim_cutoff)
        .fetch_all(&mut *transaction)
        .await?;
        transaction.commit().await?;
        Ok(rows
            .into_iter()
            .map(|(id, storage_name)| ClaimedAttachment {
                id,
                storage_name,
                claimed_at: now,
            })
            .collect())
    }

    async fn remove_claimed_attachment_files(
        &self,
        rows: Vec<ClaimedAttachment>,
    ) -> Result<usize, AppError> {
        let mut removed = 0;
        for row in rows {
            let path = self.attachment_root.join(row.storage_name);
            match tokio::fs::remove_file(path).await {
                Ok(()) => {
                    removed += sqlx::query(
                        "DELETE FROM attachments WHERE id = ? AND content_kind IS NULL \
                         AND cleanup_claimed_at = ?",
                    )
                    .bind(&row.id)
                    .bind(row.claimed_at)
                    .execute(&self.pool)
                    .await?
                    .rows_affected() as usize;
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    removed += sqlx::query(
                        "DELETE FROM attachments WHERE id = ? AND content_kind IS NULL \
                         AND cleanup_claimed_at = ?",
                    )
                    .bind(&row.id)
                    .bind(row.claimed_at)
                    .execute(&self.pool)
                    .await?
                    .rows_affected() as usize;
                }
                Err(_) => {
                    sqlx::query(
                        "UPDATE attachments SET cleanup_claimed_at = NULL WHERE id = ? \
                         AND content_kind IS NULL AND cleanup_claimed_at = ?",
                    )
                    .bind(&row.id)
                    .bind(row.claimed_at)
                    .execute(&self.pool)
                    .await?;
                }
            }
        }
        Ok(removed)
    }
}

pub(super) async fn link_attachments(
    transaction: &mut Transaction<'_, Sqlite>,
    ids: &[String],
    owner: AttachmentOwner,
    content_kind: &str,
    content_id: i64,
) -> Result<Vec<Attachment>, AppError> {
    if ids.len() > MAX_ATTACHMENTS_PER_CONTENT {
        return Err(AppError::bad_request(format!(
            "at most {MAX_ATTACHMENTS_PER_CONTENT} images may be attached"
        )));
    }
    let mut unique = ids.to_vec();
    unique.sort();
    unique.dedup();
    if unique.len() != ids.len() {
        return Err(AppError::bad_request("attachment IDs must be unique"));
    }
    let mut attachments = Vec::with_capacity(ids.len());
    let mut total = 0;
    for id in ids {
        if !valid_id(id) {
            return Err(AppError::bad_request("invalid attachment ID"));
        }
        let attachment: Attachment = sqlx::query_as(
            "SELECT id, original_filename, media_type, byte_size FROM attachments \
             WHERE id = ? AND content_kind IS NULL AND owner_is_human = ? \
             AND cleanup_claimed_at IS NULL \
             AND ((? IS NULL AND owner_agent_id IS NULL) OR owner_agent_id = ?)",
        )
        .bind(id)
        .bind(owner.is_human())
        .bind(owner.agent_id())
        .bind(owner.agent_id())
        .fetch_optional(&mut **transaction)
        .await?
        .ok_or_else(|| {
            AppError::forbidden("attachment is unavailable or belongs to another caller")
        })?;
        total += attachment.byte_size;
        if total > MAX_ATTACHMENTS_TOTAL_BYTES {
            return Err(AppError::bad_request(format!(
                "attached images may total at most {MAX_ATTACHMENTS_TOTAL_BYTES} bytes"
            )));
        }
        let updated = sqlx::query(
            "UPDATE attachments SET content_kind = ?, content_id = ?, linked_at = unixepoch() \
             WHERE id = ? AND content_kind IS NULL AND cleanup_claimed_at IS NULL",
        )
        .bind(content_kind)
        .bind(content_id)
        .bind(id)
        .execute(&mut **transaction)
        .await?;
        if updated.rows_affected() != 1 {
            return Err(AppError::conflict("attachment was already linked"));
        }
        attachments.push(attachment);
    }
    Ok(attachments)
}

pub(super) async fn attachments_for(
    pool: &sqlx::SqlitePool,
    content_kind: &str,
    content_id: i64,
) -> Result<Vec<Attachment>, sqlx::Error> {
    sqlx::query_as(
        "SELECT id, original_filename, media_type, byte_size FROM attachments \
         WHERE content_kind = ? AND content_id = ? ORDER BY linked_at, id",
    )
    .bind(content_kind)
    .bind(content_id)
    .fetch_all(pool)
    .await
}

fn valid_id(id: &str) -> bool {
    id.len() == 32 && id.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn safe_filename(input: &str) -> Result<String, AppError> {
    let normalized = input.replace('\\', "/");
    let name = Path::new(&normalized)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("")
        .chars()
        .filter(|character| !character.is_control())
        .take(255)
        .collect::<String>();
    if name.is_empty() || name == "." || name == ".." {
        return Err(AppError::bad_request("image filename is invalid"));
    }
    Ok(name)
}

fn inspect_image(
    bytes: &[u8],
    declared: &str,
    filename: &str,
) -> Result<(&'static str, &'static str), AppError> {
    let detected = if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        validate_raster(bytes, image::ImageFormat::Png)?;
        ("image/png", "png")
    } else if bytes.starts_with(b"\xff\xd8\xff") && bytes.ends_with(b"\xff\xd9") {
        validate_raster(bytes, image::ImageFormat::Jpeg)?;
        ("image/jpeg", "jpg")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        validate_raster(bytes, image::ImageFormat::Gif)?;
        ("image/gif", "gif")
    } else if bytes.len() >= 16 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        validate_raster(bytes, image::ImageFormat::WebP)?;
        ("image/webp", "webp")
    } else if let Ok(text) = std::str::from_utf8(bytes) {
        validate_svg(text)?;
        ("image/svg+xml", "svg")
    } else {
        return Err(AppError::bad_request("file is not a supported image"));
    };
    let declared = declared
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    if declared != detected.0 {
        return Err(AppError::bad_request(
            "declared image media type does not match its contents",
        ));
    }
    let extension = filename
        .rsplit('.')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    let extension_matches = match detected.0 {
        "image/jpeg" => matches!(extension.as_str(), "jpg" | "jpeg"),
        _ => extension == detected.1,
    };
    if !extension_matches {
        return Err(AppError::bad_request(
            "image filename extension does not match its contents",
        ));
    }
    Ok(detected)
}

fn validate_raster(bytes: &[u8], format: image::ImageFormat) -> Result<(), AppError> {
    let mut reader = image::ImageReader::with_format(std::io::Cursor::new(bytes), format);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(16_384);
    limits.max_image_height = Some(16_384);
    limits.max_alloc = Some(128 * 1024 * 1024);
    reader.limits(limits);
    reader
        .decode()
        .map(|_| ())
        .map_err(|_| AppError::bad_request("image data is malformed"))
}

fn validate_svg(svg: &str) -> Result<(), AppError> {
    let lower = svg.to_ascii_lowercase();
    if lower.contains("<!doctype") || lower.contains("<!entity") {
        return Err(AppError::bad_request(
            "SVG contains executable or external content",
        ));
    }
    let document = roxmltree::Document::parse(svg)
        .map_err(|_| AppError::bad_request("file is not a valid SVG image"))?;
    if document.root_element().tag_name().name() != "svg" {
        return Err(AppError::bad_request("file is not a valid SVG image"));
    }
    for node in document.descendants().filter(|node| node.is_element()) {
        let name = node.tag_name().name().to_ascii_lowercase();
        if matches!(
            name.as_str(),
            "script" | "style" | "foreignobject" | "iframe" | "object" | "embed" | "use"
        ) {
            return Err(AppError::bad_request(
                "SVG contains executable or external content",
            ));
        }
        for attribute in node.attributes() {
            let name = attribute.name().to_ascii_lowercase();
            let value = attribute.value().trim().to_ascii_lowercase();
            let compact_value = value
                .chars()
                .filter(|character| !character.is_ascii_whitespace())
                .collect::<String>();
            let external_reference = matches!(name.as_str(), "href" | "xlink:href" | "src")
                && !value.is_empty()
                && !value.starts_with('#');
            if name.starts_with("on")
                || name == "style"
                || external_reference
                || value.contains("javascript:")
                || value.contains("@import")
                || has_unsafe_url_reference(&compact_value)
            {
                return Err(AppError::bad_request(
                    "SVG contains executable or external content",
                ));
            }
        }
    }
    Ok(())
}

fn has_unsafe_url_reference(value: &str) -> bool {
    let mut rest = value;
    while let Some(start) = rest.find("url(") {
        let after = &rest[start + 4..];
        let Some(end) = after.find(')') else {
            return true;
        };
        let target = after[..end].trim_matches(['\'', '"']);
        if !target.starts_with('#') || target.len() == 1 {
            return true;
        }
        rest = &after[end + 1..];
    }
    false
}

#[cfg(test)]
pub(crate) fn test_png_bytes() -> Vec<u8> {
    let image = image::RgbaImage::from_pixel(1, 1, image::Rgba([0, 0, 0, 0]));
    let mut cursor = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(image)
        .write_to(&mut cursor, image::ImageFormat::Png)
        .unwrap();
    cursor.into_inner()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(filename: &str) -> AttachmentUpload {
        AttachmentUpload {
            filename: filename.into(),
            declared_media_type: "image/png".into(),
            bytes: test_png_bytes(),
        }
    }

    #[test]
    fn traversal_names_are_reduced_to_a_display_basename() {
        assert_eq!(
            safe_filename("../../private/photo.png").unwrap(),
            "photo.png"
        );
        assert_eq!(
            safe_filename("..\\private\\photo.png").unwrap(),
            "photo.png"
        );
    }

    #[test]
    fn malicious_or_external_svg_is_rejected() {
        assert!(validate_svg("<svg><script>alert(1)</script></svg>").is_err());
        assert!(validate_svg("<svg><image href=\"https://example.test/x.png\"/></svg>").is_err());
        assert!(validate_svg("<svg><circle cx=\"2\" cy=\"2\" r=\"2\"/></svg>").is_ok());
        assert!(validate_svg("<svg xmlns=\"http://www.w3.org/2000/svg\"><circle/></svg>").is_ok());
        assert!(
            validate_svg("<svg><style>@import url(https://example.test/x.css);</style></svg>")
                .is_err()
        );
        assert!(
            validate_svg("<svg><circle style=\"fill:url( https://example.test/x.svg)\"/></svg>")
                .is_err()
        );
        assert!(validate_svg("<svg><circle fill=\"url( #safe-gradient )\"/></svg>").is_ok());
    }

    #[tokio::test]
    async fn upload_links_once_to_its_owner_and_allows_empty_content() {
        let store = Store::memory().await.unwrap();
        let uploaded = store
            .upload_attachment(AttachmentOwner::Human, png("../../secret/photo.png"))
            .await
            .unwrap();
        assert_eq!(uploaded.original_filename, "photo.png");
        let (message, _) = store
            .post_message_with_attachments(
                "general",
                None,
                "human",
                "",
                std::slice::from_ref(&uploaded.id),
            )
            .await
            .unwrap();
        assert_eq!(message.attachments, vec![uploaded.clone()]);
        assert!(
            store
                .attachment_download(&uploaded.id)
                .await
                .unwrap()
                .is_some()
        );

        let error = store
            .post_message_with_attachments(
                "general",
                None,
                "human",
                "reuse",
                std::slice::from_ref(&uploaded.id),
            )
            .await
            .unwrap_err();
        assert!(error.to_string().contains("unavailable"));
    }

    #[tokio::test]
    async fn another_agent_cannot_link_a_pending_upload() {
        let store = Store::memory().await.unwrap();
        let first = store.create_agent("first", "/tmp/first").await.unwrap();
        let second = store.create_agent("second", "/tmp/second").await.unwrap();
        let uploaded = store
            .upload_attachment(AttachmentOwner::Agent(first.id), png("photo.png"))
            .await
            .unwrap();
        let error = store
            .post_message_with_attachments(
                "general",
                Some(second.id),
                "second",
                "not mine",
                &[uploaded.id],
            )
            .await
            .unwrap_err();
        assert!(error.to_string().contains("another caller"));
    }

    #[tokio::test]
    async fn linked_file_and_metadata_survive_reopen() {
        let path = std::env::temp_dir().join(format!(
            "multi-agent-board-attachment-{}.sqlite3",
            uuid::Uuid::new_v4()
        ));
        let store = Store::open(&path).await.unwrap();
        let uploaded = store
            .upload_attachment(AttachmentOwner::Human, png("photo.png"))
            .await
            .unwrap();
        store
            .post_message_with_attachments(
                "general",
                None,
                "human",
                "persistent",
                std::slice::from_ref(&uploaded.id),
            )
            .await
            .unwrap();
        store.close().await;

        let reopened = Store::open(&path).await.unwrap();
        let download = reopened
            .attachment_download(&uploaded.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            tokio::fs::read(download.path).await.unwrap(),
            png("x.png").bytes
        );
    }

    #[tokio::test]
    async fn stale_orphans_are_cleaned_in_bounded_batches() {
        let store = Store::memory().await.unwrap();
        let uploaded = store
            .upload_attachment(AttachmentOwner::Human, png("old.png"))
            .await
            .unwrap();
        sqlx::query("UPDATE attachments SET created_at = 1 WHERE id = ?")
            .bind(&uploaded.id)
            .execute(&store.pool)
            .await
            .unwrap();
        assert_eq!(
            store
                .cleanup_orphaned_attachments(ORPHAN_MAX_AGE_SECONDS + 2)
                .await
                .unwrap(),
            1
        );
        let exists: bool =
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM attachments WHERE id = ?)")
                .bind(uploaded.id)
                .fetch_one(&store.pool)
                .await
                .unwrap();
        assert!(!exists);
    }

    #[tokio::test]
    async fn cleanup_claim_and_link_are_mutually_exclusive() {
        let store = Store::memory().await.unwrap();
        let uploaded = store
            .upload_attachment(AttachmentOwner::Human, png("racing.png"))
            .await
            .unwrap();
        sqlx::query("UPDATE attachments SET created_at = 1 WHERE id = ?")
            .bind(&uploaded.id)
            .execute(&store.pool)
            .await
            .unwrap();
        let claimed = store
            .claim_orphaned_attachment_batch(ORPHAN_MAX_AGE_SECONDS + 2)
            .await
            .unwrap();
        assert_eq!(claimed.len(), 1);
        let path = store.attachment_root.join(&claimed[0].storage_name);
        assert!(path.exists());

        let error = store
            .post_message_with_attachments(
                "general",
                None,
                "human",
                "link lost race",
                std::slice::from_ref(&uploaded.id),
            )
            .await
            .unwrap_err();
        assert!(error.to_string().contains("unavailable"));
        assert_eq!(
            store
                .remove_claimed_attachment_files(claimed)
                .await
                .unwrap(),
            1
        );
        assert!(!path.exists());
        let linked: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM attachments WHERE id = ? AND content_kind IS NOT NULL)",
        )
        .bind(&uploaded.id)
        .fetch_one(&store.pool)
        .await
        .unwrap();
        assert!(!linked);

        let linked_first = store
            .upload_attachment(AttachmentOwner::Human, png("linked-first.png"))
            .await
            .unwrap();
        sqlx::query("UPDATE attachments SET created_at = 1 WHERE id = ?")
            .bind(&linked_first.id)
            .execute(&store.pool)
            .await
            .unwrap();
        store
            .post_message_with_attachments(
                "general",
                None,
                "human",
                "link won",
                std::slice::from_ref(&linked_first.id),
            )
            .await
            .unwrap();
        assert_eq!(
            store
                .cleanup_orphaned_attachments(ORPHAN_MAX_AGE_SECONDS + 2)
                .await
                .unwrap(),
            0
        );
        let download = store
            .attachment_download(&linked_first.id)
            .await
            .unwrap()
            .unwrap();
        assert!(download.path.exists());
    }

    #[tokio::test]
    async fn cleanup_runs_after_restart_without_another_upload() {
        let path = std::env::temp_dir().join(format!(
            "multi-agent-board-idle-cleanup-{}.sqlite3",
            uuid::Uuid::new_v4()
        ));
        let store = Store::open(&path).await.unwrap();
        let uploaded = store
            .upload_attachment(AttachmentOwner::Human, png("abandoned.png"))
            .await
            .unwrap();
        let storage_name: String =
            sqlx::query_scalar("SELECT storage_name FROM attachments WHERE id = ?")
                .bind(&uploaded.id)
                .fetch_one(&store.pool)
                .await
                .unwrap();
        let stored_path = store.attachment_root.join(storage_name);
        sqlx::query("UPDATE attachments SET created_at = 1 WHERE id = ?")
            .bind(&uploaded.id)
            .execute(&store.pool)
            .await
            .unwrap();
        store.close().await;

        let reopened = Store::open(&path).await.unwrap();
        let cleanup = tokio::spawn(
            reopened
                .clone()
                .run_attachment_cleanup(std::time::Duration::from_millis(10)),
        );
        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            loop {
                let exists: bool =
                    sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM attachments WHERE id = ?)")
                        .bind(&uploaded.id)
                        .fetch_one(&reopened.pool)
                        .await
                        .unwrap();
                if !exists {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        cleanup.abort();
        assert!(!stored_path.exists());
    }

    #[tokio::test]
    async fn declared_type_extension_and_limits_are_enforced() {
        let store = Store::memory().await.unwrap();
        let mut mismatch = png("photo.jpg");
        mismatch.declared_media_type = "image/jpeg".into();
        assert!(
            store
                .upload_attachment(AttachmentOwner::Human, mismatch)
                .await
                .is_err()
        );
        let ids = (0..=MAX_ATTACHMENTS_PER_CONTENT)
            .map(|_| "a".repeat(32))
            .collect::<Vec<_>>();
        let error = store
            .post_message_with_attachments("general", None, "human", "x", &ids)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("at most"));
    }

    #[tokio::test]
    async fn agent_messages_and_ticket_comments_return_linked_metadata() {
        let store = Store::memory().await.unwrap();
        let agent = store.create_agent("worker", "/tmp/worker").await.unwrap();
        let agent_image = store
            .upload_attachment(AttachmentOwner::Human, png("agent.png"))
            .await
            .unwrap();
        let message = store
            .enqueue_agent_message_with_attachments(
                agent.id,
                "",
                std::slice::from_ref(&agent_image.id),
                AttachmentOwner::Human,
            )
            .await
            .unwrap();
        assert_eq!(message.attachments, vec![agent_image.clone()]);
        let (messages, _) = store
            .agent_messages(agent.id, None, None, None)
            .await
            .unwrap();
        assert_eq!(messages[0].attachments, vec![agent_image]);

        let ticket = store
            .create_ticket(crate::store::NewTicket {
                title: "Attachment activity".into(),
                description: String::new(),
                blocker_ids: Vec::new(),
                created_by_agent_id: None,
            })
            .await
            .unwrap();
        let comment_image = store
            .upload_attachment(AttachmentOwner::Human, png("comment.png"))
            .await
            .unwrap();
        let (comment, _) = store
            .add_ticket_comment_with_attachments(
                ticket.id,
                None,
                "",
                std::slice::from_ref(&comment_image.id),
            )
            .await
            .unwrap();
        assert_eq!(comment.attachments, vec![comment_image.clone()]);
        let (activity, _) = store
            .ticket_activity_page(ticket.id, None, None)
            .await
            .unwrap();
        assert_eq!(activity.last().unwrap().attachments, vec![comment_image]);
    }
}
