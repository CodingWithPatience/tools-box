#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod hotkey;
mod plugin;
mod plugins;
mod storage;
mod tray;
mod utils;

use app::App;
use plugins::settings::AppSettings;
use storage::Database;

fn main() -> eframe::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .init();

    log::info!("Tools Box 启动中...");

    let db = Database::open().expect("数据库初始化失败");

    // 从数据库加载设置
    let settings = AppSettings::load(db.conn()).unwrap_or_default();

    // 创建系统托盘
    let tray_manager = tray::TrayManager::new();

    // 根据设置构建热键绑定（而非使用默认值）
    let plugin_count = plugins::register_all_plugins().len();
    let bindings = build_hotkey_bindings(&settings, plugin_count);
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

/// 根据设置构建热键绑定列表
fn build_hotkey_bindings(
    settings: &AppSettings,
    plugin_count: usize,
) -> Vec<hotkey::HotkeyBinding> {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;

    let mut bindings = Vec::new();

    // 主窗口唤出: Ctrl+Alt+Space（固定）
    bindings.push(hotkey::HotkeyBinding {
        id: 1,
        modifiers: MOD_CONTROL | MOD_ALT,
        vk: VK_SPACE as u32,
        plugin_index: usize::MAX,
    });

    // 各工具的自定义热键
    for (i, &ch) in settings.tool_hotkeys.iter().enumerate() {
        if i >= plugin_count {
            break;
        }
        let vk = ch.to_ascii_uppercase() as u32;
        bindings.push(hotkey::HotkeyBinding {
            id: 2 + i as i32,
            modifiers: MOD_CONTROL | MOD_ALT,
            vk,
            plugin_index: i,
        });
    }

    bindings
}
