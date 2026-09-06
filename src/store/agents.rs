use crate::{
    domain::{Agent, AgentMessage, AgentProjectWorktree, AgentState},
    error::AppError,
};

use super::Store;

impl Store {
    pub async fn agent_scopes(&self, agent_id: i64) -> Result<Vec<String>, AppError> {
        if self.agent(agent_id).await?.is_none() {
            return Err(AppError::not_found("agent not found"));
        }
        Ok(
            sqlx::query_scalar("SELECT scope FROM agent_scopes WHERE agent_id = ? ORDER BY scope")
                .bind(agent_id)
                .fetch_all(&self.pool)
                .await?,
        )
    }

    pub async fn set_agent_scopes(
        &self,
        agent_id: i64,
        scopes: &[String],
    ) -> Result<Vec<String>, AppError> {
        if self.agent(agent_id).await?.is_none() {
            return Err(AppError::not_found("agent not found"));
        }
        for scope in scopes {
            validate_scope(scope)?;
        }
        let mut transaction = self.pool.begin().await?;
        sqlx::query("DELETE FROM agent_scopes WHERE agent_id = ?")
            .bind(agent_id)
            .execute(&mut *transaction)
            .await?;
        for scope in scopes {
            sqlx::query("INSERT OR IGNORE INTO agent_scopes (agent_id, scope) VALUES (?, ?)")
                .bind(agent_id)
                .bind(scope)
                .execute(&mut *transaction)
                .await?;
        }
        transaction.commit().await?;
        self.agent_scopes(agent_id).await
    }

