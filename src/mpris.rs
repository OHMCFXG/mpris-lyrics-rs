use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::Result;
use log::{debug, error, info, warn};
use mpris::{PlaybackStatus as MprisPlaybackStatus, Player, PlayerFinder, TrackID};
use tokio::sync::mpsc::{self, Receiver, Sender};
use tokio::time::{self};

use crate::config::Config;

/// 播放器状态变化事件
#[derive(Debug, Clone)]
pub enum PlayerEvent {
    /// 播放状态改变事件
    PlaybackStatusChanged {
        player_name: String,
        status: PlaybackStatus,
    },
    /// 轨道变更事件
    TrackChanged {
        player_name: String,
        track_info: TrackInfo,
    },
    /// 播放位置变更事件
    PositionChanged {
        player_name: String,
        position_ms: u64,
    },
    /// 播放器消失事件
    PlayerDisappeared { player_name: String },
    /// 播放器出现事件
    PlayerAppeared { player_name: String },
    /// 当前活跃播放器变更事件
    ActivePlayerChanged {
        player_name: String,
        /// 导致此播放器变为活跃的状态
        status: PlaybackStatus,
    },
}

/// 播放状态
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlaybackStatus {
    Playing,
    Paused,
    Stopped,
}

/// 轨道信息
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrackInfo {
    /// 歌曲标题
    pub title: String,
    /// 艺术家
    pub artist: String,
    /// 专辑
    pub album: String,
    /// 歌曲时长（毫秒）
    pub length_ms: u64,
    /// 唯一ID
    pub id: TrackID,
}

impl Default for TrackInfo {
    fn default() -> Self {
        Self {
            title: String::new(),
            artist: String::new(),
            album: String::new(),
            length_ms: 0,
            id: TrackID::new("/org/mpris/MediaPlayer2/TrackList/NoTrack")
                .expect("Failed to create default TrackID"),
        }
    }
}

/// MPRIS监听器，负责监听播放器事件
pub struct MprisListener {
    /// 播放器事件发送通道
    event_sender: Sender<PlayerEvent>,
    /// 播放器黑名单
    player_blacklist: HashSet<String>,
    /// 轮询间隔
    poll_interval: Duration,
    /// 播放器状态缓存
    player_states: Arc<Mutex<HashMap<String, PlayerState>>>,
    /// 当前活跃播放器名称
    active_player_name: Arc<Mutex<Option<String>>>,
}

/// 用于缓存每个播放器的状态
#[derive(Debug, Clone)]
struct PlayerState {
    track_info: Option<TrackInfo>,
    playback_status: Option<PlaybackStatus>,
    last_position_ms: u64,
}

impl MprisListener {
    /// 创建新的MPRIS监听器
    pub fn new(event_sender: Sender<PlayerEvent>, config: &Arc<Config>) -> Self {
        Self {
            event_sender,
            player_blacklist: config.player_blacklist.clone(),
            poll_interval: Duration::from_secs(config.mpris.sync_interval_seconds.max(1)),
            player_states: Arc::new(Mutex::new(HashMap::new())),
            active_player_name: Arc::new(Mutex::new(None)),
        }
    }

    /// 启动MPRIS监听循环（后台任务）
    fn start_polling_task(&self) {
        // 克隆需要在后台任务中使用的数据
        let sender = self.event_sender.clone();
        let blacklist = self.player_blacklist.clone(); // 克隆 HashSet
        let poll_interval = self.poll_interval;
        let player_states = Arc::clone(&self.player_states);
        let active_player_name = Arc::clone(&self.active_player_name);

        tokio::spawn(async move {
            let mut interval = time::interval(poll_interval);
            loop {
                interval.tick().await;
                // 在后台任务中调用轮询逻辑
                if let Err(e) = Self::poll_players_and_send_events_static(
                    &sender,
                    &blacklist,
                    &player_states,
                    &active_player_name,
                )
                .await
                {
                    error!("轮询播放器状态时出错: {}", e);
                    // 避免错误快速循环
                    time::sleep(Duration::from_secs(5)).await;
                }
            }
        });
    }

