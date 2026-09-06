use crate::{error::AppError, store::validate_channel};

use super::Store;

impl Store {
    pub async fn saved_merge_notification_channel(
        &self,
    ) -> Result<Option<Option<String>>, sqlx::Error> {
        sqlx::query_scalar("SELECT merge_notification_channel FROM board_settings WHERE id = 1")
            .fetch_optional(&self.pool)
            .await
    }

    pub async fn set_merge_notification_channel(
        &self,
        channel: Option<&str>,
    ) -> Result<Option<String>, AppError> {
        if let Some(channel) = channel {
            validate_channel(channel)?;
        }
        sqlx::query(
            "INSERT INTO board_settings (id, merge_notification_channel) VALUES (1, ?) \
             ON CONFLICT(id) DO UPDATE SET merge_notification_channel = excluded.merge_notification_channel, \
             updated_at = unixepoch()",
        )
        .bind(channel)
        .execute(&self.pool)
        .await?;
        Ok(channel.map(str::to_owned))
    }
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    #[tokio::test]
    async fn merge_notification_setting_survives_restart_and_preserves_explicit_disable() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("mab-settings-{unique}.sqlite3"));
        let store = Store::open(&path).await.unwrap();
        assert_eq!(
            store.saved_merge_notification_channel().await.unwrap(),
            None
        );
        store
            .set_merge_notification_channel(Some("release-notes"))
            .await
            .unwrap();
        store.close().await;

        let store = Store::open(&path).await.unwrap();
        assert_eq!(
            store.saved_merge_notification_channel().await.unwrap(),
            Some(Some("release-notes".into()))
        );
        store.set_merge_notification_channel(None).await.unwrap();
        store.close().await;

        let store = Store::open(&path).await.unwrap();
        assert_eq!(
            store.saved_merge_notification_channel().await.unwrap(),
            Some(None)
        );
        store.close().await;
        std::fs::remove_file(path).unwrap();
    }
}
