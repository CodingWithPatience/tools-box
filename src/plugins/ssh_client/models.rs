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
    /// 排序顺序
    pub sort_order: i32,
    /// 创建时间
    pub created_at: String,
    /// 更新时间
    pub updated_at: String,
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
                AuthMethod::KeyFile { private_key_path, .. } => private_key_path.clone(),
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
    /// 修改时间戳（秒）
    pub modified: Option<i64>,
    /// 文件权限（Unix 模式，仅远程文件有意义）
    pub permissions: Option<u32>,
}

impl FileEntry {
    /// 获取文件大小的可读格式
    pub fn size_display(&self) -> String {
        if self.is_dir {
            return "<DIR>".to_string();
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
    /// 源路径
    pub source: String,
    /// 目标路径
    pub destination: String,
    /// 传输方向
    pub direction: TransferDirection,
    /// 文件总大小（字节）
    pub total_size: u64,
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
    /// 获取传输进度百分比 (0.0 ~ 1.0)
    pub fn progress(&self) -> f32 {
        if self.total_size == 0 {
            return 1.0;
        }
        let ratio = self.transferred as f64 / self.total_size as f64;
        ratio.min(1.0) as f32
    }

    /// 获取进度显示文本
    pub fn progress_text(&self) -> String {
        if self.done {
            match &self.error {
                Some(err) => format!("❌ 失败: {}", err),
                None => "✅ 完成".to_string(),
            }
        } else {
            format!(
                "{} / {} ({:.0}%)",
                Self::format_size(self.transferred),
                Self::format_size(self.total_size),
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
    Upload(String, String),
    /// 下载文件 (远程路径, 本地路径)
    Download(String, String),
    /// 创建远程目录
    MkDir(String),
    /// 删除远程文件
    Delete(String),
    /// 断开 SFTP 连接
    Disconnect,
}

/// SFTP 操作响应（SFTP 线程 → UI）
#[derive(Debug)]
pub enum SftpResponse {
    /// 目录列表结果
    DirectoryList(String, Vec<FileEntry>),
    /// 传输进度更新
    TransferProgress(TransferTask),
    /// 操作完成
    OperationDone(String),
    /// 操作错误
    Error(String),
    /// SFTP 连接已就绪
    Connected,
    /// SFTP 连接已断开
    Disconnected,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_file_entry_size_display() {
        let dir = FileEntry {
            name: "test".to_string(),
            path: "/test".to_string(),
            is_dir: true,
            size: 0,
            modified: None,
            permissions: None,
        };
        assert_eq!(dir.size_display(), "<DIR>");

        let small = FileEntry {
            name: "a.txt".to_string(),
            path: "/a.txt".to_string(),
            is_dir: false,
            size: 500,
            modified: None,
            permissions: None,
        };
        assert_eq!(small.size_display(), "500 B");

        let kb = FileEntry {
            name: "b.txt".to_string(),
            path: "/b.txt".to_string(),
            is_dir: false,
            size: 2048,
            modified: None,
            permissions: None,
        };
        assert_eq!(kb.size_display(), "2.0 KB");

        let mb = FileEntry {
            name: "c.bin".to_string(),
            path: "/c.bin".to_string(),
            is_dir: false,
            size: 5 * 1024 * 1024,
            modified: None,
            permissions: None,
        };
        assert_eq!(mb.size_display(), "5.0 MB");

        let gb = FileEntry {
            name: "d.bin".to_string(),
            path: "/d.bin".to_string(),
            is_dir: false,
            size: 2 * 1024 * 1024 * 1024,
            modified: None,
            permissions: None,
        };
        assert_eq!(gb.size_display(), "2.0 GB");
    }

    #[test]
    fn test_transfer_task_progress() {
        let task = TransferTask {
            source: "/a".to_string(),
            destination: "/b".to_string(),
            direction: TransferDirection::Upload,
            total_size: 1000,
            transferred: 500,
            filename: "test.bin".to_string(),
            done: false,
            error: None,
        };
        assert!((task.progress() - 0.5).abs() < 0.01);

        let empty = TransferTask {
            source: "/a".to_string(),
            destination: "/b".to_string(),
            direction: TransferDirection::Download,
            total_size: 0,
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
            source: "/a".to_string(),
            destination: "/b".to_string(),
            direction: TransferDirection::Upload,
            total_size: 1024,
            transferred: 512,
            filename: "test.bin".to_string(),
            done: false,
            error: None,
        };
        assert!(in_progress.progress_text().contains("50%"));

        let done = TransferTask {
            source: "/a".to_string(),
            destination: "/b".to_string(),
            direction: TransferDirection::Upload,
            total_size: 1024,
            transferred: 1024,
            filename: "test.bin".to_string(),
            done: true,
            error: None,
        };
        assert_eq!(done.progress_text(), "✅ 完成");

        let failed = TransferTask {
            source: "/a".to_string(),
            destination: "/b".to_string(),
            direction: TransferDirection::Download,
            total_size: 1024,
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
            modified: None,
            permissions: None,
        };
        assert_eq!(entry.modified_display(), "-");
    }
}