    /// 轮询逻辑，现在是静态方法，接收所有需要的状态
    async fn poll_players_and_send_events_static(
        sender: &Sender<PlayerEvent>,
        blacklist: &HashSet<String>,
        player_states_arc: &Arc<Mutex<HashMap<String, PlayerState>>>,
        active_player_name_arc: &Arc<Mutex<Option<String>>>,
    ) -> Result<()> {
        // 1. 获取所有非黑名单播放器的当前状态
        let (current_states_data, current_playing_players, current_paused_players) =
            Self::fetch_current_player_states(blacklist).await?; // 传递 blacklist 引用

        let mut events_to_send = Vec::new();
        let mut active_player_event_opt = None; // 重命名变量

        // 2. 与缓存状态比较，生成事件
        {
            let mut player_states_guard = player_states_arc.lock().unwrap();
            let mut active_player_name_guard = active_player_name_arc.lock().unwrap();

            // 比较状态并生成基础事件
            Self::compare_states_and_generate_events(
                &current_states_data,
                &mut player_states_guard,
                &mut events_to_send,
            );

            // 确定新的活跃播放器
            active_player_event_opt = Self::determine_and_update_active_player(
                &current_playing_players,
                &current_paused_players,
                &current_states_data,
                &mut active_player_name_guard,
            );

            // 更新状态缓存
            *player_states_guard = current_states_data;
        } // 锁释放

        // 3. 发送事件
        if let Some(active_event) = active_player_event_opt {
            events_to_send.push(active_event);
        }

        for event in events_to_send {
            if let Err(e) = sender.send(event).await {
                error!("发送播放器事件失败: {}", e);
                // 通道关闭可能意味着退出，记录错误但允许循环继续尝试
            }
        }

        Ok(())
    }

    /// 获取所有非黑名单播放器的当前状态 (spawn_blocking)
    async fn fetch_current_player_states(
        blacklist: &HashSet<String>,
    ) -> Result<(HashMap<String, PlayerState>, Vec<String>, Vec<String>)> {
        let blacklist_clone = blacklist.clone(); // 克隆 blacklist 以满足 'static 要求
        tokio::task::spawn_blocking(move || {
            let finder =
                PlayerFinder::new().map_err(|e| anyhow::anyhow!("创建PlayerFinder失败: {}", e))?;
            let players = finder
                .find_all()
                .map_err(|e| anyhow::anyhow!("查找播放器失败: {}", e))?;

            let mut current_states: HashMap<String, PlayerState> = HashMap::new();
            let mut current_playing_players: Vec<String> = Vec::new();
            let mut current_paused_players: Vec<String> = Vec::new();

            for player in players {
                let identity = player.identity().to_string();
                // 使用克隆的 blacklist
                if Self::is_blacklisted(&identity, &blacklist_clone) {
                    continue;
                }

                let (state, status_opt) = Self::get_player_state(&player);
                if let Some(status) = status_opt {
                    match status {
                        PlaybackStatus::Playing => current_playing_players.push(identity.clone()),
                        PlaybackStatus::Paused => current_paused_players.push(identity.clone()),
                        PlaybackStatus::Stopped => {}
                    }
                }
                current_states.insert(identity, state);
            }
            Ok((
                current_states,
                current_playing_players,
                current_paused_players,
            ))
        })
        .await? // 等待spawn_blocking完成并处理内部Result
    }

    /// 获取单个播放器的状态
    fn get_player_state(player: &Player) -> (PlayerState, Option<PlaybackStatus>) {
        let identity_str = player.identity().to_string(); // 获取 identity 字符串用于日志
        let mut state = PlayerState {
            track_info: None,
            playback_status: None,
            last_position_ms: 0,
        };
        let mut current_status = None;

        match Self::get_playback_status(player) {
            Ok(status) => {
                state.playback_status = Some(status.clone());
                current_status = Some(status);
            }
            // 使用获取到的 identity_str
            Err(e) => debug!("无法获取播放器 {} 的状态: {}", identity_str, e),
        }
        match Self::extract_track_info(player) {
            Ok(track_info) => state.track_info = Some(track_info),
            // 使用获取到的 identity_str
            Err(e) => debug!("无法获取播放器 {} 的轨道信息: {}", identity_str, e),
        }
        if let Ok(pos) = player.get_position() {
            state.last_position_ms = pos.as_millis() as u64;
        } else if state.playback_status == Some(PlaybackStatus::Playing) {
            // 使用获取到的 identity_str
            debug!("无法获取播放器 {} 的播放位置", identity_str);
        }
        (state, current_status)
    }

