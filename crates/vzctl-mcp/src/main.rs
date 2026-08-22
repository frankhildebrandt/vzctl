use rmcp::{ServiceExt, transport::stdio};
use vzctl_mcp::VzctlMcp;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let server = VzctlMcp::new().map_err(anyhow::Error::msg)?;
    let service = server.serve(stdio()).await?;
    service.waiting().await?;
    Ok(())
}
