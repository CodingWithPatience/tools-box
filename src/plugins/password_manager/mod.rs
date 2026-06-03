mod crypto;
mod models;
mod store;
mod ui;

use crate::plugin::Plugin;
use crate::storage::Database;
use ui::PasswordManagerUi;

/// 密码管理器插件
pub struct PasswordManagerPlugin {
    ui: PasswordManagerUi,
    /// 数据库实例
    db: Option<Database>,
}

impl PasswordManagerPlugin {
    pub fn new() -> Self {
        Self {
            ui: PasswordManagerUi::new(),
            db: None,
        }
    }

    /// 初始化数据库连接
    fn init_db(&mut self) {
        if self.db.is_none() {
            match Database::open() {
                Ok(db) => {
                    self.db = Some(db);
                    log::info!("密码管理器数据库连接成功");
                }
                Err(e) => {
                    log::error!("密码管理器无法打开数据库: {}", e);
                }
            }
        }
    }
}

impl Plugin for PasswordManagerPlugin {
    fn name(&self) -> &str {
        "密码管理器"
    }

    fn icon(&self) -> &str {
        "🔑"
    }

    fn description(&self) -> &str {
        "针对不同网站的账号密码存储记录"
    }

    fn render(&mut self, ui: &mut egui::Ui) {
        self.init_db();

        if let Some(db) = &self.db {
            self.ui.render(ui, db.conn());
        } else {
            ui.centered_and_justified(|ui| {
                ui.label("⚠ 数据库连接失败，无法加载密码管理器");
            });
        }
    }

    fn init(&mut self) {
        log::info!("密码管理器插件已初始化");
    }

    fn cleanup(&mut self) {
        log::info!("密码管理器插件已清理");
    }
}
