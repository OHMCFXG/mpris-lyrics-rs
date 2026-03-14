use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use tokio::sync::broadcast;
use tokio::time;
use crate::config::{Config, DisplayConfig};
use crate::events::{Event, EventHub};
use crate::lyrics::Lyrics;
use crate::state::{GlobalState, StateStore};

pub struct SimpleOutput {
    config: Arc<Config>,
    hub: EventHub,
    store: Arc<StateStore>,
    last_output: String,
}

impl SimpleOutput {
    pub fn new(config: Arc<Config>, hub: EventHub, store: Arc<StateStore>) -> Self {
        Self {
            config,
            hub,
            store,
            last_output: String::new(),
        }
    }

    pub async fn run(self) -> Result<()> {
        let mut this = self;
        let mut rx = this.hub.subscribe();
        let mut tick = time::interval(Duration::from_millis(400));
        loop {
            tokio::select! {
                _ = tick.tick() => {
                    let snapshot = this.store.snapshot().await;
                    if should_tick(&snapshot) {
                        emit_output(&mut this, &snapshot);
                    }
                }
                event = rx.recv() => {
                    let event = match event {
                        Ok(ev) => ev,
                        Err(broadcast::error::RecvError::Closed) => break,
                        Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    };

                    match event {
                        Event::TrackChanged { .. }
                        | Event::PlaybackStatusChanged { .. }
                        | Event::LyricsUpdated { .. }
                        | Event::LyricsFailed { .. }
                        | Event::ActivePlayerChanged { .. } => {
                            let snapshot = this.store.snapshot().await;
                            emit_output(&mut this, &snapshot);
                        }
                        _ => {}
                    }
                }
            }
        }
        Ok(())
    }
}

fn emit_output(this: &mut SimpleOutput, snapshot: &GlobalState) {
    let output = build_output(snapshot, &this.config.display);
    if output != this.last_output {
        println!("{}", output);
        this.last_output = output;
    }
}

fn should_tick(state: &GlobalState) -> bool {
    let Some(active) = &state.active_player else { return false; };
    let Some(player_state) = state.players.get(active) else { return false; };
    player_state.playback_status == crate::state::PlaybackStatus::Playing
}

fn build_output(state: &GlobalState, display: &DisplayConfig) -> String {
    let Some(active) = &state.active_player else {
        return "no player".to_string();
    };
    let Some(player_state) = state.players.get(active) else {
        return "no player".to_string();
    };

    let Some(track) = &player_state.track else {
        return "no track".to_string();
    };

    let position_ms = player_state.estimate_position_ms();
    let position_with_advance = position_ms + display.lyric_advance_time_ms;

    match &state.lyrics.status {
        crate::state::LyricsStatus::Ready => {
            if let Some(lyrics) = &state.lyrics.lyrics {
                if let Some(line) = find_line(lyrics, position_with_advance) {
                    if display.show_timestamp {
                        return format!("[{}] {}", format_time(line.start_time_ms), line.text);
                    }
                    return line.text.clone();
                }
            }
            format_track(track)
        }
        crate::state::LyricsStatus::Loading => "searching lyrics".to_string(),
        crate::state::LyricsStatus::Failed(_) => format_track(track),
        crate::state::LyricsStatus::Idle => format_track(track),
    }
}

fn format_track(track: &crate::state::TrackInfo) -> String {
    if track.artist.is_empty() {
        track.title.clone()
    } else {
        format!("{} - {}", track.title, track.artist)
    }
}

fn find_line<'a>(lyrics: &'a Lyrics, time_ms: u64) -> Option<&'a crate::lyrics::LyricLine> {
    if lyrics.lines.is_empty() {
        return None;
    }

    let idx = lyrics
        .lines
        .partition_point(|line| line.start_time_ms <= time_ms);
    if idx == 0 {
        return lyrics.lines.first();
    }
    lyrics.lines.get(idx - 1)
}

fn format_time(ms: u64) -> String {
    let total_seconds = ms / 1000;
    let minutes = total_seconds / 60;
    let seconds = total_seconds % 60;
    format!("{:02}:{:02}", minutes, seconds)
}
