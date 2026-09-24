use reqwest::Method;
use serde::{Serialize, de::DeserializeOwned};

use multi_agent_board_protocol::{
    AGENT_ID_HEADER, Agent, AgentMessagePage, AgentProjectWorktree, BoardSettings, Channel,
    CompleteExternalMergeRequest, CreateAgentRequest, CreateChannelRequest, CreateTicketRequest,
    CreateWakeEventRequest, ErrorResponse, Message, MessagePage, MessagePageQuery,
    PostMessageRequest, PostTicketCommentRequest, Project, RegisterProjectRequest, Reminder,
    RenameAgentRequest, RequestChangesRequest, ReserveTicketsRequest, ReturnExternalMergeRequest,
    ReviewRevisionRequest, ScheduleSelfReminderRequest, SendAgentMessageRequest,
    SetAgentChannelSubscriptionsRequest, SetAgentProjectSubscriptionsRequest,
    SetAgentScopesRequest, SetAgentWakeSettingsRequest, SetBoardSettingsRequest,
    SetExternalChangeRequest, SetProjectCompletionWorkflowRequest, Ticket, TicketActivity,
    TicketActivityPage, TicketClaimResponse, TicketSummary, TicketSummaryPage, UpdateTicketRequest,
    WakeEvent,
};

#[derive(Debug)]
pub struct McpError(String);

impl McpError {
    pub(crate) fn bad_request(message: impl Into<String>) -> Self {
        Self(message.into())
    }

    pub(crate) fn internal(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl std::fmt::Display for McpError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for McpError {}

#[derive(Clone)]
pub struct BoardClient {
    base_url: String,
    agent_id: i64,
    client: reqwest::Client,
}

impl BoardClient {
    pub fn new(base_url: String, agent_id: i64) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').into(),
            agent_id,
            client: reqwest::Client::builder()
                .no_proxy()
                .build()
                .expect("HTTP client configuration is valid"),
        }
    }

