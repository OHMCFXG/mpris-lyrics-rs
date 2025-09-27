use anyhow::Result;
use dirs;
use log::{debug, info};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::env;
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Config {
    /// 启用的歌词源列表
    pub lyrics_sources: Vec<String>,

    /// 播放器黑名单（基于关键字）
    pub player_blacklist: HashSet<String>,

    /// 歌词显示设置
    pub display: DisplayConfig,

    /// MPRIS相关设置
    pub mpris: MprisSettings,

    /// 歌词源特定配置
    pub sources: SourcesConfig,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DisplayConfig {
    /// 是否显示时间戳
    pub show_timestamp: bool,

    /// 是否显示当前播放进度
    pub show_progress: bool,

    /// 歌词前后显示的行数
    pub context_lines: usize,

    /// 当前行的颜色 (ANSI color code)
    pub current_line_color: String,

    /// 是否启用简单输出模式（适用于waybar等外部集成）
    pub simple_output: bool,

    /// 是否启用 TUI 界面（默认启用，简单输出模式时自动禁用）
    pub enable_tui: bool,

    /// 歌词提前显示时间（毫秒）
    pub lyric_advance_time: u64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SourcesConfig {
    /// 网易云音乐API配置
    pub netease: Option<NeteaseConfig>,

    /// QQ音乐API配置
    pub qqmusic: Option<QQMusicConfig>,

    /// 本地歌词文件配置
    pub local: Option<LocalConfig>,
}

/// 网易云音乐配置
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct NeteaseConfig {}

/// QQ音乐配置
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct QQMusicConfig {}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct LocalConfig {
    /// 本地歌词目录路径
    pub lyrics_path: String,
}

/// MPRIS设置
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct MprisSettings {
    /// 播放位置同步间隔（秒）
    pub sync_interval_seconds: u64,
}

impl Default for Config {
    fn default() -> Self {
        let pkg_name = env!("CARGO_PKG_NAME");
        let default_lyrics_path = dirs::config_dir()
            .map(|p| p.join(pkg_name).join("lyrics"))
            .unwrap_or_else(|| PathBuf::from("lyrics"));

        Config {
            lyrics_sources: vec!["netease".to_string(), "qq".to_string(), "local".to_string()],
            player_blacklist: ["firefox", "mozilla", "chromium", "chrome", "kdeconnect"]
                .iter()
                .map(|s| s.to_string())
                .collect(),
            display: DisplayConfig {
                show_timestamp: false,
                show_progress: true,
                context_lines: 2,
                current_line_color: "green".to_string(),
                simple_output: false,
                enable_tui: true,
                lyric_advance_time: 300,
            },
            mpris: MprisSettings {
                sync_interval_seconds: 1,
            },
            sources: SourcesConfig {
                netease: Some(NeteaseConfig {}),
                qqmusic: Some(QQMusicConfig {}),
                local: Some(LocalConfig {
                    lyrics_path: default_lyrics_path.to_string_lossy().to_string(),
                }),
            },
        }
    }
}

impl Config {
    /// 加载配置，支持从指定路径或默认路径加载
    pub fn load(path: Option<std::path::PathBuf>) -> Result<Self> {
        let pkg_name = env!("CARGO_PKG_NAME");
        let config_path = path.unwrap_or_else(|| {
            dirs::config_dir()
                .map(|p| p.join(pkg_name).join("config.toml"))
                .unwrap_or_else(|| PathBuf::from(format!("{}-config.toml", pkg_name)))
        });

        debug!("尝试从 {:?} 加载配置文件", config_path);

        if !config_path.exists() {
            debug!("配置文件 {:?} 不存在，将创建默认配置", config_path);
            let default_config = Config::default();
            let toml = toml::to_string_pretty(&default_config)?;

            if let Some(parent) = config_path.parent() {
                fs::create_dir_all(parent)?;
                debug!("已确保目录 {:?} 存在", parent);
            }

            fs::write(&config_path, toml)?;
            info!("已创建默认配置文件: {:?}", config_path);
            return Ok(default_config);
        }

        let content = fs::read_to_string(&config_path)?;
        let config: Config = match toml::from_str(&content) {
            Ok(cfg) => cfg,
            Err(e) => {
                log::error!("解析配置文件 {:?} 失败: {}", config_path, e);
                log::warn!("由于解析错误，将加载默认配置");
                Config::default()
            }
        };

        debug!("已成功加载配置文件");
        Ok(config)
    }
}
