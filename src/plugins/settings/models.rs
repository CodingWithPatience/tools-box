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
            tool_hotkeys: vec!['1', '2', '3', '4', '5', '6', '7', '8'],
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

    /// 按插件数量补齐缺失的工具热键，返回设置是否发生变化
    ///
    /// 新增插件后会多出一个工具，而老数据库里保存的热键数量可能不足；
    /// 这里按未占用的字符（数字优先）自动补位，避免新工具没有可用热键。
    pub fn ensure_tool_hotkeys(&mut self, plugin_count: usize) -> bool {
        if self.tool_hotkeys.len() >= plugin_count {
            return false;
        }

        let candidates: Vec<char> = "1234567890ABCDEFGHIJKLMNOPQRSTUVWXYZ".chars().collect();
        while self.tool_hotkeys.len() < plugin_count {
            let Some(candidate) = candidates
                .iter()
                .copied()
                .find(|candidate| !self.tool_hotkeys.contains(candidate))
            else {
                break;
            };
            self.tool_hotkeys.push(candidate);
        }

        log::info!(
            "已为新增工具补齐热键: {}",
            self.tool_hotkeys.iter().collect::<String>()
        );
        true
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

#[cfg(test)]
mod tests {
    use super::AppSettings;

    #[test]
    fn keeps_hotkeys_when_plugin_count_matches() {
        let mut settings = AppSettings::default();

        assert!(!settings.ensure_tool_hotkeys(8));
        assert_eq!(
            settings.tool_hotkeys,
            vec!['1', '2', '3', '4', '5', '6', '7', '8']
        );
    }

    #[test]
    fn fills_missing_hotkeys_with_unused_characters() {
        let mut settings = AppSettings::default();
        settings.tool_hotkeys = vec!['1', '2', '3'];

        assert!(settings.ensure_tool_hotkeys(6));
        assert_eq!(settings.tool_hotkeys, vec!['1', '2', '3', '4', '5', '6']);
    }

    #[test]
    fn skips_characters_already_used_by_other_tools() {
        let mut settings = AppSettings::default();
        settings.tool_hotkeys = vec!['8', '2'];

        assert!(settings.ensure_tool_hotkeys(4));
        assert_eq!(settings.tool_hotkeys, vec!['8', '2', '1', '3']);
    }
}
