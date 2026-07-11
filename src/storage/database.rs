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
                name        TEXT NOT NULL,
                url         TEXT,
                username    TEXT NOT NULL,
                password    BLOB NOT NULL,
                iv          BLOB NOT NULL,
                notes       TEXT,
                created_at  DATETIME DEFAULT CURRENT_TIMESTAMP,
                updated_at  DATETIME DEFAULT CURRENT_TIMESTAMP
            );",
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

        // SSH 客户端 - 会话配置表
        self.conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS ssh_sessions (
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                name        TEXT NOT NULL,
                host        TEXT NOT NULL,
                port        INTEGER NOT NULL DEFAULT 22,
                username    TEXT NOT NULL,
                auth_type   TEXT NOT NULL DEFAULT 'password',
                auth_data   TEXT,
                sort_order  INTEGER DEFAULT 0,
                created_at  DATETIME DEFAULT CURRENT_TIMESTAMP,
                updated_at  DATETIME DEFAULT CURRENT_TIMESTAMP
            );",
        )?;

        // 数据库迁移：清除旧版本的 SSH 会话数据，添加唯一约束
        let db_version: i32 = self
            .conn
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap_or(0);
        if db_version < 1 {
            // 清除 v0 版本的旧数据（auth_data 格式不兼容）
            self.conn
                .execute("DELETE FROM ssh_sessions", [])
                .context("清除旧 SSH 会话数据失败")?;
            // 添加唯一约束：同主机同用户名只能有一个配置
            self.conn
                .execute(
                    "CREATE UNIQUE INDEX IF NOT EXISTS idx_ssh_sessions_unique
                     ON ssh_sessions(host, username)",
                    [],
                )
                .context("创建唯一索引失败")?;
            self.conn
                .pragma_update(None, "user_version", 1)
                .context("更新数据库版本号失败")?;
            log::info!("SSH 连接配置表已迁移到 v1（清除旧数据，添加唯一约束）");
        }

        // 数据库迁移：密码管理器 website 字段重命名为 name
        if db_version < 2 {
            // 通过 PRAGMA table_info 检查旧的 website 列是否存在
            let has_website = self
                .conn
                .prepare("PRAGMA table_info(passwords)")
                .context("查询密码表结构失败")?
                .query_map([], |row| {
                    let col_name: String = row.get(1)?;
                    Ok(col_name)
                })
                .context("读取密码表列信息失败")?
                .filter_map(|r| r.ok())
                .any(|col| col == "website");

            if has_website {
                self.conn
                    .execute_batch("ALTER TABLE passwords RENAME COLUMN website TO name")
                    .context("迁移密码表 website → name 失败")?;
                // 重建索引
                self.conn
                    .execute_batch("DROP INDEX IF EXISTS idx_passwords_website")
                    .context("删除旧索引失败")?;
                log::info!("密码管理器表已迁移到 v2（website → name）");
            }
            // 确保索引存在（新数据库或迁移后都需要）
            self.conn
                .execute_batch("CREATE INDEX IF NOT EXISTS idx_passwords_name ON passwords(name)")
                .context("创建密码表索引失败")?;
            self.conn
                .pragma_update(None, "user_version", 2)
                .context("更新数据库版本号失败")?;
        }

        // SSH 客户端 - 连接历史表
        self.conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS ssh_history (
                id              INTEGER PRIMARY KEY AUTOINCREMENT,
                session_id      INTEGER NOT NULL,
                connected_at    DATETIME DEFAULT CURRENT_TIMESTAMP,
                disconnected_at DATETIME,
                FOREIGN KEY (session_id) REFERENCES ssh_sessions(id) ON DELETE CASCADE
            );",
        )?;

        // 应用设置表（单行配置）
        self.conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS app_settings (
                id              INTEGER PRIMARY KEY DEFAULT 1,
                theme           TEXT NOT NULL DEFAULT 'dark',
                font_size       REAL NOT NULL DEFAULT 14.0,
                sidebar_width   REAL NOT NULL DEFAULT 200.0,
                tool_hotkeys    TEXT NOT NULL DEFAULT '1234567',
                auto_start      BOOLEAN NOT NULL DEFAULT 0,
                updated_at      DATETIME DEFAULT CURRENT_TIMESTAMP
            );",
        )?;

        // 迁移：确保 app_settings 表有 tool_hotkeys 和 auto_start 列
        let has_tool_hotkeys = self
            .conn
            .prepare("PRAGMA table_info(app_settings)")
            .context("查询 app_settings 表结构失败")?
            .query_map([], |row| {
                let col_name: String = row.get(1)?;
                Ok(col_name)
            })
            .context("读取 app_settings 列信息失败")?
            .filter_map(|r| r.ok())
            .any(|col| col == "tool_hotkeys");

        if !has_tool_hotkeys {
            self.conn
                .execute_batch("ALTER TABLE app_settings ADD COLUMN tool_hotkeys TEXT NOT NULL DEFAULT '1234567'")
                .context("添加 tool_hotkeys 列失败")?;
            log::info!("app_settings 表已迁移：添加 tool_hotkeys 列");
        }

        let has_auto_start = self
            .conn
            .prepare("PRAGMA table_info(app_settings)")
            .context("查询 app_settings 表结构失败")?
            .query_map([], |row| {
                let col_name: String = row.get(1)?;
                Ok(col_name)
            })
            .context("读取 app_settings 列信息失败")?
            .filter_map(|r| r.ok())
            .any(|col| col == "auto_start");

        if !has_auto_start {
            self.conn
                .execute_batch("ALTER TABLE app_settings ADD COLUMN auto_start BOOLEAN NOT NULL DEFAULT 0")
                .context("添加 auto_start 列失败")?;
            log::info!("app_settings 表已迁移：添加 auto_start 列");
        }

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
