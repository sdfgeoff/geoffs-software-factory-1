use sqlx::{Sqlite, Transaction};

use crate::{
    domain::{Channel, ChannelMessageWakePayload, Message, WakeEventData},
    error::AppError,
};

use super::{Store, message_page_size};

pub(super) struct MessageWrite<'a> {
    pub(super) channel: &'a str,
    pub(super) agent_id: Option<i64>,
    pub(super) author: &'a str,
    pub(super) body: &'a str,
    pub(super) deduplication_key: Option<&'a str>,
}

impl Store {
    pub async fn channels(&self) -> Result<Vec<Channel>, sqlx::Error> {
        sqlx::query_as(
            "SELECT c.name, c.description, MAX(m.created_at) AS most_recent_message_at FROM channels c \
             LEFT JOIN messages m ON m.channel_name = c.name GROUP BY c.name ORDER BY c.name",
        )
        .fetch_all(&self.pool)
        .await
    }

    pub async fn channel(&self, name: &str) -> Result<Option<Channel>, AppError> {
        validate_channel(name)?;
        Ok(sqlx::query_as(
            "SELECT c.name, c.description, MAX(m.created_at) AS most_recent_message_at \
             FROM channels c LEFT JOIN messages m ON m.channel_name = c.name \
             WHERE c.name = ? GROUP BY c.name",
        )
        .bind(name)
        .fetch_optional(&self.pool)
        .await?)
    }

    pub async fn create_channel(&self, name: &str, description: &str) -> Result<Channel, AppError> {
        validate_channel(name)?;
        validate_channel_description(description)?;
        let result =
            sqlx::query("INSERT OR IGNORE INTO channels (name, description) VALUES (?, ?)")
                .bind(name)
                .bind(description)
                .execute(&self.pool)
                .await?;
        if result.rows_affected() == 0 {
            return Err(AppError::conflict("channel already exists"));
        }
        Ok(self.channel(name).await?.unwrap())
    }

    pub(crate) async fn ensure_channel(
        &self,
        name: &str,
        description: &str,
    ) -> Result<Channel, AppError> {
        validate_channel(name)?;
        validate_channel_description(description)?;
        sqlx::query("INSERT OR IGNORE INTO channels (name, description) VALUES (?, ?)")
            .bind(name)
            .bind(description)
            .execute(&self.pool)
            .await?;
        Ok(self.channel(name).await?.unwrap())
    }

    pub async fn messages(
        &self,
        channel: &str,
        before_id: Option<i64>,
        after_id: Option<i64>,
        limit: Option<i64>,
    ) -> Result<(Vec<Message>, bool), AppError> {
        validate_channel(channel)?;
        if !self.channel_exists(channel).await? {
            return Err(AppError::not_found("channel not found"));
        }
        let page_size = message_page_size(before_id, after_id, limit)?;
        let mut messages: Vec<Message> = if let Some(after_id) = after_id {
            sqlx::query_as(
                "SELECT id, channel_name, agent_id, author, body, created_at FROM messages \
                 WHERE channel_name = ? AND id > ? ORDER BY id LIMIT ?",
            )
            .bind(channel)
            .bind(after_id)
            .bind(page_size + 1)
            .fetch_all(&self.pool)
            .await?
        } else {
            sqlx::query_as(
                "SELECT id, channel_name, agent_id, author, body, created_at FROM messages \
                 WHERE channel_name = ? AND (? IS NULL OR id < ?) ORDER BY id DESC LIMIT ?",
            )
            .bind(channel)
            .bind(before_id)
            .bind(before_id)
            .bind(page_size + 1)
            .fetch_all(&self.pool)
            .await?
        };
        let has_more = messages.len() > page_size as usize;
        messages.truncate(page_size as usize);
        if after_id.is_none() {
            messages.reverse();
        }
        Ok((messages, has_more))
    }

    pub async fn post_message(
        &self,
        channel: &str,
        agent_id: Option<i64>,
        author: &str,
        body: &str,
    ) -> Result<Message, AppError> {
        Ok(self
            .post_message_with_key(channel, agent_id, author, body, None)
            .await?
            .0)
    }

    pub async fn post_message_once(
        &self,
        channel: &str,
        agent_id: Option<i64>,
        author: &str,
        body: &str,
        deduplication_key: &str,
    ) -> Result<Message, AppError> {
        if deduplication_key.is_empty() || deduplication_key.len() > 500 {
            return Err(AppError::bad_request(
                "message deduplication key must contain between 1 and 500 characters",
            ));
        }
        Ok(self
            .post_message_with_key(channel, agent_id, author, body, Some(deduplication_key))
            .await?
            .0)
    }

