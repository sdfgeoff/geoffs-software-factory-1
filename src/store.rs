use std::path::Path;

use sqlx::{
    SqlitePool,
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
};

use crate::{
    domain::{Channel, Message, Project, Ticket, TicketState},
    error::AppError,
};

#[derive(Clone)]
pub struct Store {
    pool: SqlitePool,
}

#[derive(Debug)]
pub struct NewTicket {
    pub title: String,
    pub description: String,
}

#[derive(Debug, Default)]
pub struct TicketChanges {
    pub title: Option<String>,
    pub description: Option<String>,
    pub state: Option<TicketState>,
    pub expected_revision: Option<i64>,
}

impl Store {
    pub async fn open(path: &Path) -> Result<Self, sqlx::Error> {
        let options = SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(true)
            .foreign_keys(true);
        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .connect_with(options)
            .await?;
        let store = Self { pool };
        store.migrate().await?;
        Ok(store)
    }

    pub async fn memory() -> Result<Self, sqlx::Error> {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await?;
        let store = Self { pool };
        store.migrate().await?;
        Ok(store)
    }

    pub async fn close(self) {
        self.pool.close().await;
    }

    async fn migrate(&self) -> Result<(), sqlx::Error> {
        sqlx::migrate!().run(&self.pool).await.map_err(Into::into)
    }

    pub async fn project(&self) -> Result<Option<Project>, sqlx::Error> {
        sqlx::query_as("SELECT path, updated_at FROM projects WHERE id = 1")
            .fetch_optional(&self.pool)
            .await
    }

    pub async fn set_project(&self, path: &str) -> Result<Project, sqlx::Error> {
        sqlx::query(
            "INSERT INTO projects (id, path) VALUES (1, ?) \
             ON CONFLICT(id) DO UPDATE SET path = excluded.path, updated_at = unixepoch()",
        )
        .bind(path)
        .execute(&self.pool)
        .await?;
        Ok(self.project().await?.expect("project was just inserted"))
    }

    pub async fn tickets(&self) -> Result<Vec<Ticket>, sqlx::Error> {
        sqlx::query_as(
            "SELECT id, title, description, state, assignee_id, revision, created_at, updated_at \
             FROM tickets ORDER BY id",
        )
        .fetch_all(&self.pool)
        .await
    }

    pub async fn ticket(&self, id: i64) -> Result<Option<Ticket>, sqlx::Error> {
        sqlx::query_as(
            "SELECT id, title, description, state, assignee_id, revision, created_at, updated_at \
             FROM tickets WHERE id = ?",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
    }

    pub async fn create_ticket(&self, new: NewTicket) -> Result<Ticket, AppError> {
        validate_title(&new.title)?;
        let result = sqlx::query("INSERT INTO tickets (title, description) VALUES (?, ?)")
            .bind(new.title.trim())
            .bind(new.description)
            .execute(&self.pool)
            .await?;
        Ok(self.ticket(result.last_insert_rowid()).await?.unwrap())
    }

    pub async fn update_ticket(&self, id: i64, changes: TicketChanges) -> Result<Ticket, AppError> {
        let current = self
            .ticket(id)
            .await?
            .ok_or_else(|| AppError::not_found("ticket not found"))?;
        if let Some(expected) = changes.expected_revision
            && expected != current.revision
        {
            return Err(AppError::conflict("ticket was changed by another user"));
        }
        let title = changes.title.unwrap_or(current.title);
        validate_title(&title)?;
        let description = changes.description.unwrap_or(current.description);
        let state = changes.state.unwrap_or(current.state);
        let result = sqlx::query(
            "UPDATE tickets SET title = ?, description = ?, state = ?, revision = revision + 1, \
             updated_at = unixepoch() WHERE id = ? AND revision = ?",
        )
        .bind(title.trim())
        .bind(description)
        .bind(state)
        .bind(id)
        .bind(current.revision)
        .execute(&self.pool)
        .await?;
        if result.rows_affected() == 0 {
            return Err(AppError::conflict("ticket was changed by another user"));
        }
        Ok(self.ticket(id).await?.unwrap())
    }

    pub async fn channels(&self) -> Result<Vec<Channel>, sqlx::Error> {
        sqlx::query_as(
            "SELECT c.name, MAX(m.created_at) AS most_recent_message_at FROM channels c \
             LEFT JOIN messages m ON m.channel_name = c.name GROUP BY c.name ORDER BY c.name",
        )
        .fetch_all(&self.pool)
        .await
    }

    pub async fn messages(
        &self,
        channel: &str,
        start: Option<i64>,
        end: Option<i64>,
    ) -> Result<Vec<Message>, AppError> {
        validate_channel(channel)?;
        if !self.channel_exists(channel).await? {
            return Err(AppError::not_found("channel not found"));
        }
        Ok(sqlx::query_as(
            "SELECT id, channel_name, author, body, created_at FROM messages \
             WHERE channel_name = ? AND (? IS NULL OR created_at >= ?) \
             AND (? IS NULL OR created_at <= ?) ORDER BY created_at, id",
        )
        .bind(channel)
        .bind(start)
        .bind(start)
        .bind(end)
        .bind(end)
        .fetch_all(&self.pool)
        .await?)
    }

    pub async fn post_message(
        &self,
        channel: &str,
        author: &str,
        body: &str,
    ) -> Result<Message, AppError> {
        validate_channel(channel)?;
        validate_message(author, body)?;
        let mut transaction = self.pool.begin().await?;
        sqlx::query("INSERT OR IGNORE INTO channels (name) VALUES (?)")
            .bind(channel)
            .execute(&mut *transaction)
            .await?;
        let result =
            sqlx::query("INSERT INTO messages (channel_name, author, body) VALUES (?, ?, ?)")
                .bind(channel)
                .bind(author.trim())
                .bind(body.trim())
                .execute(&mut *transaction)
                .await?;
        let id = result.last_insert_rowid();
        let message = sqlx::query_as(
            "SELECT id, channel_name, author, body, created_at FROM messages WHERE id = ?",
        )
        .bind(id)
        .fetch_one(&mut *transaction)
        .await?;
        transaction.commit().await?;
        Ok(message)
    }

    async fn channel_exists(&self, channel: &str) -> Result<bool, sqlx::Error> {
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM channels WHERE name = ?)")
            .bind(channel)
            .fetch_one(&self.pool)
            .await
    }
}

