use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::config::Config;
use crate::lyrics::{LyricLine, Lyrics, LyricsManager};
use crate::mpris::{PlaybackStatus, TrackInfo};
use crate::tui::theme::Theme;
use crate::tui::widgets::StatusInfo;

/// UI 状态
pub struct UiState {
    pub current_track: Option<TrackInfo>,
    pub current_player: Option<String>,
    pub current_position: u64,
    pub playback_status: PlaybackStatus,
    pub status_info: StatusInfo,
}

impl Default for UiState {
    fn default() -> Self {
        Self {
            current_track: None,
            current_player: None,
            current_position: 0,
            playback_status: PlaybackStatus::Stopped,
            status_info: StatusInfo::default(),
        }
    }
}

/// 渲染主界面（新设计）
pub fn render_ui(
    f: &mut Frame,
    config: &Config,
    lyrics_manager: &LyricsManager,
    ui_state: &UiState,
    theme: &Theme,
) {
    let size = f.area();

    // 创建主边框（标题在边框上）
    let main_title = if let Some(track) = &ui_state.current_track {
        format!("MPRIS 歌词同步器 - {}", track.title)
    } else {
        "MPRIS 歌词同步器".to_string()
    };

    let main_block = Block::default()
        .borders(Borders::ALL)
        .border_style(theme.border_style())
        .title(main_title)
        .title_style(theme.title_style());

    let inner_area = main_block.inner(size);
    f.render_widget(main_block, size);

    // 创建内部布局
    let inner_layout = create_inner_layout(inner_area);

    // 1. 渲染播放器和歌曲信息（合并）
    render_combined_info_bar(f, inner_layout[0], ui_state, theme);

    // 2. 渲染歌词面板（增加高度，垂直居中）
    let lyrics = lyrics_manager.get_current_lyrics();
    render_centered_lyrics(
        f,
        inner_layout[1],
        lyrics.as_ref(),
        ui_state.current_position + config.display.lyric_advance_time,
        config.display.context_lines,
        theme,
    );

    // 3. 渲染进度条
    render_progress_bar(f, inner_layout[2], ui_state, theme);

    // 4. 渲染操作提示栏
    render_help_bar(f, inner_layout[3], theme);
}

/// 创建内部布局（在主边框内）
fn create_inner_layout(area: Rect) -> Vec<Rect> {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(5), // 播放器和歌曲信息（合并，两行）
            Constraint::Min(10),   // 歌词面板（占据大部分空间，增加高度）
            Constraint::Length(3), // 进度条
            Constraint::Length(3), // 操作提示栏
        ])
        .split(area);

    chunks.to_vec()
}

/// 渲染合并的播放器和歌曲信息栏
fn render_combined_info_bar(f: &mut Frame, area: Rect, ui_state: &UiState, theme: &Theme) {
    // 第一行：播放器|状态|来源
    let player_name = ui_state.current_player.as_deref().unwrap_or("无播放器");
    let status_text = match ui_state.playback_status {
        PlaybackStatus::Playing => "播放中",
        PlaybackStatus::Paused => "已暂停",
        PlaybackStatus::Stopped => "已停止",
    };
    let source_text = ui_state
        .status_info
        .lyrics_source
        .as_deref()
        .unwrap_or("无来源");

    let status_line = Line::from(vec![
        Span::styled("播放器: ", theme.status_style()),
        Span::styled(player_name, theme.player_style()),
        Span::styled(" | 状态: ", theme.status_style()),
        Span::styled(status_text, theme.accent_style()),
        Span::styled(" | 来源: ", theme.status_style()),
        Span::styled(source_text, theme.accent_style()),
    ]);

    // 第二行：艺术家 - 歌曲 (专辑)
    let track_line = if let Some(track) = &ui_state.current_track {
        Line::from(vec![
            Span::styled(&track.artist, theme.text_style()),
            Span::styled(" - ", theme.status_style()),
            Span::styled(&track.title, theme.accent_style()),
            if !track.album.is_empty() {
                Span::styled(format!(" ({})", track.album), theme.status_style())
            } else {
                Span::raw("")
            },
        ])
    } else {
        Line::from(vec![Span::styled("等待播放音乐...", theme.dimmed_style())])
    };

    let content = vec![
        Line::from(""), // 顶部填充
        status_line,
        track_line,
        Line::from(""), // 底部填充
    ];

    let paragraph = Paragraph::new(content).alignment(Alignment::Center).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(theme.border_style()),
    );
    f.render_widget(paragraph, area);
}

