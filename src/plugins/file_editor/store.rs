//! 常用文件编辑器的 SQLite 持久化
//!
//! 表结构在 `src/storage/database.rs::init_tables()` 中创建，本模块只负责读写。
//! 磁盘文件本身不受本模块影响：删除条目只删列表记录，不删除文件。

use anyhow::{Context, Result, bail};
use rusqlite::{Connection, OptionalExtension, params};

use super::models::{FavoriteFile, FileDirectory};

/// 左侧列表默认宽度（像素）
pub const DEFAULT_LIST_WIDTH: f32 = 260.0;

/// 探测数据库当前是否可写
///
/// 用「写事务 + 回滚」探测：建一张临时命名的探测表后立即回滚，不留下任何修改，
/// 但能真实触发写入路径，从而暴露只读数据库
/// （例如程序运行在带低完整性标签的目录下、无法向上写入 `%APPDATA%` 的情况）。
pub fn probe_writable(conn: &Connection) -> bool {
    /// 探测用表名（随事务回滚，不会留在数据库中）
    const PROBE_TABLE: &str = "__tools_box_write_probe__";

    let statement = format!(
        "BEGIN IMMEDIATE; CREATE TABLE IF NOT EXISTS {PROBE_TABLE} (id INTEGER); ROLLBACK;"
    );

    match conn.execute_batch(&statement) {
        Ok(()) => true,
        Err(error) => {
            log::warn!("数据库不可写: {error}");
            false
        }
    }
}

/// 读取全部自定义目录（按排序序号、名称）
pub fn load_directories(conn: &Connection) -> Result<Vec<FileDirectory>> {
    let mut statement = conn
        .prepare(
            "SELECT id, name, sort_order, created_at
             FROM file_editor_directories
             ORDER BY sort_order, name",
        )
        .context("查询常用文件目录失败")?;
    let rows = statement
        .query_map([], |row| {
            Ok(FileDirectory {
                id: row.get(0)?,
                name: row.get(1)?,
                sort_order: row.get(2)?,
                created_at: row.get(3)?,
            })
        })
        .context("读取常用文件目录失败")?;

    let mut directories = Vec::new();
    for row in rows {
        directories.push(row.context("解析常用文件目录失败")?);
    }

    Ok(directories)
}

/// 新建目录，返回新目录 id
pub fn insert_directory(conn: &Connection, name: &str) -> Result<i64> {
    let name = name.trim();
    if name.is_empty() {
        bail!("目录名称不能为空");
    }

    let next_order: i64 = conn
        .query_row(
            "SELECT COALESCE(MAX(sort_order), 0) + 1 FROM file_editor_directories",
            [],
            |row| row.get(0),
        )
        .context("计算目录排序序号失败")?;

    conn.execute(
        "INSERT INTO file_editor_directories (name, sort_order) VALUES (?1, ?2)",
        params![name, next_order],
    )
    .map_err(|error| anyhow::anyhow!("新建目录失败（目录名可能已存在）: {error}"))?;

    Ok(conn.last_insert_rowid())
}

/// 重命名目录
pub fn rename_directory(conn: &Connection, id: i64, name: &str) -> Result<()> {
    let name = name.trim();
    if name.is_empty() {
        bail!("目录名称不能为空");
    }

    conn.execute(
        "UPDATE file_editor_directories SET name = ?1 WHERE id = ?2",
        params![name, id],
    )
    .map_err(|error| anyhow::anyhow!("重命名目录失败（目录名可能已存在）: {error}"))?;

    Ok(())
}

/// 删除目录；其下条目由外键 `ON DELETE SET NULL` 自动变为未分组
pub fn delete_directory(conn: &Connection, id: i64) -> Result<()> {
    conn.execute(
        "DELETE FROM file_editor_directories WHERE id = ?1",
        params![id],
    )
    .context("删除目录失败")?;

    Ok(())
}

/// 读取全部常用文件条目
pub fn load_files(conn: &Connection) -> Result<Vec<FavoriteFile>> {
    let mut statement = conn
        .prepare(
            "SELECT id, directory_id, path, alias, sort_order, last_opened_at
             FROM file_editor_files
             ORDER BY sort_order, path",
        )
        .context("查询常用文件列表失败")?;
    let rows = statement
        .query_map([], |row| {
            Ok(FavoriteFile {
                id: row.get(0)?,
                directory_id: row.get(1)?,
                path: row.get(2)?,
                alias: row.get(3)?,
                sort_order: row.get(4)?,
                last_opened_at: row.get(5)?,
            })
        })
        .context("读取常用文件列表失败")?;

    let mut files = Vec::new();
    for row in rows {
        files.push(row.context("解析常用文件条目失败")?);
    }

    Ok(files)
}

