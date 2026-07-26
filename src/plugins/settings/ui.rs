//! 设置插件 UI 渲染
//!
//! 提供主题、字体、侧边栏宽度、自定义热键、跟随系统启动的设置界面。

use super::models::AppSettings;

/// 字体大小范围
const MIN_FONT_SIZE: f32 = 10.0;
const MAX_FONT_SIZE: f32 = 24.0;
const FONT_SIZE_STEP: f32 = 1.0;

/// 可用于热键的字符列表（大写字母 + 数字）
const HOTKEY_CHARS: &[char] = &[
    '1', '2', '3', '4', '5', '6', '7', '8', '9', '0', 'A', 'B', 'C', 'D', 'E', 'F', 'G', 'H', 'I',
    'J', 'K', 'L', 'M', 'N', 'O', 'P', 'Q', 'R', 'S', 'T', 'U', 'V', 'W', 'X', 'Y', 'Z',
];

/// 设置变更类型
#[derive(Debug, Default)]
pub struct SettingsChange {
    pub theme_changed: bool,
    pub font_size_changed: bool,
    pub sidebar_width_changed: bool,
    pub hotkeys_changed: bool,
    pub save_requested: bool,
    pub reset_requested: bool,
}

/// 设置面板 UI
pub struct SettingsUi {
    /// 当前设置（编辑中的副本）
    settings: AppSettings,
    /// 是否有未保存的更改
    dirty: bool,
    /// 插件名称列表（用于热键配置显示）
    plugin_names: Vec<String>,
}

impl SettingsUi {
    pub fn new(settings: AppSettings, plugin_names: Vec<String>) -> Self {
        Self {
            settings,
            dirty: false,
            plugin_names,
        }
    }

    /// 获取当前设置
    pub fn settings(&self) -> &AppSettings {
        &self.settings
    }

    /// 从外部更新设置
    pub fn update_settings(&mut self, settings: AppSettings) {
        self.settings = settings;
        self.dirty = false;
    }

    /// 标记为已保存
    pub fn mark_saved(&mut self) {
        self.dirty = false;
    }

    /// 渲染设置面板
    pub fn render(&mut self, ui: &mut egui::Ui) -> SettingsChange {
        let mut change = SettingsChange::default();

        ui.heading("⚙ 设置");
        ui.add_space(16.0);

        // ── 外观设置 ──
        self.render_appearance_section(ui, &mut change);

        ui.add_space(16.0);

        // ── 全局快捷键设置 ──
        self.render_hotkey_section(ui, &mut change);

        ui.add_space(16.0);

        // ── 系统设置 ──
        self.render_system_section(ui, &mut change);

        ui.add_space(16.0);

        // ── 操作按钮 ──
        ui.horizontal(|ui| {
            if ui.button("💾 保存设置").clicked() {
                self.dirty = false;
                change.save_requested = true;
            }

            if ui.button("🔄 恢复默认").clicked() {
                let default = AppSettings::default();
                let theme_changed = self.settings.theme != default.theme;
                let font_changed = self.settings.font_size != default.font_size;
                let width_changed = self.settings.sidebar_width != default.sidebar_width;
                self.settings = default;
                self.dirty = true;
                change.theme_changed = theme_changed;
                change.font_size_changed = font_changed;
                change.sidebar_width_changed = width_changed;
                change.hotkeys_changed = true;
                change.reset_requested = true;
            }
        });

        // 未保存提示
        if self.dirty {
            ui.add_space(4.0);
            ui.colored_label(
                egui::Color32::from_rgb(255, 200, 0),
                "⚠ 有未保存的更改，请点击保存",
            );
        }

        change
    }

    /// 渲染外观设置区域
    fn render_appearance_section(&mut self, ui: &mut egui::Ui, change: &mut SettingsChange) {
        ui.group(|ui| {
            ui.label("🎨 外观设置");
            ui.add_space(8.0);

            // 主题切换
            ui.horizontal(|ui| {
                ui.label("主题：");
                let is_dark = self.settings.theme == "dark";
                if ui.selectable_label(is_dark, "🌙 暗色").clicked() && !is_dark {
                    self.settings.theme = "dark".to_string();
                    self.dirty = true;
                    change.theme_changed = true;
                }
                if ui.selectable_label(!is_dark, "☀ 亮色").clicked() && is_dark {
                    self.settings.theme = "light".to_string();
                    self.dirty = true;
                    change.theme_changed = true;
                }
            });

            ui.add_space(4.0);

            // 字体大小
            ui.horizontal(|ui| {
                ui.label("字体大小：");
                if ui.small_button("A-").clicked() && self.settings.font_size > MIN_FONT_SIZE {
                    self.settings.font_size -= FONT_SIZE_STEP;
                    self.dirty = true;
                    change.font_size_changed = true;
                }
                ui.label(format!("{:.0}", self.settings.font_size));
                if ui.small_button("A+").clicked() && self.settings.font_size < MAX_FONT_SIZE {
                    self.settings.font_size += FONT_SIZE_STEP;
                    self.dirty = true;
                    change.font_size_changed = true;
                }
            });

            ui.add_space(4.0);

            // 侧边栏宽度
            ui.horizontal(|ui| {
                ui.label("侧边栏宽度：");
                let old_width = self.settings.sidebar_width;
                ui.add(
                    egui::Slider::new(&mut self.settings.sidebar_width, 150.0..=400.0)
                        .suffix(" px")
                        .step_by(10.0),
                );
                if (self.settings.sidebar_width - old_width).abs() > f32::EPSILON {
                    self.dirty = true;
                    change.sidebar_width_changed = true;
                }
            });
        });
    }

