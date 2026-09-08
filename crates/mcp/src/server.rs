use serde_json::{Value, json};
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWrite, AsyncWriteExt};

use crate::{BoardClient, McpError, tool, tools};

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

    #[tokio::test]
    async fn protocol_lists_registered_tools_without_http() {
        let board = BoardClient::new("http://127.0.0.1:1".into(), 1);
        let registry = tools::registry().unwrap();
        let response = handle(
            &board,
            &registry,
            json!({"jsonrpc":"2.0","id":1,"method":"tools/list"}),
        )
        .await
        .unwrap();

        assert_eq!(
            response
                .pointer("/result/tools")
                .unwrap()
                .as_array()
                .unwrap()
                .len(),
            registry.definitions().len()
        );
    }
}
