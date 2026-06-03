use anyhow::{Context, Result};
use rusqlite::{params, Connection};

use super::crypto;
use super::models::{MasterConfig, NewPasswordEntry, PasswordEntry};

/// 密码数据库操作
pub struct PasswordStore<'a> {
    conn: &'a Connection,
}

impl<'a> PasswordStore<'a> {
    pub fn new(conn: &'a Connection) -> Self {
        Self { conn }
    }

    /// 获取主密码配置
    pub fn get_master_config(&self) -> Result<Option<MasterConfig>> {
        let mut stmt = self
            .conn
            .prepare("SELECT salt, verify_hash FROM master_config WHERE id = 1")
            .context("查询主密码配置失败")?;

        let result = stmt
            .query_row([], |row| {
                Ok(MasterConfig {
                    salt: row.get(0)?,
                    verify_hash: row.get(1)?,
                })
            })
            .optional()
            .context("读取主密码配置失败")?;

        Ok(result)
    }

    /// 保存主密码配置
    pub fn save_master_config(&self, config: &MasterConfig) -> Result<()> {
        self.conn
            .execute(
                "INSERT OR REPLACE INTO master_config (id, salt, verify_hash) VALUES (1, ?1, ?2)",
                params![config.salt, config.verify_hash],
            )
            .context("保存主密码配置失败")?;
        Ok(())
    }

    /// 验证主密码
    pub fn verify_master_password(&self, password: &str) -> Result<Option<[u8; 32]>> {
        let config = match self.get_master_config()? {
            Some(c) => c,
            None => return Ok(None), // 未设置主密码
        };

        let hash = crypto::hash_master_password(password, &config.salt);

        if hash == config.verify_hash {
            let key = crypto::derive_key(password, &config.salt);
            Ok(Some(key))
        } else {
            Ok(None)
        }
    }

    /// 初始化主密码
    pub fn setup_master_password(&self, password: &str) -> Result<[u8; 32]> {
        let salt = crypto::generate_salt();
        let verify_hash = crypto::hash_master_password(password, &salt);
        let key = crypto::derive_key(password, &salt);

        let config = MasterConfig {
            salt: salt.to_vec(),
            verify_hash,
        };

        self.save_master_config(&config)?;
        Ok(key)
    }

    /// 获取所有密码条目
    pub fn get_all_entries(&self, key: &[u8; 32]) -> Result<Vec<PasswordEntry>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, website, url, username, password, iv, notes, created_at, updated_at
                 FROM passwords ORDER BY website ASC",
            )
            .context("查询密码列表失败")?;

        let entries = stmt
            .query_map([], |row| {
                let id: i64 = row.get(0)?;
                let website: String = row.get(1)?;
                let url: Option<String> = row.get(2)?;
                let username: String = row.get(3)?;
                let encrypted_pwd: Vec<u8> = row.get(4)?;
                let iv: Vec<u8> = row.get(5)?;
                let notes: Option<String> = row.get(6)?;
                let created_at: String = row.get(7)?;
                let updated_at: String = row.get(8)?;

                // 解密密码
                let password =
                    crypto::decrypt_password(key, &encrypted_pwd, &iv).unwrap_or_default();

                Ok(PasswordEntry {
                    id,
                    website,
                    url,
                    username,
                    password,
                    notes,
                    created_at,
                    updated_at,
                })
            })
            .context("读取密码列表失败")?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(entries)
    }

    /// 搜索密码条目
    pub fn search_entries(&self, query: &str, key: &[u8; 32]) -> Result<Vec<PasswordEntry>> {
        let search_pattern = format!("%{}%", query);
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, website, url, username, password, iv, notes, created_at, updated_at
                 FROM passwords
                 WHERE website LIKE ?1 OR username LIKE ?1 OR url LIKE ?1
                 ORDER BY website ASC",
            )
            .context("搜索密码失败")?;

        let entries = stmt
            .query_map(params![search_pattern], |row| {
                let id: i64 = row.get(0)?;
                let website: String = row.get(1)?;
                let url: Option<String> = row.get(2)?;
                let username: String = row.get(3)?;
                let encrypted_pwd: Vec<u8> = row.get(4)?;
                let iv: Vec<u8> = row.get(5)?;
                let notes: Option<String> = row.get(6)?;
                let created_at: String = row.get(7)?;
                let updated_at: String = row.get(8)?;

                let password =
                    crypto::decrypt_password(key, &encrypted_pwd, &iv).unwrap_or_default();

                Ok(PasswordEntry {
                    id,
                    website,
                    url,
                    username,
                    password,
                    notes,
                    created_at,
                    updated_at,
                })
            })
            .context("搜索密码失败")?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(entries)
    }

    /// 添加密码条目
    pub fn add_entry(&self, entry: &NewPasswordEntry, key: &[u8; 32]) -> Result<i64> {
        let (encrypted_pwd, iv) = crypto::encrypt_password(key, &entry.password)?;

        self.conn.execute(
            "INSERT INTO passwords (website, url, username, password, iv, notes) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![entry.website, entry.url, entry.username, encrypted_pwd, iv, entry.notes],
        ).context("添加密码失败")?;

        Ok(self.conn.last_insert_rowid())
    }

    /// 更新密码条目
    pub fn update_entry(&self, entry: &PasswordEntry, key: &[u8; 32]) -> Result<()> {
        let (encrypted_pwd, iv) = crypto::encrypt_password(key, &entry.password)?;

        self.conn.execute(
            "UPDATE passwords SET website = ?1, url = ?2, username = ?3, password = ?4, iv = ?5, notes = ?6, updated_at = CURRENT_TIMESTAMP WHERE id = ?7",
            params![entry.website, entry.url, entry.username, encrypted_pwd, iv, entry.notes, entry.id],
        ).context("更新密码失败")?;

        Ok(())
    }

    /// 删除密码条目
    pub fn delete_entry(&self, id: i64) -> Result<()> {
        self.conn
            .execute("DELETE FROM passwords WHERE id = ?1", params![id])
            .context("删除密码失败")?;
        Ok(())
    }

    /// 获取密码条目数量
    pub fn count_entries(&self) -> Result<usize> {
        let count: usize = self
            .conn
            .query_row("SELECT COUNT(*) FROM passwords", [], |row| row.get(0))
            .context("查询密码数量失败")?;
        Ok(count)
    }
}

/// rusqlite 扩展 trait，用于 query_row 的 optional 方法
trait OptionalExtension<T> {
    fn optional(self) -> Result<Option<T>>;
}

impl<T> OptionalExtension<T> for Result<T, rusqlite::Error> {
    fn optional(self) -> Result<Option<T>> {
        match self {
            Ok(value) => Ok(Some(value)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }
}
