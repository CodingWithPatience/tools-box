use std::path::Path;
use std::sync::mpsc;

use egui::{Color32, RichText};

use super::client::SshClient;
use super::crypto;
use super::models::{
    AuthMethod, AuthType, NewSession, SessionForm, SessionState, SessionTab, SessionViewTab,
    SftpControl, SftpRequest, SftpResponse, SshInput, SshOutput, TerminalSelection,
    fail_unfinished_transfers,
};
use super::sftp::{self, SftpClient};
use super::store::SshStore;
use super::terminal::TerminalEmulator;

/// 终端内容与底部状态栏之间的安全间距
const TERMINAL_BOTTOM_PADDING: f32 = 6.0;
/// 等待系统剪贴板 Paste 事件的最长时间
const TERMINAL_PASTE_TIMEOUT_SECONDS: f64 = 2.0;
/// 滚轮每格滚动的终端行数
const TERMINAL_WHEEL_LINES_PER_NOTCH: f32 = 3.0;
/// 无法读取 egui 滚动速度时使用的每格滚动距离（点）
const DEFAULT_SCROLL_POINTS_PER_NOTCH: f32 = 40.0;
/// 单次滚轮事件最多滚动的行数，避免触控板惯性滚动跳跃过大
const TERMINAL_WHEEL_MAX_LINES: f32 = 300.0;
/// 终端滚动条宽度
const TERMINAL_SCROLLBAR_WIDTH: f32 = 8.0;
/// 终端滚动条与终端内容之间的间距
const TERMINAL_SCROLLBAR_GAP: f32 = 6.0;
/// 滚动条轨道上下两端与终端画布之间的内缩
const TERMINAL_SCROLLBAR_TRACK_INSET: f32 = 2.0;
/// 滚动条轨道最小高度（小于该值时不显示，避免滑块高度下限超过轨道高度）
const TERMINAL_SCROLLBAR_MIN_TRACK_HEIGHT: f32 = 24.0;
/// 滚动条滑块最小高度
const TERMINAL_SCROLLBAR_MIN_HANDLE_HEIGHT: f32 = 12.0;
/// 点击终端滚动条空白区域时的页面滚动比例（相对可视行数）
const TERMINAL_SCROLLBAR_PAGE_FRACTION: f32 = 0.9;
/// Backspace 键序列（DEL）
const TERMINAL_BACKSPACE_SEQUENCE: &[u8] = b"\x7f";
/// Ctrl+Backspace 键序列（Ctrl+W）
///
/// 等价于远端的 Ctrl+W：readline（bash）、zsh、fish 在默认的 emacs 编辑模式下
/// 都绑定为删除光标前一个单词；vi 编辑模式下该键为其他含义，不做转换
const TERMINAL_WORD_DELETE_SEQUENCE: &[u8] = b"\x17";

/// 会话标签切换方向
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TabSwitchDirection {
    /// 下一个标签
    Next,
    /// 上一个标签
    Previous,
}

/// 判断按键是否为会话标签切换快捷键（Ctrl+Tab / Ctrl+Shift+Tab）
fn tab_switch_direction(key: egui::Key, modifiers: egui::Modifiers) -> Option<TabSwitchDirection> {
    if key != egui::Key::Tab || !modifiers.ctrl {
        return None;
    }

    Some(if modifiers.shift {
        TabSwitchDirection::Previous
    } else {
        TabSwitchDirection::Next
    })
}

/// 从输入事件中查找会话标签切换快捷键
fn tab_switch_shortcut_from_events(
    events: &[egui::Event],
) -> Option<(egui::Modifiers, TabSwitchDirection)> {
    events.iter().find_map(|event| {
        let egui::Event::Key {
            key,
            pressed: true,
            modifiers,
            ..
        } = event
        else {
            return None;
        };

        tab_switch_direction(*key, *modifiers).map(|direction| (*modifiers, direction))
    })
}

/// 计算循环切换后的标签索引
///
/// 索引循环回绕；`current` 越界时按最后一个标签处理，标签数为 0 时返回 `None`
fn switched_tab_index(
    current: usize,
    tab_count: usize,
    direction: TabSwitchDirection,
) -> Option<usize> {
    if tab_count == 0 {
        return None;
    }

    let current = current.min(tab_count - 1);
    Some(match direction {
        TabSwitchDirection::Next => (current + 1) % tab_count,
        TabSwitchDirection::Previous => (current + tab_count - 1) % tab_count,
    })
}

/// 终端删除键序列；Ctrl+Backspace 删除光标前一个单词
fn terminal_backspace_sequence(ctrl: bool) -> &'static [u8] {
    if ctrl {
        TERMINAL_WORD_DELETE_SEQUENCE
    } else {
        TERMINAL_BACKSPACE_SEQUENCE
    }
}

/// 终端方向键序列；Ctrl+左右箭头按单词移动（xterm 的 CSI 1;5 形式）
fn terminal_arrow_sequence(key: egui::Key, ctrl: bool) -> Option<&'static [u8]> {
    match (key, ctrl) {
        (egui::Key::ArrowUp, _) => Some(b"\x1b[A"),
        (egui::Key::ArrowDown, _) => Some(b"\x1b[B"),
        (egui::Key::ArrowRight, false) => Some(b"\x1b[C"),
        (egui::Key::ArrowRight, true) => Some(b"\x1b[1;5C"),
        (egui::Key::ArrowLeft, false) => Some(b"\x1b[D"),
        (egui::Key::ArrowLeft, true) => Some(b"\x1b[1;5D"),
        _ => None,
    }
}

/// 终端编辑键序列：Home/End 跳转行首行尾、Insert/Delete/PageUp/PageDown 与 xterm 对齐
///
/// Home/End 在普通模式下发送 CSI 形式（`ESC [ H` / `ESC [ F`），远端开启应用光标键模式
/// （DECCKM，由 vim/less 等全屏程序打开）后发送 SS3 形式（`ESC O H` / `ESC O F`）；
/// Ctrl+Home/Ctrl+End 发送 xterm 带修饰键的 `CSI 1;5 H/F` 形式（带修饰键时不受 DECCKM 影响）。
/// Insert/Delete/PageUp/PageDown 固定为 `CSI n ~` 形式，该形式不受 DECCKM 影响。
///
/// 与方向键一致，这里只区分 Ctrl 修饰键，Shift/Alt 组合不改变发送的序列；
/// Ctrl+Insert、Shift+Insert、Shift+Delete 在 Windows 上由 `egui-winit` 提前转换为
/// 系统 Copy/Paste/Cut 事件（不产生 Key 事件），因此不会进入本映射。
fn terminal_edit_key_sequence(
    key: egui::Key,
    ctrl: bool,
    application_cursor: bool,
) -> Option<&'static [u8]> {
    match (key, ctrl, application_cursor) {
        (egui::Key::Home, true, _) => Some(b"\x1b[1;5H"),
        (egui::Key::Home, false, false) => Some(b"\x1b[H"),
        (egui::Key::Home, false, true) => Some(b"\x1bOH"),
        (egui::Key::End, true, _) => Some(b"\x1b[1;5F"),
        (egui::Key::End, false, false) => Some(b"\x1b[F"),
        (egui::Key::End, false, true) => Some(b"\x1bOF"),
        (egui::Key::Insert, false, _) => Some(b"\x1b[2~"),
        (egui::Key::Delete, false, _) => Some(b"\x1b[3~"),
        (egui::Key::PageUp, false, _) => Some(b"\x1b[5~"),
        (egui::Key::PageDown, false, _) => Some(b"\x1b[6~"),
        _ => None,
    }
}

/// 计算一次滚轮事件应滚动的终端行数
///
/// egui 会把一格滚轮换算为 `points_per_notch` 点（原生平台默认 40 点），
/// 因此按该比例换算为每格 [`TERMINAL_WHEEL_LINES_PER_NOTCH`] 行；
/// 触控板按滚动距离等比例换算，每次事件至少 1 行、最多 [`TERMINAL_WHEEL_MAX_LINES`] 行
fn terminal_scroll_lines(scroll_delta: f32, points_per_notch: f32) -> usize {
    if !scroll_delta.is_finite() || scroll_delta == 0.0 {
        return 0;
    }

    let points_per_notch = if points_per_notch.is_finite() && points_per_notch > 0.0 {
        points_per_notch
    } else {
        DEFAULT_SCROLL_POINTS_PER_NOTCH
    };

    let lines = (scroll_delta.abs() / points_per_notch * TERMINAL_WHEEL_LINES_PER_NOTCH)
        .round()
        .clamp(1.0, TERMINAL_WHEEL_MAX_LINES);

    // 已收敛到 1.0..=300.0，转换不会丢失精度
    lines as usize
}

/// 计算滚动条滑块的位置与高度（均为 0.0..=1.0 的比例）
///
/// 返回 `(滑块顶部位置比例, 滑块高度比例)`；位置 0 表示最早期历史、
/// 1 表示当前屏幕（滚动偏移为 0）
fn terminal_scrollbar_ratios(offset: usize, max_offset: usize, visible_rows: u16) -> (f32, f32) {
    let visible = f32::from(visible_rows);
    if max_offset == 0 || visible <= 0.0 {
        return (1.0, 1.0);
    }

    let max_offset_points = lines_to_f32(max_offset);
    let total = visible + max_offset_points;
    let handle = (visible / total).clamp(0.08, 1.0);
    // 窗口顶部在内容中的行号（0 表示最早的历史行）
    let start_line = max_offset_points - lines_to_f32(offset.min(max_offset));
    let position = (start_line / max_offset_points).clamp(0.0, 1.0);

    (position, handle)
}

/// 将滚动条滑块位置比例换算为滚动偏移（0 表示最底部）
fn terminal_scrollbar_offset(ratio: f32, max_offset: usize) -> usize {
    if max_offset == 0 || !ratio.is_finite() {
        return 0;
    }

    let max_offset_points = lines_to_f32(max_offset);
    let clamped = ratio.clamp(0.0, 1.0);
    let offset = max_offset_points - clamped * max_offset_points;

    // 已裁剪到 0.0..=max_offset 且不超过百万行，转换不会丢失精度
    offset.round() as usize
}

