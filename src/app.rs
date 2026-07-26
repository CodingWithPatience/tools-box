use crate::hotkey::HotkeyManager;
use crate::plugin::Plugin;
use crate::plugins;
use crate::plugins::settings::{AppSettings, SettingsPlugin};
use crate::storage::Database;
use crate::tray::{TrayEvent, TrayManager};
use egui::FontFamily;
use raw_window_handle::HasWindowHandle;
use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    IsIconic, SW_HIDE, SW_RESTORE, SW_SHOW, SetForegroundWindow, ShowWindow,
};

/// 侧边栏面板的持久化状态 ID。
const SIDEBAR_PANEL_ID: &str = "sidebar";

/// 设置原生 Windows 窗口的可见性。
///
/// # Safety
/// `hwnd` 必须是当前进程持有的有效窗口句柄。
unsafe fn set_native_window_visible(hwnd: HWND, visible: bool) {
    let is_minimized = if visible {
        // SAFETY: 调用方保证 hwnd 是当前进程持有的有效窗口句柄。
        unsafe { IsIconic(hwnd) != 0 }
    } else {
        false
    };
    let command = match (visible, is_minimized) {
        (false, _) => SW_HIDE,
        (true, true) => SW_RESTORE,
        (true, false) => SW_SHOW,
    };

    // SAFETY: 调用方保证 hwnd 是当前进程持有的有效窗口句柄。
    unsafe {
        ShowWindow(hwnd, command);
    }
}

/// 应用侧边栏宽度，并清除 egui 保存的旧面板尺寸。
fn apply_sidebar_width(ctx: &egui::Context, current_width: &mut f32, new_width: f32) {
    if (*current_width - new_width).abs() <= f32::EPSILON {
        return;
    }

    *current_width = new_width;
    ctx.data_mut(|data| {
        data.remove::<egui::containers::panel::PanelState>(egui::Id::new(SIDEBAR_PANEL_ID));
    });
    ctx.request_repaint();
}

fn sidebar_panel(width: f32) -> egui::SidePanel {
    egui::SidePanel::left(SIDEBAR_PANEL_ID)
        .resizable(true)
        .default_width(width)
        .width_range(150.0..=400.0)
}

/// 主题模式
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Theme {
    Light,
    Dark,
}

/// 主应用状态
pub struct App {
    /// 已注册的插件列表
    plugins: Vec<Box<dyn Plugin>>,
    /// 当前选中的插件索引
    selected: usize,
    /// 侧边栏搜索关键词
    search_query: String,
    /// 状态栏消息
    status_message: String,
    /// 当前主题
    theme: Theme,
    /// 是否聚焦搜索框
    focus_search: bool,
    /// 字体大小
    font_size: f32,
    /// 是否已应用字体设置
    font_applied: bool,
    /// 侧边栏宽度（手动管理，防止自动扩展）
    sidebar_width: f32,
    /// 全局热键管理器（先于托盘管理器析构，避免退出时继续访问主窗口句柄）
    hotkey_manager: HotkeyManager,
    /// 系统托盘管理器（保持存活以维持托盘图标）
    tray_manager: TrayManager,
    /// 最近使用的插件索引（用于全局热键唤出）
    last_active_tool: usize,
    /// 窗口是否可见
    window_visible: bool,
    /// 是否处于设置面板模式
    settings_mode: bool,
    /// 是否显示保存确认弹窗
    show_save_confirm: bool,
    /// 设置插件（不在侧边栏，通过右上角按钮访问）
    settings_plugin: SettingsPlugin,
    /// 数据库连接（用于保存设置）
    db: Database,
}

