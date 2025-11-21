use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use anyhow::Result;
use log::{debug, error, info, warn};
use tokio::sync::mpsc::Sender;

use crate::mpris::{PlaybackStatus, PlayerEvent};

/// 播放器管理器
/// 负责维护播放器状态、选择活跃播放器
#[derive(Clone)]
pub struct PlayerManager {
    player_status: Arc<RwLock<HashMap<String, PlaybackStatus>>>,
    current_player: Arc<RwLock<Option<String>>>,
    manual_mode: Arc<RwLock<bool>>, // TUI模式为true（手动切换），Simple-output模式为false（自动切换）
    last_position_update: Arc<RwLock<HashMap<String, std::time::Instant>>>, // 跟踪播放器位置更新时间
    event_sender: Option<Sender<PlayerEvent>>,
}

impl PlayerManager {
    /// 创建新的播放器管理器
    pub fn new() -> Self {
        Self {
            player_status: Arc::new(RwLock::new(HashMap::new())),
            current_player: Arc::new(RwLock::new(None)),
            manual_mode: Arc::new(RwLock::new(false)), // 默认为自动模式
            last_position_update: Arc::new(RwLock::new(HashMap::new())),
            event_sender: None,
        }
    }

    /// 设置事件发送器
    pub fn set_event_sender(&mut self, sender: Sender<PlayerEvent>) {
        self.event_sender = Some(sender);
    }

    /// 设置播放器切换模式
    pub fn set_manual_mode(&self, manual: bool) {
        let mut manual_mode = self.manual_mode.write().unwrap();
        *manual_mode = manual;
        log::info!(
            "播放器切换模式设置为: {}",
            if manual {
                "手动模式"
            } else {
                "自动模式"
            }
        );
    }

    /// 处理播放器事件
    pub async fn handle_event(&self, event: &PlayerEvent) -> Result<()> {
        match event {
            PlayerEvent::PlaybackStatusChanged {
                player_name,
                status,
            } => {
                debug!("播放状态变更: {} - {:?}", player_name, status);

                // 更新播放器状态映射
                {
                    let mut player_status = self.player_status.write().unwrap();
                    player_status.insert(player_name.clone(), status.clone());
                }

                // 检查是否需要切换当前活跃播放器
                let manual_mode = *self.manual_mode.read().unwrap();

                match status {
                    PlaybackStatus::Playing => {
                        if !manual_mode {
                            // 自动模式：如果有播放器开始播放，立即切换到该播放器
                            let mut current = self.current_player.write().unwrap();

                            // 如果当前没有活跃的播放器，或者当前活跃播放器不是正在播放的播放器，则切换
                            if current.is_none() || current.as_ref().unwrap() != player_name {
                                *current = Some(player_name.clone());
                                info!("播放器开始播放，自动切换到播放器: {}", player_name);

                                // 发送活跃播放器变更事件
                                self.notify_active_player_changed(player_name);
                            }
                        } else {
                            // 手动模式：如果当前没有活跃播放器，才设置为当前播放器
                            let mut current = self.current_player.write().unwrap();
                            if current.is_none() {
                                *current = Some(player_name.clone());
                                info!("手动模式下设置初始播放器: {}", player_name);
                                self.notify_active_player_changed(player_name);
                            } else {
                                debug!(
                                    "手动模式下播放器 {} 开始播放，但不自动切换",
                                    player_name
                                );
                            }
                        }
                    }
                    PlaybackStatus::Paused | PlaybackStatus::Stopped => {
                        // 检查是否是当前活跃播放器暂停/停止
                        let mut current = self.current_player.write().unwrap();
                        let is_current_player = current.as_ref() == Some(player_name);

                        if is_current_player {
                            info!(
                                "[播放器切换] 当前活跃播放器 {} 已{}，寻找其他正在播放的播放器",
                                player_name,
                                match status {
                                    PlaybackStatus::Paused => "暂停",
                                    PlaybackStatus::Stopped => "停止",
                                    _ => "未知状态",
                                }
                            );

                            let best_player_option = self.select_best_player();
                            match best_player_option {
                                Some(best_player) => {
                                    // 如果找到了其他正在播放的播放器，立即切换
                                    if &best_player != player_name {
                                        info!(
                                            "[播放器切换] 成功切换：{} -> {}",
                                            player_name, best_player
                                        );
                                        *current = Some(best_player.clone());
                                        self.notify_active_player_changed(&best_player);
                                    } else {
                                        debug!("[播放器切换] 当前播放器仍是最佳选择，保持不变");
                                    }
                                }
                                None => {
                                    // 没有找到合适的播放器（例如所有播放器都停止了）
                                    info!(
                                        "[播放器切换] 没有其他可用的播放器，保持当前播放器: {}",
                                        player_name
                                    );
                                    // 保持当前播放器不变，即使它已暂停
                                }
                            }
                        } else {
                            debug!(
                                "[播放器切换] 非当前播放器 {} 状态变更为{:?}，无需切换",
                                player_name, status
                            );
                        }
                    }
                }
            }
            PlayerEvent::PlayerAppeared { player_name } => {
                info!("播放器出现: {}", player_name);
                // 不设置默认状态，等待真实的PlaybackStatusChanged事件
            }
            PlayerEvent::PlayerDisappeared { player_name } => {
                info!("播放器消失: {}", player_name);

                // 从播放器状态映射中移除
                {
                    let mut player_status = self.player_status.write().unwrap();
                    player_status.remove(player_name);
                }

                // 清除位置更新记录
                {
                    let mut last_update = self.last_position_update.write().unwrap();
                    last_update.remove(player_name);
                }

                // 如果是当前活跃播放器，需要切换到另一个播放器
                let mut current = self.current_player.write().unwrap();
                if let Some(current_name) = current.as_ref() {
                    if current_name == player_name {
                        // 清除当前播放器
                        *current = None;

                        // 优先选择状态为Playing的播放器
                        if let Some(best_player) = self.select_best_player() {
                            *current = Some(best_player.clone());
                            info!("切换到新的活跃播放器: {}", best_player);

                            // 发送活跃播放器变更事件
                            self.notify_active_player_changed(&best_player);
                        }
                    }
                }
            }
            PlayerEvent::PositionChanged {
                player_name,
                position_ms: _,
            } => {
                // 智能状态推断：通过位置更新推断播放器真实状态
                self.handle_position_update(player_name).await;
            }
            _ => {}
        }
        Ok(())
    }

