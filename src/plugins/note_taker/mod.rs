pub mod markdown;
pub mod models;
pub mod store;
pub mod ui;

use egui::Ui;

use crate::plugin::Plugin;
use crate::storage::Database;
use ui::NoteTakerUi;

/// 临时笔记插件
pub struct NoteTakerPlugin {
    /// UI 状态
    ui: NoteTakerUi,
    /// 数据库实例
    db: Option<Database>,
    /// 是否已初始化
    initialized: bool,
}

impl NoteTakerPlugin {
    /// 创建新的插件实例
    pub fn new() -> Self {
        Self {
            ui: NoteTakerUi::new(),
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
                    log::info!("临时笔记数据库连接成功");
                }
                Err(e) => {
                    log::error!("临时笔记无法打开数据库: {}", e);
                }
            }
        }
    }
}

impl Plugin for NoteTakerPlugin {
    fn name(&self) -> &str {
        "临时笔记"
    }

    fn icon(&self) -> &str {
        "📝"
    }

    fn description(&self) -> &str {
        "记录临时文本，支持目录分类和 Markdown 格式"
    }

    fn render(&mut self, ui: &mut Ui) {
        self.init_db();

        if let Some(db) = &self.db {
            // 首次进入时初始化表和 UI
            if !self.initialized {
                self.ui.init(db.conn());
                self.initialized = true;
            }

            self.ui.render(ui, db.conn());
        } else {
            ui.centered_and_justified(|ui| {
                ui.label("⚠ 数据库连接失败，无法加载临时笔记");
            });
        }
    }

    fn init(&mut self) {
        log::info!("临时笔记插件已初始化");
    }

    fn cleanup(&mut self) {
        log::info!("临时笔记插件已清理");
    }
}