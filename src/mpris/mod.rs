// MPRIS 交互模块
// 导出与媒体播放器交互的结构体和函数

mod events;
mod listener;
mod types;

pub use events::*;
pub use listener::*;
pub use types::*;

// 重新导出原 mpris 模块的 setup_mpris_listener 函数
pub use listener::setup_mpris_listener;
