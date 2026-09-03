use axum::{
    Json, Router,
    extract::{Path, State},
    http::HeaderMap,
    response::sse::{Event, KeepAlive, Sse},
    routing::{get, post},
};
use multi_agent_board_protocol::{
    AGENT_ID_HEADER, CreateAgentRequest, RenameAgentRequest, RequestChangesRequest,
    ReviewRevisionRequest, SendAgentMessageRequest, SetAgentScopesRequest,
    SetAgentWakeSettingsRequest,
};
use std::{convert::Infallible, time::Duration};

use crate::{
    agent_runtime::AgentManager,
    domain::{Agent, AgentMessage},
    error::AppError,
};

pub fn router(manager: AgentManager) -> Router {
    Router::new()
        .route("/agents", get(list_agents).post(create_agent))
        .route("/agents/{id}", axum::routing::patch(rename_agent))
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
        .route("/tickets/{id}/submit-review", post(submit_review))
        .route("/tickets/{id}/claim-review", post(claim_review))
        .route("/tickets/{id}/request-changes", post(request_changes))
        .route("/tickets/{id}/approve-review", post(approve_review))
        .route("/events", get(events))
        .with_state(manager)
}

async fn list_agents(State(manager): State<AgentManager>) -> Result<Json<Vec<Agent>>, AppError> {
    Ok(Json(manager.list().await?))
}

async fn create_agent(
    State(manager): State<AgentManager>,
    Json(input): Json<CreateAgentRequest>,
) -> Result<Json<Agent>, AppError> {
    Ok(Json(manager.create(&input.name).await?))
}

async fn rename_agent(
    State(manager): State<AgentManager>,
    Path(id): Path<i64>,
    Json(input): Json<RenameAgentRequest>,
) -> Result<Json<Agent>, AppError> {
    Ok(Json(manager.rename(id, &input.name).await?))
}

async fn list_messages(
    State(manager): State<AgentManager>,
    Path(id): Path<i64>,
) -> Result<Json<Vec<AgentMessage>>, AppError> {
    Ok(Json(manager.messages(id).await?))
}

async fn send_message(
    State(manager): State<AgentManager>,
    Path(id): Path<i64>,
    Json(input): Json<SendAgentMessageRequest>,
) -> Result<Json<Agent>, AppError> {
    Ok(Json(manager.send_message(id, &input.message).await?))
}

async fn pause_agent(
    State(manager): State<AgentManager>,
    Path(id): Path<i64>,
) -> Result<Json<Agent>, AppError> {
    Ok(Json(manager.pause(id).await?))
}

async fn resume_agent(
    State(manager): State<AgentManager>,
    Path(id): Path<i64>,
) -> Result<Json<Agent>, AppError> {
    Ok(Json(manager.resume(id).await?))
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
) -> Result<Json<Agent>, AppError> {
    Ok(Json(
        manager
            .set_wake_settings(id, input.wake_on_pending_task, input.idle_heartbeat_minutes)
            .await?,
    ))
}

async fn submit_review(
    State(manager): State<AgentManager>,
    Path(id): Path<i64>,
    headers: HeaderMap,
    Json(input): Json<ReviewRevisionRequest>,
) -> Result<Json<crate::domain::Ticket>, AppError> {
    Ok(Json(
        manager
            .submit_review(id, caller_agent(&headers)?, input.expected_revision)
            .await?,
    ))
}

async fn claim_review(
    State(manager): State<AgentManager>,
    Path(id): Path<i64>,
    headers: HeaderMap,
    Json(input): Json<ReviewRevisionRequest>,
) -> Result<Json<crate::domain::Ticket>, AppError> {
    Ok(Json(
        manager
            .claim_review(id, caller_agent(&headers)?, input.expected_revision)
            .await?,
    ))
}

async fn request_changes(
    State(manager): State<AgentManager>,
    Path(id): Path<i64>,
    headers: HeaderMap,
    Json(input): Json<RequestChangesRequest>,
) -> Result<Json<crate::domain::Ticket>, AppError> {
    Ok(Json(
        manager
            .request_changes(
                id,
                caller_agent(&headers)?,
                &input.feedback,
                input.expected_revision,
            )
            .await?,
    ))
}

async fn approve_review(
    State(manager): State<AgentManager>,
    Path(id): Path<i64>,
    headers: HeaderMap,
    Json(input): Json<ReviewRevisionRequest>,
) -> Result<Json<crate::domain::Ticket>, AppError> {
    Ok(Json(
        manager
            .approve_review(id, caller_agent(&headers)?, input.expected_revision)
            .await?,
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
                    Ok(event) => yield Ok(Event::default().json_data(event).unwrap()),
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                        yield Ok(Event::default().data(r#"{"kind":"refresh"}"#));
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                },
                _ = refresh.tick() => yield Ok(Event::default().data(r#"{"kind":"refresh"}"#)),
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

        let response = router(manager)
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
        assert_eq!(body["worktree_path"], "/tmp/worker");
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
        assert!(!agent.wake_on_pending_task);
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
                        r#"{"wake_on_pending_task":true,"idle_heartbeat_minutes":20}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body: Value =
            serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await.unwrap())
                .unwrap();
        assert_eq!(body["wake_on_pending_task"], true);
        assert_eq!(body["idle_heartbeat_minutes"], 20);

        let response = app
            .clone()
            .oneshot(
                Request::put(format!("/agents/{}/wake-settings", agent.id))
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        r#"{"wake_on_pending_task":false,"idle_heartbeat_minutes":null}"#,
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

        let response = app
            .oneshot(
                Request::put(format!("/agents/{}/wake-settings", agent.id))
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        r#"{"wake_on_pending_task":false,"idle_heartbeat_minutes":0}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }
}
