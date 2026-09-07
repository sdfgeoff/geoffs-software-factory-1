use axum::{
    Json, Router,
    extract::{Path, State},
    http::HeaderMap,
    response::sse::{Event, KeepAlive, Sse},
    routing::{get, post},
};
use multi_agent_board_protocol as protocol;
use multi_agent_board_protocol::{
    AGENT_ID_HEADER, CreateAgentRequest, RenameAgentRequest, RequestChangesRequest,
    ReviewRevisionRequest, SendAgentMessageRequest, SetAgentChannelSubscriptionsRequest,
    SetAgentProjectSubscriptionsRequest, SetAgentScopesRequest, SetAgentWakeSettingsRequest,
};
use std::{convert::Infallible, time::Duration};

use crate::{agent_runtime::AgentManager, error::AppError};

pub fn router(manager: AgentManager) -> Router {
    Router::new()
        .route("/agents", get(list_agents).post(create_agent))
        .route("/agents/{id}", axum::routing::patch(rename_agent))
        .route(
            "/agents/{id}/project-worktrees",
            get(agent_project_worktrees),
        )
        .route(
            "/agents/{id}/messages",
            get(list_messages).post(send_message),
        )
        .route("/agents/{id}/pause", post(pause_agent))
        .route("/agents/{id}/resume", post(resume_agent))
        .route(
            "/agents/{id}/scopes",
            get(agent_scopes).put(set_agent_scopes),
        )
        .route(
            "/agents/{id}/wake-settings",
            axum::routing::put(set_agent_wake_settings),
        )
        .route(
            "/agents/{id}/channel-subscriptions",
            get(agent_channel_subscriptions).put(set_agent_channel_subscriptions),
        )
        .route(
            "/agents/{id}/project-subscriptions",
            get(agent_project_subscriptions).put(set_agent_project_subscriptions),
        )
        .route("/tickets/{id}/submit-review", post(submit_review))
        .route("/tickets/{id}/claim-review", post(claim_review))
        .route("/tickets/{id}/request-changes", post(request_changes))
        .route("/tickets/{id}/approve-review", post(approve_review))
        .route("/events", get(events))
        .with_state(manager)
}

async fn list_agents(
    State(manager): State<AgentManager>,
) -> Result<Json<Vec<protocol::Agent>>, AppError> {
    Ok(Json(
        manager.list().await?.into_iter().map(Into::into).collect(),
    ))
}

async fn create_agent(
    State(manager): State<AgentManager>,
    Json(input): Json<CreateAgentRequest>,
) -> Result<Json<protocol::Agent>, AppError> {
    Ok(Json(manager.create(&input.name).await?.into()))
}

async fn rename_agent(
    State(manager): State<AgentManager>,
    Path(id): Path<i64>,
    Json(input): Json<RenameAgentRequest>,
) -> Result<Json<protocol::Agent>, AppError> {
    Ok(Json(manager.rename(id, &input.name).await?.into()))
}

async fn agent_project_worktrees(
    State(manager): State<AgentManager>,
    Path(id): Path<i64>,
) -> Result<Json<Vec<protocol::AgentProjectWorktree>>, AppError> {
    Ok(Json(
        manager
            .project_worktrees(id)
            .await?
            .into_iter()
            .map(Into::into)
            .collect(),
    ))
}

async fn list_messages(
    State(manager): State<AgentManager>,
    Path(id): Path<i64>,
) -> Result<Json<Vec<protocol::AgentMessage>>, AppError> {
    Ok(Json(
        manager
            .messages(id)
            .await?
            .into_iter()
            .map(TryInto::try_into)
            .collect::<Result<_, _>>()
            .map_err(AppError::internal)?,
    ))
}

async fn send_message(
    State(manager): State<AgentManager>,
    Path(id): Path<i64>,
    Json(input): Json<SendAgentMessageRequest>,
) -> Result<Json<protocol::Agent>, AppError> {
    Ok(Json(manager.send_message(id, &input.message).await?.into()))
}

async fn pause_agent(
    State(manager): State<AgentManager>,
    Path(id): Path<i64>,
) -> Result<Json<protocol::Agent>, AppError> {
    Ok(Json(manager.pause(id).await?.into()))
}

async fn resume_agent(
    State(manager): State<AgentManager>,
    Path(id): Path<i64>,
) -> Result<Json<protocol::Agent>, AppError> {
    Ok(Json(manager.resume(id).await?.into()))
}

async fn agent_scopes(
    State(manager): State<AgentManager>,
    Path(id): Path<i64>,
) -> Result<Json<Vec<String>>, AppError> {
    Ok(Json(manager.scopes(id).await?))
}

async fn set_agent_scopes(
    State(manager): State<AgentManager>,
    Path(id): Path<i64>,
    Json(input): Json<SetAgentScopesRequest>,
) -> Result<Json<Vec<String>>, AppError> {
    Ok(Json(manager.set_scopes(id, &input.scopes).await?))
}

