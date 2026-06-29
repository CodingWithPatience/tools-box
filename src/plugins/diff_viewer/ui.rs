use std::cell::{Cell, RefCell};
use std::hash::{DefaultHasher, Hash, Hasher};

use egui::{Color32, RichText, text::LayoutJob};

use super::differ;
use super::highlight::SyntaxHighlighter;
use super::models::{DiffResult, DiffType, SplitLine, TextSegment, ViewMode};

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

type HighlightCache = RefCell<Option<(u64, LayoutJob)>>;

pub struct DiffViewerUi {
    left_text: String,
    right_text: String,
    left_file_name: Option<String>,
    right_file_name: Option<String>,
    view_mode: ViewMode,
    diff_result: Option<DiffResult>,
    error: Option<String>,
    highlighter: SyntaxHighlighter,
    selected_language: String,
    left_highlight_cache: HighlightCache,
    right_highlight_cache: HighlightCache,
    last_split_left_y: Cell<f32>,
    last_split_right_y: Cell<f32>,
    pending_split_sync_y: Cell<Option<f32>>,
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
            last_split_left_y: Cell::new(0.0),
            last_split_right_y: Cell::new(0.0),
            pending_split_sync_y: Cell::new(None),
        }
    }

    pub fn render(&mut self, ui: &mut egui::Ui) {
        match self.view_mode {
            ViewMode::Edit => self.render_edit_mode(ui),
            ViewMode::Split => self.render_split_view(ui),
            ViewMode::Unified => self.render_unified_view(ui),
        }
    }

    // ===================================================================
    // 编辑模式
    // ===================================================================

    fn render_edit_mode(&mut self, ui: &mut egui::Ui) {
        ui.heading("📝 文本对比工具");
        ui.separator();

        let available_height = ui.available_height() - 120.0;
        let syntax_name = self.get_syntax_name();
        let is_dark_mode = ui.visuals().dark_mode;
        let font_size = ui
            .style()
            .text_styles
            .get(&egui::TextStyle::Monospace)
            .map(|font_id| font_id.size)
            .unwrap_or(14.0);

        let left_lines = self.left_text.lines().count().max(1);
        let right_lines = self.right_text.lines().count().max(1);
        let max_lines = left_lines.max(right_lines);
        let line_num_digits = format!("{}", max_lines).len().max(3);
        let gutter_char_count = line_num_digits + 3;
        let gutter_width = gutter_char_count as f32 * font_size * 0.6;
        let mono_font_id = egui::FontId::monospace(font_size);
        let raw_line_height = ui.fonts(|f| f.row_height(&mono_font_id));
        let pixels_per_point = ui.pixels_per_point();
        let line_height = (raw_line_height * pixels_per_point).round() / pixels_per_point;
        const TEXTEDIT_MARGIN: egui::Margin = egui::Margin::symmetric(4, 2);
        let text_edit_margin_top = TEXTEDIT_MARGIN.top as f32;

        ui.columns(2, |columns| {
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
                let highlighter = &self.highlighter;
                let cache = &self.left_highlight_cache;
                let line_count = self.left_text.lines().count();
                ui.horizontal(|ui| {
                    let gutter_origin = ui.cursor().left_top();
                    ui.allocate_space(egui::vec2(gutter_width, available_height));
                    egui::ScrollArea::both()
                        .id_salt("diff_edit_left")
                        .max_height(available_height)
                        .show(ui, |ui| {
                            let mut left_layouter =
                                |ui: &egui::Ui, string: &str, _wrap_width: f32| {
                                    Self::highlight_text_with_cache(
                                        highlighter, cache, string, &syntax_name,
                                        is_dark_mode, font_size, f32::INFINITY, ui,
                                    )
                                };
                            egui::TextEdit::multiline(&mut self.left_text)
                                .hint_text("在此输入原始文本...")
                                .layouter(&mut left_layouter)
                                .desired_width(f32::INFINITY)
                                .min_size(egui::vec2(0.0, available_height))
                                .margin(TEXTEDIT_MARGIN)
                                .show(ui);
                        });
                    let scroll_id = ui.make_persistent_id(egui::Id::new("diff_edit_left"));
                    let offset_y = egui::scroll_area::State::load(ui.ctx(), scroll_id)
                        .map(|s| s.offset.y)
                        .unwrap_or(0.0);
                    Self::render_gutter(
                        ui, gutter_origin, gutter_width, available_height,
                        line_count, line_num_digits, line_height, offset_y,
                        font_size, text_edit_margin_top,
                    );
                });
            });

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
                let highlighter = &self.highlighter;
                let cache = &self.right_highlight_cache;
                let line_count = self.right_text.lines().count();
                ui.horizontal(|ui| {
                    let gutter_origin = ui.cursor().left_top();
                    ui.allocate_space(egui::vec2(gutter_width, available_height));
                    egui::ScrollArea::both()
                        .id_salt("diff_edit_right")
                        .max_height(available_height)
                        .show(ui, |ui| {
                            let mut right_layouter =
                                |ui: &egui::Ui, string: &str, _wrap_width: f32| {
                                    Self::highlight_text_with_cache(
                                        highlighter, cache, string, &syntax_name,
                                        is_dark_mode, font_size, f32::INFINITY, ui,
                                    )
                                };
                            egui::TextEdit::multiline(&mut self.right_text)
                                .hint_text("在此输入对比文本...")
                                .layouter(&mut right_layouter)
                                .desired_width(f32::INFINITY)
                                .min_size(egui::vec2(0.0, available_height))
                                .margin(TEXTEDIT_MARGIN)
                                .show(ui);
                        });
                    let scroll_id = ui.make_persistent_id(egui::Id::new("diff_edit_right"));
                    let offset_y = egui::scroll_area::State::load(ui.ctx(), scroll_id)
                        .map(|s| s.offset.y)
                        .unwrap_or(0.0);
                    Self::render_gutter(
                        ui, gutter_origin, gutter_width, available_height,
                        line_count, line_num_digits, line_height, offset_y,
                        font_size, text_edit_margin_top,
                    );
                });
            });
        });

        ui.add_space(10.0);

        if let Some(err) = &self.error {
            ui.label(RichText::new(err).color(Color32::RED));
            ui.add_space(5.0);
        }

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
            if ui.button("📊 开始对比").clicked() {
                self.diff_result = Some(differ::compute_diff(&self.left_text, &self.right_text));
                self.view_mode = ViewMode::Split;
                self.last_split_left_y.set(0.0);
                self.last_split_right_y.set(0.0);
                self.pending_split_sync_y.set(None);
            }
        });
    }

    // ===================================================================
    // Split 视图
    // ===================================================================

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

        let Some(result) = &self.diff_result else {
            ui.label("暂无对比结果");
            return;
        };

        // 底部统计栏
        let text_style = egui::TextStyle::Small;
        let stats_height = ui.text_style_height(&text_style) + 16.0;
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

        let dim_color = Color32::from_rgb(128, 128, 128);

        ui.vertical(|ui| {

            let row_height = ui.text_style_height(&egui::TextStyle::Monospace) + 4.0;
            let font_size = ui
                .style()
                .text_styles
                .get(&egui::TextStyle::Monospace)
                .map(|font_id| font_id.size)
                .unwrap_or(14.0);
            let syntax_name = self.get_syntax_name();
            let is_dark_mode = ui.visuals().dark_mode;
            let text_color = ui.visuals().text_color();

            // 行号位数
            let max_left_num = result.split_lines.iter().filter_map(|l| l.left_line_number).max().unwrap_or(1);
            let max_right_num = result.split_lines.iter().filter_map(|l| l.right_line_number).max().unwrap_or(1);
            let num_digits = format!("{}", max_left_num.max(max_right_num)).len().max(3);
            let gutter_w = ((num_digits + 3) as f32 * font_size * 0.6).max(40.0);

            let available_size = ui.available_size_before_wrap();
            let col_width = (available_size.x / 2.0).max(100.0);

            // 读取待同步目标
            let sync_target = self.pending_split_sync_y.get().unwrap_or(0.0);

            // 以闭包形式记录输出状态（避免 borrow 冲突）
            let left_y_ref: Cell<f32> = Cell::new(0.0);
            let right_y_ref: Cell<f32> = Cell::new(0.0);

            ui.horizontal(|ui| {
                // ===== 左面板 =====
                ui.allocate_ui_with_layout(
                    egui::vec2(col_width, available_size.y),
                    egui::Layout::top_down(egui::Align::LEFT),
                    |ui| {
                        ui.label(RichText::new("原始文本").strong().color(dim_color));
                        let mut scroll = egui::ScrollArea::both()
                            .id_salt("split_left")
                            .auto_shrink([false, false]);
                        if sync_target > 0.5 {
                            scroll = scroll.vertical_scroll_offset(sync_target);
                        }
                        let output = scroll.show(ui, |ui| {
                            for line in &result.split_lines {
                                ui.horizontal(|ui| {
                                    let num_text = match line.left_line_number {
                                        Some(n) => format!("{:>w$}", n, w = num_digits),
                                        None => " ".repeat(num_digits),
                                    };
                                    ui.add_sized(
                                        [gutter_w, row_height],
                                        egui::Label::new(
                                            RichText::new(format!("{} │ ", num_text))
                                                .monospace()
                                                .color(dim_color),
                                        ),
                                    );
                                    ui.allocate_ui_with_layout(
                                        egui::vec2(col_width - gutter_w, row_height),
                                        egui::Layout::left_to_right(egui::Align::Min),
                                        |ui| {
                                            self.render_cell(
                                                ui, line, true, row_height, font_size,
                                                &syntax_name, is_dark_mode, text_color,
                                            );
                                        },
                                    );
                                });
                            }
                        });
                        left_y_ref.set(output.state.offset.y);
                    },
                );

                ui.separator();

                // ===== 右面板 =====
                ui.allocate_ui_with_layout(
                    egui::vec2(col_width, available_size.y),
                    egui::Layout::top_down(egui::Align::LEFT),
                    |ui| {
                        ui.label(RichText::new("对比文本").strong().color(dim_color));
                        let mut scroll = egui::ScrollArea::both()
                            .id_salt("split_right")
                            .auto_shrink([false, false]);
                        if sync_target > 0.5 {
                            scroll = scroll.vertical_scroll_offset(sync_target);
                        }
                        let output = scroll.show(ui, |ui| {
                            for line in &result.split_lines {
                                ui.horizontal(|ui| {
                                    let num_text = match line.right_line_number {
                                        Some(n) => format!("{:>w$}", n, w = num_digits),
                                        None => " ".repeat(num_digits),
                                    };
                                    ui.add_sized(
                                        [gutter_w, row_height],
                                        egui::Label::new(
                                            RichText::new(format!("{} │ ", num_text))
                                                .monospace()
                                                .color(dim_color),
                                        ),
                                    );
                                    ui.allocate_ui_with_layout(
                                        egui::vec2(col_width - gutter_w, row_height),
                                        egui::Layout::left_to_right(egui::Align::Min),
                                        |ui| {
                                            self.render_cell(
                                                ui, line, false, row_height, font_size,
                                                &syntax_name, is_dark_mode, text_color,
                                            );
                                        },
                                    );
                                });
                            }
                        });
                        right_y_ref.set(output.state.offset.y);
                    },
                );
            });

            // 检测滚动变化并设置同步目标
            let left_y = left_y_ref.get();
            let right_y = right_y_ref.get();
            let last_left = self.last_split_left_y.get();
            let last_right = self.last_split_right_y.get();

            let left_changed = (left_y - last_left).abs() > 0.5;
            let right_changed = (right_y - last_right).abs() > 0.5;

            if left_changed && !right_changed {
                self.pending_split_sync_y.set(Some(left_y));
            } else if right_changed && !left_changed {
                self.pending_split_sync_y.set(Some(right_y));
            } else {
                self.pending_split_sync_y.set(None);
            }

            self.last_split_left_y.set(left_y);
            self.last_split_right_y.set(right_y);
        });
    }

    fn render_cell(
        &self,
        ui: &mut egui::Ui,
        line: &SplitLine,
        is_left: bool,
        _row_height: f32,
        font_size: f32,
        syntax_name: &Option<String>,
        is_dark_mode: bool,
        text_color: Color32,
    ) {
        let (content, diff_type, segments) = if is_left {
            (&line.left_content, &line.left_type, &line.left_segments)
        } else {
            (&line.right_content, &line.right_type, &line.right_segments)
        };

        let bg = match diff_type {
            DiffType::Removed if is_left => Color32::from_rgb(255, 220, 220),
            DiffType::Added if !is_left => Color32::from_rgb(220, 255, 220),
            _ => Color32::TRANSPARENT,
        };

        if bg != Color32::TRANSPARENT {
            let rect = ui.max_rect();
            ui.painter().rect_filled(rect, 0.0, bg);
        }

        if let Some(text) = content {
            if *diff_type == DiffType::Equal && syntax_name.is_some() {
                let mut job = LayoutJob::default();
                job.wrap.max_width = f32::INFINITY;
                let highlighted = self.highlighter.highlight_line(
                    text, syntax_name.as_deref(), font_size, is_dark_mode,
                );
                for (color, t) in highlighted {
                    job.append(
                        &t, 0.0,
                        egui::TextFormat {
                            font_id: egui::FontId::monospace(font_size),
                            color,
                            ..Default::default()
                        },
                    );
                }
                ui.label(job);
            } else if !segments.is_empty() {
                let mut job = self.create_segment_layout(segments, font_size, text_color);
                job.wrap.max_width = f32::INFINITY;
                ui.label(job);
            } else {
                let color = match diff_type {
                    DiffType::Removed => Color32::from_rgb(180, 0, 0),
                    DiffType::Added => Color32::from_rgb(0, 150, 0),
                    _ => text_color,
                };
                let mut job = LayoutJob::default();
                job.wrap.max_width = f32::INFINITY;
                job.append(
                    text.as_str(), 0.0,
                    egui::TextFormat {
                        font_id: egui::FontId::monospace(font_size),
                        color,
                        ..Default::default()
                    },
                );
                ui.label(job);
            }
        }
    }

    fn create_segment_layout(
        &self,
        segments: &[TextSegment],
        font_size: f32,
        text_color: Color32,
    ) -> LayoutJob {
        let mut job = LayoutJob::default();
        job.wrap.max_width = f32::INFINITY;
        for segment in segments {
            let color = match segment.diff_type {
                DiffType::Equal => text_color,
                DiffType::Added => Color32::from_rgb(0, 150, 0),
                DiffType::Removed => Color32::from_rgb(180, 0, 0),
            };
            let bg_color = match segment.diff_type {
                DiffType::Added => Color32::from_rgb(180, 255, 180),
                DiffType::Removed => Color32::from_rgb(255, 180, 180),
                DiffType::Equal => Color32::TRANSPARENT,
            };
            job.append(
                &segment.text, 0.0,
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

    // ===================================================================
    // Unified 视图
    // ===================================================================

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

        let Some(result) = &self.diff_result else {
            ui.label("暂无对比结果");
            return;
        };

        let text_style = egui::TextStyle::Small;
        let stats_height = ui.text_style_height(&text_style) + 16.0;
        let text_color = ui.visuals().text_color();
        let dim_color = Color32::from_rgb(128, 128, 128);

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

        ui.vertical(|ui| {
            let row_height = ui.text_style_height(&egui::TextStyle::Monospace) + 4.0;
            let font_size = ui
                .style()
                .text_styles
                .get(&egui::TextStyle::Monospace)
                .map(|font_id| font_id.size)
                .unwrap_or(14.0);
            let syntax_name = self.get_syntax_name();
            let is_dark_mode = ui.visuals().dark_mode;

            let max_left_num = result.unified_lines.iter().filter_map(|l| l.line_number_left).max().unwrap_or(1);
            let max_right_num = result.unified_lines.iter().filter_map(|l| l.line_number_right).max().unwrap_or(1);
            let num_digits = format!("{}", max_left_num.max(max_right_num)).len().max(3);
            let gutter_w = ((num_digits * 2 + 4) as f32 * font_size * 0.6).max(80.0);

            let available_height = ui.available_height() - 10.0;

            egui::ScrollArea::both()
                .auto_shrink([false, false])
                .id_salt("unified_scroll")
                .max_height(available_height)
                .show(ui, |ui| {
                    for line in &result.unified_lines {
                        ui.horizontal(|ui| {
                            let left_num = match line.line_number_left {
                                Some(n) => format!("{:>w$}", n, w = num_digits),
                                None => " ".repeat(num_digits),
                            };
                            let right_num = match line.line_number_right {
                                Some(n) => format!("{:>w$}", n, w = num_digits),
                                None => " ".repeat(num_digits),
                            };
                            ui.add_sized(
                                [gutter_w, row_height],
                                egui::Label::new(
                                    RichText::new(format!("{} {} │ ", left_num, right_num))
                                        .monospace()
                                        .color(dim_color),
                                ),
                            );

                            let bg = match line.diff_type {
                                DiffType::Added => Color32::from_rgb(220, 255, 220),
                                DiffType::Removed => Color32::from_rgb(255, 220, 220),
                                DiffType::Equal => Color32::TRANSPARENT,
                            };
                            let prefix = match line.diff_type {
                                DiffType::Added => "+ ",
                                DiffType::Removed => "- ",
                                DiffType::Equal => "  ",
                            };

                            ui.allocate_ui_with_layout(
                                egui::vec2(ui.available_width(), row_height),
                                egui::Layout::left_to_right(egui::Align::Min),
                                |ui| {
                                    if bg != Color32::TRANSPARENT {
                                        let rect = ui.max_rect();
                                        ui.painter().rect_filled(rect, 0.0, bg);
                                    }
                                    if line.diff_type == DiffType::Equal && syntax_name.is_some() {
                                        let mut job = LayoutJob::default();
                                        job.wrap.max_width = f32::INFINITY;
                                        job.append(
                                            prefix, 0.0,
                                            egui::TextFormat {
                                                font_id: egui::FontId::monospace(font_size),
                                                color: dim_color,
                                                ..Default::default()
                                            },
                                        );
                                        let highlighted = self.highlighter.highlight_line(
                                            &line.content, syntax_name.as_deref(), font_size, is_dark_mode,
                                        );
                                        for (color, t) in highlighted {
                                            job.append(
                                                &t, 0.0,
                                                egui::TextFormat {
                                                    font_id: egui::FontId::monospace(font_size),
                                                    color,
                                                    ..Default::default()
                                                },
                                            );
                                        }
                                        ui.label(job);
                                    } else {
                                        let text = format!("{}{}", prefix, line.content);
                                        let color = match line.diff_type {
                                            DiffType::Added => Color32::from_rgb(0, 150, 0),
                                            DiffType::Removed => Color32::from_rgb(180, 0, 0),
                                            _ => text_color,
                                        };
                                        let mut job = LayoutJob::default();
                                        job.wrap.max_width = f32::INFINITY;
                                        job.append(
                                            &text, 0.0,
                                            egui::TextFormat {
                                                font_id: egui::FontId::monospace(font_size),
                                                color,
                                                ..Default::default()
                                            },
                                        );
                                        ui.label(job);
                                    }
                                },
                            );
                        });
                    }
                });
        });
    }

    // ===================================================================
    // 工具方法
    // ===================================================================

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
        margin_top: f32,
    ) {
        let gutter_rect = egui::Rect::from_min_size(origin, egui::vec2(width, height));
        let painter = ui.painter().with_clip_rect(gutter_rect);
        let text_color = Color32::from_rgb(128, 128, 128);
        let font_id = egui::FontId::monospace(font_size);
        let first_visible = ((scroll_offset_y - margin_top) / line_height).floor().max(0.0) as usize;
        let visible_count = ((height - margin_top) / line_height).ceil().max(0.0) as usize + 1;
        let last_visible = (first_visible + visible_count).min(line_count);
        let frac_offset = scroll_offset_y - first_visible as f32 * line_height;
        for i in first_visible..last_visible {
            let y = origin.y + margin_top + (i - first_visible) as f32 * line_height - frac_offset;
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

    fn get_syntax_name(&self) -> Option<String> {
        if self.selected_language == "自动检测" {
            self.left_file_name
                .as_deref()
                .or(self.right_file_name.as_deref())
                .and_then(|name| {
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

    fn clear_cache(&self) {
        self.left_highlight_cache.borrow_mut().take();
        self.right_highlight_cache.borrow_mut().take();
    }
}
