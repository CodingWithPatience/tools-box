use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};

use egui::{Color32, RichText, text::LayoutJob};

use super::differ;
use crate::utils::highlight::SyntaxHighlighter;
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
/// 行级高亮缓存：key 是文本内容的 hash，value 是 LayoutJob
type LineHighlightCache = RefCell<HashMap<u64, LayoutJob>>;

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
    /// Unified 视图行级高亮缓存
    unified_line_cache: LineHighlightCache,
    /// 上一帧左面板的纵向滚动偏移量
    last_left_offset: Cell<f32>,
    /// 上一帧右面板的纵向滚动偏移量
    last_right_offset: Cell<f32>,
    /// 待同步到左面板的纵向偏移量（右面板被用户滚动时设置）
    pending_sync_left: Cell<Option<f32>>,
    /// 待同步到右面板的纵向偏移量（左面板被用户滚动时设置）
    pending_sync_right: Cell<Option<f32>>,
    /// 上一帧左面板的横向滚动偏移量
    last_left_offset_x: Cell<f32>,
    /// 上一帧右面板的横向滚动偏移量
    last_right_offset_x: Cell<f32>,
    /// 待同步到左面板的横向偏移量（右面板被用户滚动时设置）
    pending_sync_left_x: Cell<Option<f32>>,
    /// 待同步到右面板的横向偏移量（左面板被用户滚动时设置）
    pending_sync_right_x: Cell<Option<f32>>,
    /// 上一帧 Unified 视图的纵向滚动偏移量
    last_unified_offset: Cell<f32>,
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
            unified_line_cache: RefCell::new(HashMap::new()),
            last_left_offset: Cell::new(0.0),
            last_right_offset: Cell::new(0.0),
            pending_sync_left: Cell::new(None),
            pending_sync_right: Cell::new(None),
            last_left_offset_x: Cell::new(0.0),
            last_right_offset_x: Cell::new(0.0),
            pending_sync_left_x: Cell::new(None),
            pending_sync_right_x: Cell::new(None),
            last_unified_offset: Cell::new(0.0),
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
                self.last_left_offset.set(0.0);
                self.last_right_offset.set(0.0);
                self.pending_sync_left.set(None);
                self.pending_sync_right.set(None);
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
            let font_size = ui
                .style()
                .text_styles
                .get(&egui::TextStyle::Monospace)
                .map(|f| f.size)
                .unwrap_or(14.0);
            let font_id = egui::FontId::monospace(font_size);
            let row_height = ui.fonts(|f| f.row_height(&font_id)) + 4.0;
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

            // 读取上一帧的待同步偏移量
            let sync_left = self.pending_sync_left.get();
            let sync_right = self.pending_sync_right.get();
            let sync_left_x = self.pending_sync_left_x.get();
            let sync_right_x = self.pending_sync_right_x.get();

            // 清除 pending，准备为当前帧设置新的同步
            self.pending_sync_left.set(None);
            self.pending_sync_right.set(None);
            self.pending_sync_left_x.set(None);
            self.pending_sync_right_x.set(None);

            // 用于记录当前帧的面板偏移量
            let left_current_offset: Cell<f32> = Cell::new(0.0);
            let right_current_offset: Cell<f32> = Cell::new(0.0);
            let left_current_offset_x: Cell<f32> = Cell::new(0.0);
            let right_current_offset_x: Cell<f32> = Cell::new(0.0);
            // 记录面板是否可滚动（内容高度超过视口高度）
            let left_scrollable: Cell<bool> = Cell::new(false);
            let right_scrollable: Cell<bool> = Cell::new(false);
            // 标记是否因同步而应用了偏移（用于 ignoreChange 模式）
            let left_sync_applied: Cell<bool> = Cell::new(false);
            let left_sync_applied_x: Cell<bool> = Cell::new(false);

            ui.horizontal(|ui| {
                // ===== 左面板（先渲染，获取滚动位置） =====
                ui.allocate_ui_with_layout(
                    egui::vec2(col_width, available_size.y),
                    egui::Layout::top_down(egui::Align::LEFT),
                    |ui| {
                        ui.label(RichText::new("原始文本").strong().color(dim_color));
                        let content_height = available_size.y - 30.0;
                        // 计算行号区域的纵向偏移量
                        // 优先使用来自右面板的同步值，否则使用当前帧的内容偏移量
                        let gutter_offset_y = sync_left.unwrap_or(self.last_left_offset.get());
                        ui.horizontal(|ui| {
                            // 行号区域（固定宽度，隐藏滚动条）
                            ui.allocate_ui_with_layout(
                                egui::vec2(gutter_w, content_height),
                                egui::Layout::top_down(egui::Align::LEFT),
                                |ui| {
                                    let mut gutter_scroll = egui::ScrollArea::vertical()
                                        .id_salt("split_left_gutter")
                                        .auto_shrink([false, false])
                                        .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden);
                                    gutter_scroll = gutter_scroll.vertical_scroll_offset(gutter_offset_y);
                                    gutter_scroll.show(ui, |ui| {
                                        ui.spacing_mut().item_spacing.y = 0.0;
                                        for line in &result.split_lines {
                                            let gutter_bg = match line.left_type {
                                                DiffType::Removed => if is_dark_mode {
                                                    Color32::from_rgba_unmultiplied(80, 40, 45, 200)
                                                } else {
                                                    Color32::from_rgba_unmultiplied(255, 180, 185, 240)
                                                },
                                                _ => Color32::TRANSPARENT,
                                            };
                                            let symbol = match line.left_type {
                                                DiffType::Removed => "-",
                                                _ => " ",
                                            };
                                            ui.allocate_ui_with_layout(
                                                egui::vec2(gutter_w, row_height),
                                                egui::Layout::left_to_right(egui::Align::Min),
                                                |ui| {
                                                    if gutter_bg != Color32::TRANSPARENT {
                                                        let rect = ui.max_rect();
                                                        ui.painter().rect_filled(rect, 0.0, gutter_bg);
                                                    }
                                                    let num_text = match line.left_line_number {
                                                        Some(n) => format!("{:>w$}", n, w = num_digits),
                                                        None => " ".repeat(num_digits),
                                                    };
                                                    ui.add_sized(
                                                        [gutter_w, row_height],
                                                        egui::Label::new(
                                                            RichText::new(format!("{} {} ", num_text, symbol))
                                                                .monospace()
                                                                .color(dim_color),
                                                        ),
                                                    );
                                                },
                                            );
                                        }
                                    });
                                },
                            );

                            // 内容区域（可横向和纵向滚动）
                            let content_width = col_width - gutter_w;
                            ui.allocate_ui_with_layout(
                                egui::vec2(content_width, content_height),
                                egui::Layout::top_down(egui::Align::LEFT),
                                |ui| {
                                    let mut content_scroll = egui::ScrollArea::both()
                                        .id_salt("split_left_content")
                                        .auto_shrink([false, false]);
                                    // 应用上一帧右面板的同步偏移量（纵向）
                                    if let Some(offset_y) = sync_left {
                                        content_scroll = content_scroll.vertical_scroll_offset(offset_y);
                                        left_sync_applied.set(true);
                                    }
                                    // 应用上一帧右面板的同步偏移量（横向）
                                    if let Some(offset_x) = sync_left_x {
                                        content_scroll = content_scroll.horizontal_scroll_offset(offset_x);
                                        left_sync_applied_x.set(true);
                                    }
                                    let output = content_scroll.show(ui, |ui| {
                                        ui.spacing_mut().item_spacing.y = 0.0;
                                        for (line_index, line) in result.split_lines.iter().enumerate() {
                                            let line_bg = match line.left_type {
                                                DiffType::Removed => if is_dark_mode {
                                                    Color32::from_rgba_unmultiplied(61, 31, 35, 180)
                                                } else {
                                                    Color32::from_rgba_unmultiplied(255, 210, 215, 230)
                                                },
                                                _ => Color32::TRANSPARENT,
                                            };
                                            ui.allocate_ui_with_layout(
                                                egui::vec2(content_width, row_height),
                                                egui::Layout::left_to_right(egui::Align::Center),
                                                |ui| {
                                                    if line_bg != Color32::TRANSPARENT {
                                                        let mut rect = ui.max_rect();
                                                        rect.set_width(rect.width().max(2000.0));
                                                        ui.painter().rect_filled(rect, 0.0, line_bg);
                                                    }
                                                    self.render_cell(
                                                        ui, line, true, row_height, font_size,
                                                        &syntax_name, is_dark_mode, text_color, line_index,
                                                    );
                                                },
                                            );
                                        }
                                    });
                                    left_current_offset.set(output.state.offset.y);
                                    left_current_offset_x.set(output.state.offset.x);
                                    left_scrollable.set(output.content_size.y > output.inner_rect.height());
                                    // 检测纵向滚动变化
                                    let left_changed = !left_sync_applied.get()
                                        && (output.state.offset.y - self.last_left_offset.get()).abs() > 0.5;
                                    if left_changed {
                                        self.pending_sync_right.set(Some(output.state.offset.y));
                                    }
                                    // 检测横向滚动变化
                                    let left_changed_x = !left_sync_applied_x.get()
                                        && (output.state.offset.x - self.last_left_offset_x.get()).abs() > 0.5;
                                    if left_changed_x {
                                        self.pending_sync_right_x.set(Some(output.state.offset.x));
                                    }
                                },
                            );
                        });
                    },
                );

                ui.separator();

                // ===== 右面板（同帧同步左面板的滚动位置） =====
                ui.allocate_ui_with_layout(
                    egui::vec2(col_width, available_size.y),
                    egui::Layout::top_down(egui::Align::LEFT),
                    |ui| {
                        ui.label(RichText::new("对比文本").strong().color(dim_color));
                        let content_height = available_size.y - 30.0;
                        // 计算行号区域的纵向偏移量
                        // 优先使用来自左面板的同步值，否则使用当前帧的内容偏移量
                        let gutter_offset_y = sync_right.unwrap_or(self.last_right_offset.get());
                        ui.horizontal(|ui| {
                            // 行号区域（固定宽度，隐藏滚动条）
                            ui.allocate_ui_with_layout(
                                egui::vec2(gutter_w, content_height),
                                egui::Layout::top_down(egui::Align::LEFT),
                                |ui| {
                                    let mut gutter_scroll = egui::ScrollArea::vertical()
                                        .id_salt("split_right_gutter")
                                        .auto_shrink([false, false])
                                        .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden);
                                    gutter_scroll = gutter_scroll.vertical_scroll_offset(gutter_offset_y);
                                    gutter_scroll.show(ui, |ui| {
                                        ui.spacing_mut().item_spacing.y = 0.0;
                                        for line in &result.split_lines {
                                            let gutter_bg = match line.right_type {
                                                DiffType::Added => if is_dark_mode {
                                                    Color32::from_rgba_unmultiplied(40, 80, 50, 200)
                                                } else {
                                                    Color32::from_rgba_unmultiplied(150, 230, 170, 240)
                                                },
                                                _ => Color32::TRANSPARENT,
                                            };
                                            let symbol = match line.right_type {
                                                DiffType::Added => "+",
                                                _ => " ",
                                            };
                                            ui.allocate_ui_with_layout(
                                                egui::vec2(gutter_w, row_height),
                                                egui::Layout::left_to_right(egui::Align::Min),
                                                |ui| {
                                                    if gutter_bg != Color32::TRANSPARENT {
                                                        let rect = ui.max_rect();
                                                        ui.painter().rect_filled(rect, 0.0, gutter_bg);
                                                    }
                                                    let num_text = match line.right_line_number {
                                                        Some(n) => format!("{:>w$}", n, w = num_digits),
                                                        None => " ".repeat(num_digits),
                                                    };
                                                    ui.add_sized(
                                                        [gutter_w, row_height],
                                                        egui::Label::new(
                                                            RichText::new(format!("{} {} ", num_text, symbol))
                                                                .monospace()
                                                                .color(dim_color),
                                                        ),
                                                    );
                                                },
                                            );
                                        }
                                    });
                                },
                            );

                            // 内容区域（可横向和纵向滚动）
                            let content_width = col_width - gutter_w;
                            ui.allocate_ui_with_layout(
                                egui::vec2(content_width, content_height),
                                egui::Layout::top_down(egui::Align::LEFT),
                                |ui| {
                                    let mut content_scroll = egui::ScrollArea::both()
                                        .id_salt("split_right_content")
                                        .auto_shrink([false, false]);
                                    // 应用上一帧左面板的同步偏移量（纵向）
                                    if let Some(offset_y) = sync_right {
                                        content_scroll = content_scroll.vertical_scroll_offset(offset_y);
                                    }
                                    // 应用上一帧左面板的同步偏移量（横向）
                                    if let Some(offset_x) = sync_right_x {
                                        content_scroll = content_scroll.horizontal_scroll_offset(offset_x);
                                    }
                                    let output = content_scroll.show(ui, |ui| {
                                        ui.spacing_mut().item_spacing.y = 0.0;
                                        for (line_index, line) in result.split_lines.iter().enumerate() {
                                            let line_bg = match line.right_type {
                                                DiffType::Added => if is_dark_mode {
                                                    Color32::from_rgba_unmultiplied(31, 61, 38, 180)
                                                } else {
                                                    Color32::from_rgba_unmultiplied(180, 240, 195, 230)
                                                },
                                                _ => Color32::TRANSPARENT,
                                            };
                                            ui.allocate_ui_with_layout(
                                                egui::vec2(content_width, row_height),
                                                egui::Layout::left_to_right(egui::Align::Center),
                                                |ui| {
                                                    if line_bg != Color32::TRANSPARENT {
                                                        let mut rect = ui.max_rect();
                                                        rect.set_width(rect.width().max(2000.0));
                                                        ui.painter().rect_filled(rect, 0.0, line_bg);
                                                    }
                                                    self.render_cell(
                                                        ui, line, false, row_height, font_size,
                                                        &syntax_name, is_dark_mode, text_color, line_index,
                                                    );
                                                },
                                            );
                                        }
                                    });
                                    right_current_offset.set(output.state.offset.y);
                                    right_current_offset_x.set(output.state.offset.x);
                                    right_scrollable.set(output.content_size.y > output.inner_rect.height());
                                    // 检测纵向滚动变化
                                    let right_changed = sync_right.is_none()
                                        && (output.state.offset.y - self.last_right_offset.get()).abs() > 0.5;
                                    if right_changed {
                                        self.pending_sync_left.set(Some(output.state.offset.y));
                                    }
                                    // 检测横向滚动变化
                                    let right_changed_x = sync_right_x.is_none()
                                        && (output.state.offset.x - self.last_right_offset_x.get()).abs() > 0.5;
                                    if right_changed_x {
                                        self.pending_sync_left_x.set(Some(output.state.offset.x));
                                    }
                                },
                            );
                        });
                    },
                );
            });
            // 纵向边界补正：当滚动到顶部时，强制另一边面板也对齐
            // 仅当两侧面板都可滚动时才触发补正，避免短内容面板误判
            let left_offset = left_current_offset.get();
            let right_offset = right_current_offset.get();
            let left_at_top = left_offset < 1.0;
            let right_at_top = right_offset < 1.0;
            if left_at_top && !right_at_top && right_scrollable.get() {
                // 左面板在顶部，右面板不在顶部且可滚动 → 强制右面板到顶部
                self.pending_sync_right.set(Some(0.0));
            } else if right_at_top && !left_at_top && left_scrollable.get() {
                // 右面板在顶部，左面板不在顶部且可滚动 → 强制左面板到顶部
                self.pending_sync_left.set(Some(0.0));
            }
            // 横向边界补正：当滚动到最左侧时，强制另一边面板也对齐
            let left_offset_x = left_current_offset_x.get();
            let right_offset_x = right_current_offset_x.get();
            let left_at_left = left_offset_x < 1.0;
            let right_at_left = right_offset_x < 1.0;
            if left_at_left && !right_at_left {
                self.pending_sync_right_x.set(Some(0.0));
            } else if right_at_left && !left_at_left {
                self.pending_sync_left_x.set(Some(0.0));
            }
            // 更新上一帧的偏移量
            self.last_left_offset.set(left_offset);
            self.last_right_offset.set(right_offset);
            self.last_left_offset_x.set(left_offset_x);
            self.last_right_offset_x.set(right_offset_x);
        });
    }

    fn render_cell(
        &self,
        ui: &mut egui::Ui,
        line: &SplitLine,
        is_left: bool,
        row_height: f32,
        font_size: f32,
        syntax_name: &Option<String>,
        is_dark_mode: bool,
        text_color: Color32,
        line_index: usize,
    ) {
        let (content, diff_type, segments) = if is_left {
            (&line.left_content, &line.left_type, &line.left_segments)
        } else {
            (&line.right_content, &line.right_type, &line.right_segments)
        };

        // 判断是否是整行删除/新增（只有一侧有内容，另一侧为空）
        let is_whole_line_change = (is_left && line.right_content.is_none())
            || (!is_left && line.left_content.is_none());

        if let Some(text) = content {
            if *diff_type == DiffType::Equal && syntax_name.is_some() {
                // 相同行使用语法高亮（带缓存）
                let mut job = self.get_line_highlight_job(
                    text,
                    syntax_name.as_deref(),
                    font_size,
                    is_dark_mode,
                );
                job.wrap.max_width = f32::INFINITY;
                ui.label(job);
            } else if !segments.is_empty() && !is_whole_line_change {
                // 有字符级差异的行（修改行）：在行级背景色基础上叠加字符级背景色
                let mut job = LayoutJob::default();
                job.wrap.max_width = f32::INFINITY;
                let (added_bg, removed_bg) = Self::diff_background_colors(is_dark_mode);
                self.append_diff_segments_to_job(
                    &mut job,
                    text,
                    segments,
                    syntax_name.as_deref(),
                    font_size,
                    is_dark_mode,
                    text_color,
                    added_bg,
                    removed_bg,
                );
                ui.label(job);
            } else {
                // 整行删除/新增或无字符级差异：只有行级背景色（已在外部绘制），这里只渲染文本
                if syntax_name.is_some() {
                    // 有语法高亮时，使用缓存
                    let mut job = self.get_line_highlight_job(
                        text,
                        syntax_name.as_deref(),
                        font_size,
                        is_dark_mode,
                    );
                    job.wrap.max_width = f32::INFINITY;
                    ui.label(job);
                } else {
                    // 无语法高亮时，直接渲染文本
                    let mut job = LayoutJob::default();
                    job.wrap.max_width = f32::INFINITY;
                    job.append(
                        text.as_str(), 0.0,
                        egui::TextFormat {
                            font_id: egui::FontId::monospace(font_size),
                            color: text_color,
                            ..Default::default()
                        },
                    );
                    ui.label(job);
                }
            }
        } else {
            // 空行（删除/新增行的对侧行）：绘制斜线背景，标识行不存在
            let mut rect = ui.max_rect();
            // 扩展宽度以覆盖横向滚动后的内容
            rect.set_width(rect.width().max(2000.0));
            let hatched_color = if is_dark_mode {
                Color32::from_rgba_unmultiplied(50, 50, 55, 255)
            } else {
                Color32::from_rgba_unmultiplied(180, 180, 190, 255)
            };
            Self::paint_hatched_background(
                ui.painter(),
                rect,
                line_index,
                row_height,
                hatched_color,
            );
            ui.allocate_space(egui::vec2(ui.available_width(), row_height));
        }
    }

    /// 计算字符级差异背景色（深色/浅色主题）
    ///
    /// 字符级背景色比行级背景色更深，用于区分修改行中的具体差异字符。
    fn diff_background_colors(is_dark_mode: bool) -> (Color32, Color32) {
        if is_dark_mode {
            (
                Color32::from_rgba_unmultiplied(53, 110, 53, 200),
                Color32::from_rgba_unmultiplied(110, 53, 53, 200),
            )
        } else {
            (
                Color32::from_rgba_unmultiplied(120, 220, 145, 220),
                Color32::from_rgba_unmultiplied(255, 150, 150, 220),
            )
        }
    }

    /// 向 LayoutJob 追加差异片段（带语法高亮和字符级背景色）
    fn append_diff_segments_to_job(
        &self,
        job: &mut LayoutJob,
        full_text: &str,
        segments: &[TextSegment],
        syntax_name: Option<&str>,
        font_size: f32,
        is_dark_mode: bool,
        text_color: Color32,
        added_bg: Color32,
        removed_bg: Color32,
    ) {
        if syntax_name.is_some() {
            let highlighted = self.highlighter.highlight_line(
                full_text, syntax_name, font_size, is_dark_mode,
            );

            let mut char_colors: Vec<Color32> = Vec::new();
            for (color, text) in &highlighted {
                for _ in text.chars() {
                    char_colors.push(*color);
                }
            }

            let mut char_offset = 0;
            for segment in segments {
                let seg_len = segment.text.chars().count();
                let bg = match segment.diff_type {
                    DiffType::Added => added_bg,
                    DiffType::Removed => removed_bg,
                    DiffType::Equal => Color32::TRANSPARENT,
                };

                let mut seg_text = String::new();
                let mut current_color = None;
                for (i, ch) in segment.text.chars().enumerate() {
                    let color_idx = char_offset + i;
                    let color = if color_idx < char_colors.len() {
                        char_colors[color_idx]
                    } else {
                        text_color
                    };

                    if Some(color) != current_color {
                        if !seg_text.is_empty() {
                            job.append(
                                &seg_text, 0.0,
                                egui::TextFormat {
                                    font_id: egui::FontId::monospace(font_size),
                                    color: current_color.unwrap_or(text_color),
                                    background: bg,
                                    ..Default::default()
                                },
                            );
                            seg_text = String::new();
                        }
                        current_color = Some(color);
                    }
                    seg_text.push(ch);
                }

                if !seg_text.is_empty() {
                    job.append(
                        &seg_text, 0.0,
                        egui::TextFormat {
                            font_id: egui::FontId::monospace(font_size),
                            color: current_color.unwrap_or(text_color),
                            background: bg,
                            ..Default::default()
                        },
                    );
                }

                char_offset += seg_len;
            }
        } else {
            for segment in segments {
                let color = match segment.diff_type {
                    DiffType::Equal => text_color,
                    DiffType::Added => Color32::from_rgb(0, 150, 0),
                    DiffType::Removed => Color32::from_rgb(180, 0, 0),
                };
                let bg = match segment.diff_type {
                    DiffType::Added => added_bg,
                    DiffType::Removed => removed_bg,
                    DiffType::Equal => Color32::TRANSPARENT,
                };
                job.append(
                    &segment.text, 0.0,
                    egui::TextFormat {
                        font_id: egui::FontId::monospace(font_size),
                        color,
                        background: bg,
                        ..Default::default()
                    },
                );
            }
        }
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
        let is_dark_mode = ui.visuals().dark_mode;

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
            let font_size = ui
                .style()
                .text_styles
                .get(&egui::TextStyle::Monospace)
                .map(|f| f.size)
                .unwrap_or(14.0);
            let font_id = egui::FontId::monospace(font_size);
            let row_height = ui.fonts(|f| f.row_height(&font_id)) + 4.0;
            let syntax_name = self.get_syntax_name();

            let max_left_num = result.unified_lines.iter().filter_map(|l| l.line_number_left).max().unwrap_or(1);
            let max_right_num = result.unified_lines.iter().filter_map(|l| l.line_number_right).max().unwrap_or(1);
            let num_digits = format!("{}", max_left_num.max(max_right_num)).len().max(3);
            // 双行号 + 分隔符的宽度
            let gutter_w = ((num_digits * 2 + 4) as f32 * font_size * 0.6).max(80.0);

            let available_height = ui.available_height() - 10.0;
            let available_width = ui.available_width();
            let content_width = (available_width - gutter_w).max(100.0);

            // 先渲染内容区域获取纵向偏移量，再用它同步行号
            // 使用一个 Cell 来存储内容区域的纵向偏移量
            let content_offset_y: Cell<f32> = Cell::new(self.last_unified_offset.get());

            ui.horizontal(|ui| {
                // 行号区域（固定宽度，隐藏滚动条）
                ui.allocate_ui_with_layout(
                    egui::vec2(gutter_w, available_height),
                    egui::Layout::top_down(egui::Align::LEFT),
                    |ui| {
                        let mut gutter_scroll = egui::ScrollArea::vertical()
                            .id_salt("unified_gutter")
                            .auto_shrink([false, false])
                            .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden);
                        // 使用内容区域的偏移量同步行号
                        gutter_scroll = gutter_scroll.vertical_scroll_offset(content_offset_y.get());
                        gutter_scroll.show(ui, |ui| {
                            ui.spacing_mut().item_spacing.y = 0.0;
                            for line in &result.unified_lines {
                                let (gutter_bg, symbol) = match line.diff_type {
                                    DiffType::Removed => {
                                        let gutter_bg = if is_dark_mode {
                                            Color32::from_rgba_unmultiplied(80, 40, 45, 200)
                                        } else {
                                            Color32::from_rgba_unmultiplied(255, 180, 185, 240)
                                        };
                                        (gutter_bg, "-")
                                    }
                                    DiffType::Added => {
                                        let gutter_bg = if is_dark_mode {
                                            Color32::from_rgba_unmultiplied(40, 80, 50, 200)
                                        } else {
                                            Color32::from_rgba_unmultiplied(150, 230, 170, 240)
                                        };
                                        (gutter_bg, "+")
                                    }
                                    DiffType::Equal => (Color32::TRANSPARENT, " "),
                                };
                                ui.allocate_ui_with_layout(
                                    egui::vec2(gutter_w, row_height),
                                    egui::Layout::left_to_right(egui::Align::Min),
                                    |ui| {
                                        if gutter_bg != Color32::TRANSPARENT {
                                            let rect = ui.max_rect();
                                            ui.painter().rect_filled(rect, 0.0, gutter_bg);
                                        }
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
                                                RichText::new(format!("{} {} {} ", left_num, right_num, symbol))
                                                    .monospace()
                                                    .color(dim_color),
                                            ),
                                        );
                                    },
                                );
                            }
                        });
                    },
                );

            // 内容区域（可横向和纵向滚动）
            ui.allocate_ui_with_layout(
                egui::vec2(content_width, available_height),
                egui::Layout::top_down(egui::Align::LEFT),
                |ui| {
                    let output = egui::ScrollArea::both()
                        .auto_shrink([false, false])
                        .id_salt("unified_content")
                        .show(ui, |ui| {
                            ui.spacing_mut().item_spacing.y = 0.0;

                            for line in &result.unified_lines {
                                let line_bg = match line.diff_type {
                                    DiffType::Removed => if is_dark_mode {
                                        Color32::from_rgba_unmultiplied(61, 31, 35, 180)
                                    } else {
                                        Color32::from_rgba_unmultiplied(255, 210, 215, 230)
                                    },
                                    DiffType::Added => if is_dark_mode {
                                        Color32::from_rgba_unmultiplied(31, 61, 38, 180)
                                    } else {
                                        Color32::from_rgba_unmultiplied(180, 240, 195, 230)
                                    },
                                    DiffType::Equal => Color32::TRANSPARENT,
                                };

                                let is_whole_line_change = line.segments.is_empty()
                                    || (line.diff_type != DiffType::Equal
                                        && line.segments.iter().all(|s| s.diff_type != DiffType::Equal));

                                ui.allocate_ui_with_layout(
                                    egui::vec2(content_width, row_height),
                                    egui::Layout::left_to_right(egui::Align::Center),
                                    |ui| {
                                        if line_bg != Color32::TRANSPARENT {
                                            let mut bg_rect = ui.max_rect();
                                            bg_rect.set_width(bg_rect.width().max(2000.0));
                                            ui.painter().rect_filled(bg_rect, 0.0, line_bg);
                                        }

                                    if line.diff_type == DiffType::Equal && syntax_name.is_some() {
                                        let mut job = self.get_line_highlight_job(
                                            &line.content,
                                            syntax_name.as_deref(),
                                            font_size,
                                            is_dark_mode,
                                        );
                                        job.wrap.max_width = f32::INFINITY;
                                        ui.label(job);
                                    } else if !is_whole_line_change && !line.segments.is_empty() {
                                        let mut job = LayoutJob::default();
                                        job.wrap.max_width = f32::INFINITY;
                                        let (added_bg, removed_bg) = Self::diff_background_colors(is_dark_mode);
                                        self.append_diff_segments_to_job(
                                            &mut job,
                                            &line.content,
                                            &line.segments,
                                            syntax_name.as_deref(),
                                            font_size,
                                            is_dark_mode,
                                            text_color,
                                            added_bg,
                                            removed_bg,
                                        );
                                        ui.label(job);
                                    } else {
                                        if syntax_name.is_some() {
                                            let mut job = self.get_line_highlight_job(
                                                &line.content,
                                                syntax_name.as_deref(),
                                                font_size,
                                                is_dark_mode,
                                            );
                                            job.wrap.max_width = f32::INFINITY;
                                            ui.label(job);
                                        } else {
                                            let color = match line.diff_type {
                                                DiffType::Added => Color32::from_rgb(0, 150, 0),
                                                DiffType::Removed => Color32::from_rgb(180, 0, 0),
                                                _ => text_color,
                                            };
                                            let mut job = LayoutJob::default();
                                            job.wrap.max_width = f32::INFINITY;
                                            job.append(
                                                &line.content, 0.0,
                                                egui::TextFormat {
                                                    font_id: egui::FontId::monospace(font_size),
                                                    color,
                                                    ..Default::default()
                                                },
                                            );
                                            ui.label(job);
                                        }
                                    }
                                },
                            );
                        }
                    });
                    // 更新 Unified 视图的纵向偏移量，用于下一帧同步行号
                    self.last_unified_offset.set(output.state.offset.y);
                },
            );
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

    /// 绘制斜线背景（用于标识不存在的行）
    ///
    /// 在指定矩形区域内绘制右上到左下的45度斜线。
    /// 使用全局 y位置计算斜线偏移，确保相邻行的斜线对接。
    /// 使用 extend 批量绘制以提升性能。
    ///
    /// # 参数
    /// - `painter`: 绘制器
    /// - `rect`: 绘制区域
    /// - `line_index`: 行索引（用于计算全局 y位置）
    /// - `row_height`: 行高
    /// - `color`: 斜线颜色
    fn paint_hatched_background(
        painter: &egui::Painter,
        rect: egui::Rect,
        line_index: usize,
        row_height: f32,
        color: Color32,
    ) {
        let spacing = row_height; // 斜线间距等于行高，确保相邻行对接
        let line_width = 1.0;
        let rect_height = rect.max.y - rect.min.y;

        // 计算全局 y 起始位置（基于行索引和行高）
        let global_y_start = line_index as f32 * row_height;

        // 计算斜线的起始偏移（基于全局 y 位置）
        let offset = global_y_start % spacing;

        // 收集所有线段形状，批量绘制
        let stroke = egui::Stroke::new(line_width, color);
        let mut shapes = Vec::new();

        // 绘制斜线（从右上到左下）
        let mut x_start = rect.min.x - offset;
        while x_start <= rect.max.x {
            let x1 = x_start;
            let x2 = x_start - rect_height;

            // 只绘制起点和终点都在矩形区域内的斜线
            if x1 >= rect.min.x && x2 <= rect.max.x {
                let p1 = egui::pos2(x1, rect.min.y);
                let p2 = egui::pos2(x2, rect.max.y);
                shapes.push(egui::Shape::line_segment([p1, p2], stroke));
            }

            x_start += spacing;
        }

        // 批量绘制所有线段
        if !shapes.is_empty() {
            painter.extend(shapes);
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

    /// 获取行级语法高亮的 LayoutJob（带缓存）
    ///
    /// 用于 Unified 视图中每行的语法高亮，避免每帧重复计算。
    fn get_line_highlight_job(
        &self,
        text: &str,
        syntax_name: Option<&str>,
        font_size: f32,
        is_dark_mode: bool,
    ) -> LayoutJob {
        let hash = Self::compute_highlight_hash(text, &syntax_name.map(|s| s.to_string()), is_dark_mode, font_size);
        let mut cache = self.unified_line_cache.borrow_mut();

        if let Some(job) = cache.get(&hash) {
            return job.clone();
        }

        let job = self.highlighter.highlight_to_layout_job(text, syntax_name, font_size, is_dark_mode);
        cache.insert(hash, job.clone());
        job
    }

    fn clear_cache(&self) {
        self.left_highlight_cache.borrow_mut().take();
        self.right_highlight_cache.borrow_mut().take();
        self.unified_line_cache.borrow_mut().clear();
    }
}
