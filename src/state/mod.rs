use std::collections::HashMap;
use std::time::Instant;

use crate::events::{ActiveReason, Event, UiCommand};
use crate::lyrics::{Lyrics, TrackKey};
use crate::policy;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlaybackStatus {
    Playing,
    Paused,
    Stopped,
}

#[derive(Debug, Clone)]
pub struct TrackInfo {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub length_ms: u64,
    pub track_id: Option<String>,
}

#[derive(Debug, Clone)]
pub struct PlayerState {
    pub playback_status: PlaybackStatus,
    pub track: Option<TrackInfo>,
    pub position_ms: u64,
    pub position_ts: Instant,
    pub rate: f64,
    pub last_seen: Instant,
}

impl PlayerState {
    pub fn new() -> Self {
        let now = Instant::now();
        Self {
            playback_status: PlaybackStatus::Stopped,
            track: None,
            position_ms: 0,
            position_ts: now,
            rate: 1.0,
            last_seen: now,
        }
    }

    pub fn estimate_position_ms(&self) -> u64 {
        if self.playback_status != PlaybackStatus::Playing {
            return self.position_ms;
        }

        let elapsed_ms = self.position_ts.elapsed().as_secs_f64() * 1000.0;
        let delta = (elapsed_ms * self.rate).max(0.0);
        self.position_ms.saturating_add(delta as u64)
    }
}

#[derive(Debug, Clone)]
pub enum LyricsStatus {
    Idle,
    Loading,
    Ready,
    Failed(String),
}

#[derive(Debug, Clone)]
pub struct LyricsState {
    pub track_key: Option<TrackKey>,
    pub lyrics: Option<Lyrics>,
    pub status: LyricsStatus,
    pub fail_count: u32,
    pub last_failed_at: Option<Instant>,
}

