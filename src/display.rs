use std::io::{self, Write};
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use colored::Colorize;
use log::debug;
use tokio::sync::mpsc::Receiver;
use tokio::time;

use crate::config::Config;
use crate::lyrics::LyricsManager;
use crate::mpris::{PlaybackStatus, PlayerEvent, TrackInfo};

/// 显示管理器，负责在终端中显示歌词
#[derive(Clone)]
pub struct DisplayManager {
    /// 配置 (共享)
    config: Arc<Config>,
    /// 歌词管理器
    lyrics_manager: LyricsManager,
    /// 当前播放状态
    current_status: PlaybackStatus,
    /// 当前播放时间位置（毫秒）
    current_position: u64,
    /// 当前播放轨道
    current_track: Option<TrackInfo>,
    /// 当前播放器名称
    current_player: Option<String>,
    /// 上次更新时间
    last_update: u64,
    /// 上次输出的内容（用于避免简单模式下重复输出）
    last_output: String,
}

impl DisplayManager {
    /// 创建新的显示管理器
    pub fn new(config: Arc<Config>, lyrics_manager: LyricsManager) -> Self {
        Self {
            config,
            lyrics_manager,
            current_status: PlaybackStatus::Stopped,
            current_position: 0,
            current_track: None,
            current_player: None,
            last_update: 0,
            last_output: String::new(),
        }
    }

    /// 启动显示管理器
    pub async fn run(&mut self, mut player_events: Receiver<PlayerEvent>) -> Result<()> {
        // 设置定时刷新
        let mut refresh_interval = time::interval(Duration::from_millis(500));
        // 设置定期同步播放位置的定时器
        let mut position_sync_interval = time::interval(Duration::from_secs(5));

        // 主循环
        loop {
            tokio::select! {
                // 刷新显示
                _ = refresh_interval.tick() => {
                    if self.current_status == PlaybackStatus::Playing {
                        // 播放中时，更新位置估计
                        let now = chrono::Utc::now().timestamp_millis() as u64;
                        if self.last_update > 0 {
                            let elapsed = now - self.last_update;
                            self.current_position += elapsed;
                        }
                        self.last_update = now;

                        // 刷新显示
                        self.refresh_display()?;
                    }
                }

                // 定期同步位置（此处仅作为标记，实际同步需要在外部进行）
                _ = position_sync_interval.tick() => {
                    // 这里不做具体实现，因为DisplayManager无法直接获取播放位置
                    // 需要通过外部的MPRIS事件获取
                }

                // 处理播放器事件
                Some(event) = player_events.recv() => {
                    self.handle_player_event(event)?;
                }
            }
        }
    }

    /// 处理播放器事件
    fn handle_player_event(&mut self, event: PlayerEvent) -> Result<()> {
        match event {
            PlayerEvent::PlaybackStatusChanged {
                player_name,
                status,
            } => {
                // 只处理当前播放器的状态变化
                if self.is_current_player(&player_name) {
                    self.current_status = status;
                    if self.current_status == PlaybackStatus::Playing {
                        // 开始播放时记录当前时间
                        self.last_update = chrono::Utc::now().timestamp_millis() as u64;
                    } else {
                        // 暂停或停止时重置最后更新时间
                        self.last_update = 0;
                    }
                    self.refresh_display()?;
                }
            }

            PlayerEvent::TrackChanged {
                player_name,
                track_info,
            } => {
                // 只处理当前播放器的轨道变化
                if self.is_current_player(&player_name) {
                    // 保存曲目信息
                    self.current_track = Some(track_info);

                    // 在轨道变更时重置播放位置，避免显示旧歌词
                    self.current_position = 0;
                    self.last_update = 0;

                    // 刷新显示
                    self.refresh_display()?;
                }
            }

            PlayerEvent::PositionChanged {
                player_name,
                position_ms,
            } => {
                // 只处理当前播放器的位置变化
                if self.is_current_player(&player_name) {
                    self.current_position = position_ms;
                    self.last_update = chrono::Utc::now().timestamp_millis() as u64;
                    self.refresh_display()?;
                }
            }

            PlayerEvent::PlayerAppeared { player_name } => {
                // 如果没有当前播放器，设置此播放器为当前播放器
                if self.current_player.is_none() {
                    self.current_player = Some(player_name);
                    self.refresh_display()?;
                }
            }

            PlayerEvent::PlayerDisappeared { player_name } => {
                // 如果是当前播放器消失，清除当前状态
                if self.is_current_player(&player_name) {
                    self.current_player = None;
                    self.current_track = None;
                    self.current_position = 0;
                    self.current_status = PlaybackStatus::Stopped;
                    self.last_update = 0;
                    self.refresh_display()?;
                }
            }

            PlayerEvent::ActivePlayerChanged {
                player_name,
                status,
            } => {
                // 当活跃播放器变更时，更新当前播放器
                debug!(
                    "收到活跃播放器变更通知: {}, 状态: {:?}",
                    player_name, status
                );

                // 保存当前播放器旧的名称，用于判断是否发生变化
                let changed = {
                    if let Some(current) = &self.current_player {
                        current != &player_name
                    } else {
                        true
                    }
                };

                if changed {
                    // 更新当前播放器
                    self.current_player = Some(player_name.clone());

                    // 尝试从LyricsManager获取当前播放器的轨道信息
                    if let Some(track_info) = self.lyrics_manager.get_track_info(&player_name) {
                        self.current_track = Some(track_info);
                    } else {
                        // 如果找不到轨道信息，清除当前轨道
                        self.current_track = None;
                    }

                    // 重置播放位置和更新时间
                    self.current_position = 0;
                    self.last_update = 0;

                    // 直接使用事件传递过来的状态
                    self.current_status = status;

                    // 根据收到的状态设置 last_update
                    if self.current_status == PlaybackStatus::Playing {
                        self.last_update = chrono::Utc::now().timestamp_millis() as u64;
                    }

                    // 刷新显示
                    self.refresh_display()?;
                }
            }
        }

        Ok(())
    }