fn validate_title(title: &str) -> Result<(), AppError> {
    if title.trim().is_empty() || title.chars().count() > 200 {
        return Err(AppError::bad_request(
            "title must contain between 1 and 200 characters",
        ));
    }
    Ok(())
}

fn validate_channel(channel: &str) -> Result<(), AppError> {
    if channel.is_empty()
        || channel.len() > 64
        || !channel
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-_.".contains(&byte))
    {
        return Err(AppError::bad_request(
            "channel names may contain 1-64 letters, numbers, '-', '_', or '.'",
        ));
    }
    Ok(())
}

fn validate_message(author: &str, body: &str) -> Result<(), AppError> {
    if author.trim().is_empty() || author.chars().count() > 100 {
        return Err(AppError::bad_request(
            "author must contain between 1 and 100 characters",
        ));
    }
    if body.trim().is_empty() || body.chars().count() > 10_000 {
        return Err(AppError::bad_request(
            "message must contain between 1 and 10000 characters",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    #[tokio::test]
    async fn ticket_updates_use_revisions() {
        let store = Store::memory().await.unwrap();
        let ticket = store
            .create_ticket(NewTicket {
                title: "First".into(),
                description: String::new(),
            })
            .await
            .unwrap();
        let updated = store
            .update_ticket(
                ticket.id,
                TicketChanges {
                    state: Some(TicketState::InProgress),
                    expected_revision: Some(ticket.revision),
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        assert_eq!(updated.state, TicketState::InProgress);
        assert_eq!(updated.revision, 2);
        assert!(
            store
                .update_ticket(
                    ticket.id,
                    TicketChanges {
                        expected_revision: Some(1),
                        ..Default::default()
                    }
                )
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn posting_creates_a_channel() {
        let store = Store::memory().await.unwrap();
        store
            .post_message("backend", "Ada", "Schema is ready")
            .await
            .unwrap();

        assert_eq!(store.channels().await.unwrap().len(), 2);
        assert_eq!(
            store
                .messages("backend", None, None)
                .await
                .unwrap()
                .first()
                .unwrap()
                .body,
            "Schema is ready"
        );
    }

    #[tokio::test]
    async fn file_store_survives_reopen() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("multi-agent-board-{unique}.sqlite3"));
        let store = Store::open(&path).await.unwrap();
        store
            .create_ticket(NewTicket {
                title: "Persistent".into(),
                description: String::new(),
            })
            .await
            .unwrap();
        store.close().await;

        let reopened = Store::open(&path).await.unwrap();
        assert_eq!(reopened.tickets().await.unwrap().len(), 1);
        reopened.close().await;
        std::fs::remove_file(path).unwrap();
    }
}
