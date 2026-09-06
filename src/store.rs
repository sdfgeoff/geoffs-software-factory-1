use std::{path::Path, time::Duration};

use sqlx::{
    SqlitePool,
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions},
};

mod agents;
mod messages;
mod settings;
mod tickets;
mod wakes;

pub(crate) use messages::validate_channel;
pub(crate) use settings::MERGE_NOTIFICATION_CHANNEL_DESCRIPTION;
pub(crate) use tickets::MergeNotification;
pub use tickets::{NewTicket, ReviewSubmission, TicketChanges};

#[derive(Clone)]
pub struct Store {
    pool: SqlitePool,
}

impl Store {
    pub async fn open(path: &Path) -> Result<Self, sqlx::Error> {
        let options = SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(true)
            .foreign_keys(true)
            .journal_mode(SqliteJournalMode::Wal)
            .busy_timeout(Duration::from_secs(5));
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
        store
            .set_project("/tmp/multi-agent-board-test-project")
            .await?;
        Ok(store)
    }

    pub async fn close(self) {
        self.pool.close().await;
    }

    #[cfg(test)]
    pub(crate) async fn execute_test_sql(&self, statement: &str) -> Result<(), sqlx::Error> {
        sqlx::query(statement).execute(&self.pool).await?;
        Ok(())
    }

    async fn migrate(&self) -> Result<(), sqlx::Error> {
        sqlx::migrate!().run(&self.pool).await.map_err(Into::into)
    }
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    #[tokio::test]
    async fn project_migration_backfills_existing_tickets() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        let migrator = sqlx::migrate!();
        for migration in migrator.iter().filter(|migration| migration.version < 18) {
            sqlx::raw_sql(&migration.sql).execute(&pool).await.unwrap();
        }
        sqlx::query("INSERT INTO projects (id, path) VALUES (1, '/existing-project')")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO tickets (title) VALUES ('existing ticket')")
            .execute(&pool)
            .await
            .unwrap();
        let migration = migrator
            .iter()
            .find(|migration| migration.version == 18)
            .unwrap();
        sqlx::raw_sql(&migration.sql).execute(&pool).await.unwrap();

        let project_id: i64 = sqlx::query_scalar("SELECT project_id FROM tickets WHERE id = 1")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(project_id, 1);
        let foreign_key_errors: Vec<(String, i64, String, i64)> =
            sqlx::query_as("PRAGMA foreign_key_check")
                .fetch_all(&pool)
                .await
                .unwrap();
        assert!(foreign_key_errors.is_empty());
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
            .set_project("/tmp/multi-agent-board-test-project")
            .await
            .unwrap();
        store
            .create_ticket(NewTicket {
                title: "Persistent".into(),
                description: String::new(),
                blocker_ids: Vec::new(),
                created_by_agent_id: None,
            })
            .await
            .unwrap();
        let agent = store
            .create_agent("persistent-agent", "/tmp/persistent-agent")
            .await
            .unwrap();
        store
            .set_agent_wake_settings(agent.id, true, false, Some(20))
            .await
            .unwrap();
        store.close().await;

        let reopened = Store::open(&path).await.unwrap();
        assert_eq!(reopened.tickets().await.unwrap().len(), 1);
        let agent = reopened.agent(agent.id).await.unwrap().unwrap();
        let project = reopened.project().await.unwrap().unwrap();
        assert_eq!(
            reopened
                .agent_project_worktree(agent.id, project.id)
                .await
                .unwrap()
                .unwrap()
                .path,
            "/tmp/persistent-agent"
        );
        assert!(agent.wake_on_pending_work);
        assert!(!agent.wake_on_pending_review);
        assert_eq!(agent.idle_heartbeat_minutes, Some(20));
        reopened.close().await;
        std::fs::remove_file(path).unwrap();
    }
}
