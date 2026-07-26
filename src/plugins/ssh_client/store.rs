use anyhow::{Context, Result};
use rusqlite::{Connection, params};

use super::models::{AuthMethod, NewSession, SshSession};

/// SSH 会话数据存储层
pub struct SshStore<'a> {
    conn: &'a Connection,
}

impl<'a> SshStore<'a> {
    /// 创建新的 SSH 会话存储实例
    pub fn new(conn: &'a Connection) -> Self {
        Self { conn }
    }

    /// 获取所有会话，按 sort_order 排序
    pub fn list_sessions(&self) -> Result<Vec<SshSession>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, name, host, port, username, auth_type, auth_data
                 FROM ssh_sessions
                 ORDER BY id",
            )
            .context("准备查询语句失败")?;

        let rows = stmt.query_map([], |row| {
            let id: i64 = row.get(0)?;
            let name: String = row.get(1)?;
            let host: String = row.get(2)?;
            let port_i64: i64 = row.get(3)?;
            let username: String = row.get(4)?;
            let auth_type: String = row.get(5)?;
            let auth_data: String = row.get(6)?;
            Ok((id, name, host, port_i64, username, auth_type, auth_data))
        })?;

        let mut sessions = Vec::new();
        for row in rows {
            let (id, name, host, port_i64, username, auth_type, auth_data) =
                match row.context("读取会话记录失败") {
                    Ok(r) => r,
                    Err(e) => {
                        log::warn!("跳过一条损坏的 SSH 连接记录: {}", e);
                        continue;
                    }
                };
            let port = match u16::try_from(port_i64) {
                Ok(p) => p,
                Err(_) => {
                    log::warn!(
                        "SSH 连接 '{}' (id={}) 端口值 {} 无效，跳过",
                        name,
                        id,
                        port_i64
                    );
                    continue;
                }
            };
            let auth_method = match Self::parse_auth(&auth_type, &auth_data) {
                Ok(m) => m,
                Err(e) => {
                    log::warn!(
                        "SSH 连接 '{}' (id={}) {} 解析失败，跳过: {}",
                        name,
                        id,
                        auth_type,
                        e
                    );
                    continue;
                }
            };
            sessions.push(SshSession {
                id,
                name,
                host,
                port,
                username,
                auth_method,
            });
        }
        Ok(sessions)
    }

    /// 新增连接配置（host+username 唯一）
    pub fn insert_session(&self, session: &NewSession) -> Result<i64> {
        // 检查是否已存在同主机同用户的配置
        let exists: bool = self
            .conn
            .query_row(
                "SELECT COUNT(*) > 0 FROM ssh_sessions WHERE host = ?1 AND username = ?2",
                params![session.host, session.username],
                |row| row.get(0),
            )
            .context("查询重复配置失败")?;
        if exists {
            anyhow::bail!(
                "连接配置已存在: {}@{}，请通过编辑按钮更新现有配置",
                session.username,
                session.host
            );
        }

        let auth_type = session.auth_method.type_str().to_string();
        let auth_data = Self::serialize_auth(&session.auth_method)?;
        let max_order: i32 = self
            .conn
            .query_row(
                "SELECT COALESCE(MAX(sort_order), -1) FROM ssh_sessions",
                [],
                |row| row.get(0),
            )
            .context("查询排序序号失败")?;

        self.conn
            .execute(
                "INSERT INTO ssh_sessions (name, host, port, username, auth_type, auth_data, sort_order)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    session.name,
                    session.host,
                    i64::from(session.port),
                    session.username,
                    auth_type,
                    auth_data,
                    max_order + 1,
                ],
            )
            .context("保存连接失败")?;

        Ok(self.conn.last_insert_rowid())
    }

    /// 更新会话
    pub fn update_session(&self, id: i64, session: &NewSession) -> Result<()> {
        let auth_type = session.auth_method.type_str().to_string();
        let auth_data = Self::serialize_auth(&session.auth_method)?;

        let affected = self
            .conn
            .execute(
                "UPDATE ssh_sessions
                 SET name = ?1, host = ?2, port = ?3, username = ?4, auth_type = ?5, auth_data = ?6, updated_at = CURRENT_TIMESTAMP
                 WHERE id = ?7",
                params![
                    session.name,
                    session.host,
                    i64::from(session.port),
                    session.username,
                    auth_type,
                    auth_data,
                    id,
                ],
            )
            .context("更新连接失败")?;

        if affected == 0 {
            anyhow::bail!("未找到 id 为 {} 的连接", id);
        }
        log::info!("SSH 连接 '{}' (id={}) 已更新", session.name, id);
        Ok(())
    }

    /// 删除会话
    pub fn delete_session(&self, id: i64) -> Result<()> {
        let affected = self
            .conn
            .execute("DELETE FROM ssh_sessions WHERE id = ?1", params![id])
            .context("删除连接失败")?;

        if affected == 0 {
            anyhow::bail!("未找到 id 为 {} 的连接", id);
        }
        log::info!("SSH 连接 (id={}) 已删除", id);
        Ok(())
    }

    // ===================================================================
    // auth_data 序列化/反序列化
    // ===================================================================

    /// 将 AuthMethod 序列化为 JSON 存储
    fn serialize_auth(auth: &AuthMethod) -> Result<String> {
        match auth {
            AuthMethod::Password {
                encrypted_password,
                iv,
                salt,
            } => {
                let json = serde_json::json!({
                    "type": "password",
                    "encrypted_password": base64_encode(encrypted_password),
                    "iv": base64_encode(iv),
                    "salt": base64_encode(salt),
                });
                Ok(json.to_string())
            }
            AuthMethod::KeyFile {
                private_key_path,
                encrypted_passphrase,
            } => {
                let passphrase = encrypted_passphrase.as_ref().map(|(ct, iv, salt)| {
                    serde_json::json!({
                        "encrypted_passphrase": base64_encode(ct),
                        "iv": base64_encode(iv),
                        "salt": base64_encode(salt),
                    })
                });
                let json = serde_json::json!({
                    "type": "keyfile",
                    "private_key_path": private_key_path,
                    "passphrase": passphrase,
                });
                Ok(json.to_string())
            }
        }
    }

    /// 从 JSON 反序列化 AuthMethod
    fn parse_auth(auth_type: &str, auth_data: &str) -> Result<AuthMethod> {
        let data: serde_json::Value = serde_json::from_str(auth_data)
            .with_context(|| format!("解析 auth_data 失败: {}", auth_data))?;
        match auth_type {
            "password" => {
                let enc_pw = data["encrypted_password"]
                    .as_str()
                    .and_then(base64_decode)
                    .ok_or_else(|| anyhow::anyhow!("auth_data 缺少 encrypted_password 字段"))?;
                let iv = data["iv"]
                    .as_str()
                    .and_then(base64_decode)
                    .ok_or_else(|| anyhow::anyhow!("auth_data 缺少 iv 字段"))?;
                let salt = data["salt"]
                    .as_str()
                    .and_then(base64_decode)
                    .ok_or_else(|| anyhow::anyhow!("auth_data 缺少 salt 字段"))?;
                Ok(AuthMethod::Password {
                    encrypted_password: enc_pw,
                    iv,
                    salt,
                })
            }
            "keyfile" => {
                let path = data["private_key_path"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("auth_data 缺少 private_key_path 字段"))?
                    .to_string();
                let passphrase = if let Some(obj) = data["passphrase"].as_object() {
                    let ct = obj["encrypted_passphrase"].as_str().and_then(base64_decode);
                    let iv = obj["iv"].as_str().and_then(base64_decode);
                    let salt = obj["salt"].as_str().and_then(base64_decode);
                    match (ct, iv, salt) {
                        (Some(c), Some(i), Some(s)) => Some((c, i, s)),
                        _ => None,
                    }
                } else {
                    None
                };
                Ok(AuthMethod::KeyFile {
                    private_key_path: path,
                    encrypted_passphrase: passphrase,
                })
            }
            _ => anyhow::bail!("未知的认证类型: {}", auth_type),
        }
    }
}