/// 配置中文字体和 Emoji 字体
///
/// 从 Windows 系统字体目录加载 Microsoft YaHei（中文）和 Segoe UI Emoji（表情符号），
/// 确保中文和 Emoji 图标正常显示。
pub fn setup_chinese_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();

    // 尝试加载 Microsoft YaHei 字体（中文支持）
    let chinese_font_paths = [
        r"C:\Windows\Fonts\msyh.ttc",
        r"C:\Windows\Fonts\msyhbd.ttc",
        r"C:\Windows\Fonts\simhei.ttf",
        r"C:\Windows\Fonts\simsun.ttc",
    ];

    let mut chinese_loaded = false;
    for path in &chinese_font_paths {
        if let Ok(font_data) = std::fs::read(path) {
            fonts.font_data.insert(
                "chinese".to_owned(),
                egui::FontData::from_owned(font_data).into(),
            );

            // 将中文字体设为 Proportional 和 Monospace 的首选 fallback
            if let Some(family) = fonts.families.get_mut(&FontFamily::Proportional) {
                family.insert(0, "chinese".to_owned());
            }
            if let Some(family) = fonts.families.get_mut(&FontFamily::Monospace) {
                family.push("chinese".to_owned());
            }

            log::info!("已加载中文字体: {}", path);
            chinese_loaded = true;
            break;
        }
    }

    if !chinese_loaded {
        log::warn!("未找到中文字体，中文可能显示为乱码");
    }

    // 尝试加载 Emoji 字体（支持 Unicode 表情符号）
    let emoji_font_paths = [
        r"C:\Windows\Fonts\seguiemj.ttf", // Segoe UI Emoji
        r"C:\Windows\Fonts\seguisym.ttf", // Segoe UI Symbol
    ];

    let mut emoji_loaded = false;
    for path in &emoji_font_paths {
        if let Ok(font_data) = std::fs::read(path) {
            fonts.font_data.insert(
                "emoji".to_owned(),
                egui::FontData::from_owned(font_data).into(),
            );

            // 将 Emoji 字体添加为 fallback
            if let Some(family) = fonts.families.get_mut(&FontFamily::Proportional) {
                family.push("emoji".to_owned());
            }
            if let Some(family) = fonts.families.get_mut(&FontFamily::Monospace) {
                family.push("emoji".to_owned());
            }

            log::info!("已加载 Emoji 字体: {}", path);
            emoji_loaded = true;
            break;
        }
    }

    if !emoji_loaded {
        log::warn!("未找到 Emoji 字体，部分图标可能显示为方框");
    }

    ctx.set_fonts(fonts);
}

impl App {
    /// 创建应用实例
    ///
    /// # 参数
    /// - `db`: SQLite 数据库连接
    /// - `tray_manager`: 系统托盘管理器
    pub fn new(db: Database, tray_manager: TrayManager, hotkey_manager: HotkeyManager) -> Self {
        // 从数据库加载设置
        let current_settings = AppSettings::load(db.conn()).unwrap_or_default();

        let mut plugins = plugins::register_all_plugins();

        // 收集插件名称（用于设置面板的热键配置显示）
        let plugin_names: Vec<String> = plugins.iter().map(|p| p.name().to_string()).collect();

        // 创建设置插件（不在侧边栏，通过右上角按钮访问）
        let settings_plugin = SettingsPlugin::new(current_settings.clone(), plugin_names);

        for plugin in plugins.iter_mut() {
            plugin.init();
        }

        // 从设置中恢复主题
        let theme = if current_settings.theme == "light" {
            Theme::Light
        } else {
            Theme::Dark
        };

        Self {
            plugins,
            selected: 0,
            search_query: String::new(),
            db,
            status_message: "就绪".to_string(),
            theme,
            focus_search: false,
            font_size: current_settings.font_size,
            font_applied: false,
            sidebar_width: current_settings.sidebar_width,
            hotkey_manager,
            tray_manager,
            last_active_tool: 0,
            window_visible: true,
            settings_mode: false,
            show_save_confirm: false,
            settings_plugin,
        }
    }

