use anyhow::Result;
use crossterm::{
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};
use std::io;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;

use crate::config::Config;
use crate::lyrics::LyricsManager;
use crate::mpris::{PlaybackStatus, PlayerEvent};
use crate::tui::events::{EventHandler, TuiEvent};
use crate::tui::theme::Theme;
use crate::tui::ui::{render_help, render_ui, UiState};
use crate::tui::widgets::SourceStatus;

/// TUI 应用主结构
pub struct TuiApp {
    config: Arc<Config>,
    lyrics_manager: LyricsManager,
    theme: Theme,
    ui_state: UiState,
    should_quit: bool,
    show_help: bool,
    needs_redraw: bool,
}

impl TuiApp {
    /// 创建新的 TUI 应用
    pub fn new(config: Arc<Config>, lyrics_manager: LyricsManager) -> Self {
        let theme = Theme::default(); // 使用终端原生配色

        Self {
            config,
            lyrics_manager,
            theme,
            ui_state: UiState::default(),
            should_quit: false,
            show_help: false,
            needs_redraw: true, // 初始需要绘制
        }
    }

    /// 运行 TUI 应用
    pub async fn run(&mut self, player_events: mpsc::Receiver<PlayerEvent>) -> Result<()> {
        // 设置终端
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen)?;
        let backend = CrosstermBackend::new(stdout);
        let mut terminal = Terminal::new(backend)?;

        // 创建事件处理器 - 降低刷新频率以提升性能
        let (tx, mut rx) = mpsc::channel(100);
        let mut event_handler = EventHandler::new(player_events, Duration::from_millis(100));

        // 启动事件监听
        let event_tx = tx.clone();
        tokio::spawn(async move {
            if let Err(err) = event_handler.run(event_tx).await {
                log::error!("事件处理器错误: {}", err);
            }
        });

        // 主循环
        while !self.should_quit {
            // 只在需要时重绘界面
            if self.needs_redraw {
                terminal.draw(|f| {
                    // 先渲染主界面
                    render_ui(
                        f,
                        &self.config,
                        &self.lyrics_manager,
                        &self.ui_state,
                        &self.theme,
                    );

                    // 如果显示帮助，覆盖显示帮助界面
                    if self.show_help {
                        render_help(f, &self.theme);
                    }
                })?;
                self.needs_redraw = false;
            }

            // 处理事件
            if let Some(event) = rx.recv().await {
                self.handle_event(event).await?;
            }
        }

        // 恢复终端
        disable_raw_mode()?;
        execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
        terminal.show_cursor()?;

