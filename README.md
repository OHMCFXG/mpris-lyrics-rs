# mpris-lyrics-rs

A lyrics display program for Linux that synchronizes with music players via MPRIS.

## Features

- Displays synchronized lyrics for currently playing music
- Supports multiple music players through MPRIS protocol
- Multiple lyrics sources supported (NetEase Music, QQ Music, local files)
- Terminal-based interface with progress bar
- Simple output mode for integration with external programs (e.g., waybar)

## Installation

### Prerequisites

- Rust and Cargo

### Building from source

```bash
git clone https://github.com/yourusername/mpris-lyrics-rs.git
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

Create a configuration file in one of these locations:
- `~/.config/mpris-lyrics-rs/config.toml`
- Custom location specified with `--config` option

Example configuration:

```toml
# Example configuration - refer to documentation for all options
[display]
show_progress = true
simple_output = false

[sources.netease]
# NetEase Music configuration

[sources.qqmusic]
# QQ Music configuration

[sources.local]
lyrics_path = "~/Music/Lyrics"
```

## License

[GPL v3](LICENSE)

## Contributing

Contributions are welcome! Please feel free to submit a Pull Request. 