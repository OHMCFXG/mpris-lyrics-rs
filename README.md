# mpris-lyrics-rs

A lyrics display program for Linux that synchronizes with music players via MPRIS.

## Features

- Displays synchronized lyrics for currently playing music
- Supports multiple music players through MPRIS
- Multiple lyrics sources supported (NetEase Music, QQ Music)
- TUI with centered lyrics, status header, and progress bar
- Simple output mode for integration with external programs (e.g., waybar)

## Installation

### Prerequisites

- Rust and Cargo

### Building from source

```bash
git clone https://github.com/OHMCFXG/mpris-lyrics-rs.git
cd mpris-lyrics-rs
cargo build --release
```

The compiled binary will be available at `target/release/mpris-lyrics-rs`.

## Usage

```bash
# Run with default settings
mpris-lyrics-rs

# Run with custom config file
mpris-lyrics-rs --config /path/to/config.toml

# Run in debug mode
mpris-lyrics-rs --debug

# Run without clearing the screen (keeping log output)
mpris-lyrics-rs --no-clear

# Run in simple output mode (for integration with external programs)
mpris-lyrics-rs --simple-output
```

## Configuration

A default configuration file will be automatically generated at `~/.config/mpris-lyrics-rs/config.toml` on the first run.

You can place your configuration file in one of these locations:
- `~/.config/mpris-lyrics-rs/config.toml`
- Custom location specified with `--config` option

Example configuration:

```toml
[display]
show_timestamp = false
show_progress = true
context_lines = 2
current_line_color = "green"
simple_output = false
enable_tui = true
lyric_advance_time_ms = 300

[mpris]
fallback_sync_interval_seconds = 5

[sources.netease]
# NetEase Music configuration

[sources.qqmusic]
# QQ Music configuration

[players]
blacklist = ["firefox", "mozilla", "chromium", "chrome", "kdeconnect"]
```

## License

[GPL v3](LICENSE)

## Contributing

Contributions are welcome! Please feel free to submit a Pull Request. 
