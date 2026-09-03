use std::net::SocketAddr;

use multi_agent_board::server;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let address = SocketAddr::from(([127, 0, 0, 1], 3000));
    let listener = tokio::net::TcpListener::bind(address).await?;
    println!("Multi-agent board listening on http://{address}");
    axum::serve(listener, server::router()).await?;
    Ok(())
}
