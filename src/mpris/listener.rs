use anyhow::Result;
use log::{error, info, warn};
use mpris::{PlayerFinder, TrackID};
use std::collections::HashMap;
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use tokio::sync::mpsc::{self, Receiver, Sender};

use crate::config::Config;
use crate::mpris::events::{compare_states_and_generate_events, determine_and_update_active_player};
use crate::mpris::types::{PlaybackStatus, PlayerEvent, PlayerState, TrackInfo};

/// 设置 MPRIS 监听器
pub fn setup_mpris_listener(config: &Config) -> Result<Receiver<PlayerEvent>> {
    let (tx, rx) = mpsc::channel(100);
    let config = Arc::new(config.clone());

    // 使用 std::thread::spawn 而不是 tokio::spawn，因为 mpris::PlayerFinder 不是 Send
    // DBus 连接通常绑定到特定线程
    thread::spawn(move || {
        // 外层重试循环：处理 D-Bus 连接失败
        loop {
            match PlayerFinder::new() {
                Ok(finder) => {
                    info!("MPRIS 监听器已连接到 D-Bus");
                    // 运行监听循环，如果出错则返回错误信息
                    if let Err(e) = run_listener_loop(finder, &tx, &config) {
                        error!("MPRIS 监听器异常退出: {}, 5秒后重试", e);
                        thread::sleep(Duration::from_secs(5));
                    } else {
                        // 正常退出（通道关闭），不重试
                        info!("MPRIS 监听器正常退出");
                        break;
                    }
                }
                Err(e) => {
                    error!("D-Bus 连接失败: {}, 5秒后重试", e);
                    thread::sleep(Duration::from_secs(5));
                }
            }
        }
    });

    Ok(rx)
}

/// 运行监听循环
fn run_listener_loop(
    player_finder: PlayerFinder,
    tx: &Sender<PlayerEvent>,
    config: &Arc<Config>,
) -> Result<()> {
    let mut old_states: HashMap<String, PlayerState> = HashMap::new();
    let mut active_player_name: Option<String> = None;
    // 跟踪每个播放器的最后轮询时间
    let mut player_last_poll: HashMap<String, std::time::Instant> = HashMap::new();

    loop {
        let now = std::time::Instant::now();
        
        let mut events_to_send = Vec::new();
        let mut current_states_data: HashMap<String, PlayerState> = HashMap::new();
        let mut current_playing_players = Vec::new();
        let mut current_paused_players = Vec::new();

        // 查找所有播放器
        match player_finder.find_all() {
            Ok(players) => {
                for player in players {
                    let identity = player.identity().to_string();
                    let bus_name = player.bus_name().to_string();

                    // 检查黑名单
                    let is_blacklisted = config.player_blacklist.iter().any(|keyword| {
                        identity.to_lowercase().contains(&keyword.to_lowercase())
                            || bus_name.to_lowercase().contains(&keyword.to_lowercase())
                    });

                    if is_blacklisted {
                        continue;
                    }

                    // 差异化轮询：根据播放器状态决定是否需要查询
                    let should_poll = should_poll_player(
                        &identity,
                        &old_states,
                        &player_last_poll,
                        &active_player_name,
                        now,
                    );

                    if !should_poll {
                        // 跳过此播放器，使用旧状态
                        if let Some(old_state) = old_states.get(&identity) {
                            current_states_data.insert(identity.clone(), old_state.clone());
                            
                            // 更新playing/paused列表
                            if let Some(status) = &old_state.playback_status {
                                match status {
                                    PlaybackStatus::Playing => current_playing_players.push(identity.clone()),
                                    PlaybackStatus::Paused => current_paused_players.push(identity.clone()),
                                    _ => {}
                                }
                            }
                        }
                        continue;
                    }

                    // 更新轮询时间
                    player_last_poll.insert(identity.clone(), now);

                    // 获取播放状态
                    let playback_status = match player.get_playback_status() {
                        Ok(status) => match status {
                            mpris::PlaybackStatus::Playing => Some(PlaybackStatus::Playing),
                            mpris::PlaybackStatus::Paused => Some(PlaybackStatus::Paused),
                            mpris::PlaybackStatus::Stopped => Some(PlaybackStatus::Stopped),
                        },
                        Err(_) => None,
                    };

                    // 获取元数据
                    let track_info = match player.get_metadata() {
                        Ok(metadata) => {
                            let title = metadata.title().unwrap_or("Unknown Title").to_string();
                            let artist = metadata
                                .artists()
                                .map(|a| a.join(", "))
                                .unwrap_or_else(|| "Unknown Artist".to_string());
                            let album = metadata.album_name().unwrap_or("").to_string();
                            let length_ms = metadata.length().map(|d| d.as_millis() as u64).unwrap_or(0);
                            let id = metadata.track_id().unwrap_or_else(|| {
                                TrackID::new("/org/mpris/MediaPlayer2/TrackList/NoTrack").unwrap()
                            });

                            Some(TrackInfo {
                                title,
                                artist,
                                album,
                                length_ms,
                                id,
                            })
                        }
                        Err(_) => None,
                    };

                    // 获取播放位置
                    let position_ms = if playback_status == Some(PlaybackStatus::Playing) {
                        player.get_position().map(|d| d.as_millis() as u64).unwrap_or(0)
                    } else {
                        0
                    };

                    // 记录当前状态
                    if let Some(status) = &playback_status {
                        match status {
                            PlaybackStatus::Playing => current_playing_players.push(identity.clone()),
                            PlaybackStatus::Paused => current_paused_players.push(identity.clone()),
                            _ => {}
                        }
                    }

                    let state = PlayerState {
                        track_info,
                        playback_status,
                        last_position_ms: position_ms,
                    };

                    current_states_data.insert(identity, state);
                }
            }
            Err(e) => {
                warn!("查找播放器失败: {}", e);
            }
        }

        // 清理已消失播放器的轮询记录
        player_last_poll.retain(|name, _| current_states_data.contains_key(name));

        // 1. 比较状态并生成事件
        compare_states_and_generate_events(
            &current_states_data,
            &mut old_states,
            &mut events_to_send,
        );

        // 2. 确定并更新活跃播放器
        if let Some(event) = determine_and_update_active_player(
            &current_playing_players,
            &current_paused_players,
            &current_states_data,
            &mut active_player_name,
        ) {
            events_to_send.push(event);
        }

        // 如果没有播放器，且之前有活跃播放器，发送 NoPlayersAvailable
        if current_states_data.is_empty() && !old_states.is_empty() {
            events_to_send.push(PlayerEvent::NoPlayersAvailable);
        }

        // 更新旧状态
        old_states = current_states_data;

        // 发送事件
        for event in events_to_send {
            if let Err(e) = tx.blocking_send(event) {
                error!("发送 MPRIS 事件失败: {}", e);
                return Ok(()); // 通道关闭，正常退出
            }
        }

        // 计算下次唤醒时间（动态间隔）
        let sleep_duration = calculate_next_poll_interval(&old_states, &active_player_name);
        thread::sleep(sleep_duration);
    }
}

