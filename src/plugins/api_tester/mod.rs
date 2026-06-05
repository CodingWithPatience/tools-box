mod client;
mod mock;
mod models;
mod store;
mod ui;

use crate::plugin::Plugin;
use crate::storage::Database;
use store::ApiStore;
use ui::ApiTesterUi;

/// API 调试工具插件
pub struct ApiTesterPlugin {
    /// UI 状态
    ui: ApiTesterUi,
    /// 数据库实例
    db: Option<Database>,
    /// 是否已初始化
    initialized: bool,
}

impl ApiTesterPlugin {
    pub fn new() -> Self {
        Self {
            ui: ApiTesterUi::new(),
            db: None,
            initialized: false,
        }
    }

    /// 初始化数据库连接
    fn init_db(&mut self) {
        if self.db.is_none() {
            match Database::open() {
                Ok(db) => {
                    self.db = Some(db);
                    log::info!("API 调试工具数据库连接成功");
                }
                Err(e) => {
                    log::error!("API 调试工具无法打开数据库: {}", e);
                }
            }
        }
    }
}

impl Plugin for ApiTesterPlugin {
    fn name(&self) -> &str {
        "API 调试工具"
    }

    fn icon(&self) -> &str {
        "⚙"
    }

    fn description(&self) -> &str {
        "轻量级 HTTP API 调试工具，类似 Postman"
    }

    fn render(&mut self, ui: &mut egui::Ui) {
        self.init_db();

        if let Some(db) = &self.db {
            // 首次进入时初始化表和 UI
            if !self.initialized {
                let store = ApiStore::new(db.conn());
                if let Err(e) = store.init_table() {
                    log::error!("API 调试工具初始化表失败: {}", e);
                }
                self.ui.init(db.conn());
                self.initialized = true;
            }

            self.ui.render(ui, db.conn());
        } else {
            ui.centered_and_justified(|ui| {
                ui.label("⚠ 数据库连接失败，无法加载 API 调试工具");
            });
        }
    }

    fn init(&mut self) {
        log::info!("API 调试工具插件已初始化");
    }

    fn cleanup(&mut self) {
        log::info!("API 调试工具插件已清理");
    }
}
