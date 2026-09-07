use crate::{
    domain::{
        NotificationWakePayload, WakeDeliveryPolicy, WakeEvent, WakeEventData, WakeEventState,
    },
    error::AppError,
};

use super::Store;

#[derive(sqlx::FromRow)]
struct WakeEventRow {
    id: i64,
    agent_id: i64,
    kind: String,
    payload: String,
    deduplication_key: Option<String>,
    delivery_policy: WakeDeliveryPolicy,
    delivery_state: WakeEventState,
    delivery_attempts: i64,
    last_attempt_at: Option<i64>,
    delivered_at: Option<i64>,
    created_at: i64,
    updated_at: i64,
}

impl Store {
    pub async fn complete_ticket_assignment_wake(
        &self,
        agent_id: i64,
        ticket_id: i64,
        revision: i64,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            "UPDATE agent_wake_events SET delivery_state = 'delivered', delivered_at = unixepoch(), \
             updated_at = unixepoch() WHERE agent_id = ? AND kind = 'ticket.assigned' \
             AND deduplication_key = ? AND delivery_state = 'pending'",
        )
        .bind(agent_id)
        .bind(format!(
            "ticket:{ticket_id}:{revision}:ticket.assigned"
        ))
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn enqueue_wake_event(
        &self,
        agent_id: i64,
        event: WakeEventData,
        deduplication_key: Option<&str>,
        delivery_policy: WakeDeliveryPolicy,
    ) -> Result<WakeEvent, AppError> {
        if self.agent(agent_id).await?.is_none() {
            return Err(AppError::not_found("agent not found"));
        }
        validate_deduplication_key(deduplication_key)?;
        let kind = event.kind();
        let payload = serde_json::to_string(&event.payload()).map_err(|error| {
            AppError::bad_request(format!("invalid wake event payload: {error}"))
        })?;
        if payload.len() > 100_000 {
            return Err(AppError::bad_request("wake event payload is too large"));
        }
        let result = sqlx::query(
            "INSERT OR IGNORE INTO agent_wake_events \
             (agent_id, kind, payload, deduplication_key, delivery_policy) VALUES (?, ?, ?, ?, ?)",
        )
        .bind(agent_id)
        .bind(kind)
        .bind(payload)
        .bind(deduplication_key)
        .bind(delivery_policy)
        .execute(&self.pool)
        .await?;
        let id = if result.rows_affected() == 1 {
            result.last_insert_rowid()
        } else {
            sqlx::query_scalar(
                "SELECT id FROM agent_wake_events \
                 WHERE agent_id = ? AND deduplication_key = ?",
            )
            .bind(agent_id)
            .bind(deduplication_key)
            .fetch_one(&self.pool)
            .await?
        };
        self.wake_event(id)
            .await?
            .ok_or_else(|| AppError::internal("wake event disappeared after insertion"))
    }

    pub async fn wake_events(&self, agent_id: i64) -> Result<Vec<WakeEvent>, AppError> {
        if self.agent(agent_id).await?.is_none() {
            return Err(AppError::not_found("agent not found"));
        }
        let rows: Vec<WakeEventRow> = sqlx::query_as(
            "SELECT id, agent_id, kind, payload, deduplication_key, delivery_policy, delivery_state, \
             delivery_attempts, last_attempt_at, delivered_at, created_at, updated_at \
             FROM agent_wake_events WHERE agent_id = ? ORDER BY id",
        )
        .bind(agent_id)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(wake_event_from_row).collect()
    }