/// 滚动行数转浮点（滚动缓冲区有限，裁剪后 f32 可精确表示）
fn lines_to_f32(lines: usize) -> f32 {
    lines.min(1_000_000) as f32
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TerminalClipboardAction {
    Copy,
    Paste,
}

#[derive(Debug, Default, PartialEq, Eq)]
struct TerminalClipboardFrame {
    copy_requested: bool,
    interrupt_requested: bool,
    paste_requested: bool,
    paste_text: Option<String>,
}

fn terminal_clipboard_action(
    key: egui::Key,
    pressed: bool,
    repeat: bool,
    modifiers: egui::Modifiers,
) -> Option<TerminalClipboardAction> {
    if !pressed || repeat || !modifiers.ctrl || !modifiers.shift {
        return None;
    }

    match key {
        egui::Key::C => Some(TerminalClipboardAction::Copy),
        egui::Key::V => Some(TerminalClipboardAction::Paste),
        _ => None,
    }
}

fn is_terminal_clipboard_shortcut(key: egui::Key, modifiers: egui::Modifiers) -> bool {
    modifiers.ctrl && modifiers.shift && matches!(key, egui::Key::C | egui::Key::V)
}

fn analyze_terminal_clipboard_events(events: &[egui::Event]) -> TerminalClipboardFrame {
    let mut frame = TerminalClipboardFrame::default();
    for event in events {
        match event {
            egui::Event::Key {
                key,
                pressed,
                repeat,
                modifiers,
                ..
            } if modifiers.ctrl && *key == egui::Key::C => {
                if *pressed && !*repeat && !modifiers.shift {
                    frame.interrupt_requested = true;
                } else if let Some(action) =
                    terminal_clipboard_action(*key, *pressed, *repeat, *modifiers)
                {
                    match action {
                        TerminalClipboardAction::Copy => frame.copy_requested = true,
                        TerminalClipboardAction::Paste => frame.paste_requested = true,
                    }
                }
            }
            egui::Event::Key {
                key,
                pressed,
                repeat,
                modifiers,
                ..
            } if is_terminal_clipboard_shortcut(*key, *modifiers) => {
                match terminal_clipboard_action(*key, *pressed, *repeat, *modifiers) {
                    Some(TerminalClipboardAction::Copy) => frame.copy_requested = true,
                    Some(TerminalClipboardAction::Paste) => frame.paste_requested = true,
                    None => {}
                }
            }
            egui::Event::Copy if cfg!(target_os = "windows") => {
                // 本地 egui-winit 补丁会为 Ctrl+Shift+C 保留 Key 事件，
                // 因此 Windows 下剩余的 Copy 对应普通 Ctrl+C。
                frame.interrupt_requested = true;
            }
            egui::Event::Copy => frame.copy_requested = true,
            egui::Event::Paste(text) if frame.paste_text.is_none() => {
                frame.paste_text = Some(text.clone());
            }
            _ => {}
        }
    }

    frame
}

fn accepted_terminal_paste_text(
    frame: &TerminalClipboardFrame,
    paste_was_pending: bool,
) -> Option<&str> {
    if frame.paste_requested || paste_was_pending {
        frame.paste_text.as_deref()
    } else {
        None
    }
}

fn send_terminal_paste(tx: &mpsc::SyncSender<SshInput>, text: &str) -> Result<bool, String> {
    if text.is_empty() {
        return Ok(false);
    }

    tx.try_send(SshInput::KeyInput(text.as_bytes().to_vec()))
        .map(|()| true)
        .map_err(|error| match error {
            mpsc::TrySendError::Full(_) => "终端输入队列已满，请稍后重试".to_string(),
            mpsc::TrySendError::Disconnected(_) => "终端连接已关闭，无法粘贴".to_string(),
        })
}

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

    /// 复制终端选区；没有选区时复制当前可见内容
    fn copy_terminal_text(&mut self, ctx: &egui::Context) {
        let Some(tab) = self.current_tab_mut() else {
            return;
        };
        let Some(terminal) = tab.terminal.as_ref() else {
            return;
        };

        let (text, selected) = match tab.terminal_selection {
            Some(selection) => {
                let (start, end) = selection.normalized();
                (terminal.text_between(start, end), true)
            }
            None => (terminal.visible_text(), false),
        };

        if text.is_empty() {
            tab.status_msg = "终端没有可复制的文本".to_string();
            return;
        }

        ctx.copy_text(text);
        tab.status_msg = if selected {
            "已复制终端选中文本".to_string()
        } else {
            "已复制当前可见终端内容".to_string()
        };
    }

    /// 请求平台读取系统剪贴板并生成 Paste 事件
    fn request_terminal_paste(&mut self, ctx: &egui::Context) {
        ctx.send_viewport_cmd(egui::ViewportCommand::RequestPaste);
        ctx.request_repaint_after(std::time::Duration::from_secs_f64(
            TERMINAL_PASTE_TIMEOUT_SECONDS,
        ));
        let deadline = ctx.input(|i| i.time + TERMINAL_PASTE_TIMEOUT_SECONDS);
        if let Some(tab) = self.current_tab_mut() {
            tab.terminal_paste_deadline = Some(deadline);
            tab.status_msg = "已请求读取剪贴板".to_string();
        }
    }

    /// 应用终端粘贴成功后的统一状态
    fn finish_terminal_paste(&mut self) {
        if let Some(tab) = self.current_tab_mut() {
            tab.status_msg = "已粘贴剪贴板文本".to_string();
            tab.terminal_paste_deadline = None;
            tab.terminal_selection = None;
            if let Some(terminal) = &mut tab.terminal {
                terminal.set_scrollback(0);
            }
        }
    }

    /// 应用平台未返回可粘贴文本时的状态
    fn finish_empty_terminal_paste(&mut self) {
        if let Some(tab) = self.current_tab_mut() {
            tab.terminal_paste_deadline = None;
            tab.status_msg = "剪贴板为空，未执行粘贴".to_string();
        }
    }

    /// 处理会话标签切换快捷键（Ctrl+Tab 下一个 / Ctrl+Shift+Tab 上一个）
    ///
    /// 长按会按系统按键重复速率连续切换；消费按键只为阻止 Tab 被发送到远程，
    /// 阻止 egui 焦点导航依赖终端自身设置的 focus lock filter
    fn handle_tab_switch_shortcut(&mut self, ctx: &egui::Context) {
        let Some(current) = self.active_tab_index else {
            return;
        };

        let Some((modifiers, direction)) =
            ctx.input(|i| tab_switch_shortcut_from_events(&i.events))
        else {
            return;
        };

        // 消费按键，避免 Tab 被当作终端输入发送到远程
        ctx.input_mut(|i| i.consume_key(modifiers, egui::Key::Tab));

        let Some(next) = switched_tab_index(current, self.tabs.len(), direction) else {
            return;
        };

        if next != current {
            self.active_tab_index = Some(next);
            if let Some(tab) = self.tabs.get(next) {
                log::info!("SSH 会话标签已切换到: {}", tab.name);
            }
        }
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

    /// 请求加载指定远程目录，成功响应后再更新当前目录
    fn request_remote_directory(&mut self, path: String) {
        let Some(tab) = self.current_tab_mut() else {
            return;
        };
        if let Err(error) = send_remote_directory_request(tab.sftp_tx.as_ref(), path) {
            tab.remote_dir_input = tab.remote_current_dir.clone();
            tab.sftp_status_msg = error;
        }
    }

    /// 将文件传输任务加入队列
    fn enqueue_transfer(&mut self, task: super::models::TransferTask) {
        let Some(tab) = self.current_tab_mut() else {
            return;
        };
        if has_active_transfer(&tab.transfer_tasks, &task.source, &task.destination) {
            tab.sftp_status_msg = format!("传输任务已存在: {}", task.filename);
            return;
        }

        let request = match task.direction {
            super::models::TransferDirection::Upload => SftpRequest::Upload(task.clone()),
            super::models::TransferDirection::Download => SftpRequest::Download(task.clone()),
        };
        match tab.sftp_tx.as_ref().map(|tx| tx.try_send(request)) {
            Some(Ok(())) => {
                tab.transfer_tasks.push(task);
            }
            Some(Err(mpsc::TrySendError::Full(_))) => {
                tab.sftp_status_msg = "SFTP 请求队列已满，请稍后重试".to_string();
            }
            Some(Err(mpsc::TrySendError::Disconnected(_))) | None => {
                tab.sftp_status_msg = "SFTP 未连接，无法创建传输任务".to_string();
            }
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
            Ok((tx, control_tx, rx)) => {
                let Some(tab) = self.current_tab_mut() else {
                    return;
                };
                tab.sftp_tx = Some(tx);
                tab.sftp_control_tx = Some(control_tx);
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
            // 会话标签切换快捷键需在终端输入处理之前消费，避免 Tab 被发送到远程
            self.handle_tab_switch_shortcut(ui.ctx());

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
                    // 滚轮滚动终端历史（每格多行，触控板按滚动距离换算）
                    let points_per_notch = ui.ctx().options(|options| options.line_scroll_speed);
                    if let Some(tab) = self.current_tab_mut() {
                        if let Some(term) = &mut tab.terminal {
                            let lines = terminal_scroll_lines(scroll_y, points_per_notch);
                            let max_offset = term.scrollback_count();
                            let current = term.scrollback();
                            let next = if scroll_y > 0.0 {
                                current.saturating_add(lines).min(max_offset)
                            } else {
                                current.saturating_sub(lines)
                            };

                            if next != current {
                                term.set_scrollback(next);
                                tab.terminal_selection = None;
                            }
                        }
                    }
                } else if had_input {
                    // 有键盘输入时，自动滚动到光标位置
                    if let Some(tab) = self.current_tab_mut() {
                        if let Some(term) = &mut tab.terminal {
                            term.set_scrollback(0);
                        }
                        tab.terminal_selection = None;
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
                        if !data.is_empty() {
                            tab.terminal_selection = None;
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
                            fail_unfinished_transfers(
                                &mut tab.transfer_tasks,
                                "SFTP 通道已关闭，传输结果未知",
                            );
                            tab.sftp_connected = false;
                            tab.sftp_tx = None;
                            tab.sftp_control_tx = None;
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
                    SftpResponse::DirectoryError(path, error) => {
                        if tab.remote_dir_input == path {
                            tab.remote_dir_input = tab.remote_current_dir.clone();
                        }
                        tab.sftp_status_msg = error;
                    }
                    SftpResponse::TransferProgress(task) => {
                        let existing = tab
                            .transfer_tasks
                            .iter_mut()
                            .find(|existing| existing.id == task.id);
                        if let Some(existing) = existing {
                            *existing = task;
                        } else {
                            tab.transfer_tasks.push(task);
                        }
                    }
                    SftpResponse::OperationDone(task) => {
                        let message = match task.direction {
                            super::models::TransferDirection::Upload => {
                                format!("上传完成: {} → {}", task.source, task.destination)
                            }
                            super::models::TransferDirection::Download => {
                                format!("下载完成: {} → {}", task.source, task.destination)
                            }
                        };
                        let direction = task.direction.clone();
                        let destination = task.destination.clone();
                        if let Some(existing) = tab
                            .transfer_tasks
                            .iter_mut()
                            .find(|existing| existing.id == task.id)
                        {
                            *existing = task;
                        } else {
                            tab.transfer_tasks.push(task);
                        }
                        tab.sftp_status_msg = message;
                        match direction {
                            super::models::TransferDirection::Upload => {
                                if remote_parent_path_for_file(&destination).is_some_and(|parent| {
                                    parent == normalize_remote_path(&tab.remote_current_dir)
                                }) {
                                    match tab.sftp_tx.as_ref().map(|tx| {
                                        tx.try_send(SftpRequest::ListDirectory(
                                            tab.remote_current_dir.clone(),
                                        ))
                                    }) {
                                        Some(Ok(())) => {}
                                        Some(Err(_)) | None => {
                                            tab.sftp_status_msg = format!(
                                                "{}；远程目录自动刷新失败",
                                                tab.sftp_status_msg
                                            );
                                        }
                                    }
                                }
                            }
                            super::models::TransferDirection::Download => {
                                if Path::new(&destination).parent()
                                    == Some(Path::new(&tab.local_current_dir))
                                {
                                    match sftp::list_local_dir(&tab.local_current_dir) {
                                        Ok(files) => {
                                            tab.local_files = files;
                                            tab.local_selected = None;
                                        }
                                        Err(error) => {
                                            tab.sftp_status_msg = format!(
                                                "{}；本地目录自动刷新失败: {}",
                                                tab.sftp_status_msg, error
                                            );
                                        }
                                    }
                                }
                            }
                        }
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
                        fail_unfinished_transfers(
                            &mut tab.transfer_tasks,
                            "SFTP 已断开，传输已取消",
                        );
                        tab.sftp_connected = false;
                        tab.sftp_tx = None;
                        tab.sftp_control_tx = None;
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
                    self.tabs[active_idx].active_tab = active_sub_tab.clone();
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
        let mut copy_terminal = false;
        let mut paste_terminal = false;
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
        let terminal_content_height = (available_height - TERMINAL_BOTTOM_PADDING).max(line_height);
        let new_rows = terminal_content_rows(available_height, line_height);

        // 调整终端大小
        let Some(tab) = self.current_tab_mut() else {
            return;
        };
        if let Some(term) = &mut tab.terminal {
            let (cur_cols, cur_rows) = term.size();
            if new_cols != cur_cols || new_rows != cur_rows {
                term.resize(new_cols, new_rows);
                tab.terminal_selection = None;
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
            ui.set_min_height(terminal_content_height);

            let layout = TerminalTextLayout {
                char_width,
                line_height,
                font_id: &mono_font,
                pixels_per_point,
            };

            let response = ui.add(
                egui::Label::new(job)
                    .sense(egui::Sense::click_and_drag())
                    .selectable(false),
            );
            let text_rect = response.rect;
            // 终端画布：行列固定的文本区域，滚动条与光标可见性都以它为准
            let terminal_canvas_rect = egui::Rect::from_min_size(
                text_rect.min,
                egui::vec2(f32::from(new_cols) * char_width, terminal_content_height),
            );

            if response.clicked_by(egui::PointerButton::Primary) {
                tab.terminal_selection = None;
            }
            if response.drag_started_by(egui::PointerButton::Primary)
                && let Some(pointer_pos) = response.interact_pointer_pos()
                && let Some(position) = ui.fonts(|fonts| {
                    terminal_position_from_pointer(
                        pointer_pos,
                        text_rect,
                        term,
                        fonts,
                        &layout,
                        new_cols,
                        new_rows,
                    )
                })
            {
                tab.terminal_selection =
                    Some(TerminalSelection::new(term.normalize_position(position)));
            }
            if response.dragged_by(egui::PointerButton::Primary)
                && let Some(pointer_pos) = response.interact_pointer_pos()
                && let Some(position) = ui.fonts(|fonts| {
                    terminal_position_from_pointer(
                        pointer_pos,
                        text_rect,
                        term,
                        fonts,
                        &layout,
                        new_cols,
                        new_rows,
                    )
                })
                && let Some(selection) = &mut tab.terminal_selection
            {
                selection.focus = term.normalize_position(position);
            }

            if let Some(selection) = tab.terminal_selection {
                let (_, end) = selection.normalized();
                paint_terminal_selection(
                    ui,
                    term,
                    &layout,
                    text_rect,
                    selection,
                    new_cols,
                    term.char_width_at(end),
                );
            }

            // 右侧滚动条：拖动可快速定位历史内容
            // 以终端画布右缘为基准，放在画布右侧预留的留白内，避免遮挡文字
            let scrollbar_track = terminal_scrollbar_track(terminal_canvas_rect);
            if let Some(new_offset) = paint_terminal_scrollbar(
                ui,
                scrollbar_track,
                term.scrollback(),
                term.scrollback_count(),
                new_rows,
            ) {
                term.set_scrollback(new_offset);
                tab.terminal_selection = None;
            }

            response.context_menu(|ui| {
                if ui.button("📋 复制").clicked() {
                    copy_terminal = true;
                    ui.close_menu();
                }
                if ui.button("📥 粘贴").clicked() {
                    paste_terminal = true;
                    ui.close_menu();
                }
                if ui.button("清除选择").clicked() {
                    tab.terminal_selection = None;
                    ui.close_menu();
                }
            });

            // 绘制闪烁光标（考虑滚动偏移）
            let (c_col, c_row) = term.cursor_position();
            let cursor_display_row = terminal_cursor_display_row(c_row, term.scrollback());
            // 光标列按渲染宽度计算，宽字符（中文）下与文字对齐
            let (cursor_col, _) = term.normalize_position((c_col, cursor_display_row));
            let (cursor_x, cursor_width) = ui.fonts(|fonts| {
                (
                    layout.column_offset(term, fonts, cursor_display_row, cursor_col),
                    layout.cell_width(term, fonts, cursor_display_row, cursor_col),
                )
            });
            let cursor_rect = terminal_cursor_rect(
                text_rect.min,
                cursor_x,
                cursor_display_row,
                cursor_width,
                line_height,
            );

            // 使用显式终端画布判断可见性，避免最后一行受文本 galley 浮点边界影响
            let cursor_in_view = terminal_cursor_in_view(cursor_rect, terminal_canvas_rect);
            if cursor_in_view {
                let time = ui.ctx().input(|i| i.time);
                let blink_on = (time * 2.0) as u64 % 2 == 0;
                if blink_on {
                    let cursor_color = if is_dark_mode {
                        Color32::from_rgb(0xd0, 0xd0, 0xd0)
                    } else {
                        Color32::from_rgb(0x30, 0x30, 0x30)
                    };
                    ui.painter()
                        .with_clip_rect(terminal_canvas_rect)
                        .rect_filled(cursor_rect, 0.0, cursor_color);
                }
            }

            // 设置 IME 输出位置
            let to_global = ui
                .ctx()
                .layer_transform_to_global(ui.layer_id())
                .unwrap_or_default();
            ui.ctx().output_mut(|o| {
                o.ime = Some(egui::output::IMEOutput {
                    rect: to_global * terminal_canvas_rect,
                    cursor_rect: to_global
                        * terminal_ime_cursor_rect(cursor_rect, terminal_canvas_rect),
                });
            });
        } else if tab.connection_state == SessionState::Connecting {
            ui.centered_and_justified(|ui| {
                ui.label("正在连接...");
            });
        }

        ui.add_space(TERMINAL_BOTTOM_PADDING);

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

        if copy_terminal {
            self.copy_terminal_text(ui.ctx());
        }
        if paste_terminal {
            self.request_terminal_paste(ui.ctx());
        }
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
                let mut local_dir_changed = false;
                ui.horizontal(|ui| {
                    let Some(tab) = self.current_tab_mut() else {
                        return;
                    };
                    let resp = ui.text_edit_singleline(&mut tab.local_dir_input);
                    if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        let p = tab.local_dir_input.trim().to_string();
                        if Path::new(&p).is_dir() {
                            tab.local_current_dir = p;
                            local_dir_changed = true;
                        } else {
                            tab.sftp_status_msg = format!("无效的本地目录: {}", p);
                            tab.local_dir_input = tab.local_current_dir.clone();
                        }
                    }
                    if ui.button("📂").clicked() {
                        if let Some(path) = rfd::FileDialog::new()
                            .set_title("选择本地目录")
                            .set_directory(&tab.local_current_dir)
                            .pick_folder()
                        {
                            tab.local_current_dir = path.to_string_lossy().to_string();
                            local_dir_changed = true;
                        }
                    }
                    if ui.button("⬆").clicked() {
                        if let Some(parent) = Path::new(&tab.local_current_dir).parent() {
                            tab.local_current_dir = parent.to_string_lossy().to_string();
                            local_dir_changed = true;
                        }
                    }
                    if ui.button("🏠").clicked() {
                        tab.local_current_dir = sftp::home_dir();
                        local_dir_changed = true;
                    }
                });
                if local_dir_changed {
                    self.refresh_local_files();
                }

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

                let upload_target = self
                    .current_tab()
                    .and_then(|t| t.local_selected.and_then(|i| t.local_files.get(i)))
                    .filter(|entry| !entry.is_dir)
                    .map(|entry| {
                        let remote_path = format!(
                            "{}/{}",
                            self.current_tab()
                                .map(|tab| tab.remote_current_dir.trim_end_matches('/'))
                                .unwrap_or_default(),
                            entry.name
                        );
                        (entry.path.clone(), remote_path)
                    });
                let can_upload = upload_target
                    .as_ref()
                    .map(|(source, destination)| {
                        !self.current_tab().is_some_and(|tab| {
                            has_active_transfer(&tab.transfer_tasks, source, destination)
                        })
                    })
                    .unwrap_or(false);

                if ui
                    .add_enabled(can_upload, egui::Button::new("上传 →"))
                    .clicked()
                {
                    if let Some((local_path, remote_path)) = upload_target {
                        let task = super::models::TransferTask::new(
                            local_path,
                            remote_path,
                            super::models::TransferDirection::Upload,
                            None,
                        );
                        self.enqueue_transfer(task);
                    }
                }

                ui.add_space(12.0);

                let download_target = self
                    .current_tab()
                    .and_then(|t| t.remote_selected.and_then(|i| t.remote_files.get(i)))
                    .filter(|entry| !entry.is_dir)
                    .map(|entry| {
                        let local_path = Path::new(
                            self.current_tab()
                                .map(|tab| tab.local_current_dir.as_str())
                                .unwrap_or_default(),
                        )
                        .join(&entry.name)
                        .to_string_lossy()
                        .to_string();
                        (
                            entry.path.clone(),
                            local_path,
                            entry.size_known.then_some(entry.size),
                        )
                    });
                let can_download = download_target
                    .as_ref()
                    .map(|(source, destination, _)| {
                        !self.current_tab().is_some_and(|tab| {
                            has_active_transfer(&tab.transfer_tasks, source, destination)
                        })
                    })
                    .unwrap_or(false);

                if ui
                    .add_enabled(can_download, egui::Button::new("← 下载"))
                    .clicked()
                {
                    if let Some((remote_path, local_path, total_size)) = download_target {
                        let task = super::models::TransferTask::new(
                            remote_path,
                            local_path,
                            super::models::TransferDirection::Download,
                            total_size,
                        );
                        self.enqueue_transfer(task);
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
                let mut remote_dir_to_load = None;
                ui.horizontal(|ui| {
                    let Some(tab) = self.current_tab_mut() else {
                        return;
                    };
                    let resp = ui.text_edit_singleline(&mut tab.remote_dir_input);
                    if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        let path = tab.remote_dir_input.trim().to_string();
                        if path.is_empty() {
                            tab.sftp_status_msg = "远程目录不能为空".to_string();
                            tab.remote_dir_input = tab.remote_current_dir.clone();
                        } else {
                            remote_dir_to_load = Some(path);
                        }
                    }
                    if ui.button("⬆").clicked() {
                        remote_dir_to_load = remote_parent_path(&tab.remote_current_dir);
                    }
                    if ui.button("🔄").clicked() {
                        remote_dir_to_load = Some(tab.remote_current_dir.clone());
                    }
                });
                if let Some(path) = remote_dir_to_load {
                    self.request_remote_directory(path);
                }

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
                            if let Some(path) = self
                                .current_tab()
                                .and_then(|tab| tab.remote_files.get(idx))
                                .map(|entry| entry.path.clone())
                            {
                                self.request_remote_directory(path);
                            }
                        }
                    });
            });
        });

        // ---- 下方：传输进度区（始终显示） ----
        ui.separator();
        ui.horizontal(|ui| {
            ui.strong("传输进度:");
            let has_completed = self
                .current_tab()
                .is_some_and(|tab| tab.transfer_tasks.iter().any(|task| task.done));
            if ui
                .add_enabled(has_completed, egui::Button::new("清除已完成"))
                .clicked()
            {
                if let Some(tab) = self.current_tab_mut() {
                    tab.transfer_tasks.retain(|task| !task.done);
                }
            }
        });
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
                    let mut task_to_cancel = None;
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
                            if !task.done && ui.small_button("取消").clicked() {
                                task_to_cancel = Some(task.id.clone());
                            }
                        });
                    }
                    if let Some(task_id) = task_to_cancel {
                        if let Some(tab) = self.current_tab_mut() {
                            match &tab.sftp_control_tx {
                                Some(tx)
                                    if tx.send(SftpControl::CancelTransfer(task_id)).is_ok() =>
                                {
                                    tab.sftp_status_msg = "已发送取消请求".to_string();
                                }
                                _ => {
                                    tab.sftp_status_msg =
                                        "无法发送取消请求，SFTP 控制通道已关闭".to_string();
                                }
                            }
                        }
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
        // Home/End 的序列形式取决于远端是否开启应用光标键模式（DECCKM）
        let application_cursor = self
            .current_tab()
            .and_then(|tab| tab.terminal.as_ref())
            .map(TerminalEmulator::application_cursor)
            .unwrap_or(false);
        let current_time = ctx.input(|i| i.time);
        let paste_deadline = self
            .current_tab()
            .and_then(|tab| tab.terminal_paste_deadline);
        let paste_was_pending = paste_deadline.is_some_and(|deadline| current_time <= deadline);
        let paste_request_expired = paste_deadline.is_some_and(|deadline| current_time > deadline);
        let mut clipboard_frame = TerminalClipboardFrame::default();

        ctx.input_mut(|i| {
            clipboard_frame = analyze_terminal_clipboard_events(&i.events);
            for event in i.events.clone() {
                match event {
                    egui::Event::Key {
                        key,
                        pressed,
                        modifiers,
                        ..
                    } => {
                        if modifiers.ctrl && key == egui::Key::C {
                            i.consume_key(modifiers, key);
                            continue;
                        }
                        if is_terminal_clipboard_shortcut(key, modifiers) {
                            i.consume_key(modifiers, key);
                            continue;
                        }
                        if !pressed {
                            continue;
                        }
                        if ime_active && !modifiers.ctrl {
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
                            // Ctrl+Backspace 删除光标前一个单词
                            let sequence = terminal_backspace_sequence(modifiers.ctrl);
                            let _ = tx.send(SshInput::KeyInput(sequence.to_vec()));
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
                        if let Some(sequence) = terminal_arrow_sequence(key, modifiers.ctrl) {
                            // Ctrl+左右箭头按单词移动
                            let _ = tx.send(SshInput::KeyInput(sequence.to_vec()));
                            i.consume_key(modifiers, key);
                            had_input = true;
                            continue;
                        }
                        if let Some(sequence) =
                            terminal_edit_key_sequence(key, modifiers.ctrl, application_cursor)
                        {
                            // Home/End 跳转行首行尾，Insert/Delete/PageUp/PageDown 发送对应序列
                            let _ = tx.send(SshInput::KeyInput(sequence.to_vec()));
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
                    egui::Event::Paste(_) => {}
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

        if clipboard_frame.copy_requested {
            self.copy_terminal_text(ctx);
        }
        if clipboard_frame.interrupt_requested {
            let _ = tx.send(SshInput::KeyInput(vec![0x03]));
            had_input = true;
        }
        let accepted_paste =
            accepted_terminal_paste_text(&clipboard_frame, paste_was_pending).map(str::to_owned);
        if let Some(text) = accepted_paste {
            match send_terminal_paste(tx, &text) {
                Ok(true) => {
                    had_input = true;
                    self.finish_terminal_paste();
                }
                Ok(false) => self.finish_empty_terminal_paste(),
                Err(error) => {
                    if let Some(tab) = self.current_tab_mut() {
                        tab.terminal_paste_deadline = None;
                        tab.status_msg = format!("粘贴失败: {}", error);
                    }
                    log::error!("发送终端粘贴文本失败: {}", error);
                }
            }
        } else if clipboard_frame.paste_requested {
            self.request_terminal_paste(ctx);
        } else if paste_request_expired {
            self.finish_empty_terminal_paste();
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

/// 终端文本排版度量
///
/// 光标、选区与鼠标定位共用同一套度量，保证与 egui 实际渲染一致
#[derive(Debug, Clone, Copy)]
struct TerminalTextLayout<'a> {
    /// 等宽字符宽度（已按像素舍入）
    char_width: f32,
    /// 行高（已按像素舍入）
    line_height: f32,
    /// 终端等宽字体
    font_id: &'a egui::FontId,
    /// 屏幕像素比例，用于与 epaint 一致的逐字形像素舍入
    pixels_per_point: f32,
}

impl TerminalTextLayout<'_> {
    /// 计算某行指定列左侧的实际渲染宽度
    ///
    /// 宽字符按其自身字形宽度推进、其后的延续单元格不产生宽度，
    /// 因此中文等宽字符不会累积位置偏差
    fn column_offset(
        &self,
        term: &TerminalEmulator,
        fonts: &egui::text::Fonts,
        row: u16,
        col: u16,
    ) -> f32 {
        self.column_span(term, fonts, row, col, col).0
    }

    /// 单次遍历计算某行两个列位置左侧的实际渲染宽度
    ///
    /// 返回 `(start_col 左侧宽度, end_col 左侧宽度)`，要求 `start_col <= end_col`；
    /// 选区按行取起止位置时只需遍历一次该行前缀
    fn column_span(
        &self,
        term: &TerminalEmulator,
        fonts: &egui::text::Fonts,
        row: u16,
        start_col: u16,
        end_col: u16,
    ) -> (f32, f32) {
        let mut start_offset = 0.0;
        let mut offset = 0.0;

        for col in 0..end_col {
            if col == start_col {
                start_offset = offset;
            }

            if let Some(text) = term.rendered_cell_text(row, col) {
                offset = terminal_text_width_from(offset, &text, fonts, self);
            }
        }

        if start_col >= end_col {
            start_offset = offset;
        }

        (start_offset, offset)
    }

    /// 计算某单元格的渲染宽度（宽字符后半单元格等不渲染内容的单元格为 0）
    fn cell_width(
        &self,
        term: &TerminalEmulator,
        fonts: &egui::text::Fonts,
        row: u16,
        col: u16,
    ) -> f32 {
        match term.rendered_cell_text(row, col) {
            Some(text) => terminal_text_width(&text, fonts, self),
            None => 0.0,
        }
    }
}

/// 按 epaint 的排版规则计算文本渲染宽度
///
/// 逐字符累加字形宽度，并在每个字形后按像素网格舍入，
/// 与 epaint 推进排版光标的方式保持一致
fn terminal_text_width(
    text: &str,
    fonts: &egui::text::Fonts,
    layout: &TerminalTextLayout<'_>,
) -> f32 {
    terminal_text_width_from(0.0, text, fonts, layout)
}

/// 在已有宽度基础上继续累加文本渲染宽度
fn terminal_text_width_from(
    mut width: f32,
    text: &str,
    fonts: &egui::text::Fonts,
    layout: &TerminalTextLayout<'_>,
) -> f32 {
    for c in text.chars() {
        width += fonts.glyph_width(layout.font_id, c);
        width = round_to_pixel(width, layout.pixels_per_point);
    }
    width
}

/// 按像素网格舍入（与 epaint 的 `round_to_pixel` 一致）
fn round_to_pixel(value: f32, pixels_per_point: f32) -> f32 {
    if pixels_per_point > 0.0 {
        (value * pixels_per_point).round() / pixels_per_point
    } else {
        value
    }
}

/// 根据鼠标位置计算终端单元格坐标
///
/// 列位置按该行实际渲染的字形宽度反查，宽字符（中文）不会造成列偏移
fn terminal_position_from_pointer(
    pointer_pos: egui::Pos2,
    text_rect: egui::Rect,
    term: &TerminalEmulator,
    fonts: &egui::text::Fonts,
    layout: &TerminalTextLayout<'_>,
    cols: u16,
    rows: u16,
) -> Option<(u16, u16)> {
    if layout.char_width <= 0.0 || layout.line_height <= 0.0 || cols == 0 || rows == 0 {
        return None;
    }

    let relative_x = (pointer_pos.x - text_rect.left()).max(0.0);
    let relative_y = (pointer_pos.y - text_rect.top()).max(0.0);
    let row = terminal_axis_position(relative_y, layout.line_height, rows);
    let col = terminal_column_from_x(term, fonts, layout, row, relative_x, cols);
    Some((col, row))
}

/// 按渲染宽度反查某行内的列号
///
/// 逐单元格累加渲染宽度，返回包含 `x` 的单元格；
/// `x` 超出该行渲染宽度时取最后一列
fn terminal_column_from_x(
    term: &TerminalEmulator,
    fonts: &egui::text::Fonts,
    layout: &TerminalTextLayout<'_>,
    row: u16,
    x: f32,
    cols: u16,
) -> u16 {
    if cols == 0 {
        return 0;
    }

    let mut offset = 0.0;

    for col in 0..cols {
        let cell_width = layout.cell_width(term, fonts, row, col);
        if x < offset + cell_width {
            return col;
        }
        offset += cell_width;
    }

    cols - 1
}

fn terminal_content_rows(available_height: f32, line_height: f32) -> u16 {
    if !available_height.is_finite() || !line_height.is_finite() || line_height <= 0.0 {
        return 1;
    }

    let usable_height = (available_height - TERMINAL_BOTTOM_PADDING).max(line_height);
    let mut low = 1_u16;
    let mut high = u16::MAX;
    while low < high {
        let middle = low + (high - low).saturating_add(1) / 2;
        if f32::from(middle) * line_height <= usable_height {
            low = middle;
        } else {
            high = middle - 1;
        }
    }
    low
}

/// 构建终端光标矩形
///
/// `x` 与 `cursor_width` 由实际渲染宽度计算，宽字符（中文）下与文字对齐
fn terminal_cursor_rect(
    text_origin: egui::Pos2,
    x: f32,
    row: u16,
    cursor_width: f32,
    line_height: f32,
) -> egui::Rect {
    let vertical_inset = 1.0_f32.min(line_height / 4.0);
    let cursor_height = (line_height - vertical_inset * 2.0).max(1.0);
    egui::Rect::from_min_size(
        egui::pos2(
            text_origin.x + x,
            text_origin.y + f32::from(row) * line_height + vertical_inset,
        ),
        egui::vec2(cursor_width.max(1.0), cursor_height),
    )
}

fn terminal_cursor_in_view(cursor_rect: egui::Rect, canvas_rect: egui::Rect) -> bool {
    const EDGE_TOLERANCE: f32 = 0.5;

    cursor_rect.left() >= canvas_rect.left() - EDGE_TOLERANCE
        && cursor_rect.right() <= canvas_rect.right() + EDGE_TOLERANCE
        && cursor_rect.top() >= canvas_rect.top() - EDGE_TOLERANCE
        && cursor_rect.bottom() <= canvas_rect.bottom() + EDGE_TOLERANCE
}

fn terminal_cursor_display_row(cursor_row: u16, scrollback: usize) -> u16 {
    let scrollback_rows = u16::try_from(scrollback).unwrap_or(u16::MAX);
    cursor_row.saturating_add(scrollback_rows)
}

fn terminal_ime_cursor_rect(cursor_rect: egui::Rect, canvas_rect: egui::Rect) -> egui::Rect {
    let cursor_size = egui::vec2(
        cursor_rect.width().min(canvas_rect.width()),
        cursor_rect.height().min(canvas_rect.height()),
    );
    let max_x = (canvas_rect.right() - cursor_size.x).max(canvas_rect.left());
    let max_y = (canvas_rect.bottom() - cursor_size.y).max(canvas_rect.top());
    let cursor_min = egui::pos2(
        cursor_rect.left().clamp(canvas_rect.left(), max_x),
        cursor_rect.top().clamp(canvas_rect.top(), max_y),
    );
    egui::Rect::from_min_size(cursor_min, cursor_size)
}

fn terminal_axis_position(relative: f32, cell_size: f32, cell_count: u16) -> u16 {
    if cell_count == 0 {
        return 0;
    }

    let mut low = 0;
    let mut high = cell_count;
    while low < high {
        let middle = low + (high - low) / 2;
        let boundary = f32::from(middle.saturating_add(1)) * cell_size;
        if relative < boundary {
            high = middle;
        } else {
            low = middle.saturating_add(1);
        }
    }
    low.min(cell_count - 1)
}

/// 计算终端滚动条轨道矩形
///
/// 以终端画布右缘为基准放在右侧留白内：画布宽度由终端列数决定，
/// 不随行内容宽度变化，因此滚动条位置稳定且不会遮挡正文
fn terminal_scrollbar_track(canvas: egui::Rect) -> egui::Rect {
    let left = canvas.right() + TERMINAL_SCROLLBAR_GAP;

    egui::Rect::from_min_max(
        egui::pos2(left, canvas.top() + TERMINAL_SCROLLBAR_TRACK_INSET),
        egui::pos2(
            left + TERMINAL_SCROLLBAR_WIDTH,
            canvas.bottom() - TERMINAL_BOTTOM_PADDING,
        ),
    )
}

/// 计算滚动条滑块矩形
///
/// 无历史内容或轨道过短（滑块最小高度无法容纳）时返回 `None`
fn terminal_scrollbar_handle(
    track: egui::Rect,
    offset: usize,
    max_offset: usize,
    visible_rows: u16,
) -> Option<egui::Rect> {
    if max_offset == 0
        || !track.height().is_finite()
        || track.height() < TERMINAL_SCROLLBAR_MIN_TRACK_HEIGHT
    {
        return None;
    }

    let (position, handle_ratio) = terminal_scrollbar_ratios(offset, max_offset, visible_rows);
    let min_height = TERMINAL_SCROLLBAR_MIN_HANDLE_HEIGHT.min(track.height());
    let handle_height = (track.height() * handle_ratio).clamp(min_height, track.height());
    let handle_top = track.top() + (track.height() - handle_height) * position;

    Some(egui::Rect::from_min_size(
        egui::pos2(track.left(), handle_top),
        egui::vec2(track.width(), handle_height),
    ))
}

/// 绘制终端滚动条并返回拖动/点击后的滚动偏移
///
/// 拖动滑块按位置比例定位，点击轨道空白处按一页滚动；无历史内容时不显示
fn paint_terminal_scrollbar(
    ui: &egui::Ui,
    track: egui::Rect,
    offset: usize,
    max_offset: usize,
    visible_rows: u16,
) -> Option<usize> {
    let handle_rect = terminal_scrollbar_handle(track, offset, max_offset, visible_rows)?;

    let response = ui.interact(
        track,
        egui::Id::new("ssh_terminal_scrollbar"),
        egui::Sense::click_and_drag(),
    );

    let visuals = ui.style().interact(&response);
    ui.painter().rect_filled(
        track,
        4.0,
        ui.visuals().widgets.inactive.bg_fill.gamma_multiply(0.25),
    );
    ui.painter().rect_filled(handle_rect, 4.0, visuals.bg_fill);

    // 只响应主键，避免右键拖动误改滚动位置
    if response.dragged_by(egui::PointerButton::Primary) {
        let pointer = response.interact_pointer_pos()?;
        // 让滑块中心跟随指针，拖动时不会跳变
        let travel = (track.height() - handle_rect.height()).max(1.0);
        let thumb_top = pointer.y - track.top() - handle_rect.height() * 0.5;
        let ratio = (thumb_top / travel).clamp(0.0, 1.0);
        return Some(terminal_scrollbar_offset(ratio, max_offset));
    }

    if response.clicked_by(egui::PointerButton::Primary) {
        let pointer = response.interact_pointer_pos()?;
        let page = page_scroll_lines(max_offset, visible_rows);

        if pointer.y < handle_rect.top() {
            // 点击滑块上方：向上翻页（查看更早内容）
            return Some(offset.saturating_add(page).min(max_offset));
        }
        if pointer.y > handle_rect.bottom() {
            // 点击滑块下方：向下翻页（返回较新内容）
            return Some(offset.saturating_sub(page));
        }
    }

    None
}

/// 计算点击滚动条空白处时滚动的行数（按可视行数折算，不超过可用历史）
fn page_scroll_lines(max_offset: usize, visible_rows: u16) -> usize {
    if max_offset == 0 {
        return 0;
    }

    let page = (f32::from(visible_rows) * TERMINAL_SCROLLBAR_PAGE_FRACTION).round();
    let page = lines_to_f32(max_offset).min(page.max(1.0));

    // 已裁剪到 1.0..=max_offset，转换不会丢失精度
    page as usize
}

/// 绘制终端选区
///
/// 每行的起止位置按该行实际渲染宽度计算，宽字符（中文）下与文字对齐；
/// 位置计算在字体锁内完成、绘制在锁外执行，避免在 `ui.fonts` 闭包内访问画布
fn paint_terminal_selection(
    ui: &egui::Ui,
    term: &TerminalEmulator,
    layout: &TerminalTextLayout<'_>,
    text_rect: egui::Rect,
    selection: TerminalSelection,
    cols: u16,
    end_char_width: u16,
) {
    if cols == 0 {
        return;
    }

    let ((start_col, start_row), (end_col, end_row)) = selection.normalized();

    // (行号, 起始 x, 结束 x)
    let row_spans: Vec<(u16, f32, f32)> = ui.fonts(|fonts| {
        let mut spans = Vec::with_capacity(usize::from(end_row.saturating_sub(start_row)) + 1);

        for row in start_row..=end_row {
            let row_start_col = if row == start_row { start_col } else { 0 };
            let row_end_col = if row == end_row { end_col } else { cols - 1 };
            let end_width = if row == end_row { end_char_width } else { 1 };
            let row_end_exclusive = row_end_col.saturating_add(end_width).min(cols);

            let (start_x, end_x) =
                layout.column_span(term, fonts, row, row_start_col, row_end_exclusive);
            spans.push((row, start_x, end_x));
        }

        spans
    });

    let base_color = ui.visuals().selection.bg_fill;
    let selection_color =
        Color32::from_rgba_unmultiplied(base_color.r(), base_color.g(), base_color.b(), 96);

    for (row, start_x, end_x) in row_spans {
        let min = egui::pos2(
            text_rect.left() + start_x,
            text_rect.top() + f32::from(row) * layout.line_height,
        );
        let max = egui::pos2(text_rect.left() + end_x, min.y + layout.line_height);
        ui.painter()
            .rect_filled(egui::Rect::from_min_max(min, max), 0.0, selection_color);
    }
}

fn format_session_addr(session: &super::models::SshSession) -> String {
    if session.port == 22 {
        format!("{}@{}", session.username, session.host)
    } else {
        format!("{}@{}:{}", session.username, session.host, session.port)
    }
}

fn has_active_transfer(
    tasks: &[super::models::TransferTask],
    source: &str,
    destination: &str,
) -> bool {
    tasks
        .iter()
        .any(|task| !task.done && task.source == source && task.destination == destination)
}

/// 获取 SFTP POSIX 路径的上级目录
fn remote_parent_path(path: &str) -> Option<String> {
    let normalized = normalize_remote_path(path);
    if normalized == "/" || normalized == "." {
        return None;
    }

    match normalized.rfind('/') {
        Some(0) => Some("/".to_string()),
        Some(index) => Some(normalized[..index].to_string()),
        None if normalized == ".." => Some("../..".to_string()),
        None => Some(".".to_string()),
    }
}

fn remote_parent_path_for_file(path: &str) -> Option<String> {
    let normalized = normalize_remote_path(path);
    match normalized.rfind('/') {
        Some(0) => Some("/".to_string()),
        Some(index) => Some(normalized[..index].to_string()),
        None => Some(".".to_string()),
    }
}

/// 对 SFTP POSIX 路径进行词法规范化
fn normalize_remote_path(path: &str) -> String {
    let trimmed = path.trim();
    let absolute = trimmed.starts_with('/');
    let mut components: Vec<&str> = Vec::new();

    for component in trimmed.split('/') {
        match component {
            "" | "." => {}
            ".." => match components.last() {
                Some(last) if *last != ".." => {
                    components.pop();
                }
                _ if !absolute => components.push(component),
                _ => {}
            },
            _ => components.push(component),
        }
    }

    if absolute {
        if components.is_empty() {
            "/".to_string()
        } else {
            format!("/{}", components.join("/"))
        }
    } else if components.is_empty() {
        ".".to_string()
    } else {
        components.join("/")
    }
}

/// 发送远程目录加载请求
fn send_remote_directory_request(
    tx: Option<&mpsc::SyncSender<SftpRequest>>,
    path: String,
) -> Result<(), String> {
    match tx {
        Some(tx) => match tx.try_send(SftpRequest::ListDirectory(path)) {
            Ok(()) => Ok(()),
            Err(mpsc::TrySendError::Full(_)) => Err("SFTP 请求队列已满，请稍后重试".to_string()),
            Err(mpsc::TrySendError::Disconnected(_)) => Err("无法发送远程目录刷新请求".to_string()),
        },
        None => Err("SFTP 未连接，无法刷新远程目录".to_string()),
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

#[cfg(test)]
mod tests {
    use std::sync::mpsc;

    use super::{
        DEFAULT_SCROLL_POINTS_PER_NOTCH, SshClientUi, TERMINAL_BACKSPACE_SEQUENCE,
        TERMINAL_BOTTOM_PADDING, TERMINAL_SCROLLBAR_GAP, TERMINAL_SCROLLBAR_MIN_HANDLE_HEIGHT,
        TERMINAL_SCROLLBAR_MIN_TRACK_HEIGHT, TERMINAL_SCROLLBAR_TRACK_INSET,
        TERMINAL_SCROLLBAR_WIDTH, TERMINAL_WHEEL_MAX_LINES, TERMINAL_WORD_DELETE_SEQUENCE,
        TabSwitchDirection, TerminalClipboardAction, TerminalEmulator, TerminalTextLayout,
        accepted_terminal_paste_text, analyze_terminal_clipboard_events, has_active_transfer,
        page_scroll_lines, paint_terminal_selection, remote_parent_path, round_to_pixel,
        send_remote_directory_request, send_terminal_paste, switched_tab_index,
        tab_switch_direction, tab_switch_shortcut_from_events, terminal_arrow_sequence,
        terminal_backspace_sequence, terminal_clipboard_action, terminal_content_rows,
        terminal_cursor_display_row, terminal_cursor_in_view, terminal_cursor_rect,
        terminal_edit_key_sequence, terminal_ime_cursor_rect, terminal_position_from_pointer,
        terminal_scroll_lines, terminal_scrollbar_handle, terminal_scrollbar_offset,
        terminal_scrollbar_ratios, terminal_scrollbar_track,
    };
    use crate::plugins::ssh_client::models::{
        SessionTab, SftpRequest, SshInput, TerminalSelection,
    };

    /// 在测试用 egui 上下文中读取字体度量
    fn with_terminal_fonts<R>(
        size: f32,
        reader: impl FnOnce(&egui::text::Fonts, &egui::FontId) -> R,
    ) -> R {
        let ctx = egui::Context::default();
        let font_id = egui::FontId::monospace(size);
        let mut result = None;
        let mut reader = Some(reader);

        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            let Some(reader) = reader.take() else {
                return;
            };

            ctx.fonts(|fonts| {
                result = Some(reader(fonts, &font_id));
            });
        });

        match result {
            Some(value) => value,
            None => panic!("未能读取测试字体度量"),
        }
    }

    /// 构造终端排版度量（等宽字符宽度取自实际字体）
    fn test_terminal_layout<'a>(
        fonts: &egui::text::Fonts,
        font_id: &'a egui::FontId,
        line_height: f32,
    ) -> TerminalTextLayout<'a> {
        TerminalTextLayout {
            char_width: fonts.glyph_width(font_id, 'M'),
            line_height,
            font_id,
            pixels_per_point: 1.0,
        }
    }

    /// 构造带指定修饰键的按键事件
    fn key_event(key: egui::Key, modifiers: egui::Modifiers) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: Some(key),
            pressed: true,
            repeat: false,
            modifiers,
        }
    }

    #[test]
    fn tab_switch_shortcut_requires_ctrl_tab() {
        let ctrl = egui::Modifiers {
            ctrl: true,
            ..Default::default()
        };
        let ctrl_shift = egui::Modifiers {
            ctrl: true,
            shift: true,
            ..Default::default()
        };

        assert_eq!(
            tab_switch_direction(egui::Key::Tab, ctrl),
            Some(TabSwitchDirection::Next)
        );
        assert_eq!(
            tab_switch_direction(egui::Key::Tab, ctrl_shift),
            Some(TabSwitchDirection::Previous)
        );

        // 单独的 Tab / Shift+Tab 仍交给终端处理，不做标签切换
        assert_eq!(
            tab_switch_direction(egui::Key::Tab, egui::Modifiers::NONE),
            None
        );
        assert_eq!(
            tab_switch_direction(
                egui::Key::Tab,
                egui::Modifiers {
                    shift: true,
                    ..Default::default()
                }
            ),
            None
        );
        assert_eq!(tab_switch_direction(egui::Key::A, ctrl), None);
    }

    #[test]
    fn switched_tab_index_wraps_around() {
        assert_eq!(switched_tab_index(0, 3, TabSwitchDirection::Next), Some(1));
        assert_eq!(switched_tab_index(2, 3, TabSwitchDirection::Next), Some(0));
        assert_eq!(
            switched_tab_index(0, 3, TabSwitchDirection::Previous),
            Some(2)
        );
        assert_eq!(
            switched_tab_index(1, 3, TabSwitchDirection::Previous),
            Some(0)
        );

        // 单标签时保持原索引
        assert_eq!(switched_tab_index(0, 1, TabSwitchDirection::Next), Some(0));

        // 索引越界时收敛到最后一个标签，再按方向循环切换
        assert_eq!(switched_tab_index(9, 2, TabSwitchDirection::Next), Some(0));
        assert_eq!(
            switched_tab_index(9, 2, TabSwitchDirection::Previous),
            Some(0)
        );

        assert_eq!(switched_tab_index(0, 0, TabSwitchDirection::Next), None);
    }

    #[test]
    fn terminal_arrows_send_word_movement_with_ctrl() {
        assert_eq!(
            terminal_arrow_sequence(egui::Key::ArrowLeft, false),
            Some(&b"\x1b[D"[..])
        );
        assert_eq!(
            terminal_arrow_sequence(egui::Key::ArrowRight, false),
            Some(&b"\x1b[C"[..])
        );
        assert_eq!(
            terminal_arrow_sequence(egui::Key::ArrowLeft, true),
            Some(&b"\x1b[1;5D"[..])
        );
        assert_eq!(
            terminal_arrow_sequence(egui::Key::ArrowRight, true),
            Some(&b"\x1b[1;5C"[..])
        );

        // Ctrl+上下箭头保持原有行为
        assert_eq!(
            terminal_arrow_sequence(egui::Key::ArrowUp, true),
            Some(&b"\x1b[A"[..])
        );
        assert_eq!(
            terminal_arrow_sequence(egui::Key::ArrowDown, true),
            Some(&b"\x1b[B"[..])
        );
        assert_eq!(terminal_arrow_sequence(egui::Key::A, true), None);
    }

    #[test]
    fn terminal_backspace_deletes_word_with_ctrl() {
        assert_eq!(
            terminal_backspace_sequence(false),
            TERMINAL_BACKSPACE_SEQUENCE
        );
        assert_eq!(
            terminal_backspace_sequence(true),
            TERMINAL_WORD_DELETE_SEQUENCE
        );
        assert_eq!(terminal_backspace_sequence(true), b"\x17");
    }

    #[test]
    fn terminal_home_end_follow_application_cursor_mode() {
        // 普通模式：xterm 的 CSI 形式，readline 类行编辑移动到行首/行尾
        assert_eq!(
            terminal_edit_key_sequence(egui::Key::Home, false, false),
            Some(&b"\x1b[H"[..])
        );
        assert_eq!(
            terminal_edit_key_sequence(egui::Key::End, false, false),
            Some(&b"\x1b[F"[..])
        );

        // 应用光标键模式（DECCKM）：SS3 形式，与 vim/less 的 terminfo 一致
        assert_eq!(
            terminal_edit_key_sequence(egui::Key::Home, false, true),
            Some(&b"\x1bOH"[..])
        );
        assert_eq!(
            terminal_edit_key_sequence(egui::Key::End, false, true),
            Some(&b"\x1bOF"[..])
        );

        // 带 Ctrl 修饰键时统一为 CSI 1;5 形式，不受 DECCKM 影响
        assert_eq!(
            terminal_edit_key_sequence(egui::Key::Home, true, false),
            Some(&b"\x1b[1;5H"[..])
        );
        assert_eq!(
            terminal_edit_key_sequence(egui::Key::End, true, true),
            Some(&b"\x1b[1;5F"[..])
        );
    }

    #[test]
    fn terminal_edit_keys_send_xterm_sequences() {
        for (key, expected) in [
            (egui::Key::Insert, &b"\x1b[2~"[..]),
            (egui::Key::Delete, &b"\x1b[3~"[..]),
            (egui::Key::PageUp, &b"\x1b[5~"[..]),
            (egui::Key::PageDown, &b"\x1b[6~"[..]),
        ] {
            assert_eq!(
                terminal_edit_key_sequence(key, false, false),
                Some(expected),
                "{key:?} 应发送 xterm 的 CSI n ~ 序列"
            );
            // 该形式不受应用光标键模式影响
            assert_eq!(terminal_edit_key_sequence(key, false, true), Some(expected));
        }

        // 带 Ctrl 的组合返回 None；Ctrl+Insert / Shift+Insert / Shift+Delete 在平台层
        // 已转换为系统 Copy/Paste/Cut 事件，不会以 Key 事件形式到达这里
        assert_eq!(
            terminal_edit_key_sequence(egui::Key::Insert, true, false),
            None
        );
        assert_eq!(
            terminal_edit_key_sequence(egui::Key::Delete, true, false),
            None
        );
        assert_eq!(
            terminal_edit_key_sequence(egui::Key::PageUp, true, false),
            None
        );
        // 其他按键不产生序列
        assert_eq!(terminal_edit_key_sequence(egui::Key::A, false, false), None);
    }

    /// 把按键送入真实终端输入管线，返回发送到远端的字节序列
    fn forward_terminal_keys(
        ssh_ui: &mut SshClientUi,
        tx: &mpsc::SyncSender<SshInput>,
        rx: &mpsc::Receiver<SshInput>,
        keys: &[egui::Key],
    ) -> Vec<Vec<u8>> {
        let ctx = egui::Context::default();
        let mut raw_input = egui::RawInput::default();
        for key in keys {
            raw_input
                .events
                .push(key_event(*key, egui::Modifiers::NONE));
        }

        let mut had_input = false;
        let _ = ctx.run(raw_input, |ctx| {
            had_input = ssh_ui.process_terminal_input(tx, ctx);
        });
        assert!(had_input, "编辑键应被识别为终端输入");

        let mut sent = Vec::new();
        while let Ok(SshInput::KeyInput(bytes)) = rx.try_recv() {
            sent.push(bytes);
        }
        sent
    }

    #[test]
    fn terminal_home_end_are_forwarded_to_remote() {
        let (tx, rx) = mpsc::sync_channel(16);
        let mut ssh_ui = SshClientUi::new();
        ssh_ui.tabs.push(SessionTab::new(1, "测试".to_string()));
        ssh_ui.active_tab_index = Some(0);

        assert_eq!(
            forward_terminal_keys(&mut ssh_ui, &tx, &rx, &[egui::Key::Home, egui::Key::End]),
            vec![b"\x1b[H".to_vec(), b"\x1b[F".to_vec()],
            "普通模式下 Home/End 应发送 CSI 形式的行首行尾序列"
        );

        // 远端开启应用光标键模式（DECCKM）后改用 SS3 形式
        let mut terminal = TerminalEmulator::new(80, 24, 14.0);
        terminal.process(b"\x1b[?1h");
        if let Some(tab) = ssh_ui.current_tab_mut() {
            tab.terminal = Some(terminal);
        }

        assert_eq!(
            forward_terminal_keys(&mut ssh_ui, &tx, &rx, &[egui::Key::Home, egui::Key::End]),
            vec![b"\x1bOH".to_vec(), b"\x1bOF".to_vec()],
            "应用光标键模式下 Home/End 应发送 SS3 形式"
        );
    }

    #[test]
    fn tab_switch_listener_ignores_other_keys() {
        // 只有 Ctrl+Tab 事件会被识别为标签切换
        let ctrl = egui::Modifiers {
            ctrl: true,
            ..Default::default()
        };
        let ctrl_shift = egui::Modifiers {
            ctrl: true,
            shift: true,
            ..Default::default()
        };

        let events = vec![
            key_event(egui::Key::A, ctrl),
            key_event(egui::Key::Tab, egui::Modifiers::NONE),
            key_event(egui::Key::Tab, ctrl),
        ];
        assert_eq!(
            tab_switch_shortcut_from_events(&events),
            Some((ctrl, TabSwitchDirection::Next))
        );

        let events = vec![key_event(egui::Key::Tab, ctrl_shift)];
        assert_eq!(
            tab_switch_shortcut_from_events(&events),
            Some((ctrl_shift, TabSwitchDirection::Previous))
        );

        // 没有 Ctrl+Tab 时不应触发标签切换
        let events = vec![
            key_event(egui::Key::Tab, egui::Modifiers::NONE),
            key_event(egui::Key::Enter, ctrl),
        ];
        assert_eq!(tab_switch_shortcut_from_events(&events), None);
    }

    #[test]
    fn tab_switch_shortcut_ignores_release_and_accepts_repeat() {
        let ctrl = egui::Modifiers {
            ctrl: true,
            ..Default::default()
        };

        // 松开按键不应触发切换
        let released = vec![egui::Event::Key {
            key: egui::Key::Tab,
            physical_key: Some(egui::Key::Tab),
            pressed: false,
            repeat: false,
            modifiers: ctrl,
        }];
        assert_eq!(tab_switch_shortcut_from_events(&released), None);

        // 长按产生的重复事件同样触发切换（连续翻页）
        let repeated = vec![egui::Event::Key {
            key: egui::Key::Tab,
            physical_key: Some(egui::Key::Tab),
            pressed: true,
            repeat: true,
            modifiers: ctrl,
        }];
        assert_eq!(
            tab_switch_shortcut_from_events(&repeated),
            Some((ctrl, TabSwitchDirection::Next))
        );
    }

    #[test]
    fn tab_switch_shortcut_is_consumed_before_terminal_input() {
        let ctx = egui::Context::default();
        let ctrl = egui::Modifiers {
            ctrl: true,
            ..Default::default()
        };

        let mut raw_input = egui::RawInput::default();
        raw_input.events.push(key_event(egui::Key::Tab, ctrl));
        raw_input
            .events
            .push(key_event(egui::Key::A, egui::Modifiers::NONE));

        let mut tab_removed = false;
        let mut remaining_events = 0;
        let _ = ctx.run(raw_input, |ctx| {
            // 与 handle_tab_switch_shortcut 中相同的消费方式
            ctx.input_mut(|i| i.consume_key(ctrl, egui::Key::Tab));
            ctx.input(|i| {
                tab_removed = !i.events.iter().any(|event| {
                    matches!(
                        event,
                        egui::Event::Key {
                            key: egui::Key::Tab,
                            ..
                        }
                    )
                });
                remaining_events = i.events.len();
            });
        });

        assert!(tab_removed, "Ctrl+Tab 应被消费，不再发送到远端");
        assert_eq!(remaining_events, 1, "其他按键事件不应被一并消费");
    }

    fn clipboard_key_event(key: egui::Key, pressed: bool, repeat: bool) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: Some(key),
            pressed,
            repeat,
            modifiers: egui::Modifiers {
                ctrl: true,
                shift: true,
                ..Default::default()
            },
        }
    }

    #[test]
    fn terminal_clipboard_shortcuts_require_ctrl_and_shift() {
        let clipboard_modifiers = egui::Modifiers {
            ctrl: true,
            shift: true,
            ..Default::default()
        };
        assert_eq!(
            terminal_clipboard_action(egui::Key::C, true, false, clipboard_modifiers),
            Some(TerminalClipboardAction::Copy)
        );
        assert_eq!(
            terminal_clipboard_action(egui::Key::V, true, false, clipboard_modifiers),
            Some(TerminalClipboardAction::Paste)
        );

        let interrupt_modifiers = egui::Modifiers {
            ctrl: true,
            ..Default::default()
        };
        assert_eq!(
            terminal_clipboard_action(egui::Key::C, true, false, interrupt_modifiers),
            None
        );
        assert_eq!(
            terminal_clipboard_action(egui::Key::V, false, false, clipboard_modifiers),
            None
        );
        assert_eq!(
            terminal_clipboard_action(egui::Key::V, true, true, clipboard_modifiers),
            None
        );
    }

    #[test]
    fn terminal_clipboard_frame_deduplicates_key_and_paste_events() {
        let events = vec![
            clipboard_key_event(egui::Key::V, true, false),
            egui::Event::Paste("first".to_string()),
            egui::Event::Paste("second".to_string()),
        ];
        let frame = analyze_terminal_clipboard_events(&events);

        assert!(frame.paste_requested);
        assert_eq!(frame.paste_text.as_deref(), Some("first"));

        let request_only =
            analyze_terminal_clipboard_events(&[clipboard_key_event(egui::Key::V, true, false)]);
        assert!(request_only.paste_requested);
        assert!(request_only.paste_text.is_none());
    }

    #[test]
    fn terminal_clipboard_frame_accepts_pending_async_paste_once() {
        for event in [
            clipboard_key_event(egui::Key::V, true, true),
            clipboard_key_event(egui::Key::V, false, false),
        ] {
            let frame = analyze_terminal_clipboard_events(&[
                event,
                egui::Event::Paste("duplicate".to_string()),
            ]);
            assert!(!frame.paste_requested);
            assert_eq!(
                accepted_terminal_paste_text(&frame, true),
                Some("duplicate")
            );
            assert_eq!(accepted_terminal_paste_text(&frame, false), None);
        }

        let standalone =
            analyze_terminal_clipboard_events(&[egui::Event::Paste("menu".to_string())]);
        assert_eq!(accepted_terminal_paste_text(&standalone, false), None);
        assert_eq!(
            accepted_terminal_paste_text(&standalone, true),
            Some("menu")
        );
    }

    #[test]
    fn terminal_native_clipboard_events_preserve_interrupt_and_shift_shortcuts() {
        let native_copy = analyze_terminal_clipboard_events(&[egui::Event::Copy]);
        if cfg!(target_os = "windows") {
            assert!(native_copy.interrupt_requested);
            assert!(!native_copy.copy_requested);
        } else {
            assert!(native_copy.copy_requested);
            assert!(!native_copy.interrupt_requested);
        }

        let ordinary_paste =
            analyze_terminal_clipboard_events(&[egui::Event::Paste("普通粘贴".to_string())]);
        assert_eq!(accepted_terminal_paste_text(&ordinary_paste, false), None);

        let shifted_paste = analyze_terminal_clipboard_events(&[
            clipboard_key_event(egui::Key::V, true, false),
            egui::Event::Paste("快捷键粘贴".to_string()),
        ]);
        assert_eq!(
            accepted_terminal_paste_text(&shifted_paste, false),
            Some("快捷键粘贴")
        );
    }

    #[test]
    fn empty_terminal_paste_finishes_pending_state() {
        let mut ssh_ui = SshClientUi::new();
        ssh_ui.tabs.push(SessionTab::new(1, "测试".to_string()));
        ssh_ui.active_tab_index = Some(0);
        let ctx = egui::Context::default();

        ssh_ui.request_terminal_paste(&ctx);
        let Some(pending_tab) = ssh_ui.current_tab() else {
            panic!("测试标签应存在");
        };
        assert!(pending_tab.terminal_paste_deadline.is_some());

        ssh_ui.finish_empty_terminal_paste();
        let Some(finished_tab) = ssh_ui.current_tab() else {
            panic!("测试标签应存在");
        };
        assert!(finished_tab.terminal_paste_deadline.is_none());
        assert_eq!(finished_tab.status_msg, "剪贴板为空，未执行粘贴");
    }

    #[test]
    fn terminal_paste_sends_non_empty_text_once() {
        let (tx, rx) = mpsc::sync_channel(1);

        assert_eq!(send_terminal_paste(&tx, "命令\r"), Ok(true));
        match rx.try_recv() {
            Ok(SshInput::KeyInput(bytes)) => assert_eq!(bytes, "命令\r".as_bytes()),
            result => panic!("收到非预期终端输入: {:?}", result),
        }
        assert!(rx.try_recv().is_err());
        assert_eq!(send_terminal_paste(&tx, ""), Ok(false));
    }

    #[test]
    fn terminal_paste_reports_full_and_closed_channels() {
        let (full_tx, _full_rx) = mpsc::sync_channel(1);
        assert!(full_tx.try_send(SshInput::Resize(80, 24)).is_ok());
        assert!(send_terminal_paste(&full_tx, "text").is_err());

        let (closed_tx, closed_rx) = mpsc::sync_channel(1);
        drop(closed_rx);
        assert!(send_terminal_paste(&closed_tx, "text").is_err());
    }

    #[test]
    fn round_to_pixel_matches_epaint_rounding() {
        assert_eq!(round_to_pixel(10.4, 1.0), 10.0);
        assert_eq!(round_to_pixel(10.6, 1.0), 11.0);
        assert_eq!(round_to_pixel(10.25, 2.0), 10.5);
        // 非法的像素比例下保持原值
        assert_eq!(round_to_pixel(10.4, 0.0), 10.4);
    }

    #[test]
    fn terminal_pointer_position_maps_and_clamps_to_cells() {
        let mut term = TerminalEmulator::new(8, 3, 14.0);
        term.process(b"abc");
        let rect = egui::Rect::from_min_size(egui::pos2(10.0, 20.0), egui::vec2(80.0, 60.0));

        with_terminal_fonts(14.0, |fonts, font_id| {
            let layout = test_terminal_layout(fonts, font_id, 20.0);

            // 落在第 3 个单元格内部时映射到第 2 列
            let cell_start = layout.column_offset(&term, fonts, 0, 2);
            let cell_end = layout.column_offset(&term, fonts, 0, 3);
            let inside_third_cell = rect.left() + (cell_start + cell_end) / 2.0;
            assert_eq!(
                terminal_position_from_pointer(
                    egui::pos2(inside_third_cell, rect.top() + 25.0),
                    rect,
                    &term,
                    fonts,
                    &layout,
                    8,
                    3,
                ),
                Some((2, 1))
            );

            // 左上角之外钳制到 (0, 0)
            assert_eq!(
                terminal_position_from_pointer(
                    egui::pos2(-100.0, -100.0),
                    rect,
                    &term,
                    fonts,
                    &layout,
                    8,
                    3,
                ),
                Some((0, 0))
            );

            // 右下角之外钳制到最后一行最后一列
            assert_eq!(
                terminal_position_from_pointer(
                    egui::pos2(5000.0, 5000.0),
                    rect,
                    &term,
                    fonts,
                    &layout,
                    8,
                    3,
                ),
                Some((7, 2))
            );
        });
    }

    #[test]
    fn terminal_cjk_columns_follow_rendered_glyph_width() {
        // 两个汉字 + 两个半角字符：宽字符占用两个单元格
        let mut term = TerminalEmulator::new(10, 2, 14.0);
        term.process("中文ab".as_bytes());

        with_terminal_fonts(14.0, |fonts, font_id| {
            let layout = test_terminal_layout(fonts, font_id, 20.0);

            let first_wide = layout.cell_width(&term, fonts, 0, 0);
            let second_wide = layout.cell_width(&term, fonts, 0, 2);

            // 第二个汉字左侧的位置等于第一个汉字的真实渲染宽度，
            // 而不是「2 × 等宽字符宽度」的估算值
            assert_eq!(layout.column_offset(&term, fonts, 0, 2), first_wide);
            assert_eq!(
                layout.column_offset(&term, fonts, 0, 4),
                first_wide + second_wide
            );

            // 宽字符后半单元格不产生宽度，与字符起点共用位置
            assert_eq!(layout.cell_width(&term, fonts, 0, 1), 0.0);
            assert_eq!(
                layout.column_offset(&term, fonts, 0, 2),
                layout.column_offset(&term, fonts, 0, 1)
            );

            // 鼠标落在第二个汉字内部时映射到该汉字起始列
            let inside_second_char = layout.column_offset(&term, fonts, 0, 2) + first_wide / 2.0;
            let rect = egui::Rect::from_min_size(egui::pos2(10.0, 20.0), egui::vec2(80.0, 60.0));
            assert_eq!(
                terminal_position_from_pointer(
                    egui::pos2(rect.left() + inside_second_char, rect.top() + 1.0),
                    rect,
                    &term,
                    fonts,
                    &layout,
                    10,
                    2,
                ),
                Some((2, 0))
            );
        });
    }

    #[test]
    fn terminal_cjk_cursor_offset_uses_real_glyph_width() {
        // 使用应用实际加载的中文字体，复现「光标比中文更偏右」的场景
        let ctx = egui::Context::default();
        crate::app::setup_chinese_fonts(&ctx);

        let mut term = TerminalEmulator::new(6, 2, 14.0);
        term.process("中a".as_bytes());
        let font_id = egui::FontId::monospace(14.0);

        let mut measured = None;
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            ctx.fonts(|fonts| {
                let layout = test_terminal_layout(fonts, &font_id, 20.0);

                // 与 epaint 实际排版对照：屏幕第二行的第二个字形（半角 a）的 x 位置
                let job = term.render_to_layout_job(true);
                let galley = fonts.layout_job(job);
                let laid_out_x = galley
                    .rows
                    .first()
                    .and_then(|row| row.glyphs.get(1))
                    .map(|glyph| glyph.pos.x);

                measured = Some((
                    // 汉字之后的光标位置
                    layout.column_offset(&term, fonts, 0, 2),
                    laid_out_x,
                    // 汉字字形宽度与等宽字符宽度
                    fonts.glyph_width(&font_id, '中'),
                    layout.char_width,
                ));
            });
        });

        let Some((offset, laid_out_x, cjk_width, latin_width)) = measured else {
            panic!("未能读取测试字体度量");
        };

        // 计算出的列位置与 egui 实际排版出的字形位置一致
        assert_eq!(Some(round_to_pixel(offset, 1.0)), laid_out_x);
        // 光标位置等于汉字按像素舍入后的真实渲染宽度（与 epaint 的排版推进一致）
        assert_eq!(offset, round_to_pixel(cjk_width, 1.0));
        // 根因：汉字宽度并非两个等宽字符宽度，按「列号 × 等宽字符宽度」估算会持续偏右
        assert!(
            cjk_width < latin_width * 2.0,
            "汉字宽度 {} 应小于两倍等宽字符宽度 {}",
            cjk_width,
            latin_width * 2.0
        );
    }

    #[test]
    fn terminal_wheel_scrolls_multiple_lines_per_notch() {
        // 一格滚轮 = 40 点（egui 原生默认），滚动 3 行
        assert_eq!(
            terminal_scroll_lines(40.0, DEFAULT_SCROLL_POINTS_PER_NOTCH),
            3
        );
        assert_eq!(
            terminal_scroll_lines(-40.0, DEFAULT_SCROLL_POINTS_PER_NOTCH),
            3
        );
        // 触控板小幅滚动至少 1 行
        assert_eq!(
            terminal_scroll_lines(8.0, DEFAULT_SCROLL_POINTS_PER_NOTCH),
            1
        );
        assert_eq!(
            terminal_scroll_lines(-1.0, DEFAULT_SCROLL_POINTS_PER_NOTCH),
            1
        );
        // 按滚动距离等比换算
        assert_eq!(
            terminal_scroll_lines(120.0, DEFAULT_SCROLL_POINTS_PER_NOTCH),
            9
        );
        // 惯性滚动不超过上限
        assert_eq!(
            terminal_scroll_lines(100_000.0, DEFAULT_SCROLL_POINTS_PER_NOTCH),
            TERMINAL_WHEEL_MAX_LINES as usize
        );
        // 滚轮速度可由 egui 配置改变
        assert_eq!(terminal_scroll_lines(80.0, 80.0), 3);
        // 非法的每格距离回退到默认值
        assert_eq!(terminal_scroll_lines(40.0, 0.0), 3);
        assert_eq!(terminal_scroll_lines(40.0, f32::NAN), 3);
        // 无滚动与非法值
        assert_eq!(
            terminal_scroll_lines(0.0, DEFAULT_SCROLL_POINTS_PER_NOTCH),
            0
        );
        assert_eq!(
            terminal_scroll_lines(f32::NAN, DEFAULT_SCROLL_POINTS_PER_NOTCH),
            0
        );
        assert_eq!(
            terminal_scroll_lines(f32::INFINITY, DEFAULT_SCROLL_POINTS_PER_NOTCH),
            0
        );
    }

    #[test]
    fn terminal_scrollbar_track_stays_right_of_canvas() {
        let canvas = egui::Rect::from_min_size(egui::pos2(10.0, 20.0), egui::vec2(800.0, 400.0));
        let track = terminal_scrollbar_track(canvas);

        // 轨道位于终端画布右侧，宽度固定
        assert_eq!(track.width(), TERMINAL_SCROLLBAR_WIDTH);
        assert_eq!(track.left(), canvas.right() + TERMINAL_SCROLLBAR_GAP);
        // 轨道上下留出内缩与底部间距，不超出画布
        assert_eq!(track.top(), canvas.top() + TERMINAL_SCROLLBAR_TRACK_INSET);
        assert_eq!(track.bottom(), canvas.bottom() - TERMINAL_BOTTOM_PADDING);
    }

    #[test]
    fn terminal_scrollbar_handle_geometry() {
        let track = egui::Rect::from_min_size(egui::pos2(100.0, 50.0), egui::vec2(8.0, 200.0));

        // 无历史内容时不显示
        assert_eq!(terminal_scrollbar_handle(track, 0, 0, 20), None);
        // 轨道过短时不显示（滑块最小高度无法容纳，且不能触发 clamp panic）
        let short_track = egui::Rect::from_min_size(egui::pos2(100.0, 50.0), egui::vec2(8.0, 8.0));
        assert_eq!(terminal_scrollbar_handle(short_track, 0, 500, 20), None);
        // 高度非法时同样不显示
        let invalid_track =
            egui::Rect::from_min_size(egui::pos2(100.0, 50.0), egui::vec2(8.0, f32::NAN));
        assert_eq!(terminal_scrollbar_handle(invalid_track, 0, 500, 20), None);
        // 恰好达到最小轨道高度时正常显示
        let min_track = egui::Rect::from_min_size(
            egui::pos2(100.0, 50.0),
            egui::vec2(8.0, TERMINAL_SCROLLBAR_MIN_TRACK_HEIGHT),
        );
        assert!(terminal_scrollbar_handle(min_track, 0, 500, 20).is_some());

        // 当前屏幕：滑块贴底
        let bottom = terminal_scrollbar_handle(track, 0, 500, 20);
        let Some(bottom) = bottom else {
            panic!("应显示滚动条");
        };
        assert_eq!(bottom.bottom(), track.bottom());
        assert_eq!(bottom.left(), track.left());
        assert_eq!(bottom.width(), track.width());

        // 最早历史：滑块贴顶
        let top = terminal_scrollbar_handle(track, 500, 500, 20);
        let Some(top) = top else {
            panic!("应显示滚动条");
        };
        assert_eq!(top.top(), track.top());

        // 历史很长时滑块高度按比例缩小
        let handle = terminal_scrollbar_handle(track, 0, 10_000, 20);
        let Some(handle) = handle else {
            panic!("应显示滚动条");
        };
        assert_eq!(handle.height(), 16.0);

        // 比例缩到最小高度以下时使用下限
        let medium_track =
            egui::Rect::from_min_size(egui::pos2(100.0, 50.0), egui::vec2(8.0, 30.0));
        let handle = terminal_scrollbar_handle(medium_track, 0, 10_000, 20);
        let Some(handle) = handle else {
            panic!("应显示滚动条");
        };
        assert_eq!(handle.height(), TERMINAL_SCROLLBAR_MIN_HANDLE_HEIGHT);
    }

    #[test]
    fn terminal_scrollbar_ratios_follow_scroll_offset() {
        // 可视 20 行、历史 100 行：滑块占轨道 1/6
        let handle_ratio = 20.0 / 120.0;
        // 当前屏幕：滑块位于底部
        assert_eq!(terminal_scrollbar_ratios(0, 100, 20), (1.0, handle_ratio));
        // 最早历史：滑块位于顶部
        assert_eq!(terminal_scrollbar_ratios(100, 100, 20), (0.0, handle_ratio));
        // 中间位置：滑块居中
        let (position, handle) = terminal_scrollbar_ratios(50, 100, 20);
        assert!((position - 0.5).abs() < 0.001);
        assert!((handle - handle_ratio).abs() < 0.001);
        // 偏移越界时按上限处理
        assert_eq!(terminal_scrollbar_ratios(999, 100, 20), (0.0, handle_ratio));
        // 无历史时不显示
        assert_eq!(terminal_scrollbar_ratios(0, 0, 20), (1.0, 1.0));
        // 滑块高度有下限，避免历史很长时过小
        let (_, tiny_handle) = terminal_scrollbar_ratios(0, 10_000, 20);
        assert!((tiny_handle - 0.08).abs() < 0.001);
    }

    #[test]
    fn terminal_scrollbar_offset_maps_ratio_to_scrollback() {
        // 滑块在底部对应滚动偏移 0，顶部对应最早历史
        assert_eq!(terminal_scrollbar_offset(1.0, 500), 0);
        assert_eq!(terminal_scrollbar_offset(0.0, 500), 500);
        assert_eq!(terminal_scrollbar_offset(0.5, 500), 250);
        // 越界与非法值收敛
        assert_eq!(terminal_scrollbar_offset(-3.0, 500), 500);
        assert_eq!(terminal_scrollbar_offset(9.0, 500), 0);
        assert_eq!(terminal_scrollbar_offset(f32::NAN, 500), 0);
        assert_eq!(terminal_scrollbar_offset(0.5, 0), 0);
    }

    #[test]
    fn terminal_scrollbar_page_scrolls_visible_rows() {
        assert_eq!(page_scroll_lines(1000, 20), 18);
        // 历史不足一页时按可用行数滚动
        assert_eq!(page_scroll_lines(5, 20), 5);
        // 至少滚动 1 行
        assert_eq!(page_scroll_lines(1, 20), 1);
    }

    #[test]
    fn terminal_selection_paints_inside_real_frame() {
        let ctx = egui::Context::default();
        let mut term = TerminalEmulator::new(10, 2, 14.0);
        term.process("中文ab".as_bytes());

        let mut selection = TerminalSelection::new((0, 0));
        selection.focus = (4, 0);

        // 选区绘制内部会读取字体度量，若在字体锁内访问画布会造成界面卡死；
        // 该用例走完整帧，验证绘制路径能够正常完成
        let output = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let font_id = egui::FontId::monospace(14.0);
                let char_width = ui.fonts(|fonts| fonts.glyph_width(&font_id, 'M'));
                let layout = TerminalTextLayout {
                    char_width,
                    line_height: 20.0,
                    font_id: &font_id,
                    pixels_per_point: ui.pixels_per_point(),
                };

                paint_terminal_selection(
                    ui,
                    &term,
                    &layout,
                    egui::Rect::from_min_size(egui::pos2(10.0, 20.0), egui::vec2(200.0, 40.0)),
                    selection,
                    10,
                    1,
                );
            });
        });

        assert!(!output.shapes.is_empty(), "选区应绘制出矩形");
    }

    #[test]
    fn terminal_blank_cells_keep_monospace_column_positions() {
        let mut term = TerminalEmulator::new(8, 1, 14.0);
        term.process(b"ab");

        with_terminal_fonts(14.0, |fonts, font_id| {
            let layout = test_terminal_layout(fonts, font_id, 20.0);
            let space_width = layout.cell_width(&term, fonts, 0, 7);

            // 行尾空白单元格仍按一个等宽字符占位
            assert!(space_width > 0.0);
            assert_eq!(
                layout.column_offset(&term, fonts, 0, 8),
                layout.column_offset(&term, fonts, 0, 2) + space_width * 6.0
            );
        });
    }

    #[test]
    fn terminal_rows_reserve_bottom_cursor_padding() {
        assert_eq!(terminal_content_rows(65.0, 20.0), 2);
        assert_eq!(terminal_content_rows(66.0, 20.0), 3);
        assert_eq!(terminal_content_rows(5.0, 20.0), 1);
        assert_eq!(terminal_content_rows(f32::NAN, 20.0), 1);

        let rows = terminal_content_rows(105.0, 20.0);
        assert!(f32::from(rows) * 20.0 <= 105.0 - TERMINAL_BOTTOM_PADDING);
    }

    #[test]
    fn terminal_last_row_cursor_stays_inside_explicit_canvas() {
        let canvas = egui::Rect::from_min_size(egui::pos2(10.0, 20.0), egui::vec2(80.0, 100.0));
        let cursor = terminal_cursor_rect(canvas.min, 0.0, 4, 10.0, 20.0);

        assert_eq!(cursor.top(), 101.0);
        assert_eq!(cursor.bottom(), 119.0);
        assert!(terminal_cursor_in_view(cursor, canvas));

        let below_canvas = terminal_cursor_rect(canvas.min, 0.0, 5, 10.0, 20.0);
        assert!(!terminal_cursor_in_view(below_canvas, canvas));
    }

    #[test]
    fn terminal_last_row_cursor_handles_fractional_line_height() {
        let canvas = egui::Rect::from_min_size(egui::pos2(10.25, 20.25), egui::vec2(80.0, 97.5));
        let cursor = terminal_cursor_rect(canvas.min, 0.0, 4, 9.75, 19.5);

        assert!(terminal_cursor_in_view(cursor, canvas));
        assert!(cursor.bottom() < canvas.bottom());

        let near_edge = cursor.translate(egui::vec2(0.0, 1.4));
        assert!(terminal_cursor_in_view(near_edge, canvas));
        let outside_tolerance = cursor.translate(egui::vec2(0.0, 1.6));
        assert!(!terminal_cursor_in_view(outside_tolerance, canvas));
    }

    #[test]
    fn terminal_scrollback_row_saturates_and_ime_cursor_is_clamped() {
        assert_eq!(terminal_cursor_display_row(20, usize::MAX), u16::MAX);

        let canvas = egui::Rect::from_min_size(egui::pos2(10.0, 20.0), egui::vec2(80.0, 100.0));
        let cursor = terminal_cursor_rect(canvas.min, 0.0, u16::MAX, 10.0, 20.0);
        let ime_cursor = terminal_ime_cursor_rect(cursor, canvas);

        assert!(canvas.contains_rect(ime_cursor));
        assert_eq!(ime_cursor.bottom(), canvas.bottom());
    }

    #[test]
    fn terminal_cursor_rect_places_cursor_at_rendered_offset() {
        let origin = egui::pos2(10.0, 20.0);
        let cursor = terminal_cursor_rect(origin, 24.0, 2, 8.0, 20.0);

        assert_eq!(cursor.left(), 34.0);
        assert_eq!(cursor.width(), 8.0);
        assert_eq!(cursor.top(), 61.0);

        // 宽度为 0 或负值时至少保留 1px，避免光标不可见
        let minimal = terminal_cursor_rect(origin, 0.0, 0, 0.0, 20.0);
        assert_eq!(minimal.width(), 1.0);
    }

    #[test]
    fn remote_parent_path_uses_posix_semantics() {
        assert_eq!(remote_parent_path("/home/user"), Some("/home".to_string()));
        assert_eq!(remote_parent_path("/home/user/"), Some("/home".to_string()));
        assert_eq!(remote_parent_path("/home"), Some("/".to_string()));
        assert_eq!(
            remote_parent_path("relative/path"),
            Some("relative".to_string())
        );
        assert_eq!(remote_parent_path("relative"), Some(".".to_string()));
        assert_eq!(remote_parent_path(".."), Some("../..".to_string()));
        assert_eq!(remote_parent_path("../dir"), Some("..".to_string()));
        assert_eq!(remote_parent_path("/a/../b"), Some("/".to_string()));
        assert_eq!(remote_parent_path("/a//b"), Some("/a".to_string()));
        assert_eq!(remote_parent_path("/../../a"), Some("/".to_string()));
        assert_eq!(
            remote_parent_path("../../a/../b"),
            Some("../..".to_string())
        );
        assert_eq!(remote_parent_path("a/../../b"), Some("..".to_string()));
    }

    #[test]
    fn remote_parent_path_stops_at_root_or_current_directory() {
        assert_eq!(remote_parent_path("/"), None);
        assert_eq!(remote_parent_path("."), None);
        assert_eq!(remote_parent_path(""), None);
        assert_eq!(remote_parent_path("   "), None);
        assert_eq!(remote_parent_path("//"), None);
        assert_eq!(remote_parent_path("///"), None);
    }

    #[test]
    fn remote_parent_path_does_not_use_windows_separator() {
        assert_eq!(remote_parent_path(r"home\user\file"), Some(".".to_string()));
    }

    #[test]
    fn remote_directory_request_sends_requested_path() {
        let (tx, rx) = mpsc::sync_channel(1);

        assert!(send_remote_directory_request(Some(&tx), "/home/user".to_string()).is_ok());
        match rx.try_recv() {
            Ok(SftpRequest::ListDirectory(path)) => assert_eq!(path, "/home/user"),
            result => panic!("收到非预期请求: {:?}", result),
        }
    }

    #[test]
    fn remote_directory_request_reports_missing_or_closed_channel() {
        assert!(send_remote_directory_request(None, "/home".to_string()).is_err());

        let (tx, rx) = mpsc::sync_channel(1);
        drop(rx);
        assert!(send_remote_directory_request(Some(&tx), "/home".to_string()).is_err());
    }

    #[test]
    fn active_transfer_detection_ignores_completed_tasks() {
        let mut task = crate::plugins::ssh_client::models::TransferTask::new(
            "/source".to_string(),
            "/destination".to_string(),
            crate::plugins::ssh_client::models::TransferDirection::Upload,
            Some(10),
        );
        assert!(has_active_transfer(
            std::slice::from_ref(&task),
            "/source",
            "/destination"
        ));

        task.done = true;
        assert!(!has_active_transfer(&[task], "/source", "/destination"));
    }
}
