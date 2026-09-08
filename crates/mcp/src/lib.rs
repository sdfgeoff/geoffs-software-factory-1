mod client;
pub mod tool;
mod tools;

pub use client::{BoardClient, McpError};

use serde_json::{Value, json};
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWrite, AsyncWriteExt};

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
    let registry = tools::registry().expect("tool names are unique");
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
        if let Some(response) = handle(&board, &registry, request).await {
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

async fn handle(
    board: &BoardClient,
    registry: &tool::ToolRegistry,
    request: Value,
) -> Option<Value> {
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
        "tools/list" => Ok(json!({"tools": registry.definitions()})),
        "tools/call" => {
            call_tool(
                board,
                registry,
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

async fn call_tool(
    board: &BoardClient,
    registry: &tool::ToolRegistry,
    name: Option<&str>,
    arguments: Option<&Value>,
) -> Result<Value, McpError> {
    let name = name.ok_or_else(|| McpError::bad_request("missing tool name"))?;
    let arguments = arguments.cloned().unwrap_or_else(|| json!({}));
    let value = registry.call(board, name, arguments).await?;
    Ok(json!({"content": [{"type": "text", "text": value.to_string()}]}))
}

fn rpc_error(id: Value, code: i64, message: impl Into<String>) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message.into()}})
}

#[cfg(test)]
mod tests {
    use super::*;
    use multi_agent_board_protocol::Ticket;
    use std::{
        io::{Read, Write},
        net::TcpListener,
        thread,
    };

    #[tokio::test]
    async fn protocol_lists_tools_without_http() {
        let board = BoardClient::new("http://127.0.0.1:1".into(), 1);
        let registry = tools::registry().unwrap();
        let response = handle(
            &board,
            &registry,
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
