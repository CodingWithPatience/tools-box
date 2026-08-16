use egui::text::LayoutJob;
use egui::{Color32, FontId, TextFormat};

/// 默认滚动缓冲区大小
const DEFAULT_SCROLLBACK_SIZE: usize = 10000;

/// 终端仿真器（封装 vt100 解析器 + egui 渲染）
pub struct TerminalEmulator {
    /// vt100 ANSI 解析器
    parser: vt100::Parser,
    /// 终端列数
    cols: u16,
    /// 终端行数
    rows: u16,
    /// 字体大小
    font_size: f32,
    /// 滚动缓冲区大小
    scrollback_size: usize,
}

impl TerminalEmulator {
    /// 创建新的终端仿真器
    pub fn new(cols: u16, rows: u16, font_size: f32) -> Self {
        let parser = vt100::Parser::new(rows, cols, DEFAULT_SCROLLBACK_SIZE);
        Self {
            parser,
            cols,
            rows,
            font_size,
            scrollback_size: DEFAULT_SCROLLBACK_SIZE,
        }
    }

    /// 将原始字节流喂入终端解析器
    pub fn process(&mut self, data: &[u8]) {
        self.parser.process(data);
    }

    /// 获取终端 size (cols, rows)
    pub fn size(&self) -> (u16, u16) {
        (self.cols, self.rows)
    }

    /// 调整终端大小
    pub fn resize(&mut self, cols: u16, rows: u16) {
        if cols != self.cols || rows != self.rows {
            self.parser.set_size(rows, cols);
            self.cols = cols;
            self.rows = rows;
        }
    }

    /// 获取字体大小
    pub fn font_size(&self) -> f32 {
        self.font_size
    }

    /// 获取滚动缓冲区大小
    pub fn scrollback_size(&self) -> usize {
        self.scrollback_size
    }

    /// 获取当前滚动位置（0 表示在最底部）
    pub fn scrollback(&self) -> usize {
        let screen = self.parser.screen();
        screen.scrollback()
    }

    /// 设置滚动位置
    pub fn set_scrollback(&mut self, offset: usize) {
        self.parser.set_scrollback(offset);
    }

    /// 获取可见行数（包括滚动缓冲区的行）
    /// vt100 的 cell(row, col) 已通过 visible_cell 内部处理了滚动偏移
    pub fn visible_rows(&self) -> usize {
        let screen = self.parser.screen();
        let (rows, _cols) = screen.size();
        // 遍历可见行来获取实际行数
        // visible_rows() 返回的迭代器包含滚动缓冲区的行和当前屏幕的行
        let mut count = 0;
        for row in 0..rows {
            if screen.cell(row, 0).is_some() {
                count += 1;
            }
        }
        count
    }

    /// 调整字体大小
    pub fn set_font_size(&mut self, size: f32) {
        self.font_size = size;
    }

    /// 获取光标位置 (col, row)
    pub fn cursor_position(&self) -> (u16, u16) {
        let screen = self.parser.screen();
        let (row, col) = screen.cursor_position();
        (col, row)
    }

    /// 获取光标位置字符的宽度（1 或 2）
    /// 宽字符（如中文）返回 2，普通字符返回 1
    pub fn cursor_char_width(&self) -> u16 {
        let screen = self.parser.screen();
        let (row, col) = screen.cursor_position();
        if let Some(cell) = screen.cell(row, col) {
            if cell.is_wide() {
                2
            } else if cell.is_wide_continuation() {
                // 光标在宽字符的后半部分，需要获取前一个字符
                if col > 0 {
                    if let Some(prev_cell) = screen.cell(row, col - 1) {
                        if prev_cell.is_wide() { 2 } else { 1 }
                    } else {
                        1
                    }
                } else {
                    1
                }
            } else {
                1
            }
        } else {
            1
        }
    }

    /// 获取当前可见终端的纯文本内容
    pub fn visible_text(&self) -> String {
        self.parser.screen().contents()
    }

