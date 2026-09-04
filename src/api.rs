use std::path::PathBuf;

use axum::{
    Extension, Json, Router,
    extract::{Path, Query, State},
    http::HeaderMap,
    routing::get,
};
use multi_agent_board_protocol::{
    AGENT_ID_HEADER, ClaimTicketRequest, CreateTicketRequest, CreateWakeEventRequest,
    MessageRangeQuery, PostMessageRequest, SetProjectRequest,
};
use serde::Deserialize;

use crate::{
    agent_runtime::AgentManager,
    domain::{Channel, Message, Project, Ticket, TicketState, WakeEvent},
    error::AppError,
    store::{NewTicket, Store, TicketChanges},
};

pub fn router(store: Store, agents: AgentManager) -> Router {
    Router::new()
        .route("/project", get(get_project).put(set_project))
        .route("/tickets", get(list_tickets).post(create_ticket))
        .route("/tickets/{id}", get(get_ticket).patch(update_ticket))
        .route("/tickets/{id}/claim", axum::routing::post(claim_ticket))
        .route(
            "/agents/{id}/wake-events",
            get(list_wake_events).post(create_wake_event),
        )
        .route("/channels", get(list_channels))
        .route(
            "/channels/{name}/messages",
            get(read_messages).post(post_message),
        )
        .layer(Extension(agents))
        .with_state(store)
}

async fn get_project(State(store): State<Store>) -> Result<Json<Option<Project>>, AppError> {
    Ok(Json(store.project().await?))
}

async fn set_project(
    State(store): State<Store>,
    Json(input): Json<SetProjectRequest>,
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
    if let Some(current) = store.project().await?
        && current.path != path
        && !store.agents().await?.is_empty()
    {
        return Err(AppError::conflict(
            "cannot change projects while agents exist",
        ));
    }
    Ok(Json(store.set_project(path).await?))
}

async fn list_tickets(State(store): State<Store>) -> Result<Json<Vec<Ticket>>, AppError> {
    Ok(Json(store.tickets().await?))
}

async fn create_ticket(
    State(store): State<Store>,
    Extension(agents): Extension<AgentManager>,
    Json(input): Json<CreateTicketRequest>,
) -> Result<Json<Ticket>, AppError> {
    let (ticket, wake_agent_ids) = store
        .create_ticket_with_wakes(NewTicket {
            title: input.title,
            description: input.description,
        })
        .await?;
    for agent_id in wake_agent_ids {
        agents.deliver_pending(agent_id).await;
    }
    Ok(Json(ticket))
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
    assignee_id: Option<serde_json::Value>,
    expected_revision: Option<i64>,
}

async fn update_ticket(
    State(store): State<Store>,
    Extension(agents): Extension<AgentManager>,
    Path(id): Path<i64>,
    headers: HeaderMap,
    Json(input): Json<UpdateTicket>,
) -> Result<Json<Ticket>, AppError> {
    if agent_header(&headers)?.is_some()
        && matches!(input.state, Some(TicketState::Review | TicketState::Done))
    {
        return Err(AppError::forbidden(
            "agents must use the review workflow for review and done states",
        ));
    }
    let assignee_id = match input.assignee_id {
        None => None,
        Some(value) if value.is_null() => Some(None),
        Some(value) => Some(Some(value.as_i64().ok_or_else(|| {
            AppError::bad_request("assignee_id must be an integer or null")
        })?)),
    };
    let (ticket, wake_agent_ids) = store
        .update_ticket_with_wakes(
            id,
            TicketChanges {
                title: input.title,
                description: input.description,
                state: input.state,
                assignee_id,
                expected_revision: input.expected_revision,
            },
        )
        .await?;
    for agent_id in wake_agent_ids {
        agents.deliver_pending(agent_id).await;
    }
    Ok(Json(ticket))
}

