use multi_agent_board::{agent_runtime::AgentManager, store::Store};
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

async fn mcp_tool(base_url: &str, agent_id: i64, id: i64, name: &str, arguments: Value) -> Value {
    mcp_request(
        base_url,
        agent_id,
        json!({
            "jsonrpc":"2.0","id":id,"method":"tools/call",
            "params":{"name":name,"arguments":arguments}
        }),
    )
    .await
}

fn tool_content(response: &Value) -> Value {
    serde_json::from_str(
        response
            .pointer("/result/content/0/text")
            .and_then(Value::as_str)
            .unwrap(),
    )
    .unwrap()
}

#[tokio::test]
async fn mcp_calls_the_server_without_database_access() {
    let store = Store::memory().await.unwrap();
    store.create_agent("first", "/tmp/first").await.unwrap();
    store.create_agent("second", "/tmp/second").await.unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let agents = AgentManager::new(
        store.clone(),
        "codex".into(),
        "multi-agent-board".into(),
        format!("http://{address}"),
    );
    let app = multi_agent_board::api::router(store.clone(), agents);
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
    assert_eq!(tool_content(&created)["created_by_agent_id"], 1);

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

    let dependent = mcp_tool(
        &base_url,
        1,
        3,
        "create_ticket",
        json!({"title":"Blocked MCP ticket","blocker_ids":[1]}),
    )
    .await;
    assert_eq!(tool_content(&dependent)["blocker_ids"], json!([1]));

    let reservable = mcp_tool(
        &base_url,
        1,
        4,
        "create_ticket",
        json!({"title":"Reserved MCP ticket"}),
    )
    .await;
    assert_eq!(tool_content(&reservable)["created_by_agent_id"], 1);
    let reserved = mcp_tool(
        &base_url,
        1,
        5,
        "reserve_tickets",
        json!({"ticket_ids":[3]}),
    )
    .await;
    assert_eq!(tool_content(&reserved)[0]["state"], "todo");
    assert_eq!(tool_content(&reserved)[0]["reserved_for_agent_id"], 1);

    let conflict = mcp_request(
        &base_url,
        2,
        json!({
            "jsonrpc":"2.0","id":6,"method":"tools/call",
            "params":{"name":"claim_ticket","arguments":{"ticket_id":3}}
        }),
    )
    .await;
    assert_eq!(
        conflict.pointer("/result/isError"),
        Some(&Value::Bool(true))
    );

    let activity = mcp_tool(
        &base_url,
        1,
        7,
        "get_ticket_activity",
        json!({"ticket_id":3}),
    )
    .await;
    assert_eq!(tool_content(&activity), json!([]));
}

