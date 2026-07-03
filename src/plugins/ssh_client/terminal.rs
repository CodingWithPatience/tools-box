use egui::text::LayoutJob;
use egui::{Color32, FontId, TextFormat};

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
}

impl TerminalEmulator {
    /// 创建新的终端仿真器
    pub fn new(cols: u16, rows: u16, font_size: f32) -> Self {
        let parser = vt100::Parser::new(rows, cols, 0);
        Self {
            parser,
            cols,
            rows,
            font_size,
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
                        if prev_cell.is_wide() {
                            2
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
        } else {
            1
        }
    }

    /// 将终端屏幕渲染为 egui::LayoutJob
    pub fn render_to_layout_job(&mut self, is_dark_mode: bool) -> LayoutJob {
        let screen = self.parser.screen();
        // vt100 的 size() 返回 (rows, cols)
        let (current_rows, current_cols) = screen.size();

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

        for row in 0..current_rows {
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
                        let text = if contents.is_empty() {
                            " "
                        } else {
                            &contents
                        };
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
            if row < current_rows - 1 {
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
