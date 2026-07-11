//! 设置插件
//!
//! 提供主题、字体大小、侧边栏宽度、自定义热键、跟随系统启动等应用设置。
//! 不在侧边栏显示，通过右上角 ⚙ 按钮进入。

mod models;
mod ui;

pub use models::AppSettings;
pub use ui::SettingsChange;

/// 设置插件
pub struct SettingsPlugin {
    settings_ui: ui::SettingsUi,
}

impl SettingsPlugin {
    pub fn new(settings: AppSettings, plugin_names: Vec<String>) -> Self {
        Self {
            settings_ui: ui::SettingsUi::new(settings, plugin_names),
        }
    }

    /// 获取当前设置
    pub fn settings(&self) -> &AppSettings {
        self.settings_ui.settings()
    }

    /// 从外部更新设置
    pub fn update_settings(&mut self, settings: AppSettings) {
        self.settings_ui.update_settings(settings);
    }

    /// 标记为已保存
    pub fn mark_saved(&mut self) {
        self.settings_ui.mark_saved();
    }

    /// 渲染设置面板并返回变更信息
    pub fn render(&mut self, ui: &mut egui::Ui) -> SettingsChange {
        self.settings_ui.render(ui)
    }
}
