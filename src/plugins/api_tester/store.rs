use anyhow::Result;
use rusqlite::{Connection, params};

use super::models::RequestHistory;

/// API 历史记录存储
pub struct ApiStore<'a> {
    conn: &'a Connection,
}

impl<'a> ApiStore<'a> {
    /// 创建新的存储实例
    pub fn new(conn: &'a Connection) -> Self {
        Self { conn }
    }

    /// 初始化数据库表
    pub fn init_table(&self) -> Result<()> {
        self.conn.execute(
            "CREATE TABLE IF NOT EXISTS api_history (
                id           INTEGER PRIMARY KEY AUTOINCREMENT,
                request_id   TEXT NOT NULL,
                method       TEXT NOT NULL,
                url          TEXT NOT NULL,
                headers      TEXT,
                params       TEXT,
                body_type    TEXT DEFAULT 'none',
                body         TEXT,
                status_code  INTEGER,
                response     TEXT,
                elapsed_ms   INTEGER,
                executed_at  DATETIME DEFAULT CURRENT_TIMESTAMP
            )",
            [],
        )?;

        self.conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_api_history_request_id ON api_history(request_id)",
            [],
        )?;

        self.conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_api_history_executed_at ON api_history(executed_at)",
            [],
        )?;

        // 添加 params 列（如果不存在）
        match self.conn.execute(
            "ALTER TABLE api_history ADD COLUMN params TEXT",
            [],
        ) {
            Ok(_) => {}
            Err(rusqlite::Error::SqliteFailure(e, _))
                if e.extended_code == rusqlite::ffi::SQLITE_ERROR =>
            {
                // 列已存在，忽略错误
            }
            Err(e) => return Err(e.into()),
        }

        Ok(())
    }

    /// 保存请求历史
    pub fn save_history(
        &self,
        request_id: &str,
        method: &str,
        url: &str,
        headers: &str,
        params: &str,
        body_type: &str,
        body: &str,
        status_code: Option<i32>,
        response: Option<&str>,
        elapsed_ms: Option<i64>,
    ) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO api_history (request_id, method, url, headers, params, body_type, body, status_code, response, elapsed_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                request_id,
                method,
                url,
                headers,
                params,
                body_type,
                body,
                status_code,
                response,
                elapsed_ms,
            ],
        )?;

        Ok(self.conn.last_insert_rowid())
    }

    /// 获取最近的历史记录
    pub fn get_recent_history(&self, limit: usize) -> Result<Vec<RequestHistory>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, request_id, method, url, status_code, elapsed_ms, executed_at
             FROM api_history
             ORDER BY executed_at DESC
             LIMIT ?1",
        )?;

        let history = stmt
            .query_map(params![limit], |row| {
                Ok(RequestHistory {
                    id: row.get(0)?,
                    request_id: row.get(1)?,
                    method: row.get(2)?,
                    url: row.get(3)?,
                    status_code: row.get(4)?,
                    elapsed_ms: row.get(5)?,
                    executed_at: row.get(6)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(history)
    }

    /// 根据 ID 获取历史记录详情
    pub fn get_history_by_id(&self, id: i64) -> Result<Option<(String, String, String, String, String)>> {
        let mut stmt = self.conn.prepare(
            "SELECT method, url, headers, params, body FROM api_history WHERE id = ?1",
        )?;

        let result = stmt
            .query_row(params![id], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2).unwrap_or_default(),
                    row.get::<_, String>(3).unwrap_or_default(),
                    row.get::<_, String>(4).unwrap_or_default(),
                ))
            })
            .optional()?;

        Ok(result)
    }

    /// 删除历史记录
    pub fn delete_history(&self, id: i64) -> Result<()> {
        self.conn
            .execute("DELETE FROM api_history WHERE id = ?1", params![id])?;
        Ok(())
    }

    /// 清空所有历史记录
    pub fn clear_history(&self) -> Result<()> {
        self.conn.execute("DELETE FROM api_history", [])?;
        Ok(())
    }

    /// 获取历史记录数量
    pub fn count_history(&self) -> Result<usize> {
        let count: usize = self.conn.query_row(
            "SELECT COUNT(*) FROM api_history",
            [],
            |row| row.get(0),
        )?;
        Ok(count)
    }
}

/// rusqlite 的 optional 扩展 trait
trait OptionalExt<T> {
    fn optional(self) -> Result<Option<T>>;
}

impl<T> OptionalExt<T> for Result<T, rusqlite::Error> {
    fn optional(self) -> Result<Option<T>> {
        match self {
            Ok(v) => Ok(Some(v)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }
}
