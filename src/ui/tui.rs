use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use crossterm::{
    cursor::Show,
    event::{self, Event as CEvent, KeyCode, KeyEvent},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    prelude::Stylize,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Terminal,
};
use tokio::sync::broadcast;
use tokio::sync::watch;

use crate::config::Config;
use crate::events::{Event, EventHub, UiCommand};
use crate::state::{GlobalState, LyricsStatus, PlayerState};
use crate::ui::common;

pub struct TuiApp {
    config: Arc<Config>,
    hub: EventHub,
    state_rx: watch::Receiver<GlobalState>,
}

impl TuiApp {
    pub fn new(config: Arc<Config>, hub: EventHub, state_rx: watch::Receiver<GlobalState>) -> Self {
        Self {
            config,
            hub,
            state_rx,
        }
    }

    pub async fn run(self) -> Result<()> {
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen)?;
        let _terminal_guard = TerminalGuard;
        let backend = CrosstermBackend::new(stdout);
        let mut terminal = Terminal::new(backend)?;

        let shutdown_flag = Arc::new(AtomicBool::new(false));
        let input_hub = self.hub.clone();
        let input_shutdown = shutdown_flag.clone();
        let input_task = tokio::task::spawn_blocking(move || loop {
            if input_shutdown.load(Ordering::Relaxed) {
                break;
            }
            if event::poll(Duration::from_millis(100)).unwrap_or(false) {
                if let Ok(CEvent::Key(key)) = event::read() {
                    if handle_key(key, &input_hub) {
                        break;
                    }
                }
            }
        });

        let mut rx = self.hub.subscribe();
        let mut state_rx = self.state_rx.clone();
        let mut tick = tokio::time::interval(Duration::from_millis(250));
        let mut should_quit = false;

        render(&mut terminal, &self.config, &state_rx.borrow().clone())?;

        while !should_quit {
            tokio::select! {
                _ = tick.tick() => {
                    let snapshot = state_rx.borrow().clone();
                    if common::should_tick(&snapshot) {
                        render(&mut terminal, &self.config, &snapshot)?;
                    }
                }
                changed = state_rx.changed() => {
                    if changed.is_err() {
                        should_quit = true;
                        shutdown_flag.store(true, Ordering::Relaxed);
                        continue;
                    }
                    let snapshot = state_rx.borrow().clone();
                    render(&mut terminal, &self.config, &snapshot)?;
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
                                shutdown_flag.store(true, Ordering::Relaxed);
                            }
                        }
                        Event::Shutdown => {
                            should_quit = true;
                            shutdown_flag.store(true, Ordering::Relaxed);
                        }
                        _ => {}
                    }
                }
            }
        }

        shutdown_flag.store(true, Ordering::Relaxed);
        let _ = input_task.await;
        Ok(())
    }
}

