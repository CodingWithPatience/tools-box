use rusqlite::Connection;

use super::parser;
use super::store::*;

/// UI 状态
#[derive(Debug, Clone, PartialEq)]
enum UiState {
    /// 环境列表主界面
    EnvironmentList,
    /// 新增环境
    AddEnvironment,
    /// 编辑环境名称
    EditEnvironment(i64),
    /// 条目列表
    EntryList(i64),
    /// 新增条目
    AddEntry(i64),
    /// 编辑条目
    EditEntry(i64, i64), // (env_id, entry_id)
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
        }
    }

    /// 渲染主界面
    pub fn render(&mut self, ui: &mut egui::Ui, conn: &Connection) {
        match self.state.clone() {
            UiState::EnvironmentList => self.render_environment_list(ui, conn),
            UiState::AddEnvironment => self.render_add_environment(ui, conn),
            UiState::EditEnvironment(id) => self.render_edit_environment(ui, conn, id),
            UiState::EntryList(env_id) => self.render_entry_list(ui, conn, env_id),
            UiState::AddEntry(env_id) => self.render_add_entry(ui, conn, env_id),
            UiState::EditEntry(env_id, entry_id) => {
                self.render_edit_entry(ui, conn, env_id, entry_id)
            }
        }
    }

    /// 渲染消息提示
    fn render_messages(&mut self, ui: &mut egui::Ui) {
        if let Some(err) = &self.error_msg.clone() {
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
                self.state = UiState::AddEnvironment;
                self.env_form = EnvironmentForm::new();
                self.clear_messages();
            }

            if ui.button("💾 应用到系统").clicked() {
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

                            if ui
                                .selectable_label(env.is_active, &radio_text)
                                .clicked()
                            {
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
                                    self.delete_environment(conn, env.id);
                                }
                            });

                            ui.end_row();
                        }
                    });
            });

        // 底部提示
        ui.separator();
        ui.horizontal(|ui| {
            ui.label("⚠ 应用环境需要管理员权限运行程序");
        });
    }

    /// 切换环境激活状态
    fn toggle_environment(&mut self, conn: &Connection, env_id: i64) {
        let store = HostsStore::new(conn);

        // 检查当前是否已激活
        let is_active = self
            .environments
            .iter()
            .find(|e| e.id == env_id)
            .map(|e| e.is_active)
            .unwrap_or(false);

        let new_active_id = if is_active { None } else { Some(env_id) };

        match store.set_active_environment(new_active_id) {
            Ok(()) => {
                self.load_environments(conn);
                if is_active {
                    self.set_success("已禁用环境".to_string());
                } else {
                    self.set_success("已启用环境".to_string());
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

    /// 渲染新增环境界面
    fn render_add_environment(&mut self, ui: &mut egui::Ui, conn: &Connection) {
        ui.horizontal(|ui| {
            ui.heading("➕ 新增环境");
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
            if ui.button("💾 保存").clicked() {
                self.save_new_environment(conn);
            }
        });

        ui.add_space(8.0);
        self.render_messages(ui);
    }

    /// 保存新环境
    fn save_new_environment(&mut self, conn: &Connection) {
        self.clear_messages();

        if !self.env_form.is_valid() {
            self.set_error("请输入环境名称".to_string());
            return;
        }

        let store = HostsStore::new(conn);
        match store.add_environment(&self.env_form.name) {
            Ok(_) => {
                self.set_success("环境创建成功".to_string());
                self.state = UiState::EnvironmentList;
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

        if !self.env_form.is_valid() {
            self.set_error("请输入环境名称".to_string());
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
                    self.clear_messages();
                }
            });
        });
        ui.separator();

        // 工具栏
        ui.horizontal(|ui| {
            if ui.button("➕ 新增条目").clicked() {
                self.state = UiState::AddEntry(env_id);
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
                                    self.delete_entry(conn, entry.id, env_id);
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
    }

    /// 切换条目启用状态
    fn toggle_entry(&mut self, conn: &Connection, entry_id: i64, enabled: bool) {
        let store = HostsStore::new(conn);
        match store.toggle_entry(entry_id, enabled) {
            Ok(()) => {
                if let Some(env_id) = self.selected_env {
                    self.load_entries(conn, env_id);
                }
                self.set_success(if enabled {
                    "条目已启用"
                } else {
                    "条目已禁用"
                }
                .to_string());
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

    /// 渲染新增条目界面
    fn render_add_entry(&mut self, ui: &mut egui::Ui, conn: &Connection, env_id: i64) {
        ui.horizontal(|ui| {
            ui.heading("➕ 新增 Hosts 条目");
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

        if ui.button("💾 保存").clicked() {
            self.save_new_entry(conn, env_id);
        }

        ui.add_space(8.0);
        self.render_messages(ui);
    }

    /// 保存新条目
    fn save_new_entry(&mut self, conn: &Connection, env_id: i64) {
        self.clear_messages();

        if !self.entry_form.is_valid() {
            self.set_error("请输入 IP 地址和主机名".to_string());
            return;
        }

        let store = HostsStore::new(conn);
        let entry = self.entry_form.to_new_entry();

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

        if !self.entry_form.is_valid() {
            self.set_error("请输入 IP 地址和主机名".to_string());
            return;
        }

        let store = HostsStore::new(conn);
        let comment = if self.entry_form.comment.is_empty() {
            None
        } else {
            Some(self.entry_form.comment.clone())
        };

        match store.update_entry(
            entry_id,
            &self.entry_form.ip_address,
            &self.entry_form.hostname,
            &comment,
        ) {
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

        match parser::read_system_hosts() {
            Ok(content) => {
                let hosts_lines = parser::parse_hosts(&content);
                let store = HostsStore::new(conn);

                let mut imported = 0;
                for line in &hosts_lines {
                    let entry = NewHostsEntry {
                        ip_address: line.ip.clone(),
                        hostname: line.hostname.clone(),
                        comment: line.comment.clone(),
                    };

                    if store.add_entry(env_id, &entry).is_ok() {
                        imported += 1;
                    }
                }

                self.load_entries(conn, env_id);
                self.set_success(format!("已导入 {} 条记录", imported));
            }
            Err(e) => {
                self.set_error(format!("读取系统 hosts 失败: {}", e));
            }
        }
    }

    /// 应用 hosts 到系统
    fn apply_hosts(&mut self, conn: &Connection) {
        self.clear_messages();

        // 获取当前激活的环境
        let store = HostsStore::new(conn);
        let active_env = match store.get_active_environment() {
            Ok(env) => env,
            Err(e) => {
                self.set_error(format!("获取激活环境失败: {}", e));
                return;
            }
        };

        let env = match active_env {
            Some(env) => env,
            None => {
                self.set_error("请先选择一个环境".to_string());
                return;
            }
        };

        // 获取环境条目
        let entries = match store.get_entries_by_env(env.id) {
            Ok(entries) => entries,
            Err(e) => {
                self.set_error(format!("获取环境条目失败: {}", e));
                return;
            }
        };

        // 转换为 parser 格式
        let env_entries: Vec<parser::HostsLine> = entries
            .iter()
            .map(|e| parser::HostsLine {
                ip: e.ip_address.clone(),
                hostname: e.hostname.clone(),
                comment: e.comment.clone(),
                is_active: e.is_enabled,
            })
            .collect();

        // 备份当前 hosts
        match parser::backup_hosts() {
            Ok(path) => {
                log::info!("已备份 hosts 到: {}", path.display());
            }
            Err(e) => {
                log::warn!("备份 hosts 失败: {}", e);
            }
        }

        // 以追加方式更新系统 hosts
        match parser::append_to_system_hosts(&env_entries) {
            Ok(()) => {
                self.set_success(format!("已应用环境 '{}' 到系统 hosts", env.name));
            }
            Err(e) => {
                self.set_error(format!("写入系统 hosts 失败（需要管理员权限）: {}", e));
            }
        }
    }
}
