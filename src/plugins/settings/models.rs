//! 设置插件数据模型
//!
//! 定义应用设置的数据结构和数据库读写方法。

use anyhow::{Context, Result};
use rusqlite::Connection;

/// 应用设置（持久化到 SQLite app_settings 表）
#[derive(Debug, Clone)]
pub struct AppSettings {
    /// 主题："dark" | "light"
    pub theme: String,
    /// 字体大小（10.0 ~ 24.0）
    pub font_size: f32,
    /// 侧边栏宽度
    pub sidebar_width: f32,
    /// 各工具的自定义热键（插件索引 → 虚拟键码字符，如 '1', 'a' 等）
    pub tool_hotkeys: Vec<char>,
    /// 是否跟随系统启动
    pub auto_start: bool,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            theme: "dark".to_string(),
            font_size: 14.0,
            sidebar_width: 200.0,
            tool_hotkeys: vec!['1', '2', '3', '4', '5', '6', '7'],
            auto_start: false,
        }
    }
}

impl AppSettings {
    /// 从数据库加载设置，如果不存在则返回默认值
    pub fn load(conn: &Connection) -> Result<Self> {
        let result = conn.query_row(
            "SELECT theme, font_size, sidebar_width, tool_hotkeys, auto_start FROM app_settings WHERE id = 1",
            [],
            |row| {
                let hotkeys_str: String = row.get(3)?;
                let hotkeys = hotkeys_str.chars().collect();
                Ok(Self {
                    theme: row.get(0)?,
                    font_size: row.get(1)?,
                    sidebar_width: row.get(2)?,
                    tool_hotkeys: hotkeys,
                    auto_start: row.get(4)?,
                })
            },
        );

        match result {
            Ok(settings) => Ok(settings),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(Self::default()),
            Err(e) => Err(e).context("加载应用设置失败"),
        }
    }

    /// 保存设置到数据库
    pub fn save(&self, conn: &Connection) -> Result<()> {
        let hotkeys_str: String = self.tool_hotkeys.iter().collect();
        log::info!(
            "保存设置: theme={}, font_size={}, sidebar_width={}, hotkeys={}, auto_start={}",
            self.theme,
            self.font_size,
            self.sidebar_width,
            hotkeys_str,
            self.auto_start
        );
        conn.execute(
            "INSERT OR REPLACE INTO app_settings (id, theme, font_size, sidebar_width, tool_hotkeys, auto_start, updated_at)
             VALUES (1, ?1, ?2, ?3, ?4, ?5, CURRENT_TIMESTAMP)",
            rusqlite::params![self.theme, self.font_size, self.sidebar_width, hotkeys_str, self.auto_start],
        )
        .map_err(|e| {
            log::error!("SQL 执行失败: {}", e);
            anyhow::anyhow!("保存应用设置失败: {}", e)
        })?;
        log::info!("设置保存成功");
        Ok(())
    }

}
