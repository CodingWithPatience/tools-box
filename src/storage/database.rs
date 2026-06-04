use anyhow::{Context, Result};
use rusqlite::Connection;
use std::path::PathBuf;

/// SQLite 数据库连接管理器
pub struct Database {
    conn: Connection,
}

impl Database {
    /// 打开或创建数据库文件
    ///
    /// 数据库文件默认存放在用户数据目录下：
    /// - Windows: `%APPDATA%/tools-box/data.db`
    /// - macOS:   `~/Library/Application Support/tools-box/data.db`
    /// - Linux:   `~/.local/share/tools-box/data.db`
    pub fn open() -> Result<Self> {
        let db_path = Self::db_path()?;
        log::info!("数据库路径: {}", db_path.display());

        // 确保父目录存在
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("无法创建数据库目录: {}", parent.display()))?;
        }

        let conn = Connection::open(&db_path)
            .with_context(|| format!("无法打开数据库: {}", db_path.display()))?;

        let db = Self { conn };
        db.init_tables()?;
        Ok(db)
    }

    /// 获取数据库文件路径
    fn db_path() -> Result<PathBuf> {
        let data_dir = dirs::data_dir()
            .context("无法获取系统数据目录")?;
        Ok(data_dir.join("tools-box").join("data.db"))
    }

    /// 初始化所有插件所需的数据库表
    fn init_tables(&self) -> Result<()> {
        // 启用 WAL 模式，提升并发读取性能
        self.conn.execute_batch("PRAGMA journal_mode = WAL;")?;

        // 启用外键约束
        self.conn.execute_batch("PRAGMA foreign_keys = ON;")?;

        // 密码管理器 - 主密码配置表
        self.conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS master_config (
                id          INTEGER PRIMARY KEY DEFAULT 1,
                salt        BLOB NOT NULL,
                verify_hash BLOB NOT NULL,
                created_at  DATETIME DEFAULT CURRENT_TIMESTAMP
            );",
        )?;

        // 密码管理器 - 密码条目表
        self.conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS passwords (
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                website     TEXT NOT NULL,
                url         TEXT,
                username    TEXT NOT NULL,
                password    BLOB NOT NULL,
                iv          BLOB NOT NULL,
                notes       TEXT,
                created_at  DATETIME DEFAULT CURRENT_TIMESTAMP,
                updated_at  DATETIME DEFAULT CURRENT_TIMESTAMP
            );
            CREATE INDEX IF NOT EXISTS idx_passwords_website ON passwords(website);",
        )?;

        // Hosts 管理器 - 环境表
        self.conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS hosts_environments (
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                name        TEXT NOT NULL UNIQUE,
                is_active   BOOLEAN DEFAULT FALSE,
                created_at  DATETIME DEFAULT CURRENT_TIMESTAMP,
                updated_at  DATETIME DEFAULT CURRENT_TIMESTAMP
            );",
        )?;

        // Hosts 管理器 - 条目表
        self.conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS hosts_entries (
                id              INTEGER PRIMARY KEY AUTOINCREMENT,
                environment_id  INTEGER NOT NULL,
                ip_address      TEXT NOT NULL,
                hostname        TEXT NOT NULL,
                comment         TEXT,
                is_enabled      BOOLEAN DEFAULT TRUE,
                sort_order      INTEGER DEFAULT 0,
                FOREIGN KEY (environment_id) REFERENCES hosts_environments(id) ON DELETE CASCADE
            );
            CREATE INDEX IF NOT EXISTS idx_hosts_entries_env ON hosts_entries(environment_id);",
        )?;

        log::info!("数据库表初始化完成");
        Ok(())
    }

    /// 获取底层 Connection 的引用（供插件直接使用）
    pub fn conn(&self) -> &Connection {
        &self.conn
    }

    /// 获取底层 Connection 的可变引用（供插件直接使用）
    pub fn conn_mut(&mut self) -> &mut Connection {
        &mut self.conn
    }
}
