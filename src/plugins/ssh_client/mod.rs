mod crypto;
pub mod models;
mod store;
mod ui;

use crate::plugin::Plugin;
use crate::storage::Database;
use store::SshStore;
use ui::SshClientUi;

/// SSH 客户端插件
pub struct SshClientPlugin {
    ui: SshClientUi,
    /// 数据库实例（懒加载）
    db: Option<Database>,
}

impl SshClientPlugin {
    pub fn new() -> Self {
        Self {
            ui: SshClientUi::new(),
            db: None,
        }
    }

    /// 初始化数据库连接
    fn init_db(&mut self) {
        if self.db.is_none() {
            match Database::open() {
                Ok(db) => {
                    log::info!("SSH 客户端数据库连接成功");
                    self.db = Some(db);
                }
                Err(e) => {
                    log::error!("SSH 客户端无法打开数据库: {}", e);
                }
            }
        }
    }
}

impl Plugin for SshClientPlugin {
    fn name(&self) -> &str {
        "SSH 客户端"
    }

    fn icon(&self) -> &str {
        "🖥"
    }

    fn description(&self) -> &str {
        "SSH 远程连接客户端，支持交互终端与会话管理"
    }

    fn render(&mut self, ui: &mut egui::Ui) {
        self.init_db();

        if let Some(db) = &self.db {
            let store = SshStore::new(db.conn());
            self.ui.render(ui, &store);
        } else {
            ui.centered_and_justified(|ui| {
                ui.label("⚠ 数据库连接失败，无法加载 SSH 客户端");
            });
        }
    }

    fn init(&mut self) {
        log::info!("SSH 客户端插件已初始化");
    }
}
