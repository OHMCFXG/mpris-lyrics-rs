use anyhow::Result;
use colored::{Color, Colorize};
use std::io::{self, Write};

/// 将文本着色
pub fn colorize_text(text: &str, color_name: &str) -> String {
    match color_name.to_lowercase().as_str() {
        "red" => text.red().to_string(),
        "green" => text.green().to_string(),
        "yellow" => text.yellow().to_string(),
        "blue" => text.blue().to_string(),
        "magenta" => text.magenta().to_string(),
        "cyan" => text.cyan().to_string(),
        "white" => text.white().to_string(),
        "bright_red" => text.bright_red().to_string(),
        "bright_green" => text.bright_green().to_string(),
        "bright_yellow" => text.bright_yellow().to_string(),
        "bright_blue" => text.bright_blue().to_string(),
        "bright_magenta" => text.bright_magenta().to_string(),
        "bright_cyan" => text.bright_cyan().to_string(),
        "bright_white" => text.bright_white().to_string(),
        _ => text.green().to_string(),
    }
}

/// 渲染进度条
pub fn render_progress_bar(current_ms: u64, total_ms: u64) -> Result<()> {
    // 进度条宽度 (终端80列减去其他文本长度)
    let width = 50;

    if total_ms == 0 {
        return Ok(());
    }

    // 计算进度
    let percent = current_ms as f64 / total_ms as f64;
    let filled_width = (percent * width as f64) as usize;

    // 创建进度条
    print!("[");
    for i in 0..width {
        if i < filled_width {
            print!("=");
        } else if i == filled_width {
            print!(">");
        } else {
            print!(" ");
        }
    }
    print!("] ");

    // 打印百分比
    println!("{:.1}%", percent * 100.0);

    io::stdout().flush()?;
    Ok(())
}
