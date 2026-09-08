use reqwest::Method;
use serde::{Serialize, de::DeserializeOwned};

use multi_agent_board_protocol::{
    AGENT_ID_HEADER, Agent, AgentMessagePage, AgentProjectWorktree, BoardSettings, Channel,
    CreateAgentRequest, CreateChannelRequest, CreateTicketRequest, CreateWakeEventRequest,
    ErrorResponse, Message, MessagePage, MessagePageQuery, PostMessageRequest, Project,
    RegisterProjectRequest, Reminder, RenameAgentRequest, RequestChangesRequest,
    ReserveTicketsRequest, ReviewRevisionRequest, ScheduleSelfReminderRequest,
    SendAgentMessageRequest, SetAgentChannelSubscriptionsRequest,
    SetAgentProjectSubscriptionsRequest, SetAgentScopesRequest, SetAgentWakeSettingsRequest,
    SetBoardSettingsRequest, SetProjectRequest, Ticket, TicketActivity, TicketClaimResponse,
    UpdateTicketRequest, WakeEvent,
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

    pub async fn list_tickets(&self) -> Result<Vec<Ticket>, McpError> {
        self.get("/tickets").await
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

    pub async fn project(&self) -> Result<Option<Project>, McpError> {
        self.get("/project").await
    }

    pub async fn set_project(&self, request: &SetProjectRequest) -> Result<Project, McpError> {
        self.request(Method::PUT, "/project", Some(request)).await
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

    pub async fn create_ticket(&self, request: &CreateTicketRequest) -> Result<Ticket, McpError> {
        self.request(Method::POST, "/tickets", Some(request)).await
    }

    pub async fn ticket(&self, ticket_id: i64) -> Result<Ticket, McpError> {
        self.get(&format!("/tickets/{ticket_id}")).await
    }

    pub async fn ticket_activity(&self, ticket_id: i64) -> Result<Vec<TicketActivity>, McpError> {
        self.get(&format!("/tickets/{ticket_id}/activity")).await
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
