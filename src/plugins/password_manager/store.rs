use anyhow::{Context, Result};
use rusqlite::{Connection, params};

use super::crypto;
use super::models::{
    EncryptedPasswordEntry, ExportData, ExportEntry, ExportFormat, MasterConfig, NewPasswordEntry,
    PasswordEntry,
};

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
    ///
    /// 返回值：
    /// - `Ok(Some(key))` - 验证成功，返回派生密钥
    /// - `Ok(None)` - 主密码未设置
    /// - `Err(...)` - 主密码错误或其他错误
    pub fn verify_master_password(&self, password: &str) -> Result<Option<[u8; 32]>> {
        let config = match self.get_master_config()? {
            Some(c) => c,
            None => return Ok(None), // 未设置主密码
        };

        // 使用新函数一次性计算 key 和 hash，避免重复 PBKDF2
        let (key, hash) = crypto::hash_master_password_with_key(password, &config.salt);

        if hash == config.verify_hash {
            Ok(Some(key))
        } else {
            Err(anyhow::anyhow!("主密码错误"))
        }
    }

    /// 初始化主密码
    pub fn setup_master_password(&self, password: &str) -> Result<[u8; 32]> {
        let salt = crypto::generate_salt();
        // 使用新函数一次性计算 key 和 hash，避免重复 PBKDF2
        let (key, verify_hash) = crypto::hash_master_password_with_key(password, &salt);

        let config = MasterConfig {
            salt: salt.to_vec(),
            verify_hash,
        };

        self.save_master_config(&config)?;
        Ok(key)
    }

    /// 检查是否已设置主密码
    pub fn has_master_password(&self) -> Result<bool> {
        let config = self.get_master_config()?;
        Ok(config.is_some())
    }

    /// 修改主密码
    ///
    /// 验证旧密码后，使用新密码重新加密所有密码条目。
    /// 整个过程在事务中执行，失败时自动回滚（RAII）。
    ///
    /// 调用前应确保主密码已设置。
    pub fn change_master_password(
        &self,
        old_password: &str,
        new_password: &str,
    ) -> Result<[u8; 32]> {
        // 1. 验证旧密码（在事务外执行，避免事务状态干扰）
        let old_key = self
            .verify_master_password(old_password)?
            .ok_or_else(|| anyhow::anyhow!("主密码未设置"))?;

        if new_password.len() < 6 {
            anyhow::bail!("新密码长度至少 6 位");
        }

        if old_password == new_password {
            anyhow::bail!("新密码不能与旧密码相同");
        }

        // 2. 开启事务（RAII，Drop 时自动回滚）
        let tx = self.conn.unchecked_transaction().context("开启事务失败")?;

        // 3. 获取所有加密条目
        let entries = {
            let mut stmt = tx
                .prepare(
                    "SELECT id, name, url, username, password, iv, notes
                     FROM passwords ORDER BY name ASC",
                )
                .context("查询密码列表失败")?;

            stmt.query_map([], |row| {
                let id: i64 = row.get(0)?;
                let name: String = row.get(1)?;
                let url: Option<String> = row.get(2)?;
                let username: String = row.get(3)?;
                let encrypted_password: Vec<u8> = row.get(4)?;
                let iv: Vec<u8> = row.get(5)?;
                let notes: Option<String> = row.get(6)?;

                Ok(EncryptedPasswordEntry {
                    id,
                    name,
                    url,
                    username,
                    encrypted_password,
                    iv,
                    notes,
                })
            })
            .context("读取密码列表失败")?
            .collect::<Result<Vec<_>, _>>()?
        };

        // 4. 生成新密钥和配置
        let salt = crypto::generate_salt();
        let (new_key, verify_hash) = crypto::hash_master_password_with_key(new_password, &salt);

        // 5. 用新密钥重新加密所有条目
        for entry in &entries {
            let plaintext =
                crypto::decrypt_password(&old_key, &entry.encrypted_password, &entry.iv)
                    .context(format!("解密条目 '{}' 失败", entry.name))?;

            let (new_encrypted, new_iv) = crypto::encrypt_password(&new_key, &plaintext)?;

            tx.execute(
                "UPDATE passwords SET password = ?1, iv = ?2 WHERE id = ?3",
                params![new_encrypted, new_iv, entry.id],
            )
            .context("更新加密数据失败")?;
        }

        // 6. 更新主密码配置
        tx.execute(
            "INSERT OR REPLACE INTO master_config (id, salt, verify_hash) VALUES (1, ?1, ?2)",
            params![salt.to_vec(), verify_hash],
        )
        .context("更新主密码配置失败")?;

        // 7. 提交事务
        tx.commit().context("提交事务失败")?;

        Ok(new_key)
    }

    /// 获取所有密码条目（加密版本，延迟解密）
    pub fn get_all_entries_encrypted(&self) -> Result<Vec<EncryptedPasswordEntry>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, name, url, username, password, iv, notes
                 FROM passwords ORDER BY name ASC",
            )
            .context("查询密码列表失败")?;

        let entries = stmt
            .query_map([], |row| {
                let id: i64 = row.get(0)?;
                let name: String = row.get(1)?;
                let url: Option<String> = row.get(2)?;
                let username: String = row.get(3)?;
                let encrypted_password: Vec<u8> = row.get(4)?;
                let iv: Vec<u8> = row.get(5)?;
                let notes: Option<String> = row.get(6)?;
                Ok(EncryptedPasswordEntry {
                    id,
                    name,
                    url,
                    username,
                    encrypted_password,
                    iv,
                    notes,
                })
            })
            .context("读取密码列表失败")?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(entries)
    }

    /// 获取所有密码条目（解密版本，兼容旧代码）
    pub fn get_all_entries(&self, key: &[u8; 32]) -> Result<Vec<PasswordEntry>> {
        let encrypted_entries = self.get_all_entries_encrypted()?;
        let entries: Vec<PasswordEntry> = encrypted_entries
            .iter()
            .map(|e| e.to_decrypted(key))
            .collect();
        Ok(entries)
    }

    /// 搜索密码条目（加密版本，延迟解密）
    pub fn search_entries_encrypted(&self, query: &str) -> Result<Vec<EncryptedPasswordEntry>> {
        let search_pattern = format!("%{}%", query);
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, name, url, username, password, iv, notes
                 FROM passwords
                 WHERE name LIKE ?1 OR username LIKE ?1 OR url LIKE ?1
                 ORDER BY name ASC",
            )
            .context("搜索密码失败")?;

        let entries = stmt
            .query_map(params![search_pattern], |row| {
                let id: i64 = row.get(0)?;
                let name: String = row.get(1)?;
                let url: Option<String> = row.get(2)?;
                let username: String = row.get(3)?;
                let encrypted_password: Vec<u8> = row.get(4)?;
                let iv: Vec<u8> = row.get(5)?;
                let notes: Option<String> = row.get(6)?;
                Ok(EncryptedPasswordEntry {
                    id,
                    name,
                    url,
                    username,
                    encrypted_password,
                    iv,
                    notes,
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
            "INSERT INTO passwords (name, url, username, password, iv, notes) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![entry.name, entry.url, entry.username, encrypted_pwd, iv, entry.notes],
        ).context("添加密码失败")?;

        Ok(self.conn.last_insert_rowid())
    }

    /// 更新密码条目
    pub fn update_entry(&self, entry: &PasswordEntry, key: &[u8; 32]) -> Result<()> {
        let (encrypted_pwd, iv) = crypto::encrypt_password(key, &entry.password)?;

        self.conn.execute(
            "UPDATE passwords SET name = ?1, url = ?2, username = ?3, password = ?4, iv = ?5, notes = ?6, updated_at = CURRENT_TIMESTAMP WHERE id = ?7",
            params![entry.name, entry.url, entry.username, encrypted_pwd, iv, entry.notes, entry.id],
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

    /// 导出所有密码条目
    ///
    /// 支持 JSON 和 CSV 两种格式。CSV 使用 Chrome 兼容的列头。
    pub fn export_entries(&self, key: &[u8; 32], format: ExportFormat) -> Result<String> {
        let entries = self.get_all_entries(key)?;

        let export_entries: Vec<ExportEntry> = entries
            .iter()
            .map(ExportEntry::from_password_entry)
            .collect();

        match format {
            ExportFormat::Json => {
                let export_data = ExportData {
                    version: "1.0".to_string(),
                    exported_at: chrono::Local::now().to_rfc3339(),
                    entries: export_entries,
                };
                serde_json::to_string_pretty(&export_data).context("序列化导出数据失败")
            }
            ExportFormat::Csv => {
                let mut wtr = csv::Writer::from_writer(vec![]);
                for entry in &export_entries {
                    wtr.serialize(CsvRecord::from_export_entry(entry))
                        .context("序列化 CSV 记录失败")?;
                }
                let data = wtr.into_inner().context("获取 CSV 数据失败")?;
                String::from_utf8(data).context("CSV 数据不是有效的 UTF-8 文本")
            }
        }
    }

    /// 从文件内容导入密码条目
    ///
    /// 根据 `format` 参数解析 JSON 或 CSV 格式的数据。
    /// JSON 格式兼容旧版 `website` 字段和新版 `name` 字段。
    /// CSV 格式兼容 Chrome 密码导出文件。
    pub fn import_entries(
        &self,
        content: &str,
        key: &[u8; 32],
        format: ExportFormat,
    ) -> Result<usize> {
        let new_entries = match format {
            ExportFormat::Json => self.parse_json_import(content)?,
            ExportFormat::Csv => self.parse_csv_import(content)?,
        };

        // 使用事务保证导入的原子性（RAII，Drop 时自动回滚）
        let tx = self
            .conn
            .unchecked_transaction()
            .context("开启导入事务失败")?;

        let mut imported = 0;
        for entry in &new_entries {
            let (encrypted_pwd, iv) = crypto::encrypt_password(key, &entry.password)?;

            tx.execute(
                "INSERT INTO passwords (name, url, username, password, iv, notes) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![entry.name, entry.url, entry.username, encrypted_pwd, iv, entry.notes],
            )
            .context("导入密码失败")?;
            imported += 1;
        }

        tx.commit().context("提交导入事务失败")?;
        Ok(imported)
    }

    /// 解析 JSON 格式的导入数据
    fn parse_json_import(&self, content: &str) -> Result<Vec<NewPasswordEntry>> {
        let export_data: ExportData =
            serde_json::from_str(content).context("解析 JSON 导入数据失败")?;

        if export_data.version != "1.0" {
            anyhow::bail!("不支持的导出数据版本: {}", export_data.version);
        }

        Ok(export_data
            .entries
            .iter()
            .map(|e| e.to_new_entry())
            .collect())
    }

    /// 解析 CSV 格式的导入数据
    ///
    /// 支持 Chrome 密码导出的 CSV 列头：name, url, username, password, note
    fn parse_csv_import(&self, content: &str) -> Result<Vec<NewPasswordEntry>> {
        let mut rdr = csv::ReaderBuilder::new()
            .has_headers(true)
            .from_reader(content.as_bytes());

        let headers = rdr.headers().context("读取 CSV 表头失败")?.clone();

        // 查找各列的索引
        let name_idx = find_column_index(&headers, &["name", "website", "网站", "名称"])
            .context("CSV 缺少 name 列")?;
        let url_idx = find_column_index(&headers, &["url", "网址"]);
        let username_idx = find_column_index(&headers, &["username", "user", "账号", "用户名"])
            .context("CSV 缺少 username 列")?;
        let password_idx = find_column_index(&headers, &["password", "pass", "密码"])
            .context("CSV 缺少 password 列")?;
        let notes_idx = find_column_index(&headers, &["note", "notes", "备注"]);

        let mut entries = Vec::new();
        for result in rdr.records() {
            let record = result.context("读取 CSV 记录失败")?;

            let name = record.get(name_idx).unwrap_or("").trim().to_string();
            let username = record.get(username_idx).unwrap_or("").trim().to_string();
            let password = record.get(password_idx).unwrap_or("").trim().to_string();

            if name.is_empty() || username.is_empty() || password.is_empty() {
                continue;
            }

            let url = url_idx
                .and_then(|i| record.get(i))
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty());
            let notes = notes_idx
                .and_then(|i| record.get(i))
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty());

            entries.push(NewPasswordEntry {
                name,
                url,
                username,
                password,
                notes,
            });
        }

        Ok(entries)
    }
}

/// 在 CSV 表头中查找匹配的列索引
fn find_column_index(headers: &csv::StringRecord, candidates: &[&str]) -> Option<usize> {
    for (i, header) in headers.iter().enumerate() {
        let h = header.trim().to_lowercase();
        if candidates.iter().any(|c| c.to_lowercase() == h) {
            return Some(i);
        }
    }
    None
}

/// Chrome CSV 格式的记录结构
#[derive(serde::Serialize)]
struct CsvRecord {
    name: String,
    url: String,
    username: String,
    password: String,
    note: String,
}

impl CsvRecord {
    fn from_export_entry(entry: &ExportEntry) -> Self {
        Self {
            name: entry.name.clone(),
            url: entry.url.clone().unwrap_or_default(),
            username: entry.username.clone(),
            password: entry.password.clone(),
            note: entry.notes.clone().unwrap_or_default(),
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    /// 创建测试用的内存数据库
    fn setup_test_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE master_config (
                id          INTEGER PRIMARY KEY DEFAULT 1,
                salt        BLOB NOT NULL,
                verify_hash BLOB NOT NULL,
                created_at  DATETIME DEFAULT CURRENT_TIMESTAMP
            );
            CREATE TABLE passwords (
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                name        TEXT NOT NULL,
                url         TEXT,
                username    TEXT NOT NULL,
                password    BLOB NOT NULL,
                iv          BLOB NOT NULL,
                notes       TEXT,
                created_at  DATETIME DEFAULT CURRENT_TIMESTAMP,
                updated_at  DATETIME DEFAULT CURRENT_TIMESTAMP
            );",
        )
        .unwrap();
        conn
    }

    /// 测试 JSON 导出导入往返
    #[test]
    fn test_export_import_json_roundtrip() {
        let conn = setup_test_db();
        let store = PasswordStore::new(&conn);

        let salt = crypto::generate_salt();
        let key = crypto::derive_key("test_master", &salt);

        // 添加测试数据
        let entry1 = NewPasswordEntry {
            name: "GitHub".to_string(),
            url: Some("https://github.com".to_string()),
            username: "user1".to_string(),
            password: "pass1".to_string(),
            notes: Some("代码托管".to_string()),
        };
        let entry2 = NewPasswordEntry {
            name: "Google".to_string(),
            url: Some("https://google.com".to_string()),
            username: "user2@gmail.com".to_string(),
            password: "pass2".to_string(),
            notes: None,
        };
        store.add_entry(&entry1, &key).unwrap();
        store.add_entry(&entry2, &key).unwrap();

        // 导出为 JSON
        let json = store.export_entries(&key, ExportFormat::Json).unwrap();
        assert!(json.contains("GitHub"));
        assert!(json.contains("Google"));

        // 清空数据
        store.conn.execute("DELETE FROM passwords", []).unwrap();
        assert_eq!(store.get_all_entries(&key).unwrap().len(), 0);

        // 从 JSON 导入
        let count = store
            .import_entries(&json, &key, ExportFormat::Json)
            .unwrap();
        assert_eq!(count, 2);

        // 验证导入的数据
        let entries = store.get_all_entries(&key).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].name, "GitHub");
        assert_eq!(entries[0].url, Some("https://github.com".to_string()));
        assert_eq!(entries[0].username, "user1");
        assert_eq!(entries[0].password, "pass1");
        assert_eq!(entries[0].notes, Some("代码托管".to_string()));
        assert_eq!(entries[1].name, "Google");
        assert_eq!(entries[1].password, "pass2");
    }

    /// 测试 CSV 导出导入往返
    #[test]
    fn test_export_import_csv_roundtrip() {
        let conn = setup_test_db();
        let store = PasswordStore::new(&conn);

        let salt = crypto::generate_salt();
        let key = crypto::derive_key("test_master", &salt);

        // 添加测试数据
        let entry = NewPasswordEntry {
            name: "GitHub".to_string(),
            url: Some("https://github.com".to_string()),
            username: "dev_user".to_string(),
            password: "secret123".to_string(),
            notes: Some("开发账号".to_string()),
        };
        store.add_entry(&entry, &key).unwrap();

        // 导出为 CSV
        let csv_data = store.export_entries(&key, ExportFormat::Csv).unwrap();
        assert!(csv_data.contains("name"));
        assert!(csv_data.contains("GitHub"));

        // 清空数据
        store.conn.execute("DELETE FROM passwords", []).unwrap();
        assert_eq!(store.get_all_entries(&key).unwrap().len(), 0);

        // 从 CSV 导入
        let count = store
            .import_entries(&csv_data, &key, ExportFormat::Csv)
            .unwrap();
        assert_eq!(count, 1);

        // 验证导入的数据
        let entries = store.get_all_entries(&key).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "GitHub");
        assert_eq!(entries[0].url, Some("https://github.com".to_string()));
        assert_eq!(entries[0].username, "dev_user");
        assert_eq!(entries[0].password, "secret123");
        assert_eq!(entries[0].notes, Some("开发账号".to_string()));
    }

    /// 测试兼容 Chrome CSV 格式导入
    #[test]
    fn test_import_chrome_csv() {
        let conn = setup_test_db();
        let store = PasswordStore::new(&conn);

        let salt = crypto::generate_salt();
        let key = crypto::derive_key("test_master", &salt);

        // 模拟 Chrome 导出的 CSV
        let chrome_csv = "\
name,url,username,password,note
Google,https://google.com,user@gmail.com,chrome_pass1,个人账号
GitHub,https://github.com,dev_user,chrome_pass2,
";

        let count = store
            .import_entries(chrome_csv, &key, ExportFormat::Csv)
            .unwrap();
        assert_eq!(count, 2);

        let entries = store.get_all_entries(&key).unwrap();
        // 按 name ASC 排序，GitHub 在前
        assert_eq!(entries[0].name, "GitHub");
        assert_eq!(entries[0].url, Some("https://github.com".to_string()));
        assert_eq!(entries[0].username, "dev_user");
        assert_eq!(entries[0].password, "chrome_pass2");
        assert_eq!(entries[0].notes, None);
        assert_eq!(entries[1].name, "Google");
        assert_eq!(entries[1].password, "chrome_pass1");
        assert_eq!(entries[1].notes, Some("个人账号".to_string()));
    }

    /// 测试兼容旧版 JSON 格式（website 字段）导入
    #[test]
    fn test_import_legacy_json_with_website() {
        let conn = setup_test_db();
        let store = PasswordStore::new(&conn);

        let salt = crypto::generate_salt();
        let key = crypto::derive_key("test_master", &salt);

        // 模拟旧版导出的 JSON（使用 website 字段）
        let legacy_json = r#"{
            "version": "1.0",
            "exported_at": "2024-01-01T00:00:00+08:00",
            "entries": [
                {
                    "website": "旧版网站",
                    "url": "https://old.example.com",
                    "username": "old_user",
                    "password": "old_pass",
                    "notes": "旧版数据"
                }
            ]
        }"#;

        let count = store
            .import_entries(legacy_json, &key, ExportFormat::Json)
            .unwrap();
        assert_eq!(count, 1);

        let entries = store.get_all_entries(&key).unwrap();
        assert_eq!(entries[0].name, "旧版网站");
        assert_eq!(entries[0].password, "old_pass");
    }

    /// 测试修改主密码
    #[test]
    fn test_change_master_password() {
        let conn = setup_test_db();
        let store = PasswordStore::new(&conn);

        // 设置初始主密码
        let old_key = store.setup_master_password("old_password").unwrap();

        // 添加测试数据
        let entry = NewPasswordEntry {
            name: "TestSite".to_string(),
            url: Some("https://test.com".to_string()),
            username: "test_user".to_string(),
            password: "my_secret".to_string(),
            notes: None,
        };
        store.add_entry(&entry, &old_key).unwrap();

        // 修改主密码
        let new_key = store
            .change_master_password("old_password", "new_password")
            .unwrap();

        // 新密钥与旧密钥不同
        assert_ne!(old_key, new_key);

        // 用新密钥能正确解密数据
        let entries = store.get_all_entries(&new_key).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "TestSite");
        assert_eq!(entries[0].password, "my_secret");

        // 旧密码验证失败（返回错误）
        let result = store.verify_master_password("old_password");
        assert!(result.is_err(), "旧密码验证应失败");

        // 新密码验证成功
        let result = store.verify_master_password("new_password").unwrap();
        assert!(result.is_some(), "新密码验证应成功");
    }

    /// 测试修改主密码时旧密码错误
    #[test]
    fn test_change_master_password_wrong_old_password() {
        let conn = setup_test_db();
        let store = PasswordStore::new(&conn);

        store.setup_master_password("correct_password").unwrap();

        // 旧密码错误
        let result = store.change_master_password("wrong_password", "new_password");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("主密码错误"));

        // 原密码仍然有效
        let result = store.verify_master_password("correct_password").unwrap();
        assert!(result.is_some());
    }

    /// 测试修改主密码时新旧密码相同
    #[test]
    fn test_change_master_password_same_password() {
        let conn = setup_test_db();
        let store = PasswordStore::new(&conn);

        store.setup_master_password("same_password").unwrap();

        let result = store.change_master_password("same_password", "same_password");
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("新密码不能与旧密码相同")
        );
    }
}