/// 渲染居中歌词面板
fn render_centered_lyrics(
    f: &mut Frame,
    area: Rect,
    lyrics: Option<&Lyrics>,
    current_position_ms: u64,
    context_lines: usize,
    theme: &Theme,
) {
    let content = if let Some(lyrics) = lyrics {
        if lyrics.lines.is_empty() {
            create_empty_lyrics_display(area, "暂无歌词", theme)
        } else {
            create_centered_lyrics_lines(lyrics, current_position_ms, context_lines, area, theme)
        }
    } else {
        create_empty_lyrics_display(area, "正在加载歌词...", theme)
    };

    let paragraph = Paragraph::new(content).alignment(Alignment::Center).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(theme.border_style()),
    );
    f.render_widget(paragraph, area);
}

/// 创建居中的歌词行
fn create_centered_lyrics_lines<'a>(
    lyrics: &'a Lyrics,
    current_position_ms: u64,
    context_lines: usize,
    area: Rect,
    theme: &'a Theme,
) -> Vec<Line<'a>> {
    let current_index = find_current_lyric_index(&lyrics.lines, current_position_ms);
    let mut lines = Vec::new();

    // 计算可用高度（减去边框）
    let available_height = area.height.saturating_sub(2) as usize;

    // 动态调整显示行数，确保能显示更多歌词
    let max_display_lines = available_height.min(15); // 最多显示15行
    let actual_context = (context_lines * 2).max(6).min(max_display_lines / 2); // 至少显示6行上下文

    // 计算显示范围
    let start_index = current_index.saturating_sub(actual_context);
    let end_index = (current_index + actual_context + 1).min(lyrics.lines.len());
    let total_lyrics_lines = end_index - start_index;

    // 计算垂直居中需要的填充
    let top_padding = if total_lyrics_lines < available_height {
        (available_height - total_lyrics_lines) / 2
    } else {
        0
    };

    // 添加顶部填充空行
    for _ in 0..top_padding {
        lines.push(Line::from(""));
    }

    // 添加歌词行
    for i in start_index..end_index {
        let line = &lyrics.lines[i];
        let content = if i == current_index {
            Line::from(vec![
                Span::styled("♪ ", theme.accent_style()),
                Span::styled(&line.text, theme.current_line_style()),
            ])
        } else {
            Line::from(vec![Span::styled(&line.text, theme.dimmed_style())])
        };
        lines.push(content);
    }

    lines
}

/// 创建空歌词显示（垂直居中）
fn create_empty_lyrics_display<'a>(
    area: Rect,
    message: &'a str,
    theme: &'a Theme,
) -> Vec<Line<'a>> {
    let mut lines = Vec::new();
    let available_height = area.height.saturating_sub(2) as usize;

    // 垂直居中显示消息
    let top_padding = available_height / 2;
    for _ in 0..top_padding {
        lines.push(Line::from(""));
    }

    lines.push(Line::from(vec![Span::styled(
        message,
        theme.dimmed_style(),
    )]));

    lines
}

/// 渲染进度条
fn render_progress_bar(f: &mut Frame, area: Rect, ui_state: &UiState, theme: &Theme) {
    let content = if let Some(track) = &ui_state.current_track {
        create_progress_line(track, ui_state.current_position, theme)
    } else {
        Line::from(vec![
            Span::styled("░".repeat(50), theme.dimmed_style()),
            Span::styled(" 00:00 / 00:00", theme.status_style()),
        ])
    };

    let paragraph = Paragraph::new(content).alignment(Alignment::Center).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(theme.border_style()),
    );
    f.render_widget(paragraph, area);
}

/// 创建进度条行
fn create_progress_line<'a>(track: &'a TrackInfo, position_ms: u64, theme: &'a Theme) -> Line<'a> {
    let progress_width = 30; // 减少进度条宽度以适应时间显示
    let progress = if track.length_ms > 0 {
        (position_ms as f64 / track.length_ms as f64).min(1.0)
    } else {
        0.0
    };

    let filled_width = (progress * progress_width as f64) as usize;
    let filled = "█".repeat(filled_width);
    let empty = "░".repeat(progress_width - filled_width);

    Line::from(vec![
        Span::styled(format_time(position_ms), theme.text_style()),
        Span::styled(" ", theme.text_style()),
        Span::styled(filled, theme.progress_style()),
        Span::styled(empty, theme.dimmed_style()),
        Span::styled(" ", theme.text_style()),
        Span::styled(format_time(track.length_ms), theme.text_style()),
    ])
}

