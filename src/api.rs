use std::path::PathBuf;

use axum::{
    Extension, Json, Router,
    extract::{Path, Query, State},
    http::HeaderMap,
    routing::get,
};
use jiff::Timestamp;
use multi_agent_board_protocol::{
    AGENT_ID_HEADER, ClaimTicketRequest, CreateChannelRequest, CreateTicketRequest,
    CreateWakeEventRequest, MessageRangeQuery, PostMessageRequest, RegisterProjectRequest,
    ReserveTicketsRequest, ScheduleSelfReminderRequest, SetAgentChannelSubscriptionsRequest,
    SetAgentProjectSubscriptionsRequest, SetProjectRequest,
};
use serde::Deserialize;

use crate::{
    agent_runtime::AgentManager,
    domain::{
        BoardSettings, Channel, Message, Project, Reminder, Ticket, TicketActivity,
        TicketClaimResponse, TicketState, WakeDeliveryPolicy, WakeEvent,
    },
    error::AppError,
    store::{MAX_REMINDER_DELAY_SECONDS, NewTicket, Store, TicketChanges},
};

pub fn router(store: Store, agents: AgentManager) -> Router {
    Router::new()
        .route("/settings", get(get_settings).put(set_settings))
        .route("/project", get(get_project).put(set_project))
        .route("/projects", get(list_projects).post(register_project))
        .route(
            "/projects/{id}/archive",
            axum::routing::post(archive_project),
        )
        .route("/tickets", get(list_tickets).post(create_ticket))
        .route("/ticket-reservations", axum::routing::post(reserve_tickets))
        .route("/tickets/{id}", get(get_ticket).patch(update_ticket))
        .route("/tickets/{id}/activity", get(ticket_activity))
        .route("/tickets/{id}/claim", axum::routing::post(claim_ticket))
        .route(
            "/agents/{id}/wake-events",
            get(list_wake_events).post(create_wake_event),
        )
        .route(
            "/agent/channel-subscriptions",
            get(my_channel_subscriptions).put(replace_my_channel_subscriptions),
        )
        .route(
            "/agent/channel-subscriptions/add",
            axum::routing::post(add_my_channel_subscriptions),
        )
        .route(
            "/agent/channel-subscriptions/remove",
            axum::routing::post(remove_my_channel_subscriptions),
        )
        .route(
            "/agent/project-subscriptions",
            get(my_project_subscriptions).put(replace_my_project_subscriptions),
        )
        .route(
            "/agent/project-subscriptions/add",
            axum::routing::post(add_my_project_subscriptions),
        )
        .route(
            "/agent/project-subscriptions/remove",
            axum::routing::post(remove_my_project_subscriptions),
        )
        .route(
            "/agent/reminders",
            get(list_my_reminders).post(schedule_self_reminder),
        )
        .route(
            "/agent/reminders/{id}",
            axum::routing::delete(cancel_my_reminder),
        )
        .route("/channels", get(list_channels).post(create_channel))
        .route("/channels/{name}", get(get_channel))
        .route(
            "/channels/{name}/messages",
            get(read_messages).post(post_message),
        )
        .layer(Extension(agents))
        .with_state(store)
}

async fn get_settings(
    Extension(agents): Extension<AgentManager>,
) -> Result<Json<BoardSettings>, AppError> {
    Ok(Json(agents.settings().await?))
}

async fn set_settings(
    Extension(agents): Extension<AgentManager>,
    Json(input): Json<BoardSettings>,
) -> Result<Json<BoardSettings>, AppError> {
    Ok(Json(
        agents
            .set_settings(input.merge_notification_channel.as_deref())
            .await?,
    ))
}

async fn get_project(State(store): State<Store>) -> Result<Json<Option<Project>>, AppError> {
    Ok(Json(store.project().await?))
}

async fn set_project(
    State(store): State<Store>,
    Json(input): Json<SetProjectRequest>,
) -> Result<Json<Project>, AppError> {
    let canonical = canonical_git_root(&input.path).await?;
    let path = canonical
        .to_str()
        .ok_or_else(|| AppError::bad_request("project path is not valid UTF-8"))?;
    let project = if let Some(project) = store.project_by_path(path).await? {
        store.select_project(project.id).await?
    } else {
        let name = canonical
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| AppError::bad_request("project name is not valid UTF-8"))?;
        let default_branch = current_branch(&canonical).await?;
        let project = store.register_project(name, path, &default_branch).await?;
        store.select_project(project.id).await?
    };
    Ok(Json(project))
}

async fn list_projects(State(store): State<Store>) -> Result<Json<Vec<Project>>, AppError> {
    Ok(Json(store.projects().await?))
}

async fn register_project(
    State(store): State<Store>,
    Json(input): Json<RegisterProjectRequest>,
) -> Result<Json<Project>, AppError> {
    let canonical = canonical_git_root(&input.path).await?;
    validate_default_branch(&canonical, &input.default_branch).await?;
    let path = canonical
        .to_str()
        .ok_or_else(|| AppError::bad_request("project path is not valid UTF-8"))?;
    Ok(Json(
        store
            .register_project(&input.name, path, &input.default_branch)
            .await?,
    ))
}

