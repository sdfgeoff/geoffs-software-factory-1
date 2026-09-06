use crate::{
    domain::{Reminder, ReminderState, WakeDeliveryPolicy},
    error::AppError,
};

use super::Store;

pub(crate) const MAX_REMINDER_DELAY_SECONDS: i64 = 31_536_000;

impl Store {
    pub async fn create_reminder(
        &self,
        agent_id: i64,
        message: &str,
        deliver_at: i64,
        now: i64,
    ) -> Result<Reminder, AppError> {
        if self.agent(agent_id).await?.is_none() {
            return Err(AppError::not_found("agent not found"));
        }
        let message = message.trim();
        if message.is_empty() || message.chars().count() > 50_000 {
            return Err(AppError::bad_request(
                "reminder message must contain between 1 and 50000 characters",
            ));
        }
        if deliver_at <= now {
            return Err(AppError::bad_request("deliver_at must be in the future"));
        }
        if deliver_at > now.saturating_add(MAX_REMINDER_DELAY_SECONDS) {
            return Err(AppError::bad_request(
                "deliver_at must be within 31536000 seconds",
            ));
        }
        let id = sqlx::query(
            "INSERT INTO agent_reminders (agent_id, message, deliver_at) VALUES (?, ?, ?)",
        )
        .bind(agent_id)
        .bind(message)
        .bind(deliver_at)
        .execute(&self.pool)
        .await?
        .last_insert_rowid();
        self.reminder(agent_id, id)
            .await?
            .ok_or_else(|| AppError::internal("reminder disappeared after insertion"))
    }

    pub async fn reminders(&self, agent_id: i64) -> Result<Vec<Reminder>, AppError> {
        if self.agent(agent_id).await?.is_none() {
            return Err(AppError::not_found("agent not found"));
        }
        Ok(sqlx::query_as(
            "SELECT id, agent_id, message, deliver_at, state, wake_event_id, enqueued_at, \
             cancelled_at, created_at, updated_at FROM agent_reminders \
             WHERE agent_id = ? ORDER BY id",
        )
        .bind(agent_id)
        .fetch_all(&self.pool)
        .await?)
    }

    pub async fn cancel_reminder(&self, agent_id: i64, id: i64) -> Result<Reminder, AppError> {
        let cancelled: Option<Reminder> = sqlx::query_as(
            "UPDATE agent_reminders SET state = 'cancelled', cancelled_at = unixepoch(), \
             updated_at = unixepoch() WHERE id = ? AND agent_id = ? AND state = 'scheduled' \
             RETURNING id, agent_id, message, deliver_at, state, wake_event_id, enqueued_at, \
             cancelled_at, created_at, updated_at",
        )
        .bind(id)
        .bind(agent_id)
        .fetch_optional(&self.pool)
        .await?;
        if let Some(cancelled) = cancelled {
            return Ok(cancelled);
        }
        let reminder = self
            .reminder(agent_id, id)
            .await?
            .ok_or_else(|| AppError::not_found("reminder not found"))?;
        Err(AppError::conflict(format!(
            "reminder is already {}",
            match reminder.state {
                ReminderState::Enqueued => "enqueued",
                ReminderState::Cancelled => "cancelled",
                ReminderState::Scheduled => "being updated",
            }
        )))
    }

    pub(crate) async fn enqueue_due_reminders(&self, now: i64) -> Result<Vec<i64>, AppError> {
        let mut transaction = self.pool.begin().await?;
        let due: Vec<(i64, i64, String, i64)> = sqlx::query_as(
            "SELECT id, agent_id, message, deliver_at FROM agent_reminders \
             WHERE state = 'scheduled' AND deliver_at <= ? ORDER BY deliver_at, id",
        )
        .bind(now)
        .fetch_all(&mut *transaction)
        .await?;
        let mut agent_ids = Vec::new();
        for (id, agent_id, message, deliver_at) in due {
            let payload = serde_json::to_string(&serde_json::json!({
                "message": message,
                "requested_delivery_at": deliver_at,
            }))
            .map_err(|error| AppError::internal(format!("failed to encode reminder: {error}")))?;
            let key = format!("reminder:{id}");
            sqlx::query(
                "INSERT OR IGNORE INTO agent_wake_events \
                 (agent_id, kind, payload, deduplication_key, delivery_policy) \
                 VALUES (?, 'reminder', ?, ?, ?)",
            )
            .bind(agent_id)
            .bind(payload)
            .bind(&key)
            .bind(WakeDeliveryPolicy::Immediate)
            .execute(&mut *transaction)
            .await?;
            let wake_event_id: i64 = sqlx::query_scalar(
                "SELECT id FROM agent_wake_events WHERE agent_id = ? AND deduplication_key = ?",
            )
            .bind(agent_id)
            .bind(&key)
            .fetch_one(&mut *transaction)
            .await?;
            let updated = sqlx::query(
                "UPDATE agent_reminders SET state = 'enqueued', wake_event_id = ?, \
                 enqueued_at = ?, updated_at = ? WHERE id = ? AND state = 'scheduled'",
            )
            .bind(wake_event_id)
            .bind(now)
            .bind(now)
            .bind(id)
            .execute(&mut *transaction)
            .await?;
            if updated.rows_affected() == 1 && !agent_ids.contains(&agent_id) {
                agent_ids.push(agent_id);
            }
        }
        transaction.commit().await?;
        Ok(agent_ids)
    }

