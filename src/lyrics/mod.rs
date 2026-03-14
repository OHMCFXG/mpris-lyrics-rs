use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use tokio::sync::broadcast;

use crate::events::{Event, EventHub};
use crate::state::{PlaybackStatus, StateStore, TrackInfo};

pub mod providers;

#[derive(Debug, Clone)]
pub struct LyricLine {
    pub start_time_ms: u64,
    pub end_time_ms: Option<u64>,
    pub text: String,
}

#[derive(Debug, Clone)]
pub struct LyricsMetadata {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub source: String,
}

#[derive(Debug, Clone)]
pub struct Lyrics {
    pub metadata: LyricsMetadata,
    pub lines: Vec<LyricLine>,
}

pub type TrackKey = String;

pub fn make_track_key(track: &TrackInfo) -> TrackKey {
    format!("{}::{}::{}", track.title, track.artist, track.album)
}

pub fn parse_lrc_text(lrc_text: &str, track: &TrackInfo, source: &str) -> Result<Lyrics> {
    let parsed = lrc::Lyrics::from_str(lrc_text)?;
    let mut lines: Vec<LyricLine> = parsed
        .get_timed_lines()
        .iter()
        .filter_map(|(tag, text)| {
            if text.trim().is_empty() {
                return None;
            }
            Some(LyricLine {
                start_time_ms: tag.get_timestamp().max(0) as u64,
                end_time_ms: None,
                text: text.to_string(),
            })
        })
        .collect();

    lines.sort_by_key(|line| line.start_time_ms);
    for idx in 0..lines.len() {
        let end = lines.get(idx + 1).map(|line| line.start_time_ms);
        lines[idx].end_time_ms = end;
    }

    Ok(Lyrics {
        metadata: LyricsMetadata {
            title: track.title.clone(),
            artist: track.artist.clone(),
            album: track.album.clone(),
            source: source.to_string(),
        },
        lines,
    })
}

#[async_trait]
pub trait LyricsProvider: Send + Sync {
    fn name(&self) -> &str;
    async fn fetch(&self, track: &TrackInfo) -> Result<Option<Lyrics>>;
}

pub struct LyricsService {
    providers: Vec<Arc<dyn LyricsProvider>>,
    hub: EventHub,
    store: Arc<StateStore>,
    pending: tokio::sync::Mutex<std::collections::HashSet<TrackKey>>,
}

impl LyricsService {
    pub fn new(providers: Vec<Arc<dyn LyricsProvider>>, hub: EventHub, store: Arc<StateStore>) -> Self {
        Self {
            providers,
            hub,
            store,
            pending: tokio::sync::Mutex::new(std::collections::HashSet::new()),
        }
    }

    pub async fn run(self) -> Result<()> {
        let mut rx = self.hub.subscribe();

        // Initial catch-up: fetch lyrics for current active track if present.
        let initial = self.store.snapshot().await;
        if let Some(active) = &initial.active_player {
            if let Some(track) = initial
                .players
                .get(active)
                .and_then(|player| player.track.as_ref())
            {
                tracing::debug!("lyrics: initial fetch for active player {}", active);
                let track = track.clone();
                self.fetch_for_track(&track).await;
            }
        }

        loop {
            let event = match rx.recv().await {
                Ok(ev) => ev,
                Err(broadcast::error::RecvError::Closed) => break,
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
            };

            match event {
                Event::TrackChanged { player, track } => {
                    tracing::debug!(
                        "lyrics: track changed player={} title='{}' artist='{}'",
                        player,
                        track.title,
                        track.artist
                    );
                    let state = self.store.snapshot().await;
                    if state.active_player.as_ref() != Some(&player) {
                        tracing::debug!(
                            "lyrics: skip track change, active_player={:?}",
                            state.active_player
                        );
                        continue;
                    }
                    self.fetch_for_track(&track).await;
                }
                Event::ActivePlayerChanged { player, .. } => {
                    tracing::debug!("lyrics: active player changed to {}", player);
                    let state = self.store.snapshot().await;
                    if let Some(player_state) = state.players.get(&player) {
                        if let Some(track) = &player_state.track {
                            self.fetch_for_track(track).await;
                        } else {
                            tracing::debug!("lyrics: active player has no track yet");
                        }
                    } else {
                        tracing::debug!("lyrics: active player not found in state");
                    }
                }
                Event::PlaybackStatusChanged { player, status } => {
                    if status == PlaybackStatus::Playing {
                        tracing::debug!("lyrics: playback status playing for {}", player);
                        let state = self.store.snapshot().await;
                        if state.active_player.as_ref() == Some(&player) {
                            if let Some(player_state) = state.players.get(&player) {
                                if let Some(track) = &player_state.track {
                                    self.fetch_for_track(track).await;
                                } else {
                                    tracing::debug!("lyrics: playing but track missing");
                                }
                            } else {
                                tracing::debug!("lyrics: playing player not in state");
                            }
                        } else {
                            tracing::debug!(
                                "lyrics: playing but not active (active_player={:?})",
                                state.active_player
                            );
                        }
                    }
                }
                _ => {}
            }
        }

        Ok(())
    }

    async fn fetch_for_track(&self, track: &TrackInfo) {
        if track.title.is_empty() {
            return;
        }

        let track_key = make_track_key(track);
        tracing::debug!(
            "lyrics: fetch request key={} title='{}' artist='{}'",
            track_key,
            track.title,
            track.artist
        );

        let state = self.store.snapshot().await;
        if state.lyrics.track_key.as_ref() == Some(&track_key) {
            match state.lyrics.status {
                crate::state::LyricsStatus::Ready | crate::state::LyricsStatus::Loading => {
                    tracing::debug!("lyrics: skip fetch, already ready/loading");
                    return;
                }
                crate::state::LyricsStatus::Failed(_) => {
                    if state.lyrics.fail_count >= 2 {
                        tracing::debug!("lyrics: skip fetch, fail_count >= 2");
                        return;
                    }
                    if let Some(last_failed_at) = state.lyrics.last_failed_at {
                        if last_failed_at.elapsed() < std::time::Duration::from_secs(30) {
                            tracing::debug!("lyrics: skip fetch, backoff active");
                            return;
                        }
                    }
                }
                crate::state::LyricsStatus::Idle => {}
            }
        }

        {
            let mut pending = self.pending.lock().await;
            if pending.contains(&track_key) {
                tracing::debug!("lyrics: skip fetch, pending");
                return;
            }
            pending.insert(track_key.clone());
        }

        self.hub.emit(Event::LyricsRequested { track_key: track_key.clone() });

        let mut found: Option<Lyrics> = None;
        for provider in &self.providers {
            tracing::debug!("lyrics: trying provider {}", provider.name());
            match provider.fetch(track).await {
                Ok(Some(lyrics)) => {
                    tracing::debug!("lyrics: provider {} hit", provider.name());
                    found = Some(lyrics);
                    break;
                }
                Ok(None) => continue,
                Err(err) => {
                    let msg = format!("{}: {}", provider.name(), err);
                    tracing::debug!("lyrics: provider {} error {}", provider.name(), msg);
                    self.hub.emit(Event::LyricsFailed { track_key: track_key.clone(), error: msg });
                }
            }
        }

        if let Some(lyrics) = found {
            self.hub.emit(Event::LyricsUpdated { track_key: track_key.clone(), lyrics });
        } else {
            self.hub.emit(Event::LyricsFailed {
                track_key: track_key.clone(),
                error: "not found".to_string(),
            });
        }

        let mut pending = self.pending.lock().await;
        pending.remove(&track_key);
    }
}
