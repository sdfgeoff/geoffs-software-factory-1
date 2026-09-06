use crate::{
    domain::{WakeDeliveryPolicy, WakeEvent, WakeEventState},
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
    pub async fn enqueue_wake_event(
        &self,
        agent_id: i64,
        kind: &str,
        payload: serde_json::Value,
        deduplication_key: Option<&str>,
        delivery_policy: WakeDeliveryPolicy,
    ) -> Result<WakeEvent, AppError> {
        if self.agent(agent_id).await?.is_none() {
            return Err(AppError::not_found("agent not found"));
        }
        validate_wake_event(kind, deduplication_key)?;
        let payload = serde_json::to_string(&payload).map_err(|error| {
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
        .fetch_optional(&self.pool)
        .await?;
        row.map(wake_event_from_row).transpose()
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
            "notification",
            serde_json::json!({"message": body.trim()}),
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
                let message = event
                    .payload
                    .get("message")
                    .and_then(serde_json::Value::as_str)
                    .ok_or_else(|| AppError::internal("notification payload has no message"))?
                    .to_owned();
                Ok((event.id, message))
            })
            .transpose()
    }

    pub async fn delete_notification(&self, id: i64) -> Result<(), AppError> {
        self.complete_wake_event(id).await
    }
}

fn validate_wake_event(kind: &str, deduplication_key: Option<&str>) -> Result<(), AppError> {
    if kind.is_empty()
        || kind.len() > 100
        || !kind
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.' | b':'))
    {
        return Err(AppError::bad_request("invalid wake event kind"));
    }
    if deduplication_key.is_some_and(|key| key.is_empty() || key.len() > 500) {
        return Err(AppError::bad_request(
            "invalid wake event deduplication key",
        ));
    }
    Ok(())
}

fn wake_event_from_row(row: WakeEventRow) -> Result<WakeEvent, AppError> {
    Ok(WakeEvent {
        id: row.id,
        agent_id: row.agent_id,
        kind: row.kind,
        payload: serde_json::from_str(&row.payload)
            .map_err(|error| AppError::internal(format!("invalid stored wake event: {error}")))?,
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
                "channel.message",
                serde_json::json!({"message_id": 1}),
                None,
                WakeDeliveryPolicy::Immediate,
            )
            .await
            .unwrap();
        store
            .enqueue_wake_event(
                agent.id,
                "notification",
                serde_json::json!({"unexpected": true}),
                None,
                WakeDeliveryPolicy::Immediate,
            )
            .await
            .unwrap();
        store
            .enqueue_notification(agent.id, "review requested")
            .await
            .unwrap();

        let notification = store.pending_notification(agent.id).await.unwrap().unwrap();
        assert_eq!(notification.1, "review requested");
        let events = store.wake_events(agent.id).await.unwrap();
        assert_eq!(events[0].kind, "channel.message");
        assert_eq!(events[0].delivery_state, WakeEventState::Pending);
        assert_eq!(events[1].kind, "notification");
        assert_eq!(events[1].delivery_state, WakeEventState::Pending);
        assert_eq!(events[2].id, notification.0);
        assert_eq!(events[2].delivery_state, WakeEventState::Delivering);
    }

    #[tokio::test]
    async fn wake_events_are_deduplicated_delivered_and_recovered() {
        let store = Store::memory().await.unwrap();
        let agent = store.create_agent("worker", "/tmp/worker").await.unwrap();
        let first = store
            .enqueue_wake_event(
                agent.id,
                "ticket.available",
                serde_json::json!({"ticket_id": 3}),
                Some("ticket:3:available"),
                WakeDeliveryPolicy::IdleOnly,
            )
            .await
            .unwrap();
        let duplicate = store
            .enqueue_wake_event(
                agent.id,
                "ticket.available",
                serde_json::json!({"ticket_id": 3, "ignored": true}),
                Some("ticket:3:available"),
                WakeDeliveryPolicy::IdleOnly,
            )
            .await
            .unwrap();
        assert_eq!(duplicate.id, first.id);
        assert_eq!(duplicate.payload, first.payload);

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
}
