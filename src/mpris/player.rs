use std::collections::HashMap;
use std::time::Duration;

use anyhow::Result;
use futures_util::StreamExt;
use tracing::{debug, warn};
use zbus::fdo::{PropertiesChanged, PropertiesProxy};
use zbus::names::InterfaceName;
use zbus::zvariant::{Array, Dict, ObjectPath, Optional, OwnedValue};
use zbus::{Connection, Message, Proxy};

use crate::events::{Event, EventHub};
use crate::state::{PlaybackStatus, TrackInfo};

const MPRIS_PATH: &str = "/org/mpris/MediaPlayer2";
const PLAYER_INTERFACE: &str = "org.mpris.MediaPlayer2.Player";

pub async fn spawn(
    conn: Connection,
    name: String,
    hub: EventHub,
    fallback_sync_interval_seconds: u64,
) {
    tokio::spawn(async move {
        if let Err(err) = run(conn, &name, hub, fallback_sync_interval_seconds).await {
            warn!("player actor failed for {name}: {err}");
        }
    });
}

async fn run(
    conn: Connection,
    name: &str,
    hub: EventHub,
    fallback_sync_interval_seconds: u64,
) -> Result<()> {
    let props = PropertiesProxy::builder(&conn)
        .destination(name)?
        .path(MPRIS_PATH)?
        .build()
        .await?;

    let player_proxy: Proxy<'_> = Proxy::new(&conn, name, MPRIS_PATH, PLAYER_INTERFACE).await?;

    let iface = InterfaceName::try_from(PLAYER_INTERFACE)?;

    if let Ok(all) = props.get_all(Optional::from(Some(iface.clone()))).await {
        tracing::debug!("mpris: initial get_all for {}", name);
        handle_properties(name, &hub, &all);
    }

    let mut props_stream = props.receive_properties_changed().await?;
    let mut seeked_stream = player_proxy.receive_signal("Seeked").await?;
    let mut fallback_tick = tokio::time::interval(Duration::from_secs(
        fallback_sync_interval_seconds.max(1),
    ));

    loop {
        tokio::select! {
            maybe_signal = props_stream.next() => {
                let signal: PropertiesChanged = match maybe_signal {
                    Some(s) => s,
                    None => break,
                };
                let args = signal.args()?;
                if args.interface_name.as_str() != PLAYER_INTERFACE {
                    continue;
                }
                tracing::debug!("mpris: properties changed for {}", name);
                let mut owned: HashMap<String, OwnedValue> = HashMap::new();
                for (key, value) in &args.changed_properties {
                    if let Ok(owned_value) = value.try_to_owned() {
                        owned.insert((*key).to_string(), owned_value);
                    }
                }
                let status = owned
                    .get("PlaybackStatus")
                    .and_then(parse_playback_status);
                handle_properties(name, &hub, &owned);
                if matches!(status, Some(PlaybackStatus::Playing)) {
                    if let Ok(value) = props.get(iface.clone(), "Position").await {
                        if let Some(position_ms) = parse_position_ms(&value) {
                            hub.emit(Event::PositionUpdated {
                                player: name.to_string(),
                                position_ms,
                            });
                        }
                    }
                }
            }
            maybe_msg = seeked_stream.next() => {
                let msg: Message = match maybe_msg {
                    Some(m) => m,
                    None => break,
                };
                let (position_us,): (i64,) = msg.body().deserialize()?;
                if position_us >= 0 {
                    let position_ms = (position_us as u64) / 1000;
                    hub.emit(Event::Seeked {
                        player: name.to_string(),
                        position_ms,
                    });
                }
            }
            _ = fallback_tick.tick() => {
                if let Ok(value) = props.get(iface.clone(), "Position").await {
                    if let Some(position_ms) = parse_position_ms(&value) {
                        hub.emit(Event::PositionUpdated {
                            player: name.to_string(),
                            position_ms,
                        });
                    }
                }
            }
        }
    }

    Ok(())
}

fn handle_properties(player: &str, hub: &EventHub, changed: &HashMap<String, OwnedValue>) {
    if let Some(value) = changed.get("PlaybackStatus") {
        if let Some(status) = parse_playback_status(value) {
            hub.emit(Event::PlaybackStatusChanged {
                player: player.to_string(),
                status,
            });
        }
    }

    if let Some(value) = changed.get("Metadata") {
        if let Some(track) = parse_metadata(value) {
            hub.emit(Event::TrackChanged {
                player: player.to_string(),
                track,
            });
        }
    }

    if let Some(value) = changed.get("Position") {
        if let Some(position_ms) = parse_position_ms(value) {
            hub.emit(Event::PositionUpdated {
                player: player.to_string(),
                position_ms,
            });
        }
    }

    if changed.contains_key("Rate") {
        if let Some(rate) = parse_rate(changed.get("Rate")) {
            hub.emit(Event::RateChanged {
                player: player.to_string(),
                rate,
            });
        } else {
            debug!("rate changed for {player}");
        }
    }
}

fn parse_playback_status(value: &OwnedValue) -> Option<PlaybackStatus> {
    let status: &str = value.try_into().ok()?;
    match status {
        "Playing" => Some(PlaybackStatus::Playing),
        "Paused" => Some(PlaybackStatus::Paused),
        "Stopped" => Some(PlaybackStatus::Stopped),
        _ => None,
    }
}

fn parse_position_ms(value: &OwnedValue) -> Option<u64> {
    let position_us: i64 = value.try_into().ok()?;
    if position_us < 0 {
        return None;
    }
    Some((position_us as u64) / 1000)
}

fn parse_rate(value: Option<&OwnedValue>) -> Option<f64> {
    let value = value?;
    let rate: f64 = value.try_into().ok()?;
    Some(rate)
}

fn parse_metadata(value: &OwnedValue) -> Option<TrackInfo> {
    let dict: &Dict<'_, '_> = value.try_into().ok()?;

    let title = dict
        .get::<_, &str>(&"xesam:title")
        .ok()
        .flatten()
        .unwrap_or("")
        .to_string();

    let artist = parse_artists(dict);

    let album = dict
        .get::<_, &str>(&"xesam:album")
        .ok()
        .flatten()
        .unwrap_or("")
        .to_string();

    let length_us = dict
        .get::<_, i64>(&"mpris:length")
        .ok()
        .flatten()
        .unwrap_or(0);
    let length_ms = if length_us > 0 {
        (length_us as u64) / 1000
    } else {
        0
    };

    let track_id = dict
        .get::<_, ObjectPath<'_>>(&"mpris:trackid")
        .ok()
        .flatten()
        .map(|v| v.to_string());

    Some(TrackInfo {
        title,
        artist,
        album,
        length_ms,
        track_id,
    })
}

fn parse_artists(dict: &Dict<'_, '_>) -> String {
    let artists: Option<Array<'_>> = dict.get(&"xesam:artist").ok().flatten();
    let Some(array) = artists else { return String::new(); };

    let mut names = Vec::new();
    for val in array.inner() {
        if let Ok(name) = val.downcast_ref::<zbus::zvariant::Str>() {
            names.push(name.as_str().to_string());
        }
    }
    names.join(", ")
}
