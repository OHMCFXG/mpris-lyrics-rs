use std::sync::Arc;

use anyhow::Result;
use tokio::sync::broadcast;
use tracing::{error, info};

use crate::config::Config;
use crate::events::{Event, EventHub, UiCommand};
use crate::lyrics::{providers, LyricsService};
use crate::mpris;
use crate::state::StateStore;
use crate::ui::{simple::SimpleOutput, tui::TuiApp};

pub struct App {
    config: Arc<Config>,
}

impl App {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }

    pub async fn run(&self) -> Result<()> {
        let hub = EventHub::new(256);
        let store = Arc::new(StateStore::new());

        let state_handle = spawn_state_loop(store.clone(), hub.clone());

        let providers = providers::get_enabled_providers(&self.config);
        let lyrics_service = LyricsService::new(providers, hub.clone(), store.clone());
        let lyrics_handle = tokio::spawn(async move {
            if let Err(err) = lyrics_service.run().await {
                error!("lyrics service failed: {err}");
            }
        });

        let ui_handle = if self.config.display.simple_output || !self.config.display.enable_tui {
            let ui = SimpleOutput::new(self.config.clone(), hub.clone(), store.clone());
            tokio::spawn(async move {
                if let Err(err) = ui.run().await {
                    error!("simple output failed: {err}");
                }
            })
        } else {
            let tui = TuiApp::new(self.config.clone(), hub.clone(), store.clone());
            tokio::spawn(async move {
                if let Err(err) = tui.run().await {
                    error!("tui failed: {err}");
                }
            })
        };

        // Start MPRIS last so initial events are not missed by lyrics/UI tasks.
        let mpris_handle = mpris::spawn(self.config.clone(), hub.clone());

        if self.config.display.simple_output {
            tracing::debug!("app started; simple output mode");
        } else if self.config.display.enable_tui {
            info!("app started; press Ctrl+C or q/Esc in TUI to exit");
        } else {
            info!("app started; press Ctrl+C to exit");
        }
        let mut rx = hub.subscribe();
        loop {
            tokio::select! {
                _ = tokio::signal::ctrl_c() => break,
                event = rx.recv() => {
                    let event = match event {
                        Ok(ev) => ev,
                        Err(broadcast::error::RecvError::Closed) => break,
                        Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    };
                    if let Event::UiCommand { command: UiCommand::Quit } = event {
                        break;
                    }
                }
            }
        }
        hub.emit(Event::Shutdown);
        let _ = state_handle.await;
        let _ = lyrics_handle.await;
        let _ = ui_handle.await;
        let _ = mpris_handle.await;
        Ok(())
    }
}

fn spawn_state_loop(store: Arc<StateStore>, hub: EventHub) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut rx = hub.subscribe();
        loop {
            let event = match rx.recv().await {
                Ok(ev) => ev,
                Err(broadcast::error::RecvError::Closed) => break,
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
            };

            if matches!(event, Event::Shutdown) {
                break;
            }
            let derived = store.handle_event(&event).await;
            for ev in derived {
                hub.emit(ev);
            }
        }
    })
}
