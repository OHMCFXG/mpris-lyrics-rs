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

    loop {
        // 使用阻塞式休眠
        thread::sleep(Duration::from_millis(500));

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
    }
}
