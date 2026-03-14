# Architecture Design (Rewrite)

## Goals
- Fully async event-driven MPRIS monitoring with accurate state tracking.
- Reduce polling to a low-frequency fallback only.
- Keep simple-output mode and TUI mode.
- Preserve TUI manual player switching.
- Remove local lyrics support.
- No persistent lyrics cache (allow in-flight request dedup only).

## Non-Goals
- No GUI frontend beyond TUI and simple-output.
- No local lyrics files.
- No persistent cache on disk.

## Feature Scope
- MPRIS player discovery and lifecycle tracking.
- Playback state, track metadata, position tracking.
- Lyrics fetching from online sources (NetEase, QQ).
- TUI UI with manual player switching.
- Simple-output mode for status/lyrics integration.
- Config file loading and CLI overrides.

## High-Level Architecture
- `dbus::conn` manages the zbus connection and reconnect.
- `mpris::registry` tracks `org.mpris.MediaPlayer2.*` names and emits player lifecycle events.
- `mpris::player_actor` handles a single player, subscribes to signals, and normalizes state.
- `state::store` is the single source of truth for all players and active player.
- `policy::active_player` decides active player with manual override support.
- `lyrics::service` fetches lyrics on demand with in-flight dedup only.
- `ui::tui` renders event-driven UI and handles manual switching.
- `ui::simple` prints minimal updates for external consumers.

## Library Choices
- Async runtime: `tokio`.
- D-Bus: `zbus`.
- Logging: `tracing` + `tracing-subscriber`.
- String similarity: `strsim`.
- LRC parsing: `lrc` crate.
- HTTP client: `reqwest`.
- CLI: `clap`.
- TUI: `ratatui` + `crossterm`.
- Serialization: `serde` + `toml`.

## Event Model
- `PlayerAppeared { player }`
- `PlayerDisappeared { player }`
- `PlaybackStatusChanged { player, status }`
- `TrackChanged { player, track }`
- `Seeked { player, position_ms }`
- `PositionUpdated { player, position_ms }` (rate-limited)
- `ActivePlayerChanged { player, reason }`
- `LyricsRequested { track_key }`
- `LyricsUpdated { track_key, lyrics }`
- `LyricsFailed { track_key, error }`
- `UiCommand { kind }` (switch player, toggle help, quit)

## State Model
Per-player state:
- `playback_status`
- `track` (title, artist, album, length_ms, track_id)
- `position_ms`, `position_ts`, `rate`
- `last_seen`

Global state:
- `active_player`
- `manual_override` (TUI only)
- `lyrics_state` (current lyrics and status for the active player)

## MPRIS Integration (zbus)
- Use `DBusProxy` to list names and subscribe to `NameOwnerChanged`.
- For each player name, create `PlayerProxy` and subscribe to signals.
Signals:
- `PropertiesChanged` for `PlaybackStatus`, `Metadata`, `Rate`, `Position`.
- `Seeked` for precise position jumps.

Behavior:
- Initial sync on actor start to populate state.
- Optional low-frequency fallback sync for players that miss signals.

## Position Accuracy Strategy
Derived position formula (when `PlaybackStatus == Playing`):
- `position_ms + (now - position_ts) * rate`

Rules:
- `Seeked` and `TrackChanged` force a hard sync.
- A low-frequency correction timer re-reads `Position` for drift.

## Active Player Policy
- Default: prefer `Playing` > `Paused` > `Stopped`.
- TUI manual mode can override active player selection.
- Manual override is cleared when the selected player disappears.

## Lyrics Service
- Triggered by `TrackChanged` of active player.
- Online providers in priority order: NetEase, QQ.
- No persistent cache; allow in-flight dedup by track key only.
- Emit `LyricsUpdated` or `LyricsFailed`.

## UI Behavior
- Event-driven refresh only.
- Redraw on track change, status change, lyrics change, or line transition.
- Simple-output only prints when content changes.

## Configuration
- `display.show_progress`
- `display.simple_output`
- `display.enable_tui`
- `display.show_timestamp`
- `display.context_lines`
- `display.current_line_color`
- `display.lyric_advance_time_ms`
- `mpris.fallback_sync_interval_seconds`
- `players.blacklist`
- `sources.netease`
- `sources.qqmusic`

## CLI
- `--config PATH`
- `--debug`
- `--no-clear`
- `--simple-output`

## Error Handling
- Per-actor errors should not crash the process.
- Reconnect on D-Bus failures with backoff.
- Provider failures should fall through to next provider.

## Testing Strategy
- Unit tests for track matching and LRC parsing.
- Integration tests for state transitions using mocked events.
- Optional manual test checklist for common players.