    /// 处理全局热键事件
    fn process_hotkey_events(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        for event in self.hotkey_manager.poll_events() {
            if event.plugin_index == usize::MAX {
                // Ctrl+Alt+Space：切换窗口显示/隐藏
                if self.window_visible {
                    log::info!("[热键] Ctrl+Alt+Space → 最小化到托盘");
                    self.hide_window(frame);
                } else {
                    log::info!(
                        "[热键] Ctrl+Alt+Space → 恢复窗口，工具={}",
                        self.last_active_tool
                    );
                    self.selected = self.last_active_tool;
                    self.show_window(ctx, frame);
                    self.status_message =
                        format!("热键唤出：{}", self.plugins[self.last_active_tool].name());
                }
            } else if event.plugin_index < self.plugins.len() {
                // Ctrl+Alt+数字：跳转到指定插件并显示窗口
                log::info!("[热键] 唤出插件索引={}", event.plugin_index);
                self.selected = event.plugin_index;
                self.last_active_tool = event.plugin_index;
                if !self.window_visible {
                    self.show_window(ctx, frame);
                }
                self.status_message =
                    format!("热键唤出：{}", self.plugins[event.plugin_index].name());
            }
        }
    }

    /// 处理托盘事件
    ///
    /// 通过 tray_manager 的 receiver 轮询 TrayIconEvent 和 MenuEvent
    fn process_tray_events(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        for event in self.tray_manager.poll_events() {
            match event {
                TrayEvent::ToggleVisible => {
                    log::info!("[托盘事件] 切换窗口可见性，当前={}", self.window_visible);
                    if self.window_visible {
                        self.hide_window(frame);
                    } else {
                        self.show_window(ctx, frame);
                    }
                }
                TrayEvent::ShowWindow => {
                    log::info!("[托盘事件] 显示窗口");
                    self.show_window(ctx, frame);
                }
            }
        }
    }

    /// 隐藏窗口到系统托盘
    ///
    /// 使用 Win32 `SW_HIDE` 完全隐藏窗口，不在桌面或任务栏保留最小化窗口。
    fn hide_window(&mut self, frame: &mut eframe::Frame) {
        self.window_visible = false;
        self.status_message = "已最小化到系统托盘".to_string();

        if let Ok(handle) = frame.window_handle() {
            if let raw_window_handle::RawWindowHandle::Win32(h) = handle.as_raw() {
                let hwnd = h.hwnd.get() as _;
                crate::tray::set_main_window_handle(hwnd);
                // SAFETY: hwnd 是有效的 Win32 窗口句柄
                unsafe {
                    set_native_window_visible(hwnd, false);
                }
                log::info!("窗口已通过 SW_HIDE 完全隐藏到系统托盘");
            }
        }

        log::info!("窗口已最小化到系统托盘");
    }

    /// 从系统托盘恢复窗口
    fn show_window(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        self.window_visible = true;
        self.status_message = "窗口已恢复".to_string();

        if let Ok(handle) = frame.window_handle() {
            if let raw_window_handle::RawWindowHandle::Win32(h) = handle.as_raw() {
                let hwnd = h.hwnd.get() as _;
                // SAFETY: hwnd 是有效的 Win32 窗口句柄
                unsafe {
                    set_native_window_visible(hwnd, true);
                    SetForegroundWindow(hwnd);
                }
                log::info!("窗口已从系统托盘恢复");
            }
        }

        // 强制重绘，解决恢复后内容空白问题
        ctx.request_repaint();
    }

    /// 渲染顶部标题栏
    fn render_top_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.heading("🛠 Tools Box");

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if self.settings_mode {
                    // 设置模式：显示返回按钮
                    if ui.button("← 返回").clicked() {
                        if self.settings_plugin.commit_pending_sidebar_width() {
                            apply_sidebar_width(
                                ui.ctx(),
                                &mut self.sidebar_width,
                                self.settings_plugin.settings().sidebar_width,
                            );
                        }
                        self.settings_mode = false;
                    }
                } else {
                    // 正常模式：显示设置按钮
                    if ui.button("⚙ 设置").clicked() {
                        self.settings_mode = true;
                    }
                }

                ui.separator();

