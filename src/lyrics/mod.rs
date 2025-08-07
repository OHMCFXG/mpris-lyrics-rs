mod manager;
pub mod providers;

use std::collections::HashMap;
use std::sync::Arc;

use crate::config::Config;
use crate::mpris::TrackInfo;
use anyhow::Result;
use async_trait::async_trait;

pub use manager::LyricsManager;

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
#[async_trait]
pub trait LyricsProvider: Send + Sync {
    /// 获取提供者名称
    fn name(&self) -> &str;

    /// 搜索歌词
    async fn search_lyrics(&self, track: &TrackInfo) -> Result<Option<Lyrics>>;
}

/// 设置歌词管理器
pub fn setup_lyrics_manager(config: Arc<Config>) -> LyricsManager {
    let providers = providers::get_enabled_providers(&config);
    LyricsManager::new(providers)
}
