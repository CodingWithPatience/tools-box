use rusqlite::Connection;

use super::crypto;
use super::models::*;
use super::store::PasswordStore;

/// UI 状态
#[derive(Debug, Clone, PartialEq)]
enum UiState {
    /// 需要输入主密码
    RequireMasterPassword,
    /// 首次设置主密码
    SetMasterPassword,
    /// 修改主密码
    ChangeMasterPassword,
    /// 密码列表主界面
    MainList,
    /// 新增密码条目
    AddEntry,
    /// 编辑密码条目
    EditEntry(i64),
    /// 密码生成器
    Generator,
}

/// 导出格式选择弹窗状态
#[derive(Debug, Clone, PartialEq)]
enum ExportDialogState {
    Closed,
    SelectFormat,
}

/// 密码管理器 UI
pub struct PasswordManagerUi {
    state: UiState,
    entries: Vec<EncryptedPasswordEntry>,
    search_query: String,
    master_password: String,
    new_password: String,
    confirm_password: String,
    derived_key: Option<[u8; 32]>,
    has_master_password: bool,
    form: PasswordForm,
    generator_config: GeneratorConfig,
    generated_password: String,
    error_msg: Option<String>,
    success_msg: Option<String>,
    /// 临时显示的密码 (id -> password)
    visible_passwords: std::collections::HashMap<i64, String>,
    /// 导出弹窗状态
    export_dialog: ExportDialogState,
}

impl PasswordManagerUi {
    pub fn new() -> Self {
        Self {
            state: UiState::RequireMasterPassword,
            entries: Vec::new(),
            search_query: String::new(),
            master_password: String::new(),
            new_password: String::new(),
            confirm_password: String::new(),
            derived_key: None,
            has_master_password: false,
            form: PasswordForm::new(),
            generator_config: GeneratorConfig::default(),
            generated_password: String::new(),
            error_msg: None,
            success_msg: None,
            visible_passwords: std::collections::HashMap::new(),
            export_dialog: ExportDialogState::Closed,
        }
    }

    /// 渲染主界面
    pub fn render(&mut self, ui: &mut egui::Ui, conn: &Connection) {
        match self.state.clone() {
            UiState::RequireMasterPassword => self.render_require_password(ui, conn),
            UiState::SetMasterPassword => self.render_set_password(ui, conn),
            UiState::ChangeMasterPassword => self.render_change_password(ui, conn),
            UiState::MainList => self.render_main_list(ui, conn),
            UiState::AddEntry => self.render_add_entry(ui, conn),
            UiState::EditEntry(id) => self.render_edit_entry(ui, conn, id),
            UiState::Generator => self.render_generator(ui),
        }
    }

    /// 渲染消息提示
    fn render_messages(&mut self, ui: &mut egui::Ui) {
        if let Some(err) = &self.error_msg {
            ui.colored_label(egui::Color32::from_rgb(220, 50, 50), format!("⚠ {}", err));
        }
        if let Some(success) = &self.success_msg.clone() {
            ui.colored_label(egui::Color32::from_rgb(50, 180, 50), format!("✓ {}", success));
        }
    }

    /// 清除消息
    fn clear_messages(&mut self) {
        self.error_msg = None;
        self.success_msg = None;
    }

    /// 设置错误消息
    fn set_error(&mut self, msg: String) {
        self.error_msg = Some(msg);
        self.success_msg = None;
    }

    /// 设置成功消息
    fn set_success(&mut self, msg: String) {
        self.success_msg = Some(msg);
        self.error_msg = None;
    }

