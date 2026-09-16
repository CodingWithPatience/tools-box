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
    /// 编辑密码条目
    EditEntry(i64),
}

/// 导出格式选择弹窗状态
#[derive(Debug, Clone, PartialEq)]
enum ExportDialogState {
    Closed,
    SelectFormat,
}

/// 每页条数默认值
const DEFAULT_PAGE_SIZE: usize = 20;
/// 可选的每页条数
const PAGE_SIZE_OPTIONS: [usize; 4] = [10, 20, 30, 50];
/// 为分页栏预留的高度（窄窗口下分页栏会换行，按两行预留避免被裁剪）
const PAGINATION_BAR_RESERVE: f32 = 64.0;

/// 计算总页数（无记录时按 1 页展示）
fn total_pages(entry_count: usize, page_size: usize) -> usize {
    if page_size == 0 {
        return 1;
    }

    entry_count.div_ceil(page_size).max(1)
}

/// 将页码收敛到有效范围 `1..=总页数`
fn clamp_page(page: usize, entry_count: usize, page_size: usize) -> usize {
    page.clamp(1, total_pages(entry_count, page_size))
}

/// 计算某页在条目列表中的下标范围 `[start, end)`
fn page_bounds(entry_count: usize, page_size: usize, page: usize) -> (usize, usize) {
    if entry_count == 0 || page_size == 0 {
        return (0, 0);
    }

    let page = clamp_page(page, entry_count, page_size);
    let start = (page - 1).saturating_mul(page_size).min(entry_count);

    (start, start.saturating_add(page_size).min(entry_count))
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
    /// 新增密码弹窗是否打开
    add_dialog_open: bool,
    /// 生成密码弹窗是否打开
    generator_dialog_open: bool,
    /// 待删除的密码条目 ID（用于确认弹窗）
    pending_delete_id: Option<i64>,
    /// 当前页码（从 1 开始）
    page: usize,
    /// 每页展示条数
    page_size: usize,
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
            add_dialog_open: false,
            generator_dialog_open: false,
            pending_delete_id: None,
            page: 1,
            page_size: DEFAULT_PAGE_SIZE,
        }
    }

    /// 渲染主界面
    pub fn render(&mut self, ui: &mut egui::Ui, conn: &Connection) {
        match self.state.clone() {
            UiState::RequireMasterPassword => self.render_require_password(ui, conn),
            UiState::SetMasterPassword => self.render_set_password(ui, conn),
            UiState::ChangeMasterPassword => self.render_change_password(ui, conn),
            UiState::MainList => self.render_main_list(ui, conn),
            UiState::EditEntry(id) => self.render_edit_entry(ui, conn, id),
        }
    }

    /// 渲染消息提示
    fn render_messages(&mut self, ui: &mut egui::Ui) {
        if let Some(err) = &self.error_msg {
            ui.colored_label(egui::Color32::from_rgb(220, 50, 50), format!("⚠ {}", err));
        }
        if let Some(success) = &self.success_msg.clone() {
            ui.colored_label(
                egui::Color32::from_rgb(50, 180, 50),
                format!("✓ {}", success),
            );
        }
    }

    /// 清除消息
    fn clear_messages(&mut self) {
        self.error_msg = None;
        self.success_msg = None;
    }

    /// 构建密码生成配置
    fn build_password_config(&self) -> crypto::PasswordConfig {
        crypto::PasswordConfig {
            length: self.generator_config.length,
            use_uppercase: self.generator_config.use_uppercase,
            use_lowercase: self.generator_config.use_lowercase,
            use_digits: self.generator_config.use_digits,
            use_symbols: self.generator_config.use_symbols,
        }
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
                self.add_dialog_open = true;
                self.form = PasswordForm::new();
                self.clear_messages();
            }

            if ui.button("🔑 生成密码").clicked() {
                self.generator_dialog_open = true;
                self.generated_password.clear();
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
        // 弹窗打开时跳过主列表消息渲染，避免重复显示
        if !self.add_dialog_open && !self.generator_dialog_open {
            self.render_messages(ui);
        }
        ui.add_space(4.0);

        // 密码列表表格（预留分页栏高度：窄窗口下分页栏会自动换行，预留两行避免被裁剪）
        let available_height = (ui.available_height() - PAGINATION_BAR_RESERVE).max(60.0);
        egui::ScrollArea::vertical()
            .id_salt("password_list_scroll")
            .max_height(available_height)
            .show(ui, |ui| {
                self.render_password_table(ui, conn);
            });

        // 底部状态栏 + 分页
        ui.separator();
        self.render_pagination_bar(ui);

        // 弹窗渲染
        self.render_export_dialog(ui, conn);
        self.render_add_dialog(ui, conn);
        self.render_generator_dialog(ui);
        self.render_delete_confirm_dialog(ui, conn);
    }

    /// 设置每页条数（变化时回到第一页）
    fn set_page_size(&mut self, page_size: usize) {
        if page_size != self.page_size {
            self.page_size = page_size;
            self.page = 1;
        }
    }

    /// 渲染分页工具栏（每页条数、翻页、记录数统计）
    ///
    /// 使用自动换行布局，窄窗口下统计信息会换到下一行，不会被裁剪
    fn render_pagination_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            // 每页条数选择（切换后回到第一页）
            ui.label("每页");
            let mut page_size = self.page_size;
            egui::ComboBox::from_id_salt("password_page_size")
                .selected_text(format!("{} 条", page_size))
                .width(80.0)
                .show_ui(ui, |ui| {
                    for option in PAGE_SIZE_OPTIONS {
                        ui.selectable_value(&mut page_size, option, format!("{} 条", option));
                    }
                });
            self.set_page_size(page_size);

            ui.separator();

            let entry_count = self.entries.len();
            let page = clamp_page(self.page, entry_count, self.page_size);
            let total = total_pages(entry_count, self.page_size);
            self.page = page;

            if ui
                .add_enabled(page > 1, egui::Button::new("⏮"))
                .on_hover_text("首页")
                .clicked()
            {
                self.page = 1;
            }
            if ui
                .add_enabled(page > 1, egui::Button::new("◀"))
                .on_hover_text("上一页")
                .clicked()
            {
                self.page = page - 1;
            }

            ui.label(format!("第 {} / {} 页", page, total));

            if ui
                .add_enabled(page < total, egui::Button::new("▶"))
                .on_hover_text("下一页")
                .clicked()
            {
                self.page = page + 1;
            }
            if ui
                .add_enabled(page < total, egui::Button::new("⏭"))
                .on_hover_text("末页")
                .clicked()
            {
                self.page = total;
            }

            ui.separator();

            let (start, end) = page_bounds(entry_count, self.page_size, page);
            if entry_count == 0 {
                ui.label("共 0 条记录");
            } else {
                ui.label(format!(
                    "共 {} 条记录（当前显示 {}-{}）",
                    entry_count,
                    start + 1,
                    end
                ));
            }
        });
    }

    /// 渲染删除确认弹窗
    fn render_delete_confirm_dialog(&mut self, ui: &mut egui::Ui, conn: &Connection) {
        let Some(delete_id) = self.pending_delete_id else {
            return;
        };

        let entry_info = self
            .entries
            .iter()
            .find(|e| e.id == delete_id)
            .map(|e| format!("「{}」(账号: {})", e.name, e.username))
            .unwrap_or_else(|| "未知记录".to_string());

        let mut open = true;
        let mut confirmed = false;

        egui::Window::new("确认删除")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ui.ctx(), |ui| {
                ui.label(format!(
                    "确定要删除密码记录 {} 吗？此操作不可撤销。",
                    entry_info
                ));
                ui.add_space(12.0);

                ui.horizontal(|ui| {
                    if ui.button("🗑 确认删除").clicked() {
                        confirmed = true;
                    }
                    if ui.button("取消").clicked() {
                        self.pending_delete_id = None;
                    }
                });
            });

        if confirmed {
            self.delete_entry(conn, delete_id);
            self.pending_delete_id = None;
        } else if !open {
            self.pending_delete_id = None;
        }
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

    /// 渲染密码表格（延迟解密版本，仅渲染当前页）
    fn render_password_table(&mut self, ui: &mut egui::Ui, _conn: &Connection) {
        let page_size = self.page_size;
        let page = clamp_page(self.page, self.entries.len(), page_size);
        let (start, end) = page_bounds(self.entries.len(), page_size, page);
        // 只克隆当前页条目，避免每帧复制全部记录
        let entries = self.entries[start..end].to_vec();
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
                            self.pending_delete_id = Some(entry.id);
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
                // 搜索结果变化后回到第一页
                self.page = 1;
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

    /// 渲染新增密码弹窗
    fn render_add_dialog(&mut self, ui: &mut egui::Ui, conn: &Connection) {
        if !self.add_dialog_open {
            return;
        }

        let mut open = self.add_dialog_open;
        let mut close_dialog = false;

        egui::Window::new("➕ 新增密码")
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_width(350.0)
            .show(ui.ctx(), |ui| {
                self.render_password_form(ui);

                ui.add_space(8.0);

                ui.horizontal(|ui| {
                    if ui.button("💾 保存").clicked() {
                        self.save_new_entry(conn);
                        if self.error_msg.is_none() {
                            close_dialog = true;
                        }
                    }

                    if ui.button("🎲 生成密码").clicked() {
                        let config = self.build_password_config();
                        self.form.password = crypto::generate_password(&config);
                        self.form.show_password = true;
                    }
                });

                ui.add_space(4.0);
                self.render_messages(ui);
            });

        if close_dialog || !open {
            self.add_dialog_open = false;
        }
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
                let config = self.build_password_config();
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
                        ui.add(egui::TextEdit::singleline(&mut self.form.password).password(true));
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

    /// 渲染密码生成器弹窗
    fn render_generator_dialog(&mut self, ui: &mut egui::Ui) {
        if !self.generator_dialog_open {
            return;
        }

        let mut open = self.generator_dialog_open;

        egui::Window::new("🔑 密码生成器")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(320.0)
            .show(ui.ctx(), |ui| {
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
                    let config = self.build_password_config();
                    self.generated_password = crypto::generate_password(&config);
                }

                ui.add_space(8.0);

                // 显示生成的密码
                if !self.generated_password.is_empty() {
                    ui.horizontal(|ui| {
                        ui.label("生成的密码：");
                        ui.monospace(&self.generated_password);
                    });

                    ui.add_space(4.0);

                    ui.horizontal(|ui| {
                        ui.label(format!("长度: {} 位", self.generated_password.len()));

                        if ui.button("📋 复制").clicked() {
                            self.copy_to_clipboard(&self.generated_password);
                            self.set_success("已复制到剪贴板".to_string());
                        }
                    });
                }

                ui.add_space(4.0);
                self.render_messages(ui);
            });

        if !open {
            self.generator_dialog_open = false;
        }
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
                    Ok(data) => match std::fs::write(&path, &data) {
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
                    },
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
        self.pending_delete_id = None;
        self.page = 1;
        self.state = UiState::RequireMasterPassword;
        self.clear_messages();
    }
}

