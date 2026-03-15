use std::collections::HashMap;

use crate::state::{PlaybackStatus, PlayerState};

pub fn select_active_player(players: &HashMap<String, PlayerState>) -> Option<String> {
    select_first_by_status(players, PlaybackStatus::Playing)
        .or_else(|| select_first_by_status(players, PlaybackStatus::Paused))
        .or_else(|| select_first_by_status(players, PlaybackStatus::Stopped))
}

pub fn select_next_player(
    players: &HashMap<String, PlayerState>,
    current: Option<&str>,
) -> Option<String> {
    let names = sorted_player_names(players);
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
    let names = sorted_player_names(players);
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

fn sorted_player_names(players: &HashMap<String, PlayerState>) -> Vec<String> {
    let mut names: Vec<String> = players.keys().cloned().collect();
    names.sort();
    names
}

fn select_first_by_status(
    players: &HashMap<String, PlayerState>,
    status: PlaybackStatus,
) -> Option<String> {
    players
        .iter()
        .filter(|(_, state)| state.playback_status == status)
        .map(|(name, _)| name)
        .min()
        .cloned()
}
