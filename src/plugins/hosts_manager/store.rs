use anyhow::{Context, Result, bail};
use rusqlite::{Connection, params};

/// 环境数据
#[derive(Debug, Clone)]
pub struct Environment {
    pub id: i64,
    pub name: String,
    pub is_active: bool,
}

/// Hosts 条目（数据库）
#[derive(Debug, Clone)]
pub struct DbHostsEntry {
    pub id: i64,
    pub ip_address: String,
    pub hostname: String,
    pub comment: Option<String>,
    pub is_enabled: bool,
}

/// 新增条目表单
#[derive(Debug, Clone)]
pub struct NewHostsEntry {
    pub ip_address: String,
    pub hostname: String,
    pub comment: Option<String>,
}

/// 条目编辑表单
#[derive(Debug, Clone)]
pub struct HostsEntryForm {
    pub ip_address: String,
    pub hostname: String,
    pub comment: String,
}

impl HostsEntryForm {
    pub fn new() -> Self {
        Self {
            ip_address: String::new(),
            hostname: String::new(),
            comment: String::new(),
        }
    }

    pub fn from_entry(entry: &DbHostsEntry) -> Self {
        Self {
            ip_address: entry.ip_address.clone(),
            hostname: entry.hostname.clone(),
            comment: entry.comment.clone().unwrap_or_default(),
        }
    }

    /// 校验并转换为新增条目（字段会去除首尾空白）
    pub fn to_new_entry(&self) -> Result<NewHostsEntry> {
        normalize_new_entry(&NewHostsEntry {
            ip_address: self.ip_address.clone(),
            hostname: self.hostname.clone(),
            comment: Some(self.comment.clone()),
        })
    }
}

/// 校验并规范化条目字段
///
/// 字段会作为一行写入系统 hosts 文件，因此去掉首尾空白，
/// 并拒绝会破坏行结构或改变条目语义的字符
fn normalize_new_entry(entry: &NewHostsEntry) -> Result<NewHostsEntry> {
    let ip_address = entry.ip_address.trim();
    if ip_address.is_empty() {
        bail!("请输入 IP 地址");
    }
    validate_host_field(ip_address, "IP 地址")?;

    let hostname = entry.hostname.trim();
    if hostname.is_empty() {
        bail!("请输入主机名");
    }
    validate_host_field(hostname, "主机名")?;

    let comment = entry
        .comment
        .as_deref()
        .map(str::trim)
        .filter(|comment| !comment.is_empty());
    if let Some(comment) = comment {
        validate_comment_field(comment)?;
    }

    Ok(NewHostsEntry {
        ip_address: ip_address.to_string(),
        hostname: hostname.to_string(),
        comment: comment.map(str::to_string),
    })
}

/// 校验 IP / 主机名：空白与井号会改变条目语义或截断内容，必须拒绝
fn validate_host_field(value: &str, field_name: &str) -> Result<()> {
    if value.chars().any(char::is_control) {
        bail!("{}不能包含换行或控制字符", field_name);
    }

    if value.chars().any(|c| c.is_whitespace() || c == '#') {
        bail!("{}不能包含空格或井号", field_name);
    }

    Ok(())
}

/// 校验备注：备注位于行尾、井号可正常往返，只需拒绝破坏行结构的控制字符
fn validate_comment_field(comment: &str) -> Result<()> {
    if comment.chars().any(char::is_control) {
        bail!("备注不能包含换行或控制字符");
    }

    Ok(())
}

/// 环境编辑表单
#[derive(Debug, Clone)]
pub struct EnvironmentForm {
    pub name: String,
}

impl EnvironmentForm {
    pub fn new() -> Self {
        Self {
            name: String::new(),
        }
    }

    pub fn from_env(env: &Environment) -> Self {
        Self {
            name: env.name.clone(),
        }
    }

    /// 获取名称校验失败的原因（名称合法时返回 None）
    pub fn validation_error(&self) -> Option<String> {
        normalize_environment_name(&self.name)
            .err()
            .map(|err| err.to_string())
    }
}

/// 校验并规范化环境名称
///
/// 环境名称会作为注释标题写入系统 hosts 文件，因此不允许为空，
/// 也不能包含换行、井号等会破坏配置结构的字符；首尾空白会被去除
fn normalize_environment_name(name: &str) -> Result<&str> {
    let name = name.trim();

    if name.is_empty() {
        bail!("请输入环境名称");
    }

    if name.chars().any(|c| c.is_control() || c == '#') {
        bail!("环境名称不能包含换行、井号或控制字符");
    }

    Ok(name)
}

