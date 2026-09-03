use std::path::PathBuf;

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    routing::get,
};
use serde::Deserialize;

use crate::{
    domain::{Channel, Message, Project, Ticket, TicketState},
    error::AppError,
    store::{NewTicket, Store, TicketChanges},
};

pub fn router(store: Store) -> Router {
    Router::new()
        .route("/project", get(get_project).put(set_project))
        .route("/tickets", get(list_tickets).post(create_ticket))
        .route("/tickets/{id}", get(get_ticket).patch(update_ticket))
        .route("/channels", get(list_channels))
        .route(
            "/channels/{name}/messages",
            get(read_messages).post(post_message),
        )
        .with_state(store)
}

#[derive(Deserialize)]
struct SetProject {
    path: String,
}

async fn get_project(State(store): State<Store>) -> Result<Json<Option<Project>>, AppError> {
    Ok(Json(store.project().await?))
}

async fn set_project(
    State(store): State<Store>,
    Json(input): Json<SetProject>,
) -> Result<Json<Project>, AppError> {
    let path = PathBuf::from(input.path);
    let canonical = path
        .canonicalize()
        .map_err(|_| AppError::bad_request("project path does not exist"))?;
    if !canonical.is_dir() || !canonical.join(".git").exists() {
        return Err(AppError::bad_request(
            "project path must be a Git working tree",
        ));
    }
    let path = canonical
        .to_str()
        .ok_or_else(|| AppError::bad_request("project path is not valid UTF-8"))?;
    Ok(Json(store.set_project(path).await?))
}

#[derive(Deserialize)]
struct CreateTicket {
    title: String,
    #[serde(default)]
    description: String,
}

async fn list_tickets(State(store): State<Store>) -> Result<Json<Vec<Ticket>>, AppError> {
    Ok(Json(store.tickets().await?))
}

async fn create_ticket(
    State(store): State<Store>,
    Json(input): Json<CreateTicket>,
) -> Result<Json<Ticket>, AppError> {
    Ok(Json(
        store
            .create_ticket(NewTicket {
                title: input.title,
                description: input.description,
            })
            .await?,
    ))
}

async fn get_ticket(
    State(store): State<Store>,
    Path(id): Path<i64>,
) -> Result<Json<Ticket>, AppError> {
    store
        .ticket(id)
        .await?
        .map(Json)
        .ok_or_else(|| AppError::not_found("ticket not found"))
}

#[derive(Deserialize)]
struct UpdateTicket {
    title: Option<String>,
    description: Option<String>,
    state: Option<TicketState>,
    expected_revision: Option<i64>,
}

async fn update_ticket(
    State(store): State<Store>,
    Path(id): Path<i64>,
    Json(input): Json<UpdateTicket>,
) -> Result<Json<Ticket>, AppError> {
    Ok(Json(
        store
            .update_ticket(
                id,
                TicketChanges {
                    title: input.title,
                    description: input.description,
                    state: input.state,
                    expected_revision: input.expected_revision,
                },
            )
            .await?,
    ))
}

async fn list_channels(State(store): State<Store>) -> Result<Json<Vec<Channel>>, AppError> {
    Ok(Json(store.channels().await?))
}

#[derive(Deserialize)]
struct MessageRange {
    start_time: Option<i64>,
    end_time: Option<i64>,
}

async fn read_messages(
    State(store): State<Store>,
    Path(name): Path<String>,
    Query(range): Query<MessageRange>,
) -> Result<Json<Vec<Message>>, AppError> {
    Ok(Json(
        store
            .messages(&name, range.start_time, range.end_time)
            .await?,
    ))
}

#[derive(Deserialize)]
struct PostMessage {
    author: String,
    body: String,
}

async fn post_message(
    State(store): State<Store>,
    Path(name): Path<String>,
    Json(input): Json<PostMessage>,
) -> Result<Json<Message>, AppError> {
    Ok(Json(
        store
            .post_message(&name, &input.author, &input.body)
            .await?,
    ))
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

    #[tokio::test]
    async fn tickets_round_trip_over_http() {
        let app = router(Store::memory().await.unwrap());
        let response = app
            .clone()
            .oneshot(
                Request::post("/tickets")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(r#"{"title":"Wire the API"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let ticket: Value =
            serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await.unwrap())
                .unwrap();

        let response = app
            .oneshot(
                Request::patch(format!("/tickets/{}", ticket["id"]))
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        r#"{"state":"in_progress","expected_revision":1}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn malformed_ticket_state_is_rejected() {
        let response = router(Store::memory().await.unwrap())
            .oneshot(
                Request::post("/tickets")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(r#"{"title":"Valid"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let response = router(Store::memory().await.unwrap())
            .oneshot(
                Request::patch("/tickets/1")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(r#"{"state":"maybe"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    }
}
