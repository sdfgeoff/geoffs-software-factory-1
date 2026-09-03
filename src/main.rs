use std::net::SocketAddr;
use std::path::PathBuf;

use multi_agent_board::{server, store::Store};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let address = SocketAddr::from(([127, 0, 0, 1], 3000));
    let data_path = std::env::var_os("MULTI_AGENT_BOARD_DATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(".multi-agent-board.sqlite3"));
    let store = Store::open(&data_path).await?;
    let listener = tokio::net::TcpListener::bind(address).await?;
    println!("Multi-agent board listening on http://{address}");
    axum::serve(listener, server::router(store)).await?;
    Ok(())
}
