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

    /// 获取当前滚动位置（0 表示在最底部）
    pub fn scrollback(&self) -> usize {
        let screen = self.parser.screen();
        screen.scrollback()
    }

    /// 获取已缓存的历史行数（滚动位置的最大值）
    ///
    /// 由本地 `vt100` 补丁提供，用于计算滚动上限与滚动条范围
    pub fn scrollback_count(&self) -> usize {
        self.parser.screen().scrollback_count()
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

    /// 获取指定单元格实际渲染的文本
    ///
    /// - 空白单元格返回空格（渲染时以空格填充）
    /// - 宽字符的后半单元格与越界单元格返回 `None`（不产生渲染内容）
    pub fn rendered_cell_text(&self, row: u16, col: u16) -> Option<String> {
        let screen = self.parser.screen();
        let cell = screen.cell(row, col)?;

        // 宽字符的后半部分不单独渲染
        if cell.is_wide_continuation() {
            return None;
        }

        let contents = cell.contents();
        if contents.is_empty() {
            Some(" ".to_string())
        } else {
            Some(contents)
        }
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

        // 调色板按主题选择，逐单元格复用
        let palette = ansi_palette(is_dark_mode);

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
                        let fg = ansi_color_to_egui(c.fgcolor(), default_fg, palette);
                        let bg_color = ansi_color_to_egui(c.bgcolor(), default_bg, palette);

                        job.append(
                            text,
                            0.0,
                            TextFormat {
                                font_id: FontId::monospace(self.font_size),
                                color: fg,
                                // 未显式设置背景色的单元格保持透明，避免铺满画布底色；
                                // 按语义判断而非颜色比较，避免调色板取值与默认底色相同时误判
                                background: if matches!(c.bgcolor(), vt100::Color::Default) {
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

/// 根据主题选择 ANSI 16 色调色板
///
/// 深色背景使用针对对比度优化的暗色调色板，浅色背景使用经典调色板
fn ansi_palette(is_dark_mode: bool) -> &'static [Color32; 16] {
    if is_dark_mode {
        &ANSI_PALETTE_DARK
    } else {
        &ANSI_PALETTE_LIGHT
    }
}

/// ANSI 颜色 → egui Color32
///
/// `palette` 为当前主题的 16 色调色板，用于解析 0..16 的索引色
fn ansi_color_to_egui(color: vt100::Color, default: Color32, palette: &[Color32; 16]) -> Color32 {
    match color {
        vt100::Color::Default => default,
        vt100::Color::Idx(idx) => xterm_index_to_egui(idx, palette),
        vt100::Color::Rgb(r, g, b) => Color32::from_rgb(r, g, b),
    }
}

/// 解析 xterm 256 色索引
///
/// - `0..16`：主题调色板（ANSI 标准 16 色）
/// - `16..232`：6×6×6 色立方（每通道取 `XTERM_CUBE_LEVELS`）
/// - `232..256`：24 级灰阶（8 起步，步长 10）
fn xterm_index_to_egui(idx: u8, palette: &[Color32; 16]) -> Color32 {
    match idx {
        0..=15 => palette[usize::from(idx)],
        16..=231 => {
            let cube = usize::from(idx - 16);
            Color32::from_rgb(
                XTERM_CUBE_LEVELS[cube / 36],
                XTERM_CUBE_LEVELS[(cube % 36) / 6],
                XTERM_CUBE_LEVELS[cube % 6],
            )
        }
        _ => {
            let level = 8 + (idx - 232) * 10;
            Color32::from_rgb(level, level, level)
        }
    }
}

/// 6×6×6 色立方每个通道的取值（xterm 标准）
const XTERM_CUBE_LEVELS: [u8; 6] = [0, 95, 135, 175, 215, 255];

/// 暗色主题调色板（基于 One Half Dark）
///
/// 经典 ANSI 调色板中的蓝色（`#0000AA`）在深色背景上对比度仅约 1.25:1，
/// 目录等内容几乎无法辨认，这里替换为对深色背景更友好的取值
/// （例如蓝色 `#61AFEF` 对比度约 7:1），并让亮色变体整体更亮以便区分
const ANSI_PALETTE_DARK: [Color32; 16] = [
    Color32::from_rgb(0x28, 0x2c, 0x34), // 0  Black
    Color32::from_rgb(0xe0, 0x6c, 0x75), // 1  Red
    Color32::from_rgb(0x98, 0xc3, 0x79), // 2  Green
    Color32::from_rgb(0xe5, 0xc0, 0x7b), // 3  Yellow
    Color32::from_rgb(0x61, 0xaf, 0xef), // 4  Blue
    Color32::from_rgb(0xc6, 0x78, 0xdd), // 5  Magenta
    Color32::from_rgb(0x56, 0xb6, 0xc2), // 6  Cyan
    Color32::from_rgb(0xab, 0xb2, 0xbf), // 7  White
    Color32::from_rgb(0x5c, 0x63, 0x70), // 8  Bright Black
    Color32::from_rgb(0xff, 0x8a, 0x93), // 9  Bright Red
    Color32::from_rgb(0xbe, 0xdc, 0x9a), // 10 Bright Green
    Color32::from_rgb(0xf5, 0xd9, 0xa3), // 11 Bright Yellow
    Color32::from_rgb(0x8c, 0xc5, 0xff), // 12 Bright Blue
    Color32::from_rgb(0xdc, 0x9e, 0xe8), // 13 Bright Magenta
    Color32::from_rgb(0x7f, 0xd6, 0xe0), // 14 Bright Cyan
    Color32::from_rgb(0xff, 0xff, 0xff), // 15 Bright White
];

/// 亮色主题调色板（经典 ANSI 16 色，适配浅色背景）
const ANSI_PALETTE_LIGHT: [Color32; 16] = [
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
    use egui::Color32;

    use super::{
        ANSI_PALETTE_DARK, ANSI_PALETTE_LIGHT, TerminalEmulator, XTERM_CUBE_LEVELS, ansi_palette,
        xterm_index_to_egui,
    };

    /// 计算 WCAG 相对对比度（用于锁定深色主题配色可读性）
    fn contrast_ratio(foreground: Color32, background: Color32) -> f32 {
        fn linearize(channel: u8) -> f32 {
            let value = f32::from(channel) / 255.0;
            if value <= 0.03928 {
                value / 12.92
            } else {
                ((value + 0.055) / 1.055).powf(2.4)
            }
        }

        let luminance = |color: Color32| {
            0.2126 * linearize(color.r())
                + 0.7152 * linearize(color.g())
                + 0.0722 * linearize(color.b())
        };

        let first = luminance(foreground);
        let second = luminance(background);
        let (lighter, darker) = if first >= second {
            (first, second)
        } else {
            (second, first)
        };

        (lighter + 0.05) / (darker + 0.05)
    }

    #[test]
    fn palette_is_selected_by_theme() {
        assert_eq!(ansi_palette(true), &ANSI_PALETTE_DARK);
        assert_eq!(ansi_palette(false), &ANSI_PALETTE_LIGHT);

        // 深色主题的蓝色比经典调色板明显更亮（原 #0000AA 在深色背景上几乎不可读）
        let dark_blue = ANSI_PALETTE_DARK[4];
        let light_blue = ANSI_PALETTE_LIGHT[4];
        assert!(dark_blue.b() > light_blue.b());
        assert!(dark_blue.r() > light_blue.r());
        assert!(dark_blue.g() > light_blue.g());

        // 亮色变体整体不暗于普通变体，便于区分强调内容
        for index in 0..8 {
            let normal = ANSI_PALETTE_DARK[index];
            let bright = ANSI_PALETTE_DARK[index + 8];
            let normal_sum = u32::from(normal.r()) + u32::from(normal.g()) + u32::from(normal.b());
            let bright_sum = u32::from(bright.r()) + u32::from(bright.g()) + u32::from(bright.b());
            assert!(
                bright_sum >= normal_sum,
                "亮色变体应不暗于普通变体: index={index}"
            );
        }
    }

    #[test]
    fn dark_palette_blue_is_readable_on_dark_background() {
        let background = Color32::from_rgb(0x1e, 0x1e, 0x1e);

        // 目录等内容使用 ANSI 4 蓝色，经典取值在深色背景上不可读
        assert!(contrast_ratio(ANSI_PALETTE_LIGHT[4], background) < 2.0);
        // 深色调色板达到 WCAG AA 正文对比度，锁定本次修复目标
        assert!(
            contrast_ratio(ANSI_PALETTE_DARK[4], background) >= 4.5,
            "深色主题蓝色对比度不足: {}",
            contrast_ratio(ANSI_PALETTE_DARK[4], background)
        );

        // 主要前景色在深色背景上均达到 AA
        for index in [1, 2, 3, 4, 5, 6, 7] {
            let color = ANSI_PALETTE_DARK[index];
            assert!(
                contrast_ratio(color, background) >= 4.5,
                "深色主题索引 {index} 对比度不足: {}",
                contrast_ratio(color, background)
            );
        }
    }

    #[test]
    fn xterm_index_resolves_palette_cube_and_grayscale() {
        let palette = &ANSI_PALETTE_DARK;

        // 0..16 取主题调色板
        assert_eq!(xterm_index_to_egui(0, palette), palette[0]);
        assert_eq!(xterm_index_to_egui(4, palette), palette[4]);
        assert_eq!(xterm_index_to_egui(15, palette), palette[15]);

        // 16..232 为 6×6×6 色立方
        assert_eq!(xterm_index_to_egui(16, palette), Color32::from_rgb(0, 0, 0));
        assert_eq!(
            xterm_index_to_egui(17, palette),
            Color32::from_rgb(0, 0, 95)
        );
        assert_eq!(
            xterm_index_to_egui(21, palette),
            Color32::from_rgb(0, 0, 255)
        );
        assert_eq!(
            xterm_index_to_egui(196, palette),
            Color32::from_rgb(255, 0, 0)
        );
        assert_eq!(
            xterm_index_to_egui(231, palette),
            Color32::from_rgb(255, 255, 255)
        );
        // 色立方通道取值符合 xterm 标准
        assert_eq!(XTERM_CUBE_LEVELS[1], 95);
        assert_eq!(XTERM_CUBE_LEVELS[5], 255);

        // 232..256 为 24 级灰阶
        assert_eq!(
            xterm_index_to_egui(232, palette),
            Color32::from_rgb(8, 8, 8)
        );
        assert_eq!(
            xterm_index_to_egui(255, palette),
            Color32::from_rgb(238, 238, 238)
        );

        // 取一个非端点的色立方取值（39 = (0, 175, 255)）
        assert_eq!(
            xterm_index_to_egui(39, palette),
            Color32::from_rgb(0, 175, 255)
        );

        // 两套主题调色板只影响 0..16，扩展色与主题无关
        let light_theme = &ANSI_PALETTE_LIGHT;
        assert_eq!(xterm_index_to_egui(4, palette), palette[4]);
        assert_ne!(xterm_index_to_egui(4, palette), light_theme[4]);
        assert_eq!(
            xterm_index_to_egui(39, palette),
            xterm_index_to_egui(39, light_theme)
        );
    }

    #[test]
    fn layout_job_uses_theme_palette_for_directory_blue() {
        // `ls --color` 用 SGR 01;34（加粗蓝）标记目录；本渲染器不加粗，颜色取索引 4
        let mut terminal = TerminalEmulator::new(8, 2, 14.0);
        terminal.process(b"\x1b[01;34mdir\x1b[0m");

        let dark_job = terminal.render_to_layout_job(true);
        let light_job = terminal.render_to_layout_job(false);

        let has_color = |job: &egui::text::LayoutJob, expected: Color32| {
            job.sections
                .iter()
                .any(|section| section.format.color == expected)
        };
        let count_color = |job: &egui::text::LayoutJob, expected: Color32| {
            job.sections
                .iter()
                .filter(|section| section.format.color == expected)
                .count()
        };

        assert!(has_color(&dark_job, ANSI_PALETTE_DARK[4]));
        assert!(has_color(&light_job, ANSI_PALETTE_LIGHT[4]));
        // 只有 dir 三个字符使用目录蓝色，其余单元格保持默认前景色
        assert_eq!(count_color(&dark_job, ANSI_PALETTE_DARK[4]), 3);
        assert_eq!(count_color(&light_job, ANSI_PALETTE_LIGHT[4]), 3);

        // 未显式设置背景色的单元格保持透明（画布底色自行绘制）
        assert!(
            dark_job
                .sections
                .iter()
                .filter(|section| section.format.color == ANSI_PALETTE_DARK[4])
                .all(|section| section.format.background == Color32::TRANSPARENT)
        );
    }

    #[test]
    fn resize_growth_absorbs_history_rows() {
        let mut terminal = TerminalEmulator::new(20, 10, 14.0);
        for index in 0..20 {
            terminal.process(format!("L{index}\r\n").as_bytes());
        }

        // 10 行屏幕：历史 11 行，屏幕显示最新的 9 行内容
        assert_eq!(terminal.scrollback_count(), 11);
        assert_eq!(terminal.cursor_position().1, 9);

        // 放大到 15 行：从历史行回填 5 行，历史行数随之减少
        terminal.resize(20, 15);
        assert_eq!(terminal.scrollback_count(), 6);
        assert_eq!(terminal.cursor_position().1, 14);
        // 内容底部对齐：最新一行仍在屏幕上
        assert!(terminal.visible_text().contains("L19"));

        // 继续放大到 30 行：历史不足时全部回填，滚动条区间归零
        terminal.resize(20, 30);
        assert_eq!(terminal.scrollback_count(), 0);
        let text = terminal.visible_text();
        assert!(text.starts_with("L0\nL1"));
        assert!(text.contains("L19"));
    }

    #[test]
    fn resize_shrink_pushes_rows_into_history_without_loss() {
        let mut terminal = TerminalEmulator::new(20, 20, 14.0);
        for index in 0..19 {
            terminal.process(format!("L{index}\r\n").as_bytes());
        }
        // 19 行内容刚好占满前 19 行，尚未产生历史行
        assert_eq!(terminal.scrollback_count(), 0);

        // 缩小到 5 行：光标在最后一行，顶部 15 行进入历史行，屏幕保留最新内容
        terminal.resize(20, 5);
        assert_eq!(terminal.scrollback_count(), 15);
        assert_eq!(terminal.cursor_position().1, 4);
        let text = terminal.visible_text();
        assert_eq!(text.lines().next(), Some("L15"));
        assert_eq!(text.lines().last(), Some("L18"));

        // 缩小后的历史行仍可滚动查看，且包含被移出屏幕的早期内容
        let max_offset = terminal.scrollback_count();
        terminal.set_scrollback(max_offset);
        assert!(terminal.visible_text().starts_with("L0\nL1"));
    }

    #[test]
    fn resize_restore_does_not_turn_padding_rows_into_history() {
        let mut terminal = TerminalEmulator::new(20, 10, 14.0);
        for index in 0..5 {
            terminal.process(format!("L{index}\r\n").as_bytes());
        }
        // 5 行内容 + 光标停在下一行，此时没有历史行
        assert_eq!(terminal.scrollback_count(), 0);
        let cursor_row = terminal.cursor_position().1;

        // 放大窗口：空行填充在光标下方，不产生历史行（滚动条保持隐藏）
        terminal.resize(20, 40);
        assert_eq!(terminal.scrollback_count(), 0);
        assert_eq!(terminal.cursor_position().1, cursor_row);

        // 恢复到原尺寸：填充空行不能被当作内容进入历史行
        terminal.resize(20, 10);
        assert_eq!(terminal.scrollback_count(), 0);
        assert_eq!(terminal.cursor_position().1, cursor_row);
        let text = terminal.visible_text();
        assert_eq!(text.lines().next(), Some("L0"));
        assert_eq!(text.lines().nth(4), Some("L4"));
    }

    #[test]
    fn resize_shrink_only_scrolls_rows_needed_to_keep_cursor_visible() {
        let mut terminal = TerminalEmulator::new(20, 20, 14.0);
        for index in 0..5 {
            terminal.process(format!("L{index}\r\n").as_bytes());
        }

        // 光标在第 5 行，缩小到 10 行时光标仍在屏幕内：不上移内容，也不产生历史行
        terminal.resize(20, 10);
        assert_eq!(terminal.scrollback_count(), 0);
        assert_eq!(terminal.cursor_position().1, 5);
        assert!(terminal.visible_text().starts_with("L0\nL1"));

        // 缩小到 3 行：只需上移 3 行即可让光标留在最后一行
        terminal.resize(20, 3);
        assert_eq!(terminal.scrollback_count(), 3);
        assert_eq!(terminal.cursor_position().1, 2);
        let text = terminal.visible_text();
        assert_eq!(text.lines().next(), Some("L3"));
        assert_eq!(text.lines().last(), Some("L4"));
    }

    #[test]
    fn resize_keeps_scrollback_offset_within_history() {
        let mut terminal = TerminalEmulator::new(20, 10, 14.0);
        for index in 0..30 {
            terminal.process(format!("L{index}\r\n").as_bytes());
        }

        // 光标在最后一行，回滚 5 行查看较早内容
        terminal.set_scrollback(5);
        assert_eq!(terminal.scrollback(), 5);

        // 放大回填历史行：偏移不超出新的历史行数
        terminal.resize(20, 20);
        assert!(terminal.scrollback() <= terminal.scrollback_count());

        // 缩小：偏移保持不变（视图按「距内容底部的行数」锚定），
        // 因此视图最后一行的内容与缩放前一致
        let offset = terminal.scrollback();
        let before_shrink = terminal.visible_text();
        terminal.resize(20, 6);
        assert_eq!(
            terminal.scrollback(),
            offset.min(terminal.scrollback_count())
        );
        assert_eq!(
            terminal.visible_text().lines().last(),
            before_shrink.lines().last()
        );
    }

    #[test]
    fn resize_shrink_keeps_content_below_cursor() {
        let mut terminal = TerminalEmulator::new(20, 20, 14.0);
        // 第 0 行与第 15 行各有内容，光标随后移回第 0 行（如 shell 的预测行/光标定位输出）
        terminal.process(b"TOP\x1b[15;1HBOTTOM\x1b[1;1H");
        assert!(terminal.visible_text().contains("BOTTOM"));

        // 缩小到 10 行：光标之下的空白填充先丢弃，光标之下的真实内容必须保留
        terminal.resize(20, 10);
        assert!(
            terminal.visible_text().contains("BOTTOM"),
            "光标之下的真实内容不应丢失: {:?}",
            terminal.visible_text()
        );

        // 被移出屏幕的顶部内容进入历史行，仍可回滚查看
        assert!(terminal.scrollback_count() > 0);
        terminal.set_scrollback(terminal.scrollback_count());
        assert!(terminal.visible_text().starts_with("TOP"));
    }

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

    #[test]
    fn scrollback_window_slides_through_history() {
        let mut terminal = TerminalEmulator::new(8, 3, 14.0);
        for index in 0..10 {
            terminal.process(format!("L{}\r\n", index).as_bytes());
        }

        // 当前屏幕：显示最新内容
        assert_eq!(terminal.scrollback(), 0);
        assert_eq!(terminal.visible_text(), "L8\nL9");

        // 历史行数即滚动上限
        let max_offset = terminal.scrollback_count();
        assert_eq!(max_offset, 8);

        // 偏移不超过屏幕行数时，窗口在屏幕上滑动
        terminal.set_scrollback(1);
        assert_eq!(terminal.visible_text(), "L7\nL8\nL9");

        // 偏移超过屏幕行数时，窗口继续向上滑过历史行（本地补丁修复下溢）
        terminal.set_scrollback(4);
        assert_eq!(terminal.visible_text(), "L4\nL5\nL6");

        // 偏移达到上限时显示最早的历史内容
        terminal.set_scrollback(max_offset);
        assert_eq!(terminal.visible_text(), "L0\nL1\nL2");

        // 超过上限的偏移被钳制
        terminal.set_scrollback(max_offset + 100);
        assert_eq!(terminal.scrollback(), max_offset);
        assert_eq!(terminal.visible_text(), "L0\nL1\nL2");
    }

    #[test]
    fn rendered_cell_text_skips_wide_continuation_cells() {
        let mut terminal = TerminalEmulator::new(6, 2, 14.0);
        terminal.process("中ab".as_bytes());

        assert_eq!(terminal.rendered_cell_text(0, 0).as_deref(), Some("中"));
        // 宽字符后半单元格不产生渲染内容
        assert_eq!(terminal.rendered_cell_text(0, 1), None);
        assert_eq!(terminal.rendered_cell_text(0, 2).as_deref(), Some("a"));
        // 空白单元格以空格占位
        assert_eq!(terminal.rendered_cell_text(0, 5).as_deref(), Some(" "));
        // 越界单元格不产生渲染内容
        assert_eq!(terminal.rendered_cell_text(9, 0), None);
        assert_eq!(terminal.rendered_cell_text(0, 9), None);
    }
}
