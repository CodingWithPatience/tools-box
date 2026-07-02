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

/// SSH 会话配置（数据库模型）
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

/// 新增会话数据结构
#[derive(Debug, Clone)]
pub struct NewSession {
    pub name: String,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub auth_method: AuthMethod,
}