/// 新增常用文件条目，返回新条目 id
///
/// 路径按不区分大小写判重（Windows 路径大小写不敏感）。
pub fn insert_file(
    conn: &Connection,
    directory_id: Option<i64>,
    path: &str,
    alias: Option<&str>,
) -> Result<i64> {
    let path = path.trim();
    if path.is_empty() {
        bail!("文件路径不能为空");
    }

    let existing: Option<i64> = conn
        .query_row(
            "SELECT id FROM file_editor_files WHERE path = ?1 COLLATE NOCASE",
            params![path],
            |row| row.get(0),
        )
        .optional()
        .context("检查重复路径失败")?;
    if existing.is_some() {
        bail!("该文件已在常用列表中: {path}");
    }

    let next_order: i64 = conn
        .query_row(
            "SELECT COALESCE(MAX(sort_order), 0) + 1 FROM file_editor_files",
            [],
            |row| row.get(0),
        )
        .context("计算条目排序序号失败")?;

    let alias = alias.map(str::trim).filter(|value| !value.is_empty());
    conn.execute(
        "INSERT INTO file_editor_files (directory_id, path, alias, sort_order)
         VALUES (?1, ?2, ?3, ?4)",
        params![directory_id, path, alias, next_order],
    )
    .context("新增常用文件失败")?;

    Ok(conn.last_insert_rowid())
}

/// 删除常用文件条目（不删除磁盘文件）
pub fn delete_file(conn: &Connection, id: i64) -> Result<()> {
    conn.execute("DELETE FROM file_editor_files WHERE id = ?1", params![id])
        .context("删除常用文件失败")?;

    Ok(())
}

/// 更新条目显示名（传 `None` 或空白表示恢复为文件名）
pub fn update_alias(conn: &Connection, id: i64, alias: Option<&str>) -> Result<()> {
    let alias = alias.map(str::trim).filter(|value| !value.is_empty());
    conn.execute(
        "UPDATE file_editor_files SET alias = ?1 WHERE id = ?2",
        params![alias, id],
    )
    .context("更新显示名失败")?;

    Ok(())
}

/// 记录条目最近一次打开时间
pub fn touch_last_opened(conn: &Connection, id: i64) -> Result<()> {
    conn.execute(
        "UPDATE file_editor_files SET last_opened_at = CURRENT_TIMESTAMP WHERE id = ?1",
        params![id],
    )
    .context("更新最近打开时间失败")?;

    Ok(())
}

/// 读取左侧列表宽度，未保存过时返回默认值
pub fn load_list_width(conn: &Connection) -> Result<f32> {
    let width: Option<f32> = conn
        .query_row(
            "SELECT list_width FROM file_editor_settings WHERE id = 1",
            [],
            |row| row.get(0),
        )
        .optional()
        .context("读取列表宽度失败")?;

    Ok(width.unwrap_or(DEFAULT_LIST_WIDTH))
}

