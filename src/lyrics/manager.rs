use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use anyhow::Result;
use log::{debug, error, info, warn};
use tokio::sync::mpsc::{Receiver, Sender};

use crate::lyrics::{LyricLine, Lyrics, LyricsProvider};
use crate::mpris::{PlaybackStatus, PlayerEvent, TrackInfo};

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
                            // 任何播放器暂停或停止时，都重新评估最佳活跃播放器
                            debug!("播放器 {} 已暂停或停止，重新评估活跃播放器", player_name);

                            let mut current = self.current_player.lock().unwrap();
                            let best_player_option = self.select_best_player();

                            match best_player_option {
                                Some(best_player) => {
                                    // 如果选出的最佳播放器与当前不同，或者之前没有，则切换
                                    if current.is_none()
                                        || current.as_ref().unwrap() != &best_player
                                    {
                                        info!("检测到状态变更，切换到最优播放器: {}", best_player);
                                        *current = Some(best_player.clone());
                                        self.notify_active_player_changed(&best_player);
                                    } else {
                                        // 否则保持不变
                                        debug!("最佳播放器({})未改变，无需切换", best_player);
                                    }
                                }
                                None => {
                                    // 没有找到合适的播放器（例如所有播放器都停止了）
                                    if current.is_some() {
                                        info!("没有可用的活跃播放器，清除当前状态");
                                        *current = None;
                                        // 注意：这里可能需要通知DisplayManager清屏或显示无播放器状态
                                        // 目前仅清除内部状态
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

                    // 不再立即设置活跃播放器，等待播放状态事件
                    debug!("等待播放状态事件后再决定活跃播放器");
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
                            }
                        }
                    }

                    // 移除此播放器的曲目信息
                    {
                        let mut current_track = self.current_track.lock().unwrap();
                        current_track.remove(&player_name);
                    }

                    // 移除此播放器的歌词
                    {
                        let mut current_lyrics = self.current_lyrics.lock().unwrap();
                        current_lyrics.remove(&player_name);
                    }
                }
                PlayerEvent::ActivePlayerChanged {
                    player_name,
                    status: _,
                } => {
                    // 外部通知活跃播放器变更
                    debug!("收到活跃播放器变更通知: {}", player_name);
                    let mut current = self.current_player.lock().unwrap();
                    *current = Some(player_name.clone());

                    // 主动获取当前播放器的轨道信息
                    if let Some(track_info) = self.get_track_info(&player_name) {
                        debug!(
                            "获取到活跃播放器曲目信息: {} - {}",
                            track_info.title, track_info.artist
                        );
                        // 触发轨道变更事件处理
                        let track_info_clone = track_info.clone();
                        let player_name_clone = player_name.clone();
                        let self_clone = self.clone();
                        tokio::spawn(async move {
                            if let Err(e) = self_clone
                                .handle_track_changed(player_name_clone, track_info_clone)
                                .await
                            {
                                error!("处理轨道变更事件失败: {}", e);
                            }
                        });
                    } else {
                        debug!("未获取到活跃播放器曲目信息");
                    }
                }
                // 对于 PositionChanged 事件，歌词管理器不需要处理
                _ => {}
            }
        }

        debug!("歌词管理器收到终止信号");
        Ok(())
    }

    /// 选择最佳播放器作为当前活跃播放器
    fn select_best_player(&self) -> Option<String> {
        let player_status = self.player_status.lock().unwrap();

        // 首先找出所有正在播放的播放器
        let playing_players: Vec<String> = player_status
            .iter()
            .filter_map(|(player, status)| {
                if *status == PlaybackStatus::Playing {
                    Some(player.clone())
                } else {
                    None
                }
            })
            .collect();

        if !playing_players.is_empty() {
            // 如果有正在播放的播放器，选择第一个
            return Some(playing_players[0].clone());
        }

        // 如果没有正在播放的播放器，找出所有暂停的播放器
        let paused_players: Vec<String> = player_status
            .iter()
            .filter_map(|(player, status)| {
                if *status == PlaybackStatus::Paused {
                    Some(player.clone())
                } else {
                    None
                }
            })
            .collect();

        if !paused_players.is_empty() {
            // 如果有暂停的播放器，选择第一个
            return Some(paused_players[0].clone());
        }

        // 如果既没有播放也没有暂停的播放器，选择第一个可用的播放器
        player_status.keys().next().cloned()
    }

    /// 处理轨道变更事件
    async fn handle_track_changed(&self, player_name: String, track_info: TrackInfo) -> Result<()> {
        // 1. 保存轨道信息到当前轨道映射
        {
            let mut current_track = self.current_track.lock().unwrap();
            current_track.insert(player_name.clone(), track_info.clone());
        }

        // 如果轨道信息不全，跳过获取歌词
        if track_info.title.is_empty() {
            debug!("轨道标题为空，跳过获取歌词");
            return Ok(());
        }

        // 2. 清除之前的歌词
        {
            let mut current_lyrics = self.current_lyrics.lock().unwrap();
            current_lyrics.remove(&player_name);
        }

        // 3. 尝试获取新歌词
        debug!(
            "尝试为 {} 获取歌词: {} - {}",
            player_name, track_info.title, track_info.artist
        );

        // 从配置的提供者按优先级依次尝试获取歌词
        match self.fetch_lyrics_from_providers(&track_info).await {
            Ok(Some(lyrics)) => {
                info!(
                    "成功获取歌词: {} - {}, 来源: {}, 共{}行",
                    track_info.title,
                    track_info.artist,
                    lyrics.metadata.source,
                    lyrics.lines.len()
                );

                // 保存歌词
                {
                    let mut current_lyrics = self.current_lyrics.lock().unwrap();
                    current_lyrics.insert(player_name, lyrics);
                }
            }
            Ok(None) => {
                info!("未找到歌词: {} - {}", track_info.title, track_info.artist);
            }
            Err(e) => {
                error!(
                    "获取歌词失败: {} - {}, 错误: {}",
                    track_info.title, track_info.artist, e
                );
            }
        }

        Ok(())
    }

    /// 从所有提供者获取歌词
    async fn fetch_lyrics_from_providers(&self, track: &TrackInfo) -> Result<Option<Lyrics>> {
        let providers = &*self.providers;

        for provider in providers.iter() {
            debug!("尝试从 {} 获取歌词", provider.name());
            match provider.search_lyrics(track) {
                Ok(Some(lyrics)) => {
                    // 找到歌词，立即返回
                    return Ok(Some(lyrics));
                }
                Ok(None) => {
                    debug!("{} 未找到歌词，尝试下一个提供者", provider.name());
                    continue;
                }
                Err(e) => {
                    warn!("{} 获取歌词失败: {}", provider.name(), e);
                    continue; // 继续尝试下一个提供者
                }
            }
        }

        debug!("所有提供者均未找到歌词");
        Ok(None)
    }

    /// 获取当前歌词
    pub fn get_current_lyrics(&self) -> Option<Lyrics> {
        let current_player = self.current_player.lock().unwrap();

        if let Some(player_name) = &*current_player {
            let current_lyrics = self.current_lyrics.lock().unwrap();
            current_lyrics.get(player_name).cloned()
        } else {
            None
        }
    }

    /// 根据时间获取当前歌词行
    pub fn get_lyric_at_time(&self, time_ms: u64) -> Option<LyricLine> {
        if let Some(lyrics) = self.get_current_lyrics() {
            if lyrics.lines.is_empty() {
                return None;
            }

            debug!(
                "查找时间点 {}ms 的歌词行，共有{}行歌词",
                time_ms,
                lyrics.lines.len()
            );

            // 先尝试找到正好适合这个时间的行
            for (i, line) in lyrics.lines.iter().enumerate() {
                if line.start_time <= time_ms
                    && (line.end_time.is_none() || line.end_time.unwrap() > time_ms)
                {
                    debug!(
                        "找到匹配行 #{}: 开始={}ms, 结束={:?}ms, 文本={}",
                        i, line.start_time, line.end_time, line.text
                    );
                    return Some(line.clone());
                }
            }

            // 如果没有适合的行，返回最后一行
            if time_ms >= lyrics.lines.last().unwrap().start_time {
                let last_line = lyrics.lines.last().unwrap();
                debug!(
                    "超过最后一行时间，返回最后一行: 开始={}ms, 文本={}",
                    last_line.start_time, last_line.text
                );
                return Some(last_line.clone());
            }

            // 如果时间在第一行开始之前，返回第一行
            let first_line = lyrics.lines.first().unwrap();
            debug!(
                "时间在第一行之前，返回第一行: 开始={}ms, 文本={}",
                first_line.start_time, first_line.text
            );
            return Some(first_line.clone());
        }

        debug!("未找到当前歌词");
        None
    }

    /// 通知活跃播放器变更
    fn notify_active_player_changed(&self, player_name: &str) {
        if let Some(sender) = &self.event_sender {
            // 获取播放器状态
            let status = {
                let player_status = self.player_status.lock().unwrap();
                player_status
                    .get(player_name)
                    .cloned()
                    .unwrap_or(PlaybackStatus::Stopped)
            };

            // 创建事件
            let event = PlayerEvent::ActivePlayerChanged {
                player_name: player_name.to_string(),
                status,
            };

            // 发送事件
            let sender = sender.clone();
            tokio::spawn(async move {
                if let Err(e) = sender.send(event).await {
                    error!("发送活跃播放器变更事件失败: {}", e);
                }
            });
        }
    }

    /// 获取指定播放器的轨道信息
    pub fn get_track_info(&self, player_name: &str) -> Option<TrackInfo> {
        let current_track = self.current_track.lock().unwrap();
        current_track.get(player_name).cloned()
    }

    /// 获取指定播放器的播放状态
    pub fn get_player_status(&self, player_name: &str) -> Option<PlaybackStatus> {
        let player_status = self.player_status.lock().unwrap();
        player_status.get(player_name).cloned()
    }
}
