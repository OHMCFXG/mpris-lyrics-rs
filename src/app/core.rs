use anyhow::Result;
use log::{debug, error, info};
use std::sync::Arc;

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
        let mpris_event_sender = mpris::setup_mpris_listener(&self.config)?;
        let rx_lyrics = mpris_event_sender.subscribe();
        let rx_display = mpris_event_sender.subscribe();
        debug!("MPRIS 监听器和事件广播通道设置完成");

        // 创建歌词管理器
        debug!("正在创建歌词管理器...");
        let lyrics_manager = lyrics::setup_lyrics_manager(Arc::clone(&self.config));
        debug!("歌词管理器创建完成");

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
