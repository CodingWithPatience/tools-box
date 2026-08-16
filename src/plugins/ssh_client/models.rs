/// SSH 认证方式
#[derive(Debug, Clone, PartialEq)]
pub enum AuthMethod {
    /// 密码认证（密文, nonce, PBKDF2 salt）
    Password {
        encrypted_password: Vec<u8>,
        iv: Vec<u8>,
        salt: Vec<u8>,
    },
    /// 密钥文件认证
    KeyFile {
        private_key_path: String,
        /// 密码短语加密数据 (密文, nonce, PBKDF2 salt)
        encrypted_passphrase: Option<(Vec<u8>, Vec<u8>, Vec<u8>)>,
    },
}

impl AuthMethod {
    /// 存储时的类型标识
    pub fn type_str(&self) -> &str {
        match self {
            AuthMethod::Password { .. } => "password",
            AuthMethod::KeyFile { .. } => "keyfile",
        }
    }
}

/// 线程间通信：SSH I/O 线程 → 主线程的消息
#[derive(Debug)]
pub enum SshOutput {
    /// 终端输出数据（ANSI 字节流）
    TerminalData(Vec<u8>),
    /// 连接已建立
    Connected,
    /// 连接断开（携带原因）
    Disconnected(String),
    /// 发生错误
    Error(String),
}

/// 线程间通信：主线程 → SSH I/O 线程的消息
#[derive(Debug)]
pub enum SshInput {
    /// 用户键盘输入
    KeyInput(Vec<u8>),
    /// 终端窗口大小变更 (cols, rows)
    Resize(u16, u16),
    /// 断开连接
    Disconnect,
}

/// SSH 连接配置（数据库模型）
#[derive(Debug, Clone)]
pub struct SshSession {
    pub id: i64,
    /// 会话名称
    pub name: String,
    /// 远程主机地址
    pub host: String,
    /// SSH 端口（默认 22)
    pub port: u16,
    /// 登录用户名
    pub username: String,
    /// 认证方式
    pub auth_method: AuthMethod,
}

/// 会话连接状态
#[derive(Debug, Clone, PartialEq)]
pub enum SessionState {
    /// 未连接
    Disconnected,
    /// 连接中
    Connecting,
    /// 已连接
    Connected,
    /// 连接失败
    Error(String),
}

/// 表单认证类型选择
#[derive(Debug, Clone, PartialEq)]
pub enum AuthType {
    Password,
    KeyFile,
}

impl AuthType {
    pub fn as_str(&self) -> &str {
        match self {
            AuthType::Password => "密码认证",
            AuthType::KeyFile => "密钥文件",
        }
    }
}

/// 会话编辑表单
#[derive(Debug, Clone)]
pub struct SessionForm {
    pub name: String,
    pub host: String,
    pub port: String,
    pub username: String,
    pub auth_type: AuthType,
    pub password: String,
    pub private_key_path: String,
    pub passphrase: String,
}

impl SessionForm {
    pub fn new() -> Self {
        Self {
            name: String::new(),
            host: String::new(),
            port: "22".to_string(),
            username: String::new(),
            auth_type: AuthType::Password,
            password: String::new(),
            private_key_path: String::new(),
            passphrase: String::new(),
        }
    }

    /// 从已有会话填充表单（密码和密钥短语不回显，需重新输入）
    pub fn from_session(session: &SshSession) -> Self {
        Self {
            name: session.name.clone(),
            host: session.host.clone(),
            port: session.port.to_string(),
            username: session.username.clone(),
            auth_type: match &session.auth_method {
                AuthMethod::Password { .. } => AuthType::Password,
                AuthMethod::KeyFile { .. } => AuthType::KeyFile,
            },
            password: String::new(),
            private_key_path: match &session.auth_method {
                AuthMethod::KeyFile {
                    private_key_path, ..
                } => private_key_path.clone(),
                _ => String::new(),
            },
            passphrase: String::new(),
        }
    }

    /// 校验表单必填项
    pub fn validate(&self) -> Result<(), String> {
        if self.name.trim().is_empty() {
            return Err("会话名称不能为空".to_string());
        }
        if self.host.trim().is_empty() {
            return Err("主机地址不能为空".to_string());
        }
        if self.username.trim().is_empty() {
            return Err("用户名不能为空".to_string());
        }
        let port: u16 = self
            .port
            .trim()
            .parse()
            .map_err(|_| "端口号格式不正确".to_string())?;
        if port == 0 {
            return Err("端口号不能为 0".to_string());
        }
        if self.auth_type == AuthType::Password && self.password.is_empty() {
            return Err("密码不能为空".to_string());
        }
        if self.auth_type == AuthType::KeyFile && self.private_key_path.is_empty() {
            return Err("密钥文件路径不能为空".to_string());
        }
        Ok(())
    }
}