#[cfg(test)]
mod tests {
    use super::{
        DEFAULT_PAGE_SIZE, PAGE_SIZE_OPTIONS, PasswordManagerUi, clamp_page, page_bounds,
        total_pages,
    };

    #[test]
    fn total_pages_rounds_up_and_keeps_at_least_one() {
        assert_eq!(total_pages(0, DEFAULT_PAGE_SIZE), 1);
        assert_eq!(total_pages(1, DEFAULT_PAGE_SIZE), 1);
        assert_eq!(total_pages(20, DEFAULT_PAGE_SIZE), 1);
        assert_eq!(total_pages(21, DEFAULT_PAGE_SIZE), 2);
        assert_eq!(total_pages(40, DEFAULT_PAGE_SIZE), 2);
        assert_eq!(total_pages(50, 10), 5);
        assert_eq!(total_pages(51, 10), 6);
        // 每页条数为 0 时退化为单页，避免除零
        assert_eq!(total_pages(100, 0), 1);
    }

    #[test]
    fn clamp_page_keeps_page_within_range() {
        assert_eq!(clamp_page(0, 100, DEFAULT_PAGE_SIZE), 1);
        assert_eq!(clamp_page(1, 100, DEFAULT_PAGE_SIZE), 1);
        assert_eq!(clamp_page(3, 100, DEFAULT_PAGE_SIZE), 3);
        assert_eq!(clamp_page(99, 100, DEFAULT_PAGE_SIZE), 5);
        // 无记录时始终为第 1 页
        assert_eq!(clamp_page(7, 0, DEFAULT_PAGE_SIZE), 1);
    }

