use ratatui::{
    layout::Rect,
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::mpris::TrackInfo;
use crate::tui::theme::Theme;

/// 播放器信息组件
pub struct PlayerInfo<'a> {
    track: Option<&'a TrackInfo>,
    player_name: Option<&'a str>,
    theme: &'a Theme,
}

impl<'a> PlayerInfo<'a> {
    pub fn new(
        track: Option<&'a TrackInfo>,
        player_name: Option<&'a str>,
        theme: &'a Theme,
    ) -> Self {
        Self {
            track,
            player_name,
            theme,
        }
    }

    /// 渲染播放器信息
    pub fn render(&self, f: &mut Frame, area: Rect) {
        // 创建边框块
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(self.theme.border_style())
            .title(self.create_title());

        let inner = block.inner(area);
        f.render_widget(block, area);

        // 如果有歌曲信息，显示详细信息
        if let Some(track) = self.track {
            self.render_track_info(f, inner, track);
        } else {
            self.render_no_track(f, inner);
        }
    }

    /// 创建标题
    fn create_title(&self) -> Line<'_> {
        let mut spans = vec![Span::styled("MPRIS Lyrics", self.theme.title_style())];

        if let Some(player_name) = self.player_name {
            spans.push(Span::styled(" ─ ", self.theme.border_style()));
            spans.push(Span::styled(player_name, self.theme.accent_style()));
            spans.push(Span::styled(" ●", self.theme.accent_style())); // 活跃指示器
        } else {
            spans.push(Span::styled(" ○", self.theme.status_style())); // 非活跃指示器
        }

        Line::from(spans)
    }

    /// 渲染歌曲信息
    fn render_track_info(&self, f: &mut Frame, area: Rect, track: &TrackInfo) {
        // 创建紧凑的单行显示
        let track_line = self.create_compact_track_line(track);

        let paragraph = Paragraph::new(track_line).style(self.theme.text_style());

        f.render_widget(paragraph, area);
    }

    /// 创建紧凑的歌曲信息行
    fn create_compact_track_line<'b>(&self, track: &'b TrackInfo) -> Line<'b> {
        let mut spans = Vec::new();

        // 歌名
        spans.push(Span::styled(&track.title, self.theme.accent_style()));

        // 分隔符
        spans.push(Span::styled(" • ", self.theme.status_style()));

        // 艺术家
        spans.push(Span::styled(&track.artist, self.theme.text_style()));

        // 如果有专辑信息且不为空，添加专辑
        if !track.album.trim().is_empty() && track.album != track.title {
            spans.push(Span::styled(" • ", self.theme.status_style()));
            spans.push(Span::styled(&track.album, self.theme.status_style()));
        }

        Line::from(spans)
    }

    /// 渲染无歌曲状态
    fn render_no_track(&self, f: &mut Frame, area: Rect) {
        let message = if self.player_name.is_some() {
            "没有正在播放的歌曲"
        } else {
            "🎵 等待播放器连接..."
        };

        let paragraph = Paragraph::new(Line::from(vec![Span::styled(
            message.to_string(),
            self.theme.status_style(),
        )]));

        f.render_widget(paragraph, area);
    }
}
