use reqwest::Method;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWrite, AsyncWriteExt};

use multi_agent_board_protocol::{
    AGENT_ID_HEADER, Agent, AgentMessagePage, AgentProjectWorktree, BoardSettings, Channel,
    ClaimTicketRequest, CreateAgentRequest, CreateChannelRequest, CreateTicketRequest,
    CreateWakeEventRequest, ErrorResponse, Message, MessagePage, MessagePageQuery,
    PostMessageRequest, Project, RegisterProjectRequest, Reminder, RenameAgentRequest,
    RequestChangesRequest, ReserveTicketsRequest, ReviewRevisionRequest,
    ScheduleSelfReminderRequest, SendAgentMessageRequest, SetAgentChannelSubscriptionsRequest,
    SetAgentProjectSubscriptionsRequest, SetAgentScopesRequest, SetAgentWakeSettingsRequest,
    SetBoardSettingsRequest, SetProjectRequest, Ticket, TicketActivity, TicketClaimResponse,
    UpdateTicketRequest, WakeEvent,
};

#[derive(Debug)]
pub struct McpError(String);

impl McpError {
    fn bad_request(message: impl Into<String>) -> Self {
        Self(message.into())
    }

    fn internal(message: impl Into<String>) -> Self {
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

pub async fn run<R, W>(
    reader: R,
    mut writer: W,
    board_url: String,
    agent_id: i64,
) -> Result<(), std::io::Error>
where
    R: AsyncBufRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let board = BoardClient::new(board_url, agent_id);
    let mut lines = reader.lines();
    while let Some(line) = lines.next_line().await? {
        let request = match serde_json::from_str(&line) {
            Ok(request) => request,
            Err(error) => {
                write_message(
                    &mut writer,
                    rpc_error(Value::Null, -32700, error.to_string()),
                )
                .await?;
                continue;
            }
        };
        if let Some(response) = handle(&board, request).await {
            write_message(&mut writer, response).await?;
        }
    }
    Ok(())
}

async fn write_message<W: AsyncWrite + Unpin>(
    writer: &mut W,
    message: Value,
) -> Result<(), std::io::Error> {
    writer.write_all(message.to_string().as_bytes()).await?;
    writer.write_all(b"\n").await?;
    writer.flush().await
}

async fn handle(board: &BoardClient, request: Value) -> Option<Value> {
    let id = request.get("id")?.clone();
    let method = request.get("method").and_then(Value::as_str)?;
    let result = match method {
        "initialize" => Ok(json!({
            "protocolVersion": request.pointer("/params/protocolVersion")
                .and_then(Value::as_str).unwrap_or("2025-06-18"),
            "capabilities": {"tools": {"listChanged": false}},
            "serverInfo": {"name": "multi-agent-board", "version": env!("CARGO_PKG_VERSION")}
        })),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({"tools": tools()})),
        "tools/call" => {
            call_tool(
                board,
                request.pointer("/params/name").and_then(Value::as_str),
                request.pointer("/params/arguments"),
            )
            .await
        }
        _ => return Some(rpc_error(id, -32601, format!("unknown method: {method}"))),
    };
    Some(match result {
        Ok(result) => json!({"jsonrpc": "2.0", "id": id, "result": result}),
        Err(error) => json!({
            "jsonrpc": "2.0", "id": id,
            "result": {"content": [{"type": "text", "text": error.to_string()}], "isError": true}
        }),
    })
}

