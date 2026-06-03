/// 密码条目（解密后的视图）
#[derive(Debug, Clone)]
pub struct PasswordEntry {
    pub id: i64,
    pub website: String,
    pub url: Option<String>,
    pub username: String,
    pub password: String,
    pub notes: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

/// 新增密码条目的表单数据
#[derive(Debug, Clone)]
pub struct NewPasswordEntry {
    pub website: String,
    pub url: Option<String>,
    pub username: String,
    pub password: String,
    pub notes: Option<String>,
}

/// 主密码配置
#[derive(Debug, Clone)]
pub struct MasterConfig {
    pub salt: Vec<u8>,
    pub verify_hash: Vec<u8>,
}

/// 密码编辑表单状态
#[derive(Debug, Clone)]
pub struct PasswordForm {
    pub website: String,
    pub url: String,
    pub username: String,
    pub password: String,
    pub notes: String,
    pub show_password: bool,
}

impl PasswordForm {
    pub fn new() -> Self {
        Self {
            website: String::new(),
            url: String::new(),
            username: String::new(),
            password: String::new(),
            notes: String::new(),
            show_password: false,
        }
    }

    pub fn from_entry(entry: &PasswordEntry) -> Self {
        Self {
            website: entry.website.clone(),
            url: entry.url.clone().unwrap_or_default(),
            username: entry.username.clone(),
            password: entry.password.clone(),
            notes: entry.notes.clone().unwrap_or_default(),
            show_password: false,
        }
    }

    pub fn to_new_entry(&self) -> NewPasswordEntry {
        NewPasswordEntry {
            website: self.website.clone(),
            url: if self.url.is_empty() {
                None
            } else {
                Some(self.url.clone())
            },
            username: self.username.clone(),
            password: self.password.clone(),
            notes: if self.notes.is_empty() {
                None
            } else {
                Some(self.notes.clone())
            },
        }
    }

    pub fn is_valid(&self) -> bool {
        !self.website.is_empty() && !self.username.is_empty() && !self.password.is_empty()
    }
}

/// 密码生成配置
#[derive(Debug, Clone)]
pub struct GeneratorConfig {
    pub length: usize,
    pub use_uppercase: bool,
    pub use_lowercase: bool,
    pub use_digits: bool,
    pub use_symbols: bool,
}

impl Default for GeneratorConfig {
    fn default() -> Self {
        Self {
            length: 16,
            use_uppercase: true,
            use_lowercase: true,
            use_digits: true,
            use_symbols: true,
        }
    }
}