    /// 选择最佳播放器作为当前活跃播放器
    fn select_best_player(&self) -> Option<String> {
        let player_status = self.player_status.read().unwrap();

        debug!("[选择播放器] 开始选择最佳播放器，当前播放器状态:");
        for (player, status) in player_status.iter() {
            debug!("[选择播放器]   {} -> {:?}", player, status);
        }

        // 获取位置更新记录用于智能推断
        let last_update = self.last_position_update.read().unwrap();
        let now = std::time::Instant::now();

        // 首先找出所有正在播放的播放器（包括通过位置更新推断的）
        let mut playing_players: Vec<String> = Vec::new();

        for (player, status) in player_status.iter() {
            let is_playing = if *status == PlaybackStatus::Playing {
                true
            } else {
                // 检查是否通过位置更新推断为播放状态
                if let Some(last_time) = last_update.get(player) {
                    let duration = now.duration_since(*last_time);
                    let recently_updated = duration < std::time::Duration::from_secs(3);
                    if recently_updated {
                        debug!(
                            "[选择播放器] 播放器 {} 状态为 {:?}，但最近有位置更新，推断为播放中",
                            player, status
                        );
                    }
                    recently_updated
                } else {
                    false
                }
            };

            if is_playing {
                playing_players.push(player.clone());
            }
        }

        if !playing_players.is_empty() {
            // 如果有正在播放的播放器，选择第一个
            debug!(
                "[选择播放器] 找到正在播放的播放器（包括推断）: {:?}, 选择: {}",
                playing_players, playing_players[0]
            );
            return Some(playing_players[0].clone());
        }

        // 如果没有正在播放的播放器，找出所有暂停的播放器
        let paused_players: Vec<String> = player_status
            .iter()
            .filter_map(|(player, status)| {
                if *status == PlaybackStatus::Paused {
                    Some(player.clone())
                } else {
                    None
                }
            })
            .collect();

        if !paused_players.is_empty() {
            // 如果有暂停的播放器，选择第一个
            debug!(
                "[选择播放器] 找到暂停的播放器: {:?}, 选择: {}",
                paused_players, paused_players[0]
            );
            return Some(paused_players[0].clone());
        }

        // 如果既没有播放也没有暂停的播放器，选择第一个可用的播放器
        let fallback = player_status.keys().next().cloned();
        debug!(
            "[选择播放器] 没有播放或暂停的播放器，回退选择: {:?}",
            fallback
        );
        fallback
    }

