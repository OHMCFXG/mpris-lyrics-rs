use std::collections::HashSet;
use std::env;
use std::fs;
use std::path::PathBuf;

use anyhow::Result;
use dirs;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub display: DisplayConfig,
    pub mpris: MprisConfig,
    pub sources: SourcesConfig,
    pub players: PlayersConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DisplayConfig {
    pub show_timestamp: bool,
    pub show_progress: bool,
    pub context_lines: usize,
    pub current_line_color: String,
    pub simple_output: bool,
    pub enable_tui: bool,
    pub lyric_advance_time_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MprisConfig {
    pub fallback_sync_interval_seconds: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourcesConfig {
    pub netease: Option<NeteaseConfig>,
    pub qqmusic: Option<QQMusicConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NeteaseConfig {}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QQMusicConfig {}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlayersConfig {
    pub blacklist: HashSet<String>,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            display: DisplayConfig {
                show_timestamp: false,
                show_progress: true,
                context_lines: 2,
                current_line_color: "green".to_string(),
                simple_output: false,
                enable_tui: true,
                lyric_advance_time_ms: 300,
            },
            mpris: MprisConfig {
                fallback_sync_interval_seconds: 5,
            },
            sources: SourcesConfig {
                netease: Some(NeteaseConfig {}),
                qqmusic: Some(QQMusicConfig {}),
            },
            players: PlayersConfig {
                blacklist: ["firefox", "mozilla", "chromium", "chrome", "kdeconnect"]
                    .iter()
                    .map(|s| s.to_string())
                    .collect(),
            },
        }
    }
}

impl Config {
    pub fn load(path: Option<PathBuf>) -> Result<Self> {
        let pkg_name = env!("CARGO_PKG_NAME");
        let config_path = path.unwrap_or_else(|| {
            dirs::config_dir()
                .map(|p| p.join(pkg_name).join("config.toml"))
                .unwrap_or_else(|| PathBuf::from(format!("{}-config.toml", pkg_name)))
        });

        if !config_path.exists() {
            let default_config = Config::default();
            let toml = toml::to_string_pretty(&default_config)?;

            if let Some(parent) = config_path.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(&config_path, toml)?;
            return Ok(default_config);
        }

        let content = fs::read_to_string(&config_path)?;
        let cfg: Config = toml::from_str(&content)?;
        Ok(cfg)
    }
}
