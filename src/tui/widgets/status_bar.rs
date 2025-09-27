use ratatui::{
    layout::Rect,
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::tui::theme::Theme;

/// 状态栏信息
#[derive(Debug, Clone)]
pub struct StatusInfo {
    pub lyrics_source: Option<String>,
    pub source_status: SourceStatus,
    pub network_delay: Option<u64>,
    pub shortcuts_enabled: bool,
}

/// 歌词源状态
#[derive(Debug, Clone, PartialEq)]
pub enum SourceStatus {
    Success,
    Loading,
    Failed,
    None,
}

/// 状态栏组件
pub struct StatusBar<'a> {
    status_info: &'a StatusInfo,
    theme: &'a Theme,
}

impl<'a> StatusBar<'a> {
    pub fn new(status_info: &'a StatusInfo, theme: &'a Theme) -> Self {
        Self {
            status_info,
            theme,
        }
    }

    /// 渲染状态栏
    pub fn render(&self, f: &mut Frame, area: Rect) {
        let status_line = self.create_status_line();
        let paragraph = Paragraph::new(status_line);
        f.render_widget(paragraph, area);
    }

    /// 创建状态栏内容
    fn create_status_line(&self) -> Line<'_> {
        let mut spans = Vec::new();

        // 歌词源状态
        self.add_lyrics_source_status(&mut spans);

        // 分隔符
        if self.status_info.network_delay.is_some() {
            spans.push(Span::styled(" │ ", self.theme.status_style()));
            self.add_network_delay(&mut spans);
        }

        // 快捷键提示
        if self.status_info.shortcuts_enabled {
            spans.push(Span::styled(" │ ", self.theme.status_style()));
            self.add_shortcuts(&mut spans);
        }

        Line::from(spans)
    }

    /// 添加歌词源状态
    fn add_lyrics_source_status(&self, spans: &mut Vec<Span<'a>>) {
        if let Some(source) = &self.status_info.lyrics_source {
            spans.push(Span::styled(source, self.theme.text_style()));
            spans.push(Span::styled(" ", self.theme.text_style()));
            
            let (symbol, style) = match self.status_info.source_status {
                SourceStatus::Success => ("✓", self.theme.accent_style()),
                SourceStatus::Loading => ("⟳", self.theme.status_style()),
                SourceStatus::Failed => ("✗", self.theme.status_style()),
                SourceStatus::None => ("○", self.theme.status_style()),
            };
            
            spans.push(Span::styled(symbol, style));
        } else {
            spans.push(Span::styled("无来源", self.theme.status_style()));
        }
    }

    /// 添加网络延迟信息
    fn add_network_delay(&self, spans: &mut Vec<Span<'a>>) {
        if let Some(delay) = self.status_info.network_delay {
            spans.push(Span::styled(
                format!("{}ms", delay),
                if delay < 100 {
                    self.theme.accent_style()
                } else if delay < 500 {
                    self.theme.text_style()
                } else {
                    self.theme.status_style()
                }
            ));
        }
    }

    /// 添加快捷键提示
    fn add_shortcuts(&self, spans: &mut Vec<Span<'a>>) {
        let shortcuts = vec![
            ("[h]", "帮助"),
            ("[q]", "退出"),
            ("[r]", "刷新"),
        ];

        for (i, (key, desc)) in shortcuts.iter().enumerate() {
            if i > 0 {
                spans.push(Span::styled(" ", self.theme.text_style()));
            }
            spans.push(Span::styled(*key, self.theme.accent_style()));
            spans.push(Span::styled(*desc, self.theme.text_style()));
        }
    }
}

impl Default for StatusInfo {
    fn default() -> Self {
        Self {
            lyrics_source: None,
            source_status: SourceStatus::None,
            network_delay: None,
            shortcuts_enabled: true,
        }
    }
}
