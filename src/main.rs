#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod plugin;
mod plugins;
mod storage;
mod tray;
mod utils;

use app::App;
use storage::Database;

fn main() -> eframe::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .init();

    log::info!("Tools Box 启动中...");

    let db = Database::open().expect("数据库初始化失败");
    let tray_manager = tray::TrayManager::new();

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1024.0, 680.0])
            .with_min_inner_size([800.0, 500.0])
            .with_title("Tools Box"),
        run_and_return: true,
        ..Default::default()
    };

    eframe::run_native(
        "Tools Box",
        options,
        Box::new(move |cc| {
            app::setup_chinese_fonts(&cc.egui_ctx);
            Ok(Box::new(App::new(db, tray_manager, cc.egui_ctx.clone())))
        }),
    )?;

    log::info!("Tools Box 已退出");
    Ok(())
}
