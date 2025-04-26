use std::io::{self, Write};
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use colored::Colorize;
use log::debug;
use tokio::sync::mpsc::Receiver;
use tokio::time;

use crate::config::Config;
use crate::display::formatter;
use crate::display::renderer;
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
                        debug!(
                            "获取到活跃播放器曲目信息: {} - {}",
                            track_info.title, track_info.artist
                        );
                        self.current_track = Some(track_info);
                    } else {
                        debug!("未获取到活跃播放器曲目信息");
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

                    // 播放器变更时刷新显示
                    self.refresh_display()?;
                } else {
                    // 即使播放器没有变化，也更新状态
                    self.current_status = status;

                    // 根据状态更新 last_update
                    if self.current_status == PlaybackStatus::Playing {
                        self.last_update = chrono::Utc::now().timestamp_millis() as u64;
                    } else {
                        self.last_update = 0;
                    }

                    // 刷新显示
                    self.refresh_display()?;
                }
            }
        }

        Ok(())
    }

    /// 刷新显示内容
    fn refresh_display(&mut self) -> Result<()> {
        // 检查是否使用简单输出模式
        if self.config.display.simple_output {
            return self.refresh_display_simple();
        }

        // 如果进度条显示已禁用（例如 --no-clear 模式），则直接返回
        // 在这种模式下，显示管理器只响应特定事件，如轨道变更，而不会定期刷新显示
        if !self.config.display.show_progress {
            return Ok(());
        }

        // 清屏
        print!("\x1B[2J\x1B[1;1H");
        io::stdout().flush()?;

        // 显示轨道信息
        if let Some(track) = &self.current_track {
            self.display_track_info(track)?;
        } else {
            // 如果没有轨道信息，显示等待消息
            println!("没有正在播放的歌曲");
        }

        // 显示播放状态
        self.display_status()?;

        // 显示歌词
        self.display_lyrics()?;

        // 刷新输出
        io::stdout().flush()?;

        Ok(())
    }

    /// 简单输出模式刷新
    fn refresh_display_simple(&mut self) -> Result<()> {
        let lyric_advance_time = self.config.display.lyric_advance_time;
        let position_with_advance = self.current_position + lyric_advance_time;

        // 如果停止播放，或者没有播放器，显示默认信息
        if self.current_status != PlaybackStatus::Playing || self.current_player.is_none() {
            let output = "没有正在播放的歌曲".to_string();
            // 避免输出相同的内容
            if output != self.last_output {
                println!("{}", output);
                self.last_output = output;
            }
            return Ok(());
        }

        // 如果有歌词，尝试获取当前行
        let current_lyric = self
            .lyrics_manager
            .get_lyric_at_time(position_with_advance)
            .map(|line| line.text)
            .unwrap_or_else(|| {
                if let Some(track) = &self.current_track {
                    format!("{} - {}", track.title, track.artist)
                } else {
                    "无歌词".to_string()
                }
            });

        // 避免输出相同的内容
        if current_lyric != self.last_output {
            println!("{}", current_lyric);
            self.last_output = current_lyric;
        }

        Ok(())
    }

    /// 显示轨道信息
    fn display_track_info(&self, track: &TrackInfo) -> Result<()> {
        println!(
            "当前播放: {} - {} / {}",
            track.title.bold(),
            track.artist.bold(),
            track.album
        );

        if let Some(current_player) = &self.current_player {
            println!("播放器: {}", current_player);
        }

        println!("长度: {}", formatter::format_time(track.length_ms));
        println!();

        Ok(())
    }

    /// 显示播放状态
    fn display_status(&self) -> Result<()> {
        // 显示播放位置
        let position_str = formatter::format_time(self.current_position);
        let total_str = match &self.current_track {
            Some(track) if track.length_ms > 0 => formatter::format_time(track.length_ms),
            _ => "未知".to_string(),
        };

        match self.current_status {
            PlaybackStatus::Playing => print!("▶ "),
            PlaybackStatus::Paused => print!("⏸ "),
            PlaybackStatus::Stopped => print!("⏹ "),
        }

        println!("{} / {}", position_str, total_str);

        // 显示进度条
        if let Some(track) = &self.current_track {
            if track.length_ms > 0 {
                renderer::render_progress_bar(self.current_position, track.length_ms)?;
            }
        }

        println!();
        Ok(())
    }

    /// 显示歌词
    fn display_lyrics(&self) -> Result<()> {
        // 加上提前显示时间
        let lyric_advance_time = self.config.display.lyric_advance_time;
        let position_with_advance = self.current_position + lyric_advance_time;

        let lyrics = self.lyrics_manager.get_current_lyrics();

        // 1. 如果没有歌词，显示提示
        if lyrics.is_none() {
            println!("没有可用的歌词");
            return Ok(());
        }

        let lyrics = lyrics.unwrap();
        if lyrics.lines.is_empty() {
            println!("歌词为空");
            return Ok(());
        }

        // 调试输出当前播放位置
        debug!(
            "当前播放位置: {}ms (加上提前时间: {}ms)",
            self.current_position, position_with_advance
        );

        // 2. 寻找当前行 - 修改查找逻辑
        let mut current_index = 0;
        let mut found_exact_match = false;

        // 首先尝试找到一个精确匹配的行（当前时间在其开始和结束时间之间）
        for (i, line) in lyrics.lines.iter().enumerate() {
            // 如果当前时间在这一行的时间范围内
            if line.start_time <= position_with_advance
                && (line.end_time.is_none() || position_with_advance < line.end_time.unwrap())
            {
                current_index = i;
                found_exact_match = true;
                debug!(
                    "找到匹配行 #{}: 开始={}, 结束={:?}, 文本={}",
                    i, line.start_time, line.end_time, line.text
                );
                break;
            }
        }

        // 如果没有找到精确匹配，使用最接近的行
        if !found_exact_match {
            if position_with_advance < lyrics.lines[0].start_time {
                // 如果当前时间在第一行开始前，使用第一行
                current_index = 0;
                debug!(
                    "当前时间在第一行开始前，使用第一行: 开始={}, 文本={}",
                    lyrics.lines[0].start_time, lyrics.lines[0].text
                );
            } else {
                // 找到最后一个开始时间不大于当前时间的行
                for (i, line) in lyrics.lines.iter().enumerate() {
                    if line.start_time <= position_with_advance {
                        current_index = i;
                    } else {
                        break;
                    }
                }
                debug!(
                    "使用最近的行 #{}: 开始={}, 结束={:?}, 文本={}",
                    current_index,
                    lyrics.lines[current_index].start_time,
                    lyrics.lines[current_index].end_time,
                    lyrics.lines[current_index].text
                );
            }
        }

        // 3. 显示上下文行
        let context_lines = self.config.display.context_lines;
        let start_index = if current_index >= context_lines {
            current_index - context_lines
        } else {
            0
        };

        let end_index = std::cmp::min(current_index + context_lines + 1, lyrics.lines.len());

        // 打印歌词
        for i in start_index..end_index {
            let line = &lyrics.lines[i];
            let line_text = &line.text;

            // 如果是当前行，使用彩色显示
            if i == current_index {
                // 应用颜色
                let color_name = &self.config.display.current_line_color;
                let colored_text = renderer::colorize_text(line_text, color_name);

                if self.config.display.show_timestamp {
                    println!(
                        "[{}] {}",
                        formatter::format_time(line.start_time),
                        colored_text
                    );
                } else {
                    println!("▶ {}", colored_text);
                }
            } else {
                // 其他行正常显示
                if self.config.display.show_timestamp {
                    println!(
                        "[{}] {}",
                        formatter::format_time(line.start_time),
                        line_text
                    );
                } else {
                    println!("  {}", line_text);
                }
            }
        }

        Ok(())
    }

    /// 检查是否是当前播放器
    fn is_current_player(&self, player_name: &str) -> bool {
        self.current_player
            .as_ref()
            .map_or(false, |p| p == player_name)
    }
}

/// 运行显示管理器
pub async fn run_display_manager(
    config: Arc<Config>,
    lyrics_manager: LyricsManager,
    player_events: Receiver<PlayerEvent>,
) -> Result<()> {
    let mut display_manager = DisplayManager::new(config, lyrics_manager);
    display_manager.run(player_events).await
}
