use egui::{Color32, RichText, text::LayoutJob};

use super::models::{DiffResult, DiffType, SplitLine, TextSegment, ViewMode};

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
                egui::ScrollArea::vertical()
                    .id_salt("diff_edit_left")
                    .max_height(available_height)
                    .show(ui, |ui| {
                        egui::TextEdit::multiline(&mut self.left_text)
                            .hint_text("在此输入原始文本...")
                            .code_editor()
                            .desired_width(f32::INFINITY)
                            .min_size(egui::vec2(0.0, available_height))
                            .show(ui);
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
                egui::ScrollArea::vertical()
                    .id_salt("diff_edit_right")
                    .max_height(available_height)
                    .show(ui, |ui| {
                        egui::TextEdit::multiline(&mut self.right_text)
                            .hint_text("在此输入对比文本...")
                            .code_editor()
                            .desired_width(f32::INFINITY)
                            .min_size(egui::vec2(0.0, available_height))
                            .show(ui);
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
            }

            if ui.button("🗑 清空").clicked() {
                self.left_text.clear();
                self.right_text.clear();
                self.left_file_name = None;
                self.right_file_name = None;
                self.diff_result = None;
                self.error = None;
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
                }
                Err(e) => {
                    self.error = Some(format!("读取文件失败: {}", e));
                }
            }
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

                        egui::Grid::new("diff_split_grid")
                            .striped(true)
                            .spacing([0.0, 0.0])
                            .min_col_width(ui.available_width() / 2.0)
                            .show(ui, |ui| {
                                for line in &result.split_lines {
                                    self.render_split_line(ui, line, row_height, font_size);
                                    ui.end_row();
                                }
                            });
                    });
            });
        }
    }

    /// 渲染 Split 视图的单行
    fn render_split_line(&self, ui: &mut egui::Ui, line: &SplitLine, row_height: f32, font_size: f32) {
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

            if let Some(_content) = &line.left_content {
                let line_num = line
                    .left_line_number
                    .map(|n| format!("{:>4} │ ", n))
                    .unwrap_or_else(|| "      │ ".to_string());

                // 使用字符级差异渲染
                if !line.left_segments.is_empty() {
                    let job = self.create_segment_layout(&line.left_segments, Some(&line_num), text_color, font_size);
                    ui.label(job);
                } else {
                    let text = format!("{}{}", line_num, _content);
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

        // 右侧（移除了 ui.separator()）
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
                    let job = self.create_segment_layout(&line.right_segments, Some(&line_num), text_color, font_size);
                    ui.label(job);
                } else {
                    let text = format!("{}{}", line_num, _content);
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
                // Unified 视图
                egui::ScrollArea::both()
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
                                _ => rich_text.color(text_color), // 使用主题文字颜色
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
            });
        }
    }
}

// 引入 differ 模块
use super::differ;