async fn set_agent_wake_settings(
    State(manager): State<AgentManager>,
    Path(id): Path<i64>,
    Json(input): Json<SetAgentWakeSettingsRequest>,
) -> Result<Json<protocol::Agent>, AppError> {
    Ok(Json(
        manager
            .set_wake_settings(
                id,
                input.wake_on_pending_work,
                input.wake_on_pending_review,
                input.idle_heartbeat_minutes,
            )
            .await?
            .into(),
    ))
}

async fn agent_channel_subscriptions(
    State(manager): State<AgentManager>,
    Path(id): Path<i64>,
) -> Result<Json<Vec<String>>, AppError> {
    Ok(Json(manager.channel_subscriptions(id).await?))
}

async fn set_agent_channel_subscriptions(
    State(manager): State<AgentManager>,
    Path(id): Path<i64>,
    Json(input): Json<SetAgentChannelSubscriptionsRequest>,
) -> Result<Json<Vec<String>>, AppError> {
    Ok(Json(
        manager
            .set_channel_subscriptions(id, &input.channels)
            .await?,
    ))
}

async fn agent_project_subscriptions(
    State(manager): State<AgentManager>,
    Path(id): Path<i64>,
) -> Result<Json<Vec<i64>>, AppError> {
    Ok(Json(manager.project_subscriptions(id).await?))
}

async fn set_agent_project_subscriptions(
    State(manager): State<AgentManager>,
    Path(id): Path<i64>,
    Json(input): Json<SetAgentProjectSubscriptionsRequest>,
) -> Result<Json<Vec<i64>>, AppError> {
    Ok(Json(
        manager
            .set_project_subscriptions(id, &input.project_ids)
            .await?,
    ))
}

async fn submit_review(
    State(manager): State<AgentManager>,
    Path(id): Path<i64>,
    headers: HeaderMap,
    Json(input): Json<ReviewRevisionRequest>,
) -> Result<Json<protocol::Ticket>, AppError> {
    Ok(Json(
        manager
            .submit_review(id, caller_agent(&headers)?, input.expected_revision)
            .await?
            .into(),
    ))
}

async fn claim_review(
    State(manager): State<AgentManager>,
    Path(id): Path<i64>,
    headers: HeaderMap,
    Json(input): Json<ReviewRevisionRequest>,
) -> Result<Json<protocol::TicketClaimResponse>, AppError> {
    Ok(Json(
        manager
            .claim_review_with_transition(id, caller_agent(&headers)?, input.expected_revision)
            .await?
            .into(),
    ))
}

async fn request_changes(
    State(manager): State<AgentManager>,
    Path(id): Path<i64>,
    headers: HeaderMap,
    Json(input): Json<RequestChangesRequest>,
) -> Result<Json<protocol::Ticket>, AppError> {
    Ok(Json(
        manager
            .request_changes(
                id,
                caller_agent(&headers)?,
                &input.feedback,
                input.expected_revision,
            )
            .await?
            .into(),
    ))
}

async fn approve_review(
    State(manager): State<AgentManager>,
    Path(id): Path<i64>,
    headers: HeaderMap,
    Json(input): Json<ReviewRevisionRequest>,
) -> Result<Json<protocol::Ticket>, AppError> {
    Ok(Json(
        manager
            .approve_review(id, caller_agent(&headers)?, input.expected_revision)
            .await?
            .into(),
    ))
}

fn caller_agent(headers: &HeaderMap) -> Result<i64, AppError> {
    headers
        .get(AGENT_ID_HEADER)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse().ok())
        .ok_or_else(|| AppError::forbidden("agent identity is required"))
}

