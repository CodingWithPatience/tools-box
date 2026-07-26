use std::path::Path;
use std::sync::mpsc;

use egui::{Color32, RichText};

use super::client::SshClient;
use super::crypto;
use super::models::{
    AuthMethod, AuthType, NewSession, SessionForm, SessionState, SessionTab, SessionViewTab,
    SftpRequest, SftpResponse, SshInput, SshOutput,
};
use super::sftp::{self, SftpClient};
use super::store::SshStore;
use super::terminal::TerminalEmulator;

/// SSH 客户端 UI 状态
pub struct SshClientUi {
    // ===== 会话配置管理 =====
    sessions: Vec<super::models::SshSession>,
    selected_index: Option<usize>,
    form: SessionForm,
    show_new_modal: bool,
    show_edit_modal: bool,
    editing_id: Option<i64>,
    form_error: Option<String>,
    left_panel_width: f32,
    last_load_error: String,

    // ===== 多标签会话 =====
    /// 打开的会话标签列表
    tabs: Vec<SessionTab>,
    /// 当前活动标签索引
    active_tab_index: Option<usize>,
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
            left_panel_width: 250.0,
            last_load_error: String::new(),
            tabs: Vec::new(),
            active_tab_index: None,
        }
    }

    /// 是否在会话视图中（有活动标签即显示会话视图）
    fn is_terminal_view(&self) -> bool {
        self.active_tab_index.is_some() && !self.tabs.is_empty()
    }

    /// 获取当前活动标签的不可变引用
    fn current_tab(&self) -> Option<&SessionTab> {
        self.active_tab_index.and_then(|idx| self.tabs.get(idx))
    }

    /// 获取当前活动标签的可变引用
    fn current_tab_mut(&mut self) -> Option<&mut SessionTab> {
        self.active_tab_index.and_then(|idx| self.tabs.get_mut(idx))
    }

    /// 关闭指定索引的标签
    fn close_tab(&mut self, index: usize) {
        if index < self.tabs.len() {
            // 断开该标签的所有连接
            self.tabs[index].disconnect_all();
            // 移除标签
            self.tabs.remove(index);
            // 调整活动标签索引
            if self.tabs.is_empty() {
                self.active_tab_index = None;
            } else if let Some(active) = self.active_tab_index {
                if active >= self.tabs.len() {
                    self.active_tab_index = Some(self.tabs.len() - 1);
                } else if active > index {
                    self.active_tab_index = Some(active - 1);
                }
            }
        }
    }

    /// 重新连接终端
    fn reconnect_terminal(&mut self, ui: &mut egui::Ui) {
        let Some(tab) = self.current_tab_mut() else {
            return;
        };

        let session_id = tab.session_id;
        let session = self.sessions.iter().find(|s| s.id == session_id);
        let Some(session) = session else {
            return;
        };

        let host = session.host.clone();
        let port = session.port;
        let username = session.username.clone();
        let auth = session.auth_method.clone();

        let mono_font = egui::FontId::monospace(
            ui.style()
                .text_styles
                .get(&egui::TextStyle::Monospace)
                .map(|f| f.size)
                .unwrap_or(14.0),
        );
        let est_char_w = ui.fonts(|f| f.glyph_width(&mono_font, 'M'));
        let est_line_h = ui.fonts(|f| f.row_height(&mono_font));
        let est_cols = if est_char_w > 0.0 {
            ((ui.available_width() * 0.7 / est_char_w)
                .max(80.0)
                .min(f32::from(u16::MAX))) as u16
        } else {
            120
        };
        let est_rows = if est_line_h > 0.0 {
            ((ui.available_height() * 0.5 / est_line_h)
                .max(24.0)
                .min(f32::from(u16::MAX))) as u16
        } else {
            30
        };

        match SshClient::connect(&host, port, &username, &auth, est_cols, est_rows) {
            Ok((tx, rx)) => {
                let Some(tab) = self.current_tab_mut() else {
                    return;
                };
                tab.input_tx = Some(tx);
                tab.output_rx = Some(rx);
                tab.terminal = Some(TerminalEmulator::new(
                    est_cols,
                    est_rows,
                    ui.style()
                        .text_styles
                        .get(&egui::TextStyle::Monospace)
                        .map(|f| f.size)
                        .unwrap_or(14.0),
                ));
                tab.connection_state = SessionState::Connecting;
                tab.status_msg = "正在重新连接...".to_string();
                tab.active_tab = SessionViewTab::Terminal;
                log::info!("SSH 重新连接已发起: {}@{}", username, host);
            }
            Err(e) => {
                let Some(tab) = self.current_tab_mut() else {
                    return;
                };
                tab.connection_state = SessionState::Error(format!("重连失败: {}", e));
                tab.status_msg = format!("重连失败: {}", e);
                log::error!("SSH 重新连接失败: {}", e);
            }
        }
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
                if self.last_load_error != err_msg {
                    log::error!("{}", err_msg);
                    self.last_load_error = err_msg;
                }
            }
        }
    }

    /// 刷新本地文件列表
    fn refresh_local_files(&mut self) {
        let Some(tab) = self.current_tab_mut() else {
            return;
        };
        match sftp::list_local_dir(&tab.local_current_dir) {
            Ok(files) => {
                tab.local_files = files;
                tab.local_selected = None;
                tab.local_dir_input = tab.local_current_dir.clone();
            }
            Err(e) => {
                tab.sftp_status_msg = format!("无法读取本地目录: {}", e);
                log::warn!("读取本地目录失败: {}", e);
            }
        }
    }

    /// 请求刷新远程文件列表
    fn refresh_remote_files(&mut self) {
        let Some(tab) = self.current_tab_mut() else {
            return;
        };
        if let Some(tx) = &tab.sftp_tx {
            let _ = tx.send(SftpRequest::ListDirectory(tab.remote_current_dir.clone()));
        }
    }

    /// 初始化 SFTP 连接
    fn init_sftp_connection(&mut self) {
        let Some(tab) = self.current_tab_mut() else {
            return;
        };
        if tab.sftp_tx.is_some() {
            return; // 已连接
        }
        let session_id = tab.session_id;
        let Some(session) = self.sessions.iter().find(|s| s.id == session_id) else {
            return;
        };

        let host = session.host.clone();
        let port = session.port;
        let username = session.username.clone();
        let auth = session.auth_method.clone();

        match SftpClient::connect(&host, port, &username, &auth) {
            Ok((tx, rx)) => {
                let Some(tab) = self.current_tab_mut() else {
                    return;
                };
                tab.sftp_tx = Some(tx);
                tab.sftp_rx = Some(rx);
                log::info!("SFTP 连接已发起");
            }
            Err(e) => {
                let Some(tab) = self.current_tab_mut() else {
                    return;
                };
                tab.sftp_status_msg = format!("SFTP 连接失败: {}", e);
                log::error!("SFTP 连接失败: {}", e);
            }
        }
    }

    // ===================================================================
    // 主渲染入口
    // ===================================================================

    pub fn render(&mut self, ui: &mut egui::Ui, store: &SshStore) {
        if self.sessions.is_empty() {
            self.refresh_sessions(store);
        }

        self.poll_ssh_output();
        self.poll_sftp_output();

        if self.is_terminal_view() {
            // 获取全局字体大小
            let global_font_size = ui
                .style()
                .text_styles
                .get(&egui::TextStyle::Monospace)
                .map(|f| f.size)
                .unwrap_or(14.0);

            // 在渲染前处理终端键盘输入，避免与 disconnect 按钮的借用冲突
            let active_tab_is_terminal = self
                .current_tab()
                .map(|t| t.active_tab == SessionViewTab::Terminal)
                .unwrap_or(false);

            if active_tab_is_terminal {
                let had_input = if let Some(tab) = self.current_tab() {
                    if let Some(tx) = &tab.input_tx {
                        let tx_clone = tx.clone();
                        self.process_terminal_input(&tx_clone, ui.ctx())
                    } else {
                        false
                    }
                } else {
                    false
                };

                // 处理滚轮事件
                let (ctrl_held, scroll_y) =
                    ui.ctx().input(|i| (i.modifiers.ctrl, i.raw_scroll_delta.y));
                // 使用 signum 确保各平台步进一致，避免触控板/滚轮差异
                if ctrl_held && scroll_y != 0.0 {
                    // Ctrl+滚轮调整终端字体大小
                    if let Some(tab) = self.current_tab_mut() {
                        if let Some(term) = &mut tab.terminal {
                            let delta = scroll_y.signum();
                            let current_size = tab.custom_font_size.unwrap_or(global_font_size);
                            let new_size = (current_size + delta).clamp(8.0, 36.0);
                            tab.custom_font_size = Some(new_size);
                            term.set_font_size(new_size);
                        }
                    }
                } else if scroll_y != 0.0 {
                    // 滚轮滚动终端历史
                    if let Some(tab) = self.current_tab_mut() {
                        if let Some(term) = &mut tab.terminal {
                            let scrollback = term.scrollback();
                            let scrollback_size = term.scrollback_size();
                            if scroll_y > 0.0 && scrollback < scrollback_size {
                                // 向上滚动
                                term.set_scrollback(scrollback + 1);
                            } else if scroll_y < 0.0 && scrollback > 0 {
                                // 向下滚动
                                term.set_scrollback(scrollback - 1);
                            }
                        }
                    }
                } else if had_input {
                    // 有键盘输入时，自动滚动到光标位置
                    if let Some(tab) = self.current_tab_mut() {
                        if let Some(term) = &mut tab.terminal {
                            term.set_scrollback(0);
                        }
                    }
                } else if let Some(tab) = self.current_tab_mut() {
                    // 同步全局字体大小（用户未自定义且大小变化时）
                    if let Some(term) = &mut tab.terminal {
                        if tab.custom_font_size.is_none() && term.font_size() != global_font_size {
                            term.set_font_size(global_font_size);
                        }
                    }
                }
            }
            self.render_session_view(ui);
        } else {
            self.render_management_view(ui, store);
        }
    }

    // ===================================================================
    // SSH 输出轮询
    // ===================================================================

    fn poll_ssh_output(&mut self) {
        // 轮询所有标签的 SSH 输出
        for tab in &mut self.tabs {
            let has_rx = tab.output_rx.is_some();
            if !has_rx {
                continue;
            }

            // 收集需要处理的断开/错误状态
            let mut disconnect_reason: Option<String> = None;
            let mut error_msg: Option<String> = None;

            loop {
                let rx = tab.output_rx.as_ref().unwrap();
                match rx.try_recv() {
                    Ok(SshOutput::TerminalData(data)) => {
                        if let Some(term) = &mut tab.terminal {
                            term.process(&data);
                        }
                    }
                    Ok(SshOutput::Connected) => {
                        tab.connection_state = SessionState::Connected;
                        tab.status_msg = "已连接".to_string();
                        log::info!("SSH 连接就绪: {}", tab.name);
                    }
                    Ok(SshOutput::Disconnected(reason)) => {
                        tab.status_msg = format!("连接断开: {}", reason);
                        log::info!("SSH 连接断开: {} - {}", tab.name, reason);
                        tab.connection_state = SessionState::Error(reason.clone());
                        disconnect_reason = Some(reason);
                        break;
                    }
                    Ok(SshOutput::Error(err)) => {
                        tab.status_msg = format!("连接错误: {}", err);
                        tab.connection_state = SessionState::Error(err.clone());
                        log::error!("SSH 错误: {} - {}", tab.name, err);
                        error_msg = Some(err);
                        break;
                    }
                    Err(mpsc::TryRecvError::Empty) => break,
                    Err(mpsc::TryRecvError::Disconnected) => {
                        tab.status_msg = "SSH 连接已断开".to_string();
                        disconnect_reason = Some("通道已断开".to_string());
                        break;
                    }
                }
            }

            // 在循环外处理断开连接
            if disconnect_reason.is_some() || error_msg.is_some() {
                tab.disconnect_terminal();
            }
        }
    }

    // ===================================================================
    // SFTP 输出轮询
    // ===================================================================

    fn poll_sftp_output(&mut self) {
        // 轮询所有标签的 SFTP 输出
        for tab in &mut self.tabs {
            // 先收集所有待处理的消息，再逐个处理，避免借用冲突
            let mut messages = Vec::new();
            if let Some(rx) = &tab.sftp_rx {
                loop {
                    match rx.try_recv() {
                        Ok(msg) => messages.push(msg),
                        Err(mpsc::TryRecvError::Empty) => break,
                        Err(mpsc::TryRecvError::Disconnected) => {
                            tab.sftp_connected = false;
                            tab.sftp_tx = None;
                            tab.sftp_rx = None;
                            tab.sftp_status_msg = "SFTP 通道已关闭".to_string();
                            break;
                        }
                    }
                }
            }

            for msg in messages {
                match msg {
                    SftpResponse::DirectoryList(path, entries) => {
                        tab.remote_current_dir = path.clone();
                        tab.remote_dir_input = path;
                        tab.remote_files = entries;
                        tab.remote_selected = None;
                    }
                    SftpResponse::TransferProgress(task) => {
                        let existing = tab
                            .transfer_tasks
                            .iter_mut()
                            .find(|t| t.source == task.source && t.destination == task.destination);
                        if let Some(existing) = existing {
                            *existing = task;
                        } else {
                            tab.transfer_tasks.push(task);
                        }
                    }
                    SftpResponse::OperationDone(msg) => {
                        tab.sftp_status_msg = msg;
                    }
                    SftpResponse::Error(err) => {
                        tab.sftp_status_msg = err;
                        log::warn!("SFTP 错误: {}", tab.sftp_status_msg);
                    }
                    SftpResponse::Connected => {
                        tab.sftp_connected = true;
                        tab.sftp_status_msg = "SFTP 已连接".to_string();
                        log::info!("SFTP 连接就绪: {}", tab.name);
                        // 请求列出远程家目录
                        if let Some(tx) = &tab.sftp_tx {
                            let _ = tx.send(SftpRequest::ListDirectory(".".to_string()));
                        }
                    }
                    SftpResponse::Disconnected => {
                        tab.sftp_connected = false;
                        tab.sftp_tx = None;
                        tab.sftp_rx = None;
                        tab.remote_files.clear();
                        tab.sftp_status_msg = "SFTP 已断开".to_string();
                        log::info!("SFTP 连接已断开: {}", tab.name);
                    }
                }
            }
        }
    }

    // ===================================================================
    // 会话视图（终端 + SFTP）
    // ===================================================================

    fn render_session_view(&mut self, ui: &mut egui::Ui) {
        ui.ctx().request_repaint();

        // 顶部标签栏
        let mut go_back = false;
        let mut tab_to_close = None;
        let mut tab_to_activate = None;

        ui.horizontal(|ui| {
            // 返回按钮
            if ui.button("⬅ 返回列表").clicked() {
                go_back = true;
            }

            ui.separator();

            // 渲染会话标签
            for (idx, tab) in self.tabs.iter().enumerate() {
                let is_active = self.active_tab_index == Some(idx);

                // 状态图标
                let status_icon = match tab.connection_state {
                    SessionState::Connected => "🟢",
                    SessionState::Connecting => "🟡",
                    SessionState::Error(_) => "🔴",
                    SessionState::Disconnected => "⚪",
                };

                let label = format!("{} {}", status_icon, tab.name);

                // 标签按钮
                let response = ui.selectable_label(is_active, &label);
                if response.clicked() {
                    tab_to_activate = Some(idx);
                }

                // 关闭按钮
                if ui.small_button("✕").clicked() {
                    tab_to_close = Some(idx);
                }

                ui.separator();
            }
        });

        // 处理返回操作
        if go_back {
            self.active_tab_index = None;
            return;
        }

        // 处理标签切换
        if let Some(idx) = tab_to_activate {
            self.active_tab_index = Some(idx);
        }

        // 处理标签关闭
        if let Some(idx) = tab_to_close {
            self.close_tab(idx);
        }

        ui.separator();

        // 如果没有活动标签，显示提示
        let Some(active_idx) = self.active_tab_index else {
            ui.centered_and_justified(|ui| {
                ui.label("请从左侧选择一个连接，点击「创建会话」打开新标签");
            });
            return;
        };

        // 确保活动标签索引有效
        if active_idx >= self.tabs.len() {
            self.active_tab_index = None;
            return;
        }

        // 获取当前标签的会话信息用于显示
        let session_info = self
            .sessions
            .iter()
            .find(|s| s.id == self.tabs[active_idx].session_id)
            .map(|s| (s.name.clone(), s.username.clone(), s.host.clone(), s.port));

        // 克隆标签状态用于显示
        let tab_state = (
            self.tabs[active_idx].connection_state.clone(),
            self.tabs[active_idx].sftp_connected,
            self.tabs[active_idx].sftp_tx.is_some(),
        );

        // 顶部连接信息栏
        let mut disconnect_terminal = false;
        let mut reconnect_terminal = false;
        let mut disconnect_sftp = false;

        ui.horizontal(|ui| {
            if let Some((ref name, ref username, ref host, port)) = session_info {
                ui.heading(format!("🖥 {} ({}@{}:{})", name, username, host, port));
            }

            // 终端连接状态
            let term_color = match &tab_state.0 {
                SessionState::Connected => Color32::from_rgb(50, 200, 50),
                SessionState::Connecting => Color32::from_rgb(200, 200, 50),
                _ => Color32::from_rgb(220, 50, 50),
            };
            let term_text = match &tab_state.0 {
                SessionState::Connected => "🟢 终端",
                SessionState::Connecting => "🟡 终端...",
                _ => "🔴 终端",
            };
            ui.colored_label(term_color, term_text);

            // SFTP 连接状态
            if tab_state.1 {
                ui.colored_label(Color32::from_rgb(50, 200, 50), "🟢 SFTP");
            } else if tab_state.2 {
                ui.colored_label(Color32::from_rgb(200, 200, 50), "🟡 SFTP...");
            }

            // 终端按钮：根据连接状态显示不同按钮
            let has_terminal =
                tab_state.0 == SessionState::Connected || tab_state.0 == SessionState::Connecting;
            if has_terminal {
                if ui.button("🔌 断开终端").clicked() {
                    disconnect_terminal = true;
                }
            } else {
                if ui.button("🔄 重连终端").clicked() {
                    reconnect_terminal = true;
                }
            }

            // 断开 SFTP（仅在 SFTP 已连接时显示）
            if tab_state.1 || tab_state.2 {
                if ui.button("📁 断开SFTP").clicked() {
                    disconnect_sftp = true;
                }
            }
        });

        // 在闭包外处理断开操作
        if disconnect_terminal {
            if let Some(tab) = self.current_tab_mut() {
                tab.disconnect_terminal();
            }
        }
        if disconnect_sftp {
            if let Some(tab) = self.current_tab_mut() {
                tab.disconnect_sftp();
            }
        }
        if reconnect_terminal {
            self.reconnect_terminal(ui);
        }

        // 终端已断开但 SFTP 仍连接时，自动切换到 SFTP tab
        if let Some(tab) = self.current_tab_mut() {
            if tab.terminal.is_none()
                && tab.active_tab == SessionViewTab::Terminal
                && (tab.sftp_connected || tab.sftp_tx.is_some())
            {
                tab.active_tab = SessionViewTab::Sftp;
            }
        }

        // 子 Tab 栏（终端/SFTP)
        ui.horizontal(|ui| {
            let tab = &self.tabs[active_idx];
            if tab.terminal.is_some() {
                let mut active_sub_tab = tab.active_tab.clone();
                ui.selectable_value(&mut active_sub_tab, SessionViewTab::Terminal, "🖥 终端");
                // 更新到标签
                if active_sub_tab != tab.active_tab {
                    self.tabs[active_idx].active_tab = active_sub_tab;
                }
            } else {
                ui.add_enabled(false, egui::Label::new("🖥 终端(已断开)"));
            }

            let tab = &self.tabs[active_idx];
            let mut active_sub_tab = tab.active_tab.clone();
            if ui
                .selectable_value(&mut active_sub_tab, SessionViewTab::Sftp, "📁 SFTP")
                .clicked()
            {
                self.tabs[active_idx].active_tab = active_sub_tab;
                self.init_sftp_connection();
                if self.tabs[active_idx].local_files.is_empty() {
                    self.refresh_local_files();
                }
            }
        });
        ui.separator();

        // 根据当前子 Tab 渲染内容
        let active_sub_tab = self.tabs[active_idx].active_tab.clone();
        match active_sub_tab {
            SessionViewTab::Terminal => self.render_terminal_content(ui),
            SessionViewTab::Sftp => self.render_sftp_panel(ui),
        }
    }

    // ===================================================================
    // 终端内容渲染
    // ===================================================================

    fn render_terminal_content(&mut self, ui: &mut egui::Ui) {
        let Some(tab) = self.current_tab() else {
            ui.centered_and_justified(|ui| {
                ui.label("没有活动的会话标签");
            });
            return;
        };

        if tab.terminal.is_none() {
            ui.centered_and_justified(|ui| {
                ui.label("终端未连接");
            });
            return;
        }

        let font_size = tab.terminal.as_ref().map(|t| t.font_size()).unwrap_or(14.0);
        let is_dark_mode = ui.visuals().dark_mode;
        let status_msg = tab.status_msg.clone();

        let header_height = 40.0;
        let status_height = 28.0;
        let available_height = ui.available_height() - status_height - header_height;

        let mono_font = egui::FontId::monospace(font_size);
        let raw_char_width = ui.fonts(|f| {
            let glyph = f.glyph_width(&mono_font, 'M');
            if glyph > 0.0 { glyph } else { font_size * 0.6 }
        });
        let raw_line_height = ui.fonts(|f| f.row_height(&mono_font));
        // 像素级舍入，确保光标位置与 egui 渲染的文本精确对齐
        // 参考 diff 工具的处理方式，避免字体大小改变时的累积误差
        let pixels_per_point = ui.pixels_per_point();
        let char_width = (raw_char_width * pixels_per_point).round() / pixels_per_point;
        let line_height = (raw_line_height * pixels_per_point).round() / pixels_per_point;

        let new_cols = ((ui.available_width() - 20.0) / char_width)
            .max(1.0)
            .min(f32::from(u16::MAX)) as u16;
        let new_rows = (available_height / line_height)
            .max(1.0)
            .min(f32::from(u16::MAX)) as u16;

        // 调整终端大小
        let Some(tab) = self.current_tab_mut() else {
            return;
        };
        if let Some(term) = &mut tab.terminal {
            let (cur_cols, cur_rows) = term.size();
            if new_cols != cur_cols || new_rows != cur_rows {
                term.resize(new_cols, new_rows);
                if let Some(tx) = &tab.input_tx {
                    let _ = tx.send(SshInput::Resize(new_cols, new_rows));
                }
            }
        }

        // 渲染终端内容（不使用 ScrollArea，直接用滚轮控制终端滚动）
        let bg_color = if is_dark_mode {
            Color32::from_rgb(0x1e, 0x1e, 0x1e)
        } else {
            Color32::from_rgb(0xff, 0xff, 0xff)
        };

        // 获取当前可用区域（不覆盖标题）
        let available_rect = ui.available_rect_before_wrap();
        ui.painter().rect_filled(available_rect, 0.0, bg_color);

        let Some(tab) = self.current_tab_mut() else {
            return;
        };
        if let Some(term) = &mut tab.terminal {
            let job = term.render_to_layout_job(is_dark_mode);
            let content_w = f32::from(new_cols) * char_width + 20.0;
            ui.set_min_width(content_w);
            ui.set_min_height(available_height);

            let response = ui.label(job);
            let text_rect = response.rect;

            // 绘制闪烁光标（考虑滚动偏移）
            let (c_col, c_row) = term.cursor_position();
            let scrollback = term.scrollback();
            let cursor_w = term.cursor_char_width();
            let cursor_x = text_rect.left() + f32::from(c_col) * char_width;
            // 光标位置需要考虑滚动偏移
            let cursor_y = text_rect.top() + f32::from(c_row + scrollback as u16) * line_height;
            let cursor_width = char_width * f32::from(cursor_w);

            // 只有光标在可视区域内才显示
            let cursor_in_view =
                cursor_y >= text_rect.top() && cursor_y + line_height <= text_rect.bottom();
            if cursor_in_view {
                let time = ui.ctx().input(|i| i.time);
                let blink_on = (time * 2.0) as u64 % 2 == 0;
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
            }

            // 设置 IME 输出位置
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
        } else if tab.connection_state == SessionState::Connecting {
            ui.centered_and_justified(|ui| {
                ui.label("正在连接...");
            });
        }

        // 底部状态栏
        let cursor_pos = tab
            .terminal
            .as_ref()
            .map(|t| t.cursor_position())
            .unwrap_or((0, 0));
        ui.separator();
        ui.horizontal(|ui| {
            ui.label(format!(
                "列: {} 行: {} | 字号: {:.0} | {}",
                cursor_pos.0, cursor_pos.1, font_size, status_msg
            ));
        });
    }

    // ===================================================================
    // SFTP 面板渲染
    // ===================================================================

    fn render_sftp_panel(&mut self, ui: &mut egui::Ui) {
        // 克隆需要的状态，避免借用冲突
        let sftp_state = self
            .current_tab()
            .map(|t| (t.sftp_connected, t.sftp_tx.is_some()));

        let Some((sftp_connected, has_sftp_tx)) = sftp_state else {
            ui.centered_and_justified(|ui| {
                ui.label("没有活动的会话标签");
            });
            return;
        };

        if !sftp_connected {
            if has_sftp_tx {
                ui.centered_and_justified(|ui| {
                    ui.label("正在连接 SFTP...");
                });
            } else {
                let mut init_sftp = false;
                ui.centered_and_justified(|ui| {
                    ui.label("SFTP 未连接");
                    if ui.button("连接 SFTP").clicked() {
                        init_sftp = true;
                    }
                });
                if init_sftp {
                    self.init_sftp_connection();
                }
            }
            return;
        }

        // 清理已完成的传输任务
        if let Some(tab) = self.current_tab_mut() {
            tab.transfer_tasks.retain(|t| !t.done);
        }

        let total_h = ui.available_height();

        // 上下分区：上方文件面板区占 3/4，下方传输进度区占 1/4（上限 120px）
        let transfer_h = (total_h * 0.25).min(120.0);
        let file_area_h = total_h - transfer_h;

        // ---- 上方：文件面板区域 ----
        let total_w = ui.available_width();

        // 按钮列宽
        let btn_col_w = 70.0_f32;
        // 每侧面板宽度 = (总宽 - 按钮列宽) / 2
        let side_w = (total_w - btn_col_w) / 2.0;

        // 文件面板区的净高度（去掉分隔线等）
        let panel_inner_h = file_area_h - 4.0;

        ui.horizontal(|ui| {
            // 减小元素间距，使按钮列与两侧面板紧凑排列
            ui.spacing_mut().item_spacing.x = 2.0;

            // ===== 左侧：本地文件面板 =====
            ui.vertical(|ui| {
                ui.set_min_width(side_w);
                ui.set_max_width(side_w);
                ui.set_min_height(panel_inner_h);

                ui.strong("📁 本地文件");
                ui.add_space(2.0);

                // 路径导航栏
                ui.horizontal(|ui| {
                    let Some(tab) = self.current_tab_mut() else {
                        return;
                    };
                    let resp = ui.text_edit_singleline(&mut tab.local_dir_input);
                    if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        let p = tab.local_dir_input.trim().to_string();
                        if Path::new(&p).is_dir() {
                            tab.local_current_dir = p;
                        }
                    }
                    if ui.button("📂").clicked() {
                        if let Some(path) = rfd::FileDialog::new()
                            .set_title("选择本地目录")
                            .set_directory(&tab.local_current_dir)
                            .pick_folder()
                        {
                            tab.local_current_dir = path.to_string_lossy().to_string();
                        }
                    }
                    if ui.button("⬆").clicked() {
                        if let Some(parent) = Path::new(&tab.local_current_dir).parent() {
                            tab.local_current_dir = parent.to_string_lossy().to_string();
                        }
                    }
                    if ui.button("🏠").clicked() {
                        tab.local_current_dir = sftp::home_dir();
                    }
                });

                // 表头
                render_file_header(ui);

                // 文件列表为空时加载
                let should_refresh = self
                    .current_tab()
                    .map(|t| t.local_files.is_empty())
                    .unwrap_or(false);
                if should_refresh {
                    self.refresh_local_files();
                }

                // 列表高度 = 面板高度 - 标题/路径栏/表头约 70px
                let list_h = (panel_inner_h - 70.0).max(60.0);

                egui::ScrollArea::vertical()
                    .max_height(list_h)
                    .id_salt("local_files_scroll")
                    .show(ui, |ui| {
                        let Some(tab) = self.current_tab() else {
                            return;
                        };
                        let action = render_file_rows(ui, &tab.local_files, tab.local_selected);
                        if let Some(idx) = action.select {
                            if let Some(tab) = self.current_tab_mut() {
                                tab.local_selected = Some(idx);
                            }
                        }
                        if let Some(idx) = action.open {
                            if let Some(tab) = self.current_tab_mut() {
                                tab.local_current_dir = tab.local_files[idx].path.clone();
                            }
                            self.refresh_local_files();
                        }
                    });
            });

            // ===== 中间：上传/下载按钮列（左对齐消除右侧间隙） =====
            ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
                ui.set_min_width(btn_col_w);
                ui.set_max_width(btn_col_w);
                ui.set_min_height(panel_inner_h);
                // 垂直留白使按钮居中
                let btn_area_h = panel_inner_h - 70.0;
                ui.add_space(btn_area_h / 2.0 - 30.0);

                let can_upload = self
                    .current_tab()
                    .and_then(|t| t.local_selected.and_then(|i| t.local_files.get(i)))
                    .map(|f| !f.is_dir)
                    .unwrap_or(false);

                if ui
                    .add_enabled(can_upload, egui::Button::new("上传 →"))
                    .clicked()
                {
                    if let Some(tab) = self.current_tab() {
                        if let Some(idx) = tab.local_selected {
                            if let Some(entry) = tab.local_files.get(idx) {
                                let local_path = entry.path.clone();
                                let remote_path = format!(
                                    "{}/{}",
                                    tab.remote_current_dir.trim_end_matches('/'),
                                    entry.name
                                );
                                if let Some(tx) = &tab.sftp_tx {
                                    let _ = tx.send(SftpRequest::Upload(local_path, remote_path));
                                }
                            }
                        }
                    }
                }

                ui.add_space(12.0);

                let can_download = self
                    .current_tab()
                    .and_then(|t| t.remote_selected.and_then(|i| t.remote_files.get(i)))
                    .map(|f| !f.is_dir)
                    .unwrap_or(false);

                if ui
                    .add_enabled(can_download, egui::Button::new("← 下载"))
                    .clicked()
                {
                    if let Some(tab) = self.current_tab() {
                        if let Some(idx) = tab.remote_selected {
                            if let Some(entry) = tab.remote_files.get(idx) {
                                let remote_path = entry.path.clone();
                                let local_path = format!(
                                    "{}/{}",
                                    tab.local_current_dir.trim_end_matches('/'),
                                    entry.name
                                );
                                if let Some(tx) = &tab.sftp_tx {
                                    let _ = tx.send(SftpRequest::Download(remote_path, local_path));
                                }
                            }
                        }
                    }
                }
            });

            // ===== 右侧：远程文件面板 =====
            ui.vertical(|ui| {
                ui.set_min_width(side_w);
                ui.set_max_width(side_w);
                ui.set_min_height(panel_inner_h);

                ui.strong("📁 远程文件");
                ui.add_space(2.0);

                // 路径导航栏
                ui.horizontal(|ui| {
                    let Some(tab) = self.current_tab_mut() else {
                        return;
                    };
                    let resp = ui.text_edit_singleline(&mut tab.remote_dir_input);
                    if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        tab.remote_current_dir = tab.remote_dir_input.trim().to_string();
                    }
                    if ui.button("⬆").clicked() {
                        if let Some(parent) = Path::new(&tab.remote_current_dir).parent() {
                            tab.remote_current_dir = parent.to_string_lossy().to_string();
                        }
                    }
                    if ui.button("🔄").clicked() {
                        self.refresh_remote_files();
                    }
                });

                // 表头
                render_file_header(ui);

                // 列表高度 = 面板高度 - 标题/路径栏/表头约 70px
                let list_h = (panel_inner_h - 70.0).max(60.0);

                egui::ScrollArea::vertical()
                    .max_height(list_h)
                    .id_salt("remote_files_scroll")
                    .show(ui, |ui| {
                        let Some(tab) = self.current_tab() else {
                            return;
                        };
                        let action = render_file_rows(ui, &tab.remote_files, tab.remote_selected);
                        if let Some(idx) = action.select {
                            if let Some(tab) = self.current_tab_mut() {
                                tab.remote_selected = Some(idx);
                            }
                        }
                        if let Some(idx) = action.open {
                            if let Some(tab) = self.current_tab_mut() {
                                tab.remote_current_dir = tab.remote_files[idx].path.clone();
                            }
                            self.refresh_remote_files();
                        }
                    });
            });
        });

        // ---- 下方：传输进度区（始终显示） ----
        ui.separator();
        ui.strong("传输进度:");
        let has_transfer_tasks = self
            .current_tab()
            .map(|t| !t.transfer_tasks.is_empty())
            .unwrap_or(false);

        if !has_transfer_tasks {
            ui.weak("暂无传输任务");
        } else {
            egui::ScrollArea::vertical()
                .max_height(transfer_h - 40.0)
                .id_salt("transfer_progress_scroll")
                .show(ui, |ui| {
                    let Some(tab) = self.current_tab() else {
                        return;
                    };
                    for task in &tab.transfer_tasks {
                        ui.horizontal(|ui| {
                            let icon = match task.direction {
                                super::models::TransferDirection::Upload => "⬆",
                                super::models::TransferDirection::Download => "⬇",
                            };
                            ui.label(format!("{} {}", icon, task.filename));
                            let bar = egui::ProgressBar::new(task.progress()).show_percentage();
                            ui.add_sized([160.0, 16.0], bar);
                            ui.label(task.progress_text());
                        });
                    }
                });
        }

        // 底部状态栏
        ui.separator();
        ui.horizontal(|ui| {
            if let Some(tab) = self.current_tab() {
                ui.label(&tab.sftp_status_msg);
            }
        });
    }

    // ===================================================================
    // 终端键盘输入处理
    // ===================================================================

    fn process_terminal_input(
        &mut self,
        tx: &mpsc::SyncSender<SshInput>,
        ctx: &egui::Context,
    ) -> bool {
        let mut had_input = false;
        let mut ime_active = self.current_tab().map(|t| t.ime_active).unwrap_or(false);

        ctx.input_mut(|i| {
            for event in i.events.clone() {
                match event {
                    egui::Event::Key {
                        key,
                        pressed: true,
                        modifiers,
                        ..
                    } => {
                        if ime_active && !modifiers.ctrl {
                            continue;
                        }
                        if key == egui::Key::C && modifiers.ctrl {
                            let _ = tx.send(SshInput::KeyInput(vec![0x03]));
                            i.consume_key(modifiers, key);
                            had_input = true;
                            continue;
                        }
                        if key == egui::Key::D && modifiers.ctrl {
                            let _ = tx.send(SshInput::KeyInput(vec![0x04]));
                            i.consume_key(modifiers, key);
                            had_input = true;
                            continue;
                        }
                        if key == egui::Key::Z && modifiers.ctrl {
                            let _ = tx.send(SshInput::KeyInput(vec![0x1a]));
                            i.consume_key(modifiers, key);
                            had_input = true;
                            continue;
                        }
                        if key == egui::Key::L && modifiers.ctrl {
                            let _ = tx.send(SshInput::KeyInput(vec![0x0c]));
                            i.consume_key(modifiers, key);
                            had_input = true;
                            continue;
                        }
                        if key == egui::Key::Enter {
                            let _ = tx.send(SshInput::KeyInput(vec![0x0d]));
                            i.consume_key(modifiers, key);
                            had_input = true;
                            continue;
                        }
                        if key == egui::Key::Backspace {
                            let _ = tx.send(SshInput::KeyInput(vec![0x7f]));
                            i.consume_key(modifiers, key);
                            had_input = true;
                            continue;
                        }
                        if key == egui::Key::Tab {
                            let _ = tx.send(SshInput::KeyInput(vec![0x09]));
                            i.consume_key(modifiers, key);
                            had_input = true;
                            continue;
                        }
                        if key == egui::Key::Escape {
                            let _ = tx.send(SshInput::KeyInput(vec![0x1b]));
                            i.consume_key(modifiers, key);
                            had_input = true;
                            continue;
                        }
                        if key == egui::Key::ArrowUp {
                            let _ = tx.send(SshInput::KeyInput(b"\x1b[A".to_vec()));
                            i.consume_key(modifiers, key);
                            had_input = true;
                            continue;
                        }
                        if key == egui::Key::ArrowDown {
                            let _ = tx.send(SshInput::KeyInput(b"\x1b[B".to_vec()));
                            i.consume_key(modifiers, key);
                            had_input = true;
                            continue;
                        }
                        if key == egui::Key::ArrowRight {
                            let _ = tx.send(SshInput::KeyInput(b"\x1b[C".to_vec()));
                            i.consume_key(modifiers, key);
                            had_input = true;
                            continue;
                        }
                        if key == egui::Key::ArrowLeft {
                            let _ = tx.send(SshInput::KeyInput(b"\x1b[D".to_vec()));
                            i.consume_key(modifiers, key);
                            had_input = true;
                            continue;
                        }
                    }
                    egui::Event::Text(text) => {
                        if ime_active {
                            continue;
                        }
                        if text.is_empty() || text == "\r" || text == "\x08" || text == "\t" {
                            continue;
                        }
                        let _ = tx.send(SshInput::KeyInput(text.as_bytes().to_vec()));
                        had_input = true;
                    }
                    egui::Event::Ime(ime_event) => match ime_event {
                        egui::ImeEvent::Enabled => {
                            ime_active = true;
                        }
                        egui::ImeEvent::Disabled => {
                            ime_active = false;
                        }
                        egui::ImeEvent::Commit(text) => {
                            if !text.is_empty() {
                                let _ = tx.send(SshInput::KeyInput(text.as_bytes().to_vec()));
                                had_input = true;
                            }
                        }
                        egui::ImeEvent::Preedit(_) => {}
                    },
                    _ => {}
                }
            }
            i.events.clear();
        });

        // 更新当前标签的 IME 状态
        if let Some(tab) = self.current_tab_mut() {
            tab.ime_active = ime_active;
        }

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

        had_input
    }

    // ===================================================================
    // 管理视图
    // ===================================================================

    fn render_management_view(&mut self, ui: &mut egui::Ui, store: &SshStore) {
        ui.heading("🖥 SSH 客户端");
        ui.separator();

        self.render_toolbar(ui, store);

        ui.add_space(4.0);

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
                self.left_panel_width =
                    (self.left_panel_width + delta).clamp(min_left_width, max_left_width);
            }
            if separator_response.hovered() || separator_response.dragged() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeColumn);
            }

            ui.add_space(4.0);

            ui.vertical(|ui| {
                self.render_detail_panel(ui, store);
            });
        });

        let stats_height = ui.text_style_height(&egui::TextStyle::Small) + 16.0;
        egui::TopBottomPanel::bottom("ssh_status")
            .exact_height(stats_height)
            .show_inside(ui, |ui| {
                ui.separator();
                self.render_status_bar(ui);
            });

        let title = if self.editing_id.is_some() {
            "编辑连接"
        } else {
            "新增连接"
        };

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
            ui.add_sized(
                [100.0, 20.0],
                egui::TextEdit::singleline(&mut self.form.port),
            );
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
            let btn_label = if is_editing {
                "💾 保存修改"
            } else {
                "✅ 保存连接"
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
            }

            // 会话按钮：如果有打开的标签，点击进入最近的会话
            if !self.tabs.is_empty() {
                if ui.button("🖥 会话").clicked() {
                    // 进入最近的会话（最后一个标签）
                    self.active_tab_index = Some(self.tabs.len() - 1);
                }
            }
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

                    // 检查该会话是否有打开的标签
                    let tab_index = self.tabs.iter().position(|t| t.session_id == session.id);
                    let icon = if let Some(tab_idx) = tab_index {
                        match &self.tabs[tab_idx].connection_state {
                            SessionState::Connected => "🟢",
                            SessionState::Connecting => "🟡",
                            SessionState::Error(_) => "🔴",
                            SessionState::Disconnected => "⚪",
                        }
                    } else {
                        "⚪"
                    };

                    let mut label = format!(
                        "{} {} ({})",
                        icon,
                        session.name,
                        format_session_addr(session)
                    );

                    // 如果有打开的标签，添加标记
                    if tab_index.is_some() {
                        label.push_str(" [已打开]");
                    }

                    let response = ui.selectable_label(is_selected, &label);
                    if response.clicked() {
                        to_select = Some(idx);
                    }

                    // 添加工具提示
                    if response.hovered() {
                        response.on_hover_text("点击选中连接配置");
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
            AuthMethod::KeyFile {
                private_key_path, ..
            } => Some(private_key_path.clone()),
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
                let s = &self.sessions[idx];
                let host = s.host.clone();
                let port = s.port;
                let username = s.username.clone();
                let auth = s.auth_method.clone();
                let session_id = s.id;

                let mono_font = egui::FontId::monospace(
                    ui.style()
                        .text_styles
                        .get(&egui::TextStyle::Monospace)
                        .map(|f| f.size)
                        .unwrap_or(14.0),
                );
                let est_char_w = ui.fonts(|f| f.glyph_width(&mono_font, 'M'));
                let est_line_h = ui.fonts(|f| f.row_height(&mono_font));
                let est_cols = if est_char_w > 0.0 {
                    ((ui.available_width() * 0.7 / est_char_w)
                        .max(80.0)
                        .min(f32::from(u16::MAX))) as u16
                } else {
                    120
                };
                let est_rows = if est_line_h > 0.0 {
                    ((ui.available_height() * 0.5 / est_line_h)
                        .max(24.0)
                        .min(f32::from(u16::MAX))) as u16
                } else {
                    30
                };

                // 创建新的会话标签
                let mut new_tab = SessionTab::new(session_id, session_name.clone());
                new_tab.connection_state = SessionState::Connecting;
                new_tab.status_msg = format!("正在连接 {}...", session_name);

                match SshClient::connect(&host, port, &username, &auth, est_cols, est_rows) {
                    Ok((tx, rx)) => {
                        new_tab.input_tx = Some(tx);
                        new_tab.output_rx = Some(rx);
                        new_tab.terminal = Some(TerminalEmulator::new(
                            est_cols,
                            est_rows,
                            ui.style()
                                .text_styles
                                .get(&egui::TextStyle::Monospace)
                                .map(|f| f.size)
                                .unwrap_or(14.0),
                        ));
                        new_tab.active_tab = SessionViewTab::Terminal;

                        // 添加标签并设置为活动标签
                        self.tabs.push(new_tab);
                        self.active_tab_index = Some(self.tabs.len() - 1);

                        log::info!("SSH 连接已发起: {}@{}", username, host);
                    }
                    Err(e) => {
                        new_tab.connection_state = SessionState::Error(format!("连接失败: {}", e));
                        new_tab.status_msg = format!("连接失败: {}", e);

                        // 添加标签并设置为活动标签（显示错误状态）
                        self.tabs.push(new_tab);
                        self.active_tab_index = Some(self.tabs.len() - 1);

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
                        // 关闭该会话的所有标签
                        let tabs_to_close: Vec<usize> = self
                            .tabs
                            .iter()
                            .enumerate()
                            .filter(|(_, t)| t.session_id == session_id)
                            .map(|(i, _)| i)
                            .collect();
                        for idx in tabs_to_close.into_iter().rev() {
                            self.close_tab(idx);
                        }

                        self.selected_index = None;
                        self.refresh_sessions(store);
                        log::info!("已删除连接: {}", session_name);
                    }
                    Err(e) => {
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
                let (encrypted_password, iv, salt) = crypto::encrypt_password(&self.form.password)
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
            store
                .update_session(id, &new_session)
                .map_err(|e| format!("更新连接失败: {}", e))?;
            log::info!("SSH 连接 '{}' 已更新", new_session.name);
        } else {
            store
                .insert_session(&new_session)
                .map_err(|e| format!("保存连接失败: {}", e))?;
            log::info!("SSH 连接 '{}' 已创建", new_session.name);
        }

        Ok(())
    }

    fn render_status_bar(&self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if let Some(tab) = self.current_tab() {
                let state_color = match &tab.connection_state {
                    SessionState::Connected => Color32::from_rgb(50, 200, 50),
                    SessionState::Connecting => Color32::from_rgb(200, 200, 50),
                    SessionState::Error(_) => Color32::from_rgb(220, 50, 50),
                    SessionState::Disconnected => Color32::from_rgb(128, 128, 128),
                };
                let state_text = match &tab.connection_state {
                    SessionState::Connected => "已连接",
                    SessionState::Connecting => "连接中...",
                    SessionState::Error(e) => e.as_str(),
                    SessionState::Disconnected => "未连接",
                };
                ui.colored_label(state_color, state_text);
                ui.separator();
                ui.label(&tab.status_msg);
            } else {
                ui.label("就绪");
            }
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

/// 文件列表交互结果
struct FileListAction {
    select: Option<usize>,
    open: Option<usize>,
}

/// 渲染文件表头（三列等宽）
fn render_file_header(ui: &mut egui::Ui) {
    ui.columns(3, |cols| {
        cols[0].label(RichText::new("名称").strong());
        cols[1].label(RichText::new("大小").strong());
        cols[2].label(RichText::new("修改时间").strong());
    });
    ui.separator();
}

/// 渲染文件列表行（本地和远程共用）
///
/// 使用 `ui.columns(3)` 三列等宽，保证表头与数据列对齐。
/// 返回用户的点击/双击操作，由调用方处理状态变更。
fn render_file_rows(
    ui: &mut egui::Ui,
    files: &[super::models::FileEntry],
    selected: Option<usize>,
) -> FileListAction {
    let mut action = FileListAction {
        select: None,
        open: None,
    };

    for (idx, entry) in files.iter().enumerate() {
        let is_selected = selected == Some(idx);
        let icon = if entry.is_dir { "📁" } else { "📄" };

        ui.columns(3, |cols| {
            // 名称列：图标 + 文件名，使用 Label::truncate() 自动按列宽裁切
            let resp = cols[0].horizontal(|ui| {
                ui.label(icon);
                let text = if is_selected {
                    egui::RichText::new(&entry.name).strong()
                } else {
                    egui::RichText::new(&entry.name)
                };
                ui.add(egui::Label::new(text).truncate())
            });
            if resp.inner.clicked() {
                action.select = Some(idx);
            }
            if resp.inner.double_clicked() && entry.is_dir {
                action.open = Some(idx);
            }
            cols[1].label(entry.size_display());
            cols[2].label(entry.modified_display());
        });
    }

    action
}
