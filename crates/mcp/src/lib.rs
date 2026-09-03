use reqwest::Method;
use serde_json::{Value, json};
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWrite, AsyncWriteExt};

use multi_agent_board_protocol::{AGENT_ID_HEADER, ErrorResponse, PostMessageRequest};

#[derive(Debug)]
struct McpError(String);

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
struct BoardClient {
    base_url: String,
    agent_id: i64,
    client: reqwest::Client,
}

impl BoardClient {
    fn new(base_url: String, agent_id: i64) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').into(),
            agent_id,
            client: reqwest::Client::builder()
                .no_proxy()
                .build()
                .expect("HTTP client configuration is valid"),
        }
    }

    async fn request(
        &self,
        method: Method,
        path: &str,
        body: Option<Value>,
    ) -> Result<Value, McpError> {
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
            .json::<Value>()
            .await
            .map_err(|error| McpError::internal(format!("invalid board response: {error}")))?;
        if !status.is_success() {
            let error = serde_json::from_value::<ErrorResponse>(body)
                .map(|body| body.error)
                .unwrap_or_else(|_| "board request failed".into());
            return Err(McpError::internal(error));
        }
        Ok(body)
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
            "create_ticket",
            "Create a board ticket",
            json!({"type":"object","properties":{"title":{"type":"string"},"description":{"type":"string"}},"required":["title"]})
        ),
        tool(
            "get_ticket_details",
            "Get one ticket",
            json!({"type":"object","properties":{"ticket_id":{"type":"integer"}},"required":["ticket_id"]})
        ),
        tool(
            "set_ticket_details",
            "Update ticket fields",
            json!({"type":"object","properties":{"ticket_id":{"type":"integer"},"title":{"type":"string"},"description":{"type":"string"},"state":{"type":"string","enum":["todo","in_progress"]},"assignee_id":{"type":["integer","null"]},"expected_revision":{"type":"integer"}},"required":["ticket_id"]})
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
            "List channels with latest message times",
            json!({"type":"object","properties":{}})
        ),
        tool(
            "read_message_board_channel",
            "Read messages in an optional inclusive time range",
            json!({"type":"object","properties":{"channel_name":{"type":"string"},"start_time":{"type":"integer"},"end_time":{"type":"integer"}},"required":["channel_name"]})
        ),
        tool(
            "post_to_message_board",
            "Post a message, creating the channel if needed",
            json!({"type":"object","properties":{"channel_name":{"type":"string"},"author":{"type":"string"},"message":{"type":"string"}},"required":["channel_name","author","message"]})
        )
    ])
}

fn tool(name: &str, description: &str, input_schema: Value) -> Value {
    json!({"name": name, "description": description, "inputSchema": input_schema})
}

async fn call_tool(
    board: &BoardClient,
    name: Option<&str>,
    arguments: Option<&Value>,
) -> Result<Value, McpError> {
    let name = name.ok_or_else(|| McpError::bad_request("missing tool name"))?;
    let args = arguments.cloned().unwrap_or_else(|| json!({}));
    let value = match name {
        "list_tickets" => board.request(Method::GET, "/tickets", None).await?,
        "create_ticket" => {
            string(&args, "title")?;
            board.request(Method::POST, "/tickets", Some(args)).await?
        }
        "get_ticket_details" => {
            let ticket_id = integer(&args, "ticket_id")?;
            board
                .request(Method::GET, &format!("/tickets/{ticket_id}"), None)
                .await?
        }
        "set_ticket_details" => {
            let ticket_id = integer(&args, "ticket_id")?;
            let mut body = args;
            body.as_object_mut()
                .ok_or_else(|| McpError::bad_request("tool arguments must be an object"))?
                .remove("ticket_id");
            board
                .request(Method::PATCH, &format!("/tickets/{ticket_id}"), Some(body))
                .await?
        }
        "claim_ticket" => {
            let ticket_id = integer(&args, "ticket_id")?;
            let mut body = args;
            body.as_object_mut()
                .ok_or_else(|| McpError::bad_request("tool arguments must be an object"))?
                .remove("ticket_id");
            board
                .request(
                    Method::POST,
                    &format!("/tickets/{ticket_id}/claim"),
                    Some(body),
                )
                .await?
        }
        "submit_ticket_for_review" => {
            review_request(board, Method::POST, &args, "submit-review").await?
        }
        "claim_ticket_review" => review_request(board, Method::POST, &args, "claim-review").await?,
        "request_ticket_changes" => {
            review_request(board, Method::POST, &args, "request-changes").await?
        }
        "approve_ticket_review" => {
            review_request(board, Method::POST, &args, "approve-review").await?
        }
        "list_message_board_channels" => board.request(Method::GET, "/channels", None).await?,
        "read_message_board_channel" => {
            let channel = string(&args, "channel_name")?;
            let mut query = Vec::new();
            if let Some(start) = optional_integer(&args, "start_time")? {
                query.push(format!("start_time={start}"));
            }
            if let Some(end) = optional_integer(&args, "end_time")? {
                query.push(format!("end_time={end}"));
            }
            let suffix = if query.is_empty() {
                String::new()
            } else {
                format!("?{}", query.join("&"))
            };
            board
                .request(
                    Method::GET,
                    &format!("/channels/{channel}/messages{suffix}"),
                    None,
                )
                .await?
        }
        "post_to_message_board" => {
            let channel = string(&args, "channel_name")?.to_owned();
            let author = string(&args, "author")?;
            let message = string(&args, "message")?;
            board
                .request(
                    Method::POST,
                    &format!("/channels/{channel}/messages"),
                    Some(
                        serde_json::to_value(PostMessageRequest {
                            author: author.into(),
                            body: message.into(),
                        })
                        .expect("message request is serializable"),
                    ),
                )
                .await?
        }
        _ => return Err(McpError::bad_request(format!("unknown tool: {name}"))),
    };
    Ok(json!({"content": [{"type": "text", "text": value.to_string()}]}))
}

async fn review_request(
    board: &BoardClient,
    method: Method,
    args: &Value,
    action: &str,
) -> Result<Value, McpError> {
    let ticket_id = integer(args, "ticket_id")?;
    let mut body = args.clone();
    body.as_object_mut()
        .ok_or_else(|| McpError::bad_request("tool arguments must be an object"))?
        .remove("ticket_id");
    board
        .request(
            method,
            &format!("/tickets/{ticket_id}/{action}"),
            Some(body),
        )
        .await
}

fn string<'a>(args: &'a Value, name: &str) -> Result<&'a str, McpError> {
    optional_string(args, name)
        .ok_or_else(|| McpError::bad_request(format!("{name} must be a string")))
}

fn optional_string<'a>(args: &'a Value, name: &str) -> Option<&'a str> {
    args.get(name).and_then(Value::as_str)
}

fn integer(args: &Value, name: &str) -> Result<i64, McpError> {
    args.get(name)
        .and_then(Value::as_i64)
        .ok_or_else(|| McpError::bad_request(format!("{name} must be an integer")))
}

fn optional_integer(args: &Value, name: &str) -> Result<Option<i64>, McpError> {
    match args.get(name) {
        None => Ok(None),
        Some(value) => value
            .as_i64()
            .map(Some)
            .ok_or_else(|| McpError::bad_request(format!("{name} must be an integer"))),
    }
}

fn rpc_error(id: Value, code: i64, message: impl Into<String>) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message.into()}})
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
