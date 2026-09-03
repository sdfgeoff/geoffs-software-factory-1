use std::net::SocketAddr;
use std::path::PathBuf;

use multi_agent_board::{agent_runtime::AgentManager, server, store::Store};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let address = SocketAddr::from(([127, 0, 0, 1], 3000));
    let data_path = std::env::var_os("MULTI_AGENT_BOARD_DATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(".multi-agent-board.sqlite3"));
    let data_path = if data_path.is_absolute() {
        data_path
    } else {
        std::env::current_dir()?.join(data_path)
    };
    let store = Store::open(&data_path).await?;
    if std::env::args().nth(1).as_deref() == Some("mcp") {
        return Ok(multi_agent_board::mcp::run(
            tokio::io::BufReader::new(tokio::io::stdin()),
            tokio::io::stdout(),
            store,
        )
        .await?);
    }
    let agents = AgentManager::new(
        store.clone(),
        std::env::var_os("CODEX_BIN")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("codex")),
        std::env::current_exe()?,
        data_path,
    );
    let listener = tokio::net::TcpListener::bind(address).await?;
    println!("Multi-agent board listening on http://{address}");
    axum::serve(listener, server::router(store, agents)).await?;
    Ok(())
}
