use std::collections::HashMap;

use crate::state::{PlaybackStatus, PlayerState};

pub fn select_active_player(players: &HashMap<String, PlayerState>) -> Option<String> {
    let mut playing: Vec<String> = Vec::new();
    let mut paused: Vec<String> = Vec::new();
    let mut stopped: Vec<String> = Vec::new();

    for (name, state) in players {
        match state.playback_status {
            PlaybackStatus::Playing => playing.push(name.clone()),
            PlaybackStatus::Paused => paused.push(name.clone()),
            PlaybackStatus::Stopped => stopped.push(name.clone()),
        }
    }

    playing.sort();
    paused.sort();
    stopped.sort();

    playing
        .into_iter()
        .next()
        .or_else(|| paused.into_iter().next())
        .or_else(|| stopped.into_iter().next())
}

pub fn select_next_player(
    players: &HashMap<String, PlayerState>,
    current: Option<&str>,
) -> Option<String> {
    let mut names: Vec<String> = players.keys().cloned().collect();
    names.sort();
    if names.is_empty() {
        return None;
    }

    match current {
        None => names.first().cloned(),
        Some(cur) => {
            let idx = names.iter().position(|n| n == cur).unwrap_or(0);
            let next_idx = (idx + 1) % names.len();
            Some(names[next_idx].clone())
        }
    }
}

pub fn select_prev_player(
    players: &HashMap<String, PlayerState>,
    current: Option<&str>,
) -> Option<String> {
    let mut names: Vec<String> = players.keys().cloned().collect();
    names.sort();
    if names.is_empty() {
        return None;
    }

    match current {
        None => names.first().cloned(),
        Some(cur) => {
            let idx = names.iter().position(|n| n == cur).unwrap_or(0);
            let prev_idx = if idx == 0 { names.len() - 1 } else { idx - 1 };
            Some(names[prev_idx].clone())
        }
    }
}
