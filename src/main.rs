#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod hotkey;
mod instance;
mod plugin;
mod plugins;
mod storage;
mod tray;
mod utils;

use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};

use app::App;
use plugins::settings::AppSettings;
use storage::Database;

/// 日志文件大小上限（字节）：超过后归档为 `tools-box.log.old` 并重新开始写入。
const LOG_FILE_MAX_BYTES: u64 = 5 * 1024 * 1024;
/// 日志文件名。
const LOG_FILE_NAME: &str = "tools-box.log";

fn main() -> eframe::Result<()> {
    init_logging();

    log::info!("Tools Box 启动中...");

    let _instance_guard = match instance::acquire_single_instance_or_notify(|| {
        log::info!("检测到 Tools Box 已运行，正在显示现有窗口");
        let notified = tray::notify_existing_instance();
        if !notified {
            log::warn!("通知已有实例失败，正在重新检查主实例状态");
        }
        notified
    }) {
        Ok(instance::SingleInstanceState::Primary(guard)) => guard,
        Ok(instance::SingleInstanceState::Existing) => {
            return Ok(());
        }
        Err(error) => {
            log::error!("单实例检测失败，取消启动: {}", error);
            return Ok(());
        }
    };

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
            plugins::note_taker::markdown::install_markdown_image_loader(&cc.egui_ctx);

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

/// 初始化日志：同时写入 `%APPDATA%\tools-box\logs\tools-box.log` 与标准错误。
///
/// Windows GUI 子系统构建没有控制台，只写标准错误的日志会被直接丢弃
/// （托盘注册失败、窗口唤醒失败等关键信息都收不到），因此这里把日志同时落盘。
fn init_logging() {
    let mut builder =
        env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"));

    match open_log_writer() {
        Ok(writer) => {
            builder.target(env_logger::Target::Pipe(Box::new(writer)));
        }
        Err(error) => {
            eprintln!("日志文件不可用，日志仅写入标准错误: {error}");
        }
    }

    builder.init();
}

/// 打开日志文件（必要时先归档旧日志），返回同时写文件与标准错误的写入器。
fn open_log_writer() -> std::io::Result<TeeWriter> {
    let path = log_file_path().ok_or_else(|| std::io::Error::other("无法获取系统数据目录"))?;

    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    rotate_log_if_needed(&path)?;

    let file = OpenOptions::new().create(true).append(true).open(&path)?;
    Ok(TeeWriter { file })
}

/// 日志文件路径：`%APPDATA%\tools-box\logs\tools-box.log`。
fn log_file_path() -> Option<PathBuf> {
    let data_dir = dirs::data_dir()?;
    Some(data_dir.join("tools-box").join("logs").join(LOG_FILE_NAME))
}

/// 日志超过大小上限时归档为 `tools-box.log.old`。
fn rotate_log_if_needed(path: &Path) -> std::io::Result<()> {
    let Ok(metadata) = std::fs::metadata(path) else {
        return Ok(());
    };
    if !should_rotate_log(metadata.len()) {
        return Ok(());
    }

    std::fs::rename(path, path.with_extension("log.old"))
}

/// 判断日志文件是否已达到需要归档的大小。
fn should_rotate_log(len: u64) -> bool {
    len >= LOG_FILE_MAX_BYTES
}

/// 同时写入日志文件与标准错误的日志写入器。
///
/// GUI 子系统下标准错误通常无效，写入失败会被忽略，不影响文件日志。
struct TeeWriter {
    file: std::fs::File,
}

impl Write for TeeWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let written = self.file.write(buf)?;
        let _ = std::io::stderr().write_all(&buf[..written]);
        Ok(written)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.file.flush()?;
        let _ = std::io::stderr().flush();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{LOG_FILE_MAX_BYTES, should_rotate_log};

    #[test]
    fn rotates_log_only_after_reaching_size_limit() {
        assert!(!should_rotate_log(0));
        assert!(!should_rotate_log(LOG_FILE_MAX_BYTES - 1));
        assert!(should_rotate_log(LOG_FILE_MAX_BYTES));
    }
}