#[tokio::test]
async fn mcp_manages_only_the_calling_agents_channel_subscriptions() {
    let store = Store::memory().await.unwrap();
    let first = store.create_agent("first", "/tmp/first").await.unwrap();
    let second = store.create_agent("second", "/tmp/second").await.unwrap();
    store
        .create_channel("backend", "Backend work")
        .await
        .unwrap();
    store
        .post_message("backend", None, "human", "Ready")
        .await
        .unwrap();
    store
        .set_agent_channel_subscriptions(second.id, &["general".into()])
        .await
        .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let agents = AgentManager::new(
        store.clone(),
        "codex".into(),
        "multi-agent-board".into(),
        format!("http://{address}"),
    );
    let app = multi_agent_board::api::router(store.clone(), agents);
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let base_url = format!("http://{address}");

    let listed = mcp_tool(
        &base_url,
        first.id,
        1,
        "list_my_channel_subscriptions",
        json!({}),
    )
    .await;
    assert_eq!(tool_content(&listed), json!([]));

    let added = mcp_tool(
        &base_url,
        first.id,
        2,
        "add_my_channel_subscriptions",
        json!({"channels":["general", "backend"]}),
    )
    .await;
    assert_eq!(tool_content(&added), json!(["backend", "general"]));

    let removed = mcp_tool(
        &base_url,
        first.id,
        3,
        "remove_my_channel_subscriptions",
        json!({"channels":["general"]}),
    )
    .await;
    assert_eq!(tool_content(&removed), json!(["backend"]));

    let replaced = mcp_tool(
        &base_url,
        first.id,
        4,
        "replace_my_channel_subscriptions",
        json!({"channels":["general"]}),
    )
    .await;
    assert_eq!(tool_content(&replaced), json!(["general"]));

    let isolated = mcp_tool(
        &base_url,
        first.id,
        5,
        "replace_my_channel_subscriptions",
        json!({"channels":["backend"], "agent_id":second.id}),
    )
    .await;
    assert_eq!(tool_content(&isolated), json!(["backend"]));
    assert_eq!(
        store.agent_channel_subscriptions(first.id).await.unwrap(),
        vec!["backend"]
    );
    assert_eq!(
        store.agent_channel_subscriptions(second.id).await.unwrap(),
        vec!["general"]
    );
    let isolated_read = mcp_tool(
        &base_url,
        first.id,
        6,
        "list_my_channel_subscriptions",
        json!({"agent_id":second.id}),
    )
    .await;
    assert_eq!(tool_content(&isolated_read), json!(["backend"]));

    for (id, channel, expected) in [
        (7, "bad channel", "channel names may contain"),
        (8, "missing", "channel #missing does not exist"),
    ] {
        let response = mcp_tool(
            &base_url,
            first.id,
            id,
            "add_my_channel_subscriptions",
            json!({"channels":[channel]}),
        )
        .await;
        assert_eq!(
            response.pointer("/result/isError"),
            Some(&Value::Bool(true))
        );
        assert!(
            response
                .pointer("/result/content/0/text")
                .and_then(Value::as_str)
                .unwrap()
                .contains(expected)
        );
    }
}

#[tokio::test]
async fn mcp_creates_and_describes_empty_channels() {
    let store = Store::memory().await.unwrap();
    let agent = store.create_agent("first", "/tmp/first").await.unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let agents = AgentManager::new(
        store.clone(),
        "codex".into(),
        "multi-agent-board".into(),
        format!("http://{address}"),
    );
    let app = multi_agent_board::api::router(store, agents);
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let base_url = format!("http://{address}");

    let created = mcp_tool(
        &base_url,
        agent.id,
        1,
        "create_message_board_channel",
        json!({"name":"architecture","description":"Architecture decisions"}),
    )
    .await;
    assert_eq!(
        tool_content(&created)["description"],
        "Architecture decisions"
    );

    let listed = mcp_tool(
        &base_url,
        agent.id,
        2,
        "list_message_board_channels",
        json!({}),
    )
    .await;
    assert_eq!(tool_content(&listed)[0]["name"], "architecture");
    assert_eq!(
        tool_content(&listed)[0]["description"],
        "Architecture decisions"
    );

    let read = mcp_tool(
        &base_url,
        agent.id,
        3,
        "read_message_board_channel",
        json!({"channel_name":"architecture"}),
    )
    .await;
    assert_eq!(
        tool_content(&read)["channel"]["description"],
        "Architecture decisions"
    );
    assert_eq!(tool_content(&read)["messages"], json!([]));

    for (id, name, arguments, expected) in [
        (
            4,
            "create_message_board_channel",
            json!({"name":"architecture"}),
            "already exists",
        ),
        (
            5,
            "create_message_board_channel",
            json!({"name":"bad channel"}),
            "channel names may contain",
        ),
        (
            6,
            "post_to_message_board",
            json!({"channel_name":"missing","author":"first","message":"No channel"}),
            "create it before posting",
        ),
    ] {
        let response = mcp_tool(&base_url, agent.id, id, name, arguments).await;
        assert_eq!(
            response.pointer("/result/isError"),
            Some(&Value::Bool(true))
        );
        assert!(
            response
                .pointer("/result/content/0/text")
                .and_then(Value::as_str)
                .unwrap()
                .contains(expected)
        );
    }

    let unauthenticated = mcp_tool(
        &base_url,
        999,
        7,
        "create_message_board_channel",
        json!({"name":"unauthorized"}),
    )
    .await;
    assert_eq!(
        unauthenticated.pointer("/result/isError"),
        Some(&Value::Bool(true))
    );
}
