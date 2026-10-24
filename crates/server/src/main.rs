//! The `nekowriter` binary: parses configuration and starts the server.

use anyhow::Context;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config = nekowriter::config::Config::load().context("invalid configuration")?;
    nekowriter::run(config).await
}
