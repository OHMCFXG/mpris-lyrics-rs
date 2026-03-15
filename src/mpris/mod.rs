use std::sync::Arc;

use tracing::error;

use crate::config::Config;
use crate::events::EventHub;

mod player;
mod registry;

pub fn spawn(config: Arc<Config>, hub: EventHub) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        if let Err(err) = registry::run(config, hub).await {
            error!("mpris registry failed: {err}");
        }
    })
}