/// 新增连接数据结构
#[derive(Debug, Clone)]
pub struct NewSession {
    pub name: String,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub auth_method: AuthMethod,
}

// ===================================================================
// SFTP 相关数据结构
// ===================================================================

/// 文件条目（用于本地和远程文件列表）
#[derive(Debug, Clone)]
pub struct FileEntry {
    /// 文件名
    pub name: String,
    /// 完整路径
    pub path: String,
    /// 是否为目录
    pub is_dir: bool,
    /// 文件大小（字节，目录为 0）
    pub size: u64,
    /// 文件大小是否由文件系统明确提供
    pub size_known: bool,
    /// 修改时间戳（秒）
    pub modified: Option<i64>,
}

impl FileEntry {
    /// 获取文件大小的可读格式
    pub fn size_display(&self) -> String {
        if self.is_dir {
            return "<DIR>".to_string();
        }
        if !self.size_known {
            return "未知".to_string();
        }
        if self.size < 1024 {
            format!("{} B", self.size)
        } else if self.size < 1024 * 1024 {
            format!("{:.1} KB", self.size as f64 / 1024.0)
        } else if self.size < 1024 * 1024 * 1024 {
            format!("{:.1} MB", self.size as f64 / (1024.0 * 1024.0))
        } else {
            format!("{:.1} GB", self.size as f64 / (1024.0 * 1024.0 * 1024.0))
        }
    }

    /// 获取修改时间的可读格式
    pub fn modified_display(&self) -> String {
        self.modified
            .map(|ts| {
                let dt = chrono::DateTime::from_timestamp(ts, 0);
                match dt {
                    Some(d) => d.format("%Y-%m-%d %H:%M").to_string(),
                    None => "-".to_string(),
                }
            })
            .unwrap_or_else(|| "-".to_string())
    }
}

/// SFTP 文件传输方向
#[derive(Debug, Clone, PartialEq)]
pub enum TransferDirection {
    /// 上传（本地 → 远程）
    Upload,
    /// 下载（远程 → 本地）
    Download,
}

/// 文件传输任务
#[derive(Debug, Clone)]
pub struct TransferTask {
    /// 任务唯一标识
    pub id: String,
    /// 源路径
    pub source: String,
    /// 目标路径
    pub destination: String,
    /// 传输方向
    pub direction: TransferDirection,
    /// 文件总大小（字节）
    pub total_size: Option<u64>,
    /// 已传输大小（字节）
    pub transferred: u64,
    /// 文件名（用于显示）
    pub filename: String,
    /// 是否完成
    pub done: bool,
    /// 错误信息（如有）
    pub error: Option<String>,
}

impl TransferTask {
    /// 创建文件传输任务
    pub fn new(
        source: String,
        destination: String,
        direction: TransferDirection,
        total_size: Option<u64>,
    ) -> Self {
        let filename = std::path::Path::new(&source)
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_else(|| source.clone());
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            source,
            destination,
            direction,
            total_size,
            transferred: 0,
            filename,
            done: false,
            error: None,
        }
    }

    /// 获取传输进度百分比 (0.0 ~ 1.0)
    pub fn progress(&self) -> f32 {
        match self.total_size {
            Some(0) => 1.0,
            Some(total_size) => {
                let ratio = self.transferred as f64 / total_size as f64;
                ratio.min(1.0) as f32
            }
            None => 0.0,
        }
    }

    /// 获取进度显示文本
    pub fn progress_text(&self) -> String {
        if self.done {
            match &self.error {
                Some(err) => format!("❌ 失败: {}", err),
                None => "✅ 完成".to_string(),
            }
        } else {
            let Some(total_size) = self.total_size else {
                return if self.transferred == 0 {
                    "等待中".to_string()
                } else {
                    format!("已传输 {}", Self::format_size(self.transferred))
                };
            };
            format!(
                "{} / {} ({:.0}%)",
                Self::format_size(self.transferred),
                Self::format_size(total_size),
                self.progress() * 100.0
            )
        }
    }

    fn format_size(bytes: u64) -> String {
        if bytes < 1024 {
            format!("{} B", bytes)
        } else if bytes < 1024 * 1024 {
            format!("{:.1} KB", bytes as f64 / 1024.0)
        } else if bytes < 1024 * 1024 * 1024 {
            format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
        } else {
            format!("{:.1} GB", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
        }
    }
}