        Ok(())
    }

    /// 处理事件
    async fn handle_event(&mut self, event: TuiEvent) -> Result<()> {
        match event {
            TuiEvent::Key(key) => {
                if EventHandler::handle_key_event(key) {
                    self.should_quit = true;
                } else {
                    self.handle_key_input(key).await?;
                    self.needs_redraw = true; // 按键事件需要重绘
                }
            }
            TuiEvent::Player(player_event) => {
                log::debug!("收到播放器事件: {:?}", player_event);
                self.handle_player_event(player_event).await?;
                self.needs_redraw = true; // 播放器事件需要重绘
            }
            TuiEvent::Tick => {
                // 检查歌词状态是否需要更新
                let old_source_status = self.ui_state.status_info.source_status.clone();
                self.handle_tick().await?;
                // 只在歌词状态变化时重绘
                if self.ui_state.status_info.source_status != old_source_status {
                    self.needs_redraw = true;
                }
            }
            TuiEvent::Quit => {
                self.should_quit = true;
            }
        }
        Ok(())
    }

    /// 处理按键输入
    async fn handle_key_input(&mut self, key: crossterm::event::KeyEvent) -> Result<()> {
        use crossterm::event::KeyCode;

        match key.code {
            KeyCode::Char('h') | KeyCode::Char('?') => {
                self.show_help = !self.show_help;
            }
            KeyCode::Char('r') => {
                // 刷新歌词
                if let Some(track) = &self.ui_state.current_track {
                    // TODO: 触发歌词重新获取
                    log::info!("手动刷新歌词: {} - {}", track.title, track.artist);
                }
            }
            KeyCode::Tab => {
                // Tab键切换播放器
                self.switch_to_next_player().await?;
            }
            KeyCode::Char('t') => {
                // 切换时间戳显示
                // TODO: 实现时间戳切换
                log::info!("切换时间戳显示");
            }
            _ => {}
        }
        Ok(())
    }

    /// 处理播放器事件
    async fn handle_player_event(&mut self, event: PlayerEvent) -> Result<()> {
        match event {
            PlayerEvent::TrackChanged {
                player_name,
                track_info,
            } => {
                // 只处理当前活跃播放器的轨道变更
                if self.is_current_player(&player_name) {
                    self.ui_state.current_track = Some(track_info.clone());

                    // 更新状态信息
                    self.ui_state.status_info.lyrics_source = Some("搜索中".to_string());
                    self.ui_state.status_info.source_status = SourceStatus::Loading;

                    log::info!(
                        "当前播放器轨道变更: {} - {} ({})",
                        track_info.title,
                        track_info.artist,
                        player_name
                    );
                } else {
                    log::debug!(
                        "非当前播放器轨道变更: {} - {} ({})",
                        track_info.title,
                        track_info.artist,
                        player_name
                    );
                }
            }
            PlayerEvent::PlaybackStatusChanged {
                player_name,
                status,
            } => {
                if self.is_current_player(&player_name) {
                    log::debug!("播放状态变更: {:?}", status);
                    self.ui_state.playback_status = status;
                }
            }
            PlayerEvent::PositionChanged {
                player_name,
                position_ms,
            } => {
                if self.is_current_player(&player_name) {
                    self.ui_state.current_position = position_ms;
                }
            }
            PlayerEvent::PlayerAppeared { player_name } => {
                if self.ui_state.current_player.is_none() {
                    self.ui_state.current_player = Some(player_name.clone());

                    // 尝试获取轨道信息
                    if let Some(track_info) = self.lyrics_manager.get_track_info(&player_name) {
                        log::debug!(
                            "播放器连接时获取到轨道信息: {} - {}",
                            track_info.title,
                            track_info.artist
                        );
                        self.ui_state.current_track = Some(track_info);
                        self.ui_state.status_info.lyrics_source = Some("搜索中".to_string());
                        self.ui_state.status_info.source_status = SourceStatus::Loading;

                        // 尝试获取播放状态，但不设置默认状态，等待真实的PlaybackStatusChanged事件
                        if let Some(status) = self.lyrics_manager.get_player_status(&player_name) {
                            log::debug!("获取到播放器状态: {:?}", status);
                            self.ui_state.playback_status = status;
                        } else {
                            log::debug!("未获取到播放器状态，等待PlaybackStatusChanged事件");
                            // 不设置默认状态，保持当前状态或等待真实状态事件
                        }
                    }
                }
                log::info!("播放器连接: {}", player_name);
            }
            PlayerEvent::PlayerDisappeared { player_name } => {
                if self.is_current_player(&player_name) {
                    self.ui_state.current_player = None;
                    self.ui_state.current_track = None;
                    self.ui_state.playback_status = PlaybackStatus::Stopped;
                }
                log::info!("播放器断开: {}", player_name);
            }
            PlayerEvent::ActivePlayerChanged {
                player_name,
                status,
            } => {
                log::info!("活跃播放器变更: {} (状态: {:?})", player_name, status);

                // 更新当前播放器和状态
                self.ui_state.current_player = Some(player_name.clone());
                self.ui_state.playback_status = status;

                // 尝试从歌词管理器获取当前播放器的轨道信息
                if let Some(track_info) = self.lyrics_manager.get_track_info(&player_name) {
                    log::debug!(
                        "从歌词管理器获取到轨道信息: {} - {}",
                        track_info.title,
                        track_info.artist
                    );
                    self.ui_state.current_track = Some(track_info);
                    self.ui_state.status_info.lyrics_source = Some("搜索中".to_string());
                    self.ui_state.status_info.source_status = SourceStatus::Loading;
                } else {
                    log::debug!("歌词管理器中没有当前播放器的轨道信息，等待轨道变更事件");
                    // 只有在没有轨道信息时才清空，避免显示"等待播放音乐"
                    if self.ui_state.current_track.is_none() {
                        self.ui_state.status_info.lyrics_source = Some("加载中".to_string());
                        self.ui_state.status_info.source_status = SourceStatus::Loading;
                    }
                }
            }
            PlayerEvent::NoPlayersAvailable => {
                self.ui_state.current_player = None;
                self.ui_state.current_track = None;
                self.ui_state.playback_status = PlaybackStatus::Stopped;
                log::info!("没有可用的播放器");
            }
        }
        Ok(())
    }

    /// 处理定时事件
    async fn handle_tick(&mut self) -> Result<()> {
        // 更新歌词状态
        self.update_lyrics_status();

        Ok(())
    }

    /// 更新歌词状态
    fn update_lyrics_status(&mut self) {
        let lyrics = self.lyrics_manager.get_current_lyrics();

        if lyrics.is_some() {
            self.ui_state.status_info.source_status = SourceStatus::Success;
            // 获取歌词来源
            // TODO: 从歌词管理器获取实际来源信息
            self.ui_state.status_info.lyrics_source = Some("网易云".to_string());
        } else if self.ui_state.current_track.is_some() {
            self.ui_state.status_info.source_status = SourceStatus::Loading;
        } else {
            self.ui_state.status_info.source_status = SourceStatus::None;
            self.ui_state.status_info.lyrics_source = None;
        }
    }

    /// 检查是否为当前播放器
    fn is_current_player(&self, player_name: &str) -> bool {
        self.ui_state
            .current_player
            .as_ref()
            .map_or(false, |current| current == player_name)
    }

    /// 切换到下一个播放器
    async fn switch_to_next_player(&mut self) -> Result<()> {
        let available_players = self.lyrics_manager.get_available_players();

        if available_players.len() <= 1 {
            log::info!("只有 {} 个播放器，无需切换", available_players.len());
            return Ok(());
        }

        let current_player = self.ui_state.current_player.clone();
        let next_player = if let Some(current) = current_player {
            // 找到当前播放器在列表中的位置
            if let Some(current_index) = available_players.iter().position(|p| p == &current) {
                // 切换到下一个播放器（循环）
                let next_index = (current_index + 1) % available_players.len();
                available_players[next_index].clone()
            } else {
                // 当前播放器不在列表中，选择第一个
                available_players[0].clone()
            }
        } else {
            // 没有当前播放器，选择第一个
            available_players[0].clone()
        };

        // 使用歌词管理器切换播放器
        if self.lyrics_manager.set_current_player(next_player.clone()) {
            log::info!("手动切换到播放器: {}", next_player);

            // 立即更新UI状态，不等待ActivePlayerChanged事件
            self.ui_state.current_player = Some(next_player.clone());

            // 尝试获取轨道信息
            if let Some(track_info) = self.lyrics_manager.get_track_info(&next_player) {
                self.ui_state.current_track = Some(track_info);
                self.ui_state.status_info.lyrics_source = Some("搜索中".to_string());
                self.ui_state.status_info.source_status = SourceStatus::Loading;
            }

            // 获取播放状态，但不设置默认值
            if let Some(status) = self.lyrics_manager.get_player_status(&next_player) {
                log::debug!("切换时获取到播放状态: {:?}", status);
                self.ui_state.playback_status = status;
            } else {
                log::debug!("切换时未获取到播放状态，保持当前状态");
                // 保持当前播放状态，等待真实的PlaybackStatusChanged事件
            }
        } else {
            log::warn!("切换到播放器 {} 失败", next_player);
        }

        Ok(())
    }
}
