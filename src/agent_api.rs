use axum::{
    Json, Router,
    extract::{Path, State},
    http::HeaderMap,
    response::sse::{Event, KeepAlive, Sse},
    routing::{get, post},
};
use serde::Deserialize;
use std::{convert::Infallible, time::Duration};

use crate::{
    agent_runtime::AgentManager,
    domain::{Agent, AgentMessage},
    error::AppError,
};

pub fn router(manager: AgentManager) -> Router {
    Router::new()
        .route("/agents", get(list_agents).post(create_agent))
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
        .route("/tickets/{id}/submit-review", post(submit_review))
        .route("/tickets/{id}/claim-review", post(claim_review))
        .route("/tickets/{id}/request-changes", post(request_changes))
        .route("/tickets/{id}/approve-review", post(approve_review))
        .route("/events", get(events))
        .with_state(manager)
}

#[derive(Deserialize)]
struct CreateAgent {
    name: String,
}

async fn list_agents(State(manager): State<AgentManager>) -> Result<Json<Vec<Agent>>, AppError> {
    Ok(Json(manager.list().await?))
}

async fn create_agent(
    State(manager): State<AgentManager>,
    Json(input): Json<CreateAgent>,
) -> Result<Json<Agent>, AppError> {
    Ok(Json(manager.create(&input.name).await?))
}

async fn list_messages(
    State(manager): State<AgentManager>,
    Path(id): Path<i64>,
) -> Result<Json<Vec<AgentMessage>>, AppError> {
    Ok(Json(manager.messages(id).await?))
}

#[derive(Deserialize)]
struct SendMessage {
    message: String,
}

async fn send_message(
    State(manager): State<AgentManager>,
    Path(id): Path<i64>,
    Json(input): Json<SendMessage>,
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

#[derive(Deserialize)]
struct SetScopes {
    scopes: Vec<String>,
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
    Json(input): Json<SetScopes>,
) -> Result<Json<Vec<String>>, AppError> {
    Ok(Json(manager.set_scopes(id, &input.scopes).await?))
}

#[derive(Deserialize)]
struct ReviewRevision {
    expected_revision: Option<i64>,
}

async fn submit_review(
    State(manager): State<AgentManager>,
    Path(id): Path<i64>,
    headers: HeaderMap,
    Json(input): Json<ReviewRevision>,
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
    Json(input): Json<ReviewRevision>,
) -> Result<Json<crate::domain::Ticket>, AppError> {
    Ok(Json(
        manager
            .claim_review(id, caller_agent(&headers)?, input.expected_revision)
            .await?,
    ))
}

#[derive(Deserialize)]
struct RequestChanges {
    feedback: String,
    expected_revision: Option<i64>,
}

async fn request_changes(
    State(manager): State<AgentManager>,
    Path(id): Path<i64>,
    headers: HeaderMap,
    Json(input): Json<RequestChanges>,
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
    Json(input): Json<ReviewRevision>,
) -> Result<Json<crate::domain::Ticket>, AppError> {
    Ok(Json(
        manager
            .approve_review(id, caller_agent(&headers)?, input.expected_revision)
            .await?,
    ))
}

fn caller_agent(headers: &HeaderMap) -> Result<i64, AppError> {
    headers
        .get("x-multi-agent-board-agent-id")
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