    pub async fn post_message_with_wakes(
        &self,
        channel: &str,
        agent_id: Option<i64>,
        author: &str,
        body: &str,
    ) -> Result<(Message, Vec<i64>), AppError> {
        self.post_message_with_key(channel, agent_id, author, body, None)
            .await
    }

    async fn post_message_with_key(
        &self,
        channel: &str,
        agent_id: Option<i64>,
        author: &str,
        body: &str,
        deduplication_key: Option<&str>,
    ) -> Result<(Message, Vec<i64>), AppError> {
        let mut transaction = self.pool.begin().await?;
        let message = insert_message(
            &mut transaction,
            MessageWrite {
                channel,
                agent_id,
                author,
                body,
                deduplication_key,
            },
        )
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

    pub async fn agent_channel_subscriptions(
        &self,
        agent_id: i64,
    ) -> Result<Vec<String>, AppError> {
        if self.agent(agent_id).await?.is_none() {
            return Err(AppError::not_found("agent not found"));
        }
        Ok(sqlx::query_scalar(
            "SELECT channel_name FROM agent_channel_subscriptions \
             WHERE agent_id = ? ORDER BY channel_name",
        )
        .bind(agent_id)
        .fetch_all(&self.pool)
        .await?)
    }

    pub async fn set_agent_channel_subscriptions(
        &self,
        agent_id: i64,
        channels: &[String],
    ) -> Result<Vec<String>, AppError> {
        let channels = self.valid_agent_channels(agent_id, channels).await?;
        let mut transaction = self.pool.begin().await?;
        sqlx::query("DELETE FROM agent_channel_subscriptions WHERE agent_id = ?")
            .bind(agent_id)
            .execute(&mut *transaction)
            .await?;
        for channel in &channels {
            sqlx::query(
                "INSERT INTO agent_channel_subscriptions (agent_id, channel_name) VALUES (?, ?)",
            )
            .bind(agent_id)
            .bind(channel)
            .execute(&mut *transaction)
            .await?;
        }
        transaction.commit().await?;
        Ok(channels)
    }

    pub async fn add_agent_channel_subscriptions(
        &self,
        agent_id: i64,
        channels: &[String],
    ) -> Result<Vec<String>, AppError> {
        let channels = self.valid_agent_channels(agent_id, channels).await?;
        let mut transaction = self.pool.begin().await?;
        for channel in channels {
            sqlx::query(
                "INSERT OR IGNORE INTO agent_channel_subscriptions (agent_id, channel_name) \
                 VALUES (?, ?)",
            )
            .bind(agent_id)
            .bind(channel)
            .execute(&mut *transaction)
            .await?;
        }
        transaction.commit().await?;
        self.agent_channel_subscriptions(agent_id).await
    }

    pub async fn remove_agent_channel_subscriptions(
        &self,
        agent_id: i64,
        channels: &[String],
    ) -> Result<Vec<String>, AppError> {
        let channels = self.valid_agent_channels(agent_id, channels).await?;
        let mut transaction = self.pool.begin().await?;
        for channel in channels {
            sqlx::query(
                "DELETE FROM agent_channel_subscriptions WHERE agent_id = ? AND channel_name = ?",
            )
            .bind(agent_id)
            .bind(channel)
            .execute(&mut *transaction)
            .await?;
        }
        transaction.commit().await?;
        self.agent_channel_subscriptions(agent_id).await
    }

    async fn valid_agent_channels(
        &self,
        agent_id: i64,
        channels: &[String],
    ) -> Result<Vec<String>, AppError> {
        if self.agent(agent_id).await?.is_none() {
            return Err(AppError::not_found("agent not found"));
        }
        let mut channels = channels.to_vec();
        channels.sort();
        channels.dedup();
        for channel in &channels {
            validate_channel(channel)?;
            if !self.channel_exists(channel).await? {
                return Err(AppError::bad_request(format!(
                    "channel #{channel} does not exist"
                )));
            }
        }
        Ok(channels)
    }
}

pub(super) async fn insert_message(
    transaction: &mut Transaction<'_, Sqlite>,
    message: MessageWrite<'_>,
) -> Result<(Message, Vec<i64>), AppError> {
    validate_channel(message.channel)?;
    validate_message(message.author, message.body)?;
    if message
        .deduplication_key
        .is_some_and(|key| key.is_empty() || key.len() > 500)
    {
        return Err(AppError::bad_request(
            "message deduplication key must contain between 1 and 500 characters",
        ));
    }
    let channel_exists: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM channels WHERE name = ?)")
            .bind(message.channel)
            .fetch_one(&mut **transaction)
            .await?;
    if !channel_exists {
        return Err(AppError::not_found(
            "channel not found; create it before posting",
        ));
    }
    let result = sqlx::query(
        "INSERT OR IGNORE INTO messages \
         (channel_name, agent_id, author, body, deduplication_key) VALUES (?, ?, ?, ?, ?)",
    )
    .bind(message.channel)
    .bind(message.agent_id)
    .bind(message.author.trim())
    .bind(message.body.trim())
    .bind(message.deduplication_key)
    .execute(&mut **transaction)
    .await?;
    let id = if result.rows_affected() == 1 {
        result.last_insert_rowid()
    } else {
        sqlx::query_scalar("SELECT id FROM messages WHERE deduplication_key = ?")
            .bind(message.deduplication_key)
            .fetch_one(&mut **transaction)
            .await?
    };
    let message: Message = sqlx::query_as(
        "SELECT id, channel_name, agent_id, author, body, created_at FROM messages WHERE id = ?",
    )
    .bind(id)
    .fetch_one(&mut **transaction)
    .await?;
    let agents: Vec<(i64, String, bool)> = sqlx::query_as(
        "SELECT a.id, a.name, EXISTS(SELECT 1 FROM agent_channel_subscriptions s \
         WHERE s.agent_id = a.id AND s.channel_name = ?) AS subscribed FROM agents a",
    )
    .bind(&message.channel_name)
    .fetch_all(&mut **transaction)
    .await?;
    let mut wake_agent_ids = Vec::new();
    for (target_id, name, subscribed) in agents {
        if message.agent_id == Some(target_id) {
            continue;
        }
        let mentioned = mentions_agent(&message.body, &name);
        if !subscribed && !mentioned {
            continue;
        }
        let prompt = format!(
            "A message was posted in #{} by {}:\n{}",
            message.channel_name, message.author, message.body
        );
        let event = WakeEventData::ChannelMessage(ChannelMessageWakePayload {
            message: prompt,
            channel: message.channel_name.clone(),
            message_id: message.id,
            author: message.author.clone(),
            body: message.body.clone(),
            mentioned,
            subscribed,
        });
        let payload = serde_json::to_string(&event.payload())
            .map_err(|error| AppError::internal(format!("failed to encode wake event: {error}")))?;
        sqlx::query(
            "INSERT OR IGNORE INTO agent_wake_events \
             (agent_id, kind, payload, deduplication_key, delivery_policy) \
             VALUES (?, ?, ?, ?, 'immediate')",
        )
        .bind(target_id)
        .bind(event.kind())
        .bind(payload)
        .bind(format!("channel-message:{}", message.id))
        .execute(&mut **transaction)
        .await?;
        wake_agent_ids.push(target_id);
    }
    Ok((message, wake_agent_ids))
}