    /// 通知活跃播放器变更
    fn notify_active_player_changed(&self, player_name: &str) {
        if let Some(sender) = &self.event_sender {
            // 获取播放器状态，如果不存在则延迟发送通知，等待真实状态
            let status = {
                let player_status = self.player_status.read().unwrap();
                player_status.get(player_name).cloned()
            };

            // 如果没有状态信息，使用停止状态作为默认值
            let status = status.unwrap_or(PlaybackStatus::Stopped);

            info!(
                "[事件通知] 发送活跃播放器变更事件: {} (状态: {:?})",
                player_name, status
            );

            // 创建事件
            let event = PlayerEvent::ActivePlayerChanged {
                player_name: player_name.to_string(),
                status,
            };

            // 发送事件
            let sender = sender.clone();
            tokio::spawn(async move {
                if let Err(e) = sender.send(event).await {
                    error!("发送活跃播放器变更事件失败: {}", e);
                } else {
                    debug!("[事件通知] 活跃播放器变更事件发送成功");
                }
            });
        } else {
            warn!("[事件通知] 没有事件发送器，无法发送活跃播放器变更事件");
        }
    }

    /// 获取指定播放器的播放状态
    pub fn get_player_status(&self, player_name: &str) -> Option<PlaybackStatus> {
        let player_status = self.player_status.read().unwrap();
        player_status.get(player_name).cloned()
    }

    /// 获取所有可用播放器的列表
    pub fn get_available_players(&self) -> Vec<String> {
        let player_status = self.player_status.read().unwrap();
        player_status.keys().cloned().collect()
    }

    /// 获取当前活跃播放器名称
    pub fn get_current_player(&self) -> Option<String> {
        let current_player = self.current_player.read().unwrap();
        current_player.clone()
    }

    /// 手动设置当前播放器（用于TUI模式的手动切换）
    pub fn set_current_player(&self, player_name: String) -> bool {
        // 检查播放器是否存在
        let player_exists = {
            let player_status = self.player_status.read().unwrap();
            player_status.contains_key(&player_name)
        };

        if player_exists {
            let mut current = self.current_player.write().unwrap();
            *current = Some(player_name.clone());
            drop(current);

            // 发送活跃播放器变更事件
            self.notify_active_player_changed(&player_name);
            true
        } else {
            false
        }
    }

    /// 处理位置更新事件，进行智能状态推断
    async fn handle_position_update(&self, player_name: &str) {
        let now = std::time::Instant::now();

        // 更新播放器的最后位置更新时间
        {
            let mut last_update = self.last_position_update.write().unwrap();
            last_update.insert(player_name.to_string(), now);
        }

        // 获取播放器当前报告的状态
        let reported_status = {
            let player_status = self.player_status.read().unwrap();
            player_status.get(player_name).cloned()
        };

        // 如果播放器状态不是 Playing，但持续发送位置更新，推断为实际在播放
        if let Some(status) = reported_status {
            if status != PlaybackStatus::Playing {
                // 检查是否在短时间内持续收到位置更新（表明实际在播放）
                let should_infer_playing = {
                    let last_update = self.last_position_update.read().unwrap();
                    if let Some(last_time) = last_update.get(player_name) {
                        now.duration_since(*last_time) < std::time::Duration::from_secs(2)
                    } else {
                        false
                    }
                };

                if should_infer_playing {
                    info!(
                        "[状态纠正] 播放器 {} 发送位置更新但状态为 {:?}，推断为正在播放",
                        player_name, status
                    );

                    // 更新播放器状态为 Playing
                    {
                        let mut player_status = self.player_status.write().unwrap();
                        player_status.insert(player_name.to_string(), PlaybackStatus::Playing);
                    }

                    // 在自动模式下，切换到推断为播放状态的播放器
                    let manual_mode = *self.manual_mode.read().unwrap();
                    if !manual_mode {
                        let mut current = self.current_player.write().unwrap();

                        // 如果当前没有活跃播放器，或者当前播放器不是正在播放的，则切换
                        let should_switch = if let Some(current_player) = current.as_ref() {
                            let current_status = {
                                let player_status = self.player_status.read().unwrap();
                                player_status
                                    .get(current_player)
                                    .cloned()
                                    .unwrap_or(PlaybackStatus::Stopped)
                            };
                            current_status != PlaybackStatus::Playing
                        } else {
                            true
                        };

                        if should_switch {
                            info!("[状态纠正] 切换到推断为播放状态的播放器: {}", player_name);
                            *current = Some(player_name.to_string());
                            drop(current);

                            // 发送活跃播放器变更事件
                            self.notify_active_player_changed(player_name);
                        }
                    }
                }
            }
        }
    }
}