/// Hosts 数据库操作
pub struct HostsStore<'a> {
    conn: &'a Connection,
}

impl<'a> HostsStore<'a> {
    pub fn new(conn: &'a Connection) -> Self {
        Self { conn }
    }

    // ========== 环境管理 ==========

    /// 获取所有环境
    pub fn get_all_environments(&self) -> Result<Vec<Environment>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, name, is_active
                 FROM hosts_environments ORDER BY name ASC",
            )
            .context("查询环境列表失败")?;

        let envs = stmt
            .query_map([], |row| {
                Ok(Environment {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    is_active: row.get(2)?,
                })
            })
            .context("读取环境列表失败")?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(envs)
    }

    /// 获取所有已启用的环境（按名称升序）
    ///
    /// 允许同时存在多个已启用环境，应用时按返回顺序合并写入 hosts 文件
    pub fn get_active_environments(&self) -> Result<Vec<Environment>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, name, is_active
                 FROM hosts_environments WHERE is_active = TRUE ORDER BY name ASC",
            )
            .context("查询启用环境失败")?;

        let envs = stmt
            .query_map([], |row| {
                Ok(Environment {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    is_active: row.get(2)?,
                })
            })
            .context("读取启用环境失败")?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(envs)
    }

    /// 添加环境
    pub fn add_environment(&self, name: &str) -> Result<i64> {
        let name = normalize_environment_name(name)?;

        self.conn
            .execute(
                "INSERT INTO hosts_environments (name) VALUES (?1)",
                params![name],
            )
            .context("添加环境失败")?;

        Ok(self.conn.last_insert_rowid())
    }

    /// 更新环境名称
    pub fn update_environment(&self, id: i64, name: &str) -> Result<()> {
        let name = normalize_environment_name(name)?;

        let affected = self
            .conn
            .execute(
                "UPDATE hosts_environments SET name = ?1, updated_at = CURRENT_TIMESTAMP WHERE id = ?2",
                params![name, id],
            )
            .context("更新环境失败")?;

        if affected == 0 {
            bail!("环境不存在: id={}", id);
        }

        Ok(())
    }

    /// 删除环境
    pub fn delete_environment(&self, id: i64) -> Result<()> {
        let affected = self
            .conn
            .execute("DELETE FROM hosts_environments WHERE id = ?1", params![id])
            .context("删除环境失败")?;

        if affected == 0 {
            bail!("环境不存在: id={}", id);
        }

        Ok(())
    }

    /// 设置指定环境的启用状态
    ///
    /// 只修改目标环境，不影响其他环境，因此允许多个环境同时生效
    pub fn set_environment_active(&self, id: i64, active: bool) -> Result<()> {
        let affected = self
            .conn
            .execute(
                "UPDATE hosts_environments SET is_active = ?1, updated_at = CURRENT_TIMESTAMP WHERE id = ?2",
                params![active, id],
            )
            .context("更新环境状态失败")?;

        if affected == 0 {
            bail!("环境不存在: id={}", id);
        }

        Ok(())
    }

    // ========== 条目管理 ==========

    /// 获取指定环境的所有条目
    pub fn get_entries_by_env(&self, env_id: i64) -> Result<Vec<DbHostsEntry>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, ip_address, hostname, comment, is_enabled
                 FROM hosts_entries
                 WHERE environment_id = ?1
                 ORDER BY hostname ASC",
            )
            .context("查询条目列表失败")?;

        let entries = stmt
            .query_map(params![env_id], |row| {
                Ok(DbHostsEntry {
                    id: row.get(0)?,
                    ip_address: row.get(1)?,
                    hostname: row.get(2)?,
                    comment: row.get(3)?,
                    is_enabled: row.get(4)?,
                })
            })
            .context("读取条目列表失败")?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(entries)
    }

    /// 添加条目
    pub fn add_entry(&self, env_id: i64, entry: &NewHostsEntry) -> Result<i64> {
        let entry = normalize_new_entry(entry)?;

        self.conn
            .execute(
                "INSERT INTO hosts_entries (environment_id, ip_address, hostname, comment) VALUES (?1, ?2, ?3, ?4)",
                params![env_id, entry.ip_address, entry.hostname, entry.comment],
            )
            .context("添加条目失败")?;

        Ok(self.conn.last_insert_rowid())
    }

    /// 更新条目
    pub fn update_entry(
        &self,
        id: i64,
        ip: &str,
        hostname: &str,
        comment: &Option<String>,
    ) -> Result<()> {
        let entry = normalize_new_entry(&NewHostsEntry {
            ip_address: ip.to_string(),
            hostname: hostname.to_string(),
            comment: comment.clone(),
        })?;

        let affected = self
            .conn
            .execute(
                "UPDATE hosts_entries SET ip_address = ?1, hostname = ?2, comment = ?3 WHERE id = ?4",
                params![entry.ip_address, entry.hostname, entry.comment, id],
            )
            .context("更新条目失败")?;

        if affected == 0 {
            bail!("条目不存在: id={}", id);
        }

        Ok(())
    }

    /// 删除条目
    pub fn delete_entry(&self, id: i64) -> Result<()> {
        let affected = self
            .conn
            .execute("DELETE FROM hosts_entries WHERE id = ?1", params![id])
            .context("删除条目失败")?;

        if affected == 0 {
            bail!("条目不存在: id={}", id);
        }

        Ok(())
    }

    /// 切换条目启用状态
    pub fn toggle_entry(&self, id: i64, enabled: bool) -> Result<()> {
        let affected = self
            .conn
            .execute(
                "UPDATE hosts_entries SET is_enabled = ?1 WHERE id = ?2",
                params![enabled, id],
            )
            .context("切换条目状态失败")?;

        if affected == 0 {
            bail!("条目不存在: id={}", id);
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 创建测试用的内存数据库并初始化 hosts 相关表
    fn setup_conn() -> Result<Connection> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch(
            "CREATE TABLE hosts_environments (
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                name        TEXT NOT NULL UNIQUE,
                is_active   BOOLEAN DEFAULT FALSE,
                created_at  DATETIME DEFAULT CURRENT_TIMESTAMP,
                updated_at  DATETIME DEFAULT CURRENT_TIMESTAMP
            );
            CREATE TABLE hosts_entries (
                id              INTEGER PRIMARY KEY AUTOINCREMENT,
                environment_id  INTEGER NOT NULL,
                ip_address      TEXT NOT NULL,
                hostname        TEXT NOT NULL,
                comment         TEXT,
                is_enabled      BOOLEAN DEFAULT TRUE,
                sort_order      INTEGER DEFAULT 0
            );",
        )?;
        Ok(conn)
    }

    #[test]
    fn set_environment_active_only_affects_target() -> Result<()> {
        let conn = setup_conn()?;
        let store = HostsStore::new(&conn);

        let dev_id = store.add_environment("dev")?;
        let test_id = store.add_environment("test")?;
        let prod_id = store.add_environment("prod")?;

        // 三个环境可以同时启用
        store.set_environment_active(dev_id, true)?;
        store.set_environment_active(test_id, true)?;
        store.set_environment_active(prod_id, true)?;
        assert_eq!(store.get_active_environments()?.len(), 3);

        // 禁用其中一个，其余环境仍然保持启用
        store.set_environment_active(test_id, false)?;
        let active = store.get_active_environments()?;
        assert_eq!(
            active
                .iter()
                .map(|env| env.name.as_str())
                .collect::<Vec<_>>(),
            vec!["dev", "prod"]
        );
        assert!(active.iter().all(|env| env.is_active));

        Ok(())
    }

    #[test]
    fn set_environment_active_reports_missing_environment() {
        let conn = match setup_conn() {
            Ok(conn) => conn,
            Err(e) => panic!("创建测试数据库失败: {}", e),
        };
        let store = HostsStore::new(&conn);

        assert!(store.set_environment_active(999, true).is_err());
    }

    #[test]
    fn add_environment_rejects_invalid_name() {
        let conn = match setup_conn() {
            Ok(conn) => conn,
            Err(e) => panic!("创建测试数据库失败: {}", e),
        };
        let store = HostsStore::new(&conn);

        assert!(store.add_environment("   ").is_err());
        assert!(store.add_environment("dev\nprod").is_err());
        assert!(store.add_environment("dev # 注入").is_err());
        assert!(store.add_environment("dev").is_ok());
    }

    #[test]
    fn add_environment_trims_name() -> Result<()> {
        let conn = setup_conn()?;
        let store = HostsStore::new(&conn);

        let id = store.add_environment("  dev  ")?;
        let envs = store.get_all_environments()?;

        assert_eq!(envs.len(), 1);
        assert_eq!(envs[0].name, "dev");
        // 首尾空白去除后与已有环境重名，应被唯一约束拒绝
        assert!(store.add_environment(" dev ").is_err());
        assert!(store.update_environment(id, "dev ").is_ok());

        Ok(())
    }

    #[test]
    fn delete_environment_reports_missing_environment() -> Result<()> {
        let conn = setup_conn()?;
        let store = HostsStore::new(&conn);

        let id = store.add_environment("dev")?;
        assert!(store.delete_environment(id).is_ok());
        assert!(store.delete_environment(id).is_err());

        Ok(())
    }

    #[test]
    fn environment_form_validation_error() {
        let mut form = EnvironmentForm::new();
        assert!(form.validation_error().is_some());

        form.name = "dev\n# <<< Tools Box END <<<".to_string();
        assert!(form.validation_error().is_some());

        form.name = "dev".to_string();
        assert!(form.validation_error().is_none());
    }

    #[test]
    fn entry_form_rejects_invalid_fields() {
        let mut form = HostsEntryForm::new();
        assert!(form.to_new_entry().is_err());

        // 换行会破坏 hosts 行结构，必须拒绝
        form.ip_address = "192.168.1.100\n10.0.0.1".to_string();
        form.hostname = "dev.api.com".to_string();
        assert!(form.to_new_entry().is_err());

        // 主机名中的空格会改变条目语义
        form.ip_address = "192.168.1.100".to_string();
        form.hostname = "dev api.com".to_string();
        assert!(form.to_new_entry().is_err());

        // 主机名中的井号会截断条目内容
        form.hostname = "dev.api.com#".to_string();
        assert!(form.to_new_entry().is_err());

        // 备注中的换行同样会破坏行结构
        form.hostname = "dev.api.com".to_string();
        form.comment = "备注\n# <<< Tools Box END <<<".to_string();
        assert!(form.to_new_entry().is_err());
    }

    #[test]
    fn entry_form_trims_fields() -> Result<()> {
        let form = HostsEntryForm {
            ip_address: "  192.168.1.100  ".to_string(),
            hostname: " dev.api.com ".to_string(),
            comment: "  API # 服务器  ".to_string(),
        };

        let entry = form.to_new_entry()?;
        assert_eq!(entry.ip_address, "192.168.1.100");
        assert_eq!(entry.hostname, "dev.api.com");
        // 备注位于行尾，可以包含井号
        assert_eq!(entry.comment.as_deref(), Some("API # 服务器"));

        // 空备注存储为 None
        let form = HostsEntryForm {
            ip_address: "192.168.1.100".to_string(),
            hostname: "dev.api.com".to_string(),
            comment: "   ".to_string(),
        };
        assert!(form.to_new_entry()?.comment.is_none());

        Ok(())
    }

    #[test]
    fn add_entry_normalizes_and_rejects_invalid_fields() -> Result<()> {
        let conn = setup_conn()?;
        let store = HostsStore::new(&conn);
        let env_id = store.add_environment("dev")?;

        let invalid = NewHostsEntry {
            ip_address: "192.168.1.100".to_string(),
            hostname: "dev.api.com\n# <<< Tools Box END <<<".to_string(),
            comment: None,
        };
        assert!(store.add_entry(env_id, &invalid).is_err());

        // 入库前去除首尾空白
        let valid = NewHostsEntry {
            ip_address: "  192.168.1.100 ".to_string(),
            hostname: " dev.api.com".to_string(),
            comment: Some("  API 服务器  ".to_string()),
        };
        let id = store.add_entry(env_id, &valid)?;

        let entries = store.get_entries_by_env(env_id)?;
        assert_eq!(entries[0].ip_address, "192.168.1.100");
        assert_eq!(entries[0].hostname, "dev.api.com");
        assert_eq!(entries[0].comment.as_deref(), Some("API 服务器"));

        // 更新同样拒绝非法字段
        assert!(
            store
                .update_entry(id, "10.0.0.1", "dev.api.com", &None)
                .is_ok()
        );
        assert!(
            store
                .update_entry(id, "10.0.0.1\n10.0.0.2", "dev.api.com", &None)
                .is_err()
        );

        // 不存在的条目应报错
        assert!(
            store
                .update_entry(999, "10.0.0.1", "dev.api.com", &None)
                .is_err()
        );
        assert!(store.delete_entry(999).is_err());
        assert!(store.toggle_entry(999, true).is_err());

        Ok(())
    }
}
