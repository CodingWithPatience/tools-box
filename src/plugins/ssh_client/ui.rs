use egui::{Color32, RichText};

use super::crypto;
use super::models::{AuthMethod, AuthType, NewSession, SessionForm, SessionState};
use super::store::SshStore;

/// SSH 客户端 UI 状态
pub struct SshClientUi {
    /// 会话列表
    sessions: Vec<super::models::SshSession>,
    /// 当前选中的会话索引
    selected_index: Option<usize>,
    /// 会话编辑表单
    form: SessionForm,
    /// 是否显示新增弹窗
    show_new_modal: bool,
    /// 是否显示编辑弹窗
    show_edit_modal: bool,
    /// 正在编辑的会话 ID（None 表示新增）
    editing_id: Option<i64>,
    /// 表单校验错误
    form_error: Option<String>,
    /// 连接状态
    connection_state: SessionState,
    /// 状态消息
    status_msg: String,
    /// 左侧会话列表面板宽度
    left_panel_width: f32,
    /// 上次加载错误消息（用于去重日志）
    last_load_error: String,
}

impl SshClientUi {
    pub fn new() -> Self {
        Self {
            sessions: Vec::new(),
            selected_index: None,
            form: SessionForm::new(),
            show_new_modal: false,
            show_edit_modal: false,
            editing_id: None,
            form_error: None,
            connection_state: SessionState::Disconnected,
            status_msg: "就绪".to_string(),
            left_panel_width: 180.0,
            last_load_error: String::new(),
        }
    }

    /// 刷新会话列表
    fn refresh_sessions(&mut self, store: &SshStore) {
        match store.list_sessions() {
            Ok(list) => {
                self.sessions = list;
                self.last_load_error.clear();
                // 之前有选中会话则维护选中状态
                if let Some(idx) = self.selected_index {
                    if idx >= self.sessions.len() {
                        self.selected_index = if self.sessions.is_empty() {
                            None
                        } else {
                            Some(0)
                        };
                    }
                }
            }
            Err(e) => {
                let err_msg = format!("加载会话列表失败: {}", e);
                self.status_msg = err_msg.clone();
                if self.last_load_error != err_msg {
                    log::error!("{}", err_msg);
                    self.last_load_error = err_msg;
                }
            }
        }
    }

