use std::sync::Arc;

use anyhow::Result;
use tracing::error;

use crate::config::Config;
use crate::events::EventHub;

mod player;
mod registry;

pub async fn spawn(config: Arc<Config>, hub: EventHub) -> Result<()> {
    tokio::spawn(async move {
        if let Err(err) = registry::run(config, hub).await {
            error!("mpris registry failed: {err}");
        }
    });
    Ok(())
}
