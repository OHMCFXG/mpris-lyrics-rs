use log::debug;
use std::collections::HashMap;

use crate::mpris::types::{PlaybackStatus, PlayerEvent, PlayerState};

/// 比较播放器状态并生成事件
pub fn compare_states_and_generate_events(
    current_states_data: &HashMap<String, PlayerState>,
    old_states: &mut HashMap<String, PlayerState>, // 可变引用传入
    events_to_send: &mut Vec<PlayerEvent>,
) {
    // 检查是否有新出现的播放器或消失的播放器
    for (identity, current_state) in current_states_data.iter() {
        if !old_states.contains_key(identity) {
            // 新播放器出现
            debug!("播放器首次出现: {}", identity);
            events_to_send.push(PlayerEvent::PlayerAppeared {
                player_name: identity.clone(),
            });

            // 对于新出现的播放器，立即发送其轨道信息（如果有）
            if let Some(track_info) = &current_state.track_info {
                debug!(
                    "为新播放器发送轨道信息: {} - {}",
                    identity, track_info.title
                );
                events_to_send.push(PlayerEvent::TrackChanged {
                    player_name: identity.clone(),
                    track_info: track_info.clone(),
                });
            }

            // 对于新出现的播放器，也要发送其播放状态（如果有）
            if let Some(playback_status) = &current_state.playback_status {
                debug!(
                    "为新播放器发送播放状态: {} - {:?}",
                    identity, playback_status
                );
                events_to_send.push(PlayerEvent::PlaybackStatusChanged {
                    player_name: identity.clone(),
                    status: playback_status.clone(),
                });
            }
        }
    }

    for (identity, _) in old_states.iter() {
        if !current_states_data.contains_key(identity) {
            // 播放器消失
            events_to_send.push(PlayerEvent::PlayerDisappeared {
                player_name: identity.clone(),
            });
        }
    }

    // 比较每个播放器的状态变化
    for (identity, current_state) in current_states_data.iter() {
        if let Some(old_state) = old_states.get(identity) {
            compare_single_player_state(identity, old_state, current_state, events_to_send);
        }
    }
}

/// 比较单个播放器的状态变化
pub fn compare_single_player_state(
    identity: &str,
    old_state: &PlayerState,
    current_state: &PlayerState,
    events_to_send: &mut Vec<PlayerEvent>,
) {
    // 输出调试信息
    debug!(
        "比较播放器状态: {} - 轨道信息: old={:?}, current={:?}",
        identity,
        old_state.track_info.as_ref().map(|t| &t.title),
        current_state.track_info.as_ref().map(|t| &t.title)
    );

    // 检查轨道是否变化
    let mut track_changed = false;
    let mut change_reason = String::new();

    if let (Some(old_track), Some(current_track)) =
        (&old_state.track_info, &current_state.track_info)
    {
        if old_track.id != current_track.id {
            track_changed = true;
            change_reason = format!("ID: {} -> {}", old_track.id, current_track.id);
        } else if old_track.title != current_track.title {
            track_changed = true;
            change_reason = format!("Title: '{}' -> '{}'", old_track.title, current_track.title);
        } else if old_track.artist != current_track.artist {
            track_changed = true;
            change_reason = format!(
                "Artist: '{}' -> '{}'",
                old_track.artist, current_track.artist
            );
        }

        if track_changed {
            debug!(
                "轨道变更 ({}) for {}: {}",
                change_reason, identity, current_track.title
            );
            events_to_send.push(PlayerEvent::TrackChanged {
                player_name: identity.to_string(),
                track_info: current_track.clone(),
            });
        }
    } else if old_state.track_info.is_none() && current_state.track_info.is_some() {
        // 之前没有轨道，现在有了
        let current_track = current_state.track_info.as_ref().unwrap();
        debug!("新增轨道: {} - {}", identity, current_track.title);
        events_to_send.push(PlayerEvent::TrackChanged {
            player_name: identity.to_string(),
            track_info: current_track.clone(),
        });
    }

    // 检查播放状态是否变化
    if let (Some(old_status), Some(current_status)) =
        (&old_state.playback_status, &current_state.playback_status)
    {
        if old_status != current_status {
            // 播放状态变更
            events_to_send.push(PlayerEvent::PlaybackStatusChanged {
                player_name: identity.to_string(),
                status: current_status.clone(),
            });
        }
    } else if old_state.playback_status.is_none() && current_state.playback_status.is_some() {
        // 之前没有状态，现在有了
        events_to_send.push(PlayerEvent::PlaybackStatusChanged {
            player_name: identity.to_string(),
            status: current_state.playback_status.clone().unwrap(),
        });
    }

    // 检查播放位置是否变化（只对于正在播放的播放器）
    if let Some(current_status) = &current_state.playback_status {
        if *current_status == PlaybackStatus::Playing
            && old_state.last_position_ms != current_state.last_position_ms
        {
            // 位置变更
            events_to_send.push(PlayerEvent::PositionChanged {
                player_name: identity.to_string(),
                position_ms: current_state.last_position_ms,
            });
        }
    }
}

/// 确定并更新活跃播放器
pub fn determine_and_update_active_player(
    current_playing_players: &[String],
    current_paused_players: &[String],
    current_states_data: &HashMap<String, PlayerState>,
    active_player_name_guard: &mut Option<String>, // 可变引用传入
) -> Option<PlayerEvent> {
    // 检查是否需要更改当前活跃播放器
    let need_change = if let Some(current_active) = active_player_name_guard.as_ref() {
        // 如果当前活跃播放器不在当前播放器列表中，则需要更改
        !current_states_data.contains_key(current_active)
    } else {
        // 如果当前没有活跃播放器，且有可用的播放器，则需要更改
        !current_states_data.is_empty()
    };

    if need_change || active_player_name_guard.is_none() {
        // 优先选择正在播放的播放器
        if !current_playing_players.is_empty() {
            // 选择第一个播放中的播放器
            let new_active = current_playing_players[0].clone();
            *active_player_name_guard = Some(new_active.clone());
            return Some(PlayerEvent::ActivePlayerChanged {
                player_name: new_active,
                status: PlaybackStatus::Playing,
            });
        } else if !current_paused_players.is_empty() {
            // 如果没有播放中的播放器，选择第一个暂停的播放器
            let new_active = current_paused_players[0].clone();
            *active_player_name_guard = Some(new_active.clone());
            return Some(PlayerEvent::ActivePlayerChanged {
                player_name: new_active,
                status: PlaybackStatus::Paused,
            });
        } else if !current_states_data.is_empty() {
            // 如果没有播放或暂停的播放器，选择第一个可用的播放器
            let new_active = current_states_data.keys().next().unwrap().clone();
            *active_player_name_guard = Some(new_active.clone());
            return Some(PlayerEvent::ActivePlayerChanged {
                player_name: new_active,
                status: PlaybackStatus::Stopped,
            });
        } else {
            // 没有任何播放器，清除当前活跃播放器
            *active_player_name_guard = None;
        }
    }

    None
}