    /// 比较新旧状态，生成事件并更新旧状态缓存
    fn compare_states_and_generate_events(
        current_states_data: &HashMap<String, PlayerState>,
        old_states: &mut HashMap<String, PlayerState>, // 可变引用传入
        events_to_send: &mut Vec<PlayerEvent>,
    ) {
        // 处理消失的播放器
        let old_keys: HashSet<String> = old_states.keys().cloned().collect();
        let current_keys: HashSet<String> = current_states_data.keys().cloned().collect();

        for identity in old_keys.difference(&current_keys) {
            info!("播放器消失: {}", identity);
            events_to_send.push(PlayerEvent::PlayerDisappeared {
                player_name: identity.clone(),
            });
            // 从 old_states (缓存) 中移除已消失的播放器
            // 注意：old_states 现在是 current_states_data 的可变引用，移除操作会直接修改缓存
            // old_states.remove(identity); // 不再需要，因为下面会用 current_states_data 覆盖
        }

        // 处理出现和更新的播放器
        for (identity, current_state) in current_states_data.iter() {
            if let Some(old_state) = old_states.get(identity) {
                // 更新现有播放器状态比较
                Self::compare_single_player_state(
                    identity,
                    old_state,
                    current_state,
                    events_to_send,
                );
            } else {
                // 新出现的播放器
                info!("播放器出现: {}", identity);
                events_to_send.push(PlayerEvent::PlayerAppeared {
                    player_name: identity.clone(),
                });
                // 如果新播放器有状态，也发送初始状态事件
                if let Some(status) = &current_state.playback_status {
                    events_to_send.push(PlayerEvent::PlaybackStatusChanged {
                        player_name: identity.clone(),
                        status: status.clone(),
                    });
                }
                if let Some(track) = &current_state.track_info {
                    events_to_send.push(PlayerEvent::TrackChanged {
                        player_name: identity.clone(),
                        track_info: track.clone(),
                    });
                }
            }
        }
    }

    /// 比较单个播放器的新旧状态并生成事件
    fn compare_single_player_state(
        identity: &str,
        old_state: &PlayerState,
        current_state: &PlayerState,
        events_to_send: &mut Vec<PlayerEvent>,
    ) {
        // 比较播放状态
        if old_state.playback_status != current_state.playback_status {
            if let Some(new_status) = &current_state.playback_status {
                debug!(
                    // 改为 debug 级别，避免过多 info 日志
                    "播放器 {} 状态改变: {:?} -> {:?}",
                    identity, old_state.playback_status, new_status
                );
                events_to_send.push(PlayerEvent::PlaybackStatusChanged {
                    player_name: identity.to_string(),
                    status: new_status.clone(),
                });
            }
        }

        // 比较轨道信息
        if old_state.track_info != current_state.track_info {
            if let Some(new_track) = &current_state.track_info {
                // 轨道变更通常是重要信息，保留 info
                info!(
                    "播放器 {} 轨道改变: {} - {}",
                    identity,
                    new_track.title,
                    new_track.artist // 简化日志输出
                );
                events_to_send.push(PlayerEvent::TrackChanged {
                    player_name: identity.to_string(),
                    track_info: new_track.clone(),
                });
            }
        }

        // 比较播放位置 (仅当正在播放时)
        if current_state.playback_status == Some(PlaybackStatus::Playing) {
            let old_pos = old_state.last_position_ms;
            let new_pos = current_state.last_position_ms;
            // 仅在位置显著变化时发送事件 (例如 > 500ms)
            // 并且新位置必须大于旧位置（处理回跳或重新开始的情况）
            if new_pos > old_pos && new_pos.saturating_sub(old_pos) > 500 {
                events_to_send.push(PlayerEvent::PositionChanged {
                    player_name: identity.to_string(),
                    position_ms: new_pos,
                });
            }
        }
    }

