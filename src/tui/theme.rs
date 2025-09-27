use ratatui::style::{Color, Modifier, Style};

/// TUI 主题配置
#[derive(Debug, Clone)]
pub struct Theme {
    pub background: Color,
    pub border: Color,
    pub text: Color,
    pub accent: Color,
    pub current_line: Color,
    pub progress_bar: Color,
    pub status_text: Color,
    pub dimmed_text: Color, // 新增：弱化文字颜色
}

impl Theme {
    /// 使用终端原生配色的主题
    pub fn terminal() -> Self {
        Self {
            background: Color::Reset,     // 透明背景，使用终端背景
            border: Color::DarkGray,      // 使用终端的深灰色边框
            text: Color::Reset,           // 使用终端的默认前景色
            accent: Color::Green,         // 使用终端的绿色作为强调色
            current_line: Color::Yellow,  // 使用终端的黄色高亮当前行
            progress_bar: Color::Blue,    // 使用终端的蓝色作为进度条
            status_text: Color::Gray,     // 使用终端的灰色作为状态文字
            dimmed_text: Color::DarkGray, // 使用终端的深灰色作为弱化文字
        }
    }

    /// 默认主题（使用终端配色）
    pub fn default() -> Self {
        Self::terminal()
    }

    /// 简约终端主题（更少的颜色使用）
    pub fn minimal() -> Self {
        Self {
            background: Color::Reset,   // 透明背景
            border: Color::Reset,       // 使用默认前景色作为边框
            text: Color::Reset,         // 使用终端的默认前景色
            accent: Color::Reset,       // 使用默认前景色，依靠粗体区分
            current_line: Color::Reset, // 使用默认前景色，依靠粗体高亮
            progress_bar: Color::Reset, // 使用默认前景色
            status_text: Color::Reset,  // 使用默认前景色
            dimmed_text: Color::Reset,  // 使用默认前景色，依靠样式区分
        }
    }

    /// 获取普通文字样式
    pub fn text_style(&self) -> Style {
        Style::default().fg(self.text)
    }

    /// 获取强调文字样式
    pub fn accent_style(&self) -> Style {
        Style::default()
            .fg(self.accent)
            .add_modifier(Modifier::BOLD)
    }

    /// 获取当前行样式
    pub fn current_line_style(&self) -> Style {
        Style::default()
            .fg(self.current_line)
            .add_modifier(Modifier::BOLD)
    }

    /// 获取边框样式
    pub fn border_style(&self) -> Style {
        Style::default().fg(self.border)
    }

    /// 获取进度条样式
    pub fn progress_style(&self) -> Style {
        Style::default().fg(self.progress_bar)
    }

    /// 获取状态栏样式
    pub fn status_style(&self) -> Style {
        if self.status_text == Color::Reset {
            // 简约主题：使用斜体来区分状态文字
            Style::default()
                .fg(self.status_text)
                .add_modifier(Modifier::ITALIC)
        } else {
            Style::default().fg(self.status_text)
        }
    }

    /// 获取标题样式
    pub fn title_style(&self) -> Style {
        Style::default()
            .fg(self.accent)
            .add_modifier(Modifier::BOLD)
    }

    /// 获取弱化文字样式
    pub fn dimmed_style(&self) -> Style {
        if self.dimmed_text == Color::Reset {
            // 简约主题：使用暗淡修饰符而不是颜色
            Style::default()
                .fg(self.dimmed_text)
                .add_modifier(Modifier::DIM)
        } else {
            Style::default().fg(self.dimmed_text)
        }
    }

    /// 获取播放器状态样式
    pub fn player_style(&self) -> Style {
        Style::default().fg(self.text).add_modifier(Modifier::BOLD)
    }
}