async fn events(
    State(manager): State<AgentManager>,
) -> Sse<impl futures_core::Stream<Item = Result<Event, Infallible>>> {
    let mut receiver = manager.subscribe();
    let stream = async_stream::stream! {
        let mut refresh = tokio::time::interval(Duration::from_secs(2));
        loop {
            tokio::select! {
                event = receiver.recv() => match event {
                    Ok(event) => {
                        let event = protocol::AgentEvent::try_from(event)
                            .unwrap_or(protocol::AgentEvent::Refresh);
                        yield Ok(Event::default().json_data(event).unwrap());
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                        yield Ok(Event::default().json_data(protocol::AgentEvent::Refresh).unwrap());
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                },
                _ = refresh.tick() => yield Ok(Event::default().json_data(protocol::AgentEvent::Refresh).unwrap()),
            }
        }
    };
    Sse::new(stream).keep_alive(KeepAlive::default())
}

#[cfg(test)]
mod tests {
    use axum::{
        body::{Body, to_bytes},
        http::{Request, StatusCode, header},
    };
    use serde_json::Value;
    use tower::ServiceExt;

    use super::*;
    use crate::{domain::AgentState, store::Store};

    #[tokio::test]
    async fn agent_can_be_renamed_over_http() {
        let store = Store::memory().await.unwrap();
        let agent = store.create_agent("worker", "/tmp/worker").await.unwrap();
        let manager = AgentManager::new(
            store,
            "codex".into(),
            "multi-agent-board".into(),
            "http://127.0.0.1:1/api".into(),
        );

        let app = router(manager);
        let response = app
            .clone()
            .oneshot(
                Request::patch(format!("/agents/{}", agent.id))
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(r#"{"name":"builder"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body: Value =
            serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await.unwrap())
                .unwrap();
        assert_eq!(body["name"], "builder");
        assert_eq!(body["home_path"], "/tmp/worker.agent-home");

        let response = app
            .oneshot(
                Request::get(format!("/agents/{}/project-worktrees", agent.id))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body: Value =
            serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await.unwrap())
                .unwrap();
        assert_eq!(body[0]["agent_id"], agent.id);
        assert_eq!(body[0]["path"], "/tmp/worker");
    }

    #[tokio::test]
    async fn messages_sent_to_paused_agents_are_accepted_and_left_pending() {
        let store = Store::memory().await.unwrap();
        let agent = store.create_agent("worker", "/tmp/worker").await.unwrap();
        store
            .update_agent_runtime(agent.id, AgentState::Paused, None, None, None)
            .await
            .unwrap();
        let manager = AgentManager::new(
            store.clone(),
            "codex".into(),
            "multi-agent-board".into(),
            "http://127.0.0.1:1/api".into(),
        );

        let response = router(manager)
            .oneshot(
                Request::post(format!("/agents/{}/messages", agent.id))
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(r#"{"message":"Read this later"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body: Value =
            serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await.unwrap())
                .unwrap();
        assert_eq!(body["state"], "paused");
        assert_eq!(
            store.wake_events(agent.id).await.unwrap()[0].delivery_state,
            crate::domain::WakeEventState::Pending
        );
    }

    #[tokio::test]
    async fn wake_settings_are_validated_and_exposed_over_http() {
        let store = Store::memory().await.unwrap();
        let agent = store.create_agent("worker", "/tmp/worker").await.unwrap();
        assert!(!agent.wake_on_pending_work);
        assert!(!agent.wake_on_pending_review);
        assert_eq!(agent.idle_heartbeat_minutes, None);
        let manager = AgentManager::new(
            store,
            "codex".into(),
            "multi-agent-board".into(),
            "http://127.0.0.1:1/api".into(),
        );
        let app = router(manager);

        let response = app
            .clone()
            .oneshot(
                Request::put(format!("/agents/{}/wake-settings", agent.id))
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        r#"{"wake_on_pending_work":true,"wake_on_pending_review":false,"idle_heartbeat_minutes":20}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body: Value =
            serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await.unwrap())
                .unwrap();
        assert_eq!(body["wake_on_pending_work"], true);
        assert_eq!(body["wake_on_pending_review"], false);
        assert_eq!(body["idle_heartbeat_minutes"], 20);

        let response = app
            .clone()
            .oneshot(
                Request::put(format!("/agents/{}/wake-settings", agent.id))
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        r#"{"wake_on_pending_work":false,"wake_on_pending_review":true,"idle_heartbeat_minutes":null}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body: Value =
            serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await.unwrap())
                .unwrap();
        assert_eq!(body["idle_heartbeat_minutes"], Value::Null);
        assert_eq!(body["wake_on_pending_work"], false);
        assert_eq!(body["wake_on_pending_review"], true);

        let response = app
            .oneshot(
                Request::put(format!("/agents/{}/wake-settings", agent.id))
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        r#"{"wake_on_pending_work":false,"wake_on_pending_review":false,"idle_heartbeat_minutes":0}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn channel_subscriptions_are_configurable_over_http() {
        let store = Store::memory().await.unwrap();
        let agent = store.create_agent("worker", "/tmp/worker").await.unwrap();
        let manager = AgentManager::new(
            store,
            "codex".into(),
            "multi-agent-board".into(),
            "http://127.0.0.1:1/api".into(),
        );
        let app = router(manager);

        let response = app
            .clone()
            .oneshot(
                Request::put(format!("/agents/{}/channel-subscriptions", agent.id))
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(r#"{"channels":["general"]}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let response = app
            .oneshot(
                Request::get(format!("/agents/{}/channel-subscriptions", agent.id))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body: Value =
            serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await.unwrap())
                .unwrap();
        assert_eq!(body, serde_json::json!(["general"]));
    }

    #[tokio::test]
    async fn project_subscriptions_are_configurable_over_http() {
        let store = Store::memory().await.unwrap();
        let first = store.project().await.unwrap().unwrap();
        let second = store.set_project("/tmp/second-project").await.unwrap();
        let agent = store.create_agent("worker", "/tmp/worker").await.unwrap();
        let manager = AgentManager::new(
            store,
            "codex".into(),
            "multi-agent-board".into(),
            "http://127.0.0.1:1/api".into(),
        );
        let app = router(manager);

        let response = app
            .clone()
            .oneshot(
                Request::put(format!("/agents/{}/project-subscriptions", agent.id))
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(format!(
                        "{{\"project_ids\":[{},{}]}}",
                        first.id, second.id
                    )))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let response = app
            .oneshot(
                Request::get(format!("/agents/{}/project-subscriptions", agent.id))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body: Value =
            serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await.unwrap())
                .unwrap();
        assert_eq!(body, serde_json::json!([first.id, second.id]));
    }
}
