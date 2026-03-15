use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use clap::Parser;
use tracing_subscriber::EnvFilter;

use mpris_lyrics_rs::app::App;
use mpris_lyrics_rs::config::{CliOverrides, Config};

#[derive(Parser, Debug)]
#[command(version, about)]
struct Args {
    #[arg(short, long)]
    config: Option<PathBuf>,

    #[arg(long, default_value_t = false)]
    debug: bool,

    #[arg(
        long,
        default_value_t = false,
        help = "do not clear screen; keep logs visible"
    )]
    no_clear: bool,

    #[arg(
        long,
        default_value_t = false,
        help = "simple output mode for integrations"
    )]
    simple_output: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let mut config = Config::load(args.config)?;
    let overrides = CliOverrides {
        simple_output: args.simple_output,
        no_clear: args.no_clear,
    };
    config.apply_cli(&overrides);

    if config.display.simple_output {
        let filter = if args.debug { "debug" } else { "error" };
        tracing_subscriber::fmt()
            .with_env_filter(EnvFilter::new(filter))
            .init();
    } else if !config.display.enable_tui {
        let filter = if args.debug { "debug" } else { "info" };
        tracing_subscriber::fmt()
            .with_env_filter(EnvFilter::new(filter))
            .init();
    } else {
        // TUI mode: sink all logs to avoid corrupting the UI.
        tracing_subscriber::fmt()
            .with_env_filter(EnvFilter::new("off"))
            .with_writer(std::io::sink)
            .init();
    }

    let app = App::new(Arc::new(config));
    app.run().await
}
