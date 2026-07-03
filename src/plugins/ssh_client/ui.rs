use std::sync::mpsc;

use egui::{Color32, RichText};

use super::client::SshClient;
use super::crypto;
use super::models::{AuthMethod, AuthType, NewSession, SessionForm, SessionState, SshInput, SshOutput};
use super::store::SshStore;
use super::terminal::TerminalEmulator;

/// SSH 客户端 UI 状态
pub struct SshClientUi {
    // ===== 会话管理 =====
    sessions: Vec<super::models::SshSession>,
    selected_index: Option<usize>,
    form: SessionForm,
    show_new_modal: bool,
    show_edit_modal: bool,
    editing_id: Option<i64>,
    form_error: Option<String>,
    connection_state: SessionState,
    status_msg: String,
    left_panel_width: f32,
    last_load_error: String,

    // ===== 终端相关 =====
    /// 终端仿真器
    terminal: Option<TerminalEmulator>,
    /// 发送消息到 SSH I/O 线程（有界通道）
    input_tx: Option<mpsc::SyncSender<SshInput>>,
    /// 从 SSH I/O 线程接收消息
    output_rx: Option<mpsc::Receiver<SshOutput>>,
    /// 当前连接的会话索引
    connected_session_idx: Option<usize>,
    /// IME（输入法）是否激活，用于避免 Event::Text 和 Event::Ime::Commit 重复触发
    ime_active: bool,
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
            terminal: None,
            input_tx: None,
            output_rx: None,
            connected_session_idx: None,
            ime_active: false,
        }
    }

    /// 是否正在终端视图中
    fn is_terminal_view(&self) -> bool {
        self.terminal.is_some() && self.connected_session_idx.is_some()
    }

    /// 刷新会话列表
    fn refresh_sessions(&mut self, store: &SshStore) {
        match store.list_sessions() {
            Ok(list) => {
                self.sessions = list;
                self.last_load_error.clear();
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
                let err_msg = format!("加载连接列表失败: {}", e);
                self.status_msg = err_msg.clone();
                if self.last_load_error != err_msg {
                    log::error!("{}", err_msg);
                    self.last_load_error = err_msg;
                }
            }
        }
    }

    /// 断开当前 SSH 连接
    fn disconnect(&mut self) {
        if let Some(tx) = &self.input_tx {
            let _ = tx.send(SshInput::Disconnect);
        }
        self.cleanup_connection();
    }

    /// 清理连接状态
    fn cleanup_connection(&mut self) {
        self.terminal = None;
        self.input_tx = None;
        self.output_rx = None;
        self.connected_session_idx = None;
        self.connection_state = SessionState::Disconnected;
        self.status_msg = "已断开连接".to_string();
    }

    // ===================================================================
    // 主渲染入口
    // ===================================================================

    /// 渲染完整的 SSH 客户端 UI
    pub fn render(&mut self, ui: &mut egui::Ui, store: &SshStore) {
        if self.sessions.is_empty() {
            self.refresh_sessions(store);
        }

        // 轮询 SSH 输出（在终端视图中每帧处理）
        self.poll_ssh_output();

        if self.is_terminal_view() {
            self.render_terminal_view(ui);
        } else {
            self.render_management_view(ui, store);
        }
    }

    // ===================================================================
    // SSH 输出轮询
    // ===================================================================

    /// 每帧非阻塞读取 SSH 输出，喂入终端解析器
    fn poll_ssh_output(&mut self) {
        let rx = match &self.output_rx {
            Some(rx) => rx,
            None => return,
        };

        loop {
            match rx.try_recv() {
                Ok(SshOutput::TerminalData(data)) => {
                    if let Some(term) = &mut self.terminal {
                        term.process(&data);
                    }
                }
                Ok(SshOutput::Connected) => {
                    self.connection_state = SessionState::Connected;
                    self.status_msg = "已连接".to_string();
                    log::info!("SSH 连接就绪");
                }
                Ok(SshOutput::Disconnected(reason)) => {
                    self.status_msg = format!("连接断开: {}", reason);
                    log::info!("SSH 连接断开: {}", reason);
                    self.connection_state = SessionState::Error(reason.clone());
                    self.cleanup_connection();
                    return;
                }
                Ok(SshOutput::Error(err)) => {
                    self.status_msg = format!("连接错误: {}", err);
                    self.connection_state = SessionState::Error(err.clone());
                    log::error!("SSH 错误: {}", err);
                    self.cleanup_connection();
                    return;
                }
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.status_msg = "SSH 连接已断开".to_string();
                    self.cleanup_connection();
                    return;
                }
            }
        }
    }

    // ===================================================================
    // 终端视图
    // ===================================================================

    fn render_terminal_view(&mut self, ui: &mut egui::Ui) {
        // 强制每帧重绘，确保光标闪烁和终端内容及时更新
        ui.ctx().request_repaint();

        // 提前处理键盘输入，防止被其他 widget 消费
        let tx_clone = self.input_tx.clone();
        if let Some(tx) = &tx_clone {
            self.process_terminal_input(tx, ui.ctx());
        }

        // 顶部连接信息栏（提前提取数据避免借用冲突）
        let session_info = self
            .connected_session_idx
            .and_then(|idx| self.sessions.get(idx))
            .map(|s| (s.name.clone(), s.username.clone(), s.host.clone(), s.port));

        ui.horizontal(|ui| {
            if let Some((ref name, ref username, ref host, port)) = session_info {
                ui.heading(format!(
                    "🖥 {} ({}@{}:{})",
                    name, username, host, port
                ));
            }
            let state_color = match self.connection_state {
                SessionState::Connected => Color32::from_rgb(50, 200, 50),
                SessionState::Connecting => Color32::from_rgb(200, 200, 50),
                _ => Color32::from_rgb(220, 50, 50),
            };
            let state_text = match self.connection_state {
                SessionState::Connected => "🟢 已连接",
                SessionState::Connecting => "🟡 连接中...",
                _ => "🔴 异常",
            };
            ui.colored_label(state_color, state_text);

            if ui.button("🔌 断开").clicked() {
                self.disconnect();
                return;
            }
        });
        ui.separator();

        // 获取终端尺寸信息
        let font_size = self
            .terminal
            .as_ref()
            .map(|t| t.font_size())
            .unwrap_or(14.0);
        let is_dark_mode = ui.visuals().dark_mode;

        // 计算终端渲染区域
        let header_height = 40.0;
        let status_height = 28.0;
        let available_height = ui.available_height() - status_height - header_height;

        // 使用精确的字体度量
        let mono_font = egui::FontId::monospace(font_size);
        let char_width = ui.fonts(|f| {
            let glyph = f.glyph_width(&mono_font, 'M');
            if glyph > 0.0 { glyph } else { font_size * 0.6 }
        });
        let line_height = ui.fonts(|f| f.row_height(&mono_font));

        let new_cols = ((ui.available_width() - 20.0) / char_width)
            .max(1.0)
            .min(f32::from(u16::MAX)) as u16;
        let new_rows = (available_height / line_height)
            .max(1.0)
            .min(f32::from(u16::MAX)) as u16;

        // 调整终端大小
        if let Some(term) = &mut self.terminal {
            let (cur_cols, cur_rows) = term.size();
            if new_cols != cur_cols || new_rows != cur_rows {
                term.resize(new_cols, new_rows);
                if let Some(tx) = &self.input_tx {
                    let _ = tx.send(SshInput::Resize(new_cols, new_rows));
                }
            }
        }

        // 渲染终端内容
        egui::ScrollArea::both()
            .id_salt("ssh_terminal_scroll")
            .show(ui, |ui| {
                // 设置终端背景色
                let bg_color = if is_dark_mode {
                    Color32::from_rgb(0x1e, 0x1e, 0x1e)
                } else {
                    Color32::from_rgb(0xff, 0xff, 0xff)
                };
                ui.painter()
                    .rect_filled(ui.max_rect(), 0.0, bg_color);

                if let Some(term) = &mut self.terminal {
                    let job = term.render_to_layout_job(is_dark_mode);
                    // 设置终端内容最小尺寸，确保填满可用区域
                    let content_w = f32::from(new_cols) * char_width + 20.0;
                    let content_h = f32::from(new_rows) * line_height;
                    ui.set_min_width(content_w);
                    ui.set_min_height(content_h);

                    // 使用 ui.label 渲染文本
                    let response = ui.label(job);
                    // 从 response.rect 获取文本区域的精确位置
                    let text_rect = response.rect;

                    // 绘制闪烁光标
                    let (c_col, c_row) = term.cursor_position();
                    let cursor_w = term.cursor_char_width();
                    let time = ui.ctx().input(|i| i.time);
                    let blink_on = (time * 2.0) as u64 % 2 == 0;
                    // 计算光标位置（用于绘制和 IME 定位）
                    let cursor_x = text_rect.left() + f32::from(c_col) * char_width.round();
                    let cursor_y = text_rect.top() + f32::from(c_row) * line_height.round();
                    let cursor_width = char_width * f32::from(cursor_w);
                    if blink_on {
                        let cursor_color = if is_dark_mode {
                            Color32::from_rgb(0xd0, 0xd0, 0xd0)
                        } else {
                            Color32::from_rgb(0x30, 0x30, 0x30)
                        };
                        ui.painter().rect_filled(
                            egui::Rect::from_min_size(
                                egui::pos2(cursor_x, cursor_y),
                                egui::vec2(cursor_width, line_height),
                            ),
                            0.0,
                            cursor_color,
                        );
                    }

                    // 设置 IME 输出位置，使输入法候选窗口跟随光标
                    let cursor_rect = egui::Rect::from_min_size(
                        egui::pos2(cursor_x, cursor_y),
                        egui::vec2(cursor_width, line_height),
                    );
                    let to_global = ui
                        .ctx()
                        .layer_transform_to_global(ui.layer_id())
                        .unwrap_or_default();
                    ui.ctx().output_mut(|o| {
                        o.ime = Some(egui::output::IMEOutput {
                            rect: to_global * text_rect,
                            cursor_rect: to_global * cursor_rect,
                        });
                    });
                } else if self.connection_state == SessionState::Connecting {
                    ui.centered_and_justified(|ui| {
                        ui.label("正在连接...");
                    });
                }
            });

        // 底部状态栏
        let cursor_pos = self
            .terminal
            .as_ref()
            .map(|t| t.cursor_position())
            .unwrap_or((0, 0));
        ui.separator();
        ui.horizontal(|ui| {
            ui.label(format!(
                "列: {} 行: {} | 字号: {:.0} | {}",
                cursor_pos.0,
                cursor_pos.1,
                font_size,
                self.status_msg
            ));
        });

    }

    /// 处理终端键盘输入，将键盘事件转换为 SSH 字节流并发送到 I/O 线程
    ///
    /// 本方法在 egui widget 渲染之前调用，确保事件不被其他组件消费。
    /// 终端视图独占键盘输入，因此消费所有事件后清空队列。
    /// 完全依赖服务器回显，不进行本地回显，避免双重回显问题。
    fn process_terminal_input(&mut self, tx: &mpsc::SyncSender<SshInput>, ctx: &egui::Context) {
        // 使用 input_mut 读取并消费事件，防止事件传播到其他 UI 组件
        ctx.input_mut(|i| {
            for event in i.events.clone() {
                match event {
                    egui::Event::Key {
                        key,
                        pressed: true,
                        modifiers,
                        ..
                    } => {
                        // IME 激活时跳过大部分键盘事件，将控制权交给 IME 系统
                        // 仅保留 Ctrl 组合快捷键（如 Ctrl+C）以便用户中断命令
                        if self.ime_active && !modifiers.ctrl {
                            continue;
                        }
                        if key == egui::Key::C && modifiers.ctrl {
                            let _ = tx.send(SshInput::KeyInput(vec![0x03]));
                            i.consume_key(modifiers, key);
                            continue;
                        }
                        if key == egui::Key::D && modifiers.ctrl {
                            let _ = tx.send(SshInput::KeyInput(vec![0x04]));
                            i.consume_key(modifiers, key);
                            continue;
                        }
                        if key == egui::Key::Z && modifiers.ctrl {
                            let _ = tx.send(SshInput::KeyInput(vec![0x1a]));
                            i.consume_key(modifiers, key);
                            continue;
                        }
                        if key == egui::Key::L && modifiers.ctrl {
                            let _ = tx.send(SshInput::KeyInput(vec![0x0c]));
                            i.consume_key(modifiers, key);
                            continue;
                        }
                        if key == egui::Key::Enter {
                            let _ = tx.send(SshInput::KeyInput(vec![0x0d]));
                            i.consume_key(modifiers, key);
                            continue;
                        }
                        if key == egui::Key::Backspace {
                            let _ = tx.send(SshInput::KeyInput(vec![0x7f]));
                            i.consume_key(modifiers, key);
                            continue;
                        }
                        if key == egui::Key::Tab {
                            let _ = tx.send(SshInput::KeyInput(vec![0x09]));
                            i.consume_key(modifiers, key);
                            continue;
                        }
                        if key == egui::Key::Escape {
                            let _ = tx.send(SshInput::KeyInput(vec![0x1b]));
                            i.consume_key(modifiers, key);
                            continue;
                        }
                        // 方向键
                        if key == egui::Key::ArrowUp {
                            let _ = tx.send(SshInput::KeyInput(b"\x1b[A".to_vec()));
                            i.consume_key(modifiers, key);
                            continue;
                        }
                        if key == egui::Key::ArrowDown {
                            let _ = tx.send(SshInput::KeyInput(b"\x1b[B".to_vec()));
                            i.consume_key(modifiers, key);
                            continue;
                        }
                        if key == egui::Key::ArrowRight {
                            let _ = tx.send(SshInput::KeyInput(b"\x1b[C".to_vec()));
                            i.consume_key(modifiers, key);
                            continue;
                        }
                        if key == egui::Key::ArrowLeft {
                            let _ = tx.send(SshInput::KeyInput(b"\x1b[D".to_vec()));
                            i.consume_key(modifiers, key);
                            continue;
                        }
                    }
                    egui::Event::Text(text) => {
                        // IME 激活时跳过，由 ImeEvent::Commit 处理，避免重复输入
                        if self.ime_active {
                            continue;
                        }
                        // 过滤掉控制字符事件
                        if text.is_empty()
                            || text == "\r"
                            || text == "\x08"
                            || text == "\t"
                        {
                            continue;
                        }
                        let _ = tx.send(SshInput::KeyInput(text.as_bytes().to_vec()));
                    }
                    // 处理 IME（输入法）事件，支持中文等非 ASCII 字符输入
                    egui::Event::Ime(ime_event) => {
                        match ime_event {
                            egui::ImeEvent::Enabled => {
                                self.ime_active = true;
                            }
                            egui::ImeEvent::Disabled => {
                                self.ime_active = false;
                            }
                            egui::ImeEvent::Commit(text) => {
                                // IME 组合完成，提交最终文本
                                if !text.is_empty() {
                                    let _ = tx.send(SshInput::KeyInput(text.as_bytes().to_vec()));
                                }
                            }
                            // Preedit 事件（组合中）不需要处理，等待 Commit
                            egui::ImeEvent::Preedit(_) => {}
                        }
                    }
                    _ => {}
                }
            }
            // 清空所有事件，防止传播到其他 UI 组件
            i.events.clear();
        });

        // 设置焦点锁定过滤器，阻止 Tab/Enter 等按键传播到其他组件
        let terminal_id = egui::Id::new("ssh_terminal_input");
        ctx.memory_mut(|mem| {
            mem.request_focus(terminal_id);
            mem.set_focus_lock_filter(
                terminal_id,
                egui::EventFilter {
                    tab: true,
                    horizontal_arrows: true,
                    vertical_arrows: true,
                    escape: true,
                },
            );
        });
    }

    // ===================================================================
    // 管理视图（原有代码）
    // ===================================================================

    fn render_management_view(&mut self, ui: &mut egui::Ui, store: &SshStore) {
        ui.heading("🖥 SSH 客户端");
        ui.separator();

        self.render_toolbar(ui, store);

        ui.add_space(4.0);

        // 主布局：左侧会话列表（可拖拽宽度）+ 右侧详情
        let available_width = ui.available_width();
        let min_left_width = 120.0;
        let max_left_width = (available_width * 0.5).min(400.0);

        ui.horizontal_top(|ui| {
            let left_width = self.left_panel_width.clamp(min_left_width, max_left_width);
            ui.vertical(|ui| {
                ui.set_min_width(left_width);
                ui.set_max_width(left_width);
                self.render_session_list(ui, store);
            });

            // 可拖拽分隔线
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

            ui.vertical(|ui| {
                self.render_detail_panel(ui, store);
            });
        });

        // 状态栏
        let stats_height = ui.text_style_height(&egui::TextStyle::Small) + 16.0;
        egui::TopBottomPanel::bottom("ssh_status")
            .exact_height(stats_height)
            .show_inside(ui, |ui| {
                ui.separator();
                self.render_status_bar(ui);
            });

        // 弹窗
        let title = if self.editing_id.is_some() { "编辑连接" } else { "新增连接" };

        if self.show_edit_modal {
            egui::Window::new(title)
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
                .fixed_size(egui::vec2(440.0, f32::INFINITY))
                .show(ui.ctx(), |ui| {
                    self.render_session_form_content(ui, store);
                });
        }

        if self.show_new_modal {
            egui::Window::new("新增连接")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
                .fixed_size(egui::vec2(440.0, f32::INFINITY))
                .show(ui.ctx(), |ui| {
                    self.render_session_form_content(ui, store);
                });
        }
    }

    /// 渲染表单内容（弹窗内部）
    fn render_session_form_content(&mut self, ui: &mut egui::Ui, store: &SshStore) {
        let label_width = 80.0;

        ui.horizontal(|ui| {
            ui.add_sized([label_width, 20.0], egui::Label::new("名称:"));
            ui.text_edit_singleline(&mut self.form.name);
        });
        ui.add_space(4.0);

        ui.horizontal(|ui| {
            ui.add_sized([label_width, 20.0], egui::Label::new("主机:"));
            ui.text_edit_singleline(&mut self.form.host);
        });
        ui.add_space(4.0);

        ui.horizontal(|ui| {
            ui.add_sized([label_width, 20.0], egui::Label::new("端口:"));
            ui.add_sized([100.0, 20.0], egui::TextEdit::singleline(&mut self.form.port));
            ui.weak("默认: 22");
        });
        ui.add_space(4.0);

        ui.horizontal(|ui| {
            ui.add_sized([label_width, 20.0], egui::Label::new("用户:"));
            ui.text_edit_singleline(&mut self.form.username);
        });
        ui.add_space(8.0);

        ui.separator();
        ui.add_space(4.0);

        ui.horizontal(|ui| {
            ui.label("认证方式:");
            ui.selectable_value(&mut self.form.auth_type, AuthType::Password, AuthType::Password.as_str());
            ui.selectable_value(&mut self.form.auth_type, AuthType::KeyFile, AuthType::KeyFile.as_str());
        });
        ui.add_space(8.0);

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
                            self.form.private_key_path = path.to_string_lossy().to_string();
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

        if let Some(error) = &self.form_error {
            ui.label(RichText::new(format!("⚠ {}", error)).color(Color32::from_rgb(220, 50, 50)));
            ui.add_space(4.0);
        }

        let is_editing = self.editing_id.is_some();
        ui.horizontal(|ui| {
            let btn_label = if is_editing { "💾 保存修改" } else { "✅ 保存连接" };

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

    fn render_toolbar(&mut self, ui: &mut egui::Ui, store: &SshStore) {
        ui.horizontal(|ui| {
            if ui.button("+ 新增连接").clicked() {
                self.form = SessionForm::new();
                self.form_error = None;
                self.editing_id = None;
                self.show_new_modal = true;
            }

            if ui.button("🔄 刷新").clicked() {
                self.refresh_sessions(store);
                self.status_msg = "列表已刷新".to_string();
            }

            ui.separator();
            ui.label("快速连接:");
            ui.label("ssh");
            ui.weak("(开发中，请通过会话列表连接)");
        });
    }

    fn render_session_list(&mut self, ui: &mut egui::Ui, _store: &SshStore) {
        ui.strong("连接列表:");
        ui.add_space(4.0);

        let height = ui.available_height() - 10.0;

        egui::ScrollArea::vertical()
            .max_height(height)
            .show(ui, |ui| {
                if self.sessions.is_empty() {
                    ui.weak("暂无保存的连接，点击「+ 新增连接」创建");
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
                    let label = format!("{} {} ({})", icon, session.name, format_session_addr(session));

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

    fn render_detail_panel(&mut self, ui: &mut egui::Ui, store: &SshStore) {
        if let Some(idx) = self.selected_index {
            if self.sessions.get(idx).is_some() {
                self.render_session_detail(ui, idx, store);
            } else {
                ui.centered_and_justified(|ui| {
                    ui.weak("选择一个连接查看详情");
                });
            }
        } else {
            ui.centered_and_justified(|ui| {
                ui.weak("选择一个连接查看详情");
            });
        }
    }

    fn render_session_detail(&mut self, ui: &mut egui::Ui, idx: usize, store: &SshStore) {
        let Some(session) = self.sessions.get(idx) else {
            return;
        };
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

        ui.strong("连接详情");
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

        ui.horizontal(|ui| {
            if ui.button("🖥 创建会话").clicked() {
                // 发起 SSH 连接
                let s = &self.sessions[idx];
                let host = s.host.clone();
                let port = s.port;
                let username = s.username.clone();
                let auth = s.auth_method.clone();

                self.status_msg = format!("正在连接 {}...", session_name);
                self.connection_state = SessionState::Connecting;

                // 根据字体大小和可用宽度估算初始终端尺寸
                let mono_font = egui::FontId::monospace(
                    ui.style().text_styles.get(&egui::TextStyle::Monospace).map(|f| f.size).unwrap_or(14.0),
                );
                let est_char_w = ui.fonts(|f| f.glyph_width(&mono_font, 'M'));
                let est_line_h = ui.fonts(|f| f.row_height(&mono_font));
                let est_cols = if est_char_w > 0.0 {
                    ((ui.available_width() * 0.7 / est_char_w).max(80.0).min(f32::from(u16::MAX))) as u16
                } else {
                    120
                };
                let est_rows = if est_line_h > 0.0 {
                    ((ui.available_height() * 0.5 / est_line_h).max(24.0).min(f32::from(u16::MAX))) as u16
                } else {
                    30
                };

                match SshClient::connect(&host, port, &username, &auth, est_cols, est_rows) {
                    Ok((tx, rx)) => {
                        self.input_tx = Some(tx);
                        self.output_rx = Some(rx);
                        self.connected_session_idx = Some(idx);
                        self.terminal = Some(TerminalEmulator::new(
                            est_cols,
                            est_rows,
                            ui.style()
                                .text_styles
                                .get(&egui::TextStyle::Monospace)
                                .map(|f| f.size)
                                .unwrap_or(14.0),
                        ));
                        log::info!("SSH 连接已发起: {}@{}", username, host);
                    }
                    Err(e) => {
                        self.connection_state =
                            SessionState::Error(format!("连接失败: {}", e));
                        self.status_msg = format!("连接失败: {}", e);
                        log::error!("SSH 连接失败: {}", e);
                    }
                }
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
                        self.status_msg = format!("已删除连接: {}", session_name);
                        self.selected_index = None;
                        self.refresh_sessions(store);
                    }
                    Err(e) => {
                        self.status_msg = format!("删除失败: {}", e);
                        log::error!("删除 SSH 连接失败: {}", e);
                    }
                }
            }
        });
    }

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
                AuthMethod::Password { encrypted_password, iv, salt }
            }
            AuthType::KeyFile => {
                let encrypted_passphrase = if self.form.passphrase.is_empty() {
                    None
                } else {
                    let (ct, iv, salt) = crypto::encrypt_password(&self.form.passphrase)
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
            store.update_session(id, &new_session).map_err(|e| format!("更新连接失败: {}", e))?;
            log::info!("SSH 连接 '{}' 已更新", new_session.name);
        } else {
            store.insert_session(&new_session).map_err(|e| format!("保存连接失败: {}", e))?;
            log::info!("SSH 连接 '{}' 已创建", new_session.name);
        }

        Ok(())
    }

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

fn format_session_addr(session: &super::models::SshSession) -> String {
    if session.port == 22 {
        format!("{}@{}", session.username, session.host)
    } else {
        format!("{}@{}:{}", session.username, session.host, session.port)
    }
}