    /// 刷新显示
    fn refresh_display(&mut self) -> Result<()> {
        // 检查是否使用简单输出模式
        if self.config.display.simple_output {
            // 使用简单输出模式
            return self.refresh_display_simple();
        }

        // 正常输出模式
        // 根据配置决定是否清屏
        // 注：我们暂时使用show_progress字段作为清屏标志
        if self.config.display.show_progress {
            // 清屏
            print!("\x1B[2J\x1B[1;1H");
            io::stdout().flush()?;
        } else {
            // 不清屏，只打印分隔符
            println!("\n{}", "-".repeat(50));
            println!("当前歌词显示 (--no-clear 模式):");
            println!("{}", "-".repeat(50));
        }

        // 当没有播放器或轨道时显示空闲信息
        if self.current_player.is_none() || self.current_track.is_none() {
            println!("等待播放器...");
            return Ok(());
        }

        // 显示当前播放信息
        if let Some(track) = &self.current_track {
            self.display_track_info(track)?;
        }

        // 显示播放状态
        self.display_status()?;

        // 显示歌词
        self.display_lyrics()?;

        Ok(())
    }

    /// 简单输出模式的显示刷新
    fn refresh_display_simple(&mut self) -> Result<()> {
        // 获取播放状态（仅用于内部逻辑，不输出）
        let status = match self.current_status {
            PlaybackStatus::Playing => "playing",
            PlaybackStatus::Paused => "paused",
            PlaybackStatus::Stopped => "stopped",
        };

        // 没有播放器或曲目时，输出空行
        if self.current_player.is_none() || self.current_track.is_none() {
            // 在停止状态时不输出任何内容
            if status == "stopped" && self.last_output != "" {
                println!("");
                self.last_output = "".to_string();
            }
            return Ok(());
        }

        // 获取曲目信息，但不再输出
        if let Some(_track) = &self.current_track {
            // 计算提前显示的位置
            let advanced_position = self.current_position + self.config.display.lyric_advance_time;

            // 获取当前歌词（使用提前的位置）
            let current_lyric = self.lyrics_manager.get_lyric_at_time(advanced_position);

            // 检查当前歌词是否为空
            let mut lyric_text = String::new();

            if let Some(line) = current_lyric {
                // 如果当前歌词非空，则使用它
                if !line.text.trim().is_empty() {
                    lyric_text = line.text.clone();
                } else {
                    // 当前歌词为空，保持使用上次的非空输出
                    // 如果last_output不为空，就继续使用之前的输出
                    if !self.last_output.is_empty() {
                        // 不更新输出，保持上次的输出
                        return Ok(());
                    }
                }
            }

            // 如果lyric_text为空，尝试从整个歌词中找一个非空行
            if lyric_text.is_empty() {
                if let Some(lyrics) = self.lyrics_manager.get_current_lyrics() {
                    for line in &lyrics.lines {
                        if !line.text.trim().is_empty() {
                            lyric_text = line.text.clone();
                            break;
                        }
                    }
                }
            }

            // 如果依然没有找到非空歌词，且上次输出不为空，则保持上次输出
            if lyric_text.is_empty() && !self.last_output.is_empty() {
                return Ok(());
            }

            // 当暂停时不输出新歌词
            if status == "paused" {
                return Ok(());
            }

            // 只有播放中才输出歌词，并且只有当歌词内容变化时才输出
            if status == "playing" && !lyric_text.is_empty() {
                // 只输出歌词文本
                if lyric_text != self.last_output {
                    println!("{}", lyric_text);
                    self.last_output = lyric_text;
                }
            }
        }

        Ok(())
    }

    /// 显示轨道信息
    fn display_track_info(&self, track: &TrackInfo) -> Result<()> {
        let title = track.title.bold();
        let artist = track.artist.cyan();
        let album = track.album.green();

        println!("{} - {} ({})", title, artist, album);
        println!();

        if let Some(player) = &self.current_player {
            println!("播放器: {}", player);
        }

        Ok(())
    }

