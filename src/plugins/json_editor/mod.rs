mod processor;
mod ui;

use crate::plugin::Plugin;
use ui::JsonEditorUi;

/// JSON 编辑器插件
pub struct JsonEditorPlugin {
    editor_ui: JsonEditorUi,
}

impl JsonEditorPlugin {
    pub fn new() -> Self {
        Self {
            editor_ui: JsonEditorUi::new(),
        }
    }
}

impl Plugin for JsonEditorPlugin {
    fn name(&self) -> &str {
        "JSON 编辑器"
    }

    fn icon(&self) -> &str {
        "📋"
    }

    fn description(&self) -> &str {
        "JSON 格式化、压缩、转义与解析"
    }

    fn render(&mut self, ui: &mut egui::Ui) {
        self.editor_ui.render(ui);
    }

    fn init(&mut self) {
        log::info!("JSON 编辑器插件已初始化");
    }
}