/// 保存左侧列表宽度
pub fn save_list_width(conn: &Connection, width: f32) -> Result<()> {
    conn.execute(
        "INSERT OR REPLACE INTO file_editor_settings (id, list_width, updated_at)
         VALUES (1, ?1, CURRENT_TIMESTAMP)",
        params![width],
    )
    .context("保存列表宽度失败")?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        DEFAULT_LIST_WIDTH, delete_directory, delete_file, insert_directory, insert_file,
        load_directories, load_files, load_list_width, probe_writable, rename_directory,
        save_list_width, touch_last_opened, update_alias,
    };
    use rusqlite::Connection;

    /// 建立与 `storage::database` 一致的表结构（内存库）
    fn setup_test_db() -> Connection {
        let conn = Connection::open_in_memory().expect("创建内存数据库失败");
        conn.execute_batch(
            "PRAGMA foreign_keys = ON;
             CREATE TABLE file_editor_directories (
                 id         INTEGER PRIMARY KEY AUTOINCREMENT,
                 name       TEXT NOT NULL UNIQUE,
                 sort_order INTEGER DEFAULT 0,
                 created_at DATETIME DEFAULT CURRENT_TIMESTAMP
             );
             CREATE TABLE file_editor_files (
                 id             INTEGER PRIMARY KEY AUTOINCREMENT,
                 directory_id   INTEGER,
                 path           TEXT NOT NULL UNIQUE,
                 alias          TEXT,
                 sort_order     INTEGER DEFAULT 0,
                 created_at     DATETIME DEFAULT CURRENT_TIMESTAMP,
                 last_opened_at DATETIME,
                 FOREIGN KEY (directory_id) REFERENCES file_editor_directories(id) ON DELETE SET NULL
             );
             CREATE TABLE file_editor_settings (
                 id         INTEGER PRIMARY KEY DEFAULT 1,
                 list_width REAL NOT NULL DEFAULT 260.0,
                 updated_at DATETIME DEFAULT CURRENT_TIMESTAMP
             );",
        )
        .expect("初始化测试表失败");

        conn
    }

    #[test]
    fn creates_renames_and_deletes_directories() {
        let conn = setup_test_db();

        let first = insert_directory(&conn, "常用配置").expect("新建目录失败");
        let second = insert_directory(&conn, "  ").expect_err("空名称应被拒绝");
        assert!(second.to_string().contains("不能为空"));

        let directories = load_directories(&conn).expect("读取目录失败");
        assert_eq!(directories.len(), 1);
        assert_eq!(directories[0].name, "常用配置");

        rename_directory(&conn, first, "配置文件").expect("重命名失败");
        assert!(insert_directory(&conn, "配置文件").is_err(), "重名应被拒绝");

        delete_directory(&conn, first).expect("删除目录失败");
        assert!(load_directories(&conn).expect("读取目录失败").is_empty());
    }

    #[test]
    fn deleting_directory_moves_files_to_ungrouped() {
        let conn = setup_test_db();
        let directory = insert_directory(&conn, "常用配置").expect("新建目录失败");
        insert_file(&conn, Some(directory), "C:\\Users\\me\\.gitconfig", None)
            .expect("新增条目失败");

        delete_directory(&conn, directory).expect("删除目录失败");

        let files = load_files(&conn).expect("读取条目失败");
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].directory_id, None, "目录删除后条目应变为未分组");
    }

    #[test]
    fn rejects_duplicate_path_ignoring_case() {
        let conn = setup_test_db();
        insert_file(&conn, None, "C:\\Users\\me\\.npmrc", None).expect("新增条目失败");

        let duplicate = insert_file(&conn, None, "c:\\users\\me\\.npmrc", None)
            .expect_err("大小写不同的同一路径应被拒绝");
        assert!(duplicate.to_string().contains("已在常用列表"));
    }

    #[test]
    fn updates_alias_and_last_opened() {
        let conn = setup_test_db();
        let id = insert_file(&conn, None, "C:\\temp\\config.toml", Some("  项目配置  "))
            .expect("新增条目失败");

        let files = load_files(&conn).expect("读取条目失败");
        assert_eq!(files[0].alias.as_deref(), Some("项目配置"));

        update_alias(&conn, id, None).expect("清空显示名失败");
        assert_eq!(load_files(&conn).expect("读取条目失败")[0].alias, None);

        touch_last_opened(&conn, id).expect("更新打开时间失败");
        assert!(
            load_files(&conn).expect("读取条目失败")[0]
                .last_opened_at
                .is_some()
        );
    }

    #[test]
    fn deletes_file_entry_without_deleting_row_of_others() {
        let conn = setup_test_db();
        let first = insert_file(&conn, None, "C:\\temp\\a.toml", None).expect("新增条目失败");
        insert_file(&conn, None, "C:\\temp\\b.toml", None).expect("新增条目失败");

        delete_file(&conn, first).expect("删除条目失败");

        let files = load_files(&conn).expect("读取条目失败");
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, "C:\\temp\\b.toml");
    }

    #[test]
    fn list_width_falls_back_to_default_and_persists() {
        let conn = setup_test_db();
        assert_eq!(
            load_list_width(&conn).expect("读取宽度失败"),
            DEFAULT_LIST_WIDTH
        );

        save_list_width(&conn, 320.0).expect("保存宽度失败");
        assert_eq!(load_list_width(&conn).expect("读取宽度失败"), 320.0);
    }

    #[test]
    fn detects_read_only_database() {
        let path = std::env::temp_dir().join(format!(
            "tools-box-file-editor-readonly-{}.db",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);

        {
            let writable = Connection::open(&path).expect("创建测试数据库失败");
            writable
                .execute_batch("CREATE TABLE probe (id INTEGER PRIMARY KEY);")
                .expect("建表失败");
            assert!(probe_writable(&writable), "可写数据库应探测为可写");
        }

        let read_only =
            Connection::open_with_flags(&path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
                .expect("以只读方式打开失败");
        assert!(!probe_writable(&read_only), "只读连接应探测为不可写");

        drop(read_only);
        let _ = std::fs::remove_file(&path);
    }
}