fn tools() -> Value {
    json!([
        tool(
            "list_tickets",
            "List all board tickets",
            json!({"type":"object","properties":{}})
        ),
        tool(
            "list_projects",
            "List registered project roots",
            json!({"type":"object","properties":{}})
        ),
        tool(
            "register_project",
            "Register a Git project root",
            json!({"type":"object","properties":{"name":{"type":"string"},"path":{"type":"string"},"default_branch":{"type":"string"}},"required":["name","path","default_branch"]})
        ),
        tool(
            "archive_project",
            "Archive a project with no active work",
            json!({"type":"object","properties":{"project_id":{"type":"integer"}},"required":["project_id"]})
        ),
        tool(
            "create_ticket",
            "Create a board ticket",
            json!({"type":"object","properties":{"project_id":{"type":"integer"},"title":{"type":"string"},"description":{"type":"string"},"blocker_ids":{"type":"array","items":{"type":"integer"}}},"required":["title"]})
        ),
        tool(
            "get_ticket_details",
            "Get one ticket",
            json!({"type":"object","properties":{"ticket_id":{"type":"integer"}},"required":["ticket_id"]})
        ),
        tool(
            "get_ticket_activity",
            "List a ticket's activity log",
            json!({"type":"object","properties":{"ticket_id":{"type":"integer"}},"required":["ticket_id"]})
        ),
        tool(
            "reserve_tickets",
            "Reserve todo tickets for the calling agent without claiming them",
            json!({"type":"object","properties":{"ticket_ids":{"type":"array","items":{"type":"integer"},"minItems":1,"uniqueItems":true}},"required":["ticket_ids"]})
        ),
        tool(
            "set_ticket_details",
            "Update ticket fields",
            json!({"type":"object","properties":{"ticket_id":{"type":"integer"},"title":{"type":"string"},"description":{"type":"string"},"state":{"type":"string","enum":["todo","in_progress"]},"assignee_id":{"type":["integer","null"]},"blocker_ids":{"type":"array","items":{"type":"integer"}},"expected_revision":{"type":"integer"}},"required":["ticket_id"]})
        ),
        tool(
            "claim_ticket",
            "Atomically claim a ticket for this agent",
            json!({"type":"object","properties":{"ticket_id":{"type":"integer"},"expected_revision":{"type":"integer"}},"required":["ticket_id"]})
        ),
        tool(
            "submit_ticket_for_review",
            "Submit this agent's ticket commit for review",
            json!({"type":"object","properties":{"ticket_id":{"type":"integer"},"expected_revision":{"type":"integer"}},"required":["ticket_id"]})
        ),
        tool(
            "claim_ticket_review",
            "Claim a review; requires approve_review scope",
            json!({"type":"object","properties":{"ticket_id":{"type":"integer"},"expected_revision":{"type":"integer"}},"required":["ticket_id"]})
        ),
        tool(
            "request_ticket_changes",
            "Return a claimed review to its worker with feedback; requires approve_review scope",
            json!({"type":"object","properties":{"ticket_id":{"type":"integer"},"feedback":{"type":"string"},"expected_revision":{"type":"integer"}},"required":["ticket_id","feedback"]})
        ),
        tool(
            "approve_ticket_review",
            "Approve and merge a claimed review; requires approve_review scope",
            json!({"type":"object","properties":{"ticket_id":{"type":"integer"},"expected_revision":{"type":"integer"}},"required":["ticket_id"]})
        ),
        tool(
            "list_message_board_channels",
            "List channels with descriptions and latest message times",
            json!({"type":"object","properties":{}})
        ),
        tool(
            "create_message_board_channel",
            "Create a message-board channel",
            json!({"type":"object","properties":{"name":{"type":"string"},"description":{"type":"string"}},"required":["name"]})
        ),
        tool(
            "list_my_channel_subscriptions",
            "List this agent's message-board channel subscriptions",
            json!({"type":"object","properties":{}})
        ),
        tool(
            "add_my_channel_subscriptions",
            "Add message-board channel subscriptions for this agent",
            channel_list_schema()
        ),
        tool(
            "remove_my_channel_subscriptions",
            "Remove message-board channel subscriptions for this agent",
            channel_list_schema()
        ),
        tool(
            "replace_my_channel_subscriptions",
            "Replace all message-board channel subscriptions for this agent",
            channel_list_schema()
        ),
        tool(
            "list_my_project_subscriptions",
            "List this agent's preferred project roots",
            json!({"type":"object","properties":{}})
        ),
        tool(
            "add_my_project_subscriptions",
            "Add preferred project roots for this agent",
            project_list_schema()
        ),
        tool(
            "remove_my_project_subscriptions",
            "Remove preferred project roots for this agent",
            project_list_schema()
        ),
        tool(
            "replace_my_project_subscriptions",
            "Replace all preferred project roots for this agent",
            project_list_schema()
        ),
        tool(
            "read_message_board_channel",
            "Read messages in an optional inclusive time range",
            json!({"type":"object","properties":{"channel_name":{"type":"string"},"before_id":{"type":"integer","minimum":1},"after_id":{"type":"integer","minimum":1},"limit":{"type":"integer","minimum":1,"maximum":100}},"required":["channel_name"]})
        ),
        tool(
            "post_to_message_board",
            "Post a message to an existing channel",
            json!({"type":"object","properties":{"channel_name":{"type":"string"},"author":{"type":"string"},"message":{"type":"string"}},"required":["channel_name","author","message"]})
        ),
        tool(
            "schedule_self_reminder",
            "Schedule a durable one-shot reminder for this agent",
            json!({
                "type":"object",
                "properties":{
                    "message":{"type":"string","minLength":1,"maxLength":50000},
                    "deliver_at":{"type":"string","format":"date-time"},
                    "delay_seconds":{"type":"integer","minimum":1,"maximum":31536000}
                },
                "required":["message"],
                "oneOf":[{"required":["deliver_at"]},{"required":["delay_seconds"]}]
            })
        ),
        tool(
            "list_my_reminders",
            "List this agent's reminders",
            json!({"type":"object","properties":{}})
        ),
        tool(
            "cancel_my_reminder",
            "Cancel one of this agent's scheduled reminders",
            json!({"type":"object","properties":{"reminder_id":{"type":"integer"}},"required":["reminder_id"]})
        )
    ])
}

