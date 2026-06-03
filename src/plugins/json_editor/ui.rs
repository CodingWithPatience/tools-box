use super::processor;

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
        });

        let height = ui.available_height() - 40.0;
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
