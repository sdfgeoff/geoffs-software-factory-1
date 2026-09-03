use serde_json::{Value, json};
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWrite, AsyncWriteExt};

use crate::{
    domain::TicketState,
    error::AppError,
    store::{NewTicket, Store, TicketChanges},
};

pub async fn run<R, W>(reader: R, mut writer: W, store: Store) -> Result<(), std::io::Error>
where
    R: AsyncBufRead + Unpin,
    W: AsyncWrite + Unpin,
{
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
        if let Some(response) = handle(&store, request).await {
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

async fn handle(store: &Store, request: Value) -> Option<Value> {
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
                store,
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
    store: &Store,
    name: Option<&str>,
    arguments: Option<&Value>,
) -> Result<Value, AppError> {
    let name = name.ok_or_else(|| AppError::bad_request("missing tool name"))?;
    let args = arguments.cloned().unwrap_or_else(|| json!({}));
    let value = match name {
        "list_tickets" => serde_json::to_value(store.tickets().await?).unwrap(),
        "create_ticket" => serde_json::to_value(
            store
                .create_ticket(NewTicket {
                    title: string(&args, "title")?.into(),
                    description: optional_string(&args, "description")
                        .unwrap_or_default()
                        .into(),
                })
                .await?,
        )
        .unwrap(),
        "get_ticket_details" => serde_json::to_value(
            store
                .ticket(integer(&args, "ticket_id")?)
                .await?
                .ok_or_else(|| AppError::not_found("ticket not found"))?,
        )
        .unwrap(),
        "set_ticket_details" => {
            let assignee_id = args
                .get("assignee_id")
                .map(|value| {
                    if value.is_null() {
                        Ok(None)
                    } else {
                        value.as_i64().map(Some).ok_or_else(|| {
                            AppError::bad_request("assignee_id must be an integer or null")
                        })
                    }
                })
                .transpose()?;
            serde_json::to_value(
                store
                    .update_ticket(
                        integer(&args, "ticket_id")?,
                        TicketChanges {
                            title: optional_string(&args, "title").map(Into::into),
                            description: optional_string(&args, "description").map(Into::into),
                            state: optional_string(&args, "state")
                                .map(parse_state)
                                .transpose()?,
                            assignee_id,
                            expected_revision: optional_integer(&args, "expected_revision")?,
                        },
                    )
                    .await?,
            )
            .unwrap()
        }
        "claim_ticket" => serde_json::to_value(
            store
                .claim_ticket(
                    integer(&args, "ticket_id")?,
                    integer(&args, "agent_id")?,
                    optional_integer(&args, "expected_revision")?,
                )
                .await?,
        )
        .unwrap(),
        "list_message_board_channels" => serde_json::to_value(store.channels().await?).unwrap(),
        "read_message_board_channel" => serde_json::to_value(
            store
                .messages(
                    string(&args, "channel_name")?,
                    optional_integer(&args, "start_time")?,
                    optional_integer(&args, "end_time")?,
                )
                .await?,
        )
        .unwrap(),
        "post_to_message_board" => serde_json::to_value(
            store
                .post_message(
                    string(&args, "channel_name")?,
                    string(&args, "author")?,
                    string(&args, "message")?,
                )
                .await?,
        )
        .unwrap(),
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

fn parse_state(value: &str) -> Result<TicketState, AppError> {
    match value {
        "todo" => Ok(TicketState::Todo),
        "in_progress" => Ok(TicketState::InProgress),
        "done" => Ok(TicketState::Done),
        _ => Err(AppError::bad_request("invalid ticket state")),
    }
}

fn rpc_error(id: Value, code: i64, message: impl Into<String>) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message.into()}})
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn protocol_lists_and_calls_tools() {
        let store = Store::memory().await.unwrap();
        let response = handle(
            &store,
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
            &store,
            json!({
                "jsonrpc":"2.0","id":2,"method":"tools/call",
                "params":{"name":"create_ticket","arguments":{"title":"From MCP"}}
            }),
        )
        .await
        .unwrap();
        assert_eq!(response.pointer("/result/isError"), None);
        assert_eq!(store.tickets().await.unwrap()[0].title, "From MCP");
    }

    #[tokio::test]
    async fn claim_is_atomic() {
        let store = Store::memory().await.unwrap();
        let ticket = store
            .create_ticket(NewTicket {
                title: "Claim me".into(),
                description: String::new(),
            })
            .await
            .unwrap();
        store.claim_ticket(ticket.id, 1, Some(1)).await.unwrap();
        assert!(store.claim_ticket(ticket.id, 2, None).await.is_err());
    }
}