    pub async fn claim_pending_wake_event(
        &self,
        agent_id: i64,
        include_idle_only: bool,
    ) -> Result<Option<WakeEvent>, AppError> {
        let mut transaction = self.pool.begin().await?;
        let row: Option<WakeEventRow> = sqlx::query_as(
            "UPDATE agent_wake_events SET delivery_state = 'delivering', \
             delivery_attempts = delivery_attempts + 1, last_attempt_at = unixepoch(), \
             updated_at = unixepoch() WHERE id = (SELECT id FROM agent_wake_events \
             WHERE agent_id = ? AND delivery_state = 'pending' \
             AND (? OR delivery_policy = 'immediate') ORDER BY id LIMIT 1) \
             RETURNING id, agent_id, kind, payload, deduplication_key, delivery_policy, delivery_state, \
             delivery_attempts, last_attempt_at, delivered_at, created_at, updated_at",
        )
        .bind(agent_id)
        .bind(include_idle_only)
        .fetch_optional(&mut *transaction)
        .await?;
        let event = row.map(wake_event_from_row).transpose()?;
        transaction.commit().await?;
        Ok(event)
    }

    pub async fn complete_wake_event(&self, id: i64) -> Result<(), AppError> {
        let result = sqlx::query(
            "UPDATE agent_wake_events SET delivery_state = 'delivered', \
             delivered_at = unixepoch(), updated_at = unixepoch() \
             WHERE id = ? AND delivery_state = 'delivering'",
        )
        .bind(id)
        .execute(&self.pool)
        .await?;
        if result.rows_affected() == 0 {
            return Err(AppError::conflict("wake event is not being delivered"));
        }
        Ok(())
    }

    pub async fn retry_wake_event(&self, id: i64) -> Result<(), AppError> {
        let result = sqlx::query(
            "UPDATE agent_wake_events SET delivery_state = 'pending', updated_at = unixepoch() \
             WHERE id = ? AND delivery_state = 'delivering'",
        )
        .bind(id)
        .execute(&self.pool)
        .await?;
        if result.rows_affected() == 0 {
            return Err(AppError::conflict("wake event is not being delivered"));
        }
        Ok(())
    }

    pub async fn recover_wake_events_after_restart(&self) -> Result<u64, sqlx::Error> {
        Ok(sqlx::query(
            "UPDATE agent_wake_events SET delivery_state = 'pending', updated_at = unixepoch() \
             WHERE delivery_state = 'delivering'",
        )
        .execute(&self.pool)
        .await?
        .rows_affected())
    }

