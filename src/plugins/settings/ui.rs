//! 设置插件 UI 渲染
//!
//! 提供主题、字体、侧边栏宽度、自定义热键、跟随系统启动的设置界面。

use super::models::AppSettings;

/// 字体大小范围
const MIN_FONT_SIZE: f32 = 10.0;
const MAX_FONT_SIZE: f32 = 24.0;
const FONT_SIZE_STEP: f32 = 1.0;
/// 侧边栏最小宽度。
const MIN_SIDEBAR_WIDTH: u16 = 150;
/// 侧边栏最大宽度。
const MAX_SIDEBAR_WIDTH: u16 = 400;
/// 侧边栏宽度输入框 ID。
const SIDEBAR_WIDTH_INPUT_ID: &str = "settings_sidebar_width_input";

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
    /// 侧边栏宽度输入内容，失焦前不写入设置。
    sidebar_width_input: String,
    /// 是否有未保存的更改
    dirty: bool,
    /// 插件名称列表（用于热键配置显示）
    plugin_names: Vec<String>,
}

impl SettingsUi {
    pub fn new(settings: AppSettings, plugin_names: Vec<String>) -> Self {
        let sidebar_width_input = format_sidebar_width(settings.sidebar_width);
        Self {
            settings,
            sidebar_width_input,
            dirty: false,
            plugin_names,
        }
    }

    /// 获取当前设置
    pub fn settings(&self) -> &AppSettings {
        &self.settings
    }

    /// 标记为已保存
    pub fn mark_saved(&mut self) {
        self.dirty = false;
    }

    /// 提交尚未确认的侧边栏宽度输入。
    pub(super) fn commit_pending_sidebar_width(&mut self) -> bool {
        let changed = commit_sidebar_width_input(&mut self.settings, &mut self.sidebar_width_input);
        if changed {
            self.dirty = true;
        }
        changed
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
                self.sidebar_width_input = format_sidebar_width(default.sidebar_width);
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
                let response = ui.add(
                    egui::TextEdit::singleline(&mut self.sidebar_width_input)
                        .id(egui::Id::new(SIDEBAR_WIDTH_INPUT_ID))
                        .desired_width(64.0)
                        .char_limit(3)
                        .hint_text("150-400"),
                );
                ui.label("px（150-400）");

                if response.lost_focus() && self.commit_pending_sidebar_width() {
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

fn format_sidebar_width(width: f32) -> String {
    format!("{width:.0}")
}

fn commit_sidebar_width_input(settings: &mut AppSettings, input: &mut String) -> bool {
    let parsed_width = input.parse::<u16>().ok();
    let Some(parsed_width) = parsed_width else {
        *input = format_sidebar_width(settings.sidebar_width);
        return false;
    };

    let normalized_width = parsed_width.clamp(MIN_SIDEBAR_WIDTH, MAX_SIDEBAR_WIDTH);
    let normalized_width = f32::from(normalized_width);
    *input = format_sidebar_width(normalized_width);

    if (settings.sidebar_width - normalized_width).abs() <= f32::EPSILON {
        return false;
    }

    settings.sidebar_width = normalized_width;
    true
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

#[cfg(test)]
mod tests {
    use super::{
        AppSettings, MAX_SIDEBAR_WIDTH, MIN_SIDEBAR_WIDTH, SIDEBAR_WIDTH_INPUT_ID, SettingsChange,
        SettingsUi, commit_sidebar_width_input, format_sidebar_width,
    };

    fn render_settings_frame(
        ctx: &egui::Context,
        settings_ui: &mut SettingsUi,
        events: Vec<egui::Event>,
    ) -> SettingsChange {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(800.0, 600.0),
            )),
            events,
            ..Default::default()
        };
        let mut change = SettingsChange::default();
        let _ = ctx.run(input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                change = settings_ui.render(ui);
            });
        });
        change
    }

    #[test]
    fn sidebar_width_text_edit_commits_only_after_enter() {
        let ctx = egui::Context::default();
        let mut settings_ui = SettingsUi::new(AppSettings::default(), Vec::new());
        settings_ui.sidebar_width_input.clear();
        ctx.memory_mut(|memory| {
            memory.request_focus(egui::Id::new(SIDEBAR_WIDTH_INPUT_ID));
        });

        let editing_change = render_settings_frame(
            &ctx,
            &mut settings_ui,
            vec![egui::Event::Text("320".to_string())],
        );

        assert_eq!(settings_ui.sidebar_width_input, "320");
        assert_eq!(settings_ui.settings.sidebar_width, 200.0);
        assert!(!editing_change.sidebar_width_changed);

        let commit_change = render_settings_frame(
            &ctx,
            &mut settings_ui,
            vec![egui::Event::Key {
                key: egui::Key::Enter,
                physical_key: Some(egui::Key::Enter),
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );

        assert_eq!(settings_ui.settings.sidebar_width, 320.0);
        assert!(commit_change.sidebar_width_changed);
    }

    #[test]
    fn sidebar_width_input_does_not_change_setting_before_commit() {
        let settings = AppSettings::default();
        let mut ui = SettingsUi::new(settings, Vec::new());

        ui.sidebar_width_input = "320".to_string();

        assert_eq!(ui.settings.sidebar_width, 200.0);
        assert_eq!(ui.sidebar_width_input, "320");
        assert!(ui.commit_pending_sidebar_width());
        assert_eq!(ui.settings.sidebar_width, 320.0);
        assert!(ui.dirty);
    }

    #[test]
    fn sidebar_width_input_commits_valid_value() {
        let mut settings = AppSettings::default();
        let mut input = "320".to_string();

        assert!(commit_sidebar_width_input(&mut settings, &mut input));
        assert_eq!(settings.sidebar_width, 320.0);
        assert_eq!(input, "320");
    }

    #[test]
    fn sidebar_width_input_is_limited_to_maximum() {
        let mut settings = AppSettings::default();
        let mut input = "999".to_string();

        assert!(commit_sidebar_width_input(&mut settings, &mut input));
        assert_eq!(settings.sidebar_width, f32::from(MAX_SIDEBAR_WIDTH));
        assert_eq!(input, "400");
    }

    #[test]
    fn sidebar_width_input_is_limited_to_minimum() {
        let mut settings = AppSettings::default();
        let mut input = "99".to_string();

        assert!(commit_sidebar_width_input(&mut settings, &mut input));
        assert_eq!(settings.sidebar_width, f32::from(MIN_SIDEBAR_WIDTH));
        assert_eq!(input, "150");
    }

    #[test]
    fn invalid_sidebar_width_input_restores_current_value() {
        let mut settings = AppSettings::default();
        settings.sidebar_width = 280.0;
        let mut input = "abc".to_string();

        assert!(!commit_sidebar_width_input(&mut settings, &mut input));
        assert_eq!(settings.sidebar_width, 280.0);
        assert_eq!(input, format_sidebar_width(settings.sidebar_width));
    }

    #[test]
    fn empty_sidebar_width_input_restores_current_value() {
        let mut settings = AppSettings::default();
        settings.sidebar_width = 280.0;
        let mut input = String::new();

        assert!(!commit_sidebar_width_input(&mut settings, &mut input));
        assert_eq!(settings.sidebar_width, 280.0);
        assert_eq!(input, format_sidebar_width(settings.sidebar_width));
    }
}
