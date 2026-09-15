use anyhow::{Context, Result};
use rusqlite::Connection;
use std::collections::HashSet;

use super::parser;
use super::store::*;

/// UI 状态
#[derive(Debug, Clone, PartialEq)]
enum UiState {
    /// 环境列表主界面
    EnvironmentList,
    /// 编辑环境名称
    EditEnvironment(i64),
    /// 条目列表
    EntryList(i64),
    /// 编辑条目
    EditEntry(i64, i64), // (env_id, entry_id)
}

/// 待用户确认的 hosts 应用（存在主机名冲突时）
#[derive(Debug, Clone)]
struct PendingApply {
    /// 待写入的环境分组
    groups: Vec<parser::HostsGroup>,
    /// 参与本次应用的环境名称
    env_names: Vec<String>,
    /// 检测到的主机名冲突
    conflicts: Vec<parser::HostnameConflict>,
}

/// Hosts 管理器 UI
pub struct HostsManagerUi {
    state: UiState,
    environments: Vec<Environment>,
    entries: Vec<DbHostsEntry>,
    selected_env: Option<i64>,
    env_form: EnvironmentForm,
    entry_form: HostsEntryForm,
    error_msg: Option<String>,
    success_msg: Option<String>,
    /// 警告提示（如主机名冲突）
    warning_msg: Option<String>,
    /// 新增环境弹窗是否打开
    add_env_dialog_open: bool,
    /// 新增条目弹窗是否打开
    add_entry_dialog_open: bool,
    /// 待删除的环境 ID（用于确认弹窗）
    pending_delete_env_id: Option<i64>,
    /// 待删除的条目 ID（用于确认弹窗）
    pending_delete_entry_id: Option<i64>,
    /// 待确认的应用（存在主机名冲突时）
    pending_apply: Option<PendingApply>,
}

impl HostsManagerUi {
    pub fn new() -> Self {
        Self {
            state: UiState::EnvironmentList,
            environments: Vec::new(),
            entries: Vec::new(),
            selected_env: None,
            env_form: EnvironmentForm::new(),
            entry_form: HostsEntryForm::new(),
            error_msg: None,
            success_msg: None,
            warning_msg: None,
            add_env_dialog_open: false,
            add_entry_dialog_open: false,
            pending_delete_env_id: None,
            pending_delete_entry_id: None,
            pending_apply: None,
        }
    }

    /// 渲染主界面
    pub fn render(&mut self, ui: &mut egui::Ui, conn: &Connection) {
        // 离开环境列表时丢弃待确认内容，避免之后写入过期快照
        if self.state != UiState::EnvironmentList {
            self.pending_apply = None;
        }

        match self.state.clone() {
            UiState::EnvironmentList => self.render_environment_list(ui, conn),
            UiState::EditEnvironment(id) => self.render_edit_environment(ui, conn, id),
            UiState::EntryList(env_id) => self.render_entry_list(ui, conn, env_id),
            UiState::EditEntry(env_id, entry_id) => {
                self.render_edit_entry(ui, conn, env_id, entry_id)
            }
        }
    }

    /// 渲染消息提示
    fn render_messages(&mut self, ui: &mut egui::Ui) {
        if let Some(err) = &self.error_msg {
            ui.colored_label(egui::Color32::from_rgb(220, 50, 50), format!("⚠ {}", err));
        }
        if let Some(success) = &self.success_msg {
            ui.colored_label(
                egui::Color32::from_rgb(50, 180, 50),
                format!("✓ {}", success),
            );
        }
        if let Some(warning) = &self.warning_msg {
            ui.colored_label(
                egui::Color32::from_rgb(230, 150, 40),
                format!("⚠ {}", warning),
            );
        }
    }

    /// 清除消息
    fn clear_messages(&mut self) {
        self.error_msg = None;
        self.success_msg = None;
        self.warning_msg = None;
    }

    /// 设置错误消息
    fn set_error(&mut self, msg: String) {
        self.error_msg = Some(msg);
        self.success_msg = None;
        self.warning_msg = None;
    }

