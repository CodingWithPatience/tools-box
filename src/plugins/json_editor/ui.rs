use std::cell::RefCell;
use std::hash::{DefaultHasher, Hash, Hasher};

use egui::text::LayoutJob;
use egui::TextStyle;

use super::processor;
use crate::utils::highlight::SyntaxHighlighter;

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
    /// 输出区高亮缓存
    output_highlight_cache: RefCell<Option<(u64, LayoutJob)>>,
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
            output_highlight_cache: RefCell::new(None),
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
        // 输出内容变化时清除高亮缓存
        self.output_highlight_cache.borrow_mut().take();
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

    /// 计算高亮哈希值（用于缓存判断）
    fn compute_highlight_hash(
        text: &str,
        is_dark_mode: bool,
        font_size: f32,
    ) -> u64 {
        let mut hasher = DefaultHasher::new();
        text.hash(&mut hasher);
        is_dark_mode.hash(&mut hasher);
        font_size.to_bits().hash(&mut hasher);
        hasher.finish()
    }

    /// 获取或计算输出文本的语法高亮 LayoutJob（带缓存）
    fn get_highlighted_job(
        &self,
        text: &str,
        is_dark_mode: bool,
        font_size: f32,
    ) -> LayoutJob {
        let hash = Self::compute_highlight_hash(text, is_dark_mode, font_size);
        let mut cache = self.output_highlight_cache.borrow_mut();
        if let Some((cached_hash, cached_job)) = cache.as_ref() {
            if *cached_hash == hash {
                return cached_job.clone();
            }
        }
        let mut job = self.highlighter.highlight_to_layout_job(
            text,
            Some("JSON"),
            font_size,
            is_dark_mode,
        );
        job.wrap.max_width = f32::INFINITY;
        *cache = Some((hash, job.clone()));
        job
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
                self.output_highlight_cache.borrow_mut().take();
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
                    self.output_highlight_cache.borrow_mut().take();
                }
            }
        });
    }

    /// 渲染输入区
    fn render_input(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label("📥 输入：");
            if self.input.is_empty() {
                ui.weak("在此粘贴或输入 JSON...");
            }
        });

        let height = ui.available_height() / 2.0 - 60.0;
        let response = egui::ScrollArea::both()
            .id_salt("json_input_scroll")
            .max_height(height)
            .show(ui, |ui| {
                ui.add_sized(
                    [ui.available_width(), height],
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

    /// 渲染输出区
    fn render_output(&mut self, ui: &mut egui::Ui) {
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
        });

        let height = ui.available_height() - 40.0;

        if self.highlight_enabled && !self.output.is_empty() {
            // 启用语法高亮模式，以只读方式渲染带颜色的 JSON
            let is_dark_mode = ui.visuals().dark_mode;
            let font_size = ui
                .style()
                .text_styles
                .get(&TextStyle::Monospace)
                .map(|font_id| font_id.size)
                .unwrap_or(14.0);

            // 检测输出是否为有效 JSON，无效时回退到纯文本显示
            let is_valid_json = serde_json::from_str::<serde_json::Value>(&self.output).is_ok();

            egui::ScrollArea::both()
                .id_salt("json_output_scroll")
                .max_height(height)
                .show(ui, |ui| {
                    if is_valid_json {
                        let job = self.get_highlighted_job(
                            &self.output,
                            is_dark_mode,
                            font_size,
                        );
                        ui.label(job);
                    } else {
                        // 非 JSON 输出（如转义字符串、错误信息）用等宽字体纯文本显示
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
                    }
                });
        } else {
            // 原始编辑模式
            egui::ScrollArea::vertical()
                .id_salt("json_output_scroll")
                .max_height(height)
                .show(ui, |ui| {
                    ui.add_sized(
                        [ui.available_width(), height],
                        egui::TextEdit::multiline(&mut self.output)
                            .font(egui::TextStyle::Monospace)
                            .code_editor(),
                    );
                });
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
                ui.label(format!("{}  |  大小: {} bytes  |  行数: {}", valid_str, self.bytes, self.lines));
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

        // 输入区（约占一半高度）
        self.render_input(ui);
        ui.add_space(4.0);

        // 输出区
        self.render_output(ui);
        ui.add_space(4.0);

        // 状态栏
        self.render_status_bar(ui);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 有效 JSON 对象应生成非空带格式的 LayoutJob
    #[test]
    fn test_highlight_valid_json() {
        let ui = JsonEditorUi::new();
        let json = r#"{"name":"test","value":42}"#;
        let job = ui.get_highlighted_job(json, false, 14.0);
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
        let job = ui.get_highlighted_job("{}", false, 14.0);
        assert!(!job.text.is_empty());
    }

    /// 嵌套 JSON 结构应正确高亮
    #[test]
    fn test_highlight_nested_json() {
        let ui = JsonEditorUi::new();
        let json = r#"{"user":{"name":"Alice","roles":["admin","user"]}}"#;
        let job = ui.get_highlighted_job(json, false, 14.0);
        assert!(!job.text.is_empty());
        assert!(job.sections.len() > 2);
    }

    /// 相同输入第二次调用应命中缓存（哈希一致）
    #[test]
    fn test_highlight_cache_hit() {
        let ui = JsonEditorUi::new();
        let json = r#"{"a":1}"#;
        // 首次调用
        let _job1 = ui.get_highlighted_job(json, false, 14.0);
        assert!(ui.output_highlight_cache.borrow().is_some());
        let hash_before = ui
            .output_highlight_cache
            .borrow()
            .as_ref()
            .map(|(h, _)| *h);
        // 第二次调用，缓存应命中
        let _job2 = ui.get_highlighted_job(json, false, 14.0);
        let hash_after = ui
            .output_highlight_cache
            .borrow()
            .as_ref()
            .map(|(h, _)| *h);
        // 哈希值不变说明未重新计算
        assert_eq!(hash_before, hash_after);
    }

    /// 不同文本使缓存失效
    #[test]
    fn test_highlight_cache_miss_different_text() {
        let ui = JsonEditorUi::new();
        let _job1 = ui.get_highlighted_job(r#"{"a":1}"#, false, 14.0);
        let hash1 = ui
            .output_highlight_cache
            .borrow()
            .as_ref()
            .map(|(h, _)| *h);
        let _job2 = ui.get_highlighted_job(r#"{"b":2}"#, false, 14.0);
        let hash2 = ui
            .output_highlight_cache
            .borrow()
            .as_ref()
            .map(|(h, _)| *h);
        assert_ne!(hash1, hash2, "不同文本应产生不同哈希");
    }

    /// 不同主题使缓存失效
    #[test]
    fn test_highlight_cache_miss_different_theme() {
        let ui = JsonEditorUi::new();
        let json = r#"{"a":1}"#;
        let _job1 = ui.get_highlighted_job(json, false, 14.0);
        let hash1 = ui
            .output_highlight_cache
            .borrow()
            .as_ref()
            .map(|(h, _)| *h);
        let _job2 = ui.get_highlighted_job(json, true, 14.0);
        let hash2 = ui
            .output_highlight_cache
            .borrow()
            .as_ref()
            .map(|(h, _)| *h);
        assert_ne!(hash1, hash2, "不同主题应产生不同哈希");
    }

    /// 不同字体大小使缓存失效
    #[test]
    fn test_highlight_cache_miss_different_font_size() {
        let ui = JsonEditorUi::new();
        let json = r#"{"a":1}"#;
        let _job1 = ui.get_highlighted_job(json, false, 14.0);
        let hash1 = ui
            .output_highlight_cache
            .borrow()
            .as_ref()
            .map(|(h, _)| *h);
        let _job2 = ui.get_highlighted_job(json, false, 18.0);
        let hash2 = ui
            .output_highlight_cache
            .borrow()
            .as_ref()
            .map(|(h, _)| *h);
        assert_ne!(hash1, hash2, "不同字体大小应产生不同哈希");
    }

    /// apply_result 后缓存应被清除
    #[test]
    fn test_cache_cleared_on_apply_result() {
        let mut ui = JsonEditorUi::new();
        ui.get_highlighted_job(r#"{"a":1}"#, false, 14.0);
        assert!(ui.output_highlight_cache.borrow().is_some());
        let result = processor::ProcessResult {
            output: String::new(),
            is_error: false,
            message: "ok".to_string(),
        };
        ui.apply_result(result);
        assert!(
            ui.output_highlight_cache.borrow().is_none(),
            "apply_result 应清除缓存"
        );
    }

    /// 哈希计算应具有确定性
    #[test]
    fn test_compute_hash_deterministic() {
        let hash1 = JsonEditorUi::compute_highlight_hash(r#"{"a":1}"#, false, 14.0);
        let hash2 = JsonEditorUi::compute_highlight_hash(r#"{"a":1}"#, false, 14.0);
        assert_eq!(hash1, hash2);
    }

    /// 非 JSON 文本（如转义后的字符串）不会使高亮器 panic
    #[test]
    fn test_highlight_non_json_text_safe() {
        let ui = JsonEditorUi::new();
        // syntect 的 JSON 语法对非 JSON 文本也能处理（按纯文本渲染）
        let text = r#""This is an escaped JSON string""#;
        let job = ui.get_highlighted_job(text, false, 14.0);
        assert!(!job.text.is_empty());
    }

    /// 含 Unicode 字符的 JSON 正确处理
    #[test]
    fn test_highlight_unicode_json() {
        let ui = JsonEditorUi::new();
        let json = r#"{"消息":"你好世界","emoji":"🎉"}"#;
        let job = ui.get_highlighted_job(json, false, 14.0);
        assert!(!job.text.is_empty());
        assert!(job.sections.len() > 1);
    }
}
