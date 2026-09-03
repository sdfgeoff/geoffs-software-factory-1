use reqwest::Method;
use serde_json::{Value, json};
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWrite, AsyncWriteExt};

use crate::error::AppError;

#[derive(Clone)]
struct BoardClient {
    base_url: String,
    client: reqwest::Client,
}

impl BoardClient {
    fn new(base_url: String) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').into(),
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
    ) -> Result<Value, AppError> {
        let mut request = self
            .client
            .request(method, format!("{}{path}", self.base_url));
        if let Some(body) = body {
            request = request.json(&body);
        }
        let response = request
            .send()
            .await
            .map_err(|error| AppError::internal(format!("board request failed: {error}")))?;
        let status = response.status();
        let body = response
            .json::<Value>()
            .await
            .map_err(|error| AppError::internal(format!("invalid board response: {error}")))?;
        if !status.is_success() {
            return Err(AppError::internal(
                body.get("error")
                    .and_then(Value::as_str)
                    .unwrap_or("board request failed"),
            ));
        }
        Ok(body)
    }
}

pub async fn run<R, W>(reader: R, mut writer: W, board_url: String) -> Result<(), std::io::Error>
where
    R: AsyncBufRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let board = BoardClient::new(board_url);
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
            json!({"type":"object","properties":{"ticket_id":{"type":"integer"},"title":{"type":"string"},"description":{"type":"string"},"state":{"type":"string","enum":["todo","in_progress","done"]},"assignee_id":{"type":["integer","null"]},"expected_revision":{"type":"integer"}},"required":["ticket_id"]})
        ),
        tool(
            "claim_ticket",
            "Atomically claim a ticket for an agent",
            json!({"type":"object","properties":{"ticket_id":{"type":"integer"},"agent_id":{"type":"integer"},"expected_revision":{"type":"integer"}},"required":["ticket_id","agent_id"]})
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
) -> Result<Value, AppError> {
    let name = name.ok_or_else(|| AppError::bad_request("missing tool name"))?;
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
                .ok_or_else(|| AppError::bad_request("tool arguments must be an object"))?
                .remove("ticket_id");
            board
                .request(Method::PATCH, &format!("/tickets/{ticket_id}"), Some(body))
                .await?
        }
        "claim_ticket" => {
            let ticket_id = integer(&args, "ticket_id")?;
            integer(&args, "agent_id")?;
            let mut body = args;
            body.as_object_mut()
                .ok_or_else(|| AppError::bad_request("tool arguments must be an object"))?
                .remove("ticket_id");
            board
                .request(
                    Method::POST,
                    &format!("/tickets/{ticket_id}/claim"),
                    Some(body),
                )
                .await?
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
                    Some(json!({"author": author, "body": message})),
                )
                .await?
        }
        _ => return Err(AppError::bad_request(format!("unknown tool: {name}"))),
    };
    Ok(json!({"content": [{"type": "text", "text": value.to_string()}]}))
}

fn string<'a>(args: &'a Value, name: &str) -> Result<&'a str, AppError> {
    optional_string(args, name)
        .ok_or_else(|| AppError::bad_request(format!("{name} must be a string")))
}

fn optional_string<'a>(args: &'a Value, name: &str) -> Option<&'a str> {
    args.get(name).and_then(Value::as_str)
}

fn integer(args: &Value, name: &str) -> Result<i64, AppError> {
    args.get(name)
        .and_then(Value::as_i64)
        .ok_or_else(|| AppError::bad_request(format!("{name} must be an integer")))
}

fn optional_integer(args: &Value, name: &str) -> Result<Option<i64>, AppError> {
    match args.get(name) {
        None => Ok(None),
        Some(value) => value
            .as_i64()
            .map(Some)
            .ok_or_else(|| AppError::bad_request(format!("{name} must be an integer"))),
    }
}

fn rpc_error(id: Value, code: i64, message: impl Into<String>) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message.into()}})
}

#[cfg(test)]
mod tests {
    use crate::store::{NewTicket, Store};

    use super::*;

    async fn test_board() -> (BoardClient, Store) {
        let store = Store::memory().await.unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let app = crate::api::router(store.clone());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (BoardClient::new(format!("http://{address}")), store)
    }

    #[tokio::test]
    async fn protocol_lists_and_calls_tools() {
        let (board, _) = test_board().await;
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

        let response = handle(
            &board,
            json!({
                "jsonrpc":"2.0","id":2,"method":"tools/call",
                "params":{"name":"create_ticket","arguments":{"title":"From MCP"}}
            }),
        )
        .await
        .unwrap();
        assert_eq!(response.pointer("/result/isError"), None);
        let tickets = board.request(Method::GET, "/tickets", None).await.unwrap();
        assert_eq!(tickets[0]["title"], "From MCP");
    }

    #[tokio::test]
    async fn claim_is_atomic() {
        let (board, store) = test_board().await;
        store.create_agent("first", "/tmp/first").await.unwrap();
        store.create_agent("second", "/tmp/second").await.unwrap();
        let ticket = store
            .create_ticket(NewTicket {
                title: "Claim me".into(),
                description: String::new(),
            })
            .await
            .unwrap();
        let first = call_tool(
            &board,
            Some("claim_ticket"),
            Some(&json!({"ticket_id": ticket.id, "agent_id": 1, "expected_revision": 1})),
        )
        .await;
        assert!(first.is_ok());
        let second = call_tool(
            &board,
            Some("claim_ticket"),
            Some(&json!({"ticket_id": ticket.id, "agent_id": 2})),
        )
        .await;
        assert!(second.is_err());
    }
}
