use anyhow::Result;
use log::{debug, error, info};
use std::sync::Arc;
use tokio::sync::mpsc;

use crate::config::Config;
use crate::display;
use crate::lyrics;
use crate::mpris;
use crate::player;
use crate::tui;

pub struct App {
    config: Arc<Config>,
}

impl App {
    /// 创建新应用实例
    pub fn new(config: Arc<Config>) -> Result<Self> {
        Ok(Self { config })
    }

    /// 运行应用
    pub async fn run(&mut self) -> Result<()> {
        debug!("开始运行应用...");

        // 设置 MPRIS 监听器
        debug!("正在设置 MPRIS 播放器监听器...");
        let mpris_events = mpris::setup_mpris_listener(&self.config)?;
        debug!("MPRIS 监听器设置完成");

        // 创建事件转发通道
        let (tx_lyrics, rx_lyrics) = mpsc::channel::<mpris::PlayerEvent>(100);
        let (tx_display, rx_display) = mpsc::channel::<mpris::PlayerEvent>(100);
        // 内部事件通道，用于 PlayerManager 发送事件
        let (tx_internal, rx_internal) = mpsc::channel::<mpris::PlayerEvent>(100);
        debug!("事件通道创建完成");

        // 创建播放器管理器
        debug!("正在创建播放器管理器...");
        let mut player_manager = player::PlayerManager::new();
        player_manager.set_event_sender(tx_internal);
        debug!("播放器管理器创建完成");

        // 创建歌词管理器
        debug!("正在创建歌词管理器...");
        let lyrics_manager = lyrics::setup_lyrics_manager(Arc::clone(&self.config));
        debug!("歌词管理器创建完成");

        // 启动事件转发器
        let mpris_events_clone = mpris_events;
        let tx_lyrics_clone = tx_lyrics.clone();
        let tx_display_clone = tx_display.clone();
        let player_manager_clone = player_manager.clone();

        debug!("启动事件转发器...");
        tokio::spawn(async move {
            forward_events(
                mpris_events_clone,
                rx_internal,
                tx_lyrics_clone,
                tx_display_clone,
                player_manager_clone,
            )
            .await;
        });

        // 启动歌词管理器
        let lyrics_manager_clone = lyrics_manager.clone();
        debug!("启动歌词管理器...");
        let lyrics_manager_handle = tokio::spawn(async move {
            if let Err(e) = lyrics_manager_clone.run(rx_lyrics).await {
                error!("歌词管理器运行失败: {}", e);
            }
        });

        // 根据配置选择界面模式
        let display_handle = if self.config.display.simple_output || !self.config.display.enable_tui {
            // 简单输出模式：使用传统显示管理器（自动切换模式）
            debug!("启动传统显示管理器（简单输出模式）...");
            player_manager.set_manual_mode(false); // Simple-output模式使用自动切换
            let config_clone = Arc::clone(&self.config);
            tokio::spawn(async move {
                info!("开始显示歌词（简单输出模式）...");
                if let Err(e) =
                    display::run_display_manager(config_clone, lyrics_manager, player_manager, rx_display).await
                {
                    error!("显示管理器运行失败: {}", e);
                }
            })
        } else {
            // TUI 模式：使用新的 ratatui 界面（手动切换模式）
            debug!("启动 TUI 界面...");
            player_manager.set_manual_mode(true); // TUI模式使用手动切换
            let config_clone = Arc::clone(&self.config);
            tokio::spawn(async move {
                info!("开始 TUI 界面...");
                let mut tui_app = tui::TuiApp::new(config_clone, lyrics_manager, player_manager);
                if let Err(e) = tui_app.run(rx_display).await {
                    error!("TUI 应用运行失败: {}", e);
                }
            })
        };

        // 等待任务完成（通常不会主动完成，除非出错）
        debug!("所有组件已启动，等待运行...");
        tokio::select! {
            result = lyrics_manager_handle => {
                if let Err(e) = result {
                    error!("歌词管理器任务出错: {}", e);
                }
            }
            result = display_handle => {
                if let Err(e) = result {
                    error!("界面管理器任务出错: {}", e);
                }
            }
        }

        debug!("应用执行完毕");
        Ok(())
    }
}

/// 将MPRIS事件转发到多个接收者，并进行优化
async fn forward_events(
    mut mpris_events: mpsc::Receiver<mpris::PlayerEvent>,
    mut internal_events: mpsc::Receiver<mpris::PlayerEvent>,
    tx_lyrics: mpsc::Sender<mpris::PlayerEvent>,
    tx_display: mpsc::Sender<mpris::PlayerEvent>,
    player_manager: player::PlayerManager,
) {
    debug!("事件转发器启动");

    loop {
        let event = tokio::select! {
            Some(event) = mpris_events.recv() => {
                // 处理 MPRIS 事件
                // 先让 PlayerManager 处理（更新状态、智能推断等）
                if let Err(e) = player_manager.handle_event(&event).await {
                    error!("PlayerManager 处理事件失败: {}", e);
                }
                Some(event)
            }
            Some(event) = internal_events.recv() => {
                // 处理内部事件（如 ActivePlayerChanged）
                Some(event)
            }
            else => None,
        };

        if let Some(event) = event {
            let mut send_to_lyrics = true;
            let send_to_display = true;

            match &event {
                mpris::PlayerEvent::PositionChanged { .. } => {
                    // 位置变更不需要发送给歌词管理器
                    send_to_lyrics = false;
                }
                _ => {}
            }

            if send_to_lyrics {
                // Clone for lyrics manager, keeping the original for display
                if let Err(e) = tx_lyrics.send(event.clone()).await {
                    error!("事件转发到歌词管理器失败: {}", e);
                }
            }

            if send_to_display {
                // Move the original event to display manager
                if let Err(e) = tx_display.send(event).await {
                    error!("事件转发到显示管理器失败: {}", e);
                }
            }
        } else {
            break;
        }
    }

    debug!("事件转发器退出");
}
