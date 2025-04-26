mod local;
mod netease;
mod qqmusic;

use std::sync::Arc;

use crate::config::Config;
use crate::lyrics::LyricsProvider;
use log::{debug, info, warn};

pub use local::LocalProvider;
pub use netease::NeteaseProvider;
pub use qqmusic::QQMusicProvider;

/// 获取所有启用的歌词提供者
pub fn get_enabled_providers(config: &Arc<Config>) -> Vec<Arc<dyn LyricsProvider>> {
    let mut providers: Vec<Arc<dyn LyricsProvider>> = Vec::new();

    debug!(
        "加载启用的歌词提供者，配置的源: {:?}",
        config.lyrics_sources
    );

    // 根据配置文件中启用的提供者进行创建
    for source in &config.lyrics_sources {
        match source.as_str() {
            "netease" => {
                if let Some(netease_config) = &config.sources.netease {
                    info!("启用网易云音乐歌词源");
                    providers.push(Arc::new(NeteaseProvider::new(netease_config.clone()))
                        as Arc<dyn LyricsProvider>);
                } else {
                    warn!("已启用网易云音乐歌词源，但未找到相关配置");
                }
            }
            "qqmusic" | "qq" => {
                if let Some(qqmusic_config) = &config.sources.qqmusic {
                    info!("启用QQ音乐歌词源");
                    providers.push(Arc::new(QQMusicProvider::new(qqmusic_config.clone()))
                        as Arc<dyn LyricsProvider>);
                } else {
                    warn!("已启用QQ音乐歌词源，但未找到相关配置");
                }
            }
            "local" => {
                if let Some(local_config) = &config.sources.local {
                    info!("启用本地歌词源，歌词目录: {}", local_config.lyrics_path);
                    providers.push(Arc::new(LocalProvider::new(local_config.clone()))
                        as Arc<dyn LyricsProvider>);
                } else {
                    warn!("已启用本地歌词源，但未找到相关配置");
                }
            }
            _ => {
                warn!("未知的歌词源: {}", source);
            }
        }
    }

    info!("成功加载 {} 个歌词提供者", providers.len());
    for (i, provider) in providers.iter().enumerate() {
        info!("歌词提供者 #{}: {}", i + 1, provider.name());
    }

    providers
}
