use anyhow::Result;
use log::{debug, error, info};
use std::sync::Arc;
use tokio::sync::mpsc;

use crate::config::Config;
use crate::display;
use crate::lyrics;
use crate::mpris;

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
        debug!("事件通道创建完成");

        // 创建歌词管理器
        debug!("正在创建歌词管理器...");
        let mut lyrics_manager = lyrics::setup_lyrics_manager(Arc::clone(&self.config));
        // 设置事件发送器，用于发送活跃播放器变更事件
        lyrics_manager.set_event_sender(tx_display.clone());
        debug!("歌词管理器创建完成");

        // 启动事件转发器
        let mpris_events_clone = mpris_events;
        let tx_lyrics_clone = tx_lyrics.clone();
        let tx_display_clone = tx_display.clone();

        debug!("启动事件转发器...");
        tokio::spawn(async move {
            forward_events(mpris_events_clone, tx_lyrics_clone, tx_display_clone).await;
        });

        // 启动歌词管理器
        let lyrics_manager_clone = lyrics_manager.clone();
        debug!("启动歌词管理器...");
        let lyrics_manager_handle = tokio::spawn(async move {
            if let Err(e) = lyrics_manager_clone.run(rx_lyrics).await {
                error!("歌词管理器运行失败: {}", e);
            }
        });

        // 启动显示管理器
        debug!("启动显示管理器...");
        let config_clone = Arc::clone(&self.config);
        let display_handle = tokio::spawn(async move {
            info!("开始显示歌词...");
            if let Err(e) =
                display::run_display_manager(config_clone, lyrics_manager, rx_display).await
            {
                error!("显示管理器运行失败: {}", e);
            }
        });

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
                    error!("显示管理器任务出错: {}", e);
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
    tx_lyrics: mpsc::Sender<mpris::PlayerEvent>,
    tx_display: mpsc::Sender<mpris::PlayerEvent>,
) {
    debug!("事件转发器启动");

    while let Some(event) = mpris_events.recv().await {
        let mut send_to_lyrics = true;
        let send_to_display = true;
        let mut skip_generic_send = false;

        match &event {
            mpris::PlayerEvent::TrackChanged {
                player_name,
                track_info,
            } => {
                info!(
                    "当前播放: {} - {} ({})",
                    track_info.title, track_info.artist, player_name
                );
                // 需要发送给两者
            }
            mpris::PlayerEvent::PlaybackStatusChanged {
                player_name,
                status,
            } => {
                debug!(
                    "收到播放状态变更事件: 播放器={}, 状态={:?}",
                    player_name, status
                );
                // 需要发送给两者
            }
            mpris::PlayerEvent::PositionChanged {
                player_name,
                position_ms,
            } => {
                debug!(
                    "收到位置变更事件: 播放器={}, 位置={}ms",
                    player_name, position_ms
                );
                // 只发送给 display
                send_to_lyrics = false;
            }
            mpris::PlayerEvent::PlayerAppeared { player_name } => {
                debug!("收到播放器出现事件: 播放器={}", player_name);
                // 需要发送给两者
            }
            mpris::PlayerEvent::PlayerDisappeared { player_name } => {
                debug!("收到播放器消失事件: 播放器={}", player_name);
                // 需要发送给两者
            }
            mpris::PlayerEvent::ActivePlayerChanged {
                player_name,
                status: _, // 在 mpris 模块中已经获取并包含在事件里
            } => {
                // 特殊处理: 在此直接发送给两者，并跳过通用发送逻辑
                info!("当前活跃播放器: {}", player_name);

                let event_clone_lyrics = event.clone();
                if let Err(e) = tx_lyrics.send(event_clone_lyrics).await {
                    error!("向LyricsManager发送ActivePlayerChanged事件失败: {}", e);
                }
                // 克隆事件发送给 tx_display
                let event_clone_display = event.clone();
                if let Err(e) = tx_display.send(event_clone_display).await {
                    error!("向DisplayManager发送ActivePlayerChanged事件失败: {}", e);
                }
                skip_generic_send = true; // 跳过下面的通用发送
            }
            mpris::PlayerEvent::NoPlayersAvailable => {
                debug!("收到无播放器事件，转发给歌词管理器和显示管理器");
                // 需要发送给两者
            }
        }

        // 通用发送逻辑 (如果未被跳过)
        if !skip_generic_send {
            if send_to_lyrics {
                let event_clone = event.clone(); // 可能需要克隆
                if let Err(e) = tx_lyrics.send(event_clone).await {
                    error!("事件转发到歌词管理器失败: {}", e);
                }
            }

            if send_to_display {
                // 如果 lyrics 也发送了，这里需要 event 的克隆
                // 如果 lyrics 没发送，event 的所有权可以直接转移
                let event_to_send = if send_to_lyrics { event.clone() } else { event };
                if let Err(e) = tx_display.send(event_to_send).await {
                    error!("事件转发到显示管理器失败: {}", e);
                }
            }
        }
    }

    debug!("事件转发器退出");
}