/// SFTP 操作请求（UI → SFTP 线程）
#[derive(Debug)]
pub enum SftpRequest {
    /// 列出远程目录
    ListDirectory(String),
    /// 上传文件 (本地路径, 远程路径)
    Upload(TransferTask),
    /// 下载文件 (远程路径, 本地路径)
    Download(TransferTask),
}

/// SFTP 控制请求，不与普通操作共用有界队列
#[derive(Debug)]
pub enum SftpControl {
    /// 取消指定传输任务
    CancelTransfer(String),
    /// 断开 SFTP 连接并取消全部未完成任务
    Disconnect,
}

/// SFTP 操作响应（SFTP 线程 → UI）
#[derive(Debug)]
pub enum SftpResponse {
    /// 目录列表结果
    DirectoryList(String, Vec<FileEntry>),
    /// 远程目录加载失败（请求路径, 错误信息）
    DirectoryError(String, String),
    /// 传输进度更新
    TransferProgress(TransferTask),
    /// 操作完成
    OperationDone(TransferTask),
    /// 操作错误
    Error(String),
    /// SFTP 连接已就绪
    Connected,
    /// SFTP 连接已断开
    Disconnected,
}

// ===================================================================
// 多标签会话相关数据结构
// ===================================================================

/// 会话视图中的 Tab 页类型
#[derive(Debug, Clone, PartialEq)]
pub enum SessionViewTab {
    /// 交互终端
    Terminal,
    /// SFTP 文件传输
    Sftp,
}

/// 终端文本选择范围
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TerminalSelection {
    /// 鼠标开始拖动时的单元格位置（列、行）
    pub anchor: (u16, u16),
    /// 鼠标当前拖动到的单元格位置（列、行）
    pub focus: (u16, u16),
}

impl TerminalSelection {
    /// 从指定终端单元格开始创建选择范围
    pub fn new(position: (u16, u16)) -> Self {
        Self {
            anchor: position,
            focus: position,
        }
    }

    /// 按终端显示顺序返回选择范围的起止位置
    pub fn normalized(&self) -> ((u16, u16), (u16, u16)) {
        let anchor_key = (self.anchor.1, self.anchor.0);
        let focus_key = (self.focus.1, self.focus.0);
        if anchor_key <= focus_key {
            (self.anchor, self.focus)
        } else {
            (self.focus, self.anchor)
        }
    }
}

/// 单个会话标签的状态
pub struct SessionTab {
    /// 关联的会话配置 ID
    pub session_id: i64,
    /// 会话名称（用于标签显示）
    pub name: String,
    /// 终端仿真器
    pub terminal: Option<super::terminal::TerminalEmulator>,
    /// SSH 输入通道
    pub input_tx: Option<std::sync::mpsc::SyncSender<SshInput>>,
    /// SSH 输出通道
    pub output_rx: Option<std::sync::mpsc::Receiver<SshOutput>>,
    /// 连接状态
    pub connection_state: SessionState,
    /// 状态消息
    pub status_msg: String,
    /// 用户自定义字体大小
    pub custom_font_size: Option<f32>,
    /// IME 状态
    pub ime_active: bool,
    /// 当前终端文本选择范围
    pub terminal_selection: Option<TerminalSelection>,
    /// 等待平台返回终端粘贴事件的截止时间（egui 时间秒）
    pub terminal_paste_deadline: Option<f64>,

    // ===== SFTP 相关 =====
    /// 当前活动的子 Tab（终端/SFTP)
    pub active_tab: SessionViewTab,
    /// SFTP 请求发送端
    pub sftp_tx: Option<std::sync::mpsc::SyncSender<SftpRequest>>,
    /// SFTP 控制请求发送端
    pub sftp_control_tx: Option<std::sync::mpsc::Sender<SftpControl>>,
    /// SFTP 响应接收端
    pub sftp_rx: Option<std::sync::mpsc::Receiver<SftpResponse>>,
    /// 本地当前目录
    pub local_current_dir: String,
    /// 本地目录文件列表
    pub local_files: Vec<FileEntry>,
    /// 远程当前目录
    pub remote_current_dir: String,
    /// 远程目录文件列表
    pub remote_files: Vec<FileEntry>,
    /// 本地选中的文件索引
    pub local_selected: Option<usize>,
    /// 远程选中的文件索引
    pub remote_selected: Option<usize>,
    /// 当前传输任务列表
    pub transfer_tasks: Vec<TransferTask>,
    /// SFTP 连接状态
    pub sftp_connected: bool,
    /// SFTP 操作状态消息
    pub sftp_status_msg: String,
    /// 远程目录输入框
    pub remote_dir_input: String,
    /// 本地目录输入框
    pub local_dir_input: String,
}

