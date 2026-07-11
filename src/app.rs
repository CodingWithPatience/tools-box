use crate::hotkey::HotkeyManager;
use crate::plugin::Plugin;
use crate::plugins;
use crate::storage::Database;
use crate::tray::{TrayEvent, TrayManager};
use egui::FontFamily;
use raw_window_handle::HasWindowHandle;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetWindowLongW, SetForegroundWindow, SetWindowLongW, ShowWindow, GWL_EXSTYLE,
    SW_HIDE, SW_RESTORE, WS_EX_APPWINDOW, WS_EX_TOOLWINDOW,
};

/// 默认字体大小
const DEFAULT_FONT_SIZE: f32 = 14.0;
/// 最小字体大小
const MIN_FONT_SIZE: f32 = 10.0;
/// 最大字体大小
const MAX_FONT_SIZE: f32 = 24.0;
/// 字体大小步长
const FONT_SIZE_STEP: f32 = 1.0;

/// 主题模式
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Theme {
    Light,
    Dark,
}

impl Theme {
    /// 切换主题
    pub fn toggle(&self) -> Self {
        match self {
            Theme::Light => Theme::Dark,
            Theme::Dark => Theme::Light,
        }
    }

    /// 获取主题图标（显示切换后的主题图标，与文字一致）
    pub fn icon(&self) -> &str {
        match self {
            Theme::Light => "🌙",  // 当前亮色，切换到暗色，显示月亮
            Theme::Dark => "☀",   // 当前暗色，切换到亮色，显示太阳
        }
    }

    /// 获取主题名称（显示切换后的主题名称，用于按钮文字）
    pub fn name(&self) -> &str {
        match self {
            Theme::Light => "暗色",  // 当前亮色，点击后切换到暗色
            Theme::Dark => "亮色",   // 当前暗色，点击后切换到亮色
        }
    }

    /// 获取当前主题的实际名称（用于提示信息）
    pub fn current_name(&self) -> &str {
        match self {
            Theme::Light => "亮色",
            Theme::Dark => "暗色",
        }
    }
}

impl Default for Theme {
    fn default() -> Self {
        Theme::Dark
    }
}

