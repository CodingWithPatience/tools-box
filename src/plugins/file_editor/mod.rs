//! 常用文件编辑器插件
//!
//! 把常用配置文件（`.gitconfig`、`Cargo.toml` 等）加入列表，随时打开、修改并保存；
//! 支持多标签、未保存关闭确认与 Ctrl+S 保存。

pub mod document;
pub mod models;
pub mod store;
pub mod ui;

use egui::Ui;

use crate::plugin::Plugin;
use crate::storage::Database;
use ui::FileEditorUi;

/// 常用文件编辑器插件
pub struct FileEditorPlugin {
    /// 界面状态
    ui: FileEditorUi,
    /// 数据库实例
    db: Option<Database>,
    /// 数据库是否可写（只读时界面给出警告，避免"建目录失败"这类隐晦报错）
    storage_writable: bool,
    /// 是否已加载数据库数据
    initialized: bool,
}

impl FileEditorPlugin {
    /// 创建新的插件实例
    pub fn new() -> Self {
        Self {
            ui: FileEditorUi::new(),
            db: None,
            storage_writable: true,
            initialized: false,
        }
    }

    /// 初始化数据库连接并探测可写性
    fn init_db(&mut self) {
        if self.db.is_none() {
            match Database::open() {
                Ok(db) => {
                    self.storage_writable = store::probe_writable(db.conn());
                    if !self.storage_writable {
                        log::error!(
                            "常用文件编辑器：数据库不可写，分组与常用文件无法保存（程序可能运行在受限目录下）"
                        );
                    }
                    self.db = Some(db);
                    log::info!("常用文件编辑器数据库连接成功");
                }
                Err(error) => {
                    log::error!("常用文件编辑器无法打开数据库: {}", error);
                }
            }
        }
    }
}

impl Plugin for FileEditorPlugin {
    fn name(&self) -> &str {
        "常用文件编辑器"
    }

    fn icon(&self) -> &str {
        "📄"
    }

    fn description(&self) -> &str {
        "管理常用配置文件并多标签编辑保存，支持 Ctrl+S"
    }

    fn render(&mut self, ui: &mut Ui) {
        self.init_db();

        if let Some(db) = &self.db {
            if !self.initialized {
                self.ui.init(db.conn());
                self.initialized = true;
            }
            self.ui.set_storage_writable(self.storage_writable);

            self.ui.render(ui, db.conn());
        } else {
            ui.centered_and_justified(|ui| {
                ui.label("⚠ 数据库连接失败，无法加载常用文件编辑器");
            });
        }
    }

    fn init(&mut self) {
        log::info!("常用文件编辑器插件已初始化");
    }
}
