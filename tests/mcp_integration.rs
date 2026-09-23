use multi_agent_board::{
    agent_runtime::AgentManager,
    domain::{AgentState, ProjectCompletionWorkflow, TicketState},
    store::{NewTicket, ReviewSubmission, Store},
};
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

fn initialize_repository(path: &std::path::Path) -> String {
    std::fs::create_dir_all(path).unwrap();
    for arguments in [
        vec!["init", "-q"],
        vec!["config", "user.name", "Test"],
        vec!["config", "user.email", "test@example.com"],
        vec!["commit", "--allow-empty", "-qm", "initial"],
    ] {
        assert!(
            std::process::Command::new("git")
                .args(arguments)
                .current_dir(path)
                .status()
                .unwrap()
                .success()
        );
    }
    String::from_utf8(
        std::process::Command::new("git")
            .args(["branch", "--show-current"])
            .current_dir(path)
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap()
    .trim()
    .into()
}

#[tokio::test]
async fn mcp_registers_lists_and_archives_projects_through_http() {
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let repository = std::env::temp_dir().join(format!("mab-mcp-project-{unique}"));
    let branch = initialize_repository(&repository);
    let store = Store::memory().await.unwrap();
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

    let registered = mcp_tool(
        &base_url,
        1,
        1,
        "register_project",
        json!({
            "name": "MCP project",
            "path": repository,
            "default_branch": branch,
        }),
    )
    .await;
    assert_eq!(registered.pointer("/result/isError"), None);
    let project = tool_content(&registered);
    let listed = mcp_tool(&base_url, 1, 2, "list_projects", json!({})).await;
    assert!(tool_content(&listed).as_array().unwrap().contains(&project));
    let archived = mcp_tool(
        &base_url,
        1,
        3,
        "archive_project",
        json!({"project_id": project["id"]}),
    )
    .await;
    assert!(tool_content(&archived)["archived_at"].is_number());

    std::fs::remove_dir_all(repository).unwrap();
}

#[tokio::test]
async fn mcp_calls_the_server_without_database_access() {
    let store = Store::memory().await.unwrap();
    let first = store.create_agent("first", "/tmp/first").await.unwrap();
    store.create_agent("second", "/tmp/second").await.unwrap();
    store
        .update_agent_runtime(
            first.id,
            AgentState::Running,
            Some("thread-1"),
            Some("turn-1"),
            None,
        )
        .await
        .unwrap();
    store
        .set_agent_wake_settings(first.id, true, false, None)
        .await
        .unwrap();
    let second_project = store.set_project("/tmp/second-project").await.unwrap();
    store
        .add_agent_project_worktree(first.id, second_project.id, "/tmp/first-second")
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

    let missing_project = mcp_request(
        &base_url,
        1,
        json!({
            "jsonrpc":"2.0","id":1,"method":"tools/call",
            "params":{"name":"create_ticket","arguments":{"title":"From MCP"}}
        }),
    )
    .await;
    assert_eq!(
        missing_project.pointer("/result/isError"),
        Some(&json!(true))
    );

    let created = mcp_request(
        &base_url,
        1,
        json!({
            "jsonrpc":"2.0","id":1,"method":"tools/call",
            "params":{"name":"create_ticket","arguments":{"project_id":second_project.id,"title":"From MCP"}}
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
    assert_eq!(
        tool_content(&claimed)["project_transition"]["worktree_path"],
        "/tmp/first-second"
    );
    let first = store.agent(first.id).await.unwrap().unwrap();
    assert_eq!(first.active_project_id, Some(second_project.id));
    assert_eq!(first.active_turn_id.as_deref(), Some("turn-1"));
    assert!(
        store
            .wake_events(first.id)
            .await
            .unwrap()
            .iter()
            .any(|event| event.kind() == "ticket.assigned"
                && event.delivery_state == multi_agent_board::domain::WakeEventState::Delivered)
    );

    let dependent = mcp_tool(
        &base_url,
        1,
        3,
        "create_ticket",
        json!({"project_id":second_project.id,"title":"Blocked MCP ticket","blocker_ids":[1]}),
    )
    .await;
    assert_eq!(tool_content(&dependent)["blocker_ids"], json!([1]));

    let reservable = mcp_tool(
        &base_url,
        1,
        4,
        "create_ticket",
        json!({"project_id":second_project.id,"title":"Reserved MCP ticket"}),
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

    let comment = mcp_tool(
        &base_url,
        1,
        7,
        "add_ticket_comment",
        json!({"ticket_id":3,"body":"MCP discussion"}),
    )
    .await;
    assert_eq!(tool_content(&comment)["action"], "comment");
    let activity = mcp_tool(
        &base_url,
        1,
        8,
        "get_ticket_activity",
        json!({"ticket_id":3}),
    )
    .await;
    assert_eq!(
        tool_content(&activity)["entries"][0]["detail"],
        "MCP discussion"
    );
}

#[tokio::test]
async fn mcp_agent_without_review_scope_cannot_finish_external_lifecycle() {
    let store = Store::memory().await.unwrap();
    let project = store.project().await.unwrap().unwrap();
    store
        .set_project_completion_workflow(project.id, ProjectCompletionWorkflow::External)
        .await
        .unwrap();
    let worker = store.create_agent("worker", "/tmp/worker").await.unwrap();
    let reviewer = store
        .create_agent("reviewer", "/tmp/reviewer")
        .await
        .unwrap();
    store
        .set_agent_scopes(reviewer.id, &["approve_review".into()])
        .await
        .unwrap();
    let unrelated_reviewer = store
        .create_agent("unrelated-reviewer", "/tmp/unrelated-reviewer")
        .await
        .unwrap();
    store
        .set_agent_scopes(unrelated_reviewer.id, &["approve_review".into()])
        .await
        .unwrap();
    let ticket = store
        .create_ticket(NewTicket {
            title: "External lifecycle".into(),
            description: String::new(),
            blocker_ids: Vec::new(),
            created_by_agent_id: None,
        })
        .await
        .unwrap();
    let ticket = store
        .claim_ticket(ticket.id, worker.id, Some(ticket.revision))
        .await
        .unwrap();
    let ticket = store
        .submit_review(
            ticket.id,
            worker.id,
            ReviewSubmission {
                implementation_branch: Some("tickets/external"),
                commit: "reviewed-commit",
                target_branch: "main",
                project_id: project.id,
                project_path: &project.path,
            },
            Some(ticket.revision),
        )
        .await
        .unwrap();
    let ticket = store
        .claim_review(ticket.id, reviewer.id, Some(ticket.revision))
        .await
        .unwrap();
    let ticket = store
        .approve_external_review(
            ticket.id,
            reviewer.id,
            "reviewed-commit",
            Some(ticket.revision),
        )
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

    let changed = mcp_tool(
        &base_url,
        worker.id,
        1,
        "set_external_change",
        json!({
            "ticket_id":ticket.id,
            "external_change_url":"https://example.test/change/61"
        }),
    )
    .await;
    assert_eq!(
        tool_content(&changed)["external_change_url"],
        "https://example.test/change/61"
    );
    let ticket = store.ticket(ticket.id).await.unwrap().unwrap();

    let unrelated_change = mcp_tool(
        &base_url,
        unrelated_reviewer.id,
        2,
        "set_external_change",
        json!({
            "ticket_id":ticket.id,
            "external_change_url":"https://example.test/unrelated"
        }),
    )
    .await;
    assert_eq!(
        unrelated_change.pointer("/result/isError"),
        Some(&Value::Bool(true))
    );

    for (id, tool, arguments) in [
        (
            3,
            "return_external_merge",
            json!({"ticket_id":ticket.id,"reason":"take it back"}),
        ),
        (
            4,
            "complete_external_merge",
            json!({"ticket_id":ticket.id,"confirmation":"merged"}),
        ),
    ] {
        let response = mcp_tool(&base_url, worker.id, id, tool, arguments).await;
        assert_eq!(
            response.pointer("/result/isError"),
            Some(&Value::Bool(true))
        );
        let unchanged = store.ticket(ticket.id).await.unwrap().unwrap();
        assert_eq!(unchanged.state, TicketState::AwaitingExternalMerge);
        assert_eq!(unchanged.revision, ticket.revision);
    }

    for (id, tool, arguments) in [
        (
            5,
            "return_external_merge",
            json!({"ticket_id":ticket.id,"reason":"unrelated return"}),
        ),
        (
            6,
            "complete_external_merge",
            json!({"ticket_id":ticket.id,"confirmation":"unrelated completion"}),
        ),
    ] {
        let response = mcp_tool(&base_url, unrelated_reviewer.id, id, tool, arguments).await;
        assert_eq!(
            response.pointer("/result/isError"),
            Some(&Value::Bool(true))
        );
        assert_eq!(
            store.ticket(ticket.id).await.unwrap().unwrap().revision,
            ticket.revision
        );
    }

    let completed = mcp_tool(
        &base_url,
        reviewer.id,
        7,
        "complete_external_merge",
        json!({
            "ticket_id":ticket.id,
            "confirmation":"provider confirmed",
            "merge_reference":"provider-merge-61"
        }),
    )
    .await;
    assert_eq!(tool_content(&completed)["state"], "done");
    assert_eq!(
        tool_content(&completed)["external_merge_reference"],
        "provider-merge-61"
    );
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
    assert_eq!(
        isolated.pointer("/result/isError"),
        Some(&Value::Bool(true))
    );
    assert_eq!(
        store.agent_channel_subscriptions(first.id).await.unwrap(),
        vec!["general"]
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
    assert_eq!(
        isolated_read.pointer("/result/isError"),
        Some(&Value::Bool(true))
    );

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
async fn mcp_manages_only_the_calling_agents_project_subscriptions() {
    let store = Store::memory().await.unwrap();
    let first_project = store.project().await.unwrap().unwrap();
    let second_project = store.set_project("/tmp/second-project").await.unwrap();
    let first = store.create_agent("first", "/tmp/first").await.unwrap();
    let second = store.create_agent("second", "/tmp/second").await.unwrap();
    store
        .set_agent_project_subscriptions(second.id, &[])
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

    let added = mcp_tool(
        &base_url,
        first.id,
        1,
        "add_my_project_subscriptions",
        json!({"project_ids":[first_project.id]}),
    )
    .await;
    assert_eq!(
        tool_content(&added),
        json!([first_project.id, second_project.id])
    );
    let removed = mcp_tool(
        &base_url,
        first.id,
        2,
        "remove_my_project_subscriptions",
        json!({"project_ids":[second_project.id]}),
    )
    .await;
    assert_eq!(tool_content(&removed), json!([first_project.id]));
    let replaced = mcp_tool(
        &base_url,
        first.id,
        3,
        "replace_my_project_subscriptions",
        json!({"project_ids":[second_project.id], "agent_id":second.id}),
    )
    .await;
    assert_eq!(
        replaced.pointer("/result/isError"),
        Some(&Value::Bool(true))
    );
    let listed = mcp_tool(
        &base_url,
        first.id,
        4,
        "list_my_project_subscriptions",
        json!({"agent_id":second.id}),
    )
    .await;
    assert_eq!(listed.pointer("/result/isError"), Some(&Value::Bool(true)));
    assert_eq!(
        store.agent_project_subscriptions(first.id).await.unwrap(),
        vec![first_project.id]
    );
    assert_eq!(
        store.agent_project_subscriptions(second.id).await.unwrap(),
        Vec::<i64>::new()
    );
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
    assert_eq!(tool_content(&read)["has_more"], false);

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

#[tokio::test]
async fn mcp_manages_only_the_calling_agents_reminders() {
    let store = Store::memory().await.unwrap();
    let first = store.create_agent("first", "/tmp/first").await.unwrap();
    let second = store.create_agent("second", "/tmp/second").await.unwrap();
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

    let scheduled = mcp_tool(
        &base_url,
        first.id,
        1,
        "schedule_self_reminder",
        json!({"message":"Check CI","delay_seconds":60}),
    )
    .await;
    let reminder = tool_content(&scheduled);
    assert_eq!(reminder["agent_id"], first.id);
    assert_eq!(reminder["state"], "scheduled");

    let rejected_override = mcp_tool(
        &base_url,
        first.id,
        2,
        "list_my_reminders",
        json!({"agent_id":second.id}),
    )
    .await;
    assert_eq!(
        rejected_override.pointer("/result/isError"),
        Some(&Value::Bool(true))
    );
    let listed = mcp_tool(&base_url, first.id, 3, "list_my_reminders", json!({})).await;
    assert_eq!(tool_content(&listed).as_array().unwrap().len(), 1);
    let hidden = mcp_tool(
        &base_url,
        second.id,
        4,
        "cancel_my_reminder",
        json!({"reminder_id":reminder["id"]}),
    )
    .await;
    assert_eq!(hidden.pointer("/result/isError"), Some(&Value::Bool(true)));

    let cancelled = mcp_tool(
        &base_url,
        first.id,
        5,
        "cancel_my_reminder",
        json!({"reminder_id":reminder["id"]}),
    )
    .await;
    assert_eq!(tool_content(&cancelled)["state"], "cancelled");

    let invalid = mcp_tool(
        &base_url,
        first.id,
        6,
        "schedule_self_reminder",
        json!({"message":"No time"}),
    )
    .await;
    assert_eq!(invalid.pointer("/result/isError"), Some(&Value::Bool(true)));
}
