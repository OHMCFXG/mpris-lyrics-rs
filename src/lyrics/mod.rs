pub mod providers;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use anyhow::Result;
use log::{debug, error, info, warn};
use tokio::sync::mpsc::{Receiver, Sender};

use crate::config::Config;
use crate::mpris::{PlaybackStatus, PlayerEvent, TrackInfo};
use providers::get_enabled_providers;

/// 表示单行歌词
#[derive(Debug, Clone)]
pub struct LyricLine {
    /// 开始时间（毫秒）
    pub start_time: u64,
    /// 结束时间（毫秒，可选）
    pub end_time: Option<u64>,
    /// 歌词文本
    pub text: String,
}

/// 完整的歌词
#[derive(Debug, Clone, Default)]
pub struct Lyrics {
    /// 歌词元数据
    pub metadata: LyricsMetadata,
    /// 按时间排序的歌词行
    pub lines: Vec<LyricLine>,
}

/// 歌词元数据
#[derive(Debug, Clone, Default)]
pub struct LyricsMetadata {
    /// 歌曲标题
    pub title: String,
    /// 艺术家
    pub artist: String,
    /// 专辑
    pub album: String,
    /// 歌词来源
    pub source: String,
    /// 其他元数据
    pub extra: HashMap<String, String>,
}

/// 歌词提供者接口
pub trait LyricsProvider: Send + Sync {
    /// 获取提供者名称
    fn name(&self) -> &str;

    /// 搜索歌词
    fn search_lyrics(&self, track: &TrackInfo) -> Result<Option<Lyrics>>;
}

/// 歌词管理器
#[derive(Clone)]
pub struct LyricsManager {
    providers: Arc<Vec<Arc<dyn LyricsProvider>>>,
    current_lyrics: Arc<Mutex<HashMap<String, Lyrics>>>,
    current_player: Arc<Mutex<Option<String>>>,
    current_track: Arc<Mutex<HashMap<String, TrackInfo>>>,
    player_status: Arc<Mutex<HashMap<String, PlaybackStatus>>>,
    event_sender: Option<Sender<PlayerEvent>>,
}

impl LyricsManager {
    /// 创建新的歌词管理器
    pub fn new(providers: Vec<Arc<dyn LyricsProvider>>) -> Self {
        Self {
            providers: Arc::new(providers),
            current_lyrics: Arc::new(Mutex::new(HashMap::new())),
            current_player: Arc::new(Mutex::new(None)),
            current_track: Arc::new(Mutex::new(HashMap::new())),
            player_status: Arc::new(Mutex::new(HashMap::new())),
            event_sender: None,
        }
    }

    /// 设置事件发送器
    pub fn set_event_sender(&mut self, sender: Sender<PlayerEvent>) {
        self.event_sender = Some(sender);
    }