async fn claim_ticket(
    State(store): State<Store>,
    Extension(agents): Extension<AgentManager>,
    Path(id): Path<i64>,
    headers: HeaderMap,
    Json(input): Json<ClaimTicketRequest>,
) -> Result<Json<Ticket>, AppError> {
    let agent_id = agent_header(&headers)?
        .or(input.agent_id)
        .ok_or_else(|| AppError::bad_request("agent_id is required"))?;
    let (ticket, wake_agent_ids) = store
        .claim_ticket_with_wakes(id, agent_id, input.expected_revision)
        .await?;
    for agent_id in wake_agent_ids {
        agents.deliver_pending(agent_id).await;
    }
    Ok(Json(ticket))
}

async fn list_wake_events(
    State(store): State<Store>,
    Path(id): Path<i64>,
) -> Result<Json<Vec<WakeEvent>>, AppError> {
    Ok(Json(store.wake_events(id).await?))
}

async fn create_wake_event(
    State(store): State<Store>,
    Path(id): Path<i64>,
    Json(input): Json<CreateWakeEventRequest>,
) -> Result<Json<WakeEvent>, AppError> {
    Ok(Json(
        store
            .enqueue_wake_event(
                id,
                &input.kind,
                input.payload,
                input.deduplication_key.as_deref(),
            )
            .await?,
    ))
}

fn agent_header(headers: &HeaderMap) -> Result<Option<i64>, AppError> {
    headers
        .get(AGENT_ID_HEADER)
        .map(|value| {
            value
                .to_str()
                .ok()
                .and_then(|value| value.parse().ok())
                .ok_or_else(|| AppError::forbidden("invalid agent identity"))
        })
        .transpose()
}

async fn list_channels(State(store): State<Store>) -> Result<Json<Vec<Channel>>, AppError> {
    Ok(Json(store.channels().await?))
}

async fn read_messages(
    State(store): State<Store>,
    Path(name): Path<String>,
    Query(range): Query<MessageRangeQuery>,
) -> Result<Json<Vec<Message>>, AppError> {
    Ok(Json(
        store
            .messages(&name, range.start_time, range.end_time)
            .await?,
    ))
}