/// 渲染操作提示栏
fn render_help_bar(f: &mut Frame, area: Rect, theme: &Theme) {
    let help_line = Line::from(vec![
        Span::styled("Tab", theme.accent_style()),
        Span::styled(": 切换播放器 | ", theme.status_style()),
        Span::styled("R", theme.accent_style()),
        Span::styled(": 刷新歌词 | ", theme.status_style()),
        Span::styled("Q", theme.accent_style()),
        Span::styled(": 退出", theme.status_style()),
    ]);

    let paragraph = Paragraph::new(help_line)
        .alignment(Alignment::Center)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(theme.border_style()),
        );
    f.render_widget(paragraph, area);
}

/// 查找当前歌词索引
fn find_current_lyric_index(lines: &[LyricLine], current_position_ms: u64) -> usize {
    for (i, line) in lines.iter().enumerate() {
        if line.start_time <= current_position_ms {
            if let Some(end_time) = line.end_time {
                if current_position_ms < end_time {
                    return i;
                }
            } else if i + 1 < lines.len() {
                if current_position_ms < lines[i + 1].start_time {
                    return i;
                }
            } else {
                return i;
            }
        }
    }
    0
}

/// 格式化时间
fn format_time(ms: u64) -> String {
    let seconds = ms / 1000;
    let minutes = seconds / 60;
    let seconds = seconds % 60;
    format!("{:02}:{:02}", minutes, seconds)
}

/// 创建紧凑布局（小终端）
pub fn create_compact_layout(area: Rect) -> Vec<Rect> {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // 播放器信息
            Constraint::Length(1), // 进度条
            Constraint::Min(3),    // 歌词面板
            Constraint::Length(1), // 状态栏
        ])
        .split(area);

    chunks.to_vec()
}

/// 渲染帮助界面（覆盖显示）
pub fn render_help(f: &mut Frame, theme: &Theme) {
    let size = f.area();

    // 创建居中的帮助窗口
    let help_area = centered_rect(60, 70, size);

    let help_lines = vec![
        Line::from(vec![Span::styled(
            "MPRIS 歌词同步器 - 帮助",
            theme.title_style(),
        )]),
        Line::from(""),
        Line::from(vec![Span::styled("快捷键操作:", theme.accent_style())]),
        Line::from(vec![
            Span::styled("  Q", theme.accent_style()),
            Span::styled(" / ", theme.status_style()),
            Span::styled("Esc", theme.accent_style()),
            Span::styled("     退出程序", theme.text_style()),
        ]),
        Line::from(vec![
            Span::styled("  R", theme.accent_style()),
            Span::styled("           刷新歌词", theme.text_style()),
        ]),
        Line::from(vec![
            Span::styled("  Tab", theme.accent_style()),
            Span::styled("         切换播放器", theme.text_style()),
        ]),
        Line::from(vec![
            Span::styled("  H", theme.accent_style()),
            Span::styled(" / ", theme.status_style()),
            Span::styled("?", theme.accent_style()),
            Span::styled("       显示/隐藏帮助", theme.text_style()),
        ]),
        Line::from(""),
        Line::from(vec![Span::styled("支持的播放器:", theme.accent_style())]),
        Line::from(vec![Span::styled("  • Spotify", theme.text_style())]),
        Line::from(vec![Span::styled("  • VLC", theme.text_style())]),
        Line::from(vec![Span::styled("  • Rhythmbox", theme.text_style())]),
        Line::from(vec![Span::styled(
            "  • 其他支持 MPRIS 的播放器",
            theme.text_style(),
        )]),
        Line::from(""),
        Line::from(vec![Span::styled("按任意键关闭帮助", theme.dimmed_style())]),
    ];

    let help_paragraph = Paragraph::new(help_lines)
        .alignment(Alignment::Center)
        .block(
            Block::default()
                .title("帮助")
                .borders(Borders::ALL)
                .border_style(theme.accent_style()),
        );

    f.render_widget(help_paragraph, help_area);
}

/// 创建居中矩形
fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}