    /// 渲染快捷键设置区域
    fn render_hotkey_section(&mut self, ui: &mut egui::Ui, change: &mut SettingsChange) {
        ui.group(|ui| {
            ui.label("⌨ 全局快捷键");
            ui.add_space(8.0);

            ui.horizontal(|ui| {
                ui.strong("唤出工具集：");
                ui.label("Ctrl+Alt+Space（固定，不可修改）");
            });

            ui.add_space(4.0);

            // 各工具的自定义热键
            for (i, name) in self.plugin_names.iter().enumerate() {
                ui.horizontal(|ui| {
                    ui.label(format!("{}：", name));

                    // 获取当前热键字符（显示为大写）
                    let current_char = self.settings.tool_hotkeys.get(i).copied().unwrap_or(' ');
                    let display_char = current_char.to_ascii_uppercase();

                    // 热键下拉选择
                    egui::ComboBox::from_id_salt(format!("hotkey_{}", i))
                        .selected_text(format!("Ctrl+Alt+{}", display_char))
                        .show_ui(ui, |ui| {
                            for &c in HOTKEY_CHARS {
                                let label = format!("Ctrl+Alt+{}", c);
                                // 检查是否已被其他工具使用
                                let used_by_other = self
                                    .settings
                                    .tool_hotkeys
                                    .iter()
                                    .enumerate()
                                    .any(|(j, &hc)| j != i && hc == c);
                                let is_current = current_char == c;

                                let enabled = !used_by_other || is_current;
                                let response = ui.add_enabled(
                                    enabled,
                                    egui::SelectableLabel::new(is_current, &label),
                                );
                                if response.clicked() && !is_current {
                                    if i < self.settings.tool_hotkeys.len() {
                                        self.settings.tool_hotkeys[i] = c;
                                    } else {
                                        self.settings.tool_hotkeys.push(c);
                                    }
                                    self.dirty = true;
                                    change.hotkeys_changed = true;
                                }
                            }
                        });
                });
            }
        });
    }

    /// 渲染系统设置区域
    fn render_system_section(&mut self, ui: &mut egui::Ui, _change: &mut SettingsChange) {
        ui.group(|ui| {
            ui.label("💻 系统设置");
            ui.add_space(8.0);

            ui.horizontal(|ui| {
                let mut auto_start = self.settings.auto_start;
                if ui.checkbox(&mut auto_start, "跟随系统启动").changed() {
                    self.settings.auto_start = auto_start;
                    self.dirty = true;
                    // 应用跟随系统启动设置
                    apply_auto_start(auto_start);
                }
            });
        });
    }
}

/// 应用跟随系统启动设置（通过 Windows 注册表）
fn apply_auto_start(enable: bool) {
    let exe_path = std::env::current_exe().unwrap_or_default();
    let exe_str = exe_path.to_string_lossy().to_string();

    // SAFETY: 调用 Windows API 操作注册表
    unsafe {
        use windows_sys::Win32::Foundation::*;
        use windows_sys::Win32::System::Registry::*;

        let key_path: Vec<u16> = "Software\\Microsoft\\Windows\\CurrentVersion\\Run\0"
            .encode_utf16()
            .collect();
        let value_name: Vec<u16> = "ToolsBox\0".encode_utf16().collect();

        let mut hkey: HKEY = std::ptr::null_mut();
        let result = RegOpenKeyExW(
            HKEY_CURRENT_USER,
            key_path.as_ptr(),
            0,
            KEY_SET_VALUE | KEY_READ,
            &mut hkey,
        );

        if result == ERROR_SUCCESS {
            if enable {
                let value_data: Vec<u16> =
                    exe_str.encode_utf16().chain(std::iter::once(0)).collect();
                RegSetValueExW(
                    hkey,
                    value_name.as_ptr(),
                    0,
                    REG_SZ,
                    value_data.as_ptr() as *const u8,
                    (value_data.len() * 2) as u32,
                );
                log::info!("已设置跟随系统启动: {}", exe_str);
            } else {
                RegDeleteValueW(hkey, value_name.as_ptr());
                log::info!("已取消跟随系统启动");
            }
            RegCloseKey(hkey);
        } else {
            log::warn!("无法打开注册表键: {}", result);
        }
    }
}