    /// 确定活跃播放器并生成事件（如果发生变化）
    fn determine_and_update_active_player(
        current_playing_players: &[String],
        current_paused_players: &[String],
        current_states_data: &HashMap<String, PlayerState>,
        active_player_name_guard: &mut Option<String>, // 可变引用传入
    ) -> Option<PlayerEvent> {
        let mut new_active_player_event = None;
        let new_active_player_identity: Option<String> = if !current_playing_players.is_empty() {
            // 优先选择第一个正在播放的
            current_playing_players.first().cloned()
        } else if !current_paused_players.is_empty() {
            // 其次选择第一个暂停的
            current_paused_players.first().cloned()
        } else {
            // 都没有则无活跃播放器
            None
        };

        // 检查活跃播放器是否变化
        if *active_player_name_guard != new_active_player_identity {
            if let Some(new_active_name) = &new_active_player_identity {
                // 获取新活跃播放器的状态
                if let Some(active_player_state) = current_states_data.get(new_active_name) {
                    if let Some(status) = &active_player_state.playback_status {
                        info!("活跃播放器变更为: {}", new_active_name);
                        new_active_player_event = Some(PlayerEvent::ActivePlayerChanged {
                            player_name: new_active_name.clone(),
                            status: status.clone(), // 使用其实际状态
                        });
                    } else {
                        // 理论上 new_active_player_identity 来自 playing 或 paused 列表，应该有状态
                        warn!("新活跃播放器 {} 没有找到状态信息", new_active_name);
                    }
                } else {
                    warn!("新活跃播放器 {} 没有找到状态数据", new_active_name);
                }
            } else {
                info!("没有活跃播放器");
                // 可以选择发送一个 "NoActivePlayer" 事件，或者让接收方处理 Option<String>
                // 当前设计是 active_player_name 变为 None
            }
            // 更新缓存的活跃播放器名称
            *active_player_name_guard = new_active_player_identity;
        }
        new_active_player_event
    }

    /// 检查播放器是否在黑名单中
    fn is_blacklisted(player_name: &str, blacklist: &HashSet<String>) -> bool {
        for keyword in blacklist {
            if player_name.to_lowercase().contains(&keyword.to_lowercase()) {
                debug!("播放器 {} 匹配黑名单关键词 {}", player_name, keyword);
                return true;
            }
        }
        false
    }

    /// 从MPRIS播放器提取轨道信息
    fn extract_track_info(player: &Player) -> Result<TrackInfo> {
        let metadata = player.get_metadata()?;
        let identity = player.identity();

        debug!("原始元数据: player={}, metadata={:?}", identity, metadata);

        let title = metadata.title().unwrap_or("未知标题").to_string();
        let artists = metadata.artists().unwrap_or_default();
        let artist = if !artists.is_empty() {
            artists.join(", ")
        } else {
            "未知艺术家".to_string()
        };
        let album = metadata.album_name().unwrap_or("未知专辑").to_string();
        let length_ms = metadata.length().map(|d| d.as_millis() as u64).unwrap_or(0);

        let id = metadata.track_id().unwrap_or_else(|| {
            let pseudo_id_str = format!(
                "/org/mpris/MediaPlayer2/Track/{}:{}:{}:{}",
                identity, title, artist, album
            );
            warn!(
                "播放器 {} 未提供 track_id，生成伪 ID: {}",
                identity, pseudo_id_str
            );
            TrackID::new(&pseudo_id_str).unwrap_or_else(|_| {
                TrackID::new("/org/mpris/MediaPlayer2/TrackList/FallbackId")
                    .expect("Failed to create fallback TrackID")
            })
        });

        let track_info = TrackInfo {
            title,
            artist,
            album,
            length_ms,
            id,
        };

        debug!("提取到的轨道信息 for {}: {:?}", identity, track_info);
        Ok(track_info)
    }

    /// 获取播放状态
    fn get_playback_status(player: &Player) -> Result<PlaybackStatus> {
        match player.get_playback_status()? {
            MprisPlaybackStatus::Playing => Ok(PlaybackStatus::Playing),
            MprisPlaybackStatus::Paused => Ok(PlaybackStatus::Paused),
            MprisPlaybackStatus::Stopped => Ok(PlaybackStatus::Stopped),
        }
    }
}

/// 创建MPRIS监听器并返回接收事件的通道
pub fn setup_mpris_listener(config: &Arc<Config>) -> Result<Receiver<PlayerEvent>> {
    let (tx, rx) = mpsc::channel(100); // Use a reasonable buffer size
    let listener = MprisListener::new(tx, config);
    listener.start_polling_task(); // 启动后台轮询任务
    Ok(rx) // 直接返回 receiver，不等待后台任务
}