/// 主应用状态
pub struct App {
    /// 已注册的插件列表
    plugins: Vec<Box<dyn Plugin>>,
    /// 当前选中的插件索引
    selected: usize,
    /// 侧边栏搜索关键词
    search_query: String,
    /// 数据库连接（预留，后续插件使用）
    _db: Database,
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
    /// 系统托盘管理器（保持存活以维持托盘图标）
    tray_manager: TrayManager,
    /// 全局热键管理器
    hotkey_manager: HotkeyManager,
    /// 最近使用的插件索引（用于全局热键唤出）
    last_active_tool: usize,
    /// 窗口是否可见
    window_visible: bool,
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
            fonts
                .font_data
                .insert("chinese".to_owned(), egui::FontData::from_owned(font_data).into());

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
        r"C:\Windows\Fonts\seguiemj.ttf",  // Segoe UI Emoji
        r"C:\Windows\Fonts\seguisym.ttf",  // Segoe UI Symbol
    ];

    let mut emoji_loaded = false;
    for path in &emoji_font_paths {
        if let Ok(font_data) = std::fs::read(path) {
            fonts
                .font_data
                .insert("emoji".to_owned(), egui::FontData::from_owned(font_data).into());

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
        let mut plugins = plugins::register_all_plugins();

        for plugin in plugins.iter_mut() {
            plugin.init();
        }

        Self {
            plugins,
            selected: 0,
            search_query: String::new(),
            _db: db,
            status_message: "就绪".to_string(),
            theme: Theme::default(),
            focus_search: false,
            font_size: DEFAULT_FONT_SIZE,
            font_applied: false,
            sidebar_width: 200.0,
            tray_manager,
            hotkey_manager,
            last_active_tool: 0,
            window_visible: true,
        }
    }

    /// 处理全局热键事件
    fn process_hotkey_events(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        for event in self.hotkey_manager.poll_events() {
            if event.plugin_index == usize::MAX {
                // Ctrl+Alt+Space：恢复最近使用的工具
                log::info!("[热键] 唤出最近工具，索引={}", self.last_active_tool);
                self.selected = self.last_active_tool;
                if !self.window_visible {
                    self.show_window(ctx, frame);
                }
                self.status_message = format!(
                    "热键唤出：{}", self.plugins[self.last_active_tool].name()
                );
            } else if event.plugin_index < self.plugins.len() {
                // Ctrl+Alt+数字：跳转到指定插件
                log::info!("[热键] 唤出插件索引={}", event.plugin_index);
                self.selected = event.plugin_index;
                self.last_active_tool = event.plugin_index;
                if !self.window_visible {
                    self.show_window(ctx, frame);
                }
                self.status_message = format!(
                    "热键唤出：{}", self.plugins[event.plugin_index].name()
                );
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
                        self.hide_window(ctx, frame);
                    } else {
                        self.show_window(ctx, frame);
                    }
                }
                TrayEvent::Quit => {
                    log::info!("[托盘事件] 退出程序");
                    std::process::exit(0);
                }
            }
        }
    }

    /// 隐藏窗口到系统托盘
    ///
    /// 使用 WS_EX_TOOLWINDOW + Minimized 方案：
    /// 1. 设置 WS_EX_TOOLWINDOW 移除任务栏按钮
    /// 2. 最小化窗口（保持事件循环活跃）
    fn hide_window(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        self.window_visible = false;
        self.status_message = "已最小化到系统托盘".to_string();

        if let Ok(handle) = frame.window_handle() {
            if let raw_window_handle::RawWindowHandle::Win32(h) = handle.as_raw() {
                let hwnd = h.hwnd.get() as _;
                // SAFETY: hwnd 是有效的 Win32 窗口句柄
                unsafe {
                    // 设置 WS_EX_TOOLWINDOW，移除任务栏按钮
                    let style = GetWindowLongW(hwnd, GWL_EXSTYLE);
                    let new_style = (style & !(WS_EX_APPWINDOW as i32)) | (WS_EX_TOOLWINDOW as i32);
                    SetWindowLongW(hwnd, GWL_EXSTYLE, new_style);
                }
                log::info!("已设置 WS_EX_TOOLWINDOW");
            }
        }

        // 最小化（保持事件循环活跃，与 request_repaint_after 配合）
        ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
        log::info!("窗口已最小化到托盘");
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
                    // 恢复 WS_EX_APPWINDOW，显示任务栏按钮
                    let style = GetWindowLongW(hwnd, GWL_EXSTYLE);
                    let new_style = (style & !(WS_EX_TOOLWINDOW as i32)) | (WS_EX_APPWINDOW as i32);
                    SetWindowLongW(hwnd, GWL_EXSTYLE, new_style);

                    // 恢复窗口
                    ShowWindow(hwnd, SW_RESTORE);
                    SetForegroundWindow(hwnd);
                }
                log::info!("窗口已恢复（WS_EX_APPWINDOW + SW_RESTORE）");
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
                // 主题切换按钮
                let theme_btn = ui.button(format!("{} {}", self.theme.icon(), self.theme.name()));
                if theme_btn.clicked() {
                    self.toggle_theme(ui.ctx());
                }
                theme_btn.on_hover_text("切换主题");

                ui.separator();

                // 字体大小调节
                ui.horizontal(|ui| {
                    if ui.small_button("A-").clicked() && self.font_size > MIN_FONT_SIZE {
                        self.font_size -= FONT_SIZE_STEP;
                        self.apply_font_size(ui.ctx());
                    }

                    ui.label(format!("字体: {:.0}", self.font_size));

                    if ui.small_button("A+").clicked() && self.font_size < MAX_FONT_SIZE {
                        self.font_size += FONT_SIZE_STEP;
                        self.apply_font_size(ui.ctx());
                    }
                });

                ui.separator();

                // 状态消息
                ui.label(&self.status_message);
            });
        });
        ui.separator();
    }

    /// 应用字体大小设置
    fn apply_font_size(&self, ctx: &egui::Context) {
        let mut style = (*ctx.style()).clone();
        style.text_styles = [
            (egui::TextStyle::Body, egui::FontId::new(self.font_size, FontFamily::Proportional)),
            (egui::TextStyle::Button, egui::FontId::new(self.font_size, FontFamily::Proportional)),
            (egui::TextStyle::Small, egui::FontId::new(self.font_size - 2.0, FontFamily::Proportional)),
            (egui::TextStyle::Heading, egui::FontId::new(self.font_size + 4.0, FontFamily::Proportional)),
            (egui::TextStyle::Monospace, egui::FontId::new(self.font_size, FontFamily::Monospace)),
        ]
        .into();
        ctx.set_style(style);
        log::info!("字体大小已设置为: {}", self.font_size);
    }

    /// 切换主题
    fn toggle_theme(&mut self, ctx: &egui::Context) {
        self.theme = self.theme.toggle();
        match self.theme {
            Theme::Light => ctx.set_visuals(egui::Visuals::light()),
            Theme::Dark => ctx.set_visuals(egui::Visuals::dark()),
        }
        self.status_message = format!("已切换到{}主题", self.theme.current_name());
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
            let button_space = if !self.search_query.is_empty() { 30.0 } else { 0.0 };
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
                ui.weak(format!(
                    "找到 {} 个插件",
                    filtered.len()
                ));
            });
            ui.add_space(4.0);
        }

        // 插件列表
        egui::ScrollArea::vertical()
            .id_salt("sidebar_plugin_list")
            .show(ui, |ui| {
                for (list_idx, &idx) in filtered.iter().enumerate() {
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
        if self.plugins.is_empty() {
            ui.centered_and_justified(|ui| {
                ui.label("暂无可用插件");
            });
            return;
        }

        // 确保 selected 索引有效
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
            self.font_applied = true;
        }

        // 2. 处理全局热键事件
        self.process_hotkey_events(ctx, frame);

        // 3. 处理托盘事件（切换显示/隐藏、退出）
        self.process_tray_events(ctx, frame);

        // 4. 处理窗口关闭事件（用户点击 ✕）→ 取消关闭，改为隐藏到托盘
        if ctx.input(|i| i.viewport().close_requested()) {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.hide_window(ctx, frame);
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

        // 左侧边栏 - 使用 egui 内置的可调整宽度
        egui::SidePanel::left("sidebar")
            .resizable(true)
            .default_width(self.sidebar_width)
            .width_range(150.0..=400.0)
            .show(ctx, |ui| {
                self.render_sidebar(ui);
            });

        // 中央内容区
        egui::CentralPanel::default().show(ctx, |ui| {
            self.render_plugin_content(ui);
        });
    }
}