    async fn reminder(&self, agent_id: i64, id: i64) -> Result<Option<Reminder>, AppError> {
        Ok(sqlx::query_as(
            "SELECT id, agent_id, message, deliver_at, state, wake_event_id, enqueued_at, \
             cancelled_at, created_at, updated_at FROM agent_reminders \
             WHERE id = ? AND agent_id = ?",
        )
        .bind(id)
        .bind(agent_id)
        .fetch_optional(&self.pool)
        .await?)
    }
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    #[tokio::test]
    async fn reminders_cancel_or_enqueue_once() {
        let store = Store::memory().await.unwrap();
        let agent = store.create_agent("worker", "/tmp/worker").await.unwrap();
        let cancelled = store
            .create_reminder(agent.id, "Do not deliver", 101, 100)
            .await
            .unwrap();
        let cancelled = store.cancel_reminder(agent.id, cancelled.id).await.unwrap();
        assert_eq!(cancelled.state, ReminderState::Cancelled);
        assert!(store.cancel_reminder(agent.id, cancelled.id).await.is_err());

        let due = store
            .create_reminder(agent.id, "Check the build", 102, 100)
            .await
            .unwrap();
        assert!(store.enqueue_due_reminders(101).await.unwrap().is_empty());
        assert_eq!(
            store.enqueue_due_reminders(102).await.unwrap(),
            vec![agent.id]
        );
        assert!(store.enqueue_due_reminders(102).await.unwrap().is_empty());
        let reminders = store.reminders(agent.id).await.unwrap();
        assert_eq!(reminders[1].state, ReminderState::Enqueued);
        assert_eq!(reminders[1].id, due.id);
        assert!(store.cancel_reminder(agent.id, due.id).await.is_err());
        let events = store.wake_events(agent.id).await.unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind, "reminder");
        assert_eq!(events[0].delivery_policy, WakeDeliveryPolicy::Immediate);
    }

    #[tokio::test]
    async fn reminders_validate_owner_message_and_delivery_bounds() {
        let store = Store::memory().await.unwrap();
        let owner = store.create_agent("owner", "/tmp/owner").await.unwrap();
        let other = store.create_agent("other", "/tmp/other").await.unwrap();
        let reminder = store
            .create_reminder(owner.id, "Mine", 101, 100)
            .await
            .unwrap();
        assert!(store.cancel_reminder(other.id, reminder.id).await.is_err());
        assert!(
            store
                .create_reminder(owner.id, " ", 101, 100)
                .await
                .is_err()
        );
        assert!(
            store
                .create_reminder(owner.id, "late", 100, 100)
                .await
                .is_err()
        );
        assert!(
            store
                .create_reminder(
                    owner.id,
                    "distant",
                    100 + MAX_REMINDER_DELAY_SECONDS + 1,
                    100
                )
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn overdue_reminders_are_recovered_after_restart() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("mab-reminders-{unique}.sqlite3"));
        let store = Store::open(&path).await.unwrap();
        let agent = store.create_agent("worker", "/tmp/worker").await.unwrap();
        store
            .create_reminder(agent.id, "Survive restart", 101, 100)
            .await
            .unwrap();
        store.close().await;

        let reopened = Store::open(&path).await.unwrap();
        assert_eq!(
            reopened.enqueue_due_reminders(102).await.unwrap(),
            vec![agent.id]
        );
        assert_eq!(reopened.wake_events(agent.id).await.unwrap().len(), 1);
        reopened.close().await;
        std::fs::remove_file(path).unwrap();
    }
}
