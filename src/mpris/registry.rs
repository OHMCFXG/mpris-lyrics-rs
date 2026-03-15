use std::collections::HashSet;
use std::sync::Arc;

use anyhow::Result;
use futures_util::StreamExt;
use tracing::{debug, info, warn};
use zbus::fdo::DBusProxy;
use zbus::Connection;

use crate::config::Config;
use crate::events::{Event, EventHub};

use super::player;

const MPRIS_PREFIX: &str = "org.mpris.MediaPlayer2.";

pub async fn run(config: Arc<Config>, hub: EventHub) -> Result<()> {
    let conn = Connection::session().await?;
    let dbus = DBusProxy::new(&conn).await?;

    let mut known: HashSet<String> = HashSet::new();

    let names = dbus.list_names().await?;
    for name in names {
        let name = name.to_string();
        if !is_mpris_name(&name) || is_blacklisted(&config, &name) {
            continue;
        }
        if known.insert(name.clone()) {
            hub.emit(Event::PlayerAppeared {
                player: name.clone(),
            });
            player::spawn(
                conn.clone(),
                name,
                hub.clone(),
                config.mpris.fallback_sync_interval_seconds,
            )
            .await;
        }
    }

    let mut stream = dbus.receive_name_owner_changed().await?;
    while let Some(signal) = stream.next().await {
        let args = match signal.args() {
            Ok(args) => args,
            Err(err) => {
                warn!("failed to parse NameOwnerChanged: {err}");
                continue;
            }
        };

        let name = args.name.to_string();
        if !is_mpris_name(&name) || is_blacklisted(&config, &name) {
            continue;
        }

        let appeared = args.old_owner.is_none() && args.new_owner.is_some();
        let disappeared = args.old_owner.is_some() && args.new_owner.is_none();

        if appeared {
            if known.insert(name.clone()) {
                info!("player appeared: {name}");
                hub.emit(Event::PlayerAppeared {
                    player: name.clone(),
                });
                player::spawn(
                    conn.clone(),
                    name,
                    hub.clone(),
                    config.mpris.fallback_sync_interval_seconds,
                )
                .await;
            }
        } else if disappeared {
            if known.remove(&name) {
                info!("player disappeared: {name}");
                hub.emit(Event::PlayerDisappeared { player: name });
            }
        } else {
            debug!("name owner changed: {name}");
        }
    }

    Ok(())
}

fn is_mpris_name(name: &str) -> bool {
    name.starts_with(MPRIS_PREFIX)
}

fn is_blacklisted(config: &Config, name: &str) -> bool {
    let lower = name.to_lowercase();
    config.players.blacklist.iter().any(|k| lower.contains(k))
}
