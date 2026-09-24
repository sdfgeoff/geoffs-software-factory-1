use multi_agent_board_protocol::AgentEvent;

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
    pub(crate) async fn append_board_event(&self, event: &AgentEvent) -> Result<i64, sqlx::Error> {
        let mut transaction = self.pool.begin().await?;
        let id: i64 = sqlx::query_scalar(
            "INSERT INTO board_change_events (event_json) VALUES (?) RETURNING id",
        )
        .bind(serde_json::to_string(event).expect("protocol events serialize"))
        .fetch_one(&mut *transaction)
        .await?;
        sqlx::query(
            "DELETE FROM board_change_events
             WHERE id <= COALESCE((
                 SELECT id FROM board_change_events
                 ORDER BY id DESC LIMIT 1 OFFSET ?
             ), 0)",
        )
        .bind(BOARD_EVENT_RETENTION)
        .execute(&mut *transaction)
        .await?;
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

#[cfg(test)]
mod tests {
    use multi_agent_board_protocol::{AgentEvent, BoardSettings};

    use super::*;

    fn event(channel: &str) -> AgentEvent {
        AgentEvent::SettingsUpdated {
            agent_id: 0,
            data: BoardSettings {
                merge_notification_channel: Some(channel.into()),
            },
        }
    }

    #[tokio::test]
    async fn replay_is_ordered_and_survives_the_retention_boundary() {
        let store = Store::memory().await.unwrap();
        for index in 0..=BOARD_EVENT_RETENTION {
            store
                .append_board_event(&event(&index.to_string()))
                .await
                .unwrap();
        }

        assert!(matches!(
            store.replay_board_events(Some(0)).await.unwrap(),
            BoardEventReplay::Reset { latest_id } if latest_id == BOARD_EVENT_RETENTION + 1
        ));
        let BoardEventReplay::Events(events) = store.replay_board_events(Some(1)).await.unwrap()
        else {
            panic!("retained cursor should replay");
        };
        assert_eq!(events.len() as i64, BOARD_EVENT_RETENTION);
        assert_eq!(events.first().unwrap().id, 2);
        assert_eq!(events.last().unwrap().id, BOARD_EVENT_RETENTION + 1);
    }

    #[tokio::test]
    async fn ticket_activity_events_replay_with_their_stable_entry_id() {
        let store = Store::memory().await.unwrap();
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

        let BoardEventReplay::Events(events) = store.replay_board_events(Some(0)).await.unwrap()
        else {
            panic!("activity event should replay");
        };
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event, event);
    }
}