    #[test]
    fn page_bounds_covers_full_and_partial_pages() {
        // 默认每页 20 条：45 条记录 → 3 页（20 + 20 + 5）
        assert_eq!(page_bounds(45, DEFAULT_PAGE_SIZE, 1), (0, 20));
        assert_eq!(page_bounds(45, DEFAULT_PAGE_SIZE, 2), (20, 40));
        assert_eq!(page_bounds(45, DEFAULT_PAGE_SIZE, 3), (40, 45));
        // 越界页码收敛到最后一页
        assert_eq!(page_bounds(45, DEFAULT_PAGE_SIZE, 9), (40, 45));
        // 页码为 0 收敛到第一页
        assert_eq!(page_bounds(45, DEFAULT_PAGE_SIZE, 0), (0, 20));
        // 每页条数大于记录数
        assert_eq!(page_bounds(3, 50, 1), (0, 3));
        // 无记录
        assert_eq!(page_bounds(0, DEFAULT_PAGE_SIZE, 1), (0, 0));
        assert_eq!(page_bounds(10, 0, 1), (0, 0));
    }

    #[test]
    fn page_size_options_and_default_match_requirements() {
        assert_eq!(DEFAULT_PAGE_SIZE, 20);
        assert_eq!(PAGE_SIZE_OPTIONS, [10, 20, 30, 50]);

        let ui = PasswordManagerUi::new();
        assert_eq!(ui.page_size, DEFAULT_PAGE_SIZE);
        assert_eq!(ui.page, 1);
    }

