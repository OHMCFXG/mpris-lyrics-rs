use anyhow::Result;
use clap::Parser;
use log::{debug, info, LevelFilter};
use std::error::Error;
use std::path::PathBuf;
use std::sync::Arc;
// use tokio::sync::mpsc;

use mpris_lyrics_rs::app::App;
use mpris_lyrics_rs::config::Config;
// use mpris_lyrics_rs::mpris::PlayerEvent;

#[derive(Parser, Debug)]
#[command(version, about)]
struct Args {
    #[arg(short, long)]
    config: Option<PathBuf>,

    #[arg(long, default_value_t = false)]
    debug: bool,

    #[arg(long, default_value_t = false, help = "不清除屏幕，保留日志输出")]
    no_clear: bool,

    #[arg(
        long,
        default_value_t = false,
        help = "简单输出模式，适用于外部程序集成（如waybar）"
    )]
    simple_output: bool,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    // 解析命令行参数
    let args = Args::parse();

    // 设置日志级别
    let log_level = if args.debug {
        LevelFilter::Debug
    } else {
        LevelFilter::Warn
    };

    // 初始化日志
    env_logger::Builder::new()
        .filter_level(log_level)
        .format_timestamp(None)
        .init();

    // 合并启动日志，避免过多输出
    info!(
        "MPRIS歌词显示器启动 {}",
        if args.debug {
            "（调试模式已启用）"
        } else {
            ""
        }
    );

    // 加载配置
    let config_path = args.config;
    debug!("尝试加载配置文件: {:?}", config_path);
    let mut config = Config::load(config_path)?;

    // 根据命令行参数调整显示配置
    if args.simple_output {
        config.display.simple_output = true;
        // 简单输出模式下，始终禁用进度条（不清屏）
        config.display.show_progress = false;
        debug!("已启用简单输出模式，适用于外部程序集成");
    } else {
        // 正常模式下，根据 no_clear 设置是否显示进度条（即是否清屏）
        config.display.show_progress = !args.no_clear;
        if args.no_clear {
            debug!("已启用不清屏模式，所有日志将保持可见");
        }
    }

    // 将配置详情日志降级为debug，只保留关键信息
    debug!("已加载配置，启用的歌词源: {:?}", config.lyrics_sources);
    if let Some(_) = &config.sources.netease {
        debug!("网易云音乐配置已加载");
    }

    if let Some(_) = &config.sources.qqmusic {
        debug!("QQ音乐配置已加载");
    }

    if let Some(local) = &config.sources.local {
        debug!("本地歌词配置: 路径={}", local.lyrics_path);
    }

    debug!("播放器黑名单: {:?}", config.player_blacklist);

    // 将 Config 放入 Arc 中共享
    let config = Arc::new(config);

    // 启动应用
    let mut app = App::new(Arc::clone(&config))?;
    info!("应用已启动，按Ctrl+C退出");
    app.run().await?;

    debug!("应用退出");
    Ok(())
}
