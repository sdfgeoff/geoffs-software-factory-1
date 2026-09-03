use multi_agent_board::store::Store;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

async fn mcp_request(base_url: &str, agent_id: i64, request: Value) -> Value {
    let (mut request_writer, request_reader) = tokio::io::duplex(4096);
    let (response_writer, response_reader) = tokio::io::duplex(4096);
    let task = tokio::spawn(multi_agent_board_mcp::run(
        BufReader::new(request_reader),
        response_writer,
        base_url.into(),
        agent_id,
    ));

    request_writer
        .write_all(format!("{request}\n").as_bytes())
        .await
        .unwrap();
    request_writer.shutdown().await.unwrap();

    let mut response = String::new();
    BufReader::new(response_reader)
        .read_line(&mut response)
        .await
        .unwrap();
    task.await.unwrap().unwrap();
    serde_json::from_str(&response).unwrap()
}

#[tokio::test]
async fn mcp_calls_the_server_without_database_access() {
    let store = Store::memory().await.unwrap();
    store.create_agent("first", "/tmp/first").await.unwrap();
    store.create_agent("second", "/tmp/second").await.unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let app = multi_agent_board::api::router(store.clone());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let base_url = format!("http://{address}");

    let created = mcp_request(
        &base_url,
        1,
        json!({
            "jsonrpc":"2.0","id":1,"method":"tools/call",
            "params":{"name":"create_ticket","arguments":{"title":"From MCP"}}
        }),
    )
    .await;
    assert_eq!(created.pointer("/result/isError"), None);

    let claimed = mcp_request(
        &base_url,
        1,
        json!({
            "jsonrpc":"2.0","id":2,"method":"tools/call",
            "params":{"name":"claim_ticket","arguments":{"ticket_id":1,"expected_revision":1}}
        }),
    )
    .await;
    assert_eq!(claimed.pointer("/result/isError"), None);

    let conflict = mcp_request(
        &base_url,
        2,
        json!({
            "jsonrpc":"2.0","id":3,"method":"tools/call",
            "params":{"name":"claim_ticket","arguments":{"ticket_id":1}}
        }),
    )
    .await;
    assert_eq!(
        conflict.pointer("/result/isError"),
        Some(&Value::Bool(true))
    );
}