    async fn wake_event(&self, id: i64) -> Result<Option<WakeEvent>, AppError> {
        let row: Option<WakeEventRow> = sqlx::query_as(
            "SELECT id, agent_id, kind, payload, deduplication_key, delivery_policy, delivery_state, \
             delivery_attempts, last_attempt_at, delivered_at, created_at, updated_at \
             FROM agent_wake_events WHERE id = ?",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        row.map(wake_event_from_row).transpose()
    }

    pub async fn enqueue_notification(&self, agent_id: i64, body: &str) -> Result<(), AppError> {
        if body.trim().is_empty() || body.chars().count() > 50_000 {
            return Err(AppError::bad_request("invalid agent notification"));
        }
        self.enqueue_wake_event(
            agent_id,
            WakeEventData::Notification(NotificationWakePayload {
                message: body.trim().to_owned(),
            }),
            None,
            WakeDeliveryPolicy::Immediate,
        )
        .await?;
        Ok(())
    }

    pub async fn pending_notification(
        &self,
        agent_id: i64,
    ) -> Result<Option<(i64, String)>, AppError> {
        let row: Option<WakeEventRow> = sqlx::query_as(
            "UPDATE agent_wake_events SET delivery_state = 'delivering', \
             delivery_attempts = delivery_attempts + 1, last_attempt_at = unixepoch(), \
             updated_at = unixepoch() WHERE id = (SELECT id FROM agent_wake_events \
             WHERE agent_id = ? AND kind = 'notification' AND delivery_state = 'pending' \
             AND json_type(payload, '$.message') = 'text' ORDER BY id LIMIT 1) \
             RETURNING id, agent_id, kind, payload, deduplication_key, delivery_policy, delivery_state, \
             delivery_attempts, last_attempt_at, delivered_at, created_at, updated_at",
        )
        .bind(agent_id)
        .fetch_optional(&self.pool)
        .await?;
        let event = row.map(wake_event_from_row).transpose()?;
        event
            .map(|event| {
                let WakeEventData::Notification(payload) = event.event else {
                    return Err(AppError::internal(
                        "claimed wake event is not a notification",
                    ));
                };
                let message = payload.message;
                Ok((event.id, message))
            })
            .transpose()
    }

    pub async fn delete_notification(&self, id: i64) -> Result<(), AppError> {
        self.complete_wake_event(id).await
    }
}

fn validate_deduplication_key(deduplication_key: Option<&str>) -> Result<(), AppError> {
    if deduplication_key.is_some_and(|key| key.is_empty() || key.len() > 500) {
        return Err(AppError::bad_request(
            "invalid wake event deduplication key",
        ));
    }
    Ok(())
}

fn wake_event_from_row(row: WakeEventRow) -> Result<WakeEvent, AppError> {
    let payload = serde_json::from_str(&row.payload).map_err(|error| {
        AppError::internal(format!("invalid stored wake event payload: {error}"))
    })?;
    let event = WakeEventData::from_parts(&row.kind, payload)
        .map_err(|error| AppError::internal(format!("invalid stored wake event: {error}")))?;
    Ok(WakeEvent {
        id: row.id,
        agent_id: row.agent_id,
        event,
        deduplication_key: row.deduplication_key,
        delivery_policy: row.delivery_policy,
        delivery_state: row.delivery_state,
        delivery_attempts: row.delivery_attempts,
        last_attempt_at: row.last_attempt_at,
        delivered_at: row.delivered_at,
        created_at: row.created_at,
        updated_at: row.updated_at,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn reviewer_notifications_are_selected_and_queued_in_order() {
        let store = Store::memory().await.unwrap();
        let worker = store.create_agent("worker", "/tmp/worker").await.unwrap();
        let reviewer = store
            .create_agent("reviewer", "/tmp/reviewer")
            .await
            .unwrap();
        store
            .set_agent_scopes(reviewer.id, &["approve_review".into()])
            .await
            .unwrap();

        assert_eq!(
            store
                .agents_with_scope("approve_review", worker.id)
                .await
                .unwrap()[0]
                .id,
            reviewer.id
        );
        store
            .enqueue_notification(reviewer.id, "first")
            .await
            .unwrap();
        store
            .enqueue_notification(reviewer.id, "second")
            .await
            .unwrap();
        let first = store
            .pending_notification(reviewer.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(first.1, "first");
        store.delete_notification(first.0).await.unwrap();
        assert_eq!(
            store
                .pending_notification(reviewer.id)
                .await
                .unwrap()
                .unwrap()
                .1,
            "second"
        );
    }

    #[tokio::test]
    async fn notification_consumer_skips_generic_and_malformed_events() {
        let store = Store::memory().await.unwrap();
        let agent = store.create_agent("worker", "/tmp/worker").await.unwrap();
        store
            .enqueue_wake_event(
                agent.id,
                WakeEventData::ChannelMessage(crate::domain::ChannelMessageWakePayload {
                    message: "posted".into(),
                    channel: "general".into(),
                    message_id: 1,
                    author: "human".into(),
                    body: "posted".into(),
                    mentioned: false,
                    subscribed: true,
                }),
                None,
                WakeDeliveryPolicy::Immediate,
            )
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO agent_wake_events (agent_id, kind, payload, delivery_policy) \
             VALUES (?, 'notification', '{\"unexpected\":true}', 'immediate')",
        )
        .bind(agent.id)
        .execute(&store.pool)
        .await
        .unwrap();
        store
            .enqueue_notification(agent.id, "review requested")
            .await
            .unwrap();

        let notification = store.pending_notification(agent.id).await.unwrap().unwrap();
        assert_eq!(notification.1, "review requested");
        let states: Vec<(String, WakeEventState)> =
            sqlx::query_as("SELECT kind, delivery_state FROM agent_wake_events ORDER BY id")
                .fetch_all(&store.pool)
                .await
                .unwrap();
        assert_eq!(
            states[0],
            ("channel.message".into(), WakeEventState::Pending)
        );
        assert_eq!(states[1], ("notification".into(), WakeEventState::Pending));
        assert_eq!(
            states[2],
            ("notification".into(), WakeEventState::Delivering)
        );
        assert!(store.wake_events(agent.id).await.is_err());
    }

    #[tokio::test]
    async fn wake_events_are_deduplicated_delivered_and_recovered() {
        let store = Store::memory().await.unwrap();
        let agent = store.create_agent("worker", "/tmp/worker").await.unwrap();
        let first = store
            .enqueue_wake_event(
                agent.id,
                WakeEventData::Notification(NotificationWakePayload {
                    message: "first".into(),
                }),
                Some("ticket:3:available"),
                WakeDeliveryPolicy::IdleOnly,
            )
            .await
            .unwrap();
        let duplicate = store
            .enqueue_wake_event(
                agent.id,
                WakeEventData::Notification(NotificationWakePayload {
                    message: "ignored".into(),
                }),
                Some("ticket:3:available"),
                WakeDeliveryPolicy::IdleOnly,
            )
            .await
            .unwrap();
        assert_eq!(duplicate.id, first.id);
        assert_eq!(duplicate.event, first.event);

        let delivering = store
            .claim_pending_wake_event(agent.id, true)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(delivering.delivery_state, WakeEventState::Delivering);
        assert_eq!(delivering.delivery_attempts, 1);
        assert_eq!(store.recover_wake_events_after_restart().await.unwrap(), 1);

        let retry = store
            .claim_pending_wake_event(agent.id, true)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(retry.id, first.id);
        assert_eq!(retry.delivery_attempts, 2);
        store.complete_wake_event(retry.id).await.unwrap();
        let events = store.wake_events(agent.id).await.unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].delivery_state, WakeEventState::Delivered);
        assert!(events[0].delivered_at.is_some());
        assert!(
            store
                .claim_pending_wake_event(agent.id, true)
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn typed_wake_event_is_delivered_after_reopen() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("mab-wakes-{unique}.sqlite3"));
        let store = Store::open(&path).await.unwrap();
        let agent = store.create_agent("worker", "/tmp/worker").await.unwrap();
        store
            .enqueue_notification(agent.id, "Survive restart")
            .await
            .unwrap();
        store.close().await;

        let reopened = Store::open(&path).await.unwrap();
        let event = reopened
            .claim_pending_wake_event(agent.id, true)
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(
            event.event,
            WakeEventData::Notification(NotificationWakePayload { ref message })
                if message == "Survive restart"
        ));
        reopened.complete_wake_event(event.id).await.unwrap();
        reopened.close().await;
        std::fs::remove_file(path).unwrap();
    }

    #[tokio::test]
    async fn invalid_stored_wake_is_rejected_without_stranding_it() {
        let store = Store::memory().await.unwrap();
        let agent = store.create_agent("worker", "/tmp/worker").await.unwrap();
        sqlx::query(
            "INSERT INTO agent_wake_events (agent_id, kind, payload, delivery_policy) \
             VALUES (?, 'unknown', '{}', 'immediate')",
        )
        .bind(agent.id)
        .execute(&store.pool)
        .await
        .unwrap();

        assert!(
            store
                .claim_pending_wake_event(agent.id, true)
                .await
                .is_err()
        );
        let state: WakeEventState =
            sqlx::query_scalar("SELECT delivery_state FROM agent_wake_events")
                .fetch_one(&store.pool)
                .await
                .unwrap();
        assert_eq!(state, WakeEventState::Pending);
    }
}