    #[test]
    fn changing_page_size_returns_to_first_page() {
        let mut ui = PasswordManagerUi::new();
        assert_eq!(ui.page_size, DEFAULT_PAGE_SIZE);

        // 只有实际切换每页条数时才回到第一页
        ui.set_page_size(DEFAULT_PAGE_SIZE);
        assert_eq!(ui.page, 1);

        ui.page = 3;
        ui.set_page_size(50);
        assert_eq!(ui.page_size, 50);
        assert_eq!(ui.page, 1);

        // 切换后页码始终落在有效范围内
        for option in PAGE_SIZE_OPTIONS {
            ui.set_page_size(option);
            let page = clamp_page(ui.page, 45, ui.page_size);
            assert_eq!(page, 1);
            assert!(page_bounds(45, ui.page_size, page).1 <= 45);
        }
    }

    #[test]
    fn pagination_keeps_page_in_range_after_entry_changes() {
        let mut ui = PasswordManagerUi::new();
        ui.page_size = 10;
        ui.page = 5;

        // 记录减少（如删除或搜索命中变少）后页码自动收敛，切片范围仍有效
        for count in [45, 12, 3, 0] {
            let page = clamp_page(ui.page, count, ui.page_size);
            let (start, end) = page_bounds(count, ui.page_size, page);
            assert!(start <= end && end <= count, "count={count} 范围非法");
            assert!(page >= 1);
        }
    }

    #[test]
    fn password_table_renders_each_page_without_panic() {
        use crate::plugins::password_manager::models::EncryptedPasswordEntry;

        let conn = match rusqlite::Connection::open_in_memory() {
            Ok(conn) => conn,
            Err(e) => panic!("创建内存数据库失败: {e}"),
        };
        let ctx = egui::Context::default();
        let mut ui_state = PasswordManagerUi::new();
        ui_state.entries = (1..=45)
            .map(|id| EncryptedPasswordEntry {
                id,
                name: format!("entry-{id:03}"),
                url: None,
                username: format!("user-{id}"),
                encrypted_password: Vec::new(),
                iv: Vec::new(),
                notes: None,
            })
            .collect();

        // 逐页渲染（含最后一页的部分页），验证分页切片不会越界
        let total = total_pages(ui_state.entries.len(), ui_state.page_size);
        for page in 1..=total {
            ui_state.page = page;
            let _ = ctx.run(egui::RawInput::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    ui_state.render_password_table(ui, &conn);
                });
            });
        }

        // 切换到最小分页数量后同样逐页渲染
        ui_state.set_page_size(10);
        let total = total_pages(ui_state.entries.len(), ui_state.page_size);
        for page in 1..=total {
            ui_state.page = page;
            let _ = ctx.run(egui::RawInput::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    ui_state.render_password_table(ui, &conn);
                });
            });
        }
    }
}
