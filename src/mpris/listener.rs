use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::Result;
use log::{debug, error, warn};
use mpris::{PlaybackStatus as MprisPlaybackStatus, Player, PlayerFinder, TrackID};
use tokio::sync::mpsc::{self, Receiver, Sender};
use tokio::time::{self};

use crate::config::Config;
use crate::mpris::events::{
    compare_states_and_generate_events, determine_and_update_active_player,
};
use crate::mpris::types::{PlaybackStatus, PlayerEvent, PlayerState, TrackInfo};

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
            Self::fetch_current_player_states(blacklist).await?;

        let mut events_to_send = Vec::new();

        // 检查是否没有找到任何播放器
        if current_states_data.is_empty() {
            // 检查之前是否有活跃播放器，只有在状态变化时才发送事件
            let mut send_no_players_event = false;
            {
                let mut active_player_name_guard = active_player_name_arc.lock().unwrap();
                if active_player_name_guard.is_some() {
                    // 之前有活跃播放器，现在没有了，需要重置
                    *active_player_name_guard = None;
                    send_no_players_event = true;
                } else {
                    // 确保至少每30秒发送一次无播放器事件，即使状态没有变化
                    // 使用静态变量或定时器可能更好，但这里简化处理
                    static mut LAST_NO_PLAYERS_NOTIFICATION: u64 = 0;
                    let now = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs();

                    // 安全的：此静态变量只在单线程上下文中访问
                    unsafe {
                        if now - LAST_NO_PLAYERS_NOTIFICATION >= 30 {
                            send_no_players_event = true;
                            LAST_NO_PLAYERS_NOTIFICATION = now;
                        }
                    }
                }
            }

            if send_no_players_event {
                debug!("未检测到任何播放器，发送NoPlayersAvailable事件");
                if let Err(e) = sender.send(PlayerEvent::NoPlayersAvailable).await {
                    error!("发送无播放器事件失败: {}", e);
                }
            }

            // 由于没有播放器，不需要进一步处理
            return Ok(());
        }

        // 2. 与缓存状态比较，生成事件
        let active_player_event = {
            let mut player_states_guard = player_states_arc.lock().unwrap();
            let mut active_player_name_guard = active_player_name_arc.lock().unwrap();

            // 比较状态并生成基础事件
            compare_states_and_generate_events(
                &current_states_data,
                &mut player_states_guard,
                &mut events_to_send,
            );

            // 确定新的活跃播放器
            let active_event = determine_and_update_active_player(
                &current_playing_players,
                &current_paused_players,
                &current_states_data,
                &mut active_player_name_guard,
            );

            // 更新状态缓存
            *player_states_guard = current_states_data;

            active_event
        }; // 锁释放

        // 3. 发送事件
        // 先发送基础事件（包括TrackChanged）
        for event in events_to_send {
            if let Err(e) = sender.send(event).await {
                error!("发送播放器事件失败: {}", e);
                // 通道关闭可能意味着退出，记录错误但允许循环继续尝试
            }
        }

        // 然后发送活跃播放器变更事件
        if let Some(active_event) = active_player_event {
            if let Err(e) = sender.send(active_event).await {
                error!("发送活跃播放器变更事件失败: {}", e);
            }
        }

        Ok(())
    }

    /// 获取所有非黑名单播放器的当前状态 (spawn_blocking)
    async fn fetch_current_player_states(
        blacklist: &HashSet<String>,
    ) -> Result<(HashMap<String, PlayerState>, Vec<String>, Vec<String>)> {
        let blacklist_clone = blacklist.clone(); // 克隆 blacklist 以满足 'static 要求

        let result = tokio::task::spawn_blocking(move || {
            let mut current_states = HashMap::<String, PlayerState>::new();
            let mut playing_players = Vec::<String>::new();
            let mut paused_players = Vec::<String>::new();

            let player_finder = PlayerFinder::new()?;
            let player_list = match player_finder.find_all() {
                Ok(list) => list,
                Err(e) => {
                    warn!("查找播放器失败 (可能暂时无音乐播放器运行): {}", e);
                    vec![]
                }
            };

            for player in player_list {
                let identity = player.identity().to_string(); // 确保保存为String
                                                              // 显式绑定 Option<String> 以延长其生命周期
                let desktop_entry_opt: Option<String> = player.get_desktop_entry().ok().flatten();
                // 从有生命周期的 Option<String> 获取 &str
                let desktop_entry_ref: &str = desktop_entry_opt.as_deref().unwrap_or("");
                if is_blacklisted(&identity, desktop_entry_ref, &blacklist_clone) {
                    debug!("播放器 {} 在黑名单中，跳过", identity);
                    continue;
                }

                let (state, _status) = match Self::get_player_state(&player) {
                    (state, Some(PlaybackStatus::Playing)) => {
                        playing_players.push(identity.clone());
                        (state, Some(PlaybackStatus::Playing))
                    }
                    (state, Some(PlaybackStatus::Paused)) => {
                        paused_players.push(identity.clone());
                        (state, Some(PlaybackStatus::Paused))
                    }
                    (state, status) => (state, status),
                };

                current_states.insert(identity, state);
            }

            Ok::<_, anyhow::Error>((current_states, playing_players, paused_players))
        })
        .await??;

        Ok(result)
    }

    /// 获取单个播放器的状态
    fn get_player_state(player: &Player) -> (PlayerState, Option<PlaybackStatus>) {
        // 提取轨道信息
        let track_info = extract_track_info(player).ok();

        // 获取播放状态
        let status = get_playback_status(player).ok();

        // 获取播放位置
        let position_ms = player
            .get_position()
            .unwrap_or_else(|_| Duration::from_secs(0))
            .as_millis() as u64;

        let state = PlayerState {
            track_info,
            playback_status: status.clone(),
            last_position_ms: position_ms,
        };

        (state, status)
    }
}

