// 显示管理模块
// 导出显示相关的结构体和函数

mod formatter;
mod manager;
mod renderer;

pub use formatter::*;
pub use manager::DisplayManager;
pub use renderer::*;

// 重新导出原 display 模块的 run_display_manager 函数
pub use manager::run_display_manager;
