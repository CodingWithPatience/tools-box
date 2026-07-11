#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod hotkey;
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

    // 创建系统托盘
    let tray_manager = tray::TrayManager::new();

    // 创建全局热键管理器
    let plugin_count = plugins::register_all_plugins().len();
    let bindings = hotkey::default_bindings(plugin_count);
    let hotkey_manager = hotkey::HotkeyManager::new(bindings);

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

            // 设置全局 egui Context，用于热键和托盘事件唤醒事件循环
            tray::set_egui_ctx(cc.egui_ctx.clone());
            hotkey::HotkeyManager::set_egui_ctx(cc.egui_ctx.clone());

            Ok(Box::new(App::new(db, tray_manager, hotkey_manager)))
        }),
    )?;

    log::info!("Tools Box 已退出");
    Ok(())
}
