use anyhow::Result;
use rusqlite::{Connection, params};

use super::models::{
    ApiCollection, Environment, EnvironmentVariable, RequestHistory, SavedApiRequest,
};

/// API 存储
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
        // 集合表
        self.conn.execute(
            "CREATE TABLE IF NOT EXISTS api_collections (
                id           INTEGER PRIMARY KEY AUTOINCREMENT,
                name         TEXT NOT NULL,
                parent_id    INTEGER,
                description  TEXT DEFAULT '',
                sort_order   INTEGER DEFAULT 0,
                created_at   DATETIME DEFAULT CURRENT_TIMESTAMP,
                FOREIGN KEY (parent_id) REFERENCES api_collections(id) ON DELETE CASCADE
            )",
            [],
        )?;

        // 保存的请求表
        self.conn.execute(
            "CREATE TABLE IF NOT EXISTS api_saved_requests (
                id             INTEGER PRIMARY KEY AUTOINCREMENT,
                collection_id  INTEGER,
                name           TEXT NOT NULL,
                method         TEXT NOT NULL,
                url            TEXT NOT NULL,
                headers        TEXT,
                params         TEXT,
                body_type      TEXT DEFAULT 'none',
                body           TEXT,
                sort_order     INTEGER DEFAULT 0,
                created_at     DATETIME DEFAULT CURRENT_TIMESTAMP,
                updated_at     DATETIME DEFAULT CURRENT_TIMESTAMP,
                FOREIGN KEY (collection_id) REFERENCES api_collections(id) ON DELETE SET NULL
            )",
            [],
        )?;

        self.conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_api_saved_requests_collection ON api_saved_requests(collection_id)",
            [],
        )?;

        // 环境表
        self.conn.execute(
            "CREATE TABLE IF NOT EXISTS api_environments (
                id           INTEGER PRIMARY KEY AUTOINCREMENT,
                name         TEXT NOT NULL,
                is_default   BOOLEAN DEFAULT FALSE,
                is_active    BOOLEAN DEFAULT FALSE,
                created_at   DATETIME DEFAULT CURRENT_TIMESTAMP
            )",
            [],
        )?;

        // 环境变量表
        self.conn.execute(
            "CREATE TABLE IF NOT EXISTS api_environment_variables (
                id             INTEGER PRIMARY KEY AUTOINCREMENT,
                environment_id INTEGER NOT NULL,
                key            TEXT NOT NULL,
                value          TEXT DEFAULT '',
                enabled        BOOLEAN DEFAULT TRUE,
                FOREIGN KEY (environment_id) REFERENCES api_environments(id) ON DELETE CASCADE
            )",
            [],
        )?;

        self.conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_api_env_vars_environment ON api_environment_variables(environment_id)",
            [],
        )?;

        // 历史记录表
        self.conn.execute(
            "CREATE TABLE IF NOT EXISTS api_history (
                id           INTEGER PRIMARY KEY AUTOINCREMENT,
                request_id   TEXT NOT NULL,
                name         TEXT NOT NULL DEFAULT '',
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

        // 兼容旧版本历史表结构
        self.ensure_history_column("params", "TEXT")?;
        self.ensure_history_column("name", "TEXT NOT NULL DEFAULT ''")?;

        // 初始化默认全局环境（如果不存在）
        self.init_default_environment()?;

        Ok(())
    }

    /// 为历史表补充缺失列
    fn ensure_history_column(&self, column: &str, definition: &str) -> Result<()> {
        let mut stmt = self.conn.prepare("PRAGMA table_info(api_history)")?;
        let column_names: Vec<String> = stmt
            .query_map([], |row| row.get(1))?
            .collect::<rusqlite::Result<Vec<_>>>()?;

        if !column_names.iter().any(|name| name == column) {
            let sql = format!(
                "ALTER TABLE api_history ADD COLUMN {} {}",
                column, definition
            );
            self.conn.execute(&sql, [])?;
        }

        Ok(())
    }

    /// 初始化默认全局环境
    fn init_default_environment(&self) -> Result<()> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM api_environments WHERE is_default = 1",
            [],
            |row| row.get(0),
        )?;

        if count == 0 {
            self.conn.execute(
                "INSERT INTO api_environments (name, is_default, is_active) VALUES ('全局', 1, 1)",
                [],
            )?;
        }

        Ok(())
    }

    /// 保存请求历史
    pub fn save_history(
        &self,
        request_id: &str,
        name: &str,
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
            "INSERT INTO api_history (request_id, name, method, url, headers, params, body_type, body, status_code, response, elapsed_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                request_id,
                name,
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
            "SELECT id, request_id, COALESCE(NULLIF(name, ''), url), method, url,
                    status_code, elapsed_ms, executed_at
             FROM api_history
             ORDER BY executed_at DESC
             LIMIT ?1",
        )?;

        let history = stmt
            .query_map(params![limit], |row| {
                Ok(RequestHistory {
                    id: row.get(0)?,
                    request_id: row.get(1)?,
                    name: row.get(2)?,
                    method: row.get(3)?,
                    url: row.get(4)?,
                    status_code: row.get(5)?,
                    elapsed_ms: row.get(6)?,
                    executed_at: row.get(7)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(history)
    }

    /// 根据 ID 获取历史记录详情
    pub fn get_history_by_id(
        &self,
        id: i64,
    ) -> Result<Option<(String, String, String, String, String, String)>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT name, method, url, headers, params, body
                 FROM api_history WHERE id = ?1",
            )?;

        let result = stmt
            .query_row(params![id], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3).unwrap_or_default(),
                    row.get::<_, String>(4).unwrap_or_default(),
                    row.get::<_, String>(5).unwrap_or_default(),
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


    // ========== 集合操作 ==========

    /// 获取所有集合
    pub fn get_all_collections(&self) -> Result<Vec<ApiCollection>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, name, parent_id, description, sort_order, created_at
             FROM api_collections ORDER BY sort_order, name",
        )?;

        let collections = stmt
            .query_map([], |row| {
                Ok(ApiCollection {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    parent_id: row.get(2)?,
                    description: row.get(3)?,
                    sort_order: row.get(4)?,
                    created_at: row.get(5)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(collections)
    }

    /// 创建集合
    pub fn create_collection(&self, name: &str, parent_id: Option<i64>) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO api_collections (name, parent_id) VALUES (?1, ?2)",
            params![name, parent_id],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// 更新集合
    pub fn update_collection(&self, id: i64, name: &str, description: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE api_collections SET name = ?1, description = ?2 WHERE id = ?3",
            params![name, description, id],
        )?;
        Ok(())
    }

    /// 删除集合
    pub fn delete_collection(&self, id: i64) -> Result<()> {
        self.conn
            .execute("DELETE FROM api_collections WHERE id = ?1", params![id])?;
        Ok(())
    }

    // ========== 保存的请求操作 ==========

    /// 获取集合下的请求
    pub fn get_requests_by_collection(
        &self,
        collection_id: Option<i64>,
    ) -> Result<Vec<SavedApiRequest>> {
        let sql = match collection_id {
            Some(_) => {
                "SELECT id, collection_id, name, method, url, headers, params, body_type, body, created_at, updated_at
                 FROM api_saved_requests WHERE collection_id = ?1 ORDER BY sort_order, name"
            }
            None => {
                "SELECT id, collection_id, name, method, url, headers, params, body_type, body, created_at, updated_at
                 FROM api_saved_requests WHERE collection_id IS NULL ORDER BY sort_order, name"
            }
        };

        let mut stmt = self.conn.prepare(sql)?;

        let requests = if let Some(cid) = collection_id {
            stmt.query_map(params![cid], |row| Self::map_saved_request(row))?
                .collect::<Result<Vec<_>, _>>()?
        } else {
            stmt.query_map([], |row| Self::map_saved_request(row))?
                .collect::<Result<Vec<_>, _>>()?
        };

        Ok(requests)
    }

    /// 保存请求到集合
    pub fn save_request(
        &self,
        collection_id: Option<i64>,
        name: &str,
        method: &str,
        url: &str,
        headers: &str,
        params: &str,
        body_type: &str,
        body: &str,
    ) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO api_saved_requests (collection_id, name, method, url, headers, params, body_type, body)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![collection_id, name, method, url, headers, params, body_type, body],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// 更新已保存的请求
    pub fn update_request(
        &self,
        id: i64,
        collection_id: Option<i64>,
        name: &str,
        method: &str,
        url: &str,
        headers: &str,
        params: &str,
        body_type: &str,
        body: &str,
    ) -> Result<()> {
        self.conn.execute(
            "UPDATE api_saved_requests
             SET collection_id = ?1, name = ?2, method = ?3, url = ?4,
                 headers = ?5, params = ?6, body_type = ?7, body = ?8,
                 updated_at = CURRENT_TIMESTAMP
             WHERE id = ?9",
            params![
                collection_id,
                name,
                method,
                url,
                headers,
                params,
                body_type,
                body,
                id,
            ],
        )?;

        if self.conn.changes() == 0 {
            return Err(anyhow::anyhow!("保存的请求不存在: {}", id));
        }

        Ok(())
    }

    /// 删除保存的请求
    pub fn delete_saved_request(&self, id: i64) -> Result<()> {
        self.conn
            .execute("DELETE FROM api_saved_requests WHERE id = ?1", params![id])?;
        Ok(())
    }

    /// 映射保存的请求
    fn map_saved_request(row: &rusqlite::Row) -> rusqlite::Result<SavedApiRequest> {
        let method_str: String = row.get(3)?;
        let headers_str: String = row.get(5).unwrap_or_default();
        let params_str: String = row.get(6).unwrap_or_default();
        let body_type_str: String = row.get(7).unwrap_or_default();

        Ok(SavedApiRequest {
            id: row.get(0)?,
            collection_id: row.get(1)?,
            name: row.get(2)?,
            method: super::models::HttpMethod::from_str(&method_str)
                .unwrap_or(super::models::HttpMethod::Get),
            url: row.get(4)?,
            headers: serde_json::from_str(&headers_str).unwrap_or_default(),
            params: serde_json::from_str(&params_str).unwrap_or_default(),
            body_type: match body_type_str.as_str() {
                "json" | "JSON" => super::models::BodyType::Json,
                "form" | "Form" => super::models::BodyType::Form,
                "raw" | "Raw" => super::models::BodyType::Raw,
                _ => super::models::BodyType::None,
            },
            body: row.get(8).unwrap_or_default(),
            created_at: row.get(9)?,
            updated_at: row.get(10)?,
        })
    }

    // ========== 环境操作 ==========

    /// 获取所有环境
    pub fn get_all_environments(&self) -> Result<Vec<Environment>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, name, is_default, is_active, created_at FROM api_environments ORDER BY is_default DESC, name",
        )?;

        let environments = stmt
            .query_map([], |row| {
                Ok(Environment {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    is_default: row.get(2)?,
                    is_active: row.get(3)?,
                    created_at: row.get(4)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(environments)
    }

    /// 创建环境
    pub fn create_environment(&self, name: &str) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO api_environments (name) VALUES (?1)",
            params![name],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// 更新环境
    pub fn update_environment(&self, id: i64, name: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE api_environments SET name = ?1 WHERE id = ?2 AND is_default = 0",
            params![name, id],
        )?;
        Ok(())
    }

    /// 删除环境（不能删除默认环境）
    pub fn delete_environment(&self, id: i64) -> Result<()> {
        self.conn.execute(
            "DELETE FROM api_environments WHERE id = ?1 AND is_default = 0",
            params![id],
        )?;
        Ok(())
    }

    /// 激活环境
    pub fn activate_environment(&self, id: i64) -> Result<()> {
        // 先取消所有激活状态
        self.conn
            .execute("UPDATE api_environments SET is_active = 0", [])?;
        // 激活指定环境
        self.conn.execute(
            "UPDATE api_environments SET is_active = 1 WHERE id = ?1",
            params![id],
        )?;
        Ok(())
    }

    // ========== 环境变量操作 ==========

    /// 获取所有环境变量（包括全局和当前环境）
    pub fn get_all_active_variables(&self) -> Result<Vec<EnvironmentVariable>> {
        // 获取全局环境变量
        let mut stmt = self.conn.prepare(
            "SELECT v.id, v.environment_id, v.key, v.value, v.enabled
             FROM api_environment_variables v
             INNER JOIN api_environments e ON v.environment_id = e.id
             WHERE e.is_default = 1 AND v.enabled = 1
             UNION
             SELECT v.id, v.environment_id, v.key, v.value, v.enabled
             FROM api_environment_variables v
             INNER JOIN api_environments e ON v.environment_id = e.id
             WHERE e.is_active = 1 AND e.is_default = 0 AND v.enabled = 1
             ORDER BY key",
        )?;

        let variables = stmt
            .query_map([], |row| {
                Ok(EnvironmentVariable {
                    id: row.get(0)?,
                    environment_id: row.get(1)?,
                    key: row.get(2)?,
                    value: row.get(3)?,
                    enabled: row.get(4)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(variables)
    }

    /// 创建环境变量
    pub fn create_environment_variable(
        &self,
        environment_id: i64,
        key: &str,
        value: &str,
    ) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO api_environment_variables (environment_id, key, value) VALUES (?1, ?2, ?3)",
            params![environment_id, key, value],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// 更新环境变量
    pub fn update_environment_variable(
        &self,
        id: i64,
        key: &str,
        value: &str,
        enabled: bool,
    ) -> Result<()> {
        self.conn.execute(
            "UPDATE api_environment_variables SET key = ?1, value = ?2, enabled = ?3 WHERE id = ?4",
            params![key, value, enabled, id],
        )?;
        Ok(())
    }

    /// 删除环境变量
    pub fn delete_environment_variable(&self, id: i64) -> Result<()> {
        self.conn.execute(
            "DELETE FROM api_environment_variables WHERE id = ?1",
            params![id],
        )?;
        Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    #[test]
    fn update_request_updates_existing_row() -> Result<()> {
        let conn = Connection::open_in_memory()?;
        let store = ApiStore::new(&conn);
        store.init_table()?;

        let id = store.save_request(
            None,
            "原始名称",
            "GET",
            "https://example.com/old",
            "[]",
            "[]",
            "None",
            "",
        )?;
        let collection_id = store.create_collection("测试集合", None)?;

        store.update_request(
            id,
            Some(collection_id),
            "更新后的完整请求名称",
            "POST",
            "https://example.com/new",
            "[]",
            "[]",
            "JSON",
            "{}",
        )?;

        let requests = store.get_requests_by_collection(Some(collection_id))?;
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].id, id);
        assert_eq!(requests[0].name, "更新后的完整请求名称");
        assert_eq!(requests[0].url, "https://example.com/new");
        assert!(store.get_requests_by_collection(None)?.is_empty());

        Ok(())
    }

    #[test]
    fn update_request_returns_error_for_missing_row() -> Result<()> {
        let conn = Connection::open_in_memory()?;
        let store = ApiStore::new(&conn);
        store.init_table()?;

        let result = store.update_request(
            999,
            None,
            "请求",
            "GET",
            "https://example.com",
            "[]",
            "[]",
            "None",
            "",
        );

        assert!(result.is_err());
        Ok(())
    }

    #[test]
    fn history_preserves_request_name() -> Result<()> {
        let conn = Connection::open_in_memory()?;
        let store = ApiStore::new(&conn);
        store.init_table()?;

        let id = store.save_history(
            "request-id",
            "用户列表请求",
            "GET",
            "https://example.com/users",
            "[]",
            "[]",
            "None",
            "",
            Some(200),
            Some("{}"),
            Some(10),
        )?;

        let history = store.get_recent_history(10)?;
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].id, id);
        assert_eq!(history[0].name, "用户列表请求");

        let detail = store.get_history_by_id(id)?.ok_or_else(|| {
            anyhow::anyhow!("历史记录不存在")
        })?;
        assert_eq!(detail.0, "用户列表请求");

        Ok(())
    }
}
