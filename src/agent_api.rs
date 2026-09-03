use axum::{
    Json, Router,
    extract::{Path, State},
    routing::{get, post},
};
use serde::Deserialize;

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
