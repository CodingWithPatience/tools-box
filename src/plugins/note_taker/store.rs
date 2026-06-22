use anyhow::Result;
use rusqlite::{Connection, OptionalExtension, params};

use super::models::{NoteEntry, NoteFolder};

/// 笔记存储
pub struct NoteStore<'a> {
    conn: &'a Connection,
}

impl<'a> NoteStore<'a> {
    /// 创建新的存储实例
    pub fn new(conn: &'a Connection) -> Self {
        Self { conn }
    }

    /// 初始化数据库表
    pub fn init_table(&self) -> Result<()> {
        self.conn.execute(
            "CREATE TABLE IF NOT EXISTS note_folders (
                id         INTEGER PRIMARY KEY AUTOINCREMENT,
                name       TEXT NOT NULL,
                parent_id  INTEGER,
                sort_order INTEGER DEFAULT 0,
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                FOREIGN KEY (parent_id) REFERENCES note_folders(id) ON DELETE CASCADE
            )",
            [],
        )?;

        self.conn.execute(
            "CREATE TABLE IF NOT EXISTS note_entries (
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                title       TEXT NOT NULL,
                content     TEXT NOT NULL DEFAULT '',
                folder_id   INTEGER,
                is_favorite BOOLEAN DEFAULT FALSE,
                tags        TEXT DEFAULT '',
                created_at  DATETIME DEFAULT CURRENT_TIMESTAMP,
                updated_at  DATETIME DEFAULT CURRENT_TIMESTAMP,
                FOREIGN KEY (folder_id) REFERENCES note_folders(id) ON DELETE SET NULL
            )",
            [],
        )?;

        self.conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_note_entries_folder ON note_entries(folder_id)",
            [],
        )?;

        self.conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_note_entries_favorite ON note_entries(is_favorite)",
            [],
        )?;

        self.conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_note_entries_updated ON note_entries(updated_at)",
            [],
        )?;

        Ok(())
    }

    // ========== 目录操作 ==========

    /// 获取所有目录
    pub fn get_all_folders(&self) -> Result<Vec<NoteFolder>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, name, parent_id, sort_order, created_at FROM note_folders ORDER BY sort_order, name")?;

        let folders = stmt
            .query_map([], |row| {
                Ok(NoteFolder {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    parent_id: row.get(2)?,
                    sort_order: row.get(3)?,
                    created_at: row.get(4)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(folders)
    }

    /// 创建目录
    pub fn create_folder(&self, name: &str, parent_id: Option<i64>) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO note_folders (name, parent_id) VALUES (?1, ?2)",
            params![name, parent_id],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// 更新目录
    pub fn update_folder(&self, id: i64, name: &str, parent_id: Option<i64>) -> Result<()> {
        self.conn.execute(
            "UPDATE note_folders SET name = ?1, parent_id = ?2 WHERE id = ?3",
            params![name, parent_id, id],
        )?;
        Ok(())
    }

    /// 删除目录
    pub fn delete_folder(&self, id: i64) -> Result<()> {
        self.conn
            .execute("DELETE FROM note_folders WHERE id = ?1", params![id])?;
        Ok(())
    }

    /// 获取目录下的笔记数量
    pub fn count_notes_in_folder(&self, folder_id: i64) -> Result<usize> {
        let count: usize = self.conn.query_row(
            "SELECT COUNT(*) FROM note_entries WHERE folder_id = ?1",
            params![folder_id],
            |row| row.get(0),
        )?;
        Ok(count)
    }

    // ========== 笔记操作 ==========

    /// 获取所有笔记
    pub fn get_all_notes(&self) -> Result<Vec<NoteEntry>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, title, content, folder_id, is_favorite, tags, created_at, updated_at 
             FROM note_entries ORDER BY updated_at DESC",
        )?;

        let notes = stmt
            .query_map([], |row| {
                let tags_str: String = row.get::<_, String>(5)?;
                Ok(NoteEntry {
                    id: row.get(0)?,
                    title: row.get(1)?,
                    content: row.get(2)?,
                    folder_id: row.get(3)?,
                    is_favorite: row.get(4)?,
                    tags: Self::parse_tags_from_str(&tags_str),
                    created_at: row.get(6)?,
                    updated_at: row.get(7)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(notes)
    }

    /// 根据目录获取笔记
    pub fn get_notes_by_folder(&self, folder_id: Option<i64>) -> Result<Vec<NoteEntry>> {
        let sql = match folder_id {
            Some(_) => {
                "SELECT id, title, content, folder_id, is_favorite, tags, created_at, updated_at 
                 FROM note_entries WHERE folder_id = ?1 ORDER BY updated_at DESC"
            }
            None => {
                "SELECT id, title, content, folder_id, is_favorite, tags, created_at, updated_at 
                 FROM note_entries WHERE folder_id IS NULL ORDER BY updated_at DESC"
            }
        };

        let mut stmt = self.conn.prepare(sql)?;

        let notes = if let Some(fid) = folder_id {
            stmt.query_map(params![fid], |row| {
                let tags_str: String = row.get::<_, String>(5)?;
                Ok(NoteEntry {
                    id: row.get(0)?,
                    title: row.get(1)?,
                    content: row.get(2)?,
                    folder_id: row.get(3)?,
                    is_favorite: row.get(4)?,
                    tags: Self::parse_tags_from_str(&tags_str),
                    created_at: row.get(6)?,
                    updated_at: row.get(7)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?
        } else {
            stmt.query_map([], |row| {
                let tags_str: String = row.get::<_, String>(5)?;
                Ok(NoteEntry {
                    id: row.get(0)?,
                    title: row.get(1)?,
                    content: row.get(2)?,
                    folder_id: row.get(3)?,
                    is_favorite: row.get(4)?,
                    tags: Self::parse_tags_from_str(&tags_str),
                    created_at: row.get(6)?,
                    updated_at: row.get(7)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?
        };

        Ok(notes)
    }

    /// 获取收藏笔记
    pub fn get_favorite_notes(&self) -> Result<Vec<NoteEntry>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, title, content, folder_id, is_favorite, tags, created_at, updated_at 
             FROM note_entries WHERE is_favorite = 1 ORDER BY updated_at DESC",
        )?;

        let notes = stmt
            .query_map([], |row| {
                let tags_str: String = row.get::<_, String>(5)?;
                Ok(NoteEntry {
                    id: row.get(0)?,
                    title: row.get(1)?,
                    content: row.get(2)?,
                    folder_id: row.get(3)?,
                    is_favorite: row.get(4)?,
                    tags: Self::parse_tags_from_str(&tags_str),
                    created_at: row.get(6)?,
                    updated_at: row.get(7)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(notes)
    }

    /// 搜索笔记
    pub fn search_notes(&self, query: &str) -> Result<Vec<NoteEntry>> {
        let search_pattern = format!("%{}%", query);
        let mut stmt = self.conn.prepare(
            "SELECT id, title, content, folder_id, is_favorite, tags, created_at, updated_at 
             FROM note_entries 
             WHERE title LIKE ?1 OR content LIKE ?1 OR tags LIKE ?1
             ORDER BY updated_at DESC",
        )?;

        let notes = stmt
            .query_map(params![search_pattern], |row| {
                let tags_str: String = row.get::<_, String>(5)?;
                Ok(NoteEntry {
                    id: row.get(0)?,
                    title: row.get(1)?,
                    content: row.get(2)?,
                    folder_id: row.get(3)?,
                    is_favorite: row.get(4)?,
                    tags: Self::parse_tags_from_str(&tags_str),
                    created_at: row.get(6)?,
                    updated_at: row.get(7)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(notes)
    }

    /// 根据 ID 获取笔记
    pub fn get_note_by_id(&self, id: i64) -> Result<Option<NoteEntry>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, title, content, folder_id, is_favorite, tags, created_at, updated_at 
             FROM note_entries WHERE id = ?1",
        )?;

        let result = stmt
            .query_row(params![id], |row| {
                let tags_str: String = row.get::<_, String>(5)?;
                Ok(NoteEntry {
                    id: row.get(0)?,
                    title: row.get(1)?,
                    content: row.get(2)?,
                    folder_id: row.get(3)?,
                    is_favorite: row.get(4)?,
                    tags: Self::parse_tags_from_str(&tags_str),
                    created_at: row.get(6)?,
                    updated_at: row.get(7)?,
                })
            })
            .optional()?;

        Ok(result)
    }

    /// 创建笔记
    pub fn create_note(
        &self,
        title: &str,
        content: &str,
        folder_id: Option<i64>,
        tags: &str,
    ) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO note_entries (title, content, folder_id, tags) VALUES (?1, ?2, ?3, ?4)",
            params![title, content, folder_id, tags],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// 更新笔记
    pub fn update_note(
        &self,
        id: i64,
        title: &str,
        content: &str,
        folder_id: Option<i64>,
        tags: &str,
    ) -> Result<()> {
        self.conn.execute(
            "UPDATE note_entries SET title = ?1, content = ?2, folder_id = ?3, tags = ?4, updated_at = CURRENT_TIMESTAMP WHERE id = ?5",
            params![title, content, folder_id, tags, id],
        )?;
        Ok(())
    }

    /// 删除笔记
    pub fn delete_note(&self, id: i64) -> Result<()> {
        self.conn
            .execute("DELETE FROM note_entries WHERE id = ?1", params![id])?;
        Ok(())
    }

    /// 切换收藏状态
    pub fn toggle_favorite(&self, id: i64) -> Result<bool> {
        self.conn.execute(
            "UPDATE note_entries SET is_favorite = NOT is_favorite WHERE id = ?1",
            params![id],
        )?;
        let is_favorite: bool = self.conn.query_row(
            "SELECT is_favorite FROM note_entries WHERE id = ?1",
            params![id],
            |row| row.get(0),
        )?;
        Ok(is_favorite)
    }

    /// 获取笔记总数
    pub fn count_notes(&self) -> Result<usize> {
        let count: usize = self
            .conn
            .query_row("SELECT COUNT(*) FROM note_entries", [], |row| {
                row.get(0)
            })?;
        Ok(count)
    }

    // ========== 辅助方法 ==========

    /// 解析标签字符串
    fn parse_tags_from_str(tags_str: &str) -> Vec<String> {
        tags_str
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect()
    }
}