impl SessionTab {
    /// 创建新的会话标签
    pub fn new(session_id: i64, name: String) -> Self {
        let home = super::sftp::home_dir();
        Self {
            session_id,
            name,
            terminal: None,
            input_tx: None,
            output_rx: None,
            connection_state: SessionState::Disconnected,
            status_msg: "就绪".to_string(),
            custom_font_size: None,
            ime_active: false,
            terminal_selection: None,
            terminal_paste_deadline: None,
            active_tab: SessionViewTab::Terminal,
            sftp_tx: None,
            sftp_control_tx: None,
            sftp_rx: None,
            local_current_dir: home.clone(),
            local_files: Vec::new(),
            remote_current_dir: String::new(),
            remote_files: Vec::new(),
            local_selected: None,
            remote_selected: None,
            transfer_tasks: Vec::new(),
            sftp_connected: false,
            sftp_status_msg: String::new(),
            remote_dir_input: String::new(),
            local_dir_input: home,
        }
    }

    /// 断开终端连接
    pub fn disconnect_terminal(&mut self) {
        if let Some(tx) = &self.input_tx {
            let _ = tx.send(SshInput::Disconnect);
        }
        self.terminal = None;
        self.input_tx = None;
        self.output_rx = None;
        self.connection_state = SessionState::Disconnected;
        self.status_msg = "终端已断开".to_string();
        self.custom_font_size = None;
        self.terminal_selection = None;
        self.terminal_paste_deadline = None;
    }

    /// 断开 SFTP 连接
    pub fn disconnect_sftp(&mut self) {
        if let Some(tx) = &self.sftp_control_tx {
            if tx.send(SftpControl::Disconnect).is_ok() {
                self.sftp_connected = false;
                self.sftp_status_msg = "正在断开 SFTP...".to_string();
                return;
            }
        }
        self.sftp_tx = None;
        self.sftp_control_tx = None;
        self.sftp_rx = None;
        self.sftp_connected = false;
        self.remote_files.clear();
        self.remote_current_dir.clear();
        fail_unfinished_transfers(&mut self.transfer_tasks, "SFTP 控制通道已关闭");
        self.sftp_status_msg = "SFTP 已断开".to_string();
    }

    /// 断开所有连接
    pub fn disconnect_all(&mut self) {
        self.disconnect_terminal();
        self.disconnect_sftp();
    }
}

