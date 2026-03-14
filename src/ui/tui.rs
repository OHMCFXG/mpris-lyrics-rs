use std::io;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use crossterm::{
    event::{self, Event as CEvent, KeyCode, KeyEvent},
    cursor::Show,
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Style},
    prelude::Stylize,
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Terminal,
};
use tokio::sync::broadcast;

use crate::config::Config;
use crate::events::{Event, EventHub, UiCommand};
use crate::lyrics::Lyrics;
use crate::state::{GlobalState, LyricsStatus, StateStore};

pub struct TuiApp {
    config: Arc<Config>,
    hub: EventHub,
    store: Arc<StateStore>,
}

impl TuiApp {
    pub fn new(config: Arc<Config>, hub: EventHub, store: Arc<StateStore>) -> Self {
        Self { config, hub, store }
    }

    pub async fn run(self) -> Result<()> {
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen)?;
        let _terminal_guard = TerminalGuard;
        let backend = CrosstermBackend::new(stdout);
        let mut terminal = Terminal::new(backend)?;

        let input_hub = self.hub.clone();
        let input_task = tokio::task::spawn_blocking(move || {
            loop {
                if event::poll(Duration::from_millis(100)).unwrap_or(false) {
                    if let Ok(CEvent::Key(key)) = event::read() {
                        if handle_key(key, &input_hub) {
                            break;
                        }
                    }
                }
            }
        });

        let mut rx = self.hub.subscribe();
        let mut tick = tokio::time::interval(Duration::from_millis(250));
        let mut should_quit = false;

        render(&mut terminal, &self.config, &self.store.snapshot().await)?;

        while !should_quit {
            tokio::select! {
                _ = tick.tick() => {
                    let snapshot = self.store.snapshot().await;
                    if should_tick(&snapshot) {
                        render(&mut terminal, &self.config, &snapshot)?;
                    }
                }
                event = rx.recv() => {
                    let event = match event {
                        Ok(ev) => ev,
                        Err(broadcast::error::RecvError::Closed) => break,
                        Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    };

                    match event {
                        Event::UiCommand { command } => {
                            if matches!(command, UiCommand::Quit) {
                                should_quit = true;
                            }
                        }
                        Event::TrackChanged { .. }
                        | Event::PlaybackStatusChanged { .. }
                        | Event::LyricsUpdated { .. }
                        | Event::LyricsFailed { .. }
                        | Event::ActivePlayerChanged { .. } => {
                            let snapshot = self.store.snapshot().await;
                            render(&mut terminal, &self.config, &snapshot)?;
                        }
                        _ => {}
                    }
                }
            }
        }

        let _ = input_task.await;
        Ok(())
    }
}

fn handle_key(key: KeyEvent, hub: &EventHub) -> bool {
    match key.code {
        KeyCode::Char('q') | KeyCode::Esc => {
            hub.emit(Event::UiCommand { command: UiCommand::Quit });
            true
        }
        KeyCode::Tab => {
            hub.emit(Event::UiCommand {
                command: UiCommand::SelectNextPlayer,
            });
            false
        }
        _ => false,
    }
}

fn render(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    config: &Config,
    state: &GlobalState,
) -> Result<()> {
    terminal.draw(|f| {
        let size = f.area();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(4),
            Constraint::Min(6),
            Constraint::Length(3),
            Constraint::Length(3),
        ])
        .split(size);

    let header = render_header(state);
    f.render_widget(header, chunks[0]);

    let body = render_body(config, state, chunks[1].height as usize);
    f.render_widget(body, chunks[1]);

    let progress = render_progress(state);
    f.render_widget(progress, chunks[2]);

    let help = render_help();
    f.render_widget(help, chunks[3]);
    })?;
    Ok(())
}