/// 检查播放器是否在黑名单中
fn is_blacklisted(identity: &str, desktop_entry: &str, blacklist: &HashSet<String>) -> bool {
    let identity_lower = identity.to_lowercase();
    let desktop_entry_lower = desktop_entry.to_lowercase();

    for pattern in blacklist {
        let pattern_lower = pattern.to_lowercase();
        if identity_lower.contains(&pattern_lower) || desktop_entry_lower.contains(&pattern_lower) {
            debug!(
                "播放器匹配黑名单模式 '{}': identity='{}', desktop_entry='{}'",
                pattern, identity, desktop_entry
            );
            return true;
        }
    }
    false
}

/// 从播放器提取轨道信息
fn extract_track_info(player: &Player) -> Result<TrackInfo> {
    let metadata = player.get_metadata()?;

    // 尝试获取轨道信息
    let title = metadata.title().unwrap_or_else(|| "未知标题").to_string();
    let artist = metadata
        .artists()
        .map(|artists| artists.join(", "))
        .unwrap_or_else(|| "未知艺术家".to_string());
    let album = metadata
        .album_name()
        .unwrap_or_else(|| "未知专辑")
        .to_string();
    let length_ms = metadata.length().map(|d| d.as_millis() as u64).unwrap_or(0);

    // 获取轨道ID，确保它存在，如果不存在则使用默认值
    let id = metadata.track_id().clone().unwrap_or_else(|| {
        // 如果没有ID，则创建一个默认的
        TrackID::new("/org/mpris/MediaPlayer2/TrackList/NoTrack").expect("无法创建默认 TrackID")
    });

    debug!(
        "获取轨道信息: 标题='{}', 艺术家='{}', 专辑='{}', 时长={}ms, ID={}",
        title, artist, album, length_ms, id
    );

    Ok(TrackInfo {
        title,
        artist,
        album,
        length_ms,
        id,
    })
}

/// 获取播放器的播放状态
fn get_playback_status(player: &Player) -> Result<PlaybackStatus> {
    match player.get_playback_status()? {
        MprisPlaybackStatus::Playing => Ok(PlaybackStatus::Playing),
        MprisPlaybackStatus::Paused => Ok(PlaybackStatus::Paused),
        MprisPlaybackStatus::Stopped => Ok(PlaybackStatus::Stopped),
    }
}

/// 设置MPRIS监听器并返回事件接收通道
pub fn setup_mpris_listener(config: &Arc<Config>) -> Result<Receiver<PlayerEvent>> {
    let (tx, rx) = mpsc::channel(100);
    let listener = MprisListener::new(tx, config);
    listener.start_polling_task();
    Ok(rx)
}