    /// 将宽字符后半单元格位置归一化到该字符的起始单元格
    pub fn normalize_position(&self, position: (u16, u16)) -> (u16, u16) {
        let screen = self.parser.screen();
        let (rows, cols) = screen.size();
        if rows == 0 || cols == 0 {
            return (0, 0);
        }

        let col = position.0.min(cols - 1);
        let row = position.1.min(rows - 1);
        match screen.cell(row, col) {
            Some(cell) if cell.is_wide_continuation() && col > 0 => (col - 1, row),
            _ => (col, row),
        }
    }

    /// 获取指定单元格字符在终端中占用的列数
    pub fn char_width_at(&self, position: (u16, u16)) -> u16 {
        let position = self.normalize_position(position);
        self.parser
            .screen()
            .cell(position.1, position.0)
            .map(|cell| if cell.is_wide() { 2 } else { 1 })
            .unwrap_or(1)
    }

    /// 获取两个终端单元格之间的纯文本内容，起止位置均包含在结果中
    pub fn text_between(&self, start: (u16, u16), end: (u16, u16)) -> String {
        let screen = self.parser.screen();
        let (rows, cols) = screen.size();
        if rows == 0 || cols == 0 {
            return String::new();
        }

        let normalized_start = self.normalize_position(start);
        let normalized_end = self.normalize_position(end);
        let ((start_col, start_row), (end_col, end_row)) =
            if (normalized_start.1, normalized_start.0) <= (normalized_end.1, normalized_end.0) {
                (normalized_start, normalized_end)
            } else {
                (normalized_end, normalized_start)
            };
        screen.contents_between(
            start_row,
            start_col,
            end_row,
            end_col.saturating_add(1).min(cols),
        )
    }

    /// 将终端屏幕渲染为 egui::LayoutJob
    pub fn render_to_layout_job(&mut self, is_dark_mode: bool) -> LayoutJob {
        let screen = self.parser.screen();
        // vt100 的 size() 返回 (rows, cols)
        let (current_rows, current_cols) = screen.size();
        // 计算可见行数（包括滚动缓冲区）
        let visible_rows = self.visible_rows();
        let total_rows = if visible_rows > 0 {
            visible_rows as u16
        } else {
            current_rows
        };

        let default_fg = if is_dark_mode {
            Color32::from_rgb(0xd0, 0xd0, 0xd0)
        } else {
            Color32::from_rgb(0x30, 0x30, 0x30)
        };
        let default_bg = if is_dark_mode {
            Color32::from_rgb(0x1e, 0x1e, 0x1e)
        } else {
            Color32::from_rgb(0xff, 0xff, 0xff)
        };

        let mut job = LayoutJob::default();
        job.wrap.max_width = f32::INFINITY;

        for row in 0..total_rows {
            // 构建当前行，不创建中间 LayoutJob，直接 append 到主 job
            for col in 0..current_cols {
                match screen.cell(row, col) {
                    Some(c) => {
                        // 跳过宽字符的后半部分，避免中文字符间出现缝隙
                        if c.is_wide_continuation() {
                            continue;
                        }

                        let contents = c.contents();
                        // 空单元格（如 TAB 跳过的位置）用空格填充
                        let text = if contents.is_empty() { " " } else { &contents };
                        let fg = ansi_color_to_egui(c.fgcolor(), default_fg);
                        let bg_color = ansi_color_to_egui(c.bgcolor(), default_bg);

                        job.append(
                            text,
                            0.0,
                            TextFormat {
                                font_id: FontId::monospace(self.font_size),
                                color: fg,
                                background: if bg_color == default_bg {
                                    Color32::TRANSPARENT
                                } else {
                                    bg_color
                                },
                                italics: c.italic(),
                                ..Default::default()
                            },
                        );
                    }
                    None => {
                        // 超出范围的单元格填充空格
                        job.append(
                            " ",
                            0.0,
                            TextFormat {
                                font_id: FontId::monospace(self.font_size),
                                color: default_fg,
                                background: default_bg,
                                ..Default::default()
                            },
                        );
                    }
                }
            }
            // 非最后一行追加换行符
            if row < total_rows - 1 {
                job.append(
                    "\n",
                    0.0,
                    TextFormat {
                        font_id: FontId::monospace(self.font_size),
                        color: default_fg,
                        background: default_bg,
                        ..Default::default()
                    },
                );
            }
        }

        job
    }
}