fn render_header(state: &GlobalState) -> Paragraph<'static> {
    let player = state
        .active_player
        .as_deref()
        .map(format_player_name)
        .unwrap_or_else(|| "no player".to_string());
    let title = state
        .active_player
        .as_ref()
        .and_then(|p| state.players.get(p))
        .and_then(|p| p.track.as_ref())
        .map(|t| {
            if t.artist.is_empty() {
                t.title.clone()
            } else {
                format!("{} - {}", t.title, t.artist)
            }
        })
        .unwrap_or_else(|| "no track".to_string());
    let (status_label, status_color) = state
        .active_player
        .as_ref()
        .and_then(|p| state.players.get(p))
        .map(|p| match p.playback_status {
            crate::state::PlaybackStatus::Playing => ("Playing", Color::Green),
            crate::state::PlaybackStatus::Paused => ("Paused", Color::Yellow),
            crate::state::PlaybackStatus::Stopped => ("Stopped", Color::Red),
        })
        .unwrap_or(("Stopped", Color::Red));
    let source = match &state.lyrics.status {
        LyricsStatus::Ready => state
            .lyrics
            .lyrics
            .as_ref()
            .map(|l| l.metadata.source.clone())
            .unwrap_or_else(|| "unknown".to_string()),
        LyricsStatus::Loading => "searching".to_string(),
        LyricsStatus::Failed(_) => "failed".to_string(),
        LyricsStatus::Idle => "idle".to_string(),
    };

    let title_line = Line::from(vec![
        Span::styled("♪ ", Style::default().fg(Color::Cyan)),
        Span::styled(title, Style::default().bold().fg(Color::White)),
    ]);
    let meta = Line::from(vec![
        Span::styled("Player: ", Style::default().fg(Color::DarkGray)),
        Span::raw(player),
        Span::raw("        "),
        Span::styled("Status: ", Style::default().fg(Color::DarkGray)),
        Span::styled("●", Style::default().fg(status_color).bold()),
        Span::raw(" "),
        Span::styled(status_label, Style::default().fg(status_color)),
        Span::raw("   "),
        Span::styled("Source: ", Style::default().fg(Color::DarkGray)),
        Span::styled(source, Style::default().fg(Color::Gray)),
    ]);

    Paragraph::new(vec![title_line, meta])
        .block(Block::default().borders(Borders::ALL).title("Status"))
}

fn render_body(config: &Config, state: &GlobalState, height: usize) -> Paragraph<'static> {
    let mut lines: Vec<Line<'static>> = Vec::new();
    match &state.lyrics.status {
        LyricsStatus::Ready => {
            if let Some(lyrics) = &state.lyrics.lyrics {
                let position_ms = active_position_ms(state);
                let position_with_advance = position_ms + config.display.lyric_advance_time_ms;
                let (current_index, _) = find_line_index(lyrics, position_with_advance);
                let context = config.display.context_lines.max(2);
                let start = current_index.saturating_sub(context);
                let end = (current_index + context + 1).min(lyrics.lines.len());
                let mut content: Vec<Line<'static>> = Vec::new();
                for idx in start..end {
                    let line = &lyrics.lines[idx];
                    let text = if config.display.show_timestamp {
                        format!("[{}] {}", format_time(line.start_time_ms), line.text)
                    } else {
                        line.text.clone()
                    };
                    let distance = if idx > current_index {
                        idx - current_index
                    } else {
                        current_index - idx
                    };
                    let (prefix, style) = if distance == 0 {
                        (
                            "> ",
                            Style::default()
                                .fg(parse_color(&config.display.current_line_color))
                                .bold(),
                        )
                    } else if distance == 1 {
                        ("  ", Style::default().fg(Color::DarkGray))
                    } else if distance == 2 {
                        ("  ", Style::default().fg(Color::Rgb(80, 80, 80)))
                    } else {
                        ("  ", Style::default().fg(Color::Rgb(60, 60, 60)))
                    };
                    content.push(Line::from(vec![
                        Span::styled(prefix, style),
                        Span::styled(text, style),
                    ]));
                }
                let total = content.len();
                let pad = height.saturating_sub(total) / 2;
                for _ in 0..pad {
                    lines.push(Line::from(""));
                }
                lines.extend(content);
            } else {
                lines.push(Line::from(Span::styled(
                    "No lyrics",
                    Style::default().fg(Color::DarkGray),
                )));
            }
        }
        LyricsStatus::Loading => lines.push(Line::from("Searching lyrics...")),
        LyricsStatus::Failed(_) => lines.push(Line::from("Lyrics failed")),
        LyricsStatus::Idle => lines.push(Line::from("Lyrics idle")),
    }

    Paragraph::new(lines)
        .alignment(ratatui::layout::Alignment::Center)
        .block(Block::default().borders(Borders::ALL).title("Lyrics"))
}