fn tool(name: &str, description: &str, input_schema: Value) -> Value {
    json!({"name": name, "description": description, "inputSchema": input_schema})
}

fn channel_list_schema() -> Value {
    json!({"type":"object","properties":{"channels":{"type":"array","items":{"type":"string"}}},"required":["channels"]})
}

fn project_list_schema() -> Value {
    json!({"type":"object","properties":{"project_ids":{"type":"array","items":{"type":"integer"}}},"required":["project_ids"]})
}

#[derive(Deserialize)]
struct ProjectIdRequest {
    project_id: i64,
}

#[derive(Deserialize)]
struct TicketIdRequest {
    ticket_id: i64,
}

#[derive(Deserialize)]
struct SetTicketDetailsRequest {
    ticket_id: i64,
    #[serde(flatten)]
    changes: UpdateTicketRequest,
}

#[derive(Deserialize)]
struct TicketRevisionRequest {
    ticket_id: i64,
    expected_revision: Option<i64>,
}

#[derive(Deserialize)]
struct TicketChangesRequest {
    ticket_id: i64,
    feedback: String,
    expected_revision: Option<i64>,
}

#[derive(Deserialize)]
struct ReadChannelRequest {
    channel_name: String,
    before_id: Option<i64>,
    after_id: Option<i64>,
    limit: Option<i64>,
}

#[derive(Serialize)]
struct ReadChannelResponse {
    channel: Channel,
    messages: Vec<Message>,
    has_more: bool,
}

#[derive(Deserialize)]
struct PostBoardMessageRequest {
    channel_name: String,
    author: String,
    message: String,
}

#[derive(Deserialize)]
struct ReminderIdRequest {
    reminder_id: i64,
}

