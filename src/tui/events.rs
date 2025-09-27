use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEvent};
use std::time::Duration;
use tokio::sync::mpsc;

use crate::mpris::PlayerEvent;

/// TUI 事件类型
#[derive(Debug, Clone)]
pub enum TuiEvent {
    /// 键盘输入事件
    Key(KeyEvent),
    /// MPRIS 播放器事件
    Player(PlayerEvent),
    /// 定时刷新事件
    Tick,
    /// 退出事件
    Quit,
}

/// 事件处理器
pub struct EventHandler {
    mpris_events: mpsc::Receiver<PlayerEvent>,
    tick_rate: Duration,
}

impl EventHandler {
    /// 创建新的事件处理器
    pub fn new(mpris_events: mpsc::Receiver<PlayerEvent>, tick_rate: Duration) -> Self {
        Self {
            mpris_events,
            tick_rate,
        }
    }

    /// 监听事件并发送到通道
    pub async fn run(&mut self, tx: mpsc::Sender<TuiEvent>) -> Result<()> {
        let mut last_tick = std::time::Instant::now();
        let mut tick_interval = tokio::time::interval(self.tick_rate);

        loop {
            tokio::select! {
                // 处理 MPRIS 播放器事件（高优先级）
                player_event = self.mpris_events.recv() => {
                    if let Some(event) = player_event {
                        if tx.send(TuiEvent::Player(event)).await.is_err() {
                            break; // 接收端已关闭
                        }
                    }
                }

                // 处理键盘输入事件（高优先级）
                _ = tick_interval.tick() => {
                    // 非阻塞检查键盘输入
                    if event::poll(Duration::from_millis(0))? {
                        match event::read()? {
                            Event::Key(key) => {
                                if tx.send(TuiEvent::Key(key)).await.is_err() {
                                    break;
                                }
                            }
                            _ => {}
                        }
                    }

                    // 发送定时刷新事件（低频率）
                    if last_tick.elapsed() >= self.tick_rate {
                        if tx.send(TuiEvent::Tick).await.is_err() {
                            break;
                        }
                        last_tick = std::time::Instant::now();
                    }
                }
            }
        }

        Ok(())
    }

    /// 处理按键事件，返回是否应该退出
    pub fn handle_key_event(key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => true,
            KeyCode::Char('c')
                if key
                    .modifiers
                    .contains(crossterm::event::KeyModifiers::CONTROL) =>
            {
                true
            }
            _ => false,
        }
    }
}