fn active_position_ms(state: &GlobalState) -> u64 {
    let Some(player) = &state.active_player else { return 0; };
    let Some(player_state) = state.players.get(player) else { return 0; };
    player_state.estimate_position_ms()
}

fn find_line_index(lyrics: &Lyrics, time_ms: u64) -> (usize, Option<&crate::lyrics::LyricLine>) {
    if lyrics.lines.is_empty() {
        return (0, None);
    }
    let idx = lyrics
        .lines
        .partition_point(|line| line.start_time_ms <= time_ms);
    let index = if idx == 0 { 0 } else { idx - 1 };
    (index, lyrics.lines.get(index))
}

fn render_progress(state: &GlobalState) -> Paragraph<'static> {
    let (pos_ms, total_ms) = match state.active_player.as_ref() {
        Some(player) => state
            .players
            .get(player)
            .and_then(|p| p.track.as_ref().map(|t| (p.estimate_position_ms(), t.length_ms)))
            .unwrap_or((0, 0)),
        None => (0, 0),
    };

    let ratio = if total_ms > 0 {
        (pos_ms as f64 / total_ms as f64).min(1.0)
    } else {
        0.0
    };
    let bar_len = 30usize;
    let marker = if bar_len == 0 {
        0
    } else {
        (ratio * (bar_len as f64 - 1.0)).round() as usize
    };
    let mut bar = String::new();
    for idx in 0..bar_len {
        if idx == marker {
            bar.push('▮');
        } else {
            bar.push('─');
        }
    }
    let label = format!(
        "{} {} {}",
        format_time(pos_ms),
        bar,
        if total_ms > 0 {
            format_time(total_ms)
        } else {
            "--:--".to_string()
        }
    );

    Paragraph::new(Line::from(Span::styled(
        label,
        Style::default().fg(Color::Gray),
    )))
        .alignment(ratatui::layout::Alignment::Center)
        .block(Block::default().borders(Borders::ALL).title("Progress"))
}

fn render_help() -> Paragraph<'static> {
    let line = Line::from(vec![
        Span::styled("Q / Esc ", Style::default().bold()),
        Span::raw("Quit  "),
        Span::styled("Tab ", Style::default().bold()),
        Span::raw("Next Player"),
    ]);
    Paragraph::new(vec![line])
        .alignment(ratatui::layout::Alignment::Center)
        .block(Block::default().borders(Borders::ALL).title("Help"))
}

fn format_time(ms: u64) -> String {
    let total_seconds = ms / 1000;
    let minutes = total_seconds / 60;
    let seconds = total_seconds % 60;
    format!("{:02}:{:02}", minutes, seconds)
}

fn format_player_name(player: &str) -> String {
    let mut name = player;
    const PREFIX: &str = "org.mpris.MediaPlayer2.";
    if let Some(stripped) = name.strip_prefix(PREFIX) {
        name = stripped;
    }
    if let Some((base, _)) = name.split_once(".instance") {
        name = base;
    }
    let name = name.trim_matches('.');
    if name.is_empty() {
        return player.to_string();
    }
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return player.to_string();
    };
    if first.is_ascii_lowercase() {
        let mut out = String::new();
        out.push(first.to_ascii_uppercase());
        out.push_str(chars.as_str());
        return out;
    }
    name.to_string()
}

fn parse_color(name: &str) -> Color {
    match name.to_lowercase().as_str() {
        "red" => Color::Red,
        "green" => Color::Green,
        "yellow" => Color::Yellow,
        "blue" => Color::Blue,
        "magenta" => Color::Magenta,
        "cyan" => Color::Cyan,
        "white" => Color::White,
        _ => Color::Green,
    }
}

fn should_tick(state: &GlobalState) -> bool {
    let Some(active) = &state.active_player else { return false; };
    let Some(player_state) = state.players.get(active) else { return false; };
    player_state.playback_status == crate::state::PlaybackStatus::Playing
}

struct TerminalGuard;

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let mut stdout = io::stdout();
        let _ = execute!(stdout, LeaveAlternateScreen, Show);
    }
}