    pub async fn has_agent_scope(&self, agent_id: i64, scope: &str) -> Result<bool, sqlx::Error> {
        sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM agent_scopes WHERE agent_id = ? AND scope = ?)",
        )
        .bind(agent_id)
        .bind(scope)
        .fetch_one(&self.pool)
        .await
    }

    pub async fn agents_with_scope(
        &self,
        scope: &str,
        exclude_agent_id: i64,
    ) -> Result<Vec<Agent>, sqlx::Error> {
        sqlx::query_as(
            "SELECT a.id, a.name, a.state, a.thread_id, a.home_path, a.active_turn_id, \
             a.last_error, a.wake_on_pending_work, a.wake_on_pending_review, a.idle_heartbeat_minutes, \
             a.idle_since, a.created_at, a.updated_at FROM agents a \
             JOIN agent_scopes s ON s.agent_id = a.id \
             WHERE s.scope = ? AND a.id != ? AND a.state != 'paused' \
             ORDER BY CASE a.state WHEN 'idle' THEN 0 WHEN 'error' THEN 1 ELSE 2 END, a.id",
        )
        .bind(scope)
        .bind(exclude_agent_id)
        .fetch_all(&self.pool)
        .await
    }

    pub async fn agents(&self) -> Result<Vec<Agent>, sqlx::Error> {
        sqlx::query_as(
            "SELECT id, name, state, thread_id, home_path, active_turn_id, last_error, \
             wake_on_pending_work, wake_on_pending_review, idle_heartbeat_minutes, idle_since, created_at, updated_at \
             FROM agents ORDER BY id",
        )
        .fetch_all(&self.pool)
        .await
    }

    pub async fn agents_due_for_idle_heartbeat(&self, now: i64) -> Result<Vec<Agent>, sqlx::Error> {
        sqlx::query_as(
            "SELECT id, name, state, thread_id, home_path, active_turn_id, last_error, \
             wake_on_pending_work, wake_on_pending_review, idle_heartbeat_minutes, idle_since, created_at, updated_at \
             FROM agents WHERE state = 'idle' AND idle_heartbeat_minutes IS NOT NULL \
             AND idle_since IS NOT NULL AND idle_since + idle_heartbeat_minutes * 60 <= ? \
             ORDER BY id",
        )
        .bind(now)
        .fetch_all(&self.pool)
        .await
    }

    pub async fn agent(&self, id: i64) -> Result<Option<Agent>, sqlx::Error> {
        sqlx::query_as(
            "SELECT id, name, state, thread_id, home_path, active_turn_id, last_error, \
             wake_on_pending_work, wake_on_pending_review, idle_heartbeat_minutes, idle_since, created_at, updated_at \
             FROM agents WHERE id = ?",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
    }

    pub async fn agent_named(&self, name: &str) -> Result<Option<Agent>, sqlx::Error> {
        sqlx::query_as(
            "SELECT id, name, state, thread_id, home_path, active_turn_id, last_error, \
             wake_on_pending_work, wake_on_pending_review, idle_heartbeat_minutes, idle_since, created_at, updated_at \
             FROM agents WHERE name = ?",
        )
        .bind(name)
        .fetch_optional(&self.pool)
        .await
    }

    pub async fn create_agent(&self, name: &str, legacy_worktree: &str) -> Result<Agent, AppError> {
        let mut transaction = self.pool.begin().await?;
        let home = format!("{legacy_worktree}.agent-home");
        let result = sqlx::query(
            "INSERT INTO agents (name, home_path, idle_since) VALUES (?, ?, unixepoch())",
        )
        .bind(name)
        .bind(home)
        .execute(&mut *transaction)
        .await?;
        let id = result.last_insert_rowid();
        let current_project_id: Option<i64> =
            sqlx::query_scalar("SELECT current_project_id FROM board_state WHERE id = 1")
                .fetch_optional(&mut *transaction)
                .await?
                .flatten();
        if let Some(project_id) = current_project_id {
            sqlx::query(
                "INSERT INTO agent_project_worktrees (agent_id, project_id, path) VALUES (?, ?, ?)",
            )
            .bind(id)
            .bind(project_id)
            .bind(legacy_worktree)
            .execute(&mut *transaction)
            .await?;
        }
        transaction.commit().await?;
        Ok(self.agent(id).await?.unwrap())
    }

    pub async fn create_agent_for_project(
        &self,
        name: &str,
        homes_root: &str,
        project_id: i64,
    ) -> Result<(Agent, AgentProjectWorktree), AppError> {
        let mut transaction = self.pool.begin().await?;
        let project_is_active: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM projects WHERE id = ? AND archived_at IS NULL)",
        )
        .bind(project_id)
        .fetch_one(&mut *transaction)
        .await?;
        if !project_is_active {
            return Err(AppError::conflict("agent project is missing or archived"));
        }
        let result = sqlx::query(
            "INSERT INTO agents (name, home_path, idle_since) \
             VALUES (?, 'pending:' || lower(hex(randomblob(16))), unixepoch())",
        )
        .bind(name)
        .execute(&mut *transaction)
        .await?;
        let agent_id = result.last_insert_rowid();
        let home = format!("{homes_root}/{agent_id}");
        let worktree_path = format!("{home}/projects/{project_id}");
        sqlx::query("UPDATE agents SET home_path = ? WHERE id = ?")
            .bind(&home)
            .bind(agent_id)
            .execute(&mut *transaction)
            .await?;
        sqlx::query(
            "INSERT INTO agent_project_worktrees (agent_id, project_id, path) VALUES (?, ?, ?)",
        )
        .bind(agent_id)
        .bind(project_id)
        .bind(&worktree_path)
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        Ok((
            self.agent(agent_id).await?.unwrap(),
            self.agent_project_worktree(agent_id, project_id)
                .await?
                .unwrap(),
        ))
    }

    pub async fn agent_project_worktree(
        &self,
        agent_id: i64,
        project_id: i64,
    ) -> Result<Option<AgentProjectWorktree>, sqlx::Error> {
        let row: Option<(i64, i64, String, i64, i64)> = sqlx::query_as(
            "SELECT agent_id, project_id, path, created_at, updated_at \
             FROM agent_project_worktrees WHERE agent_id = ? AND project_id = ?",
        )
        .bind(agent_id)
        .bind(project_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(worktree_from_row))
    }

    pub async fn agent_project_worktrees(
        &self,
        agent_id: i64,
    ) -> Result<Vec<AgentProjectWorktree>, AppError> {
        if self.agent(agent_id).await?.is_none() {
            return Err(AppError::not_found("agent not found"));
        }
        let rows: Vec<(i64, i64, String, i64, i64)> = sqlx::query_as(
            "SELECT agent_id, project_id, path, created_at, updated_at \
             FROM agent_project_worktrees WHERE agent_id = ? ORDER BY project_id",
        )
        .bind(agent_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().map(worktree_from_row).collect())
    }

    pub(crate) async fn delete_agent(&self, id: i64) -> Result<(), sqlx::Error> {
        sqlx::query("DELETE FROM agents WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn rename_agent(&self, id: i64, name: &str) -> Result<Agent, AppError> {
        let result =
            sqlx::query("UPDATE agents SET name = ?, updated_at = unixepoch() WHERE id = ?")
                .bind(name)
                .bind(id)
                .execute(&self.pool)
                .await?;
        if result.rows_affected() == 0 {
            return Err(AppError::not_found("agent not found"));
        }
        Ok(self.agent(id).await?.unwrap())
    }

    pub async fn set_agent_wake_settings(
        &self,
        id: i64,
        wake_on_pending_work: bool,
        wake_on_pending_review: bool,
        idle_heartbeat_minutes: Option<i64>,
    ) -> Result<Agent, AppError> {
        if idle_heartbeat_minutes.is_some_and(|minutes| !(1..=10_080).contains(&minutes)) {
            return Err(AppError::bad_request(
                "idle heartbeat must be between 1 and 10080 minutes",
            ));
        }
        let result = sqlx::query(
            "UPDATE agents SET wake_on_pending_work = ?, wake_on_pending_review = ?, idle_heartbeat_minutes = ?, \
             updated_at = unixepoch() WHERE id = ?",
        )
        .bind(wake_on_pending_work)
        .bind(wake_on_pending_review)
        .bind(idle_heartbeat_minutes)
        .bind(id)
        .execute(&self.pool)
        .await?;
        if result.rows_affected() == 0 {
            return Err(AppError::not_found("agent not found"));
        }
        Ok(self.agent(id).await?.unwrap())
    }

    pub async fn update_agent_runtime(
        &self,
        id: i64,
        state: AgentState,
        thread_id: Option<&str>,
        active_turn_id: Option<&str>,
        last_error: Option<&str>,
    ) -> Result<Agent, AppError> {
        let result = sqlx::query(
            "UPDATE agents SET state = ?, thread_id = COALESCE(?, thread_id), \
             active_turn_id = ?, last_error = ?, \
             idle_since = CASE WHEN ? = 'idle' THEN \
                 CASE WHEN state = 'idle' THEN idle_since ELSE unixepoch() END \
                 ELSE NULL END, updated_at = unixepoch() WHERE id = ?",
        )
        .bind(state)
        .bind(thread_id)
        .bind(active_turn_id)
        .bind(last_error)
        .bind(state)
        .bind(id)
        .execute(&self.pool)
        .await?;
        if result.rows_affected() == 0 {
            return Err(AppError::not_found("agent not found"));
        }
        Ok(self.agent(id).await?.unwrap())
    }

    pub async fn finish_agent_turn(
        &self,
        id: i64,
        turn_id: &str,
        state: AgentState,
        last_error: Option<&str>,
    ) -> Result<Option<Agent>, AppError> {
        let result = sqlx::query(
            "UPDATE agents SET state = ?, active_turn_id = NULL, last_error = ?, \
             idle_since = CASE WHEN ? = 'idle' THEN unixepoch() ELSE NULL END, \
             updated_at = unixepoch() WHERE id = ? AND active_turn_id = ?",
        )
        .bind(state)
        .bind(last_error)
        .bind(state)
        .bind(id)
        .bind(turn_id)
        .execute(&self.pool)
        .await?;
        if result.rows_affected() == 0 {
            return Ok(None);
        }
        Ok(self.agent(id).await?)
    }

    pub async fn agent_messages(&self, agent_id: i64) -> Result<Vec<AgentMessage>, AppError> {
        if self.agent(agent_id).await?.is_none() {
            return Err(AppError::not_found("agent not found"));
        }
        Ok(sqlx::query_as(
            "SELECT id, agent_id, role, body, created_at FROM agent_messages \
             WHERE agent_id = ? ORDER BY created_at, id",
        )
        .bind(agent_id)
        .fetch_all(&self.pool)
        .await?)
    }

    pub async fn add_agent_message(
        &self,
        agent_id: i64,
        role: &str,
        body: &str,
    ) -> Result<AgentMessage, AppError> {
        if body.trim().is_empty() || body.chars().count() > 50_000 {
            return Err(AppError::bad_request(
                "agent message must contain between 1 and 50000 characters",
            ));
        }
        let result =
            sqlx::query("INSERT INTO agent_messages (agent_id, role, body) VALUES (?, ?, ?)")
                .bind(agent_id)
                .bind(role)
                .bind(body.trim())
                .execute(&self.pool)
                .await?;
        Ok(sqlx::query_as(
            "SELECT id, agent_id, role, body, created_at FROM agent_messages WHERE id = ?",
        )
        .bind(result.last_insert_rowid())
        .fetch_one(&self.pool)
        .await?)
    }

    pub async fn enqueue_agent_message(
        &self,
        agent_id: i64,
        body: &str,
    ) -> Result<AgentMessage, AppError> {
        if body.trim().is_empty() || body.chars().count() > 50_000 {
            return Err(AppError::bad_request(
                "agent message must contain between 1 and 50000 characters",
            ));
        }
        let mut transaction = self.pool.begin().await?;
        let result =
            sqlx::query("INSERT INTO agent_messages (agent_id, role, body) VALUES (?, 'user', ?)")
                .bind(agent_id)
                .bind(body.trim())
                .execute(&mut *transaction)
                .await?;
        let message: AgentMessage = sqlx::query_as(
            "SELECT id, agent_id, role, body, created_at FROM agent_messages WHERE id = ?",
        )
        .bind(result.last_insert_rowid())
        .fetch_one(&mut *transaction)
        .await?;
        let payload = serde_json::json!({"message": &message.body, "message_id": message.id});
        sqlx::query(
            "INSERT INTO agent_wake_events (agent_id, kind, payload, delivery_policy) \
             VALUES (?, 'message', ?, 'immediate')",
        )
        .bind(agent_id)
        .bind(payload.to_string())
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        Ok(message)
    }

    pub async fn recover_agents_after_restart(&self) -> Result<u64, sqlx::Error> {
        Ok(sqlx::query(
            "UPDATE agents SET state = 'error', active_turn_id = NULL, \
             last_error = 'The board restarted during an active turn; send a message to retry.', \
             updated_at = unixepoch() WHERE state IN ('running', 'awaiting_approval')",
        )
        .execute(&self.pool)
        .await?
        .rows_affected())
    }
}