                // 状态消息
                ui.label(&self.status_message);
            });
        });
        ui.separator();
    }

    /// 执行保存设置到数据库
    fn do_save_settings(&mut self) {
        match self.settings_plugin.settings().save(self.db.conn()) {
            Ok(()) => {
                self.settings_plugin.mark_saved();
                self.status_message = "设置已保存".to_string();
                log::info!("设置已保存到数据库");

                // 更新热键绑定
                let settings = self.settings_plugin.settings();
                let new_bindings = self.build_hotkey_bindings(settings);
                self.hotkey_manager.update_bindings(new_bindings);

                // 更新侧边栏宽度
                self.sidebar_width = settings.sidebar_width;
            }
            Err(e) => {
                log::error!("保存设置失败: {}", e);
                self.status_message = format!("保存失败: {}", e);
            }
        }
    }

    /// 根据设置构建热键绑定列表
    fn build_hotkey_bindings(&self, settings: &AppSettings) -> Vec<crate::hotkey::HotkeyBinding> {
        let mut bindings = Vec::new();

        // 主窗口唤出: Ctrl+Alt+Space（固定）
        bindings.push(crate::hotkey::HotkeyBinding {
            id: 1,
            modifiers: windows_sys::Win32::UI::Input::KeyboardAndMouse::MOD_CONTROL
                | windows_sys::Win32::UI::Input::KeyboardAndMouse::MOD_ALT,
            vk: windows_sys::Win32::UI::Input::KeyboardAndMouse::VK_SPACE as u32,
            plugin_index: usize::MAX, // usize::MAX 表示恢复最近工具
        });

        // 各工具的自定义热键
        for (i, &ch) in settings.tool_hotkeys.iter().enumerate() {
            if i >= self.plugins.len() {
                break;
            }
            let vk = ch.to_ascii_uppercase() as u32;
            bindings.push(crate::hotkey::HotkeyBinding {
                id: 2 + i as i32,
                modifiers: windows_sys::Win32::UI::Input::KeyboardAndMouse::MOD_CONTROL
                    | windows_sys::Win32::UI::Input::KeyboardAndMouse::MOD_ALT,
                vk,
                plugin_index: i,
            });
        }

        bindings
    }

    /// 应用字体大小设置
    fn apply_font_size(&self, ctx: &egui::Context) {
        let mut style = (*ctx.style()).clone();
        style.text_styles = [
            (
                egui::TextStyle::Body,
                egui::FontId::new(self.font_size, FontFamily::Proportional),
            ),
            (
                egui::TextStyle::Button,
                egui::FontId::new(self.font_size, FontFamily::Proportional),
            ),
            (
                egui::TextStyle::Small,
                egui::FontId::new(self.font_size - 2.0, FontFamily::Proportional),
            ),
            (
                egui::TextStyle::Heading,
                egui::FontId::new(self.font_size + 4.0, FontFamily::Proportional),
            ),
            (
                egui::TextStyle::Monospace,
                egui::FontId::new(self.font_size, FontFamily::Monospace),
            ),
        ]
        .into();
        ctx.set_style(style);
        log::info!("字体大小已设置为: {}", self.font_size);
    }

    /// 处理快捷键
    fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        ctx.input(|i| {
            // Ctrl+F 聚焦搜索框
            if i.key_pressed(egui::Key::F) && i.modifiers.ctrl {
                self.focus_search = true;
            }

            // Escape 清空搜索
            if i.key_pressed(egui::Key::Escape) {
                if !self.search_query.is_empty() {
                    self.search_query.clear();
                    self.status_message = "已清空搜索".to_string();
                }
            }
        });
    }

    /// 渲染左侧边栏
    fn render_sidebar(&mut self, ui: &mut egui::Ui) {
        // 搜索框 - 限制宽度不超过侧边栏
        ui.horizontal(|ui| {
            // 设置布局宽度为可用宽度，防止扩展侧边栏
            let available_width = ui.available_width();
            ui.set_min_width(available_width);

            ui.label("🔍");

            // 计算搜索框宽度：可用宽度减去图标和清空按钮的空间
            let button_space = if !self.search_query.is_empty() {
                30.0
            } else {
                0.0
            };
            let search_width = (available_width - 50.0 - button_space).max(100.0);

            let response = ui.add_sized(
                [search_width, ui.spacing().interact_size.y],
                egui::TextEdit::singleline(&mut self.search_query)
                    .hint_text("搜索插件... (Ctrl+F)"),
            );

            // 自动聚焦搜索框
            if self.focus_search {
                response.request_focus();
                self.focus_search = false;
            }

            // 清空按钮
            if !self.search_query.is_empty() {
                if ui.button("✕").clicked() {
                    self.search_query.clear();
                }
            }
        });

        ui.add_space(4.0);

        // 过滤后的插件索引列表
        let query = self.search_query.to_lowercase();
        let filtered: Vec<usize> = self
            .plugins
            .iter()
            .enumerate()
            .filter(|(_, p)| {
                query.is_empty()
                    || p.name().to_lowercase().contains(&query)
                    || p.description().to_lowercase().contains(&query)
            })
            .map(|(i, _)| i)
            .collect();

        // 搜索结果提示
        if !self.search_query.is_empty() {
            ui.horizontal(|ui| {
                ui.weak(format!("找到 {} 个插件", filtered.len()));
            });
            ui.add_space(4.0);
        }

        // 插件列表
        egui::ScrollArea::vertical()
            .id_salt("sidebar_plugin_list")
            .show(ui, |ui| {
                for (_list_idx, &idx) in filtered.iter().enumerate() {
                    let plugin = &self.plugins[idx];
                    let is_selected = self.selected == idx;

                    let text = format!("{} {}", plugin.icon(), plugin.name());

                    let response = ui.add_sized(
                        [ui.available_width(), 36.0],
                        egui::SelectableLabel::new(is_selected, text),
                    );

                    if response.clicked() {
                        self.selected = idx;
                        self.last_active_tool = idx;
                    }

                    // 鼠标悬停时显示描述
                    if response.hovered() {
                        response.on_hover_text(plugin.description());
                    }
                }
            });
    }

    /// 渲染右侧插件内容区
    fn render_plugin_content(&mut self, ui: &mut egui::Ui) {
        if self.settings_mode {
            // 设置模式：渲染设置面板
            let change = self.settings_plugin.render(ui);

            // 处理主题变更（即时生效）
            if change.theme_changed {
                let theme = self.settings_plugin.settings().theme.clone();
                if theme == "light" {
                    ui.ctx().set_visuals(egui::Visuals::light());
                } else {
                    ui.ctx().set_visuals(egui::Visuals::dark());
                }
            }

            // 处理字体大小变更（即时生效）
            if change.font_size_changed {
                self.font_size = self.settings_plugin.settings().font_size;
                self.apply_font_size(ui.ctx());
            }

            // 处理侧边栏宽度变更（即时生效）
            if change.sidebar_width_changed {
                apply_sidebar_width(
                    ui.ctx(),
                    &mut self.sidebar_width,
                    self.settings_plugin.settings().sidebar_width,
                );
            }

            // 处理保存请求（显示确认弹窗）
            if change.save_requested {
                self.show_save_confirm = true;
            }

            // 处理恢复默认（直接保存，无需确认）
            if change.reset_requested {
                self.do_save_settings();
            }

            // 渲染保存确认弹窗
            if self.show_save_confirm {
                egui::Window::new("确认保存")
                    .collapsible(false)
                    .resizable(false)
                    .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                    .show(ui.ctx(), |ui| {
                        ui.label("确定要保存当前设置吗？");
                        ui.add_space(8.0);
                        ui.horizontal(|ui| {
                            if ui.button("✅ 确定保存").clicked() {
                                self.do_save_settings();
                                self.show_save_confirm = false;
                            }
                            if ui.button("❌ 取消").clicked() {
                                self.show_save_confirm = false;
                            }
                        });
                    });
            }

            return;
        }

        // 正常模式：渲染选中的插件
        if self.plugins.is_empty() {
            ui.centered_and_justified(|ui| {
                ui.label("暂无可用插件");
            });
            return;
        }

        if self.selected >= self.plugins.len() {
            self.selected = 0;
        }

        let plugin = &mut self.plugins[self.selected];
        plugin.render(ui);
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        // 关键：每帧无条件请求 100ms 后重绘，确保事件循环不休眠
        // 即使窗口最小化/隐藏，eframe 仍会持续调用 update()
        ctx.request_repaint_after(std::time::Duration::from_millis(100));

        // 1. 首次运行时应用字体大小和主题
        if !self.font_applied {
            self.apply_font_size(ctx);
            // 应用保存的主题
            match self.theme {
                Theme::Light => ctx.set_visuals(egui::Visuals::light()),
                Theme::Dark => ctx.set_visuals(egui::Visuals::dark()),
            }
            self.font_applied = true;
        }

        // 2. 处理全局热键事件
        self.process_hotkey_events(ctx, frame);

        // 3. 处理托盘事件（切换显示/隐藏、退出）
        self.process_tray_events(ctx, frame);

        // 4. 处理窗口关闭事件（用户点击 ✕）→ 取消关闭，改为隐藏到托盘
        if ctx.input(|i| i.viewport().close_requested()) {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.hide_window(frame);
            return;
        }

        // 5. 窗口不可见时跳过渲染，但保持事件循环活跃
        if !self.window_visible {
            // request_repaint_after 通过 eframe 内部的 EventLoopProxy 唤醒 winit，
            // 确保 update() 在窗口隐藏后仍被持续调用
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
            return;
        }

        // 6. 处理窗口内快捷键
        self.handle_shortcuts(ctx);

        // 顶部面板
        egui::TopBottomPanel::top("top_panel").show(ctx, |ui| {
            self.render_top_bar(ui);
        });

        // 底部状态栏
        egui::TopBottomPanel::bottom("status_bar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label(format!(
                    "就绪 | 已注册插件: {} | Ctrl+F 搜索 | Ctrl+Alt+Space/1-9 全局唤出 | Esc 清空",
                    self.plugins.len()
                ));
            });
        });

        // 左侧边栏 - 配置变更时清除旧状态，同时保留边缘拖拽调整能力
        sidebar_panel(self.sidebar_width).show(ctx, |ui| {
            self.render_sidebar(ui);
        });

        // 中央内容区
        egui::CentralPanel::default().show(ctx, |ui| {
            self.render_plugin_content(ui);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::{SIDEBAR_PANEL_ID, apply_sidebar_width, set_native_window_visible, sidebar_panel};
    use windows_sys::Win32::Foundation::HWND;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DestroyWindow, IsIconic, IsWindowVisible, IsZoomed, SW_MAXIMIZE,
        SW_MINIMIZE, ShowWindow, WS_OVERLAPPED,
    };

    struct TestWindow(HWND);

    impl TestWindow {
        fn new() -> Self {
            let class_name: Vec<u16> = "STATIC\0".encode_utf16().collect();
            let window_name: Vec<u16> = "Tools Box 托盘测试\0".encode_utf16().collect();

            // SAFETY: 使用系统内置 STATIC 窗口类，字符串均以空字符结尾且在调用期间有效。
            let hwnd = unsafe {
                CreateWindowExW(
                    0,
                    class_name.as_ptr(),
                    window_name.as_ptr(),
                    WS_OVERLAPPED,
                    0,
                    0,
                    100,
                    100,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null(),
                )
            };
            assert!(!hwnd.is_null(), "测试窗口创建失败");
            Self(hwnd)
        }
    }

    impl Drop for TestWindow {
        fn drop(&mut self) {
            // SAFETY: 句柄由 CreateWindowExW 创建，且仅在此处销毁一次。
            unsafe {
                DestroyWindow(self.0);
            }
        }
    }

    #[test]
    fn sidebar_width_change_clears_persisted_panel_state() {
        let ctx = egui::Context::default();
        let panel_id = egui::Id::new(SIDEBAR_PANEL_ID);
        ctx.data_mut(|data| {
            data.insert_persisted(
                panel_id,
                egui::containers::panel::PanelState {
                    rect: egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(200.0, 600.0)),
                },
            );
        });
        assert!(
            egui::containers::panel::PanelState::load(&ctx, panel_id).is_some(),
            "测试前应存在旧的侧边栏面板状态"
        );

        let mut current_width = 200.0;
        apply_sidebar_width(&ctx, &mut current_width, 320.0);

        assert_eq!(current_width, 320.0);
        assert!(
            egui::containers::panel::PanelState::load(&ctx, panel_id).is_none(),
            "宽度变化后必须清除 egui 保存的旧面板尺寸"
        );
    }

    #[test]
    fn sidebar_width_change_updates_rendered_panel_width() {
        let ctx = egui::Context::default();
        let panel_id = egui::Id::new(SIDEBAR_PANEL_ID);
        ctx.data_mut(|data| {
            data.insert_persisted(
                panel_id,
                egui::containers::panel::PanelState {
                    rect: egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(200.0, 600.0)),
                },
            );
        });

        let mut current_width = 200.0;
        apply_sidebar_width(&ctx, &mut current_width, 320.0);

        let mut rendered_width = 0.0;
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1200.0, 800.0),
            )),
            ..Default::default()
        };
        let _ = ctx.run(input, |ctx| {
            rendered_width = sidebar_panel(current_width)
                .show(ctx, |ui| {
                    ui.set_min_width(ui.available_width());
                })
                .response
                .rect
                .width();
            egui::CentralPanel::default().show(ctx, |_ui| {});
        });

        assert!(
            (rendered_width - 320.0).abs() <= 1.0,
            "侧边栏应按新配置宽度重新布局，实际宽度={rendered_width}"
        );
    }

    #[test]
    fn native_window_can_be_completely_hidden_and_restored() {
        let window = TestWindow::new();

        // SAFETY: 句柄在测试期间有效，并由 TestWindow 保持存活。
        unsafe {
            set_native_window_visible(window.0, true);
            assert_ne!(IsWindowVisible(window.0), 0, "窗口恢复后应可见");

            set_native_window_visible(window.0, false);
            assert_eq!(IsWindowVisible(window.0), 0, "窗口隐藏后应完全不可见");

            set_native_window_visible(window.0, true);
            assert_ne!(IsWindowVisible(window.0), 0, "窗口再次恢复后应可见");
        }
    }

    #[test]
    fn minimized_window_is_restored_from_tray() {
        let window = TestWindow::new();

        // SAFETY: 句柄在测试期间有效，并由 TestWindow 保持存活。
        unsafe {
            set_native_window_visible(window.0, true);
            ShowWindow(window.0, SW_MINIMIZE);
            assert_ne!(IsIconic(window.0), 0, "测试窗口应处于最小化状态");

            set_native_window_visible(window.0, false);
            set_native_window_visible(window.0, true);

            assert_ne!(IsWindowVisible(window.0), 0, "恢复后的窗口应可见");
            assert_eq!(IsIconic(window.0), 0, "恢复后的窗口不应保持最小化");
        }
    }

    #[test]
    fn maximized_window_keeps_state_after_tray_restore() {
        let window = TestWindow::new();

        // SAFETY: 句柄在测试期间有效，并由 TestWindow 保持存活。
        unsafe {
            set_native_window_visible(window.0, true);
            ShowWindow(window.0, SW_MAXIMIZE);
            assert_ne!(IsZoomed(window.0), 0, "测试窗口应处于最大化状态");

            set_native_window_visible(window.0, false);
            set_native_window_visible(window.0, true);

            assert_ne!(IsWindowVisible(window.0), 0, "恢复后的窗口应可见");
            assert_ne!(IsZoomed(window.0), 0, "恢复后的窗口应保持最大化");
        }
    }
}