impl Default for LyricsState {
    fn default() -> Self {
        Self {
            track_key: None,
            lyrics: None,
            status: LyricsStatus::Idle,
            fail_count: 0,
            last_failed_at: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct GlobalState {
    pub players: HashMap<String, PlayerState>,
    pub active_player: Option<String>,
    pub manual_override: Option<String>,
    pub lyrics: LyricsState,
}

impl Default for GlobalState {
    fn default() -> Self {
        Self {
            players: HashMap::new(),
            active_player: None,
            manual_override: None,
            lyrics: LyricsState::default(),
        }
    }
}

pub struct StateStore {
    state: tokio::sync::RwLock<GlobalState>,
}

impl StateStore {
    pub fn new() -> Self {
        Self {
            state: tokio::sync::RwLock::new(GlobalState::default()),
        }
    }

    pub async fn snapshot(&self) -> GlobalState {
        self.state.read().await.clone()
    }

    pub async fn handle_event(&self, event: &Event) -> Vec<Event> {
        let mut derived = Vec::new();
        let mut state = self.state.write().await;

        match event {
            Event::PlayerAppeared { player } => {
                let entry = state
                    .players
                    .entry(player.clone())
                    .or_insert_with(PlayerState::new);
                touch_player(entry);
                maybe_select_active_if_none(&mut state, &mut derived);
            }
            Event::PlayerDisappeared { player } => {
                state.players.remove(player);
                if state.manual_override.as_ref() == Some(player) {
                    state.manual_override = None;
                }
                if state.active_player.as_ref() == Some(player) {
                    state.active_player = None;
                }
                maybe_select_active(&mut state, ActiveReason::PlayerGone, &mut derived);
            }
            Event::PlaybackStatusChanged { player, status } => {
                let entry = state
                    .players
                    .entry(player.clone())
                    .or_insert_with(PlayerState::new);
                update_status(entry, status.clone());
                maybe_select_active(&mut state, ActiveReason::Auto, &mut derived);
            }
            Event::TrackChanged { player, track } => {
                let entry = state
                    .players
                    .entry(player.clone())
                    .or_insert_with(PlayerState::new);
                update_track(entry, track.clone());
            }
            Event::Seeked {
                player,
                position_ms,
            }
            | Event::PositionUpdated {
                player,
                position_ms,
            } => {
                if let Some(entry) = state.players.get_mut(player) {
                    update_position(entry, *position_ms);
                }
            }
            Event::RateChanged { player, rate } => {
                if let Some(entry) = state.players.get_mut(player) {
                    update_rate(entry, *rate);
                }
            }
            Event::LyricsRequested { track_key } => {
                state.lyrics.track_key = Some(track_key.clone());
                state.lyrics.status = LyricsStatus::Loading;
                state.lyrics.lyrics = None;
                state.lyrics.fail_count = 0;
                state.lyrics.last_failed_at = None;
            }
            Event::LyricsUpdated { track_key, lyrics } => {
                state.lyrics.track_key = Some(track_key.clone());
                state.lyrics.status = LyricsStatus::Ready;
                state.lyrics.lyrics = Some(lyrics.clone());
                state.lyrics.fail_count = 0;
                state.lyrics.last_failed_at = None;
            }
            Event::LyricsFailed { track_key, error } => {
                state.lyrics.track_key = Some(track_key.clone());
                state.lyrics.status = LyricsStatus::Failed(error.clone());
                state.lyrics.lyrics = None;
                state.lyrics.fail_count = state.lyrics.fail_count.saturating_add(1);
                state.lyrics.last_failed_at = Some(Instant::now());
            }
            Event::UiCommand { command } => match command {
                UiCommand::SelectNextPlayer => {
                    if let Some(next) =
                        policy::select_next_player(&state.players, state.active_player.as_deref())
                    {
                        state.manual_override = Some(next.clone());
                        state.active_player = Some(next.clone());
                        derived.push(Event::ActivePlayerChanged {
                            player: next,
                            reason: ActiveReason::Manual,
                        });
                    }
                }
                UiCommand::SelectPrevPlayer => {
                    if let Some(prev) =
                        policy::select_prev_player(&state.players, state.active_player.as_deref())
                    {
                        state.manual_override = Some(prev.clone());
                        state.active_player = Some(prev.clone());
                        derived.push(Event::ActivePlayerChanged {
                            player: prev,
                            reason: ActiveReason::Manual,
                        });
                    }
                }
                UiCommand::Quit | UiCommand::ToggleHelp => {}
            },
            Event::ActivePlayerChanged { .. } => {}
            Event::Shutdown => {}
        }

        derived
    }
}

fn touch_player(player: &mut PlayerState) {
    player.last_seen = Instant::now();
}

fn update_position(player: &mut PlayerState, position_ms: u64) {
    let now = Instant::now();
    player.position_ms = position_ms;
    player.position_ts = now;
    player.last_seen = now;
}

fn update_status(player: &mut PlayerState, status: PlaybackStatus) {
    let now = Instant::now();
    player.playback_status = status;
    player.position_ts = now;
    player.last_seen = now;
}

fn update_rate(player: &mut PlayerState, rate: f64) {
    let now = Instant::now();
    player.rate = rate;
    player.position_ts = now;
    player.last_seen = now;
}

fn update_track(player: &mut PlayerState, track: TrackInfo) {
    let now = Instant::now();
    player.track = Some(track);
    player.position_ms = 0;
    player.position_ts = now;
    player.last_seen = now;
}

fn maybe_select_active_if_none(state: &mut GlobalState, derived: &mut Vec<Event>) {
    if state.active_player.is_none() && state.manual_override.is_none() {
        if let Some(next) = policy::select_active_player(&state.players) {
            state.active_player = Some(next.clone());
            derived.push(Event::ActivePlayerChanged {
                player: next,
                reason: ActiveReason::Initial,
            });
        }
    }
}

fn maybe_select_active(state: &mut GlobalState, reason: ActiveReason, derived: &mut Vec<Event>) {
    if state.manual_override.is_some() {
        return;
    }
    if let Some(next) = policy::select_active_player(&state.players) {
        if state.active_player.as_ref() != Some(&next) {
            state.active_player = Some(next.clone());
            derived.push(Event::ActivePlayerChanged {
                player: next,
                reason,
            });
        }
    }
}
