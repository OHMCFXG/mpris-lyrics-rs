use ratatui::{
    layout::Rect,
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::display;
use crate::mpris::{PlaybackStatus, TrackInfo};
use crate::tui::theme::Theme;

/// 进度条组件
pub struct ProgressBar<'a> {
    track: Option<&'a TrackInfo>,
    position_ms: u64,
    status: &'a PlaybackStatus,
    theme: &'a Theme,
}

impl<'a> ProgressBar<'a> {
    pub fn new(
        track: Option<&'a TrackInfo>,
        position_ms: u64,
        status: &'a PlaybackStatus,
        theme: &'a Theme,
    ) -> Self {
        Self {
            track,
            position_ms,
            status,
            theme,
        }
    }

    /// 渲染进度条
    pub fn render(&self, f: &mut Frame, area: Rect) {
        if let Some(track) = self.track {
            let progress_line = self.create_progress_line(track);
            let paragraph = Paragraph::new(progress_line);
            f.render_widget(paragraph, area);
        }
    }

    /// 创建进度条行
    fn create_progress_line(&self, track: &TrackInfo) -> Line<'_> {
        let mut spans = Vec::new();

        // 计算进度条
        let progress_bar = self.create_progress_bar_chars(track);
        spans.extend(progress_bar);

        // 添加时间信息
        spans.push(Span::styled(" ", self.theme.text_style()));
        spans.push(Span::styled(
            display::format_time(self.position_ms),
            self.theme.text_style(),
        ));
        spans.push(Span::styled("/", self.theme.status_style()));
        spans.push(Span::styled(
            display::format_time(track.length_ms),
            self.theme.text_style(),
        ));

        // 添加播放状态
        spans.push(Span::styled(" [", self.theme.status_style()));
        spans.push(Span::styled(
            self.get_status_symbol(),
            self.theme.accent_style(),
        ));
        spans.push(Span::styled(" ", self.theme.text_style()));
        spans.push(Span::styled(
            self.get_status_text(),
            self.theme.text_style(),
        ));
        spans.push(Span::styled("]", self.theme.status_style()));

        Line::from(spans)
    }

    /// 创建进度条字符
    fn create_progress_bar_chars(&self, track: &TrackInfo) -> Vec<Span<'_>> {
        let mut spans = Vec::new();
        let total_width = 20; // 进度条总宽度

        if track.length_ms == 0 {
            // 如果总长度为0，显示空进度条
            spans.push(Span::styled(
                "░".repeat(total_width),
                self.theme.status_style(),
            ));
            return spans;
        }

        // 计算进度
        let progress = (self.position_ms as f64 / track.length_ms as f64).min(1.0);
        let filled_width = (progress * total_width as f64) as usize;

        // 填充部分
        if filled_width > 0 {
            spans.push(Span::styled(
                "█".repeat(filled_width.saturating_sub(1)),
                self.theme.progress_style(),
            ));
            // 播放头
            spans.push(Span::styled("▶", self.theme.current_line_style()));
        } else {
            spans.push(Span::styled("▶", self.theme.current_line_style()));
        }

        // 未填充部分
        let remaining = total_width.saturating_sub(filled_width.max(1));
        if remaining > 0 {
            spans.push(Span::styled(
                "░".repeat(remaining),
                self.theme.status_style(),
            ));
        }

        spans
    }

    /// 获取状态符号
    fn get_status_symbol(&self) -> &'static str {
        match self.status {
            PlaybackStatus::Playing => "▶",
            PlaybackStatus::Paused => "⏸",
            PlaybackStatus::Stopped => "⏹",
        }
    }

    /// 获取状态文字
    fn get_status_text(&self) -> &'static str {
        match self.status {
            PlaybackStatus::Playing => "播放中",
            PlaybackStatus::Paused => "已暂停",
            PlaybackStatus::Stopped => "已停止",
        }
    }
}