async fn post_message(
    State(store): State<Store>,
    Extension(agents): Extension<AgentManager>,
    Path(name): Path<String>,
    headers: HeaderMap,
    Json(input): Json<PostMessageRequest>,
) -> Result<Json<Message>, AppError> {
    let agent_id = agent_header(&headers)?;
    let author = if let Some(agent_id) = agent_id {
        store
            .agent(agent_id)
            .await?
            .ok_or_else(|| AppError::forbidden("agent identity is invalid"))?
            .name
    } else {
        input.author
    };
    let (message, wake_agent_ids) = store
        .post_message_with_wakes(&name, agent_id, &author, &input.body)
        .await?;
    for agent_id in wake_agent_ids {
        agents.deliver_pending(agent_id).await;
    }
    Ok(Json(message))
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

    fn test_router(store: Store) -> Router {
        router(
            store.clone(),
            AgentManager::new(
                store,
                "codex".into(),
                "multi-agent-board".into(),
                "http://127.0.0.1:1/api".into(),
            ),
        )
    }

    #[tokio::test]
    async fn tickets_round_trip_over_http() {
        let app = test_router(Store::memory().await.unwrap());
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
    async fn new_tickets_wake_opted_in_agents_over_http() {
        let store = Store::memory().await.unwrap();
        let agent = store.create_agent("worker", "/tmp/worker").await.unwrap();
        store
            .set_agent_wake_settings(agent.id, true, None)
            .await
            .unwrap();
        store
            .update_agent_runtime(
                agent.id,
                crate::domain::AgentState::Paused,
                None,
                None,
                None,
            )
            .await
            .unwrap();

        let response = test_router(store.clone())
            .oneshot(
                Request::post("/tickets")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(r#"{"title":"Available work"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let events = store.wake_events(agent.id).await.unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind, "ticket.available");
        assert_eq!(
            events[0].delivery_state,
            crate::domain::WakeEventState::Pending
        );
    }

    #[tokio::test]
    async fn malformed_ticket_state_is_rejected() {
        let response = test_router(Store::memory().await.unwrap())
            .oneshot(
                Request::post("/tickets")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(r#"{"title":"Valid"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let response = test_router(Store::memory().await.unwrap())
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

    #[tokio::test]
    async fn tickets_can_be_claimed_once_over_http() {
        let store = Store::memory().await.unwrap();
        store.create_agent("first", "/tmp/first").await.unwrap();
        store.create_agent("second", "/tmp/second").await.unwrap();
        store
            .create_ticket(NewTicket {
                title: "Claim me".into(),
                description: String::new(),
            })
            .await
            .unwrap();
        let app = test_router(store);

        let response = app
            .clone()
            .oneshot(
                Request::post("/tickets/1/claim")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(r#"{"agent_id":1,"expected_revision":1}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let response = app
            .oneshot(
                Request::post("/tickets/1/claim")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(r#"{"agent_id":2}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::CONFLICT);
    }

    #[tokio::test]
    async fn agent_messages_keep_a_stable_author_identity() {
        let store = Store::memory().await.unwrap();
        let agent = store.create_agent("worker", "/tmp/worker").await.unwrap();
        let response = test_router(store)
            .oneshot(
                Request::post("/channels/general/messages")
                    .header(header::CONTENT_TYPE, "application/json")
                    .header(AGENT_ID_HEADER, agent.id)
                    .body(Body::from(r#"{"author":"spoofed","body":"Done"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let message: Value =
            serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await.unwrap())
                .unwrap();
        assert_eq!(message["agent_id"], agent.id);
        assert_eq!(message["author"], "worker");
    }

    #[tokio::test]
    async fn posting_a_mention_enqueues_a_durable_wake_event() {
        let store = Store::memory().await.unwrap();
        let agent = store.create_agent("worker", "/tmp/worker").await.unwrap();
        store
            .update_agent_runtime(
                agent.id,
                crate::domain::AgentState::Paused,
                None,
                None,
                None,
            )
            .await
            .unwrap();

        let response = test_router(store.clone())
            .oneshot(
                Request::post("/channels/general/messages")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        r#"{"author":"human","body":"Please check this, @worker."}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let events = store.wake_events(agent.id).await.unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind, "channel.message");
        assert_eq!(
            events[0].delivery_state,
            crate::domain::WakeEventState::Pending
        );
    }

    #[tokio::test]
    async fn wake_events_round_trip_and_deduplicate_over_http() {
        let store = Store::memory().await.unwrap();
        let agent = store.create_agent("first", "/tmp/first").await.unwrap();
        let app = test_router(store);
        let request = || {
            Request::post(format!("/agents/{}/wake-events", agent.id))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    r#"{"kind":"channel.message","payload":{"channel":"general","message_id":9},"deduplication_key":"message:9"}"#,
                ))
                .unwrap()
        };

        let first = app.clone().oneshot(request()).await.unwrap();
        assert_eq!(first.status(), StatusCode::OK);
        let first: Value =
            serde_json::from_slice(&to_bytes(first.into_body(), usize::MAX).await.unwrap())
                .unwrap();
        let duplicate = app.clone().oneshot(request()).await.unwrap();
        let duplicate: Value =
            serde_json::from_slice(&to_bytes(duplicate.into_body(), usize::MAX).await.unwrap())
                .unwrap();
        assert_eq!(duplicate["id"], first["id"]);

        let response = app
            .oneshot(
                Request::get(format!("/agents/{}/wake-events", agent.id))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let events: Value =
            serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await.unwrap())
                .unwrap();
        assert_eq!(events.as_array().unwrap().len(), 1);
        assert_eq!(events[0]["delivery_state"], "pending");
        assert_eq!(events[0]["payload"]["message_id"], 9);
    }
}