    /// 渲染主密码输入界面
    fn render_require_password(&mut self, ui: &mut egui::Ui, conn: &Connection) {
        // 检查是否已设置主密码
        self.check_master_password(conn);

        ui.heading("🔑 密码管理器");
        ui.separator();

        if self.has_master_password {
            ui.add_space(20.0);
            ui.label("请输入主密码以访问密码库：");
            ui.add_space(8.0);

            ui.horizontal(|ui| {
                ui.label("🔐");
                let response = ui.add_sized(
                    [250.0, 24.0],
                    egui::TextEdit::singleline(&mut self.master_password)
                        .password(true)
                        .hint_text("输入主密码..."),
                );

                // 回车提交
                if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    self.try_unlock(conn);
                }
            });

            ui.add_space(8.0);

            if ui.button("🔓 解锁").clicked() {
                self.try_unlock(conn);
            }

            ui.add_space(16.0);
            self.render_messages(ui);

            ui.add_space(8.0);
            ui.separator();
            if ui.link("点击修改主密码").clicked() {
                self.state = UiState::ChangeMasterPassword;
                self.clear_messages();
                self.master_password.clear();
                self.new_password.clear();
                self.confirm_password.clear();
            }
        } else {
            ui.add_space(20.0);
            ui.label("尚未设置主密码，请先设置主密码以保护您的密码库：");
            ui.add_space(16.0);
            self.render_messages(ui);

            ui.add_space(8.0);
            ui.separator();
            if ui.link("首次使用？点击设置主密码").clicked() {
                self.state = UiState::SetMasterPassword;
                self.clear_messages();
                self.master_password.clear();
            }
        }
    }

    /// 检查主密码是否已设置
    fn check_master_password(&mut self, conn: &Connection) {
        let store = PasswordStore::new(conn);
        match store.has_master_password() {
            Ok(has) => self.has_master_password = has,
            Err(e) => {
                log::error!("检查主密码状态失败: {}", e);
            }
        }
    }

    /// 尝试解锁
    fn try_unlock(&mut self, conn: &Connection) {
        self.clear_messages();

        // 检查是否输入了主密码
        if self.master_password.is_empty() {
            self.set_error("请输入主密码".to_string());
            return;
        }

        let store = PasswordStore::new(conn);

        match store.verify_master_password(&self.master_password) {
            Ok(Some(key)) => {
                self.derived_key = Some(key);
                self.state = UiState::MainList;
                self.master_password.clear();
                self.load_entries(conn);
            }
            Ok(None) => {
                self.set_error("主密码未设置，请先设置主密码".to_string());
            }
            Err(e) => {
                self.set_error(format!("{}", e));
            }
        }
    }

    /// 渲染设置主密码界面
    fn render_set_password(&mut self, ui: &mut egui::Ui, conn: &Connection) {
        ui.heading("🔑 设置主密码");
        ui.separator();

        ui.add_space(10.0);
        ui.label("首次使用请设置主密码，用于保护您的密码库：");
        ui.add_space(8.0);

        ui.horizontal(|ui| {
            ui.label("🔐 新密码：");
            ui.add_sized(
                [200.0, 24.0],
                egui::TextEdit::singleline(&mut self.master_password)
                    .password(true)
                    .hint_text("输入主密码（至少 6 位）..."),
            );
        });

        ui.add_space(4.0);

        ui.horizontal(|ui| {
            ui.label("🔐 确认：");
            ui.add_sized(
                [200.0, 24.0],
                egui::TextEdit::singleline(&mut self.confirm_password)
                    .password(true)
                    .hint_text("再次输入..."),
            );
        });

        ui.add_space(8.0);

        ui.horizontal(|ui| {
            if ui.button("✓ 确认设置").clicked() {
                self.setup_master_password(conn);
            }

            if ui.button("← 返回").clicked() {
                self.state = UiState::RequireMasterPassword;
                self.clear_messages();
                self.master_password.clear();
                self.confirm_password.clear();
            }
        });

        ui.add_space(8.0);
        self.render_messages(ui);
    }

    /// 设置主密码
    fn setup_master_password(&mut self, conn: &Connection) {
        self.clear_messages();

        if self.master_password.len() < 6 {
            self.set_error("密码长度至少 6 位".to_string());
            return;
        }

        if self.master_password != self.confirm_password {
            self.set_error("两次输入的密码不一致".to_string());
            return;
        }

        let store = PasswordStore::new(conn);
        match store.setup_master_password(&self.master_password) {
            Ok(key) => {
                self.derived_key = Some(key);
                self.has_master_password = true;
                self.state = UiState::MainList;
                self.master_password.clear();
                self.confirm_password.clear();
                self.set_success("主密码设置成功！".to_string());
            }
            Err(e) => {
                self.set_error(format!("设置失败: {}", e));
            }
        }
    }

    /// 渲染修改主密码界面
    fn render_change_password(&mut self, ui: &mut egui::Ui, conn: &Connection) {
        ui.horizontal(|ui| {
            ui.heading("🔑 修改主密码");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("← 返回").clicked() {
                    self.state = UiState::RequireMasterPassword;
                    self.clear_messages();
                    self.master_password.clear();
                    self.new_password.clear();
                    self.confirm_password.clear();
                }
            });
        });
        ui.separator();

        ui.add_space(10.0);
        ui.label("请输入当前主密码和新密码：");
        ui.add_space(8.0);

        egui::Grid::new("change_password_form")
            .num_columns(2)
            .spacing([8.0, 8.0])
            .show(ui, |ui| {
                ui.label("🔐 当前密码：");
                ui.add_sized(
                    [200.0, 24.0],
                    egui::TextEdit::singleline(&mut self.master_password)
                        .password(true)
                        .hint_text("输入当前主密码..."),
                );
                ui.end_row();

                ui.label("🔐 新密码：");
                ui.add_sized(
                    [200.0, 24.0],
                    egui::TextEdit::singleline(&mut self.new_password)
                        .password(true)
                        .hint_text("输入新主密码（至少 6 位）..."),
                );
                ui.end_row();

                ui.label("🔐 确认：");
                ui.add_sized(
                    [200.0, 24.0],
                    egui::TextEdit::singleline(&mut self.confirm_password)
                        .password(true)
                        .hint_text("再次输入新密码..."),
                );
                ui.end_row();
            });

        ui.add_space(8.0);

        ui.horizontal(|ui| {
            if ui.button("✓ 确认修改").clicked() {
                self.change_master_password(conn);
            }

            if ui.button("← 返回").clicked() {
                self.state = UiState::RequireMasterPassword;
                self.clear_messages();
                self.master_password.clear();
                self.new_password.clear();
                self.confirm_password.clear();
            }
        });

        ui.add_space(8.0);
        self.render_messages(ui);
    }

    /// 修改主密码
    fn change_master_password(&mut self, conn: &Connection) {
        self.clear_messages();

        if self.master_password.is_empty() {
            self.set_error("请输入当前主密码".to_string());
            return;
        }

        if self.new_password.len() < 6 {
            self.set_error("新密码长度至少 6 位".to_string());
            return;
        }

        if self.new_password != self.confirm_password {
            self.set_error("两次输入的新密码不一致".to_string());
            return;
        }

        let store = PasswordStore::new(conn);
        match store.change_master_password(&self.master_password, &self.new_password) {
            Ok(key) => {
                self.derived_key = Some(key);
                self.state = UiState::MainList;
                self.master_password.clear();
                self.new_password.clear();
                self.confirm_password.clear();
                self.set_success("主密码修改成功！".to_string());
                self.load_entries(conn);
            }
            Err(e) => {
                self.set_error(format!("修改失败: {}", e));
            }
        }
    }

    /// 加载密码列表（延迟解密，不立即解密密码）
    fn load_entries(&mut self, conn: &Connection) {
        let store = PasswordStore::new(conn);
        match store.get_all_entries_encrypted() {
            Ok(entries) => {
                self.entries = entries;
                self.visible_passwords.clear();
            }
            Err(e) => {
                self.set_error(format!("加载密码列表失败: {}", e));
            }
        }
    }

    /// 渲染主列表界面
    fn render_main_list(&mut self, ui: &mut egui::Ui, conn: &Connection) {
        ui.horizontal(|ui| {
            ui.heading("🔑 密码管理器");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("🔒 锁定").clicked() {
                    self.lock();
                }
            });
        });
        ui.separator();

        // 工具栏
        ui.horizontal(|ui| {
            if ui.button("➕ 新增").clicked() {
                self.state = UiState::AddEntry;
                self.form = PasswordForm::new();
                self.clear_messages();
            }

            if ui.button("🔑 生成密码").clicked() {
                self.state = UiState::Generator;
                self.clear_messages();
            }

            ui.separator();

            // 导出/导入按钮
            if ui.button("📤 导出").clicked() {
                self.export_dialog = ExportDialogState::SelectFormat;
            }

            if ui.button("📥 导入").clicked() {
                self.import_from_file(conn);
            }

            ui.separator();

            ui.label("🔍");
            let response = ui.add_sized(
                [150.0, 24.0],
                egui::TextEdit::singleline(&mut self.search_query).hint_text("搜索..."),
            );

            if response.changed() {
                self.search_entries(conn);
            }
        });

        ui.add_space(8.0);
        self.render_messages(ui);
        ui.add_space(4.0);

        // 密码列表表格
        let available_height = ui.available_height() - 40.0;
        egui::ScrollArea::vertical()
            .id_salt("password_list_scroll")
            .max_height(available_height)
            .show(ui, |ui| {
                self.render_password_table(ui, conn);
            });

        // 底部状态栏
        ui.separator();
        ui.horizontal(|ui| {
            ui.label(format!("共 {} 条记录", self.entries.len()));
        });

        // 导出格式选择弹窗
        self.render_export_dialog(ui, conn);
    }

    /// 渲染导出格式选择弹窗
    fn render_export_dialog(&mut self, ui: &mut egui::Ui, conn: &Connection) {
        if self.export_dialog == ExportDialogState::Closed {
            return;
        }

        egui::Window::new("选择导出格式")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ui.ctx(), |ui| {
                ui.label("请选择导出文件格式：");
                ui.add_space(8.0);

                ui.horizontal(|ui| {
                    if ui.button("📋 JSON 格式").clicked() {
                        self.export_dialog = ExportDialogState::Closed;
                        self.export_to_file(conn, ExportFormat::Json);
                    }

                    if ui.button("📊 CSV 格式").clicked() {
                        self.export_dialog = ExportDialogState::Closed;
                        self.export_to_file(conn, ExportFormat::Csv);
                    }
                });

                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    if ui.button("取消").clicked() {
                        self.export_dialog = ExportDialogState::Closed;
                    }
                });
            });
    }

    /// 渲染密码表格（延迟解密版本）
    fn render_password_table(&mut self, ui: &mut egui::Ui, conn: &Connection) {
        let entries = self.entries.clone();
        let key = self.derived_key;

        egui::Grid::new("password_table")
            .striped(true)
            .num_columns(4)
            .spacing([8.0, 4.0])
            .show(ui, |ui| {
                // 表头
                ui.strong("名称");
                ui.strong("账号");
                ui.strong("密码");
                ui.strong("操作");
                ui.end_row();

                if entries.is_empty() {
                    ui.label("暂无密码记录");
                    ui.label("");
                    ui.label("");
                    ui.label("");
                    ui.end_row();
                }

                for entry in &entries {
                    // 名称
                    ui.label(&entry.name);

                    // 账号
                    ui.label(&entry.username);

                    // 密码（可切换显示/隐藏，按需解密）
                    ui.horizontal(|ui| {
                        if let Some(pwd) = self.visible_passwords.get(&entry.id) {
                            ui.label(pwd);
                        } else {
                            ui.label("••••••••");
                        }
                    });

                    // 操作按钮
                    ui.horizontal(|ui| {
                        // 显示/隐藏密码（按需解密）
                        let eye_icon = if self.visible_passwords.contains_key(&entry.id) {
                            "🙈"
                        } else {
                            "👁"
                        };
                        if ui.button(eye_icon).clicked() {
                            if self.visible_passwords.contains_key(&entry.id) {
                                self.visible_passwords.remove(&entry.id);
                            } else if let Some(key) = key {
                                // 按需解密单个密码
                                let decrypted = entry.decrypt_password(&key);
                                self.visible_passwords.insert(entry.id, decrypted);
                            }
                        }

                        // 复制密码（按需解密）
                        if ui.button("📋").clicked() {
                            if let Some(key) = key {
                                let decrypted = entry.decrypt_password(&key);
                                self.copy_to_clipboard(&decrypted);
                                self.set_success("密码已复制到剪贴板".to_string());
                            }
                        }

                        // 编辑（按需解密）
                        if ui.button("✏️").clicked() {
                            if let Some(key) = key {
                                let decrypted_entry = entry.to_decrypted(&key);
                                self.state = UiState::EditEntry(entry.id);
                                self.form = PasswordForm::from_entry(&decrypted_entry);
                                self.clear_messages();
                            }
                        }

                        // 删除
                        if ui.button("🗑").clicked() {
                            self.delete_entry(conn, entry.id);
                        }
                    });

                    ui.end_row();
                }
            });
    }

    /// 搜索密码（延迟解密）
    fn search_entries(&mut self, conn: &Connection) {
        let store = PasswordStore::new(conn);

        let result = if self.search_query.is_empty() {
            store.get_all_entries_encrypted()
        } else {
            store.search_entries_encrypted(&self.search_query)
        };

        match result {
            Ok(entries) => {
                self.entries = entries;
                self.visible_passwords.clear();
            }
            Err(e) => {
                self.set_error(format!("搜索失败: {}", e));
            }
        }
    }

    /// 删除密码条目
    fn delete_entry(&mut self, conn: &Connection, id: i64) {
        let store = PasswordStore::new(conn);
        match store.delete_entry(id) {
            Ok(()) => {
                self.set_success("已删除".to_string());
                self.load_entries(conn);
            }
            Err(e) => {
                self.set_error(format!("删除失败: {}", e));
            }
        }
    }

    /// 渲染新增条目界面
    fn render_add_entry(&mut self, ui: &mut egui::Ui, conn: &Connection) {
        ui.horizontal(|ui| {
            ui.heading("➕ 新增密码");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("← 返回").clicked() {
                    self.state = UiState::MainList;
                    self.clear_messages();
                }
            });
        });
        ui.separator();

        self.render_password_form(ui);

        ui.add_space(8.0);

        ui.horizontal(|ui| {
            if ui.button("💾 保存").clicked() {
                self.save_new_entry(conn);
            }

            if ui.button("🎲 生成密码").clicked() {
                let config = crypto::PasswordConfig {
                    length: self.generator_config.length,
                    use_uppercase: self.generator_config.use_uppercase,
                    use_lowercase: self.generator_config.use_lowercase,
                    use_digits: self.generator_config.use_digits,
                    use_symbols: self.generator_config.use_symbols,
                };
                self.form.password = crypto::generate_password(&config);
                self.form.show_password = true;
            }
        });

        ui.add_space(8.0);
        self.render_messages(ui);
    }

    /// 保存新条目
    fn save_new_entry(&mut self, conn: &Connection) {
        self.clear_messages();

        if !self.form.is_valid() {
            self.set_error("请填写名称、账号和密码".to_string());
            return;
        }

        if let Some(key) = &self.derived_key {
            let store = PasswordStore::new(conn);
            let entry = self.form.to_new_entry();

            match store.add_entry(&entry, key) {
                Ok(_) => {
                    self.set_success("保存成功".to_string());
                    self.state = UiState::MainList;
                    self.load_entries(conn);
                }
                Err(e) => {
                    self.set_error(format!("保存失败: {}", e));
                }
            }
        }
    }

    /// 渲染编辑条目界面
    fn render_edit_entry(&mut self, ui: &mut egui::Ui, conn: &Connection, id: i64) {
        ui.horizontal(|ui| {
            ui.heading("✏️ 编辑密码");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("← 返回").clicked() {
                    self.state = UiState::MainList;
                    self.clear_messages();
                }
            });
        });
        ui.separator();

        self.render_password_form(ui);

        ui.add_space(8.0);

        ui.horizontal(|ui| {
            if ui.button("💾 保存修改").clicked() {
                self.save_edited_entry(conn, id);
            }

            if ui.button("🎲 生成密码").clicked() {
                let config = crypto::PasswordConfig {
                    length: self.generator_config.length,
                    use_uppercase: self.generator_config.use_uppercase,
                    use_lowercase: self.generator_config.use_lowercase,
                    use_digits: self.generator_config.use_digits,
                    use_symbols: self.generator_config.use_symbols,
                };
                self.form.password = crypto::generate_password(&config);
                self.form.show_password = true;
            }
        });

        ui.add_space(8.0);
        self.render_messages(ui);
    }

    /// 保存编辑的条目
    fn save_edited_entry(&mut self, conn: &Connection, id: i64) {
        self.clear_messages();

        if !self.form.is_valid() {
            self.set_error("请填写名称、账号和密码".to_string());
            return;
        }

        if let Some(key) = &self.derived_key {
            let store = PasswordStore::new(conn);
            let entry = PasswordEntry {
                id,
                name: self.form.name.clone(),
                url: if self.form.url.is_empty() {
                    None
                } else {
                    Some(self.form.url.clone())
                },
                username: self.form.username.clone(),
                password: self.form.password.clone(),
                notes: if self.form.notes.is_empty() {
                    None
                } else {
                    Some(self.form.notes.clone())
                },
                created_at: String::new(), // 不更新
                updated_at: String::new(), // 数据库会自动更新
            };

            match store.update_entry(&entry, key) {
                Ok(()) => {
                    self.set_success("修改已保存".to_string());
                    self.state = UiState::MainList;
                    self.load_entries(conn);
                }
                Err(e) => {
                    self.set_error(format!("保存失败: {}", e));
                }
            }
        }
    }

    /// 渲染密码表单
    fn render_password_form(&mut self, ui: &mut egui::Ui) {
        egui::Grid::new("password_form")
            .num_columns(2)
            .spacing([8.0, 8.0])
            .show(ui, |ui| {
                ui.label("名称 *");
                ui.text_edit_singleline(&mut self.form.name);
                ui.end_row();

                ui.label("网址");
                ui.text_edit_singleline(&mut self.form.url);
                ui.end_row();

                ui.label("账号 *");
                ui.text_edit_singleline(&mut self.form.username);
                ui.end_row();

                ui.label("密码 *");
                ui.horizontal(|ui| {
                    if self.form.show_password {
                        ui.text_edit_singleline(&mut self.form.password);
                    } else {
                        ui.add(
                            egui::TextEdit::singleline(&mut self.form.password)
                                .password(true),
                        );
                    }

                    let eye_icon = if self.form.show_password {
                        "🙈"
                    } else {
                        "👁"
                    };
                    if ui.button(eye_icon).clicked() {
                        self.form.show_password = !self.form.show_password;
                    }
                });
                ui.end_row();

                ui.label("备注");
                ui.text_edit_multiline(&mut self.form.notes);
                ui.end_row();
            });
    }

    /// 渲染密码生成器界面
    fn render_generator(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.heading("🔑 密码生成器");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("← 返回").clicked() {
                    self.state = UiState::MainList;
                    self.clear_messages();
                }
            });
        });
        ui.separator();

        ui.add_space(10.0);

        // 配置选项
        ui.horizontal(|ui| {
            ui.label("密码长度：");
            ui.add(egui::Slider::new(&mut self.generator_config.length, 4..=64));
        });

        ui.add_space(4.0);

        ui.horizontal(|ui| {
            ui.checkbox(&mut self.generator_config.use_uppercase, "大写字母 (A-Z)");
            ui.checkbox(&mut self.generator_config.use_lowercase, "小写字母 (a-z)");
        });

        ui.horizontal(|ui| {
            ui.checkbox(&mut self.generator_config.use_digits, "数字 (0-9)");
            ui.checkbox(&mut self.generator_config.use_symbols, "特殊符号 (!@#$...)");
        });

        ui.add_space(8.0);

        // 生成按钮
        if ui.button("🎲 生成密码").clicked() {
            let config = crypto::PasswordConfig {
                length: self.generator_config.length,
                use_uppercase: self.generator_config.use_uppercase,
                use_lowercase: self.generator_config.use_lowercase,
                use_digits: self.generator_config.use_digits,
                use_symbols: self.generator_config.use_symbols,
            };
            self.generated_password = crypto::generate_password(&config);
        }

        ui.add_space(8.0);

        // 显示生成的密码
        if !self.generated_password.is_empty() {
            ui.horizontal(|ui| {
                ui.label("生成的密码：");
                ui.monospace(&self.generated_password);

                if ui.button("📋 复制").clicked() {
                    self.copy_to_clipboard(&self.generated_password);
                    self.set_success("已复制到剪贴板".to_string());
                }
            });

            ui.add_space(4.0);
            ui.label(format!("长度: {} 位", self.generated_password.len()));
        }

        ui.add_space(8.0);
        self.render_messages(ui);
    }

    /// 导出密码数据到文件
    fn export_to_file(&mut self, conn: &Connection, format: ExportFormat) {
        self.clear_messages();

        let (title, extension, filter_name) = match format {
            ExportFormat::Json => ("导出密码数据 - JSON", "json", "JSON 文件"),
            ExportFormat::Csv => ("导出密码数据 - CSV", "csv", "CSV 文件"),
        };

        let default_name = format!("passwords.{}", extension);
        let dialog = rfd::FileDialog::new()
            .set_title(title)
            .set_file_name(&default_name)
            .add_filter(filter_name, &[extension]);

        if let Some(path) = dialog.save_file() {
            if let Some(key) = &self.derived_key {
                let store = PasswordStore::new(conn);
                match store.export_entries(key, format) {
                    Ok(data) => {
                        match std::fs::write(&path, &data) {
                            Ok(()) => {
                                let fmt_name = match format {
                                    ExportFormat::Json => "JSON",
                                    ExportFormat::Csv => "CSV",
                                };
                                self.set_success(format!(
                                    "已导出 {} 条记录到 {}（{} 格式）",
                                    self.entries.len(),
                                    path.display(),
                                    fmt_name
                                ));
                                log::info!("密码数据已导出到: {}", path.display());
                            }
                            Err(e) => {
                                self.set_error(format!("写入文件失败: {}", e));
                            }
                        }
                    }
                    Err(e) => {
                        self.set_error(format!("导出失败: {}", e));
                    }
                }
            }
        }
    }

    /// 从文件导入密码数据
    fn import_from_file(&mut self, conn: &Connection) {
        self.clear_messages();

        let dialog = rfd::FileDialog::new()
            .set_title("选择导入文件")
            .add_filter("支持的格式", &["json", "csv"])
            .add_filter("JSON 文件", &["json"])
            .add_filter("CSV 文件", &["csv"]);

        if let Some(path) = dialog.pick_file() {
            let content = match std::fs::read_to_string(&path) {
                Ok(c) => c,
                Err(e) => {
                    self.set_error(format!("读取文件失败: {}", e));
                    return;
                }
            };

            if content.is_empty() {
                self.set_error("文件内容为空，请检查文件是否正确".to_string());
                return;
            }

            // 根据文件扩展名判断格式
            let format = match path.extension().and_then(|e| e.to_str()) {
                Some("csv") => ExportFormat::Csv,
                Some("json") => ExportFormat::Json,
                _ => {
                    self.set_error("不支持的文件格式，请使用 .json 或 .csv 文件".to_string());
                    return;
                }
            };

            if let Some(key) = &self.derived_key {
                let store = PasswordStore::new(conn);
                match store.import_entries(&content, key, format) {
                    Ok(count) => {
                        let fmt_name = match format {
                            ExportFormat::Json => "JSON",
                            ExportFormat::Csv => "CSV",
                        };
                        self.set_success(format!(
                            "成功从 {} 导入 {} 条记录（{} 格式）",
                            path.file_name().unwrap_or_default().to_string_lossy(),
                            count,
                            fmt_name
                        ));
                        self.load_entries(conn);
                    }
                    Err(e) => {
                        self.set_error(format!("导入失败: {}", e));
                    }
                }
            }
        }
    }

    /// 复制到剪贴板
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

    /// 锁定密码库
    fn lock(&mut self) {
        self.derived_key = None;
        self.entries.clear();
        self.visible_passwords.clear();
        self.search_query.clear();
        self.state = UiState::RequireMasterPassword;
        self.clear_messages();
    }
}
