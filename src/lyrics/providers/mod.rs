use std::sync::Arc;

use crate::config::Config;
use crate::lyrics::LyricsProvider;
use strsim::jaro_winkler;

pub mod netease;
pub mod qqmusic;

pub fn get_enabled_providers(config: &Config) -> Vec<Arc<dyn LyricsProvider>> {
    let mut providers: Vec<Arc<dyn LyricsProvider>> = Vec::new();

    if let Some(cfg) = config.sources.netease.clone() {
        providers.push(Arc::new(netease::NeteaseProvider::new(cfg)));
    }

    if let Some(cfg) = config.sources.qqmusic.clone() {
        providers.push(Arc::new(qqmusic::QQMusicProvider::new(cfg)));
    }

    providers
}

pub fn similarity(a: &str, b: &str) -> f64 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let a = a.to_lowercase();
    let b = b.to_lowercase();
    jaro_winkler(&a, &b)
}
