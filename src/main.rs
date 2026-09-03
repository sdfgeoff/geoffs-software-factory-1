use std::net::SocketAddr;
use std::path::PathBuf;

use multi_agent_board::{agent_runtime::AgentManager, server, store::Store};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    if matches!(std::env::args().nth(1).as_deref(), Some("-h" | "--help")) {
        println!(
            "Usage: multi-agent-board [mcp]\n\nEnvironment:\n  MULTI_AGENT_BOARD_ADDR      Listen address (default 127.0.0.1:3000)\n  MULTI_AGENT_BOARD_DATA      SQLite database path\n  MULTI_AGENT_BOARD_URL       Board API URL (required in MCP mode)\n  MULTI_AGENT_BOARD_AGENT_ID  Calling agent ID (required in MCP mode)\n  CODEX_BIN                   Codex CLI path (default codex)"
        );
        return Ok(());
    }
    let address: SocketAddr = std::env::var("MULTI_AGENT_BOARD_ADDR")
        .unwrap_or_else(|_| "127.0.0.1:3000".into())
        .parse()?;
    let data_path = std::env::var_os("MULTI_AGENT_BOARD_DATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(".multi-agent-board.sqlite3"));
    let data_path = if data_path.is_absolute() {
        data_path
    } else {
        std::env::current_dir()?.join(data_path)
    };
    if std::env::args().nth(1).as_deref() == Some("mcp") {
        let board_url = std::env::var("MULTI_AGENT_BOARD_URL")?;
        let agent_id = std::env::var("MULTI_AGENT_BOARD_AGENT_ID")?.parse()?;
        return Ok(multi_agent_board::mcp::run(
            tokio::io::BufReader::new(tokio::io::stdin()),
            tokio::io::stdout(),
            board_url,
            agent_id,
        )
        .await?);
    }
    let store = Store::open(&data_path).await?;
    let recovered = store.recover_agents_after_restart().await?;
    if recovered > 0 {
        eprintln!("Recovered {recovered} interrupted agent(s)");
    }
    let listener = tokio::net::TcpListener::bind(address).await?;
    let local_address = listener.local_addr()?;
    let host = if local_address.ip().is_unspecified() {
        std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST)
    } else {
        local_address.ip()
    };
    let coordination_address = SocketAddr::new(host, local_address.port());
    let agents = AgentManager::new(
        store.clone(),
        std::env::var_os("CODEX_BIN")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("codex")),
        std::env::current_exe()?,
        format!("http://{coordination_address}/api"),
    );
    println!("Multi-agent board listening on http://{address}");
    axum::serve(listener, server::router(store, agents)).await?;
    Ok(())
}
