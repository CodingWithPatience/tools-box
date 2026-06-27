use std::cell::RefCell;
use std::hash::{DefaultHasher, Hash, Hasher};

use egui::{Color32, RichText, text::LayoutJob};

use super::differ;
use super::highlight::SyntaxHighlighter;
use super::models::{DiffResult, DiffType, SplitLine, TextSegment, ViewMode};

/// 支持的编程语言列表
const SUPPORTED_LANGUAGES: &[(&str, &str)] = &[
    ("自动检测", ""),
    ("Plain Text", "Plain Text"),
    ("Bash", "Bash"),
    ("C", "C"),
    ("C++", "C++"),
    ("C#", "C#"),
    ("CSS", "CSS"),
    ("Dockerfile", "Dockerfile"),
    ("Go", "Go"),
    ("HTML", "HTML"),
    ("Java", "Java"),
    ("JavaScript", "JavaScript"),
    ("JSON", "JSON"),
    ("Kotlin", "Kotlin"),
    ("Lua", "Lua"),
    ("Markdown", "Markdown"),
    ("PHP", "PHP"),
    ("Python", "Python"),
    ("Ruby", "Ruby"),
    ("Rust", "Rust"),
    ("SQL", "SQL"),
    ("Swift", "Swift"),
    ("TypeScript", "TypeScript"),
    ("XML", "XML"),
    ("YAML", "YAML"),
];

/// 高亮缓存条目：(内容哈希, 对应的 LayoutJob)
type HighlightCache = RefCell<Option<(u64, LayoutJob)>>;

/// Diff 查看器 UI
pub struct DiffViewerUi {
    /// 左侧文本
    left_text: String,
    /// 右侧文本
    right_text: String,
    /// 左侧文件名
    left_file_name: Option<String>,
    /// 右侧文件名
    right_file_name: Option<String>,
    /// 视图模式
    view_mode: ViewMode,
    /// 差异结果
    diff_result: Option<DiffResult>,
    /// 错误信息
    error: Option<String>,
    /// 语法高亮器
    highlighter: SyntaxHighlighter,
    /// 选择的语言
    selected_language: String,
    /// 左侧编辑区高亮缓存
    left_highlight_cache: HighlightCache,
    /// 右侧编辑区高亮缓存
    right_highlight_cache: HighlightCache,
}

impl DiffViewerUi {
    pub fn new() -> Self {
        Self {
            left_text: String::new(),
            right_text: String::new(),
            left_file_name: None,
            right_file_name: None,
            view_mode: ViewMode::Edit,
            diff_result: None,
            error: None,
            highlighter: SyntaxHighlighter::new(),
            selected_language: "自动检测".to_string(),
            left_highlight_cache: RefCell::new(None),
            right_highlight_cache: RefCell::new(None),
        }
    }

    /// 渲染主界面
    pub fn render(&mut self, ui: &mut egui::Ui) {
        match self.view_mode {
            ViewMode::Edit => self.render_edit_mode(ui),
            ViewMode::Split => self.render_split_view(ui),
            ViewMode::Unified => self.render_unified_view(ui),
        }
    }

