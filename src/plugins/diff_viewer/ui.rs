use egui::{Color32, RichText, text::LayoutJob};

use super::models::{DiffResult, DiffType, SplitLine, TextSegment, ViewMode};

/// Diff 查看器 UI
pub struct DiffViewerUi {
    /// 左侧文本
    left_text: String,
    /// 右侧文本
    right_text: String,
    /// 视图模式
    view_mode: ViewMode,
    /// 差异结果
    diff_result: Option<DiffResult>,
}

impl DiffViewerUi {
    pub fn new() -> Self {
        Self {
            left_text: String::new(),
            right_text: String::new(),
            view_mode: ViewMode::Edit,
            diff_result: None,
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
        let available_height = ui.available_height() - 100.0;

        ui.horizontal(|ui| {
            // 左侧文本
            ui.vertical(|ui| {
                ui.label("原始文本 (左侧):");
                ui.add_sized(
                    [ui.available_width() / 2.0 - 10.0, available_height],
                    egui::TextEdit::multiline(&mut self.left_text)
                        .hint_text("在此输入原始文本...")
                        .code_editor(),
                );
            });

            ui.separator();

            // 右侧文本
            ui.vertical(|ui| {
                ui.label("对比文本 (右侧):");
                ui.add_sized(
                    [ui.available_width(), available_height],
                    egui::TextEdit::multiline(&mut self.right_text)
                        .hint_text("在此输入对比文本...")
                        .code_editor(),
                );
            });
        });

        ui.add_space(10.0);

        // 操作按钮
        ui.horizontal(|ui| {
            if ui.button("🔄 交换内容").clicked() {
                std::mem::swap(&mut self.left_text, &mut self.right_text);
            }

            if ui.button("🗑 清空").clicked() {
                self.left_text.clear();
                self.right_text.clear();
                self.diff_result = None;
            }

            ui.separator();

            // 开始对比按钮
            let compare_btn = ui.button("📊 开始对比");
            if compare_btn.clicked() {
                self.diff_result = Some(differ::compute_diff(&self.left_text, &self.right_text));
                self.view_mode = ViewMode::Split;
            }
        });
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
            let available_height = ui.available_height() - 60.0;

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
                .max_height(available_height)
                .auto_shrink([false, false])  // 不自动缩小，内容不足时不显示滚动条
                .id_salt("diff_split_view")
                .show(ui, |ui| {
                    // 设置等宽字体
                    let code_style = egui::TextStyle::Monospace;
                    let row_height = ui.text_style_height(&code_style) + 4.0;

                    egui::Grid::new("diff_split_grid")
                        .striped(true)
                        .spacing([0.0, 0.0])
                        .min_col_width(ui.available_width() / 2.0)
                        .show(ui, |ui| {
                            for line in &result.split_lines {
                                self.render_split_line(ui, line, row_height);
                                ui.end_row();
                            }
                        });
                });

            // 统计信息
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
        }
    }

    /// 渲染 Split 视图的单行
    fn render_split_line(&self, ui: &mut egui::Ui, line: &SplitLine, row_height: f32) {
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

            if let Some(_content) = &line.left_content {
                let line_num = line
                    .left_line_number
                    .map(|n| format!("{:>4} │ ", n))
                    .unwrap_or_else(|| "      │ ".to_string());

                // 使用字符级差异渲染
                if !line.left_segments.is_empty() {
                    let job = self.create_segment_layout(&line.left_segments, Some(&line_num));
                    ui.label(job);
                } else {
                    let text = format!("{}{}", line_num, _content);
                    let rich_text = RichText::new(text).monospace();
                    let rich_text = match line.left_type {
                        DiffType::Removed => rich_text.color(Color32::from_rgb(180, 0, 0)),
                        _ => rich_text,
                    };
                    ui.label(rich_text);
                }
            } else {
                let rich_text = RichText::new("      │ ").monospace();
                ui.label(rich_text);
            }
        });

        ui.separator();

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

            if let Some(_content) = &line.right_content {
                let line_num = line
                    .right_line_number
                    .map(|n| format!("{:>4} │ ", n))
                    .unwrap_or_else(|| "      │ ".to_string());

                // 使用字符级差异渲染
                if !line.right_segments.is_empty() {
                    let job = self.create_segment_layout(&line.right_segments, Some(&line_num));
                    ui.label(job);
                } else {
                    let text = format!("{}{}", line_num, _content);
                    let rich_text = RichText::new(text).monospace();
                    let rich_text = match line.right_type {
                        DiffType::Added => rich_text.color(Color32::from_rgb(0, 150, 0)),
                        _ => rich_text,
                    };
                    ui.label(rich_text);
                }
            } else {
                let rich_text = RichText::new("      │ ").monospace();
                ui.label(rich_text);
            }
        });
    }

    /// 创建字符级差异的布局任务
    fn create_segment_layout(&self, segments: &[TextSegment], prefix: Option<&str>) -> LayoutJob {
        let mut job = LayoutJob::default();

        // 添加行号前缀
        if let Some(prefix) = prefix {
            job.append(
                prefix,
                0.0,
                egui::TextFormat {
                    font_id: egui::FontId::monospace(14.0),
                    color: Color32::from_rgb(128, 128, 128),
                    ..Default::default()
                },
            );
        }

        // 添加字符级差异片段
        for segment in segments {
            let color = match segment.diff_type {
                DiffType::Equal => Color32::from_rgb(0, 0, 0),        // 黑色
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
                    font_id: egui::FontId::monospace(14.0),
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
            let available_height = ui.available_height() - 60.0;

            // Unified 视图
            egui::ScrollArea::both()
                .max_height(available_height)
                .auto_shrink([false, false])  // 不自动缩小，内容不足时不显示滚动条
                .id_salt("diff_unified_view")
                .show(ui, |ui| {
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

                        let text = format!("{}{}{}", line_num, prefix, line.content);
                        let rich_text = RichText::new(text).monospace();

                        let rich_text = match line.diff_type {
                            DiffType::Added => rich_text.color(Color32::from_rgb(0, 150, 0)),
                            DiffType::Removed => rich_text.color(Color32::from_rgb(180, 0, 0)),
                            _ => rich_text,
                        };

                        // 绘制背景
                        let rect = ui.available_rect_before_wrap();
                        let rect = egui::Rect::from_min_size(
                            rect.min,
                            egui::vec2(rect.width(), row_height),
                        );
                        ui.painter().rect_filled(rect, 0.0, bg_color);

                        ui.label(rich_text);
                    }
                });

            // 统计信息
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
        }
    }
}

// 引入 differ 模块
use super::differ;
