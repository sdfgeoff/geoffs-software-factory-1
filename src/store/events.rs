use multi_agent_board_protocol::AgentEvent;
use sqlx::{Sqlite, Transaction};

use super::Store;

pub(crate) const BOARD_EVENT_RETENTION: i64 = 1_024;

#[derive(Debug)]
pub(crate) struct StoredBoardEvent {
    pub id: i64,
    pub event: AgentEvent,
}

#[derive(Debug)]
pub(crate) enum BoardEventReplay {
    Events(Vec<StoredBoardEvent>),
    Reset { latest_id: i64 },
}

impl Store {
    pub(crate) async fn append_agent_change_event(
        transaction: &mut Transaction<'_, Sqlite>,
        agent_id: i64,
        revision: i64,
    ) -> Result<i64, sqlx::Error> {
        let event = AgentEvent::AgentChanged { agent_id, revision };
        let id = sqlx::query_scalar(
            "INSERT INTO board_change_events
             (event_json, resource_type, resource_id, resource_revision)
             VALUES (?, 'agent', ?, ?) RETURNING id",
        )
        .bind(serde_json::to_string(&event).expect("protocol events serialize"))
        .bind(agent_id)
        .bind(revision)
        .fetch_one(&mut **transaction)
        .await?;
        prune_board_events(transaction).await?;
        Ok(id)
    }

    pub(crate) async fn agent_change_event_id(
        &self,
        agent_id: i64,
        revision: i64,
    ) -> Result<i64, sqlx::Error> {
        sqlx::query_scalar(
            "SELECT id FROM board_change_events
             WHERE resource_type = 'agent' AND resource_id = ? AND resource_revision = ?",
        )
        .bind(agent_id)
        .bind(revision)
        .fetch_one(&self.pool)
        .await
    }

    pub(crate) async fn latest_projection_event_id(
        &self,
        resource_type: &str,
        resource_id: Option<i64>,
    ) -> Result<i64, sqlx::Error> {
        sqlx::query_scalar(
            "SELECT id FROM board_change_events
             WHERE resource_type = ? AND (? IS NULL OR resource_id = ?)
             ORDER BY id DESC LIMIT 1",
        )
        .bind(resource_type)
        .bind(resource_id)
        .bind(resource_id)
        .fetch_one(&self.pool)
        .await
    }

    #[cfg(test)]
    pub(crate) async fn latest_board_event_id(&self) -> Result<i64, sqlx::Error> {
        sqlx::query_scalar("SELECT COALESCE(MAX(id), 0) FROM board_change_events")
            .fetch_one(&self.pool)
            .await
    }

    pub(crate) async fn append_board_event(&self, event: &AgentEvent) -> Result<i64, sqlx::Error> {
        let mut transaction = self.pool.begin().await?;
        let id: i64 = sqlx::query_scalar(
            "INSERT INTO board_change_events (event_json) VALUES (?) RETURNING id",
        )
        .bind(serde_json::to_string(event).expect("protocol events serialize"))
        .fetch_one(&mut *transaction)
        .await?;
        prune_board_events(&mut transaction).await?;
        transaction.commit().await?;
        Ok(id)
    }

    pub(crate) async fn replay_board_events(
        &self,
        after_id: Option<i64>,
    ) -> Result<BoardEventReplay, sqlx::Error> {
        let (oldest_id, latest_id): (Option<i64>, Option<i64>) =
            sqlx::query_as("SELECT MIN(id), MAX(id) FROM board_change_events")
                .fetch_one(&self.pool)
                .await?;
        let latest_id = latest_id.unwrap_or_default();
        if after_id.is_none() {
            return Ok(BoardEventReplay::Reset { latest_id });
        }
        if after_id.is_some_and(|id| {
            id > latest_id || oldest_id.is_some_and(|oldest| id < oldest.saturating_sub(1))
        }) {
            return Ok(BoardEventReplay::Reset { latest_id });
        }
        let rows: Vec<(i64, String)> = sqlx::query_as(
            "SELECT id, event_json FROM board_change_events
             WHERE id > ? ORDER BY id",
        )
        .bind(after_id.unwrap_or_default())
        .fetch_all(&self.pool)
        .await?;
        let events = rows
            .into_iter()
            .map(|(id, json)| {
                serde_json::from_str(&json)
                    .map(|event| StoredBoardEvent { id, event })
                    .map_err(|error| sqlx::Error::Decode(Box::new(error)))
            })
            .collect::<Result<_, _>>()?;
        Ok(BoardEventReplay::Events(events))
    }
}