/// ANSI 颜色 → egui Color32
fn ansi_color_to_egui(color: vt100::Color, default: Color32) -> Color32 {
    match color {
        vt100::Color::Default => default,
        vt100::Color::Idx(idx) => {
            let i = usize::from(idx);
            if i < 16 {
                ANSI_PALETTE[i]
            } else {
                // 超过 16 色的索引色回退到默认前景色
                default
            }
        }
        vt100::Color::Rgb(r, g, b) => Color32::from_rgb(r, g, b),
    }
}

/// ANSI 标准 16 色调色板
const ANSI_PALETTE: [Color32; 16] = [
    Color32::from_rgb(0, 0, 0),       // 0  Black
    Color32::from_rgb(170, 0, 0),     // 1  Red
    Color32::from_rgb(0, 170, 0),     // 2  Green
    Color32::from_rgb(170, 85, 0),    // 3  Yellow
    Color32::from_rgb(0, 0, 170),     // 4  Blue
    Color32::from_rgb(170, 0, 170),   // 5  Magenta
    Color32::from_rgb(0, 170, 170),   // 6  Cyan
    Color32::from_rgb(170, 170, 170), // 7  White
    Color32::from_rgb(85, 85, 85),    // 8  Bright Black
    Color32::from_rgb(255, 85, 85),   // 9  Bright Red
    Color32::from_rgb(85, 255, 85),   // 10 Bright Green
    Color32::from_rgb(255, 255, 85),  // 11 Bright Yellow
    Color32::from_rgb(85, 85, 255),   // 12 Bright Blue
    Color32::from_rgb(255, 85, 255),  // 13 Bright Magenta
    Color32::from_rgb(85, 255, 255),  // 14 Bright Cyan
    Color32::from_rgb(255, 255, 255), // 15 Bright White
];

#[cfg(test)]
mod tests {
    use super::TerminalEmulator;

    #[test]
    fn visible_text_returns_plain_terminal_contents() {
        let mut terminal = TerminalEmulator::new(12, 3, 14.0);
        terminal.process(b"hello\r\nworld");

        assert_eq!(terminal.visible_text(), "hello\nworld");
    }

    #[test]
    fn text_between_includes_both_selected_cells() {
        let mut terminal = TerminalEmulator::new(12, 3, 14.0);
        terminal.process(b"hello\r\nworld");

        assert_eq!(terminal.text_between((1, 0), (3, 0)), "ell");
        assert_eq!(terminal.text_between((3, 0), (1, 1)), "lo\nwo");
        assert_eq!(terminal.text_between((1, 1), (3, 0)), "lo\nwo");
    }

    #[test]
    fn text_between_clamps_positions_to_terminal_bounds() {
        let mut terminal = TerminalEmulator::new(5, 2, 14.0);
        terminal.process(b"abcde\r\nfghij");

        assert_eq!(terminal.text_between((4, 1), (99, 99)), "j");
    }

    #[test]
    fn wide_character_continuation_selects_the_whole_character() {
        let mut terminal = TerminalEmulator::new(5, 2, 14.0);
        terminal.process("中ab".as_bytes());

        assert_eq!(terminal.normalize_position((1, 0)), (0, 0));
        assert_eq!(terminal.char_width_at((1, 0)), 2);
        assert_eq!(terminal.text_between((1, 0), (1, 0)), "中");
    }

    #[test]
    fn wide_character_selection_works_at_line_end_and_across_rows() {
        let mut terminal = TerminalEmulator::new(4, 3, 14.0);
        terminal.process("ab中\r\n文cd".as_bytes());

        assert_eq!(terminal.text_between((3, 0), (3, 0)), "中");
        assert_eq!(terminal.text_between((3, 0), (1, 1)), "中\n文");
    }
}