    /// 显示播放状态
    fn display_status(&self) -> Result<()> {
        let status_str = match self.current_status {
            PlaybackStatus::Playing => "▶ 播放中".green(),
            PlaybackStatus::Paused => "⏸ 已暂停".yellow(),
            PlaybackStatus::Stopped => "⏹ 已停止".red(),
        };

        if let Some(track) = &self.current_track {
            let position_str = format_time(self.current_position);
            let duration_str = format_time(track.length_ms);

            let progress = if track.length_ms > 0 {
                (self.current_position as f64 / track.length_ms as f64 * 100.0).round()
            } else {
                0.0
            };

            println!(
                "{} [{}/{}] {:.0}%",
                status_str, position_str, duration_str, progress
            );
        } else {
            println!("{}", status_str);
        }

        println!();

        Ok(())
    }

    /// 显示歌词
    fn display_lyrics(&self) -> Result<()> {
        // 获取当前歌词
        if let Some(lyrics) = self.lyrics_manager.get_current_lyrics() {
            // 计算提前显示的位置
            let advanced_position = self.current_position + self.config.display.lyric_advance_time;

            // 使用提前的位置查找歌词行
            let current_line = self.lyrics_manager.get_lyric_at_time(advanced_position);

            // 如果找到当前行，显示上下文
            if let Some(current) = current_line {
                // 找到当前行在歌词中的索引
                let mut current_index = 0;
                for (i, line) in lyrics.lines.iter().enumerate() {
                    if line.start_time == current.start_time && line.text == current.text {
                        current_index = i;
                        break;
                    }
                }

                // 计算上下文范围
                let context_lines = self.config.display.context_lines;
                let start = if current_index > context_lines {
                    current_index - context_lines
                } else {
                    0
                };

                let end = if current_index + context_lines < lyrics.lines.len() {
                    current_index + context_lines
                } else {
                    lyrics.lines.len() - 1
                };

                // 显示上下文歌词
                for i in start..=end {
                    let line = &lyrics.lines[i];
                    let line_text = if i == current_index {
                        // 当前行使用指定颜色
                        match self.config.display.current_line_color.as_str() {
                            "red" => line.text.red().bold(),
                            "green" => line.text.green().bold(),
                            "yellow" => line.text.yellow().bold(),
                            "blue" => line.text.blue().bold(),
                            "magenta" => line.text.magenta().bold(),
                            "cyan" => line.text.cyan().bold(),
                            "white" => line.text.white().bold(),
                            _ => line.text.green().bold(),
                        }
                    } else {
                        // 非当前行使用普通颜色
                        line.text.normal()
                    };

                    // 是否显示时间戳
                    if self.config.display.show_timestamp {
                        let time_str = format_time(line.start_time);
                        println!("[{}] {}", time_str, line_text);
                    } else {
                        println!("{}", line_text);
                    }
                }
            } else {
                println!("暂无歌词...");
            }
        } else {
            println!("未找到歌词");
        }

        Ok(())
    }

    /// 检查是否为当前播放器
    fn is_current_player(&self, player_name: &str) -> bool {
        if let Some(current) = &self.current_player {
            // 使用更灵活的匹配逻辑
            if current.to_lowercase() == player_name.to_lowercase() {
                return true;
            }

            // 支持部分匹配 - Spotify特殊处理
            if (current.to_lowercase().contains("spotify")
                && player_name.to_lowercase() == "spotify")
                || (current.to_lowercase() == "spotify"
                    && player_name.to_lowercase().contains("spotify"))
            {
                return true;
            }

            // 部分匹配: 如果一个包含另一个
            if current.to_lowercase().contains(&player_name.to_lowercase())
                || player_name.to_lowercase().contains(&current.to_lowercase())
            {
                return true;
            }

            false
        } else {
            false
        }
    }
}

/// 格式化时间（毫秒转为 mm:ss 格式）
fn format_time(ms: u64) -> String {
    let total_seconds = ms / 1000;
    let minutes = total_seconds / 60;
    let seconds = total_seconds % 60;
    format!("{:02}:{:02}", minutes, seconds)
}

/// 运行显示管理器
pub async fn run_display_manager(
    config: Arc<Config>,
    lyrics_manager: LyricsManager,
    player_events: Receiver<PlayerEvent>,
) -> Result<()> {
    // 创建显示管理器
    let mut display_manager = DisplayManager::new(config, lyrics_manager);

    // 简单输出模式下，不输出初始状态信息
    if !display_manager.config.display.simple_output {
        // 标准模式下，显示欢迎信息
        println!("MPRIS歌词显示器 - 等待播放器连接...");
        println!();
        println!("按 Ctrl+C 退出");
        println!();
    }

    // 运行显示管理器
    display_manager.run(player_events).await
}