async fn prune_board_events(transaction: &mut Transaction<'_, Sqlite>) -> Result<(), sqlx::Error> {
    sqlx::query(
        "DELETE FROM board_change_events
         WHERE id <= COALESCE((
             SELECT id FROM board_change_events
             ORDER BY id DESC LIMIT 1 OFFSET ?
         ), 0)",
    )
    .bind(BOARD_EVENT_RETENTION)
    .execute(&mut **transaction)
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use multi_agent_board_protocol::{AgentEvent, BoardSettings};
    use sqlx::sqlite::SqlitePoolOptions;
    use std::{
        sync::Arc,
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::*;

    fn event(channel: &str) -> AgentEvent {
        AgentEvent::SettingsUpdated {
            agent_id: 0,
            data: BoardSettings {
                merge_notification_channel: Some(channel.into()),
                revision: 1,
            },
        }
    }

    #[tokio::test]
    async fn replay_is_ordered_and_survives_the_retention_boundary() {
        let store = Store::memory().await.unwrap();
        let cursor = store.latest_board_event_id().await.unwrap();
        for index in 0..=BOARD_EVENT_RETENTION {
            store
                .append_board_event(&event(&index.to_string()))
                .await
                .unwrap();
        }

        assert!(matches!(
            store.replay_board_events(Some(cursor)).await.unwrap(),
            BoardEventReplay::Reset { latest_id } if latest_id == cursor + BOARD_EVENT_RETENTION + 1
        ));
        let BoardEventReplay::Events(events) =
            store.replay_board_events(Some(cursor + 1)).await.unwrap()
        else {
            panic!("retained cursor should replay");
        };
        assert_eq!(events.len() as i64, BOARD_EVENT_RETENTION);
        assert_eq!(events.first().unwrap().id, cursor + 2);
        assert_eq!(
            events.last().unwrap().id,
            cursor + BOARD_EVENT_RETENTION + 1
        );
    }

    #[tokio::test]
    async fn ticket_activity_events_replay_with_their_stable_entry_id() {
        let store = Store::memory().await.unwrap();
        let cursor = store.latest_board_event_id().await.unwrap();
        let event = AgentEvent::TicketActivityCreated {
            agent_id: 1,
            data: multi_agent_board_protocol::TicketActivity {
                id: 9,
                ticket_id: 4,
                action: "comment".into(),
                detail: Some("hello".into()),
                author: Some("worker".into()),
                actor_agent_id: Some(1),
                previous_reserved_for_agent_id: None,
                reserved_for_agent_id: None,
                implementation_branch: None,
                review_commit: None,
                review_target_branch: None,
                attachments: vec![multi_agent_board_protocol::Attachment {
                    id: "a".repeat(32),
                    original_filename: "evidence.png".into(),
                    media_type: "image/png".into(),
                    byte_size: 42,
                    download_url: format!("/api/attachments/{}", "a".repeat(32)),
                }],
                created_at: 1,
            },
        };
        store.append_board_event(&event).await.unwrap();

        let BoardEventReplay::Events(events) =
            store.replay_board_events(Some(cursor)).await.unwrap()
        else {
            panic!("activity event should replay");
        };
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event, event);
    }

    #[tokio::test]
    async fn mutable_projection_writes_transactionally_persist_invalidations() {
        let store = Store::memory().await.unwrap();
        let agent = store.create_agent("worker", "/tmp/worker").await.unwrap();
        let cursor = store.latest_board_event_id().await.unwrap();

        store
            .register_project(
                "second",
                "/tmp/second-live-sync-project",
                "main",
                crate::domain::ProjectCompletionWorkflow::BoardManaged,
            )
            .await
            .unwrap();
        store.create_channel("sync", "Live sync").await.unwrap();
        store
            .set_merge_notification_channel(Some("sync"))
            .await
            .unwrap();
        store
            .set_agent_scopes(agent.id, &["approve_review".into()])
            .await
            .unwrap();
        store
            .set_agent_channel_subscriptions(agent.id, &["sync".into()])
            .await
            .unwrap();
        store
            .set_agent_project_subscriptions(agent.id, &[])
            .await
            .unwrap();
        let ticket = store
            .create_ticket(crate::store::NewTicket {
                title: "Persist every invalidation".into(),
                description: String::new(),
                blocker_ids: Vec::new(),
                created_by_agent_id: Some(agent.id),
            })
            .await
            .unwrap();
        store
            .add_ticket_comment(ticket.id, Some(agent.id), "Durable activity")
            .await
            .unwrap();
        store
            .post_message("sync", Some(agent.id), "worker", "Durable message")
            .await
            .unwrap();
        store
            .add_agent_message(agent.id, "user", "Durable conversation")
            .await
            .unwrap();

        let resource_types: Vec<String> = sqlx::query_scalar(
            "SELECT DISTINCT resource_type FROM board_change_events \
             WHERE id > ? AND resource_type IS NOT NULL ORDER BY resource_type",
        )
        .bind(cursor)
        .fetch_all(&store.pool)
        .await
        .unwrap();
        for resource_type in [
            "agent_details",
            "agent_messages",
            "board_messages",
            "channels",
            "projects",
            "settings",
            "ticket_activity",
            "tickets",
        ] {
            assert!(
                resource_types.iter().any(|value| value == resource_type),
                "missing transactional invalidation for {resource_type}: {resource_types:?}"
            );
        }
    }

    #[tokio::test]
    async fn projection_mutation_rolls_back_when_the_outbox_is_unavailable() {
        let store = Store::memory().await.unwrap();
        store
            .execute_test_sql("DROP TABLE board_change_events")
            .await
            .unwrap();

        assert!(
            store
                .create_channel("uncommitted", "Must have an outbox event")
                .await
                .is_err()
        );
        assert!(store.channel("uncommitted").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn agent_revision_migration_invalidates_incompatible_legacy_replay_rows() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("board-legacy-events-{unique}.sqlite3"));
        let url = format!("sqlite://{}?mode=rwc", path.display());
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect(&url)
            .await
            .unwrap();
        let migrator = sqlx::migrate!();
        for migration in migrator.iter().filter(|migration| migration.version < 32) {
            sqlx::raw_sql(&migration.sql).execute(&pool).await.unwrap();
        }
        let legacy = serde_json::json!({
            "kind": "agent_updated",
            "agent_id": 1,
            "data": {
                "id": 1,
                "name": "worker",
                "instructions": "",
                "state": "idle",
                "thread_id": null,
                "home_path": "/tmp/worker.agent-home",
                "active_project_id": null,
                "active_turn_id": null,
                "last_error": null,
                "wake_on_pending_work": false,
                "wake_on_pending_review": false,
                "idle_heartbeat_minutes": null,
                "idle_since": 1,
                "created_at": 1,
                "updated_at": 1
            }
        });
        let old_id: i64 = sqlx::query_scalar(
            "INSERT INTO board_change_events(event_json) VALUES (?) RETURNING id",
        )
        .bind(legacy.to_string())
        .fetch_one(&pool)
        .await
        .unwrap();
        let migration = migrator
            .iter()
            .find(|migration| migration.version == 32)
            .unwrap();
        sqlx::raw_sql(&migration.sql).execute(&pool).await.unwrap();
        pool.close().await;

        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect(&url)
            .await
            .unwrap();
        let store = Store {
            pool,
            attachment_root: Arc::new(path.with_extension("attachments")),
        };
        assert!(matches!(
            store.replay_board_events(Some(old_id)).await.unwrap(),
            BoardEventReplay::Reset { latest_id: 0 }
        ));
        store.close().await;
        std::fs::remove_file(path).unwrap();
    }
}
