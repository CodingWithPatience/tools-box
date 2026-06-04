use serde::{Deserialize, Serialize};

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

/// 密码条目（加密存储，延迟解密）
#[derive(Debug, Clone)]
pub struct EncryptedPasswordEntry {
    pub id: i64,
    pub website: String,
    pub url: Option<String>,
    pub username: String,
    pub encrypted_password: Vec<u8>,
    pub iv: Vec<u8>,
    pub notes: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

impl EncryptedPasswordEntry {
    /// 解密密码字段
    pub fn decrypt_password(&self, key: &[u8; 32]) -> String {
        super::crypto::decrypt_password(key, &self.encrypted_password, &self.iv)
            .unwrap_or_default()
    }

    /// 转换为 PasswordEntry（解密）
    pub fn to_decrypted(&self, key: &[u8; 32]) -> PasswordEntry {
        PasswordEntry {
            id: self.id,
            website: self.website.clone(),
            url: self.url.clone(),
            username: self.username.clone(),
            password: self.decrypt_password(key),
            notes: self.notes.clone(),
            created_at: self.created_at.clone(),
            updated_at: self.updated_at.clone(),
        }
    }
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

/// 导出数据格式
#[derive(Debug, Serialize, Deserialize)]
pub struct ExportData {
    pub version: String,
    pub exported_at: String,
    pub entries: Vec<ExportEntry>,
}

/// 导出的单条密码条目
#[derive(Debug, Serialize, Deserialize)]
pub struct ExportEntry {
    pub website: String,
    pub url: Option<String>,
    pub username: String,
    pub password: String,
    pub notes: Option<String>,
}

impl ExportEntry {
    pub fn from_password_entry(entry: &PasswordEntry) -> Self {
        Self {
            website: entry.website.clone(),
            url: entry.url.clone(),
            username: entry.username.clone(),
            password: entry.password.clone(),
            notes: entry.notes.clone(),
        }
    }

    pub fn to_new_entry(&self) -> NewPasswordEntry {
        NewPasswordEntry {
            website: self.website.clone(),
            url: self.url.clone(),
            username: self.username.clone(),
            password: self.password.clone(),
            notes: self.notes.clone(),
        }
    }
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
