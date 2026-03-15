use std::sync::Arc;
use std::time::Duration;

use crate::config::{Config, DisplayConfig};
use crate::state::GlobalState;
use crate::ui::common;
use anyhow::Result;
use tokio::sync::watch;
use tokio::time;

pub struct SimpleOutput {
    config: Arc<Config>,
    state_rx: watch::Receiver<GlobalState>,
    last_output: String,
}

impl SimpleOutput {
    pub fn new(config: Arc<Config>, state_rx: watch::Receiver<GlobalState>) -> Self {
        Self {
            config,
            state_rx,
            last_output: String::new(),
        }
    }

    pub async fn run(self) -> Result<()> {
        let mut this = self;
        let mut state_rx = this.state_rx.clone();
        let mut tick = time::interval(Duration::from_millis(400));
        loop {
            tokio::select! {
                _ = tick.tick() => {
                    let snapshot = state_rx.borrow().clone();
                    if common::should_tick(&snapshot) {
                        emit_output(&mut this, &snapshot);
                    }
                }
                changed = state_rx.changed() => {
                    if changed.is_err() {
                        break;
                    }
                    let snapshot = state_rx.borrow().clone();
                    emit_output(&mut this, &snapshot);
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
                if let Some(line) = common::find_line(lyrics, position_with_advance) {
                    if display.show_timestamp {
                        return format!(
                            "[{}] {}",
                            common::format_time(line.start_time_ms),
                            line.text
                        );
                    }
                    return line.text.clone();
                }
            }
            common::format_track(track)
        }
        crate::state::LyricsStatus::Loading => "searching lyrics".to_string(),
        crate::state::LyricsStatus::Failed(_) => common::format_track(track),
        crate::state::LyricsStatus::Idle => common::format_track(track),
    }
}
