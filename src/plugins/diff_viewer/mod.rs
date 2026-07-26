mod differ;
mod models;
mod ui;

use crate::plugin::Plugin;
use ui::DiffViewerUi;

/// 文本对比工具插件
pub struct DiffViewerPlugin {
    viewer_ui: DiffViewerUi,
}

impl DiffViewerPlugin {
    pub fn new() -> Self {
        Self {
            viewer_ui: DiffViewerUi::new(),
        }
    }
}

impl Plugin for DiffViewerPlugin {
    fn name(&self) -> &str {
        "文本对比"
    }

    fn icon(&self) -> &str {
        "📝"
    }

    fn description(&self) -> &str {
        "文本/代码差异对比工具，支持 Split 和 Unified 视图"
    }

    fn render(&mut self, ui: &mut egui::Ui) {
        self.viewer_ui.render(ui);
    }

    fn init(&mut self) {
        log::info!("文本对比工具插件已初始化");
    }
}