/// 将所有未完成传输标记为失败
pub fn fail_unfinished_transfers(tasks: &mut [TransferTask], reason: &str) {
    for task in tasks.iter_mut().filter(|task| !task.done) {
        task.done = true;
        task.error = Some(reason.to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_selection_normalizes_reverse_drag() {
        let forward = TerminalSelection {
            anchor: (2, 1),
            focus: (5, 3),
        };
        assert_eq!(forward.normalized(), ((2, 1), (5, 3)));

        let reverse = TerminalSelection {
            anchor: (5, 3),
            focus: (2, 1),
        };
        assert_eq!(reverse.normalized(), ((2, 1), (5, 3)));
    }

    #[test]
    fn test_file_entry_size_display() {
        let dir = FileEntry {
            name: "test".to_string(),
            path: "/test".to_string(),
            is_dir: true,
            size: 0,
            size_known: true,
            modified: None,
        };
        assert_eq!(dir.size_display(), "<DIR>");

        let small = FileEntry {
            name: "a.txt".to_string(),
            path: "/a.txt".to_string(),
            is_dir: false,
            size: 500,
            size_known: true,
            modified: None,
        };
        assert_eq!(small.size_display(), "500 B");

        let kb = FileEntry {
            name: "b.txt".to_string(),
            path: "/b.txt".to_string(),
            is_dir: false,
            size: 2048,
            size_known: true,
            modified: None,
        };
        assert_eq!(kb.size_display(), "2.0 KB");

        let mb = FileEntry {
            name: "c.bin".to_string(),
            path: "/c.bin".to_string(),
            is_dir: false,
            size: 5 * 1024 * 1024,
            size_known: true,
            modified: None,
        };
        assert_eq!(mb.size_display(), "5.0 MB");

        let gb = FileEntry {
            name: "d.bin".to_string(),
            path: "/d.bin".to_string(),
            is_dir: false,
            size: 2 * 1024 * 1024 * 1024,
            size_known: true,
            modified: None,
        };
        assert_eq!(gb.size_display(), "2.0 GB");

        let unknown = FileEntry {
            name: "unknown.bin".to_string(),
            path: "/unknown.bin".to_string(),
            is_dir: false,
            size: 0,
            size_known: false,
            modified: None,
        };
        assert_eq!(unknown.size_display(), "未知");
    }

    #[test]
    fn test_transfer_task_progress() {
        let task = TransferTask {
            id: "upload-1".to_string(),
            source: "/a".to_string(),
            destination: "/b".to_string(),
            direction: TransferDirection::Upload,
            total_size: Some(1000),
            transferred: 500,
            filename: "test.bin".to_string(),
            done: false,
            error: None,
        };
        assert!((task.progress() - 0.5).abs() < 0.01);

        let empty = TransferTask {
            id: "download-1".to_string(),
            source: "/a".to_string(),
            destination: "/b".to_string(),
            direction: TransferDirection::Download,
            total_size: Some(0),
            transferred: 0,
            filename: "empty".to_string(),
            done: false,
            error: None,
        };
        assert!((empty.progress() - 1.0).abs() < 0.01);
    }

    #[test]
    fn test_transfer_task_progress_text() {
        let in_progress = TransferTask {
            id: "upload-2".to_string(),
            source: "/a".to_string(),
            destination: "/b".to_string(),
            direction: TransferDirection::Upload,
            total_size: Some(1024),
            transferred: 512,
            filename: "test.bin".to_string(),
            done: false,
            error: None,
        };
        assert!(in_progress.progress_text().contains("50%"));

        let pending = TransferTask::new(
            "/a".to_string(),
            "/b".to_string(),
            TransferDirection::Upload,
            None,
        );
        assert_eq!(pending.progress_text(), "等待中");

        let mut unknown_size = pending;
        unknown_size.transferred = 2048;
        assert_eq!(unknown_size.progress_text(), "已传输 2.0 KB");

        let done = TransferTask {
            id: "upload-3".to_string(),
            source: "/a".to_string(),
            destination: "/b".to_string(),
            direction: TransferDirection::Upload,
            total_size: Some(1024),
            transferred: 1024,
            filename: "test.bin".to_string(),
            done: true,
            error: None,
        };
        assert_eq!(done.progress_text(), "✅ 完成");

        let failed = TransferTask {
            id: "download-2".to_string(),
            source: "/a".to_string(),
            destination: "/b".to_string(),
            direction: TransferDirection::Download,
            total_size: Some(1024),
            transferred: 0,
            filename: "test.bin".to_string(),
            done: true,
            error: Some("网络错误".to_string()),
        };
        assert!(failed.progress_text().contains("网络错误"));
    }

    #[test]
    fn test_file_entry_modified_display_none() {
        let entry = FileEntry {
            name: "a".to_string(),
            path: "/a".to_string(),
            is_dir: false,
            size: 0,
            size_known: true,
            modified: None,
        };
        assert_eq!(entry.modified_display(), "-");
    }

    #[test]
    fn fail_unfinished_transfers_preserves_completed_history() {
        let mut active = TransferTask::new(
            "/active".to_string(),
            "/target".to_string(),
            TransferDirection::Upload,
            Some(10),
        );
        let mut completed = TransferTask::new(
            "/completed".to_string(),
            "/target".to_string(),
            TransferDirection::Download,
            Some(10),
        );
        completed.done = true;
        completed.transferred = 10;
        let mut tasks = vec![active.clone(), completed.clone()];

        fail_unfinished_transfers(&mut tasks, "连接已断开");

        active.done = true;
        active.error = Some("连接已断开".to_string());
        assert_eq!(tasks[0].done, active.done);
        assert_eq!(tasks[0].error, active.error);
        assert!(tasks[1].done);
        assert_eq!(tasks[1].error, completed.error);
    }
}