async fn archive_project(
    State(store): State<Store>,
    Path(id): Path<i64>,
) -> Result<Json<Project>, AppError> {
    Ok(Json(store.archive_project(id).await?))
}

async fn canonical_git_root(path: &str) -> Result<PathBuf, AppError> {
    let canonical = PathBuf::from(path)
        .canonicalize()
        .map_err(|_| AppError::bad_request("project path does not exist"))?;
    let output = tokio::process::Command::new("git")
        .arg("-C")
        .arg(&canonical)
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .await
        .map_err(|error| AppError::internal(format!("failed to start Git: {error}")))?;
    if !output.status.success() {
        return Err(AppError::bad_request(
            "project path must be a Git working tree",
        ));
    }
    let root = String::from_utf8(output.stdout)
        .map_err(|_| AppError::bad_request("project path is not valid UTF-8"))?;
    PathBuf::from(root.trim())
        .canonicalize()
        .map_err(|_| AppError::bad_request("Git root does not exist"))
}

async fn validate_default_branch(root: &std::path::Path, branch: &str) -> Result<(), AppError> {
    let branch = branch.trim();
    let valid_name = tokio::process::Command::new("git")
        .args(["check-ref-format", "--branch", branch])
        .output()
        .await
        .map_err(|error| AppError::internal(format!("failed to start Git: {error}")))?;
    if !valid_name.status.success() {
        return Err(AppError::bad_request("default branch name is invalid"));
    }
    let output = tokio::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["rev-parse", "--verify", "--quiet"])
        .arg(format!("refs/heads/{branch}^{{commit}}"))
        .output()
        .await
        .map_err(|error| AppError::internal(format!("failed to start Git: {error}")))?;
    if !output.status.success() {
        return Err(AppError::bad_request(
            "default branch does not exist in the project",
        ));
    }
    Ok(())
}

async fn current_branch(root: &std::path::Path) -> Result<String, AppError> {
    let output = tokio::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["symbolic-ref", "--short", "HEAD"])
        .output()
        .await
        .map_err(|error| AppError::internal(format!("failed to start Git: {error}")))?;
    if !output.status.success() {
        return Err(AppError::bad_request(
            "project must have a checked-out default branch",
        ));
    }
    String::from_utf8(output.stdout)
        .map(|branch| branch.trim().to_owned())
        .map_err(|_| AppError::bad_request("default branch is not valid UTF-8"))
}

async fn list_tickets(
    State(store): State<Store>,
    headers: HeaderMap,
) -> Result<Json<Vec<Ticket>>, AppError> {
    let mut tickets = store.tickets().await?;
    if let Some(agent_id) = agent_header(&headers)? {
        let subscriptions = store.agent_project_subscriptions(agent_id).await?;
        tickets.sort_by_key(|ticket| !subscriptions.contains(&ticket.project_id));
    }
    Ok(Json(tickets))
}

