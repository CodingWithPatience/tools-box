#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod plugin;
mod plugins;
mod storage;

use app::App;
use storage::Database;

fn main() -> eframe::Result<()> {
    // 初始化日志
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .init();

    log::info!("Tools Box 启动中...");

    // 初始化数据库
    let db = Database::open().expect("数据库初始化失败");

    // 配置窗口
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1024.0, 680.0])
            .with_min_inner_size([800.0, 500.0])
            .with_title("Tools Box"),
        ..Default::default()
    };

    // 启动应用
    eframe::run_native(
        "Tools Box",
        options,
        Box::new(move |cc| {
            // 配置中文字体（必须在首次渲染前完成）
            app::setup_chinese_fonts(&cc.egui_ctx);
            Ok(Box::new(App::new(db)))
        }),
    )
}