fn handle_key(key: KeyEvent, hub: &EventHub) -> bool {
    match key.code {
        KeyCode::Char('q') | KeyCode::Esc => {
            hub.emit(Event::UiCommand {
                command: UiCommand::Quit,
            });
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
    let title = active_player_state(state)
        .and_then(|p| p.track.as_ref())
        .map(|t| {
            if t.artist.is_empty() {
                t.title.clone()
            } else {
                format!("{} - {}", t.title, t.artist)
            }
        })
        .unwrap_or_else(|| "no track".to_string());
    let (status_label, status_color) = active_player_state(state)
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
    let lines = match &state.lyrics.status {
        LyricsStatus::Ready => render_lyrics_lines(config, state, height),
        LyricsStatus::Loading => vec![Line::from("Searching lyrics...")],
        LyricsStatus::Failed(_) => vec![Line::from("Lyrics failed")],
        LyricsStatus::Idle => vec![Line::from("Lyrics idle")],
    };

    Paragraph::new(lines)
        .alignment(ratatui::layout::Alignment::Center)
        .block(Block::default().borders(Borders::ALL).title("Lyrics"))
}

fn active_position_ms(state: &GlobalState) -> u64 {
    active_player_state(state)
        .map(|p| p.estimate_position_ms())
        .unwrap_or(0)
}

fn render_progress(state: &GlobalState) -> Paragraph<'static> {
    let (pos_ms, total_ms) = active_player_state(state)
        .and_then(|p| {
            p.track
                .as_ref()
                .map(|t| (p.estimate_position_ms(), t.length_ms))
        })
        .unwrap_or((0, 0));

    let ratio = if total_ms > 0 {
        (pos_ms as f64 / total_ms as f64).min(1.0)
    } else {
        0.0
    };
    let marker = if PROGRESS_BAR_LEN == 0 {
        0
    } else {
        (ratio * (PROGRESS_BAR_LEN as f64 - 1.0)).round() as usize
    };
    let mut bar = String::with_capacity(PROGRESS_BAR_LEN);
    for idx in 0..PROGRESS_BAR_LEN {
        if idx == marker {
            bar.push('▮');
        } else {
            bar.push('─');
        }
    }
    let label = format!(
        "{} {} {}",
        common::format_time(pos_ms),
        bar,
        if total_ms > 0 {
            common::format_time(total_ms)
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

fn render_lyrics_lines(config: &Config, state: &GlobalState, height: usize) -> Vec<Line<'static>> {
    let Some(lyrics) = &state.lyrics.lyrics else {
        return vec![Line::from(Span::styled(
            "No lyrics",
            Style::default().fg(Color::DarkGray),
        ))];
    };

    let highlight = parse_color(&config.display.current_line_color);
    let highlight_style = Style::default().fg(highlight).bold();
    let context = config.display.context_lines.max(2);
    let gradient = build_lyric_gradient(context);

    let position_ms = active_position_ms(state);
    let position_with_advance = position_ms + config.display.lyric_advance_time_ms;
    let (current_index, _) = common::find_line_index(lyrics, position_with_advance);
    let start = current_index.saturating_sub(context);
    let end = (current_index + context + 1).min(lyrics.lines.len());

    let mut content: Vec<Line<'static>> = Vec::new();
    for idx in start..end {
        let line = &lyrics.lines[idx];
        let text = if config.display.show_timestamp {
            format!(
                "[{}] {}",
                common::format_time(line.start_time_ms),
                line.text
            )
        } else {
            line.text.clone()
        };
        let distance = if idx > current_index {
            idx - current_index
        } else {
            current_index - idx
        };

        if distance == 0 {
            content.push(Line::from(""));
        }

        let (prefix, style) = if distance == 0 {
            ("> ", highlight_style)
        } else {
            let style = gradient[(distance.min(context)) - 1];
            ("  ", style)
        };
        content.push(Line::from(vec![
            Span::styled(prefix, style),
            Span::styled(text, style),
        ]));

        if distance == 0 {
            content.push(Line::from(""));
        }
    }

    let total = content.len();
    let pad = height.saturating_sub(total) / 2;
    let mut lines = Vec::with_capacity(pad + total);
    for _ in 0..pad {
        lines.push(Line::from(""));
    }
    lines.extend(content);
    lines
}

fn build_lyric_gradient(context: usize) -> Vec<Style> {
    let steps = context.max(1);
    let mut styles = Vec::with_capacity(steps);
    for distance in 1..=steps {
        let val = gradient_gray(distance, steps);
        let mut style = Style::default().fg(Color::Rgb(val, val, val));
        if distance >= 2 {
            style = style.add_modifier(Modifier::DIM);
        }
        styles.push(style);
    }
    styles
}

fn gradient_gray(distance: usize, steps: usize) -> u8 {
    let max: i32 = 180;
    let min: i32 = 70;
    let range = max - min;
    let clamped = distance.min(steps) as i32;
    let val = max - (range * clamped) / steps as i32;
    val.clamp(min, max) as u8
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

fn active_player_state(state: &GlobalState) -> Option<&PlayerState> {
    state
        .active_player
        .as_ref()
        .and_then(|player| state.players.get(player))
}

const PROGRESS_BAR_LEN: usize = 30;

struct TerminalGuard;

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let mut stdout = io::stdout();
        let _ = execute!(stdout, LeaveAlternateScreen, Show);
    }
}
