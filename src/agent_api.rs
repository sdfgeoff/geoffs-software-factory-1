use axum::{
    Json, Router,
    extract::{Path, State},
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