fn worktree_from_row(row: (i64, i64, String, i64, i64)) -> AgentProjectWorktree {
    AgentProjectWorktree {
        agent_id: row.0,
        project_id: row.1,
        path: row.2,
        created_at: row.3,
        updated_at: row.4,
    }
}

fn validate_scope(scope: &str) -> Result<(), AppError> {
    if scope.is_empty()
        || scope.len() > 100
        || !scope.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.' | b':' | b'/')
        })
    {
        return Err(AppError::bad_request("invalid agent scope"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use sqlx::SqlitePool;

    use super::*;

    #[tokio::test]
    async fn legacy_worktree_migration_preserves_the_checkout_mapping() {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        let migrator = sqlx::migrate!();
        for migration in migrator.iter().filter(|migration| migration.version < 20) {
            sqlx::raw_sql(&migration.sql).execute(&pool).await.unwrap();
        }
        sqlx::query("INSERT INTO projects (id, path) VALUES (1, '/repo')")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("UPDATE board_state SET current_project_id = 1 WHERE id = 1")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO agents (id, name, worktree_path) VALUES (7, 'legacy', '/dirty-checkout')",
        )
        .execute(&pool)
        .await
        .unwrap();
        let migration = migrator
            .iter()
            .find(|migration| migration.version == 20)
            .unwrap();
        sqlx::raw_sql(&migration.sql).execute(&pool).await.unwrap();

        let agent: (String, String) = sqlx::query_as(
            "SELECT agents.home_path, worktrees.path FROM agents \
             JOIN agent_project_worktrees worktrees ON worktrees.agent_id = agents.id \
             WHERE agents.id = 7 AND worktrees.project_id = 1",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(agent.0, "/dirty-checkout.agent-home");
        assert_eq!(agent.1, "/dirty-checkout");
    }

    #[tokio::test]
    async fn worktree_pairs_are_unique_and_survive_agent_renames() {
        let store = Store::memory().await.unwrap();
        let project = store.project().await.unwrap().unwrap();
        let agent = store.create_agent("worker", "/tmp/worker").await.unwrap();
        let before = store
            .agent_project_worktree(agent.id, project.id)
            .await
            .unwrap()
            .unwrap();

        let renamed = store.rename_agent(agent.id, "renamed").await.unwrap();
        let after = store
            .agent_project_worktree(agent.id, project.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(renamed.home_path, agent.home_path);
        assert_eq!(after, before);
        assert!(
            sqlx::query(
                "INSERT INTO agent_project_worktrees (agent_id, project_id, path) \
                 VALUES (?, ?, '/tmp/duplicate')",
            )
            .bind(agent.id)
            .bind(project.id)
            .execute(&store.pool)
            .await
            .is_err()
        );
    }

    #[tokio::test]
    async fn separate_wake_settings_migration_preserves_existing_preferences() {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        sqlx::query(
            "CREATE TABLE agents (id INTEGER PRIMARY KEY, wake_on_pending_task INTEGER NOT NULL)",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query("INSERT INTO agents VALUES (1, 1), (2, 0)")
            .execute(&pool)
            .await
            .unwrap();

        sqlx::raw_sql(include_str!(
            "../../migrations/0013_separate_ticket_wake_settings.sql"
        ))
        .execute(&pool)
        .await
        .unwrap();

        let settings: Vec<(bool, bool, bool)> = sqlx::query_as(
            "SELECT wake_on_pending_task, wake_on_pending_work, wake_on_pending_review \
             FROM agents ORDER BY id",
        )
        .fetch_all(&pool)
        .await
        .unwrap();
        assert_eq!(settings, vec![(true, true, true), (false, false, false)]);
    }

    #[tokio::test]
    async fn interrupted_agents_are_recovered_as_errors() {
        let store = Store::memory().await.unwrap();
        let agent = store.create_agent("worker", "/tmp/worker").await.unwrap();
        store
            .update_agent_runtime(
                agent.id,
                AgentState::Running,
                Some("thread-1"),
                Some("turn-1"),
                None,
            )
            .await
            .unwrap();

        assert_eq!(store.recover_agents_after_restart().await.unwrap(), 1);
        let recovered = store.agent(agent.id).await.unwrap().unwrap();
        assert_eq!(recovered.state, AgentState::Error);
        assert!(recovered.active_turn_id.is_none());
        assert!(recovered.thread_id.is_some());
    }

    #[tokio::test]
    async fn idle_heartbeat_timing_resets_on_each_idle_period() {
        let store = Store::memory().await.unwrap();
        let agent = store.create_agent("worker", "/tmp/worker").await.unwrap();
        let first_idle_since = agent.idle_since.unwrap();
        assert!(
            store
                .agents_due_for_idle_heartbeat(i64::MAX)
                .await
                .unwrap()
                .is_empty()
        );
        store
            .set_agent_wake_settings(agent.id, false, false, Some(20))
            .await
            .unwrap();

        assert!(
            store
                .agents_due_for_idle_heartbeat(first_idle_since + 1_199)
                .await
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            store
                .agents_due_for_idle_heartbeat(first_idle_since + 1_200)
                .await
                .unwrap()[0]
                .id,
            agent.id
        );

        store
            .update_agent_runtime(
                agent.id,
                AgentState::Running,
                Some("thread-1"),
                Some("turn-1"),
                None,
            )
            .await
            .unwrap();
        assert!(
            store
                .agent(agent.id)
                .await
                .unwrap()
                .unwrap()
                .idle_since
                .is_none()
        );
        let idle = store
            .finish_agent_turn(agent.id, "turn-1", AgentState::Idle, None)
            .await
            .unwrap()
            .unwrap();
        let next_idle_since = idle.idle_since.unwrap();
        assert!(
            store
                .agents_due_for_idle_heartbeat(next_idle_since + 1_199)
                .await
                .unwrap()
                .is_empty()
        );

        for state in [AgentState::Paused, AgentState::Running, AgentState::Error] {
            store
                .update_agent_runtime(agent.id, state, None, None, None)
                .await
                .unwrap();
            assert!(
                store
                    .agents_due_for_idle_heartbeat(i64::MAX)
                    .await
                    .unwrap()
                    .is_empty()
            );
        }
    }
}