/// 判断是否需要轮询该播放器
fn should_poll_player(
    player_name: &str,
    old_states: &HashMap<String, PlayerState>,
    player_last_poll: &HashMap<String, std::time::Instant>,
    active_player_name: &Option<String>,
    now: std::time::Instant,
) -> bool {
    // 首次发现的播放器，立即轮询
    let last_poll = match player_last_poll.get(player_name) {
        Some(time) => *time,
        None => return true,
    };

    // 获取播放器状态
    let player_status = old_states
        .get(player_name)
        .and_then(|state| state.playback_status.clone());

    // 确定轮询间隔
    let poll_interval = if active_player_name.as_ref() == Some(&player_name.to_string()) {
        // 活跃播放器：根据状态决定
        match player_status {
            Some(PlaybackStatus::Playing) => Duration::from_millis(500),  // 高频
            Some(PlaybackStatus::Paused) => Duration::from_millis(2000),  // 中频
            _ => Duration::from_secs(1),  // 其他状态
        }
    } else {
        // 非活跃播放器：低频轮询
        Duration::from_secs(5)
    };

    // 检查是否到达轮询时间
    now.duration_since(last_poll) >= poll_interval
}

/// 计算下次轮询的休眠时间
fn calculate_next_poll_interval(
    old_states: &HashMap<String, PlayerState>,
    active_player_name: &Option<String>,
) -> Duration {
    // 如果没有播放器，使用默认间隔
    if old_states.is_empty() {
        return Duration::from_secs(1);
    }

    // 获取活跃播放器的状态
    if let Some(active_name) = active_player_name {
        if let Some(state) = old_states.get(active_name) {
            return match state.playback_status {
                Some(PlaybackStatus::Playing) => Duration::from_millis(500),  // 播放中：高频
                Some(PlaybackStatus::Paused) => Duration::from_millis(2000),  // 暂停：中频
                _ => Duration::from_secs(1),  // 其他：正常
            };
        }
    }

    // 没有活跃播放器，使用正常间隔
    Duration::from_secs(1)
}
