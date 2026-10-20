use rmcp::{transport::stdio, ServiceExt};
use vzctl_mcp::VzctlMcp;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    if std::env::args().nth(1).as_deref() == Some("--version") {
        println!("vzctl-mcp {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    let server = VzctlMcp::new().map_err(anyhow::Error::msg)?;
    let service = server.serve(stdio()).await?;
    service.waiting().await?;
    Ok(())
}