    /// 渲染完整的 SSH 客户端 UI
    pub fn render(&mut self, ui: &mut egui::Ui, store: &SshStore) {
        // 每次渲染前刷新列表
        if self.sessions.is_empty() {
            self.refresh_sessions(store);
        }

        ui.heading("🖥 SSH 客户端");
        ui.separator();

        // 操作按钮
        self.render_toolbar(ui, store);

        ui.add_space(4.0);

        // 主布局：左侧会话列表（可拖拽宽度）+ 右侧详情
        let available_width = ui.available_width();
        let min_left_width = 120.0;
        let max_left_width = (available_width * 0.5).min(400.0);

        ui.horizontal_top(|ui| {
            // 左侧面板（固定宽度）
            let left_width = self.left_panel_width.clamp(min_left_width, max_left_width);
            ui.vertical(|ui| {
                ui.set_min_width(left_width);
                ui.set_max_width(left_width);
                self.render_session_list(ui, store);
            });

            // 可拖拽的分隔线（参考 note_taker 实现）
            let separator_rect = ui.available_rect_before_wrap();
            let separator_x = separator_rect.left();
            let separator_response = ui.allocate_rect(
                egui::Rect::from_min_max(
                    egui::pos2(separator_x, separator_rect.top()),
                    egui::pos2(separator_x + 8.0, separator_rect.bottom()),
                ),
                egui::Sense::drag(),
            );

            let line_color = if separator_response.hovered() || separator_response.dragged() {
                ui.visuals().selection.bg_fill
            } else {
                ui.visuals().widgets.noninteractive.bg_stroke.color
            };
            ui.painter().line_segment(
                [
                    egui::pos2(separator_x + 4.0, separator_rect.top()),
                    egui::pos2(separator_x + 4.0, separator_rect.bottom()),
                ],
                egui::Stroke::new(2.0, line_color),
            );

            if separator_response.dragged() {
                let delta = separator_response.drag_delta().x;
                self.left_panel_width = (self.left_panel_width + delta)
                    .clamp(min_left_width, max_left_width);
            }
            if separator_response.hovered() || separator_response.dragged() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeColumn);
            }

            ui.add_space(4.0);

            // 右侧详情面板
            ui.vertical(|ui| {
                self.render_detail_panel(ui, store);
            });
        });

        // 状态栏（底部固定，避免遮盖主内容）
        let stats_height = ui.text_style_height(&egui::TextStyle::Small) + 16.0;
        egui::TopBottomPanel::bottom("ssh_status")
            .exact_height(stats_height)
            .show_inside(ui, |ui| {
                ui.separator();
                self.render_status_bar(ui);
            });

        // 弹窗：新增/编辑会话（参考 api_tester 的 Window 用法）
        let is_editing = self.editing_id.is_some();
        let title = if is_editing { "编辑会话" } else { "新增会话" };

        if self.show_edit_modal {
            egui::Window::new(title)
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
                .min_width(380.0)
                .show(ui.ctx(), |ui| {
                    self.render_session_form_content(ui, store);
                });
        }

        if self.show_new_modal {
            egui::Window::new("新增会话")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
                .min_width(380.0)
                .show(ui.ctx(), |ui| {
                    self.render_session_form_content(ui, store);
                });
        }
    }

    /// 渲染表单内容（弹窗内部）
    fn render_session_form_content(&mut self, ui: &mut egui::Ui, store: &SshStore) {
        let label_width = 80.0;

        // 会话名称
        ui.horizontal(|ui| {
            ui.add_sized([label_width, 20.0], egui::Label::new("名称:"));
            ui.text_edit_singleline(&mut self.form.name);
        });
        ui.add_space(4.0);

        // 主机地址
        ui.horizontal(|ui| {
            ui.add_sized([label_width, 20.0], egui::Label::new("主机:"));
            ui.text_edit_singleline(&mut self.form.host);
        });
        ui.add_space(4.0);

        // 端口
        ui.horizontal(|ui| {
            ui.add_sized([label_width, 20.0], egui::Label::new("端口:"));
            ui.add_sized(
                [100.0, 20.0],
                egui::TextEdit::singleline(&mut self.form.port),
            );
            ui.weak("默认: 22");
        });
        ui.add_space(4.0);

        // 用户名
        ui.horizontal(|ui| {
            ui.add_sized([label_width, 20.0], egui::Label::new("用户:"));
            ui.text_edit_singleline(&mut self.form.username);
        });
        ui.add_space(8.0);

        ui.separator();
        ui.add_space(4.0);

        // 认证方式选择
        ui.horizontal(|ui| {
            ui.label("认证方式:");
            ui.selectable_value(
                &mut self.form.auth_type,
                AuthType::Password,
                AuthType::Password.as_str(),
            );
            ui.selectable_value(
                &mut self.form.auth_type,
                AuthType::KeyFile,
                AuthType::KeyFile.as_str(),
            );
        });
        ui.add_space(8.0);

        // 根据认证方式显示不同字段
        match self.form.auth_type {
            AuthType::Password => {
                ui.horizontal(|ui| {
                    ui.add_sized([label_width, 20.0], egui::Label::new("密码:"));
                    ui.add_sized(
                        [ui.available_width(), 20.0],
                        egui::TextEdit::singleline(&mut self.form.password).password(true),
                    );
                });
            }
            AuthType::KeyFile => {
                ui.horizontal(|ui| {
                    ui.add_sized([label_width, 20.0], egui::Label::new("密钥文件:"));
                    ui.add_sized(
                        [ui.available_width() - 60.0, 20.0],
                        egui::TextEdit::singleline(&mut self.form.private_key_path),
                    );
                    if ui.button("📂").clicked() {
                        if let Some(path) = rfd::FileDialog::new()
                            .set_title("选择 SSH 私钥文件")
                            .pick_file()
                        {
                            self.form.private_key_path =
                                path.to_string_lossy().to_string();
                        }
                    }
                });
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.add_sized([label_width, 20.0], egui::Label::new("密码短语:"));
                    ui.add_sized(
                        [ui.available_width(), 20.0],
                        egui::TextEdit::singleline(&mut self.form.passphrase).password(true),
                    );
                    ui.weak("(可选)");
                });
            }
        }

        ui.add_space(8.0);

        // 校验错误提示
        if let Some(error) = &self.form_error {
            ui.label(
                RichText::new(format!("⚠ {}", error))
                    .color(Color32::from_rgb(220, 50, 50)),
            );
            ui.add_space(4.0);
        }

        // 提交按钮
        let is_editing = self.editing_id.is_some();
        ui.horizontal(|ui| {
            let btn_label = if is_editing {
                "💾 保存修改"
            } else {
                "✅ 创建会话"
            };

            if ui.button(btn_label).clicked() {
                if let Err(e) = self.form.validate() {
                    self.form_error = Some(e);
                    return;
                }

                match self.submit_form(store) {
                    Ok(()) => {
                        self.show_new_modal = false;
                        self.show_edit_modal = false;
                        self.editing_id = None;
                        self.refresh_sessions(store);
                    }
                    Err(e) => {
                        self.form_error = Some(e);
                    }
                }
            }

            if ui.button("取消").clicked() {
                self.show_new_modal = false;
                self.show_edit_modal = false;
                self.editing_id = None;
                self.form_error = None;
            }
        });
    }

    /// 渲染顶部操作工具栏
    fn render_toolbar(&mut self, ui: &mut egui::Ui, store: &SshStore) {
        ui.horizontal(|ui| {
            if ui.button("+ 新增会话").clicked() {
                self.form = SessionForm::new();
                self.form_error = None;
                self.editing_id = None;
                self.show_new_modal = true;
            }

            if ui.button("🔄 刷新").clicked() {
                self.refresh_sessions(store);
                self.status_msg = "列表已刷新".to_string();
            }

            // 快速连接栏
            ui.separator();
            ui.label("快速连接:");
            ui.label("ssh");
            // 简化的快速连接：只是占位提示，后续阶段实现
            ui.weak("(开发中，请通过会话列表连接)");
        });
    }

    /// 渲染左侧会话列表
    fn render_session_list(&mut self, ui: &mut egui::Ui, _store: &SshStore) {
        ui.strong("会话列表:");
        ui.add_space(4.0);

        let height = ui.available_height() - 10.0;

        egui::ScrollArea::vertical()
            .max_height(height)
            .show(ui, |ui| {
                if self.sessions.is_empty() {
                    ui.weak("暂无保存的会话，点击「+ 新增会话」创建");
                    return;
                }

                let mut to_select: Option<usize> = None;

                for (idx, session) in self.sessions.iter().enumerate() {
                    let is_selected = self.selected_index == Some(idx);
                    let icon = match &self.connection_state {
                        SessionState::Connected => "🟢",
                        SessionState::Connecting => "🟡",
                        SessionState::Error(_) => "🔴",
                        SessionState::Disconnected => "⚪",
                    };
                    let label = format!(
                        "{} {} ({})",
                        icon,
                        session.name,
                        format_session_addr(session)
                    );

                    let response = ui.add_sized(
                        [ui.available_width(), 30.0],
                        egui::SelectableLabel::new(is_selected, label),
                    );

                    if response.clicked() {
                        to_select = Some(idx);
                    }
                }

                if let Some(idx) = to_select {
                    self.selected_index = Some(idx);
                }
            });
    }

    /// 渲染右侧详情面板
    fn render_detail_panel(&mut self, ui: &mut egui::Ui, store: &SshStore) {
        if let Some(idx) = self.selected_index {
            if self.sessions.get(idx).is_some() {
                self.render_session_detail(ui, idx, store);
            } else {
                ui.centered_and_justified(|ui| {
                    ui.weak("选择一个会话查看详情");
                });
            }
        } else {
            ui.centered_and_justified(|ui| {
                ui.weak("选择一个会话查看详情");
            });
        }
    }

    /// 渲染会话详情
    fn render_session_detail(&mut self, ui: &mut egui::Ui, idx: usize, store: &SshStore) {
        let Some(session) = self.sessions.get(idx) else {
            return;
        };
        // 先提取需要的数据（避免后续借用 self 冲突）
        let session_id = session.id;
        let session_name = session.name.clone();
        let session_host = session.host.clone();
        let session_port = session.port;
        let session_username = session.username.clone();
        let auth_text = match &session.auth_method {
            AuthMethod::Password { .. } => "密码",
            AuthMethod::KeyFile { .. } => "密钥文件",
        }
        .to_string();
        let key_path = match &session.auth_method {
            AuthMethod::KeyFile { private_key_path, .. } => Some(private_key_path.clone()),
            _ => None,
        };

        ui.strong("会话详情");
        ui.add_space(8.0);

        ui.label(format!("名称: {}", session_name));
        ui.label(format!("主机: {}", session_host));
        ui.label(format!("端口: {}", session_port));
        ui.label(format!("用户: {}", session_username));
        ui.label(format!("认证: {}", auth_text));
        if let Some(path) = &key_path {
            ui.label(format!("密钥文件: {}", path));
        }

        ui.add_space(12.0);

        // 操作按钮
        ui.horizontal(|ui| {
            if ui.button("🔌 连接").clicked() {
                self.status_msg = format!("正在连接 {}...", session_name);
                self.connection_state = SessionState::Connecting;
                // 后续阶段实现实际连接
            }

            if ui.button("✏️ 编辑").clicked() {
                let s = &self.sessions[idx];
                self.form = SessionForm::from_session(s);
                self.form_error = None;
                self.editing_id = Some(session_id);
                self.show_edit_modal = true;
            }

            if ui
                .button(RichText::new("🗑 删除").color(Color32::from_rgb(220, 50, 50)))
                .clicked()
            {
                match store.delete_session(session_id) {
                    Ok(()) => {
                        self.status_msg = format!("已删除会话: {}", session_name);
                        self.selected_index = None;
                        self.refresh_sessions(store);
                    }
                    Err(e) => {
                        self.status_msg = format!("删除失败: {}", e);
                        log::error!("删除 SSH 会话失败: {}", e);
                    }
                }
            }
        });
    }

    /// 提交表单（加密密码后保存）
    fn submit_form(&self, store: &SshStore) -> Result<(), String> {
        let port: u16 = self
            .form
            .port
            .trim()
            .parse()
            .map_err(|_| "端口号格式不正确".to_string())?;

        let auth_method = match self.form.auth_type {
            AuthType::Password => {
                let (encrypted_password, iv, salt) =
                    crypto::encrypt_password(&self.form.password)
                        .map_err(|e| format!("密码加密失败: {}", e))?;
                AuthMethod::Password {
                    encrypted_password,
                    iv,
                    salt,
                }
            }
            AuthType::KeyFile => {
                let encrypted_passphrase = if self.form.passphrase.is_empty() {
                    None
                } else {
                    let (ct, iv, salt) =
                        crypto::encrypt_password(&self.form.passphrase)
                            .map_err(|e| format!("密码短语加密失败: {}", e))?;
                    Some((ct, iv, salt))
                };
                AuthMethod::KeyFile {
                    private_key_path: self.form.private_key_path.clone(),
                    encrypted_passphrase,
                }
            }
        };

        let new_session = NewSession {
            name: self.form.name.trim().to_string(),
            host: self.form.host.trim().to_string(),
            port,
            username: self.form.username.trim().to_string(),
            auth_method,
        };

        if let Some(id) = self.editing_id {
            store
                .update_session(id, &new_session)
                .map_err(|e| format!("更新会话失败: {}", e))?;
            log::info!("SSH 会话 '{}' 已更新", new_session.name);
        } else {
            store
                .insert_session(&new_session)
                .map_err(|e| format!("新增会话失败: {}", e))?;
            log::info!("SSH 会话 '{}' 已创建", new_session.name);
        }

        Ok(())
    }

    /// 渲染底部状态栏
    fn render_status_bar(&self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let state_color = match &self.connection_state {
                SessionState::Connected => Color32::from_rgb(50, 200, 50),
                SessionState::Connecting => Color32::from_rgb(200, 200, 50),
                SessionState::Error(_) => Color32::from_rgb(220, 50, 50),
                SessionState::Disconnected => Color32::from_rgb(128, 128, 128),
            };
            let state_text = match &self.connection_state {
                SessionState::Connected => "已连接",
                SessionState::Connecting => "连接中...",
                SessionState::Error(e) => e,
                SessionState::Disconnected => "未连接",
            };
            ui.colored_label(state_color, state_text);
            ui.separator();
            ui.label(&self.status_msg);
        });
    }
}

/// 格式化会话地址显示
fn format_session_addr(session: &super::models::SshSession) -> String {
    if session.port == 22 {
        format!("{}@{}", session.username, session.host)
    } else {
        format!("{}@{}:{}", session.username, session.host, session.port)
    }
}
