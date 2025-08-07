use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use anyhow::Result;
use log::{debug, error, info, warn};
use tokio::sync::broadcast::Receiver;

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
        }
    }

    /// 启动歌词管理器处理循环
    pub async fn run(&self, mut player_events: Receiver<PlayerEvent>) -> Result<()> {
        info!("歌词管理器启动");

        loop {
            let event = match player_events.recv().await {
                Ok(event) => event,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    warn!("歌词管理器事件通道落后 {} 条消息", n);
                    continue;
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                    break;
                }
            };

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
                    let mut player_status = self.player_status.lock().unwrap();
                    player_status.insert(player_name.clone(), status.clone());
                }
                PlayerEvent::PlayerAppeared { player_name } => {
                    info!("播放器出现: {}", player_name);
                    let mut player_status = self.player_status.lock().unwrap();
                    player_status.insert(player_name.clone(), PlaybackStatus::Stopped);
                }
                PlayerEvent::PlayerDisappeared { player_name } => {
                    info!("播放器消失: {}", player_name);
                    self.player_status.lock().unwrap().remove(&player_name);
                    self.current_track.lock().unwrap().remove(&player_name);
                    self.current_lyrics.lock().unwrap().remove(&player_name);

                    let mut current = self.current_player.lock().unwrap();
                    if let Some(current_name) = current.as_ref() {
                        if current_name == &player_name {
                            *current = None;
                            info!("活跃播放器已消失，清除当前状态");
                        }
                    }
                }
                PlayerEvent::ActivePlayerChanged {
                    player_name,
                    status: _,
                } => {
                    debug!("收到活跃播放器变更通知: {}", player_name);
                    let mut current = self.current_player.lock().unwrap();
                    *current = Some(player_name.clone());

                    if let Some(track_info) = self.get_track_info(&player_name) {
                        debug!(
                            "获取到活跃播放器曲目信息: {} - {}",
                            track_info.title, track_info.artist
                        );
                        let self_clone = self.clone();
                        tokio::spawn(async move {
                            if let Err(e) = self_clone
                                .handle_track_changed(player_name, track_info)
                                .await
                            {
                                error!("处理轨道变更事件失败: {}", e);
                            }
                        });
                    }
                }
                PlayerEvent::NoPlayersAvailable => {
                    debug!("收到无可用播放器事件");
                    let mut current = self.current_player.lock().unwrap();
                    if current.is_some() {
                        *current = None;
                        info!("无可用播放器，清除当前状态");
                    }
                }
                // 对于 PositionChanged 事件，歌词管理器不需要处理
                _ => {}
            }
        }

        debug!("歌词管理器收到终止信号");
        Ok(())
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
            // 清除之前的歌词
            self.current_lyrics.lock().unwrap().remove(&player_name);
            return Ok(());
        }

        // 2. 清除之前的歌词
        self.current_lyrics.lock().unwrap().remove(&player_name);

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
            match provider.search_lyrics(track).await {
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