pub(crate) fn validate_channel(channel: &str) -> Result<(), AppError> {
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

fn validate_channel_description(description: &str) -> Result<(), AppError> {
    if description.chars().count() > 500 {
        return Err(AppError::bad_request(
            "channel description must contain at most 500 characters",
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

fn mentions_agent(body: &str, name: &str) -> bool {
    let mention = format!("@{name}");
    body.split(|character: char| {
        !(character.is_ascii_alphanumeric() || matches!(character, '@' | '-' | '_'))
    })
    .any(|token| token == mention)
}

#[cfg(test)]
mod tests {
    use sqlx::SqlitePool;

    use super::*;

    #[tokio::test]
    async fn message_pages_are_cursor_ordered_and_capped() {
        let store = Store::memory().await.unwrap();
        let mut inserted = Vec::new();
        for number in 1..=5 {
            inserted.push(
                store
                    .post_message("general", None, "human", &format!("Message {number}"))
                    .await
                    .unwrap(),
            );
        }

        let (newest, has_more) = store
            .messages("general", None, None, Some(2))
            .await
            .unwrap();
        assert_eq!(
            newest.iter().map(|message| message.id).collect::<Vec<_>>(),
            vec![inserted[3].id, inserted[4].id]
        );
        assert!(has_more);

        let (older, has_more) = store
            .messages("general", Some(inserted[3].id), None, Some(2))
            .await
            .unwrap();
        assert_eq!(
            older.iter().map(|message| message.id).collect::<Vec<_>>(),
            vec![inserted[1].id, inserted[2].id]
        );
        assert!(has_more);

        let (newer, has_more) = store
            .messages("general", None, Some(inserted[1].id), Some(2))
            .await
            .unwrap();
        assert_eq!(
            newer.iter().map(|message| message.id).collect::<Vec<_>>(),
            vec![inserted[2].id, inserted[3].id]
        );
        assert!(has_more);
        assert!(
            store
                .messages("general", Some(1), Some(2), None)
                .await
                .is_err()
        );
        assert!(
            store
                .messages("general", None, None, Some(101))
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn channel_description_migration_preserves_existing_channels() {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        sqlx::query("CREATE TABLE channels (name TEXT PRIMARY KEY)")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO channels (name) VALUES ('general')")
            .execute(&pool)
            .await
            .unwrap();

        sqlx::raw_sql(include_str!(
            "../../migrations/0019_channel_descriptions.sql"
        ))
        .execute(&pool)
        .await
        .unwrap();

        let description: String =
            sqlx::query_scalar("SELECT description FROM channels WHERE name = 'general'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(description, "");
    }

    #[tokio::test]
    async fn channels_are_created_explicitly_with_descriptions() {
        let store = Store::memory().await.unwrap();
        let channel = store
            .create_channel("backend", "Backend coordination")
            .await
            .unwrap();
        assert_eq!(channel.description, "Backend coordination");
        assert!(
            store
                .messages("backend", None, None, None)
                .await
                .unwrap()
                .0
                .is_empty()
        );
        store
            .post_message("backend", None, "Ada", "Schema is ready")
            .await
            .unwrap();

        assert_eq!(store.channels().await.unwrap().len(), 2);
        assert_eq!(
            store
                .messages("backend", None, None, None)
                .await
                .unwrap()
                .0
                .first()
                .unwrap()
                .body,
            "Schema is ready"
        );
        assert!(store.create_channel("backend", "Duplicate").await.is_err());
        assert!(store.create_channel("bad name", "Invalid").await.is_err());
        assert!(
            store
                .create_channel("too-long", &"x".repeat(501))
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn posting_requires_an_existing_channel() {
        let store = Store::memory().await.unwrap();
        let error = store
            .post_message("missing", None, "Ada", "Schema is ready")
            .await
            .unwrap_err();

        assert!(error.to_string().contains("create it before posting"));
    }

    #[tokio::test]
    async fn idempotent_messages_are_inserted_once() {
        let store = Store::memory().await.unwrap();
        store.create_channel("merges", "").await.unwrap();

        let first = store
            .post_message_once("merges", None, "board", "Merged", "merge:1")
            .await
            .unwrap();
        let retry = store
            .post_message_once("merges", None, "board", "Merged", "merge:1")
            .await
            .unwrap();

        assert_eq!(retry.id, first.id);
        assert_eq!(
            store
                .messages("merges", None, None, None)
                .await
                .unwrap()
                .0
                .len(),
            1
        );
    }

    #[tokio::test]
    async fn channel_posts_wake_subscribers_and_exact_mentions() {
        let store = Store::memory().await.unwrap();
        let subscriber = store
            .create_agent("subscriber", "/tmp/subscriber")
            .await
            .unwrap();
        let mentioned = store
            .create_agent("mentioned", "/tmp/mentioned")
            .await
            .unwrap();
        let partial = store.create_agent("mention", "/tmp/mention").await.unwrap();
        store
            .set_agent_project_subscriptions(mentioned.id, &[])
            .await
            .unwrap();
        store
            .set_agent_channel_subscriptions(subscriber.id, &["general".into()])
            .await
            .unwrap();

        let (message, mut targets) = store
            .post_message_with_wakes("general", None, "human", "Please check this, @mentioned.")
            .await
            .unwrap();
        targets.sort();

        assert_eq!(targets, vec![subscriber.id, mentioned.id]);
        assert!(store.wake_events(partial.id).await.unwrap().is_empty());
        let subscriber_events = store.wake_events(subscriber.id).await.unwrap();
        let subscriber_event = &subscriber_events[0];
        assert_eq!(subscriber_event.kind(), "channel.message");
        assert_eq!(subscriber_event.payload()["channel"], "general");
        assert_eq!(subscriber_event.payload()["message_id"], message.id);
        assert_eq!(subscriber_event.payload()["subscribed"], true);
        let mention_events = store.wake_events(mentioned.id).await.unwrap();
        let mention_event = &mention_events[0];
        assert_eq!(mention_event.payload()["mentioned"], true);
    }

    #[tokio::test]
    async fn agent_can_be_renamed() {
        let store = Store::memory().await.unwrap();
        let agent = store.create_agent("human", "/tmp/worker").await.unwrap();
        store
            .post_message("general", None, "human", "Unrelated")
            .await
            .unwrap();
        store
            .post_message("general", Some(agent.id), "human", "Finished")
            .await
            .unwrap();

        let renamed = store.rename_agent(agent.id, "builder").await.unwrap();

        assert_eq!(renamed.name, "builder");
        assert_eq!(renamed.home_path, "/tmp/worker.agent-home");
        let (messages, _) = store.messages("general", None, None, None).await.unwrap();
        assert_eq!(messages[0].author, "human");
        assert_eq!(messages[0].agent_id, None);
        assert_eq!(messages[1].author, "human");
        assert_eq!(messages[1].agent_id, Some(agent.id));
    }
}