    /// 启动歌词管理器处理循环
    pub async fn run(&self, mut player_events: Receiver<PlayerEvent>) -> Result<()> {
        info!("歌词管理器启动");

        while let Some(event) = player_events.recv().await {
            match event {
                PlayerEvent::TrackChanged {
                    player_name,
                    track_info,
                } => {
                    info!("轨道变更: {} - {}", player_name, track_info.title);
                    self.handle_track_changed(player_name, track_info).await?;
                }
                PlayerEvent::PlaybackStatusChanged {
                    player_name,
                    status,
                } => {
                    debug!("播放状态变更: {} - {:?}", player_name, status);

                    // 更新播放器状态映射
                    {
                        let mut player_status = self.player_status.lock().unwrap();
                        player_status.insert(player_name.clone(), status.clone());
                    }

                    // 检查是否需要切换当前活跃播放器
                    match status {
                        PlaybackStatus::Playing => {
                            // 如果有播放器开始播放，立即切换到该播放器
                            let mut current = self.current_player.lock().unwrap();

                            // 如果当前没有活跃的播放器，或者当前活跃播放器不是正在播放的播放器，则切换
                            if current.is_none() || current.as_ref().unwrap() != &player_name {
                                *current = Some(player_name.clone());
                                info!("播放器开始播放，切换到播放器: {}", player_name);

                                // 发送活跃播放器变更事件
                                self.notify_active_player_changed(&player_name);
                            }
                        }
                        PlaybackStatus::Paused | PlaybackStatus::Stopped => {
                            // 如果是当前活跃播放器被暂停或停止，尝试切换到其他播放中的播放器
                            let mut current = self.current_player.lock().unwrap();

                            if let Some(current_name) = current.as_ref() {
                                if current_name == &player_name {
                                    debug!("当前活跃播放器 {} 已暂停或停止，尝试切换到其他播放中的播放器", player_name);

                                    // 尝试找到一个正在播放的播放器
                                    if let Some(best_player) = self.select_best_player() {
                                        if best_player != player_name {
                                            *current = Some(best_player.clone());
                                            info!("切换到正在播放的播放器: {}", best_player);

                                            // 发送活跃播放器变更事件
                                            self.notify_active_player_changed(&best_player);
                                        }
                                    }
                                }
                            }
                        }
                    }

                    // 输出当前活跃播放器状态（仅调试用）
                    if let Some(current) = self.current_player.lock().unwrap().as_ref() {
                        debug!("当前活跃播放器: {}", current);
                    }
                }
                PlayerEvent::PlayerAppeared { player_name } => {
                    info!("播放器出现: {}", player_name);

                    // 将新播放器的状态设置为Stopped（默认值，稍后会通过PlaybackStatusChanged更新）
                    {
                        let mut player_status = self.player_status.lock().unwrap();
                        player_status.insert(player_name.clone(), PlaybackStatus::Stopped);
                    }

                    // 如果是第一个出现的播放器，设为当前活跃播放器
                    let mut current = self.current_player.lock().unwrap();
                    if current.is_none() {
                        *current = Some(player_name.clone());
                        info!("设置当前活跃播放器: {}", player_name);

                        // 发送活跃播放器变更事件
                        self.notify_active_player_changed(&player_name);
                    } else {
                        // 如果已经有活跃播放器，检查新播放器是否应该成为活跃播放器
                        // 这里不急于切换，等待收到播放状态变更事件后再决定
                        debug!("已有活跃播放器，等待播放状态变更事件后决定是否切换");
                    }
                }
                PlayerEvent::PlayerDisappeared { player_name } => {
                    info!("播放器消失: {}", player_name);

                    // 从播放器状态映射中移除
                    {
                        let mut player_status = self.player_status.lock().unwrap();
                        player_status.remove(&player_name);
                    }

                    // 如果是当前活跃播放器，需要切换到另一个播放器
                    let mut current = self.current_player.lock().unwrap();
                    if let Some(current_name) = current.as_ref() {
                        if current_name == &player_name {
                            // 清除当前播放器
                            *current = None;

                            // 优先选择状态为Playing的播放器
                            if let Some(best_player) = self.select_best_player() {
                                *current = Some(best_player.clone());
                                info!("切换到新的活跃播放器: {}", best_player);

                                // 发送活跃播放器变更事件
                                self.notify_active_player_changed(&best_player);
                            } else {
                                // 如果没有找到最佳播放器，从剩余播放器中选择一个作为当前活跃播放器
                                let track_info = self.current_track.lock().unwrap();
                                if !track_info.is_empty() {
                                    for (name, _) in track_info.iter() {
                                        if name != &player_name {
                                            *current = Some(name.clone());
                                            info!("切换到新的活跃播放器: {}", name);

                                            // 发送活跃播放器变更事件
                                            self.notify_active_player_changed(name);
                                            break;
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                PlayerEvent::PositionChanged {
                    player_name,
                    position_ms,
                } => {
                    // 只处理当前活跃播放器的位置变更
                    if let Some(current) = self.current_player.lock().unwrap().as_ref() {
                        if current == &player_name {
                            // 这里可以添加位置变更的处理逻辑
                            debug!("播放位置变更: {} ms", position_ms);
                        }
                    }
                }
                // 忽略ActivePlayerChanged事件，因为这是由我们自己发出的
                PlayerEvent::ActivePlayerChanged { .. } => {}
            }
        }

        Ok(())
    }

    /// 根据播放状态选择最佳播放器
    fn select_best_player(&self) -> Option<String> {
        let player_status = self.player_status.lock().unwrap();
        let track_info = self.current_track.lock().unwrap();

        // 优先选择状态为Playing的播放器
        for (name, status) in player_status.iter() {
            if *status == PlaybackStatus::Playing && track_info.contains_key(name) {
                debug!("找到正在播放的播放器: {}", name);
                return Some(name.clone());
            }
        }

        // 如果没有Playing状态的播放器，选择Paused状态的播放器
        for (name, status) in player_status.iter() {
            if *status == PlaybackStatus::Paused && track_info.contains_key(name) {
                debug!("找到暂停中的播放器: {}", name);
                return Some(name.clone());
            }
        }

        // 如果既没有Playing也没有Paused状态的播放器，选择任意一个有轨道信息的播放器
        if !track_info.is_empty() {
            let first_player = track_info.keys().next().unwrap().clone();
            debug!(
                "没有Playing或Paused状态的播放器，选择第一个有轨道信息的播放器: {}",
                first_player
            );
            return Some(first_player);
        }

        // 如果没有任何播放器有轨道信息，返回None
        None
    }

    /// 处理轨道变更事件
    async fn handle_track_changed(&self, player_name: String, track_info: TrackInfo) -> Result<()> {
        info!(
            "处理轨道变更: 播放器={}, 曲目={}, 艺术家={}",
            player_name, track_info.title, track_info.artist
        );

        // 检查是否是重复的轨道变更事件（相同的播放器+相同的曲目）
        let is_duplicate = {
            let tracks = self.current_track.lock().unwrap();
            if let Some(current_track) = tracks.get(&player_name) {
                // 更严格的重复检查：必须标题和艺术家都相同才认为是重复
                // 避免只检查ID导致的问题（特别是强制更新时ID可能不变）
                current_track.title == track_info.title && current_track.artist == track_info.artist
            } else {
                false
            }
        };

        if is_duplicate {
            debug!(
                "忽略重复的轨道变更事件: 播放器={}, 曲目={}",
                player_name, track_info.title
            );
            return Ok(());
        }

        // 更新当前轨道信息
        {
            let mut tracks = self.current_track.lock().unwrap();
            tracks.insert(player_name.clone(), track_info.clone());
        }

        // 如果是当前活跃播放器或没有活跃播放器，则设为活跃
        let was_player_changed = {
            let mut current = self.current_player.lock().unwrap();
            if current.is_none() || current.as_ref().unwrap() == &player_name {
                // 这里不需要更新播放器，因为它已经是当前播放器
                if current.is_none() {
                    *current = Some(player_name.clone());
                    info!("设置当前活跃播放器: {}", player_name);

                    // 通知活跃播放器变更
                    true
                } else {
                    false
                }
            } else {
                false
            }
        };

        // 如果活跃播放器发生变化，发送通知
        if was_player_changed {
            self.notify_active_player_changed(&player_name);
        }

        // 无论之前是否获取过歌词，每次轨道变更都重新获取
        // 这样可以确保当Spotify切换歌曲时，新的歌词能被获取
        {
            // 从歌词缓存中移除旧歌词
            let mut lyrics = self.current_lyrics.lock().unwrap();
            lyrics.remove(&player_name);
        }

        // 触发异步歌词查找
        let providers = self.providers.clone();
        let track = track_info.clone();
        let current_lyrics = self.current_lyrics.clone();
        let player = player_name.clone();

        tokio::spawn(async move {
            info!("开始为新轨道查找歌词: {} - {}", track.title, track.artist);
            match Self::fetch_lyrics_from_providers(&providers, &track).await {
                Ok(Some(lyrics)) => {
                    info!(
                        "已找到歌词: {} - {} (来源: {})",
                        track.title, track.artist, lyrics.metadata.source
                    );

                    // 更新歌词缓存
                    let mut lyrics_cache = current_lyrics.lock().unwrap();
                    lyrics_cache.insert(player, lyrics);
                }
                Ok(None) => {
                    warn!("未找到歌词: {} - {}", track.title, track.artist);
                }
                Err(e) => {
                    error!("搜索歌词时出错: {}", e);
                }
            }
        });

        Ok(())
    }

    /// 从提供者列表中查找歌词
    async fn fetch_lyrics_from_providers(
        providers: &[Arc<dyn LyricsProvider>],
        track: &TrackInfo,
    ) -> Result<Option<Lyrics>> {
        info!(
            "开始从 {} 个提供者查找歌词: {} - {}",
            providers.len(),
            track.title,
            track.artist
        );

        if track.title.trim().is_empty() {
            warn!("歌曲标题为空，无法查找歌词");
            return Ok(None);
        }

        // 遍历所有歌词提供者
        for provider in providers {
            let provider_name = provider.name();
            info!("尝试从提供者 '{}' 查找歌词", provider_name);

            match provider.search_lyrics(track) {
                Ok(Some(lyrics)) => {
                    if lyrics.lines.is_empty() {
                        warn!(
                            "提供者 '{}' 返回了空歌词（0行）- 这不应该发生，提供者应该返回None",
                            provider_name
                        );
                        info!("继续尝试其他提供者");
                        continue;
                    }

                    info!(
                        "从提供者 '{}' 成功找到歌词, 共 {} 行",
                        provider_name,
                        lyrics.lines.len()
                    );
                    return Ok(Some(lyrics));
                }
                Ok(None) => {
                    info!("提供者 '{}' 未找到匹配歌词", provider_name);
                }
                Err(e) => {
                    error!("提供者 '{}' 搜索歌词出错: {}", provider_name, e);
                }
            }
        }

        info!("所有提供者均未找到歌词: {} - {}", track.title, track.artist);
        Ok(None)
    }

    /// 获取当前播放器的歌词
    pub fn get_current_lyrics(&self) -> Option<Lyrics> {
        let current_player = self.current_player.lock().unwrap();
        if let Some(player) = current_player.as_ref() {
            let lyrics = self.current_lyrics.lock().unwrap();
            lyrics.get(player).cloned()
        } else {
            None
        }
    }

    /// 获取指定时间的歌词行
    pub fn get_lyric_at_time(&self, time_ms: u64) -> Option<LyricLine> {
        if let Some(lyrics) = self.get_current_lyrics() {
            // 二分查找最近的歌词行
            let mut result = None;

            for (i, line) in lyrics.lines.iter().enumerate() {
                if line.start_time <= time_ms {
                    if i + 1 < lyrics.lines.len() {
                        let next_line = &lyrics.lines[i + 1];
                        if next_line.start_time > time_ms {
                            result = Some(line.clone());
                            break;
                        }
                    } else {
                        // 最后一行
                        result = Some(line.clone());
                        break;
                    }
                }
            }

            result
        } else {
            None
        }
    }

    /// 通知外部模块活跃播放器已变更
    fn notify_active_player_changed(&self, player_name: &str) {
        if let Some(sender) = &self.event_sender {
            let player_name = player_name.to_string();
            let sender = sender.clone();

            // 获取新活跃播放器的状态
            let status = self
                .player_status
                .lock()
                .unwrap()
                .get(&player_name)
                .cloned()
                .unwrap_or(PlaybackStatus::Stopped); // 如果找不到状态，默认为 Stopped

            tokio::spawn(async move {
                debug!(
                    "LyricsManager 发送 ActivePlayerChanged 事件: {}, 状态: {:?}",
                    player_name, status
                );
                if let Err(e) = sender
                    .send(PlayerEvent::ActivePlayerChanged {
                        player_name,
                        status, // 包含状态
                    })
                    .await
                {
                    error!("发送 ActivePlayerChanged 事件失败: {}", e);
                }
            });
        }
    }

    /// 获取指定播放器的轨道信息
    pub fn get_track_info(&self, player_name: &str) -> Option<TrackInfo> {
        let tracks = self.current_track.lock().unwrap();
        tracks.get(player_name).cloned()
    }

    /// 获取指定播放器的播放状态
    pub fn get_player_status(&self, player_name: &str) -> Option<PlaybackStatus> {
        let status = self.player_status.lock().unwrap();
        status.get(player_name).cloned()
    }
}

/// 初始化歌词管理器
pub fn setup_lyrics_manager(config: Arc<Config>) -> LyricsManager {
    let providers = get_enabled_providers(&config);
    LyricsManager::new(providers)
}
