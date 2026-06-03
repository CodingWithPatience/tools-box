use anyhow::{Context, Result};
use rusqlite::{params, Connection};

/// 环境数据
#[derive(Debug, Clone)]
pub struct Environment {
    pub id: i64,
    pub name: String,
    pub is_active: bool,
    pub created_at: String,
    pub updated_at: String,
}

/// Hosts 条目（数据库）
#[derive(Debug, Clone)]
pub struct DbHostsEntry {
    pub id: i64,
    pub environment_id: i64,
    pub ip_address: String,
    pub hostname: String,
    pub comment: Option<String>,
    pub is_enabled: bool,
    pub sort_order: i32,
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

    pub fn to_new_entry(&self) -> NewHostsEntry {
        NewHostsEntry {
            ip_address: self.ip_address.clone(),
            hostname: self.hostname.clone(),
            comment: if self.comment.is_empty() {
                None
            } else {
                Some(self.comment.clone())
            },
        }
    }

    pub fn is_valid(&self) -> bool {
        !self.ip_address.is_empty() && !self.hostname.is_empty()
    }
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

    pub fn is_valid(&self) -> bool {
        !self.name.is_empty()
    }
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
                "SELECT id, name, is_active, created_at, updated_at
                 FROM hosts_environments ORDER BY name ASC",
            )
            .context("查询环境列表失败")?;

        let envs = stmt
            .query_map([], |row| {
                Ok(Environment {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    is_active: row.get(2)?,
                    created_at: row.get(3)?,
                    updated_at: row.get(4)?,
                })
            })
            .context("读取环境列表失败")?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(envs)
    }

    /// 获取当前激活的环境
    pub fn get_active_environment(&self) -> Result<Option<Environment>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, name, is_active, created_at, updated_at
                 FROM hosts_environments WHERE is_active = TRUE LIMIT 1",
            )
            .context("查询激活环境失败")?;

        let result = stmt
            .query_row([], |row| {
                Ok(Environment {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    is_active: row.get(2)?,
                    created_at: row.get(3)?,
                    updated_at: row.get(4)?,
                })
            })
            .optional()
            .context("读取激活环境失败")?;

        Ok(result)
    }

    /// 添加环境
    pub fn add_environment(&self, name: &str) -> Result<i64> {
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
        self.conn
            .execute(
                "UPDATE hosts_environments SET name = ?1, updated_at = CURRENT_TIMESTAMP WHERE id = ?2",
                params![name, id],
            )
            .context("更新环境失败")?;
        Ok(())
    }

    /// 删除环境
    pub fn delete_environment(&self, id: i64) -> Result<()> {
        self.conn
            .execute(
                "DELETE FROM hosts_environments WHERE id = ?1",
                params![id],
            )
            .context("删除环境失败")?;
        Ok(())
    }

    /// 设置激活环境（同时取消其他环境的激活状态）
    pub fn set_active_environment(&self, id: Option<i64>) -> Result<()> {
        // 先取消所有激活
        self.conn
            .execute(
                "UPDATE hosts_environments SET is_active = FALSE, updated_at = CURRENT_TIMESTAMP",
                [],
            )
            .context("重置环境状态失败")?;

        // 激活指定环境
        if let Some(env_id) = id {
            self.conn
                .execute(
                    "UPDATE hosts_environments SET is_active = TRUE, updated_at = CURRENT_TIMESTAMP WHERE id = ?1",
                    params![env_id],
                )
                .context("激活环境失败")?;
        }

        Ok(())
    }

    // ========== 条目管理 ==========

    /// 获取指定环境的所有条目
    pub fn get_entries_by_env(&self, env_id: i64) -> Result<Vec<DbHostsEntry>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, environment_id, ip_address, hostname, comment, is_enabled, sort_order
                 FROM hosts_entries
                 WHERE environment_id = ?1
                 ORDER BY sort_order ASC, hostname ASC",
            )
            .context("查询条目列表失败")?;

        let entries = stmt
            .query_map(params![env_id], |row| {
                Ok(DbHostsEntry {
                    id: row.get(0)?,
                    environment_id: row.get(1)?,
                    ip_address: row.get(2)?,
                    hostname: row.get(3)?,
                    comment: row.get(4)?,
                    is_enabled: row.get(5)?,
                    sort_order: row.get(6)?,
                })
            })
            .context("读取条目列表失败")?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(entries)
    }

    /// 添加条目
    pub fn add_entry(&self, env_id: i64, entry: &NewHostsEntry) -> Result<i64> {
        self.conn
            .execute(
                "INSERT INTO hosts_entries (environment_id, ip_address, hostname, comment) VALUES (?1, ?2, ?3, ?4)",
                params![env_id, entry.ip_address, entry.hostname, entry.comment],
            )
            .context("添加条目失败")?;

        Ok(self.conn.last_insert_rowid())
    }

    /// 更新条目
    pub fn update_entry(&self, id: i64, ip: &str, hostname: &str, comment: &Option<String>) -> Result<()> {
        self.conn
            .execute(
                "UPDATE hosts_entries SET ip_address = ?1, hostname = ?2, comment = ?3 WHERE id = ?4",
                params![ip, hostname, comment, id],
            )
            .context("更新条目失败")?;
        Ok(())
    }

    /// 删除条目
    pub fn delete_entry(&self, id: i64) -> Result<()> {
        self.conn
            .execute("DELETE FROM hosts_entries WHERE id = ?1", params![id])
            .context("删除条目失败")?;
        Ok(())
    }

    /// 切换条目启用状态
    pub fn toggle_entry(&self, id: i64, enabled: bool) -> Result<()> {
        self.conn
            .execute(
                "UPDATE hosts_entries SET is_enabled = ?1 WHERE id = ?2",
                params![enabled, id],
            )
            .context("切换条目状态失败")?;
        Ok(())
    }

    /// 获取环境下的条目数量
    pub fn count_entries(&self, env_id: i64) -> Result<usize> {
        let count: usize = self.conn
            .query_row(
                "SELECT COUNT(*) FROM hosts_entries WHERE environment_id = ?1",
                params![env_id],
                |row| row.get(0),
            )
            .context("查询条目数量失败")?;
        Ok(count)
    }
}

/// rusqlite 扩展 trait
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