    /// 渲染编辑模式
    fn render_edit_mode(&mut self, ui: &mut egui::Ui) {
        ui.heading("📝 文本对比工具");
        ui.separator();

        // 双栏输入区域
        let available_height = ui.available_height() - 120.0;

        // 提前计算语法高亮参数，供 layouter 闭包使用
        let syntax_name = self.get_syntax_name();
        let is_dark_mode = ui.visuals().dark_mode;
        let font_size = ui
            .style()
            .text_styles
            .get(&egui::TextStyle::Monospace)
            .map(|font_id| font_id.size)
            .unwrap_or(14.0);

        // 预计算行号宽度，确保左右两侧一致
        let left_lines = self.left_text.lines().count().max(1);
        let right_lines = self.right_text.lines().count().max(1);
        let max_lines = left_lines.max(right_lines);
        let line_num_digits = format!("{}", max_lines).len().max(3);

        // 行号列宽（字符数）：数字位数 + 分隔符 " │ "
        let gutter_char_count = line_num_digits + 3;
        let gutter_width = gutter_char_count as f32 * font_size * 0.6;
        let line_height = font_size * 1.2;

        // 使用 columns 实现双栏布局
        ui.columns(2, |columns| {
            // 左侧文本
            columns[0].vertical(|ui| {
                ui.horizontal(|ui| {
                    ui.label("原始文本 (左侧):");
                    if let Some(name) = &self.left_file_name {
                        ui.label(
                            RichText::new(format!("📄 {}", name))
                                .color(Color32::from_rgb(100, 100, 100))
                                .small(),
                        );
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.small_button("📂 加载文件").clicked() {
                            self.load_file_to_left();
                        }
                    });
                });
                // 此处 load_file_to_left 已执行完毕，可以安全借用 highlighter 和 cache
                let highlighter = &self.highlighter;
                let cache = &self.left_highlight_cache;
                let line_count = self.left_text.lines().count();
                // 行号在 ScrollArea 外部，不随水平滚动移动
                ui.horizontal(|ui| {
                    let gutter_origin = ui.cursor().left_top();
                    // 行号区域占位（高度与 ScrollArea 一致）
                    ui.allocate_space(egui::vec2(gutter_width, available_height));
                    // 编辑区 ScrollArea
                    egui::ScrollArea::both()
                        .id_salt("diff_edit_left")
                        .max_height(available_height)
                        .show(ui, |ui| {
                            let mut left_layouter =
                                |ui: &egui::Ui, string: &str, _wrap_width: f32| {
                                    Self::highlight_text_with_cache(
                                        highlighter,
                                        cache,
                                        string,
                                        &syntax_name,
                                        is_dark_mode,
                                        font_size,
                                        f32::INFINITY,
                                        ui,
                                    )
                                };
                            egui::TextEdit::multiline(&mut self.left_text)
                                .hint_text("在此输入原始文本...")
                                .layouter(&mut left_layouter)
                                .desired_width(f32::INFINITY)
                                .min_size(egui::vec2(0.0, available_height))
                                .show(ui);
                        });
                    // 读取 ScrollArea 垂直偏移并绘制行号
                    let scroll_id = ui.make_persistent_id(egui::Id::new("diff_edit_left"));
                    let offset_y = egui::scroll_area::State::load(ui.ctx(), scroll_id)
                        .map(|s| s.offset.y)
                        .unwrap_or(0.0);
                    Self::render_gutter(
                        ui,
                        gutter_origin,
                        gutter_width,
                        available_height,
                        line_count,
                        line_num_digits,
                        line_height,
                        offset_y,
                        font_size,
                    );
                });
            });

            // 右侧文本
            columns[1].vertical(|ui| {
                ui.horizontal(|ui| {
                    ui.label("对比文本 (右侧):");
                    if let Some(name) = &self.right_file_name {
                        ui.label(
                            RichText::new(format!("📄 {}", name))
                                .color(Color32::from_rgb(100, 100, 100))
                                .small(),
                        );
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.small_button("📂 加载文件").clicked() {
                            self.load_file_to_right();
                        }
                    });
                });
                // 此处 load_file_to_right 已执行完毕，可以安全借用 highlighter 和 cache
                let highlighter = &self.highlighter;
                let cache = &self.right_highlight_cache;
                let line_count = self.right_text.lines().count();
                // 行号在 ScrollArea 外部，不随水平滚动移动
                ui.horizontal(|ui| {
                    let gutter_origin = ui.cursor().left_top();
                    // 行号区域占位（高度与 ScrollArea 一致）
                    ui.allocate_space(egui::vec2(gutter_width, available_height));
                    // 编辑区 ScrollArea
                    egui::ScrollArea::both()
                        .id_salt("diff_edit_right")
                        .max_height(available_height)
                        .show(ui, |ui| {
                            let mut right_layouter =
                                |ui: &egui::Ui, string: &str, _wrap_width: f32| {
                                    Self::highlight_text_with_cache(
                                        highlighter,
                                        cache,
                                        string,
                                        &syntax_name,
                                        is_dark_mode,
                                        font_size,
                                        f32::INFINITY,
                                        ui,
                                    )
                                };
                            egui::TextEdit::multiline(&mut self.right_text)
                                .hint_text("在此输入对比文本...")
                                .layouter(&mut right_layouter)
                                .desired_width(f32::INFINITY)
                                .min_size(egui::vec2(0.0, available_height))
                                .show(ui);
                        });
                    // 读取 ScrollArea 垂直偏移并绘制行号
                    let scroll_id = ui.make_persistent_id(egui::Id::new("diff_edit_right"));
                    let offset_y = egui::scroll_area::State::load(ui.ctx(), scroll_id)
                        .map(|s| s.offset.y)
                        .unwrap_or(0.0);
                    Self::render_gutter(
                        ui,
                        gutter_origin,
                        gutter_width,
                        available_height,
                        line_count,
                        line_num_digits,
                        line_height,
                        offset_y,
                        font_size,
                    );
                });
            });
        });

        ui.add_space(10.0);

        // 错误信息
        if let Some(err) = &self.error {
            ui.label(RichText::new(err).color(Color32::RED));
            ui.add_space(5.0);
        }

        // 操作按钮
        ui.horizontal(|ui| {
            if ui.button("🔄 交换内容").clicked() {
                std::mem::swap(&mut self.left_text, &mut self.right_text);
                std::mem::swap(&mut self.left_file_name, &mut self.right_file_name);
                self.clear_cache();
            }

            if ui.button("🗑 清空").clicked() {
                self.left_text.clear();
                self.right_text.clear();
                self.left_file_name = None;
                self.right_file_name = None;
                self.diff_result = None;
                self.error = None;
                self.clear_cache();
            }

            ui.separator();

            // 语言选择
            ui.label("语言:");
            egui::ComboBox::from_id_salt("language_select")
                .selected_text(&self.selected_language)
                .show_ui(ui, |ui| {
                    for (name, _) in SUPPORTED_LANGUAGES {
                        if ui
                            .selectable_label(self.selected_language == *name, *name)
                            .clicked()
                        {
                            self.selected_language = name.to_string();
                            self.clear_cache();
                        }
                    }
                });

            ui.separator();

            // 开始对比按钮
            let compare_btn = ui.button("📊 开始对比");
            if compare_btn.clicked() {
                self.diff_result = Some(differ::compute_diff(&self.left_text, &self.right_text));
                self.view_mode = ViewMode::Split;
            }
        });
    }

    /// 加载文件到左侧
    fn load_file_to_left(&mut self) {
        if let Some(path) = rfd::FileDialog::new()
            .set_title("选择原始文本文件")
            .pick_file()
        {
            match std::fs::read_to_string(&path) {
                Ok(content) => {
                    self.left_text = content;
                    self.left_file_name = path.file_name().map(|n| n.to_string_lossy().to_string());
                    self.error = None;
                }
                Err(e) => {
                    self.error = Some(format!("读取文件失败: {}", e));
                }
            }
        }
    }

    /// 加载文件到右侧
    fn load_file_to_right(&mut self) {
        if let Some(path) = rfd::FileDialog::new()
            .set_title("选择对比文本文件")
            .pick_file()
        {
            match std::fs::read_to_string(&path) {
                Ok(content) => {
                    self.right_text = content;
                    self.right_file_name = path.file_name().map(|n| n.to_string_lossy().to_string());
                    self.error = None;

                    // 尝试自动检测语言
                    if let Some(ext) = path.extension() {
                        if let Some(name) = self.highlighter.get_syntax_name_for_extension(&ext.to_string_lossy()) {
                            self.selected_language = name;
                        }
                    }
                }
                Err(e) => {
                    self.error = Some(format!("读取文件失败: {}", e));
                }
            }
        }
    }

    /// 获取当前选择的语法名称
    fn get_syntax_name(&self) -> Option<String> {
        if self.selected_language == "自动检测" {
            // 尝试从文件名检测
            self.left_file_name
                .as_deref()
                .or(self.right_file_name.as_deref())
                .and_then(|name| {
                    // 从文件扩展名获取
                    let ext = std::path::Path::new(name)
                        .extension()
                        .map(|e| e.to_string_lossy().to_string())?;
                    self.highlighter.get_syntax_name_for_extension(&ext)
                })
        } else if self.selected_language == "Plain Text" {
            None
        } else {
            Some(self.selected_language.clone())
        }
    }

    /// 判断是否为深色模式
    fn is_dark_mode(&self, ui: &egui::Ui) -> bool {
        ui.visuals().dark_mode
    }

    /// 计算高亮缓存的哈希值（基于文本内容 + 语法名称 + 主题 + 字号）
    fn compute_highlight_hash(
        text: &str,
        syntax_name: &Option<String>,
        is_dark_mode: bool,
        font_size: f32,
    ) -> u64 {
        let mut hasher = DefaultHasher::new();
        text.hash(&mut hasher);
        syntax_name.hash(&mut hasher);
        is_dark_mode.hash(&mut hasher);
        font_size.to_bits().hash(&mut hasher);
        hasher.finish()
    }

    /// 带缓存的语法高亮：仅当文本或高亮参数变化时才重新计算
    fn highlight_text_with_cache(
        highlighter: &SyntaxHighlighter,
        cache: &HighlightCache,
        text: &str,
        syntax_name: &Option<String>,
        is_dark_mode: bool,
        font_size: f32,
        wrap_width: f32,
        ui: &egui::Ui,
    ) -> std::sync::Arc<egui::Galley> {
        let hash = Self::compute_highlight_hash(text, syntax_name, is_dark_mode, font_size);
        let mut cache_ref = cache.borrow_mut();
        if let Some((cached_hash, cached_job)) = cache_ref.as_ref() {
            if *cached_hash == hash {
                let mut job = cached_job.clone();
                job.wrap.max_width = wrap_width;
                return ui.fonts(|f| f.layout_job(job));
            }
        }
        let mut job =
            highlighter.highlight_to_layout_job(text, syntax_name.as_deref(), font_size, is_dark_mode);
        job.wrap.max_width = wrap_width;
        let galley = ui.fonts(|f| f.layout_job(job.clone()));
        *cache_ref = Some((hash, job));
        galley
    }

    /// 清空高亮缓存（文本内容或语言切换时调用）
    fn clear_cache(&self) {
        self.left_highlight_cache.borrow_mut().take();
        self.right_highlight_cache.borrow_mut().take();
    }

    /// 渲染固定行号面板（不随水平滚动移动，垂直与 ScrollArea 同步）
    #[allow(clippy::too_many_arguments)]
    fn render_gutter(
        ui: &egui::Ui,
        origin: egui::Pos2,
        width: f32,
        height: f32,
        line_count: usize,
        num_digits: usize,
        line_height: f32,
        scroll_offset_y: f32,
        font_size: f32,
    ) {
        let gutter_rect = egui::Rect::from_min_size(origin, egui::vec2(width, height));
        let painter = ui.painter().with_clip_rect(gutter_rect);
        let text_color = Color32::from_rgb(128, 128, 128);
        let font_id = egui::FontId::monospace(font_size);
        // 计算当前可见行范围
        let first_visible = (scroll_offset_y / line_height).floor() as usize;
        let visible_count = (height / line_height).ceil() as usize + 1;
        let last_visible = (first_visible + visible_count).min(line_count);
        let frac_offset = scroll_offset_y - first_visible as f32 * line_height;
        for i in first_visible..last_visible {
            let y = origin.y + (i - first_visible) as f32 * line_height - frac_offset;
            if y + line_height < origin.y || y > origin.y + height {
                continue;
            }
            let text = format!("{:>width$} │ ", i + 1, width = num_digits);
            painter.text(
                egui::pos2(origin.x + width, y),
                egui::Align2::RIGHT_TOP,
                &text,
                font_id.clone(),
                text_color,
            );
        }
    }

    /// 渲染 Split 视图
    fn render_split_view(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.heading("📝 文本对比工具 - Split 视图");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("返回编辑").clicked() {
                    self.view_mode = ViewMode::Edit;
                }
                if ui.button("统一视图").clicked() {
                    self.view_mode = ViewMode::Unified;
                }
            });
        });
        ui.separator();

        if let Some(result) = &self.diff_result {
            // 先计算统计栏所需高度（分隔线 + 文本高度 + 间距）
            let text_style = egui::TextStyle::Small;
            let stats_height = ui.text_style_height(&text_style) + 16.0;

            // 使用 bottom_panel 固定统计栏在底部
            egui::TopBottomPanel::bottom("diff_split_stats")
                .exact_height(stats_height)
                .show_inside(ui, |ui| {
                    ui.separator();
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(format!(
                                "统计：新增 {} 行 | 删除 {} 行 | 相似度：{:.1}%",
                                result.added_count,
                                result.removed_count,
                                result.similarity * 100.0
                            ))
                            .color(Color32::from_rgb(100, 100, 100)),
                        );
                    });
                });

            // 剩余空间用于内容区域
            ui.vertical(|ui| {
                // Split 视图：左右并排显示
                ui.horizontal(|ui| {
                    // 左侧标题
                    ui.vertical(|ui| {
                        ui.label(
                            RichText::new("原始文本")
                                .strong()
                                .color(Color32::from_rgb(100, 100, 100)),
                        );
                    });

                    ui.separator();

                    // 右侧标题
                    ui.vertical(|ui| {
                        ui.label(
                            RichText::new("对比文本")
                                .strong()
                                .color(Color32::from_rgb(100, 100, 100)),
                        );
                    });
                });

                ui.separator();

                // 使用 ScrollArea 支持横向和纵向滚动
                // 当内容超出可用高度时才显示滚动条
                egui::ScrollArea::both()
                    .auto_shrink([false, false])  // 不自动缩小，内容不足时不显示滚动条
                    .id_salt("diff_split_view")
                    .show(ui, |ui| {
                        // 获取当前字体大小
                        let font_size = ui.style().text_styles.get(&egui::TextStyle::Monospace)
                            .map(|font_id| font_id.size)
                            .unwrap_or(14.0);

                        // 设置等宽字体
                        let code_style = egui::TextStyle::Monospace;
                        let row_height = ui.text_style_height(&code_style) + 4.0;

                        // 获取语法高亮参数
                        let syntax_name = self.get_syntax_name();
                        let is_dark_mode = self.is_dark_mode(ui);

                        egui::Grid::new("diff_split_grid")
                            .striped(true)
                            .spacing([0.0, 0.0])
                            .min_col_width(ui.available_width() / 2.0)
                            .show(ui, |ui| {
                                for line in &result.split_lines {
                                    self.render_split_line(ui, line, row_height, font_size, &syntax_name, is_dark_mode);
                                    ui.end_row();
                                }
                            });
                    });
            });
        }
    }

    /// 渲染 Split 视图的单行
    fn render_split_line(&self, ui: &mut egui::Ui, line: &SplitLine, row_height: f32, font_size: f32, syntax_name: &Option<String>, is_dark_mode: bool) {
        // 获取当前主题的文字颜色
        let text_color = ui.visuals().text_color();
        let dim_color = Color32::from_rgb(128, 128, 128);

        // 左侧
        let left_bg = match line.left_type {
            DiffType::Removed => Color32::from_rgb(255, 220, 220), // 红色背景
            DiffType::Equal => Color32::TRANSPARENT,
            _ => Color32::TRANSPARENT,
        };

        ui.vertical(|ui| {
            // 绘制背景
            let rect = ui.available_rect_before_wrap();
            let rect = egui::Rect::from_min_size(rect.min, egui::vec2(rect.width(), row_height));
            ui.painter().rect_filled(rect, 0.0, left_bg);

            if let Some(content) = &line.left_content {
                let line_num = line
                    .left_line_number
                    .map(|n| format!("{:>4} │ ", n))
                    .unwrap_or_else(|| "      │ ".to_string());

                // 使用字符级差异渲染（优先级最高）
                if !line.left_segments.is_empty() {
                    let job = self.create_segment_layout(&line.left_segments, Some(&line_num), text_color, font_size);
                    ui.label(job);
                } else if line.left_type == DiffType::Equal && syntax_name.is_some() {
                    // 相同行使用语法高亮
                    let mut job = LayoutJob::default();
                    job.append(
                        &line_num,
                        0.0,
                        egui::TextFormat {
                            font_id: egui::FontId::monospace(font_size),
                            color: dim_color,
                            ..Default::default()
                        },
                    );
                    let highlighted = self.highlighter.highlight_line(content, syntax_name.as_deref(), font_size, is_dark_mode);
                    for (color, text) in highlighted {
                        job.append(
                            &text,
                            0.0,
                            egui::TextFormat {
                                font_id: egui::FontId::monospace(font_size),
                                color,
                                ..Default::default()
                            },
                        );
                    }
                    ui.label(job);
                } else {
                    // 差异行使用差异颜色
                    let text = format!("{}{}", line_num, content);
                    let rich_text = RichText::new(text).monospace();
                    let rich_text = match line.left_type {
                        DiffType::Removed => rich_text.color(Color32::from_rgb(180, 0, 0)),
                        _ => rich_text.color(text_color),
                    };
                    ui.label(rich_text);
                }
            } else {
                let rich_text = RichText::new("      │ ").monospace().color(dim_color);
                ui.label(rich_text);
            }
        });

        // 右侧
        let right_bg = match line.right_type {
            DiffType::Added => Color32::from_rgb(220, 255, 220), // 绿色背景
            DiffType::Equal => Color32::TRANSPARENT,
            _ => Color32::TRANSPARENT,
        };

        ui.vertical(|ui| {
            // 绘制背景
            let rect = ui.available_rect_before_wrap();
            let rect = egui::Rect::from_min_size(rect.min, egui::vec2(rect.width(), row_height));
            ui.painter().rect_filled(rect, 0.0, right_bg);

            if let Some(content) = &line.right_content {
                let line_num = line
                    .right_line_number
                    .map(|n| format!("{:>4} │ ", n))
                    .unwrap_or_else(|| "      │ ".to_string());

                // 使用字符级差异渲染（优先级最高）
                if !line.right_segments.is_empty() {
                    let job = self.create_segment_layout(&line.right_segments, Some(&line_num), text_color, font_size);
                    ui.label(job);
                } else if line.right_type == DiffType::Equal && syntax_name.is_some() {
                    // 相同行使用语法高亮
                    let mut job = LayoutJob::default();
                    job.append(
                        &line_num,
                        0.0,
                        egui::TextFormat {
                            font_id: egui::FontId::monospace(font_size),
                            color: dim_color,
                            ..Default::default()
                        },
                    );
                    let highlighted = self.highlighter.highlight_line(content, syntax_name.as_deref(), font_size, is_dark_mode);
                    for (color, text) in highlighted {
                        job.append(
                            &text,
                            0.0,
                            egui::TextFormat {
                                font_id: egui::FontId::monospace(font_size),
                                color,
                                ..Default::default()
                            },
                        );
                    }
                    ui.label(job);
                } else {
                    // 差异行使用差异颜色
                    let text = format!("{}{}", line_num, content);
                    let rich_text = RichText::new(text).monospace();
                    let rich_text = match line.right_type {
                        DiffType::Added => rich_text.color(Color32::from_rgb(0, 150, 0)),
                        _ => rich_text.color(text_color),
                    };
                    ui.label(rich_text);
                }
            } else {
                let rich_text = RichText::new("      │ ").monospace().color(dim_color);
                ui.label(rich_text);
            }
        });
    }

    /// 创建字符级差异的布局任务
    fn create_segment_layout(&self, segments: &[TextSegment], prefix: Option<&str>, text_color: Color32, font_size: f32) -> LayoutJob {
        let mut job = LayoutJob::default();

        // 添加行号前缀
        if let Some(prefix) = prefix {
            job.append(
                prefix,
                0.0,
                egui::TextFormat {
                    font_id: egui::FontId::monospace(font_size),
                    color: Color32::from_rgb(128, 128, 128),
                    ..Default::default()
                },
            );
        }

        // 添加字符级差异片段
        for segment in segments {
            let color = match segment.diff_type {
                DiffType::Equal => text_color,                         // 使用主题文字颜色
                DiffType::Added => Color32::from_rgb(0, 150, 0),      // 绿色
                DiffType::Removed => Color32::from_rgb(180, 0, 0),    // 红色
            };

            let bg_color = match segment.diff_type {
                DiffType::Added => Color32::from_rgb(180, 255, 180),   // 浅绿色背景
                DiffType::Removed => Color32::from_rgb(255, 180, 180), // 浅红色背景
                DiffType::Equal => Color32::TRANSPARENT,
            };

            job.append(
                &segment.text,
                0.0,
                egui::TextFormat {
                    font_id: egui::FontId::monospace(font_size),
                    color,
                    background: bg_color,
                    ..Default::default()
                },
            );
        }

        job
    }

    /// 渲染 Unified 视图
    fn render_unified_view(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.heading("📝 文本对比工具 - 统一视图");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("返回编辑").clicked() {
                    self.view_mode = ViewMode::Edit;
                }
                if ui.button("Split 视图").clicked() {
                    self.view_mode = ViewMode::Split;
                }
            });
        });
        ui.separator();

        if let Some(result) = &self.diff_result {
            // 先计算统计栏所需高度（分隔线 + 文本高度 + 间距）
            let text_style = egui::TextStyle::Small;
            let stats_height = ui.text_style_height(&text_style) + 16.0;

            // 获取当前主题的文字颜色
            let text_color = ui.visuals().text_color();
            let dim_color = Color32::from_rgb(128, 128, 128);

            // 使用 bottom_panel 固定统计栏在底部
            egui::TopBottomPanel::bottom("diff_unified_stats")
                .exact_height(stats_height)
                .show_inside(ui, |ui| {
                    ui.separator();
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(format!(
                                "统计：新增 {} 行 | 删除 {} 行 | 相似度：{:.1}%",
                                result.added_count,
                                result.removed_count,
                                result.similarity * 100.0
                            ))
                            .color(dim_color),
                        );
                    });
                });

            // 剩余空间用于内容区域
            ui.vertical(|ui| {
                // 获取语法高亮参数
                let syntax_name = self.get_syntax_name();
                let is_dark_mode = self.is_dark_mode(ui);

                // Unified 视图
                egui::ScrollArea::both()
                    .auto_shrink([false, false])  // 不自动缩小，内容不足时不显示滚动条
                    .id_salt("diff_unified_view")
                    .show(ui, |ui| {
                        // 获取当前字体大小
                        let font_size = ui.style().text_styles.get(&egui::TextStyle::Monospace)
                            .map(|font_id| font_id.size)
                            .unwrap_or(14.0);

                        // 设置等宽字体
                        let code_style = egui::TextStyle::Monospace;
                        let row_height = ui.text_style_height(&code_style) + 4.0;

                        for line in &result.unified_lines {
                            let bg_color = match line.diff_type {
                                DiffType::Added => Color32::from_rgb(220, 255, 220),
                                DiffType::Removed => Color32::from_rgb(255, 220, 220),
                                DiffType::Equal => Color32::TRANSPARENT,
                            };

                            let line_num = match (line.line_number_left, line.line_number_right) {
                                (Some(l), Some(r)) => format!("{:>4} {:>4} │ ", l, r),
                                (Some(l), None) => format!("{:>4}      │ ", l),
                                (None, Some(r)) => format!("     {:>4} │ ", r),
                                _ => "           │ ".to_string(),
                            };

                            let prefix = match line.diff_type {
                                DiffType::Added => "+ ",
                                DiffType::Removed => "- ",
                                DiffType::Equal => "  ",
                            };

                            // 绘制背景
                            let rect = ui.available_rect_before_wrap();
                            let rect = egui::Rect::from_min_size(
                                rect.min,
                                egui::vec2(rect.width(), row_height),
                            );
                            ui.painter().rect_filled(rect, 0.0, bg_color);

                            // 相同行使用语法高亮，差异行使用差异颜色
                            if line.diff_type == DiffType::Equal && syntax_name.is_some() {
                                // 相同行使用语法高亮
                                let mut job = LayoutJob::default();
                                let full_prefix = format!("{}{}", line_num, prefix);
                                job.append(
                                    &full_prefix,
                                    0.0,
                                    egui::TextFormat {
                                        font_id: egui::FontId::monospace(font_size),
                                        color: dim_color,
                                        ..Default::default()
                                    },
                                );
                                let highlighted = self.highlighter.highlight_line(&line.content, syntax_name.as_deref(), font_size, is_dark_mode);
                                for (color, text) in highlighted {
                                    job.append(
                                        &text,
                                        0.0,
                                        egui::TextFormat {
                                            font_id: egui::FontId::monospace(font_size),
                                            color,
                                            ..Default::default()
                                        },
                                    );
                                }
                                ui.label(job);
                            } else {
                                // 差异行使用差异颜色
                                let text = format!("{}{}{}", line_num, prefix, line.content);
                                let rich_text = RichText::new(text).monospace();

                                let rich_text = match line.diff_type {
                                    DiffType::Added => rich_text.color(Color32::from_rgb(0, 150, 0)),
                                    DiffType::Removed => rich_text.color(Color32::from_rgb(180, 0, 0)),
                                    _ => rich_text.color(text_color),
                                };

                                ui.label(rich_text);
                            }
                        }
                    });
            });
        }
    }
}
