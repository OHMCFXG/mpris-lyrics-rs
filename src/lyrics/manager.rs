use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use anyhow::Result;
use log::{debug, error, info, warn};
use tokio::sync::mpsc::{Receiver, Sender};

use crate::lyrics::{LyricLine, Lyrics, LyricsProvider};
use crate::mpris::{PlayerEvent, TrackInfo};

/// 歌词管理器
/// 负责获取和管理歌词
#[derive(Clone)]
pub struct LyricsManager {
    providers: Arc<Vec<Arc<dyn LyricsProvider>>>,
    current_lyrics: Arc<RwLock<HashMap<String, Lyrics>>>,
    current_track: Arc<RwLock<HashMap<String, TrackInfo>>>,
    active_player: Arc<RwLock<Option<String>>>,
    event_sender: Option<Sender<PlayerEvent>>,
}

impl LyricsManager {
    /// 创建新的歌词管理器
    pub fn new(providers: Vec<Arc<dyn LyricsProvider>>) -> Self {
        Self {
            providers: Arc::new(providers),
            current_lyrics: Arc::new(RwLock::new(HashMap::new())),
            current_track: Arc::new(RwLock::new(HashMap::new())),
            active_player: Arc::new(RwLock::new(None)),
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
                PlayerEvent::ActivePlayerChanged {
                    player_name,
                    status: _,
                } => {
                    debug!("活跃播放器变更: {}", player_name);
                    let mut active = self.active_player.write().unwrap();
                    *active = Some(player_name.clone());
                }
                PlayerEvent::PlayerDisappeared { player_name } => {
                    // 清理数据
                    let mut lyrics = self.current_lyrics.write().unwrap();
                    lyrics.remove(&player_name);
                    
                    let mut tracks = self.current_track.write().unwrap();
                    tracks.remove(&player_name);
                    
                    let mut active = self.active_player.write().unwrap();
                    if active.as_ref() == Some(&player_name) {
                        *active = None;
                    }
                }
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
            let mut current_track = self.current_track.write().unwrap();
            current_track.insert(player_name.clone(), track_info.clone());
        }

        // 如果轨道信息不全，跳过获取歌词
        if track_info.title.is_empty() {
            debug!("轨道标题为空，跳过获取歌词");
            return Ok(());
        }

        // 2. 清除之前的歌词
        {
            let mut current_lyrics = self.current_lyrics.write().unwrap();
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
                    let mut current_lyrics = self.current_lyrics.write().unwrap();
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
        let active_player = self.active_player.read().unwrap();

        if let Some(player_name) = &*active_player {
            let current_lyrics = self.current_lyrics.read().unwrap();
            current_lyrics.get(player_name).cloned()
        } else {
            None
        }
    }

    /// 根据时间获取当前歌词行
    /// 优化：使用二分查找
    pub fn get_lyric_at_time(&self, time_ms: u64) -> Option<LyricLine> {
        if let Some(lyrics) = self.get_current_lyrics() {
            if lyrics.lines.is_empty() {
                return None;
            }
            
            // binary_search_by_key 找第一个 start_time > time_ms 的位置
            let idx = lyrics.lines.partition_point(|line| line.start_time <= time_ms);
            
            if idx == 0 {
                // 时间在第一行之前
                return Some(lyrics.lines[0].clone());
            }
            
            // idx 是第一个大于 time_ms 的元素的索引
            // 所以 idx - 1 是最后一个小于等于 time_ms 的元素
            let line = &lyrics.lines[idx - 1];
            
            return Some(line.clone());
        }

        None
    }

    /// 获取指定播放器的轨道信息
    pub fn get_track_info(&self, player_name: &str) -> Option<TrackInfo> {
        let current_track = self.current_track.read().unwrap();
        current_track.get(player_name).cloned()
    }
}
