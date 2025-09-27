use ratatui::{
    layout::Rect,
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::lyrics::{LyricLine, Lyrics};
use crate::tui::theme::Theme;

/// 歌词面板组件
pub struct LyricsPanel<'a> {
    lyrics: Option<&'a Lyrics>,
    current_position_ms: u64,
    context_lines: usize,
    theme: &'a Theme,
}

impl<'a> LyricsPanel<'a> {
    pub fn new(
        lyrics: Option<&'a Lyrics>,
        current_position_ms: u64,
        context_lines: usize,
        theme: &'a Theme,
    ) -> Self {
        Self {
            lyrics,
            current_position_ms,
            context_lines,
            theme,
        }
    }

    /// 渲染歌词面板
    pub fn render(&self, f: &mut Frame, area: Rect) {
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(self.theme.border_style());

        let inner = block.inner(area);
        f.render_widget(block, area);

        if let Some(lyrics) = self.lyrics {
            self.render_lyrics(f, inner, lyrics);
        } else {
            self.render_no_lyrics(f, inner);
        }
    }

    /// 渲染歌词内容
    fn render_lyrics(&self, f: &mut Frame, area: Rect, lyrics: &Lyrics) {
        if lyrics.lines.is_empty() {
            self.render_empty_lyrics(f, area);
            return;
        }

        // 找到当前行
        let current_index = self.find_current_line_index(&lyrics.lines);
        let lines = self.create_lyrics_lines(&lyrics.lines, current_index, area.height as usize);

        let paragraph = Paragraph::new(lines);
        f.render_widget(paragraph, area);
    }

    /// 找到当前播放行的索引 - 优化版本使用二分查找
    fn find_current_line_index(&self, lines: &[LyricLine]) -> usize {
        if lines.is_empty() {
            return 0;
        }

        // 使用二分查找快速定位
        let mut left = 0;
        let mut right = lines.len();
        
        while left < right {
            let mid = left + (right - left) / 2;
            
            if lines[mid].start_time <= self.current_position_ms {
                // 检查是否在这一行的时间范围内
                if let Some(end_time) = lines[mid].end_time {
                    if self.current_position_ms < end_time {
                        return mid;
                    }
                } else {
                    // 检查下一行（如果存在）
                    if mid + 1 < lines.len() {
                        if self.current_position_ms < lines[mid + 1].start_time {
                            return mid;
                        }
                    } else {
                        // 最后一行
                        return mid;
                    }
                }
                left = mid + 1;
            } else {
                right = mid;
            }
        }
        
        // 如果没有找到，返回最接近的前一行
        left.saturating_sub(1)
    }

    /// 创建歌词显示行
    fn create_lyrics_lines<'b>(&self, lines: &'b [LyricLine], current_index: usize, available_height: usize) -> Vec<Line<'b>> {
        let mut result_lines = Vec::new();
        
        // 计算显示范围
        let max_lines = available_height.saturating_sub(2); // 减去边框
        let context = self.context_lines.min(max_lines / 2);
        
        let start_index = current_index.saturating_sub(context);
        let end_index = (current_index + context + 1).min(lines.len());

        // 如果歌词太少，居中显示
        let total_display_lines = end_index - start_index;
        let padding_top = if total_display_lines < max_lines {
            (max_lines - total_display_lines) / 2
        } else {
            0
        };

        // 添加顶部填充
        for _ in 0..padding_top {
            result_lines.push(Line::from(""));
        }

        // 添加歌词行
        for i in start_index..end_index {
            let line = &lines[i];
            let is_current = i == current_index;
            
            let lyrics_line = if is_current {
                self.create_current_lyrics_line(&line.text)
            } else {
                self.create_normal_lyrics_line(&line.text)
            };
            
            result_lines.push(lyrics_line);
        }

        result_lines
    }

    /// 创建当前行歌词
    fn create_current_lyrics_line<'b>(&self, text: &'b str) -> Line<'b> {
        Line::from(vec![
            Span::styled("▶ ", self.theme.current_line_style()),
            Span::styled(text, self.theme.current_line_style()),
        ])
    }

    /// 创建普通歌词行
    fn create_normal_lyrics_line<'b>(&self, text: &'b str) -> Line<'b> {
        Line::from(vec![
            Span::styled("  ", self.theme.text_style()),
            Span::styled(text, self.theme.text_style()),
        ])
    }

    /// 渲染无歌词状态
    fn render_no_lyrics(&self, f: &mut Frame, area: Rect) {
        let lines = vec![
            Line::from(""),
            Line::from(vec![
                Span::styled("🔍 正在搜索歌词...".to_string(), self.theme.status_style())
            ]),
            Line::from(""),
        ];

        let paragraph = Paragraph::new(lines);
        f.render_widget(paragraph, area);
    }

    /// 渲染空歌词状态
    fn render_empty_lyrics(&self, f: &mut Frame, area: Rect) {
        let lines = vec![
            Line::from(""),
            Line::from(vec![
                Span::styled("❌ 未找到歌词".to_string(), self.theme.status_style())
            ]),
            Line::from(""),
        ];

        let paragraph = Paragraph::new(lines);
        f.render_widget(paragraph, area);
    }
}