async fn call_tool(
    board: &BoardClient,
    name: Option<&str>,
    arguments: Option<&Value>,
) -> Result<Value, McpError> {
    let name = name.ok_or_else(|| McpError::bad_request("missing tool name"))?;
    let args = arguments.cloned().unwrap_or_else(|| json!({}));
    let value = match name {
        "list_tickets" => encode(board.list_tickets().await?)?,
        "list_projects" => encode(board.list_projects().await?)?,
        "register_project" => encode(board.register_project(&decode(&args)?).await?)?,
        "archive_project" => {
            let request: ProjectIdRequest = decode(&args)?;
            encode(board.archive_project(request.project_id).await?)?
        }
        "create_ticket" => encode(board.create_ticket(&decode(&args)?).await?)?,
        "get_ticket_details" => {
            let request: TicketIdRequest = decode(&args)?;
            encode(board.ticket(request.ticket_id).await?)?
        }
        "get_ticket_activity" => {
            let request: TicketIdRequest = decode(&args)?;
            encode(board.ticket_activity(request.ticket_id).await?)?
        }
        "reserve_tickets" => encode(board.reserve_tickets(&decode(&args)?).await?)?,
        "set_ticket_details" => {
            let request: SetTicketDetailsRequest = decode(&args)?;
            encode(
                board
                    .update_ticket(request.ticket_id, &request.changes)
                    .await?,
            )?
        }
        "claim_ticket" => {
            let request: TicketRevisionRequest = decode(&args)?;
            encode(
                board
                    .claim_ticket(
                        request.ticket_id,
                        &ClaimTicketRequest {
                            agent_id: None,
                            expected_revision: request.expected_revision,
                        },
                    )
                    .await?,
            )?
        }
        "submit_ticket_for_review" => {
            let request: TicketRevisionRequest = decode(&args)?;
            encode(
                board
                    .submit_review(
                        request.ticket_id,
                        &ReviewRevisionRequest {
                            expected_revision: request.expected_revision,
                        },
                    )
                    .await?,
            )?
        }
        "claim_ticket_review" => {
            let request: TicketRevisionRequest = decode(&args)?;
            encode(
                board
                    .claim_review(
                        request.ticket_id,
                        &ReviewRevisionRequest {
                            expected_revision: request.expected_revision,
                        },
                    )
                    .await?,
            )?
        }
        "request_ticket_changes" => {
            let request: TicketChangesRequest = decode(&args)?;
            encode(
                board
                    .request_changes(
                        request.ticket_id,
                        &RequestChangesRequest {
                            feedback: request.feedback,
                            expected_revision: request.expected_revision,
                        },
                    )
                    .await?,
            )?
        }
        "approve_ticket_review" => {
            let request: TicketRevisionRequest = decode(&args)?;
            encode(
                board
                    .approve_review(
                        request.ticket_id,
                        &ReviewRevisionRequest {
                            expected_revision: request.expected_revision,
                        },
                    )
                    .await?,
            )?
        }
        "list_message_board_channels" => encode(board.list_channels().await?)?,
        "create_message_board_channel" => encode(board.create_channel(&decode(&args)?).await?)?,
        "list_my_channel_subscriptions" => encode(board.channel_subscriptions().await?)?,
        "add_my_channel_subscriptions" => {
            encode(board.add_channel_subscriptions(&decode(&args)?).await?)?
        }
        "remove_my_channel_subscriptions" => {
            encode(board.remove_channel_subscriptions(&decode(&args)?).await?)?
        }
        "replace_my_channel_subscriptions" => {
            encode(board.replace_channel_subscriptions(&decode(&args)?).await?)?
        }
        "list_my_project_subscriptions" => encode(board.project_subscriptions().await?)?,
        "add_my_project_subscriptions" => {
            encode(board.add_project_subscriptions(&decode(&args)?).await?)?
        }
        "remove_my_project_subscriptions" => {
            encode(board.remove_project_subscriptions(&decode(&args)?).await?)?
        }
        "replace_my_project_subscriptions" => {
            encode(board.replace_project_subscriptions(&decode(&args)?).await?)?
        }
        "read_message_board_channel" => {
            let request: ReadChannelRequest = decode(&args)?;
            let page = board
                .messages(
                    &request.channel_name,
                    &MessagePageQuery {
                        before_id: request.before_id,
                        after_id: request.after_id,
                        limit: request.limit,
                    },
                )
                .await?;
            encode(ReadChannelResponse {
                channel: board.channel(&request.channel_name).await?,
                messages: page.messages,
                has_more: page.has_more,
            })?
        }
        "post_to_message_board" => {
            let request: PostBoardMessageRequest = decode(&args)?;
            encode(
                board
                    .post_message(
                        &request.channel_name,
                        &PostMessageRequest {
                            author: request.author,
                            body: request.message,
                        },
                    )
                    .await?,
            )?
        }
        "schedule_self_reminder" => encode(board.schedule_reminder(&decode(&args)?).await?)?,
        "list_my_reminders" => encode(board.reminders().await?)?,
        "cancel_my_reminder" => {
            let request: ReminderIdRequest = decode(&args)?;
            encode(board.cancel_reminder(request.reminder_id).await?)?
        }
        _ => return Err(McpError::bad_request(format!("unknown tool: {name}"))),
    };
    Ok(json!({"content": [{"type": "text", "text": value.to_string()}]}))
}

fn decode<T: DeserializeOwned>(value: &Value) -> Result<T, McpError> {
    serde_json::from_value(value.clone())
        .map_err(|error| McpError::bad_request(format!("invalid tool arguments: {error}")))
}

fn encode<T: Serialize>(value: T) -> Result<Value, McpError> {
    serde_json::to_value(value)
        .map_err(|error| McpError::internal(format!("invalid typed response: {error}")))
}

fn rpc_error(id: Value, code: i64, message: impl Into<String>) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message.into()}})
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{Read, Write},
        net::TcpListener,
        thread,
    };

    #[tokio::test]
    async fn protocol_lists_tools_without_http() {
        let board = BoardClient::new("http://127.0.0.1:1".into(), 1);
        let response = handle(
            &board,
            json!({"jsonrpc":"2.0","id":1,"method":"tools/list"}),
        )
        .await
        .unwrap();
        assert!(
            response
                .pointer("/result/tools")
                .unwrap()
                .as_array()
                .unwrap()
                .len()
                >= 8
        );
    }

    #[tokio::test]
    async fn typed_client_decodes_protocol_response() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let response = serde_json::to_string(&vec![Ticket {
            id: 47,
            project_id: 1,
            title: "Typed response".into(),
            description: String::new(),
            state: multi_agent_board_protocol::TicketState::Todo,
            created_by_agent_id: None,
            reserved_for_agent_id: None,
            assignee_id: None,
            implementation_branch: None,
            review_commit: None,
            review_target_branch: None,
            review_project_id: None,
            review_project_path: None,
            reviewer_id: None,
            review_feedback: None,
            blocker_ids: Vec::new(),
            unresolved_blockers: Vec::new(),
            revision: 1,
            created_at: 10,
            updated_at: 10,
        }])
        .unwrap();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0; 2048];
            let length = stream.read(&mut request).unwrap();
            let request = String::from_utf8_lossy(&request[..length]);
            assert!(request.starts_with("GET /tickets HTTP/1.1"));
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
}