// ===================================================================
// Base64 编解码（查找表实现，O(1) 解码）
// ===================================================================

const BASE64_ALPHABET: &[u8; 64] =
    b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn base64_encode(data: &[u8]) -> String {
    let mut result = String::with_capacity((data.len() + 2) / 3 * 4);
    for chunk in data.chunks(3) {
        let b0 = u32::from(chunk[0]);
        let b1 = chunk.get(1).copied().map(u32::from).unwrap_or(0);
        let b2 = chunk.get(2).copied().map(u32::from).unwrap_or(0);
        let triple = (b0 << 16) | (b1 << 8) | b2;
        result.push(BASE64_ALPHABET[((triple >> 18) & 63) as usize] as char);
        result.push(BASE64_ALPHABET[((triple >> 12) & 63) as usize] as char);
        if chunk.len() > 1 {
            result.push(BASE64_ALPHABET[((triple >> 6) & 63) as usize] as char);
        } else {
            result.push('=');
        }
        if chunk.len() > 2 {
            result.push(BASE64_ALPHABET[(triple & 63) as usize] as char);
        } else {
            result.push('=');
        }
    }
    result
}

fn base64_decode(input: &str) -> Option<Vec<u8>> {
    // 构建反向查找表
    static DECODE_TABLE: std::sync::LazyLock<[u8; 256]> = std::sync::LazyLock::new(|| {
        let mut table = [0xFFu8; 256];
        for (i, &c) in BASE64_ALPHABET.iter().enumerate() {
            table[c as usize] = i as u8;
        }
        table
    });

    let input = input.trim_end_matches('=');
    let mut result = Vec::with_capacity(input.len() * 3 / 4);
    let mut buffer = 0u32;
    let mut bits = 0u32;

    for &byte in input.as_bytes() {
        let idx = DECODE_TABLE[byte as usize];
        if idx == 0xFF {
            return None; // 非法字符
        }
        buffer = (buffer << 6) | u32::from(idx);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            result.push((buffer >> bits) as u8);
            buffer &= (1 << bits) - 1;
        }
    }
    Some(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_base64_encode_decode_roundtrip() {
        let data = b"Hello, World! Base64 test 123";
        let encoded = base64_encode(data);
        let decoded = base64_decode(&encoded).expect("解码应成功");
        assert_eq!(decoded, data);
    }

    #[test]
    fn test_base64_encode_empty() {
        let encoded = base64_encode(b"");
        assert_eq!(encoded, "");
    }

    #[test]
    fn test_base64_decode_padding() {
        // "a" → "YQ==", "ab" → "YWI=", "abc" → "YWJj"
        assert_eq!(base64_decode("YQ=="), Some(b"a".to_vec()));
        assert_eq!(base64_decode("YWI="), Some(b"ab".to_vec()));
        assert_eq!(base64_decode("YWJj"), Some(b"abc".to_vec()));
    }

    #[test]
    fn test_base64_decode_invalid_char() {
        assert_eq!(base64_decode("!!!"), None);
    }
}
