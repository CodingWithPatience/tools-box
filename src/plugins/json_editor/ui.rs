use std::collections::{BTreeMap, BTreeSet};

use egui::TextStyle;
use egui::text::LayoutJob;

use super::processor;
use crate::utils::highlight::SyntaxHighlighter;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct FoldRegion {
    start_line: usize,
    end_line: usize,
    end_column: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct VisibleLine {
    line_index: usize,
    collapsed_end_line: Option<usize>,
}

/// JSON 编辑器 UI 状态
pub struct JsonEditorUi {
    /// 输入文本
    pub input: String,
    /// 输出文本
    pub output: String,
    /// 状态消息
    pub status_msg: String,
    /// 状态是否为错误
    pub status_is_error: bool,
    /// 字节数
    pub bytes: usize,
    /// 行数
    pub lines: usize,
    /// JSON 是否有效
    pub valid: bool,
    /// 语法高亮器
    highlighter: SyntaxHighlighter,
    /// 输出区是否启用语法高亮
    highlight_enabled: bool,
    /// 输出区已折叠区域的起始行
    collapsed_output_regions: BTreeSet<usize>,
}

impl JsonEditorUi {
    pub fn new() -> Self {
        Self {
            input: String::new(),
            output: String::new(),
            status_msg: "就绪".to_string(),
            status_is_error: false,
            bytes: 0,
            lines: 0,
            valid: false,
            highlighter: SyntaxHighlighter::new(),
            highlight_enabled: true,
            collapsed_output_regions: BTreeSet::new(),
        }
    }

    /// 更新统计信息
    fn update_stats(&mut self) {
        let (bytes, lines, valid) = processor::json_stats(&self.input);
        self.bytes = bytes;
        self.lines = lines;
        self.valid = valid;
    }

    /// 应用处理结果到输出
    fn apply_result(&mut self, result: processor::ProcessResult) {
        if result.is_error {
            self.status_msg = result.message;
            self.status_is_error = true;
        } else {
            if !result.output.is_empty() {
                self.output = result.output;
            }
            self.status_msg = if result.message.is_empty() {
                "✓ 操作成功".to_string()
            } else {
                result.message
            };
            self.status_is_error = false;
        }
        self.collapsed_output_regions.clear();
    }

    /// 复制文本到剪贴板
    fn copy_to_clipboard(&self, text: &str) {
        match arboard::Clipboard::new() {
            Ok(mut clipboard) => {
                if let Err(e) = clipboard.set_text(text.to_owned()) {
                    log::error!("复制到剪贴板失败: {}", e);
                }
            }
            Err(e) => {
                log::error!("无法访问剪贴板: {}", e);
            }
        }
    }

    /// 查找跨行 JSON 对象和数组对应的可折叠区域
    fn find_fold_regions(text: &str) -> Vec<FoldRegion> {
        let mut stack = Vec::new();
        let mut regions_by_start = BTreeMap::new();
        let mut line_index = 0;
        let mut line_start_byte = 0;
        let mut in_string = false;
        let mut escaped = false;

        for (byte_index, character) in text.char_indices() {
            if in_string {
                if escaped {
                    escaped = false;
                } else if character == '\\' {
                    escaped = true;
                } else if character == '"' {
                    in_string = false;
                }
            } else {
                match character {
                    '"' => in_string = true,
                    '{' | '[' => stack.push((character, line_index)),
                    '}' | ']' => {
                        let Some((opening, start_line)) = stack.pop() else {
                            continue;
                        };
                        let matching_pair = matches!((opening, character), ('{', '}') | ('[', ']'));
                        if matching_pair && start_line < line_index {
                            let end_column = byte_index - line_start_byte;
                            regions_by_start
                                .entry(start_line)
                                .and_modify(|end: &mut (usize, usize)| {
                                    if (line_index, end_column) > *end {
                                        *end = (line_index, end_column);
                                    }
                                })
                                .or_insert((line_index, end_column));
                        }
                    }
                    _ => {}
                }
            }

            if character == '\n' {
                line_index += 1;
                line_start_byte = byte_index + character.len_utf8();
            }
        }

        regions_by_start
            .into_iter()
            .map(|(start_line, (end_line, end_column))| FoldRegion {
                start_line,
                end_line,
                end_column,
            })
            .collect()
    }

    /// 根据当前折叠状态计算需要显示的原始行
    fn visible_lines(
        line_count: usize,
        regions: &[FoldRegion],
        collapsed_regions: &BTreeSet<usize>,
    ) -> Vec<VisibleLine> {
        let regions_by_start = regions
            .iter()
            .map(|region| (region.start_line, region.end_line))
            .collect::<BTreeMap<_, _>>();
        let mut visible_lines = Vec::new();
        let mut line_index = 0;

        while line_index < line_count {
            let collapsed_end_line = regions_by_start
                .get(&line_index)
                .copied()
                .filter(|_| collapsed_regions.contains(&line_index));
            visible_lines.push(VisibleLine {
                line_index,
                collapsed_end_line,
            });
            line_index = collapsed_end_line.map_or(line_index + 1, |end_line| end_line + 1);
        }

        visible_lines
    }

    /// 为输出区的一行构建语法高亮内容
    fn get_highlighted_line_job(
        highlighter: &SyntaxHighlighter,
        line: &str,
        collapsed_closing_line: Option<&str>,
        is_dark_mode: bool,
        font_size: f32,
        placeholder_color: egui::Color32,
    ) -> LayoutJob {
        let mut job = highlighter.highlight_to_layout_job(
            line.trim_end_matches(['\r', '\n']),
            Some("JSON"),
            font_size,
            is_dark_mode,
        );
        if let Some(closing_line) = collapsed_closing_line {
            job.append(
                &format!("  …  {}", closing_line.trim()),
                0.0,
                egui::TextFormat {
                    font_id: egui::FontId::monospace(font_size),
                    color: placeholder_color,
                    ..Default::default()
                },
            );
        }
        job.wrap.max_width = f32::INFINITY;
        job
    }

    /// 获取折叠区域结束分隔符及其后的同行内容
    fn folded_closing_suffix<'a>(lines: &'a [&str], region: FoldRegion) -> Option<&'a str> {
        lines
            .get(region.end_line)
            .and_then(|line| line.get(region.end_column..))
    }

    /// 渲染操作按钮栏
    fn render_actions(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            // 格式化
            if ui.button("📐 格式化美化").clicked() {
                let r = processor::format_json(&self.input);
                self.apply_result(r);
            }

            // 压缩
            if ui.button("📦 压缩").clicked() {
                let r = processor::minify_json(&self.input);
                self.apply_result(r);
            }

            // 校验
            if ui.button("✅ 校验").clicked() {
                let r = processor::validate_json(&self.input);
                self.apply_result(r);
            }

            ui.separator();

            // 转义
            if ui.button("🔤 转义为字符串").clicked() {
                let r = processor::escape_json(&self.input);
                self.apply_result(r);
            }

            // 反转义
            if ui.button("🔡 反转义").clicked() {
                let r = processor::unescape_json(&self.input);
                self.apply_result(r);
            }

            ui.separator();

            // 清空
            if ui.button("🗑 清空").clicked() {
                self.input.clear();
                self.output.clear();
                self.status_msg = "已清空".to_string();
                self.status_is_error = false;
                self.collapsed_output_regions.clear();
            }

            // 复制输出
            if ui.button("📋 复制输出").clicked() {
                if !self.output.is_empty() {
                    self.copy_to_clipboard(&self.output);
                    self.status_msg = "✓ 已复制到剪贴板".to_string();
                    self.status_is_error = false;
                }
            }

            // 输入输出互换
            if ui.button("🔄 交换").clicked() {
                if !self.output.is_empty() {
                    std::mem::swap(&mut self.input, &mut self.output);
                    self.update_stats();
                    self.collapsed_output_regions.clear();
                }
            }
        });
    }

    /// 渲染输入区
    fn render_input(&mut self, ui: &mut egui::Ui, height: f32) {
        ui.horizontal(|ui| {
            ui.label("📥 输入：");
            if self.input.is_empty() {
                ui.weak("在此粘贴或输入 JSON...");
            }
        });

        let editor_height = (height - 30.0).max(80.0);
        let response = egui::ScrollArea::both()
            .id_salt("json_input_scroll")
            .max_height(editor_height)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.add_sized(
                    [ui.available_width(), editor_height],
                    egui::TextEdit::multiline(&mut self.input)
                        .font(egui::TextStyle::Monospace)
                        .code_editor(),
                )
            })
            .inner;

        // 输入变化时更新统计和校验
        if response.changed() {
            self.update_stats();
        }
    }

    /// 渲染支持折叠的高亮输出区
    fn render_foldable_output(
        &mut self,
        ui: &mut egui::Ui,
        height: f32,
        fold_regions: &[FoldRegion],
    ) {
        let lines = self.output.lines().collect::<Vec<_>>();
        let visible_lines =
            Self::visible_lines(lines.len(), fold_regions, &self.collapsed_output_regions);
        let fold_region_by_start = fold_regions
            .iter()
            .map(|region| (region.start_line, *region))
            .collect::<BTreeMap<_, _>>();
        let is_dark_mode = ui.visuals().dark_mode;
        let placeholder_color = ui.visuals().weak_text_color();
        let font_size = ui
            .style()
            .text_styles
            .get(&TextStyle::Monospace)
            .map(|font_id| font_id.size)
            .unwrap_or(14.0);
        let row_height = ui.text_style_height(&TextStyle::Monospace).max(18.0);
        let line_number_width = lines.len().max(1).to_string().len();
        let line_number_display_width = 64.0;
        let widest_line = lines
            .iter()
            .max_by_key(|line| line.chars().count())
            .copied()
            .unwrap_or_default();
        let widest_line_width = ui.fonts(|fonts| {
            fonts
                .layout_no_wrap(
                    widest_line.to_owned(),
                    egui::FontId::monospace(font_size),
                    placeholder_color,
                )
                .size()
                .x
        });
        let content_width = 18.0 + line_number_display_width + widest_line_width + 128.0;
        let highlighter = &self.highlighter;
        let collapsed_output_regions = &mut self.collapsed_output_regions;

        egui::ScrollArea::both()
            .id_salt("json_output_fold_scroll")
            .max_height(height)
            .auto_shrink([false, false])
            .show_rows(ui, row_height, visible_lines.len(), |ui, visible_range| {
                ui.set_min_width(content_width);
                for visible_line in &visible_lines[visible_range] {
                    let line_index = visible_line.line_index;
                    let fold_region = fold_region_by_start.get(&line_index).copied();
                    let is_collapsed = visible_line.collapsed_end_line.is_some();
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 2.0;
                        let marker = if fold_region.is_some() {
                            if is_collapsed { "▶" } else { "▼" }
                        } else {
                            " "
                        };
                        let marker_response = ui
                            .add_sized([18.0, row_height], egui::Button::new(marker).frame(false));
                        if fold_region.is_some()
                            && marker_response
                                .on_hover_text(if is_collapsed {
                                    "展开此区域"
                                } else {
                                    "折叠此区域"
                                })
                                .clicked()
                        {
                            if is_collapsed {
                                collapsed_output_regions.remove(&line_index);
                            } else {
                                collapsed_output_regions.insert(line_index);
                            }
                        }

                        ui.add_sized(
                            [line_number_display_width, row_height],
                            egui::Label::new(
                                egui::RichText::new(format!(
                                    "{:>width$}",
                                    line_index + 1,
                                    width = line_number_width
                                ))
                                .monospace()
                                .color(placeholder_color),
                            ),
                        );

                        let closing_line = fold_region
                            .filter(|_| is_collapsed)
                            .and_then(|region| Self::folded_closing_suffix(&lines, region));
                        let job = Self::get_highlighted_line_job(
                            highlighter,
                            lines[line_index],
                            closing_line,
                            is_dark_mode,
                            font_size,
                            placeholder_color,
                        );
                        ui.label(job);
                    });
                }
            });
    }

    /// 渲染输出区
    fn render_output(&mut self, ui: &mut egui::Ui, height: f32) {
        let fold_regions = Self::find_fold_regions(&self.output);
        ui.horizontal(|ui| {
            ui.label("📤 输出：");
            // 语法高亮切换按钮
            let toggle_label = if self.highlight_enabled {
                "🔆 高亮"
            } else {
                "🔅 原始"
            };
            if ui.small_button(toggle_label).clicked() {
                self.highlight_enabled = !self.highlight_enabled;
            }
            if self.highlight_enabled && !fold_regions.is_empty() {
                if ui.small_button("折叠全部").clicked() {
                    self.collapsed_output_regions = fold_regions
                        .iter()
                        .map(|region| region.start_line)
                        .collect();
                }
                if ui.small_button("展开全部").clicked() {
                    self.collapsed_output_regions.clear();
                }
            }
        });

        let editor_height = (height - 30.0).max(80.0);

        if self.highlight_enabled && !self.output.is_empty() {
            // 启用语法高亮模式，以只读方式渲染带颜色的 JSON
            let font_size = ui
                .style()
                .text_styles
                .get(&TextStyle::Monospace)
                .map(|font_id| font_id.size)
                .unwrap_or(14.0);

            // 检测输出是否为有效 JSON，无效时回退到纯文本显示
            let is_valid_json = serde_json::from_str::<serde_json::Value>(&self.output).is_ok();

            if is_valid_json {
                self.render_foldable_output(ui, editor_height, &fold_regions);
            } else {
                egui::ScrollArea::both()
                    .id_salt("json_output_scroll")
                    .max_height(editor_height)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        // 非 JSON 输出（如反转义后的普通文本）用等宽字体纯文本显示
                        let mut job = LayoutJob::default();
                        job.wrap.max_width = f32::INFINITY;
                        job.append(
                            &self.output,
                            0.0,
                            egui::TextFormat {
                                font_id: egui::FontId::monospace(font_size),
                                color: ui.visuals().text_color(),
                                ..Default::default()
                            },
                        );
                        ui.label(job);
                    });
            }
        } else {
            // 原始编辑模式
            let response = egui::ScrollArea::both()
                .id_salt("json_output_scroll")
                .max_height(editor_height)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.add_sized(
                        [ui.available_width(), editor_height],
                        egui::TextEdit::multiline(&mut self.output)
                            .font(egui::TextStyle::Monospace)
                            .code_editor(),
                    )
                })
                .inner;
            if response.changed() {
                self.collapsed_output_regions.clear();
            }
        }
    }

    /// 渲染状态栏
    fn render_status_bar(&self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let status_color = if self.status_is_error {
                egui::Color32::from_rgb(220, 50, 50)
            } else {
                egui::Color32::from_rgb(50, 180, 50)
            };

            ui.colored_label(status_color, &self.status_msg);

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let valid_str = if self.valid {
                    "✓ JSON 有效"
                } else if self.input.is_empty() {
                    "等待输入"
                } else {
                    "✗ JSON 无效"
                };
                ui.label(format!(
                    "{}  |  大小: {} bytes  |  行数: {}",
                    valid_str, self.bytes, self.lines
                ));
            });
        });
    }

    /// 渲染完整的 JSON 编辑器 UI
    pub fn render(&mut self, ui: &mut egui::Ui) {
        ui.heading("📋 JSON 编辑器");
        ui.separator();

        // 操作按钮
        self.render_actions(ui);
        ui.add_space(4.0);

        // 输入、输出区左右等宽排列
        let editor_height = (ui.available_height() - 34.0).max(160.0);
        ui.columns(2, |columns| {
            let input_width = columns[0].available_width();
            columns[0].allocate_ui_with_layout(
                egui::vec2(input_width, editor_height),
                egui::Layout::top_down(egui::Align::Min),
                |ui| self.render_input(ui, editor_height),
            );

            let output_width = columns[1].available_width();
            columns[1].allocate_ui_with_layout(
                egui::vec2(output_width, editor_height),
                egui::Layout::top_down(egui::Align::Min),
                |ui| self.render_output(ui, editor_height),
            );
        });
        ui.add_space(4.0);

        // 状态栏
        self.render_status_bar(ui);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn highlighted_job(ui: &JsonEditorUi, text: &str, is_dark_mode: bool) -> LayoutJob {
        JsonEditorUi::get_highlighted_line_job(
            &ui.highlighter,
            text,
            None,
            is_dark_mode,
            14.0,
            egui::Color32::GRAY,
        )
    }

    /// 有效 JSON 对象应生成非空带格式的 LayoutJob
    #[test]
    fn test_highlight_valid_json() {
        let ui = JsonEditorUi::new();
        let json = r#"{"name":"test","value":42}"#;
        let job = highlighted_job(&ui, json, false);
        assert!(!job.text.is_empty(), "LayoutJob 文本不应为空");
        // JSON 高亮应有多个格式段（键、字符串、数字等不同颜色）
        assert!(
            job.sections.len() > 1,
            "应有多个格式段，实际: {}",
            job.sections.len()
        );
    }

    /// 空 JSON 对象也不应 panic
    #[test]
    fn test_highlight_empty_json_object() {
        let ui = JsonEditorUi::new();
        let job = highlighted_job(&ui, "{}", false);
        assert!(!job.text.is_empty());
    }

    /// 嵌套 JSON 结构应正确高亮
    #[test]
    fn test_highlight_nested_json() {
        let ui = JsonEditorUi::new();
        let json = r#"{"user":{"name":"Alice","roles":["admin","user"]}}"#;
        let job = highlighted_job(&ui, json, false);
        assert!(!job.text.is_empty());
        assert!(job.sections.len() > 2);
    }

    /// 非 JSON 文本（如转义后的字符串）不会使高亮器 panic
    #[test]
    fn test_highlight_non_json_text_safe() {
        let ui = JsonEditorUi::new();
        // syntect 的 JSON 语法对非 JSON 文本也能处理（按纯文本渲染）
        let text = r#""This is an escaped JSON string""#;
        let job = highlighted_job(&ui, text, false);
        assert!(!job.text.is_empty());
    }

    /// 含 Unicode 字符的 JSON 正确处理
    #[test]
    fn test_highlight_unicode_json() {
        let ui = JsonEditorUi::new();
        let json = r#"{"消息":"你好世界","emoji":"🎉"}"#;
        let job = highlighted_job(&ui, json, false);
        assert!(!job.text.is_empty());
        assert!(job.sections.len() > 1);
    }

    /// 嵌套对象和数组应生成对应的跨行折叠区域
    #[test]
    fn test_find_nested_fold_regions() {
        let json = "{\n  \"user\": {\n    \"roles\": [\n      \"admin\"\n    ]\n  }\n}";
        let regions = JsonEditorUi::find_fold_regions(json);

        assert_eq!(
            regions,
            vec![
                FoldRegion {
                    start_line: 0,
                    end_line: 6,
                    end_column: 0,
                },
                FoldRegion {
                    start_line: 1,
                    end_line: 5,
                    end_column: 2,
                },
                FoldRegion {
                    start_line: 2,
                    end_line: 4,
                    end_column: 4,
                },
            ]
        );
    }

    /// JSON 字符串中的括号不能被识别为折叠边界
    #[test]
    fn test_find_fold_regions_ignores_brackets_in_strings() {
        let json = "{\n  \"text\": \"包含 { 括号 } 和转义引号 \\\"[测试]\\\"\"\n}";
        let regions = JsonEditorUi::find_fold_regions(json);

        assert_eq!(
            regions,
            vec![FoldRegion {
                start_line: 0,
                end_line: 2,
                end_column: 0,
            }]
        );
    }

    /// 折叠父区域后应跳过全部子行，展开后应恢复所有行
    #[test]
    fn test_visible_lines_respects_collapsed_parent() {
        let regions = vec![
            FoldRegion {
                start_line: 0,
                end_line: 5,
                end_column: 0,
            },
            FoldRegion {
                start_line: 1,
                end_line: 3,
                end_column: 2,
            },
        ];
        let collapsed = BTreeSet::from([0]);

        assert_eq!(
            JsonEditorUi::visible_lines(7, &regions, &collapsed),
            vec![
                VisibleLine {
                    line_index: 0,
                    collapsed_end_line: Some(5),
                },
                VisibleLine {
                    line_index: 6,
                    collapsed_end_line: None,
                },
            ]
        );
        assert_eq!(
            JsonEditorUi::visible_lines(3, &[], &BTreeSet::new()),
            vec![
                VisibleLine {
                    line_index: 0,
                    collapsed_end_line: None,
                },
                VisibleLine {
                    line_index: 1,
                    collapsed_end_line: None,
                },
                VisibleLine {
                    line_index: 2,
                    collapsed_end_line: None,
                },
            ]
        );
    }

    /// 折叠占位仅保留结束分隔符和同行尾部，不能泄露被折叠的值
    #[test]
    fn test_folded_closing_suffix_hides_content_before_delimiter() {
        let json = "{\n  \"items\": [\n    1, 2],\n  \"enabled\": true\n}";
        let regions = JsonEditorUi::find_fold_regions(json);
        let array_region = regions
            .iter()
            .find(|region| region.start_line == 1)
            .copied();
        let lines = json.lines().collect::<Vec<_>>();

        assert_eq!(
            array_region.and_then(|region| JsonEditorUi::folded_closing_suffix(&lines, region)),
            Some("],")
        );
    }
}