    /// 设置成功消息
    fn set_success(&mut self, msg: String) {
        self.success_msg = Some(msg);
        self.error_msg = None;
        self.warning_msg = None;
    }

    /// 设置警告消息（不影响已有的成功/错误提示）
    fn set_warning(&mut self, msg: String) {
        self.warning_msg = Some(msg);
    }

    /// 加载环境列表
    fn load_environments(&mut self, conn: &Connection) {
        let store = HostsStore::new(conn);
        match store.get_all_environments() {
            Ok(envs) => {
                self.environments = envs;
            }
            Err(e) => {
                self.set_error(format!("加载环境列表失败: {}", e));
            }
        }
    }

    /// 加载条目列表
    fn load_entries(&mut self, conn: &Connection, env_id: i64) {
        let store = HostsStore::new(conn);
        match store.get_entries_by_env(env_id) {
            Ok(entries) => {
                self.entries = entries;
            }
            Err(e) => {
                self.set_error(format!("加载条目列表失败: {}", e));
            }
        }
    }

    /// 渲染环境列表主界面
    fn render_environment_list(&mut self, ui: &mut egui::Ui, conn: &Connection) {
        self.load_environments(conn);

        ui.horizontal(|ui| {
            ui.heading("🌐 Hosts 管理器");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("🔄 刷新").clicked() {
                    self.load_environments(conn);
                }
            });
        });
        ui.separator();

        // 工具栏
        ui.horizontal(|ui| {
            if ui.button("➕ 新增环境").clicked() {
                self.add_env_dialog_open = true;
                self.env_form = EnvironmentForm::new();
                self.clear_messages();
            }

            let active_count = self.environments.iter().filter(|env| env.is_active).count();
            if ui
                .button(format!("💾 应用到系统（{} 个环境）", active_count))
                .clicked()
            {
                self.apply_hosts(conn);
            }
        });

        ui.add_space(8.0);
        self.render_messages(ui);
        ui.add_space(4.0);

        // 环境列表表格
        let available_height = ui.available_height() - 60.0;
        egui::ScrollArea::vertical()
            .id_salt("hosts_env_scroll")
            .max_height(available_height)
            .show(ui, |ui| {
                egui::Grid::new("hosts_env_table")
                    .striped(true)
                    .num_columns(3)
                    .spacing([8.0, 4.0])
                    .show(ui, |ui| {
                        // 表头
                        ui.strong("环境名称");
                        ui.strong("状态");
                        ui.strong("操作");
                        ui.end_row();

                        let envs = self.environments.clone();
                        if envs.is_empty() {
                            ui.label("暂无环境，请点击【新增环境】");
                            ui.label("");
                            ui.label("");
                            ui.end_row();
                        }

                        for env in &envs {
                            // 环境名称
                            let radio_text = if env.is_active {
                                format!("☑ {}", env.name)
                            } else {
                                format!("☐ {}", env.name)
                            };

                            if ui.selectable_label(env.is_active, &radio_text).clicked() {
                                self.toggle_environment(conn, env.id);
                            }

                            // 状态
                            let status = if env.is_active {
                                "已启用"
                            } else {
                                "已禁用"
                            };
                            ui.label(status);

                            // 操作
                            ui.horizontal(|ui| {
                                if ui.button("📝 条目").clicked() {
                                    self.state = UiState::EntryList(env.id);
                                    self.selected_env = Some(env.id);
                                    self.load_entries(conn, env.id);
                                    self.clear_messages();
                                }

                                if ui.button("✏️").clicked() {
                                    self.state = UiState::EditEnvironment(env.id);
                                    self.env_form = EnvironmentForm::from_env(env);
                                    self.clear_messages();
                                }

                                if ui.button("🗑").clicked() {
                                    self.pending_delete_env_id = Some(env.id);
                                }
                            });

                            ui.end_row();
                        }
                    });
            });

        // 底部提示
        ui.separator();
        ui.horizontal(|ui| {
            let active_count = self.environments.iter().filter(|env| env.is_active).count();
            ui.label(format!(
                "已启用 {} 个环境（可同时启用多个环境）",
                active_count
            ));
            ui.label("⚠ 应用环境需要管理员权限运行程序");
        });

        // 弹窗渲染
        self.render_add_environment_dialog(ui, conn);
        self.render_delete_env_confirm_dialog(ui, conn);
        self.render_apply_confirm_dialog(ui, conn);
    }

    /// 切换环境激活状态（多个环境可同时生效）
    fn toggle_environment(&mut self, conn: &Connection, env_id: i64) {
        let store = HostsStore::new(conn);

        // 检查当前是否已激活
        let Some(env) = self.environments.iter().find(|e| e.id == env_id) else {
            self.set_error("环境不存在，请刷新列表后重试".to_string());
            self.load_environments(conn);
            return;
        };
        let env_name = env.name.clone();
        let is_active = env.is_active;

        match store.set_environment_active(env_id, !is_active) {
            Ok(()) => {
                log::info!(
                    "环境 '{}'（id={}）已{}",
                    env_name,
                    env_id,
                    if is_active { "禁用" } else { "启用" }
                );
                self.load_environments(conn);
                if is_active {
                    self.set_success(format!("已禁用环境 '{}'", env_name));
                } else {
                    let active_count = self.environments.iter().filter(|env| env.is_active).count();
                    self.set_success(format!(
                        "已启用环境 '{}'（当前共 {} 个环境生效）",
                        env_name, active_count
                    ));
                }
            }
            Err(e) => {
                self.set_error(format!("切换环境失败: {}", e));
            }
        }
    }

    /// 删除环境
    fn delete_environment(&mut self, conn: &Connection, env_id: i64) {
        let store = HostsStore::new(conn);
        match store.delete_environment(env_id) {
            Ok(()) => {
                self.set_success("环境已删除".to_string());
                self.load_environments(conn);
            }
            Err(e) => {
                self.set_error(format!("删除环境失败: {}", e));
            }
        }
    }

    /// 渲染删除环境确认弹窗
    fn render_delete_env_confirm_dialog(&mut self, ui: &mut egui::Ui, conn: &Connection) {
        let Some(env_id) = self.pending_delete_env_id else {
            return;
        };

        let env_name = self
            .environments
            .iter()
            .find(|e| e.id == env_id)
            .map(|e| e.name.clone())
            .unwrap_or_else(|| "未知环境".to_string());

        let mut open = true;
        let mut confirmed = false;

        egui::Window::new("确认删除环境")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ui.ctx(), |ui| {
                ui.label(format!(
                    "确定要删除环境 \"{}\" 吗？该环境下的所有条目也将被删除。",
                    env_name
                ));
                ui.add_space(12.0);

                ui.horizontal(|ui| {
                    if ui.button("🗑 确认删除").clicked() {
                        confirmed = true;
                    }
                    if ui.button("取消").clicked() {
                        self.pending_delete_env_id = None;
                    }
                });
            });

        if confirmed {
            self.delete_environment(conn, env_id);
            self.pending_delete_env_id = None;
        } else if !open {
            self.pending_delete_env_id = None;
        }
    }

    /// 渲染删除条目确认弹窗
    fn render_delete_entry_confirm_dialog(
        &mut self,
        ui: &mut egui::Ui,
        conn: &Connection,
        env_id: i64,
    ) {
        let Some(entry_id) = self.pending_delete_entry_id else {
            return;
        };

        let entry_info = self
            .entries
            .iter()
            .find(|e| e.id == entry_id)
            .map(|e| format!("{} → {}", e.ip_address, e.hostname))
            .unwrap_or_else(|| "未知条目".to_string());

        let mut open = true;
        let mut confirmed = false;

        egui::Window::new("确认删除条目")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ui.ctx(), |ui| {
                ui.label(format!(
                    "确定要删除条目 \"{}\" 吗？此操作不可撤销。",
                    entry_info
                ));
                ui.add_space(12.0);

                ui.horizontal(|ui| {
                    if ui.button("🗑 确认删除").clicked() {
                        confirmed = true;
                    }
                    if ui.button("取消").clicked() {
                        self.pending_delete_entry_id = None;
                    }
                });
            });

        if confirmed {
            self.delete_entry(conn, entry_id, env_id);
            self.pending_delete_entry_id = None;
        } else if !open {
            self.pending_delete_entry_id = None;
        }
    }

    /// 渲染新增环境弹窗
    fn render_add_environment_dialog(&mut self, ui: &mut egui::Ui, conn: &Connection) {
        if !self.add_env_dialog_open {
            return;
        }

        let mut open = self.add_env_dialog_open;
        let mut close_dialog = false;

        egui::Window::new("➕ 新增环境")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(300.0)
            .show(ui.ctx(), |ui| {
                ui.horizontal(|ui| {
                    ui.label("环境名称：");
                    ui.text_edit_singleline(&mut self.env_form.name);
                });

                ui.add_space(8.0);

                if ui.button("💾 保存").clicked() {
                    self.save_new_environment(conn);
                    if self.error_msg.is_none() {
                        close_dialog = true;
                    }
                }

                ui.add_space(4.0);
                self.render_messages(ui);
            });

        if close_dialog || !open {
            self.add_env_dialog_open = false;
        }
    }

    /// 保存新环境
    fn save_new_environment(&mut self, conn: &Connection) {
        self.clear_messages();

        if let Some(err) = self.env_form.validation_error() {
            self.set_error(err);
            return;
        }

        let store = HostsStore::new(conn);
        match store.add_environment(&self.env_form.name) {
            Ok(_) => {
                self.set_success("环境创建成功".to_string());
                self.load_environments(conn);
            }
            Err(e) => {
                self.set_error(format!("创建环境失败: {}", e));
            }
        }
    }

    /// 渲染编辑环境界面
    fn render_edit_environment(&mut self, ui: &mut egui::Ui, conn: &Connection, env_id: i64) {
        ui.horizontal(|ui| {
            ui.heading("✏️ 编辑环境");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("← 返回").clicked() {
                    self.state = UiState::EnvironmentList;
                    self.clear_messages();
                }
            });
        });
        ui.separator();

        ui.add_space(10.0);

        ui.horizontal(|ui| {
            ui.label("环境名称：");
            ui.text_edit_singleline(&mut self.env_form.name);
        });

        ui.add_space(8.0);

        ui.horizontal(|ui| {
            if ui.button("💾 保存修改").clicked() {
                self.save_edited_environment(conn, env_id);
            }
        });

        ui.add_space(8.0);
        self.render_messages(ui);
    }

    /// 保存编辑的环境
    fn save_edited_environment(&mut self, conn: &Connection, env_id: i64) {
        self.clear_messages();

        if let Some(err) = self.env_form.validation_error() {
            self.set_error(err);
            return;
        }

        let store = HostsStore::new(conn);
        match store.update_environment(env_id, &self.env_form.name) {
            Ok(()) => {
                self.set_success("环境名称已更新".to_string());
                self.state = UiState::EnvironmentList;
            }
            Err(e) => {
                self.set_error(format!("更新环境失败: {}", e));
            }
        }
    }

    /// 渲染条目列表界面
    fn render_entry_list(&mut self, ui: &mut egui::Ui, conn: &Connection, env_id: i64) {
        // 获取环境名称
        let env_name = self
            .environments
            .iter()
            .find(|e| e.id == env_id)
            .map(|e| e.name.clone())
            .unwrap_or_else(|| "未知环境".to_string());

        ui.horizontal(|ui| {
            ui.heading(format!("📝 {} - Hosts 条目", env_name));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("← 返回").clicked() {
                    self.state = UiState::EnvironmentList;
                    self.pending_delete_entry_id = None;
                    self.clear_messages();
                }
            });
        });
        ui.separator();

        // 工具栏
        ui.horizontal(|ui| {
            if ui.button("➕ 新增条目").clicked() {
                self.add_entry_dialog_open = true;
                self.selected_env = Some(env_id);
                self.entry_form = HostsEntryForm::new();
                self.clear_messages();
            }

            if ui.button("📥 从系统导入").clicked() {
                self.import_from_system(conn, env_id);
            }
        });

        ui.add_space(8.0);
        self.render_messages(ui);
        ui.add_space(4.0);

        // 条目列表表格
        let available_height = ui.available_height() - 40.0;
        egui::ScrollArea::vertical()
            .id_salt("hosts_entry_scroll")
            .max_height(available_height)
            .show(ui, |ui| {
                egui::Grid::new("hosts_entry_table")
                    .striped(true)
                    .num_columns(5)
                    .spacing([8.0, 4.0])
                    .show(ui, |ui| {
                        // 表头
                        ui.strong("#");
                        ui.strong("IP 地址");
                        ui.strong("主机名");
                        ui.strong("状态");
                        ui.strong("操作");
                        ui.end_row();

                        let entries = self.entries.clone();
                        if entries.is_empty() {
                            ui.label("");
                            ui.label("暂无条目");
                            ui.label("");
                            ui.label("");
                            ui.label("");
                            ui.end_row();
                        }

                        for (idx, entry) in entries.iter().enumerate() {
                            // 序号
                            ui.label(format!("{}", idx + 1));

                            // IP
                            ui.monospace(&entry.ip_address);

                            // 主机名
                            let hostname_display = if let Some(comment) = &entry.comment {
                                format!("{} # {}", entry.hostname, comment)
                            } else {
                                entry.hostname.clone()
                            };
                            ui.label(&hostname_display);

                            // 状态
                            let status_icon = if entry.is_enabled { "✅" } else { "❌" };
                            if ui.selectable_label(false, status_icon).clicked() {
                                self.toggle_entry(conn, entry.id, !entry.is_enabled);
                            }

                            // 操作
                            ui.horizontal(|ui| {
                                if ui.button("✏️").clicked() {
                                    self.state = UiState::EditEntry(env_id, entry.id);
                                    self.entry_form = HostsEntryForm::from_entry(entry);
                                    self.clear_messages();
                                }

                                if ui.button("🗑").clicked() {
                                    self.pending_delete_entry_id = Some(entry.id);
                                }
                            });

                            ui.end_row();
                        }
                    });
            });

        // 底部状态栏
        ui.separator();
        ui.horizontal(|ui| {
            let active_count = self.entries.iter().filter(|e| e.is_enabled).count();
            ui.label(format!(
                "共 {} 条记录，{} 条启用",
                self.entries.len(),
                active_count
            ));
        });

        // 弹窗渲染
        self.render_add_entry_dialog(ui, conn, env_id);
        self.render_delete_entry_confirm_dialog(ui, conn, env_id);
    }

    /// 切换条目启用状态
    fn toggle_entry(&mut self, conn: &Connection, entry_id: i64, enabled: bool) {
        let store = HostsStore::new(conn);
        match store.toggle_entry(entry_id, enabled) {
            Ok(()) => {
                if let Some(env_id) = self.selected_env {
                    self.load_entries(conn, env_id);
                }
                self.set_success(
                    if enabled {
                        "条目已启用"
                    } else {
                        "条目已禁用"
                    }
                    .to_string(),
                );
            }
            Err(e) => {
                self.set_error(format!("切换状态失败: {}", e));
            }
        }
    }

    /// 删除条目
    fn delete_entry(&mut self, conn: &Connection, entry_id: i64, env_id: i64) {
        let store = HostsStore::new(conn);
        match store.delete_entry(entry_id) {
            Ok(()) => {
                self.load_entries(conn, env_id);
                self.set_success("条目已删除".to_string());
            }
            Err(e) => {
                self.set_error(format!("删除条目失败: {}", e));
            }
        }
    }

    /// 渲染新增条目弹窗
    fn render_add_entry_dialog(&mut self, ui: &mut egui::Ui, conn: &Connection, env_id: i64) {
        if !self.add_entry_dialog_open {
            return;
        }

        let mut open = self.add_entry_dialog_open;
        let mut close_dialog = false;

        egui::Window::new("➕ 新增 Hosts 条目")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(350.0)
            .show(ui.ctx(), |ui| {
                self.render_entry_form(ui);

                ui.add_space(8.0);

                if ui.button("💾 保存").clicked() {
                    self.save_new_entry(conn, env_id);
                    if self.error_msg.is_none() {
                        close_dialog = true;
                    }
                }

                ui.add_space(4.0);
                self.render_messages(ui);
            });

        if close_dialog || !open {
            self.add_entry_dialog_open = false;
        }
    }

    /// 保存新条目
    fn save_new_entry(&mut self, conn: &Connection, env_id: i64) {
        self.clear_messages();

        let entry = match self.entry_form.to_new_entry() {
            Ok(entry) => entry,
            Err(e) => {
                self.set_error(e.to_string());
                return;
            }
        };

        let store = HostsStore::new(conn);

        match store.add_entry(env_id, &entry) {
            Ok(_) => {
                self.set_success("条目已添加".to_string());
                self.entry_form = HostsEntryForm::new();
                self.load_entries(conn, env_id);
            }
            Err(e) => {
                self.set_error(format!("添加条目失败: {}", e));
            }
        }
    }

    /// 渲染编辑条目界面
    fn render_edit_entry(
        &mut self,
        ui: &mut egui::Ui,
        conn: &Connection,
        env_id: i64,
        entry_id: i64,
    ) {
        ui.horizontal(|ui| {
            ui.heading("✏️ 编辑 Hosts 条目");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("← 返回").clicked() {
                    self.state = UiState::EntryList(env_id);
                    self.load_entries(conn, env_id);
                    self.clear_messages();
                }
            });
        });
        ui.separator();

        self.render_entry_form(ui);

        ui.add_space(8.0);

        ui.horizontal(|ui| {
            if ui.button("💾 保存修改").clicked() {
                self.save_edited_entry(conn, env_id, entry_id);
            }
        });

        ui.add_space(8.0);
        self.render_messages(ui);
    }

    /// 保存编辑的条目
    fn save_edited_entry(&mut self, conn: &Connection, env_id: i64, entry_id: i64) {
        self.clear_messages();

        let entry = match self.entry_form.to_new_entry() {
            Ok(entry) => entry,
            Err(e) => {
                self.set_error(e.to_string());
                return;
            }
        };

        let store = HostsStore::new(conn);

        match store.update_entry(entry_id, &entry.ip_address, &entry.hostname, &entry.comment) {
            Ok(()) => {
                self.set_success("条目已更新".to_string());
                self.state = UiState::EntryList(env_id);
                self.load_entries(conn, env_id);
            }
            Err(e) => {
                self.set_error(format!("更新条目失败: {}", e));
            }
        }
    }

    /// 渲染条目表单
    fn render_entry_form(&mut self, ui: &mut egui::Ui) {
        egui::Grid::new("hosts_entry_form")
            .num_columns(2)
            .spacing([8.0, 8.0])
            .show(ui, |ui| {
                ui.label("IP 地址 *");
                ui.text_edit_singleline(&mut self.entry_form.ip_address);
                ui.end_row();

                ui.label("主机名 *");
                ui.text_edit_singleline(&mut self.entry_form.hostname);
                ui.end_row();

                ui.label("备注");
                ui.text_edit_singleline(&mut self.entry_form.comment);
                ui.end_row();
            });
    }

    /// 从系统 hosts 导入条目
    fn import_from_system(&mut self, conn: &Connection, env_id: i64) {
        self.clear_messages();

        let content = match parser::read_system_hosts() {
            Ok(content) => content,
            Err(e) => {
                self.set_error(format!("读取系统 hosts 失败: {}", e));
                return;
            }
        };

        // 跳过 Tools Box 管理的区域，避免把本工具写入的条目重复导入
        let content = match parser::remove_tools_box_section(&content) {
            Ok(cleaned) => cleaned,
            Err(e) => {
                self.set_error(format!("解析系统 hosts 失败: {}", e));
                return;
            }
        };

        let store = HostsStore::new(conn);
        let existing = match store.get_entries_by_env(env_id) {
            Ok(entries) => entries,
            Err(e) => {
                self.set_error(format!("读取环境条目失败: {}", e));
                return;
            }
        };

        // 已存在的 (IP, 主机名) 不再重复导入，主机名比较不区分大小写
        let mut known: HashSet<(String, String)> = existing
            .iter()
            .map(|entry| {
                (
                    entry.ip_address.trim().to_string(),
                    entry.hostname.trim().to_ascii_lowercase(),
                )
            })
            .collect();

        let hosts_lines = parser::parse_hosts(&content);
        let mut imported = 0;
        let mut skipped = 0;
        let mut failed = 0;

        // 只导入启用状态的条目（被注释掉的条目不导入）
        for line in hosts_lines.iter().filter(|line| line.is_active) {
            let key = (
                line.ip.trim().to_string(),
                line.hostname.to_ascii_lowercase(),
            );
            if !known.insert(key) {
                skipped += 1;
                continue;
            }

            let entry = NewHostsEntry {
                ip_address: line.ip.clone(),
                hostname: line.hostname.clone(),
                comment: line.comment.clone(),
            };

            match store.add_entry(env_id, &entry) {
                Ok(_) => imported += 1,
                Err(e) => {
                    log::warn!("导入条目 {} 失败: {}", line.hostname, e);
                    failed += 1;
                }
            }
        }

        self.load_entries(conn, env_id);
        if self.error_msg.is_some() {
            return;
        }
        self.set_success(format!("已导入 {} 条记录", imported));

        let mut warnings = Vec::new();
        if skipped > 0 {
            warnings.push(format!("{} 条已存在，未重复导入", skipped));
        }
        if failed > 0 {
            warnings.push(format!("{} 条导入失败", failed));
        }
        if !warnings.is_empty() {
            self.set_warning(warnings.join("；"));
        }
    }

    /// 应用 hosts 到系统
    ///
    /// 所有已启用的环境会被合并写入同一个管理区域；
    /// 检测到主机名冲突时先弹窗让用户确认
    fn apply_hosts(&mut self, conn: &Connection) {
        self.clear_messages();
        self.pending_apply = None;

        let groups = match load_active_groups(conn) {
            Ok(groups) => groups,
            Err(e) => {
                self.set_error(format!("获取启用环境失败: {}", e));
                return;
            }
        };

        if groups.is_empty() {
            self.set_error("请先启用至少一个环境".to_string());
            return;
        }

        let env_names: Vec<String> = groups.iter().map(|group| group.name.clone()).collect();

        // 检测主机名冲突（同名主机映射到不同 IP）
        let conflicts = parser::find_hostname_conflicts(&groups);
        if !conflicts.is_empty() {
            log::warn!("主机名冲突: {}", format_conflicts(&conflicts));
            self.pending_apply = Some(PendingApply {
                groups,
                env_names,
                conflicts,
            });
            return;
        }

        self.write_hosts(&groups, &env_names, &[]);
    }

    /// 确认应用：核对配置是否在弹窗期间发生变化，避免写入过期快照
    fn confirm_apply(&mut self, conn: &Connection, pending: &PendingApply) {
        let groups = match load_active_groups(conn) {
            Ok(groups) => groups,
            Err(e) => {
                self.set_error(format!("获取启用环境失败: {}", e));
                return;
            }
        };

        if groups != pending.groups {
            log::info!("环境配置已变化，取消本次应用");
            self.set_warning("环境配置已变化，请重新点击【应用到系统】".to_string());
            return;
        }

        self.write_hosts(&pending.groups, &pending.env_names, &pending.conflicts);
    }

    /// 备份并写入系统 hosts 文件
    fn write_hosts(
        &mut self,
        groups: &[parser::HostsGroup],
        applied: &[String],
        conflicts: &[parser::HostnameConflict],
    ) {
        // 备份失败时中止写入，避免覆盖后无法恢复
        match parser::backup_hosts() {
            Ok(path) => {
                log::info!("已备份 hosts 到: {}", path.display());
            }
            Err(e) => {
                log::error!("备份 hosts 失败，已取消应用: {}", e);
                self.set_error(format!("备份系统 hosts 失败，已取消应用: {}", e));
                return;
            }
        }

        // 以追加方式更新系统 hosts
        match parser::append_to_system_hosts(groups) {
            Ok(()) => {
                let entry_count: usize = groups
                    .iter()
                    .map(|group| group.entries.iter().filter(|entry| entry.is_active).count())
                    .sum();

                log::info!(
                    "已应用 {} 个环境（{} 条启用条目）到系统 hosts",
                    applied.len(),
                    entry_count
                );

                if entry_count == 0 {
                    self.set_warning(format!(
                        "已更新 {} 个环境的管理区域，但没有启用条目（环境为空或条目已全部禁用）: {}",
                        applied.len(),
                        applied.join("、")
                    ));
                } else {
                    self.set_success(format!(
                        "已应用 {} 个环境共 {} 条启用条目到系统 hosts: {}",
                        applied.len(),
                        entry_count,
                        applied.join("、")
                    ));
                }

                if !conflicts.is_empty() {
                    // 追加到已有警告之后，避免两条提示互相覆盖
                    let conflict_warning = format!(
                        "存在主机名冲突，同名条目以 hosts 文件中先出现的映射为准: {}",
                        format_conflicts(conflicts)
                    );
                    let warning = match &self.warning_msg {
                        Some(existing) => format!("{}；{}", existing, conflict_warning),
                        None => conflict_warning,
                    };
                    self.set_warning(warning);
                }
            }
            Err(e) => {
                self.set_error(format!("写入系统 hosts 失败: {}", e));
            }
        }
    }

    /// 渲染主机名冲突确认弹窗
    fn render_apply_confirm_dialog(&mut self, ui: &mut egui::Ui, conn: &Connection) {
        let Some(pending) = self.pending_apply.take() else {
            return;
        };

        let mut open = true;
        let mut confirmed = false;
        let mut cancelled = false;

        egui::Window::new("⚠ 主机名冲突")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(420.0)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ui.ctx(), |ui| {
                ui.label("以下主机名被映射到多个不同 IP，实际生效结果取决于解析顺序：");
                ui.add_space(6.0);

                egui::ScrollArea::vertical()
                    .id_salt("hosts_conflict_scroll")
                    .max_height(160.0)
                    .show(ui, |ui| {
                        for conflict in &pending.conflicts {
                            ui.label(format!("• {}", conflict.describe()));
                        }
                    });

                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("仍然应用").clicked() {
                        confirmed = true;
                    }
                    if ui.button("取消").clicked() {
                        cancelled = true;
                    }
                });
            });

        if confirmed {
            self.confirm_apply(conn, &pending);
        } else if cancelled || !open {
            self.set_warning("已取消应用（存在主机名冲突）".to_string());
        } else {
            // 弹窗仍处于打开状态，保留待确认内容以便下一帧继续显示
            self.pending_apply = Some(pending);
        }
    }
}

/// 读取所有已启用环境及其条目
fn load_active_groups(conn: &Connection) -> Result<Vec<parser::HostsGroup>> {
    let store = HostsStore::new(conn);
    let active_envs = store.get_active_environments()?;

    let mut groups = Vec::with_capacity(active_envs.len());
    for env in &active_envs {
        let entries = store
            .get_entries_by_env(env.id)
            .with_context(|| format!("获取环境 '{}' 的条目失败", env.name))?;

        groups.push(parser::HostsGroup {
            name: env.name.clone(),
            entries: entries.iter().map(to_hosts_line).collect(),
        });
    }

    Ok(groups)
}

/// 将数据库条目转换为 hosts 文本行
fn to_hosts_line(entry: &DbHostsEntry) -> parser::HostsLine {
    parser::HostsLine {
        ip: entry.ip_address.clone(),
        hostname: entry.hostname.clone(),
        comment: entry.comment.clone(),
        is_active: entry.is_enabled,
    }
}

/// 格式化主机名冲突提示
fn format_conflicts(conflicts: &[parser::HostnameConflict]) -> String {
    conflicts
        .iter()
        .map(parser::HostnameConflict::describe)
        .collect::<Vec<String>>()
        .join("；")
}