    async fn request<B: Serialize + ?Sized, R: DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
        body: Option<&B>,
    ) -> Result<R, McpError> {
        let mut request = self
            .client
            .request(method, format!("{}{path}", self.base_url))
            .header(AGENT_ID_HEADER, self.agent_id);
        if let Some(body) = body {
            request = request.json(&body);
        }
        let response = request
            .send()
            .await
            .map_err(|error| McpError::internal(format!("board request failed: {error}")))?;
        let status = response.status();
        let body = response
            .bytes()
            .await
            .map_err(|error| McpError::internal(format!("invalid board response: {error}")))?;
        if !status.is_success() {
            let error = serde_json::from_slice::<ErrorResponse>(&body)
                .map(|body| body.error)
                .unwrap_or_else(|_| "board request failed".into());
            return Err(McpError::internal(error));
        }
        serde_json::from_slice(&body)
            .map_err(|error| McpError::internal(format!("invalid board response: {error}")))
    }

    async fn get<R: DeserializeOwned>(&self, path: &str) -> Result<R, McpError> {
        self.request::<(), _>(Method::GET, path, None).await
    }

    pub async fn list_tickets(&self) -> Result<Vec<TicketSummary>, McpError> {
        let mut tickets = Vec::new();
        let mut before_id = None;
        loop {
            let path = before_id.map_or_else(
                || "/tickets?limit=100".to_owned(),
                |id| format!("/tickets?limit=100&before_id={id}"),
            );
            let page: TicketSummaryPage = self.get(&path).await?;
            let next = page.tickets.iter().map(|ticket| ticket.id).min();
            tickets.extend(page.tickets);
            if !page.has_more {
                break;
            }
            before_id = next;
        }
        Ok(tickets)
    }

    pub async fn settings(&self) -> Result<BoardSettings, McpError> {
        self.get("/settings").await
    }

    pub async fn set_settings(
        &self,
        request: &SetBoardSettingsRequest,
    ) -> Result<BoardSettings, McpError> {
        self.request(Method::PUT, "/settings", Some(request)).await
    }

    pub async fn list_projects(&self) -> Result<Vec<Project>, McpError> {
        self.get("/projects").await
    }

    pub async fn register_project(
        &self,
        request: &RegisterProjectRequest,
    ) -> Result<Project, McpError> {
        self.request(Method::POST, "/projects", Some(request)).await
    }

    pub async fn archive_project(&self, project_id: i64) -> Result<Project, McpError> {
        self.request::<(), _>(
            Method::POST,
            &format!("/projects/{project_id}/archive"),
            None,
        )
        .await
    }

    pub async fn set_project_completion_workflow(
        &self,
        project_id: i64,
        request: &SetProjectCompletionWorkflowRequest,
    ) -> Result<Project, McpError> {
        self.request(
            Method::PUT,
            &format!("/projects/{project_id}/completion-workflow"),
            Some(request),
        )
        .await
    }

    pub async fn create_ticket(&self, request: &CreateTicketRequest) -> Result<Ticket, McpError> {
        self.request(Method::POST, "/tickets", Some(request)).await
    }

    pub async fn ticket(&self, ticket_id: i64) -> Result<Ticket, McpError> {
        self.get(&format!("/tickets/{ticket_id}")).await
    }

    pub async fn ticket_activity(
        &self,
        ticket_id: i64,
        before_id: Option<i64>,
        limit: Option<i64>,
    ) -> Result<TicketActivityPage, McpError> {
        let mut query = Vec::new();
        if let Some(before_id) = before_id {
            query.push(format!("before_id={before_id}"));
        }
        if let Some(limit) = limit {
            query.push(format!("limit={limit}"));
        }
        let suffix = if query.is_empty() {
            String::new()
        } else {
            format!("?{}", query.join("&"))
        };
        self.get(&format!("/tickets/{ticket_id}/activity{suffix}"))
            .await
    }

    pub async fn add_ticket_comment(
        &self,
        ticket_id: i64,
        body: String,
    ) -> Result<TicketActivity, McpError> {
        self.request(
            Method::POST,
            &format!("/tickets/{ticket_id}/activity"),
            Some(&PostTicketCommentRequest {
                body,
                attachment_ids: Vec::new(),
            }),
        )
        .await
    }

    pub async fn set_external_change(
        &self,
        ticket_id: i64,
        request: &SetExternalChangeRequest,
    ) -> Result<Ticket, McpError> {
        self.request(
            Method::PUT,
            &format!("/tickets/{ticket_id}/external-change"),
            Some(request),
        )
        .await
    }

    pub async fn return_external_merge(
        &self,
        ticket_id: i64,
        request: &ReturnExternalMergeRequest,
    ) -> Result<Ticket, McpError> {
        self.request(
            Method::POST,
            &format!("/tickets/{ticket_id}/return-external-merge"),
            Some(request),
        )
        .await
    }

    pub async fn complete_external_merge(
        &self,
        ticket_id: i64,
        request: &CompleteExternalMergeRequest,
    ) -> Result<Ticket, McpError> {
        self.request(
            Method::POST,
            &format!("/tickets/{ticket_id}/complete-external-merge"),
            Some(request),
        )
        .await
    }

    pub async fn reserve_tickets(
        &self,
        request: &ReserveTicketsRequest,
    ) -> Result<Vec<Ticket>, McpError> {
        self.request(Method::POST, "/ticket-reservations", Some(request))
            .await
    }

    pub async fn update_ticket(
        &self,
        ticket_id: i64,
        request: &UpdateTicketRequest,
    ) -> Result<Ticket, McpError> {
        self.request(
            Method::PATCH,
            &format!("/tickets/{ticket_id}"),
            Some(request),
        )
        .await
    }

    pub async fn claim_ticket(
        &self,
        ticket_id: i64,
        request: &multi_agent_board_protocol::ClaimTicketRequest,
    ) -> Result<TicketClaimResponse, McpError> {
        self.request(
            Method::POST,
            &format!("/tickets/{ticket_id}/claim"),
            Some(request),
        )
        .await
    }

    pub async fn submit_review(
        &self,
        ticket_id: i64,
        request: &ReviewRevisionRequest,
    ) -> Result<Ticket, McpError> {
        self.review(ticket_id, "submit-review", request).await
    }

    pub async fn claim_review(
        &self,
        ticket_id: i64,
        request: &ReviewRevisionRequest,
    ) -> Result<TicketClaimResponse, McpError> {
        self.request(
            Method::POST,
            &format!("/tickets/{ticket_id}/claim-review"),
            Some(request),
        )
        .await
    }

    pub async fn request_changes(
        &self,
        ticket_id: i64,
        request: &RequestChangesRequest,
    ) -> Result<Ticket, McpError> {
        self.request(
            Method::POST,
            &format!("/tickets/{ticket_id}/request-changes"),
            Some(request),
        )
        .await
    }

    pub async fn approve_review(
        &self,
        ticket_id: i64,
        request: &ReviewRevisionRequest,
    ) -> Result<Ticket, McpError> {
        self.review(ticket_id, "approve-review", request).await
    }

    async fn review(
        &self,
        ticket_id: i64,
        action: &str,
        request: &ReviewRevisionRequest,
    ) -> Result<Ticket, McpError> {
        self.request(
            Method::POST,
            &format!("/tickets/{ticket_id}/{action}"),
            Some(request),
        )
        .await
    }

    pub async fn list_channels(&self) -> Result<Vec<Channel>, McpError> {
        self.get("/channels").await
    }

    pub async fn create_channel(
        &self,
        request: &CreateChannelRequest,
    ) -> Result<Channel, McpError> {
        self.request(Method::POST, "/channels", Some(request)).await
    }

    pub async fn channel(&self, name: &str) -> Result<Channel, McpError> {
        self.get(&format!("/channels/{name}")).await
    }

    pub async fn messages(
        &self,
        name: &str,
        page: &MessagePageQuery,
    ) -> Result<MessagePage, McpError> {
        let mut query = Vec::new();
        if let Some(before_id) = page.before_id {
            query.push(format!("before_id={before_id}"));
        }
        if let Some(after_id) = page.after_id {
            query.push(format!("after_id={after_id}"));
        }
        if let Some(limit) = page.limit {
            query.push(format!("limit={limit}"));
        }
        let suffix = if query.is_empty() {
            String::new()
        } else {
            format!("?{}", query.join("&"))
        };
        self.get(&format!("/channels/{name}/messages{suffix}"))
            .await
    }

    pub async fn post_message(
        &self,
        channel: &str,
        request: &PostMessageRequest,
    ) -> Result<Message, McpError> {
        self.request(
            Method::POST,
            &format!("/channels/{channel}/messages"),
            Some(request),
        )
        .await
    }

    pub async fn channel_subscriptions(&self) -> Result<Vec<String>, McpError> {
        self.get("/agent/channel-subscriptions").await
    }

    async fn change_channel_subscriptions(
        &self,
        method: Method,
        action: &str,
        request: &SetAgentChannelSubscriptionsRequest,
    ) -> Result<Vec<String>, McpError> {
        self.request(
            method,
            &format!("/agent/channel-subscriptions{action}"),
            Some(request),
        )
        .await
    }

    pub async fn add_channel_subscriptions(
        &self,
        request: &SetAgentChannelSubscriptionsRequest,
    ) -> Result<Vec<String>, McpError> {
        self.change_channel_subscriptions(Method::POST, "/add", request)
            .await
    }

    pub async fn remove_channel_subscriptions(
        &self,
        request: &SetAgentChannelSubscriptionsRequest,
    ) -> Result<Vec<String>, McpError> {
        self.change_channel_subscriptions(Method::POST, "/remove", request)
            .await
    }

    pub async fn replace_channel_subscriptions(
        &self,
        request: &SetAgentChannelSubscriptionsRequest,
    ) -> Result<Vec<String>, McpError> {
        self.change_channel_subscriptions(Method::PUT, "", request)
            .await
    }

    pub async fn project_subscriptions(&self) -> Result<Vec<i64>, McpError> {
        self.get("/agent/project-subscriptions").await
    }

    async fn change_project_subscriptions(
        &self,
        method: Method,
        action: &str,
        request: &SetAgentProjectSubscriptionsRequest,
    ) -> Result<Vec<i64>, McpError> {
        self.request(
            method,
            &format!("/agent/project-subscriptions{action}"),
            Some(request),
        )
        .await
    }

    pub async fn add_project_subscriptions(
        &self,
        request: &SetAgentProjectSubscriptionsRequest,
    ) -> Result<Vec<i64>, McpError> {
        self.change_project_subscriptions(Method::POST, "/add", request)
            .await
    }

    pub async fn remove_project_subscriptions(
        &self,
        request: &SetAgentProjectSubscriptionsRequest,
    ) -> Result<Vec<i64>, McpError> {
        self.change_project_subscriptions(Method::POST, "/remove", request)
            .await
    }

    pub async fn replace_project_subscriptions(
        &self,
        request: &SetAgentProjectSubscriptionsRequest,
    ) -> Result<Vec<i64>, McpError> {
        self.change_project_subscriptions(Method::PUT, "", request)
            .await
    }

    pub async fn schedule_reminder(
        &self,
        request: &ScheduleSelfReminderRequest,
    ) -> Result<Reminder, McpError> {
        self.request(Method::POST, "/agent/reminders", Some(request))
            .await
    }

    pub async fn reminders(&self) -> Result<Vec<Reminder>, McpError> {
        self.get("/agent/reminders").await
    }

    pub async fn cancel_reminder(&self, reminder_id: i64) -> Result<Reminder, McpError> {
        self.request::<(), _>(
            Method::DELETE,
            &format!("/agent/reminders/{reminder_id}"),
            None,
        )
        .await
    }

    pub async fn agents(&self) -> Result<Vec<Agent>, McpError> {
        self.get("/agents").await
    }

    pub async fn create_agent(&self, request: &CreateAgentRequest) -> Result<Agent, McpError> {
        self.request(Method::POST, "/agents", Some(request)).await
    }

    pub async fn rename_agent(
        &self,
        agent_id: i64,
        request: &RenameAgentRequest,
    ) -> Result<Agent, McpError> {
        self.request(Method::PATCH, &format!("/agents/{agent_id}"), Some(request))
            .await
    }

    pub async fn agent_project_worktrees(
        &self,
        agent_id: i64,
    ) -> Result<Vec<AgentProjectWorktree>, McpError> {
        self.get(&format!("/agents/{agent_id}/project-worktrees"))
            .await
    }

    pub async fn agent_messages(
        &self,
        agent_id: i64,
        page: &MessagePageQuery,
    ) -> Result<AgentMessagePage, McpError> {
        let mut query = Vec::new();
        if let Some(before_id) = page.before_id {
            query.push(format!("before_id={before_id}"));
        }
        if let Some(after_id) = page.after_id {
            query.push(format!("after_id={after_id}"));
        }
        if let Some(limit) = page.limit {
            query.push(format!("limit={limit}"));
        }
        let suffix = if query.is_empty() {
            String::new()
        } else {
            format!("?{}", query.join("&"))
        };
        self.get(&format!("/agents/{agent_id}/messages{suffix}"))
            .await
    }

    pub async fn send_agent_message(
        &self,
        agent_id: i64,
        request: &SendAgentMessageRequest,
    ) -> Result<Agent, McpError> {
        self.request(
            Method::POST,
            &format!("/agents/{agent_id}/messages"),
            Some(request),
        )
        .await
    }

    pub async fn pause_agent(&self, agent_id: i64) -> Result<Agent, McpError> {
        self.request::<(), _>(Method::POST, &format!("/agents/{agent_id}/pause"), None)
            .await
    }

    pub async fn resume_agent(&self, agent_id: i64) -> Result<Agent, McpError> {
        self.request::<(), _>(Method::POST, &format!("/agents/{agent_id}/resume"), None)
            .await
    }

    pub async fn agent_scopes(&self, agent_id: i64) -> Result<Vec<String>, McpError> {
        self.get(&format!("/agents/{agent_id}/scopes")).await
    }

    pub async fn set_agent_scopes(
        &self,
        agent_id: i64,
        request: &SetAgentScopesRequest,
    ) -> Result<Vec<String>, McpError> {
        self.request(
            Method::PUT,
            &format!("/agents/{agent_id}/scopes"),
            Some(request),
        )
        .await
    }

    pub async fn set_agent_wake_settings(
        &self,
        agent_id: i64,
        request: &SetAgentWakeSettingsRequest,
    ) -> Result<Agent, McpError> {
        self.request(
            Method::PUT,
            &format!("/agents/{agent_id}/wake-settings"),
            Some(request),
        )
        .await
    }

    pub async fn wake_events(&self, agent_id: i64) -> Result<Vec<WakeEvent>, McpError> {
        self.get(&format!("/agents/{agent_id}/wake-events")).await
    }

    pub async fn create_wake_event(
        &self,
        agent_id: i64,
        request: &CreateWakeEventRequest,
    ) -> Result<WakeEvent, McpError> {
        self.request(
            Method::POST,
            &format!("/agents/{agent_id}/wake-events"),
            Some(request),
        )
        .await
    }

    pub async fn agent_channel_subscriptions(
        &self,
        agent_id: i64,
    ) -> Result<Vec<String>, McpError> {
        self.get(&format!("/agents/{agent_id}/channel-subscriptions"))
            .await
    }

    pub async fn set_agent_channel_subscriptions(
        &self,
        agent_id: i64,
        request: &SetAgentChannelSubscriptionsRequest,
    ) -> Result<Vec<String>, McpError> {
        self.request(
            Method::PUT,
            &format!("/agents/{agent_id}/channel-subscriptions"),
            Some(request),
        )
        .await
    }

    pub async fn agent_project_subscriptions(&self, agent_id: i64) -> Result<Vec<i64>, McpError> {
        self.get(&format!("/agents/{agent_id}/project-subscriptions"))
            .await
    }

    pub async fn set_agent_project_subscriptions(
        &self,
        agent_id: i64,
        request: &SetAgentProjectSubscriptionsRequest,
    ) -> Result<Vec<i64>, McpError> {
        self.request(
            Method::PUT,
            &format!("/agents/{agent_id}/project-subscriptions"),
            Some(request),
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use std::{
        io::{Read, Write},
        net::TcpListener,
        thread,
    };

    use multi_agent_board_protocol::{TicketState, TicketSummary, TicketSummaryPage};

    use super::*;

    #[tokio::test]
    async fn typed_client_decodes_protocol_response() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let response = serde_json::to_string(&TicketSummaryPage {
            tickets: vec![TicketSummary {
                id: 47,
                project_id: 1,
                title: "Typed response".into(),
                state: TicketState::Todo,
                created_by_agent_id: None,
                reserved_for_agent_id: None,
                assignee_id: None,
                reviewer_id: None,
                review_completion_workflow: None,
                has_review_feedback: false,
                activity_count: 0,
                latest_activity_id: None,
                unresolved_blockers: Vec::new(),
                revision: 1,
                created_at: 10,
                updated_at: 10,
            }],
            has_more: false,
        })
        .unwrap();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0; 2048];
            let length = stream.read(&mut request).unwrap();
            let request = String::from_utf8_lossy(&request[..length]);
            assert!(request.starts_with("GET /tickets?limit=100 HTTP/1.1"));
            assert!(
                request
                    .to_ascii_lowercase()
                    .contains("x-multi-agent-board-agent-id: 2")
            );
            write!(
                stream,
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{response}",
                response.len()
            )
            .unwrap();
        });

        let tickets = BoardClient::new(format!("http://{address}"), 2)
            .list_tickets()
            .await
            .unwrap();
        server.join().unwrap();

        assert_eq!(tickets.len(), 1);
        assert_eq!(tickets[0].id, 47);
        assert_eq!(tickets[0].title, "Typed response");
    }

    #[tokio::test]
    async fn ticket_discovery_intentionally_reads_all_pages() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            for (expected_path, id, has_more) in [
                ("/tickets?limit=100", 101, true),
                ("/tickets?limit=100&before_id=101", 100, false),
            ] {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = [0; 2048];
                let length = stream.read(&mut request).unwrap();
                let request = String::from_utf8_lossy(&request[..length]);
                assert!(request.starts_with(&format!("GET {expected_path} HTTP/1.1")));
                let response = serde_json::json!({
                    "tickets": [{
                        "id": id,
                        "project_id": 1,
                        "title": format!("Ticket {id}"),
                        "state": "done",
                        "created_by_agent_id": null,
                        "reserved_for_agent_id": null,
                        "assignee_id": null,
                        "reviewer_id": null,
                        "has_review_feedback": false,
                        "activity_count": 0,
                        "latest_activity_id": null,
                        "unresolved_blockers": [],
                        "revision": 1,
                        "created_at": 1,
                        "updated_at": 1
                    }],
                    "has_more": has_more
                })
                .to_string();
                write!(
                    stream,
                    "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{response}",
                    response.len()
                )
                .unwrap();
            }
        });

        let tickets = BoardClient::new(format!("http://{address}"), 2)
            .list_tickets()
            .await
            .unwrap();
        server.join().unwrap();
        assert_eq!(
            tickets.iter().map(|ticket| ticket.id).collect::<Vec<_>>(),
            [101, 100]
        );
    }
}