async fn create_ticket(
    State(store): State<Store>,
    Extension(agents): Extension<AgentManager>,
    headers: HeaderMap,
    Json(input): Json<CreateTicketRequest>,
) -> Result<Json<Ticket>, AppError> {
    let (ticket, wake_agent_ids) = store
        .create_ticket_for_project_with_wakes(
            NewTicket {
                title: input.title,
                description: input.description,
                blocker_ids: input.blocker_ids,
                created_by_agent_id: agent_header(&headers)?,
            },
            input.project_id,
        )
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

async fn ticket_activity(
    State(store): State<Store>,
    Path(id): Path<i64>,
) -> Result<Json<Vec<TicketActivity>>, AppError> {
    Ok(Json(store.ticket_activity(id).await?))
}

async fn reserve_tickets(
    State(store): State<Store>,
    Extension(agents): Extension<AgentManager>,
    headers: HeaderMap,
    Json(input): Json<ReserveTicketsRequest>,
) -> Result<Json<Vec<Ticket>>, AppError> {
    let caller = agent_header(&headers)?;
    let agent_id = match caller {
        Some(caller_id) => {
            if input.administrative_override {
                return Err(AppError::forbidden(
                    "agents cannot use the administrative reservation override",
                ));
            }
            if input.agent_id.is_some_and(|agent_id| agent_id != caller_id) {
                return Err(AppError::forbidden(
                    "agents may reserve tickets only for themselves",
                ));
            }
            caller_id
        }
        None => input
            .agent_id
            .ok_or_else(|| AppError::bad_request("agent_id is required"))?,
    };
    let (tickets, wake_agent_ids) = store
        .reserve_tickets_with_wakes(
            &input.ticket_ids,
            agent_id,
            caller,
            input.administrative_override,
        )
        .await?;
    for agent_id in wake_agent_ids {
        agents.deliver_pending(agent_id).await;
    }
    Ok(Json(tickets))
}

#[derive(Deserialize)]
struct UpdateTicket {
    title: Option<String>,
    description: Option<String>,
    state: Option<TicketState>,
    assignee_id: Option<serde_json::Value>,
    blocker_ids: Option<Vec<i64>>,
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
        && matches!(
            input.state,
            Some(TicketState::Review | TicketState::Done | TicketState::Cancelled)
        )
    {
        return Err(AppError::forbidden(
            "agents cannot directly set review, done, or cancelled states",
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
                blocker_ids: input.blocker_ids,
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
) -> Result<Json<TicketClaimResponse>, AppError> {
    let caller = agent_header(&headers)?;
    let agent_id = caller
        .or(input.agent_id)
        .ok_or_else(|| AppError::bad_request("agent_id is required"))?;
    if caller == Some(agent_id) {
        agents.prepare_ticket_project(agent_id, id).await?;
    }
    let (ticket, wake_agent_ids) = store
        .claim_ticket_with_wakes(id, agent_id, input.expected_revision)
        .await?;
    let project_transition = if caller == Some(agent_id) {
        let transition = agents
            .transition_to_ticket_project(agent_id, &ticket)
            .await?;
        store
            .complete_ticket_assignment_wake(agent_id, ticket.id, ticket.revision)
            .await?;
        transition
    } else {
        None
    };
    for agent_id in wake_agent_ids {
        agents.deliver_pending(agent_id).await;
    }
    Ok(Json(TicketClaimResponse {
        ticket,
        project_transition,
    }))
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
                WakeDeliveryPolicy::Immediate,
            )
            .await?,
    ))
}

async fn schedule_self_reminder(
    State(store): State<Store>,
    headers: HeaderMap,
    Json(input): Json<ScheduleSelfReminderRequest>,
) -> Result<Json<Reminder>, AppError> {
    let agent_id = calling_agent(&headers)?;
    let now = Timestamp::now().as_second();
    let deliver_at = match (input.deliver_at, input.delay_seconds) {
        (Some(deliver_at), None) => deliver_at
            .parse::<Timestamp>()
            .map_err(|_| AppError::bad_request("deliver_at must be an RFC 3339 timestamp"))?
            .as_second(),
        (None, Some(delay)) if (1..=MAX_REMINDER_DELAY_SECONDS).contains(&delay) => now
            .checked_add(delay)
            .ok_or_else(|| AppError::bad_request("delay_seconds is too large"))?,
        (None, Some(_)) => {
            return Err(AppError::bad_request(
                "delay_seconds must be between 1 and 31536000",
            ));
        }
        _ => {
            return Err(AppError::bad_request(
                "provide exactly one of deliver_at or delay_seconds",
            ));
        }
    };
    Ok(Json(
        store
            .create_reminder(agent_id, &input.message, deliver_at, now)
            .await?,
    ))
}

async fn list_my_reminders(
    State(store): State<Store>,
    headers: HeaderMap,
) -> Result<Json<Vec<Reminder>>, AppError> {
    Ok(Json(store.reminders(calling_agent(&headers)?).await?))
}

async fn cancel_my_reminder(
    State(store): State<Store>,
    Path(id): Path<i64>,
    headers: HeaderMap,
) -> Result<Json<Reminder>, AppError> {
    Ok(Json(
        store.cancel_reminder(calling_agent(&headers)?, id).await?,
    ))
}

async fn my_channel_subscriptions(
    Extension(agents): Extension<AgentManager>,
    headers: HeaderMap,
) -> Result<Json<Vec<String>>, AppError> {
    Ok(Json(
        agents
            .channel_subscriptions(calling_agent(&headers)?)
            .await?,
    ))
}

async fn replace_my_channel_subscriptions(
    Extension(agents): Extension<AgentManager>,
    headers: HeaderMap,
    Json(input): Json<SetAgentChannelSubscriptionsRequest>,
) -> Result<Json<Vec<String>>, AppError> {
    Ok(Json(
        agents
            .set_channel_subscriptions(calling_agent(&headers)?, &input.channels)
            .await?,
    ))
}

async fn add_my_channel_subscriptions(
    Extension(agents): Extension<AgentManager>,
    headers: HeaderMap,
    Json(input): Json<SetAgentChannelSubscriptionsRequest>,
) -> Result<Json<Vec<String>>, AppError> {
    Ok(Json(
        agents
            .add_channel_subscriptions(calling_agent(&headers)?, &input.channels)
            .await?,
    ))
}

async fn remove_my_channel_subscriptions(
    Extension(agents): Extension<AgentManager>,
    headers: HeaderMap,
    Json(input): Json<SetAgentChannelSubscriptionsRequest>,
) -> Result<Json<Vec<String>>, AppError> {
    Ok(Json(
        agents
            .remove_channel_subscriptions(calling_agent(&headers)?, &input.channels)
            .await?,
    ))
}

async fn my_project_subscriptions(
    Extension(agents): Extension<AgentManager>,
    headers: HeaderMap,
) -> Result<Json<Vec<i64>>, AppError> {
    Ok(Json(
        agents
            .project_subscriptions(calling_agent(&headers)?)
            .await?,
    ))
}

async fn replace_my_project_subscriptions(
    Extension(agents): Extension<AgentManager>,
    headers: HeaderMap,
    Json(input): Json<SetAgentProjectSubscriptionsRequest>,
) -> Result<Json<Vec<i64>>, AppError> {
    Ok(Json(
        agents
            .set_project_subscriptions(calling_agent(&headers)?, &input.project_ids)
            .await?,
    ))
}

async fn add_my_project_subscriptions(
    Extension(agents): Extension<AgentManager>,
    headers: HeaderMap,
    Json(input): Json<SetAgentProjectSubscriptionsRequest>,
) -> Result<Json<Vec<i64>>, AppError> {
    Ok(Json(
        agents
            .add_project_subscriptions(calling_agent(&headers)?, &input.project_ids)
            .await?,
    ))
}

async fn remove_my_project_subscriptions(
    Extension(agents): Extension<AgentManager>,
    headers: HeaderMap,
    Json(input): Json<SetAgentProjectSubscriptionsRequest>,
) -> Result<Json<Vec<i64>>, AppError> {
    Ok(Json(
        agents
            .remove_project_subscriptions(calling_agent(&headers)?, &input.project_ids)
            .await?,
    ))
}

fn calling_agent(headers: &HeaderMap) -> Result<i64, AppError> {
    agent_header(headers)?.ok_or_else(|| AppError::forbidden("agent identity is required"))
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

async fn get_channel(
    State(store): State<Store>,
    Path(name): Path<String>,
) -> Result<Json<Channel>, AppError> {
    store
        .channel(&name)
        .await?
        .map(Json)
        .ok_or_else(|| AppError::not_found("channel not found"))
}

async fn create_channel(
    State(store): State<Store>,
    headers: HeaderMap,
    Json(input): Json<CreateChannelRequest>,
) -> Result<Json<Channel>, AppError> {
    if let Some(agent_id) = agent_header(&headers)?
        && store.agent(agent_id).await?.is_none()
    {
        return Err(AppError::forbidden("agent identity is invalid"));
    }
    Ok(Json(
        store
            .create_channel(&input.name, &input.description)
            .await?,
    ))
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
    agents.emit_board_message(&message);
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

    fn initialize_repository(path: &std::path::Path) -> String {
        std::fs::create_dir_all(path).unwrap();
        for arguments in [
            vec!["init", "-q"],
            vec!["config", "user.name", "Test"],
            vec!["config", "user.email", "test@example.com"],
            vec!["commit", "--allow-empty", "-qm", "initial"],
        ] {
            assert!(
                std::process::Command::new("git")
                    .args(arguments)
                    .current_dir(path)
                    .status()
                    .unwrap()
                    .success()
            );
        }
        String::from_utf8(
            std::process::Command::new("git")
                .args(["branch", "--show-current"])
                .current_dir(path)
                .output()
                .unwrap()
                .stdout,
        )
        .unwrap()
        .trim()
        .into()
    }

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
    async fn settings_use_environment_fallback_until_explicitly_saved() {
        let store = Store::memory().await.unwrap();
        let agents = AgentManager::new(
            store.clone(),
            "codex".into(),
            "multi-agent-board".into(),
            "http://127.0.0.1:1/api".into(),
        )
        .with_merge_notification_channel(Some("environment-merges".into()))
        .unwrap();
        let app = router(store.clone(), agents);

        let response = app
            .clone()
            .oneshot(Request::get("/settings").body(Body::empty()).unwrap())
            .await
            .unwrap();
        let settings: Value =
            serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await.unwrap())
                .unwrap();
        assert_eq!(settings["merge_notification_channel"], "environment-merges");
        assert!(store.channel("environment-merges").await.unwrap().is_some());

        let invalid = app
            .clone()
            .oneshot(
                Request::put("/settings")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        r#"{"merge_notification_channel":"not a channel"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(invalid.status(), StatusCode::BAD_REQUEST);

        let saved = app
            .clone()
            .oneshot(
                Request::put("/settings")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(r#"{"merge_notification_channel":"ui-merges"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(saved.status(), StatusCode::OK);
        let settings: Value =
            serde_json::from_slice(&to_bytes(saved.into_body(), usize::MAX).await.unwrap())
                .unwrap();
        assert_eq!(settings["merge_notification_channel"], "ui-merges");

        let disabled = app
            .oneshot(
                Request::put("/settings")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(r#"{"merge_notification_channel":null}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(disabled.status(), StatusCode::OK);

        let restarted = router(
            store.clone(),
            AgentManager::new(
                store,
                "codex".into(),
                "multi-agent-board".into(),
                "http://127.0.0.1:1/api".into(),
            )
            .with_merge_notification_channel(Some("environment-merges".into()))
            .unwrap(),
        );
        let response = restarted
            .oneshot(Request::get("/settings").body(Body::empty()).unwrap())
            .await
            .unwrap();
        let settings: Value =
            serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await.unwrap())
                .unwrap();
        assert_eq!(settings["merge_notification_channel"], Value::Null);
    }

    #[tokio::test]
    async fn projects_register_canonical_git_roots_and_archive_safely() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let base = std::env::temp_dir().join(format!("mab-project-api-{unique}"));
        let repository = base.join("repo");
        let branch = initialize_repository(&repository);
        let nested = repository.join("nested");
        std::fs::create_dir_all(&nested).unwrap();
        let store = Store::memory().await.unwrap();
        let app = test_router(store.clone());
        let body = serde_json::json!({
            "name": "API project",
            "path": nested,
            "default_branch": branch,
        });

        let response = app
            .clone()
            .oneshot(
                Request::post("/projects")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let project: Value =
            serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await.unwrap())
                .unwrap();
        assert_eq!(project["name"], "API project");
        assert_eq!(
            project["path"],
            repository.canonicalize().unwrap().to_str().unwrap()
        );
        assert_eq!(project["default_branch"], branch);

        let duplicate = app
            .clone()
            .oneshot(
                Request::post("/projects")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(duplicate.status(), StatusCode::CONFLICT);

        let non_repository = base.join("not-git");
        std::fs::create_dir_all(&non_repository).unwrap();
        let invalid = app
            .clone()
            .oneshot(
                Request::post("/projects")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "name": "Invalid",
                            "path": non_repository,
                            "default_branch": "main",
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(invalid.status(), StatusCode::BAD_REQUEST);

        let ticket = store
            .create_ticket_for_project_with_wakes(
                NewTicket {
                    title: "Active work".into(),
                    description: String::new(),
                    blocker_ids: Vec::new(),
                    created_by_agent_id: None,
                },
                project["id"].as_i64(),
            )
            .await
            .unwrap()
            .0;
        store
            .update_ticket(
                ticket.id,
                TicketChanges {
                    state: Some(TicketState::InProgress),
                    ..TicketChanges::default()
                },
            )
            .await
            .unwrap();
        let blocked = app
            .clone()
            .oneshot(
                Request::post(format!("/projects/{}/archive", project["id"]))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(blocked.status(), StatusCode::CONFLICT);
        store
            .update_ticket(
                ticket.id,
                TicketChanges {
                    state: Some(TicketState::Todo),
                    ..TicketChanges::default()
                },
            )
            .await
            .unwrap();
        let archived = app
            .oneshot(
                Request::post(format!("/projects/{}/archive", project["id"]))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(archived.status(), StatusCode::OK);

        std::fs::remove_dir_all(base).unwrap();
    }

    #[tokio::test]
    async fn channels_are_created_before_messages_and_expose_descriptions() {
        let store = Store::memory().await.unwrap();
        let app = test_router(store);
        let create = || {
            Request::post("/channels")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    r#"{"name":"design","description":"Design decisions"}"#,
                ))
                .unwrap()
        };

        let response = app.clone().oneshot(create()).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let channel: Value =
            serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await.unwrap())
                .unwrap();
        assert_eq!(channel["description"], "Design decisions");
        assert_eq!(channel["most_recent_message_at"], Value::Null);

        let response = app
            .clone()
            .oneshot(Request::get("/channels").body(Body::empty()).unwrap())
            .await
            .unwrap();
        let channels: Value =
            serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await.unwrap())
                .unwrap();
        assert_eq!(channels[0]["name"], "design");
        assert_eq!(channels[0]["description"], "Design decisions");

        assert_eq!(
            app.clone().oneshot(create()).await.unwrap().status(),
            StatusCode::CONFLICT
        );
        let invalid = app
            .clone()
            .oneshot(
                Request::post("/channels")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(r#"{"name":"bad channel"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(invalid.status(), StatusCode::BAD_REQUEST);

        let missing_post = app
            .oneshot(
                Request::post("/channels/missing/messages")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(r#"{"author":"human","body":"Hello"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(missing_post.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn tickets_round_trip_over_http() {
        let app = test_router(Store::memory().await.unwrap());
        let missing_project = app
            .clone()
            .oneshot(
                Request::post("/tickets")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(r#"{"project_id":999,"title":"Wrong project"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(missing_project.status(), StatusCode::BAD_REQUEST);

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
        assert_eq!(ticket["project_id"], 1);

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
    async fn authenticated_ticket_lists_rank_subscribed_projects_first() {
        let store = Store::memory().await.unwrap();
        let preferred = store.project().await.unwrap().unwrap();
        let agent = store.create_agent("worker", "/tmp/worker").await.unwrap();
        let other = store.set_project("/tmp/other-project").await.unwrap();
        let other_ticket = store
            .create_ticket_for_project_with_wakes(
                NewTicket {
                    title: "Other".into(),
                    description: String::new(),
                    blocker_ids: Vec::new(),
                    created_by_agent_id: None,
                },
                Some(other.id),
            )
            .await
            .unwrap()
            .0;
        let preferred_ticket = store
            .create_ticket_for_project_with_wakes(
                NewTicket {
                    title: "Preferred".into(),
                    description: String::new(),
                    blocker_ids: Vec::new(),
                    created_by_agent_id: None,
                },
                Some(preferred.id),
            )
            .await
            .unwrap()
            .0;
        let response = test_router(store)
            .oneshot(
                Request::get("/tickets")
                    .header(AGENT_ID_HEADER, agent.id)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let tickets: Value =
            serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await.unwrap())
                .unwrap();
        assert_eq!(tickets[0]["id"], preferred_ticket.id);
        assert_eq!(tickets[1]["id"], other_ticket.id);
    }

    #[tokio::test]
    async fn new_tickets_wake_opted_in_agents_over_http() {
        let store = Store::memory().await.unwrap();
        let agent = store.create_agent("worker", "/tmp/worker").await.unwrap();
        store
            .set_agent_wake_settings(agent.id, true, false, None)
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
                blocker_ids: Vec::new(),
                created_by_agent_id: None,
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
    async fn running_agent_claims_surface_cross_project_transitions_immediately() {
        let store = Store::memory().await.unwrap();
        let first_project = store.project().await.unwrap().unwrap();
        let reviewer = store
            .create_agent("reviewer", "/tmp/reviewer")
            .await
            .unwrap();
        store
            .set_agent_scopes(reviewer.id, &["approve_review".into()])
            .await
            .unwrap();
        store
            .update_agent_runtime(
                reviewer.id,
                crate::domain::AgentState::Running,
                Some("thread-1"),
                Some("turn-1"),
                None,
            )
            .await
            .unwrap();
        let second_project = store.set_project("/tmp/second-project").await.unwrap();
        store
            .add_agent_project_worktree(reviewer.id, second_project.id, "/tmp/reviewer-second")
            .await
            .unwrap();
        let implementation = store
            .create_ticket_for_project_with_wakes(
                NewTicket {
                    title: "Cross-project implementation".into(),
                    description: String::new(),
                    blocker_ids: Vec::new(),
                    created_by_agent_id: None,
                },
                Some(second_project.id),
            )
            .await
            .unwrap()
            .0;
        let manager = AgentManager::new(
            store.clone(),
            "codex".into(),
            "multi-agent-board".into(),
            "http://127.0.0.1:1/api".into(),
        );
        let app = router(store.clone(), manager.clone()).merge(crate::agent_api::router(manager));

        let claimed = app
            .clone()
            .oneshot(
                Request::post(format!("/tickets/{}/claim", implementation.id))
                    .header(header::CONTENT_TYPE, "application/json")
                    .header(AGENT_ID_HEADER, reviewer.id)
                    .body(Body::from(format!(
                        "{{\"expected_revision\":{}}}",
                        implementation.revision
                    )))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(claimed.status(), StatusCode::OK);
        let claimed: Value =
            serde_json::from_slice(&to_bytes(claimed.into_body(), usize::MAX).await.unwrap())
                .unwrap();
        assert_eq!(
            claimed["project_transition"]["worktree_path"],
            "/tmp/reviewer-second"
        );
        assert!(
            claimed["project_transition"]["message"]
                .as_str()
                .unwrap()
                .contains(&format!("ticket #{}", implementation.id))
        );
        assert_eq!(
            store
                .agent(reviewer.id)
                .await
                .unwrap()
                .unwrap()
                .active_project_id,
            Some(second_project.id)
        );

        let worker = store.create_agent("worker", "/tmp/worker").await.unwrap();
        let review = store
            .create_ticket_for_project_with_wakes(
                NewTicket {
                    title: "Cross-project review".into(),
                    description: String::new(),
                    blocker_ids: Vec::new(),
                    created_by_agent_id: None,
                },
                Some(first_project.id),
            )
            .await
            .unwrap()
            .0;
        let review = store
            .claim_ticket(review.id, worker.id, Some(review.revision))
            .await
            .unwrap();
        let review = store
            .submit_review(
                review.id,
                worker.id,
                crate::store::ReviewSubmission {
                    implementation_branch: Some("agents/worker"),
                    commit: "abc123",
                    target_branch: "main",
                    project_id: first_project.id,
                    project_path: &first_project.path,
                },
                Some(review.revision),
            )
            .await
            .unwrap();
        let claimed_review = app
            .oneshot(
                Request::post(format!("/tickets/{}/claim-review", review.id))
                    .header(header::CONTENT_TYPE, "application/json")
                    .header(AGENT_ID_HEADER, reviewer.id)
                    .body(Body::from(format!(
                        "{{\"expected_revision\":{}}}",
                        review.revision
                    )))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(claimed_review.status(), StatusCode::OK);
        let claimed_review: Value = serde_json::from_slice(
            &to_bytes(claimed_review.into_body(), usize::MAX)
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(
            claimed_review["project_transition"]["worktree_path"],
            "/tmp/reviewer"
        );
        assert_eq!(
            store
                .agent(reviewer.id)
                .await
                .unwrap()
                .unwrap()
                .active_project_id,
            Some(first_project.id)
        );
        assert_eq!(
            store
                .agent(reviewer.id)
                .await
                .unwrap()
                .unwrap()
                .active_turn_id
                .as_deref(),
            Some("turn-1")
        );
    }

    #[tokio::test]
    async fn reservations_keep_creator_and_require_an_explicit_admin_override() {
        let store = Store::memory().await.unwrap();
        store.create_agent("first", "/tmp/first").await.unwrap();
        store.create_agent("second", "/tmp/second").await.unwrap();
        let app = test_router(store);

        let created = app
            .clone()
            .oneshot(
                Request::post("/tickets")
                    .header(header::CONTENT_TYPE, "application/json")
                    .header(AGENT_ID_HEADER, 1)
                    .body(Body::from(r#"{"title":"Owned work"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(created.status(), StatusCode::OK);
        let created: Value =
            serde_json::from_slice(&to_bytes(created.into_body(), usize::MAX).await.unwrap())
                .unwrap();
        assert_eq!(created["created_by_agent_id"], 1);

        let reserved = app
            .clone()
            .oneshot(
                Request::post("/ticket-reservations")
                    .header(header::CONTENT_TYPE, "application/json")
                    .header(AGENT_ID_HEADER, 1)
                    .body(Body::from(r#"{"ticket_ids":[1]}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(reserved.status(), StatusCode::OK);
        let reserved: Value =
            serde_json::from_slice(&to_bytes(reserved.into_body(), usize::MAX).await.unwrap())
                .unwrap();
        assert_eq!(reserved[0]["state"], "todo");
        assert_eq!(reserved[0]["assignee_id"], Value::Null);
        assert_eq!(reserved[0]["reserved_for_agent_id"], 1);

        let forbidden = app
            .clone()
            .oneshot(
                Request::post("/ticket-reservations")
                    .header(header::CONTENT_TYPE, "application/json")
                    .header(AGENT_ID_HEADER, 2)
                    .body(Body::from(
                        r#"{"ticket_ids":[1],"administrative_override":true}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(forbidden.status(), StatusCode::FORBIDDEN);

        let overridden = app
            .clone()
            .oneshot(
                Request::post("/ticket-reservations")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        r#"{"ticket_ids":[1],"agent_id":2,"administrative_override":true}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(overridden.status(), StatusCode::OK);

        let activity = app
            .clone()
            .oneshot(
                Request::get("/tickets/1/activity")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(activity.status(), StatusCode::OK);
        let activity: Value =
            serde_json::from_slice(&to_bytes(activity.into_body(), usize::MAX).await.unwrap())
                .unwrap();
        assert_eq!(activity[0]["action"], "reservation_override");
        assert_eq!(activity[0]["previous_reserved_for_agent_id"], 1);
        assert_eq!(activity[0]["reserved_for_agent_id"], 2);

        let competing_claim = app
            .clone()
            .oneshot(
                Request::post("/tickets/1/claim")
                    .header(header::CONTENT_TYPE, "application/json")
                    .header(AGENT_ID_HEADER, 1)
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(competing_claim.status(), StatusCode::CONFLICT);

        let user_created = app
            .clone()
            .oneshot(
                Request::post("/tickets")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(r#"{"title":"User work"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        let user_created: Value = serde_json::from_slice(
            &to_bytes(user_created.into_body(), usize::MAX)
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(user_created["created_by_agent_id"], Value::Null);
        let admin_reservation = app
            .oneshot(
                Request::post("/ticket-reservations")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(r#"{"ticket_ids":[2],"agent_id":1}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(admin_reservation.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn ticket_dependencies_round_trip_and_block_claims_over_http() {
        let store = Store::memory().await.unwrap();
        store.create_agent("worker", "/tmp/worker").await.unwrap();
        let app = test_router(store);
        let blocker = app
            .clone()
            .oneshot(
                Request::post("/tickets")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(r#"{"title":"Foundation"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(blocker.status(), StatusCode::OK);

        let dependent = app
            .clone()
            .oneshot(
                Request::post("/tickets")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(r#"{"title":"Follow-up","blocker_ids":[1]}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(dependent.status(), StatusCode::OK);
        let dependent: Value =
            serde_json::from_slice(&to_bytes(dependent.into_body(), usize::MAX).await.unwrap())
                .unwrap();
        assert_eq!(dependent["blocker_ids"], serde_json::json!([1]));
        assert_eq!(dependent["unresolved_blockers"][0]["title"], "Foundation");

        let claim = app
            .oneshot(
                Request::post("/tickets/2/claim")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(r#"{"agent_id":1,"expected_revision":1}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(claim.status(), StatusCode::CONFLICT);
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
    async fn posting_a_board_message_emits_its_own_event_without_wake_recipients() {
        let store = Store::memory().await.unwrap();
        let agents = AgentManager::new(
            store.clone(),
            "codex".into(),
            "multi-agent-board".into(),
            "http://127.0.0.1:1/api".into(),
        );
        let mut events = agents.subscribe();

        let response = router(store, agents)
            .oneshot(
                Request::post("/channels/general/messages")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(r#"{"author":"human","body":"No wake needed"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let event = events.recv().await.unwrap();
        assert_eq!(event.kind, "board_message");
        assert_eq!(event.agent_id, 0);
        assert_eq!(event.data["channel_name"], "general");
        assert_eq!(event.data["body"], "No wake needed");
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

    #[tokio::test]
    async fn reminder_api_is_caller_scoped_and_validates_schedule_and_cancellation() {
        let store = Store::memory().await.unwrap();
        let first = store.create_agent("first", "/tmp/first").await.unwrap();
        let second = store.create_agent("second", "/tmp/second").await.unwrap();
        let app = test_router(store);

        let relative = app
            .clone()
            .oneshot(
                Request::post("/agent/reminders")
                    .header(header::CONTENT_TYPE, "application/json")
                    .header(AGENT_ID_HEADER, first.id)
                    .body(Body::from(r#"{"message":"Check CI","delay_seconds":60}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(relative.status(), StatusCode::OK);
        let relative: Value =
            serde_json::from_slice(&to_bytes(relative.into_body(), usize::MAX).await.unwrap())
                .unwrap();
        assert_eq!(relative["agent_id"], first.id);

        let deliver_at = Timestamp::from_second(Timestamp::now().as_second() + 120)
            .unwrap()
            .to_string();
        let absolute = app
            .clone()
            .oneshot(
                Request::post("/agent/reminders")
                    .header(header::CONTENT_TYPE, "application/json")
                    .header(AGENT_ID_HEADER, first.id)
                    .body(Body::from(
                        serde_json::json!({"message":"Review logs","deliver_at":deliver_at})
                            .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(absolute.status(), StatusCode::OK);

        let listed = app
            .clone()
            .oneshot(
                Request::get("/agent/reminders")
                    .header(AGENT_ID_HEADER, first.id)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let listed: Value =
            serde_json::from_slice(&to_bytes(listed.into_body(), usize::MAX).await.unwrap())
                .unwrap();
        assert_eq!(listed.as_array().unwrap().len(), 2);

        let hidden = app
            .clone()
            .oneshot(
                Request::delete(format!("/agent/reminders/{}", relative["id"]))
                    .header(AGENT_ID_HEADER, second.id)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(hidden.status(), StatusCode::NOT_FOUND);
        let cancelled = app
            .clone()
            .oneshot(
                Request::delete(format!("/agent/reminders/{}", relative["id"]))
                    .header(AGENT_ID_HEADER, first.id)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(cancelled.status(), StatusCode::OK);
        let duplicate_cancel = app
            .clone()
            .oneshot(
                Request::delete(format!("/agent/reminders/{}", relative["id"]))
                    .header(AGENT_ID_HEADER, first.id)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(duplicate_cancel.status(), StatusCode::CONFLICT);

        for body in [
            r#"{"message":"Missing time"}"#,
            r#"{"message":"Two times","delay_seconds":1,"deliver_at":"2099-01-01T00:00:00Z"}"#,
            r#"{"message":"Bad delay","delay_seconds":0}"#,
            r#"{"message":"Bad time","deliver_at":"tomorrow"}"#,
        ] {
            let response = app
                .clone()
                .oneshot(
                    Request::post("/agent/reminders")
                        .header(header::CONTENT_TYPE, "application/json")
                        .header(AGENT_ID_HEADER, first.id)
                        .body(Body::from(body))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        }
        let unauthenticated = app
            .oneshot(
                Request::get("/agent/reminders")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(unauthenticated.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn humans_cancel_safe_ticket_states_while_agents_and_reviews_cannot() {
        let store = Store::memory().await.unwrap();
        let agent = store.create_agent("worker", "/tmp/worker").await.unwrap();
        let todo = store
            .create_ticket(NewTicket {
                title: "Duplicate".into(),
                description: String::new(),
                blocker_ids: Vec::new(),
                created_by_agent_id: None,
            })
            .await
            .unwrap();
        let in_progress = store
            .create_ticket(NewTicket {
                title: "Obsolete work".into(),
                description: String::new(),
                blocker_ids: Vec::new(),
                created_by_agent_id: None,
            })
            .await
            .unwrap();
        let in_progress = store
            .update_ticket(
                in_progress.id,
                TicketChanges {
                    state: Some(TicketState::InProgress),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let review = store
            .create_ticket(NewTicket {
                title: "Reviewing".into(),
                description: String::new(),
                blocker_ids: Vec::new(),
                created_by_agent_id: None,
            })
            .await
            .unwrap();
        let review = store
            .update_ticket(
                review.id,
                TicketChanges {
                    state: Some(TicketState::Review),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let app = test_router(store);

        for ticket in [&todo, &in_progress] {
            let response = app
                .clone()
                .oneshot(
                    Request::patch(format!("/tickets/{}", ticket.id))
                        .header(header::CONTENT_TYPE, "application/json")
                        .body(Body::from(format!(
                            "{{\"state\":\"cancelled\",\"expected_revision\":{}}}",
                            ticket.revision
                        )))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            let body: Value =
                serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await.unwrap())
                    .unwrap();
            assert_eq!(body["state"], "cancelled");
        }

        let activity = app
            .clone()
            .oneshot(
                Request::get(format!("/tickets/{}/activity", todo.id))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let activity: Value =
            serde_json::from_slice(&to_bytes(activity.into_body(), usize::MAX).await.unwrap())
                .unwrap();
        assert_eq!(activity[0]["action"], "cancelled");

        let forbidden = app
            .clone()
            .oneshot(
                Request::patch(format!("/tickets/{}", review.id))
                    .header(header::CONTENT_TYPE, "application/json")
                    .header(AGENT_ID_HEADER, agent.id)
                    .body(Body::from(r#"{"state":"cancelled"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(forbidden.status(), StatusCode::FORBIDDEN);
        let conflict = app
            .oneshot(
                Request::patch(format!("/tickets/{}", review.id))
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(r#"{"state":"cancelled"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(conflict.status(), StatusCode::CONFLICT);
    }
}
