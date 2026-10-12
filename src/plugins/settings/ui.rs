//! 设置插件 UI 渲染
//!
//! 提供主题、字体、侧边栏宽度、自定义热键、跟随系统启动的设置界面。

use std::path::Path;

use super::models::AppSettings;
use crate::utils::fonts::{SystemFont, enumerate_system_fonts};
use windows_sys::Win32::Foundation::ERROR_SUCCESS;
use windows_sys::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_READ, KEY_SET_VALUE, REG_SZ, RegCloseKey, RegDeleteValueW,
    RegOpenKeyExW, RegSetValueExW,
};

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
/// 界面字体下拉框 ID。
const UI_FONT_COMBO_ID: &str = "settings_ui_font_combo";
/// 代码字体下拉框 ID。
const CODE_FONT_COMBO_ID: &str = "settings_code_font_combo";
/// 字体下拉框宽度。
const FONT_COMBO_WIDTH: f32 = 260.0;
/// 未选择自定义字体时下拉框显示的文案
const DEFAULT_FONT_LABEL: &str = "默认（内置字体）";

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
    /// 全局字体（界面字体 / 代码字体）发生变化，需要重新装载字体
    pub font_changed: bool,
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
    /// 系统字体列表（首次渲染设置面板时枚举，可手动刷新）
    system_fonts: Vec<SystemFont>,
    /// 系统字体列表是否已枚举
    fonts_loaded: bool,
}

impl SettingsUi {
    pub fn new(settings: AppSettings, plugin_names: Vec<String>) -> Self {
        let sidebar_width_input = format_sidebar_width(settings.sidebar_width);
        Self {
            settings,
            sidebar_width_input,
            dirty: false,
            plugin_names,
            system_fonts: Vec::new(),
            fonts_loaded: false,
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
                let font_size_changed = self.settings.font_size != default.font_size;
                let font_changed = self.settings.ui_font != default.ui_font
                    || self.settings.code_font != default.code_font;
                let width_changed = self.settings.sidebar_width != default.sidebar_width;
                self.sidebar_width_input = format_sidebar_width(default.sidebar_width);
                self.settings = default;
                self.dirty = true;
                change.theme_changed = theme_changed;
                change.font_size_changed = font_size_changed;
                change.font_changed = font_changed;
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

            // 全局字体（界面 + 代码）
            self.render_font_section(ui, change);

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

    /// 渲染全局字体选择区域
    ///
    /// 界面字体可选全部系统字体，代码字体只列**等宽**字体（行号对齐、代码观感都依赖等宽）；
    /// 选中即生效（与字体大小一致），预览行直接显示当前实际渲染效果。
    fn render_font_section(&mut self, ui: &mut egui::Ui, change: &mut SettingsChange) {
        self.ensure_fonts_loaded();

        // 下拉框闭包内同时借用 settings（可写）与 system_fonts（只读），
        // 因此变更只记标志，循环结束后再写 dirty / change
        let mut selection_changed = false;

        ui.horizontal(|ui| {
            ui.label("界面字体：");
            let selected = self
                .settings
                .ui_font
                .clone()
                .unwrap_or_else(|| DEFAULT_FONT_LABEL.to_owned());
            egui::ComboBox::from_id_salt(UI_FONT_COMBO_ID)
                .selected_text(selected)
                .width(FONT_COMBO_WIDTH)
                .show_ui(ui, |ui| {
                    let none_selected = self.settings.ui_font.is_none();
                    if ui
                        .selectable_label(none_selected, DEFAULT_FONT_LABEL)
                        .clicked()
                        && !none_selected
                    {
                        self.settings.ui_font = None;
                        selection_changed = true;
                    }
                    for font in &self.system_fonts {
                        // 已选中项点击不算变更：否则会白白让所有插件的排版缓存失效一次
                        let selected = self.settings.ui_font.as_deref() == Some(font.name.as_str());
                        let label = if font.monospace {
                            format!("{}（等宽）", font.name)
                        } else {
                            font.name.clone()
                        };
                        if ui.selectable_label(selected, label).clicked() && !selected {
                            self.settings.ui_font = Some(font.name.clone());
                            selection_changed = true;
                        }
                    }
                });
        });

        ui.add_space(4.0);

        ui.horizontal(|ui| {
            ui.label("代码字体：");
            let selected = self
                .settings
                .code_font
                .clone()
                .unwrap_or_else(|| DEFAULT_FONT_LABEL.to_owned());
            egui::ComboBox::from_id_salt(CODE_FONT_COMBO_ID)
                .selected_text(selected)
                .width(FONT_COMBO_WIDTH)
                .show_ui(ui, |ui| {
                    let none_selected = self.settings.code_font.is_none();
                    if ui
                        .selectable_label(none_selected, DEFAULT_FONT_LABEL)
                        .clicked()
                        && !none_selected
                    {
                        self.settings.code_font = None;
                        selection_changed = true;
                    }

                    let mut has_monospace = false;
                    for font in &self.system_fonts {
                        if !font.monospace {
                            continue;
                        }
                        has_monospace = true;

                        let selected =
                            self.settings.code_font.as_deref() == Some(font.name.as_str());
                        if ui.selectable_label(selected, &font.name).clicked() && !selected {
                            self.settings.code_font = Some(font.name.clone());
                            selection_changed = true;
                        }
                    }

                    if !has_monospace {
                        ui.weak("未找到等宽字体");
                    }
                });

            if ui.small_button("↻ 刷新列表").clicked() {
                self.reload_fonts();
            }
        });

        if selection_changed {
            self.dirty = true;
            change.font_changed = true;
            log::info!("字体选择已变更，等待应用");
        }

        ui.add_space(4.0);

        // 预览：直接按两个 family 渲染，选中后即时可见
        let font_size = self.settings.font_size;
        ui.label(
            egui::RichText::new("预览（界面）：字体预览 Aa Bb Cc 中文示例 0123")
                .font(egui::FontId::new(font_size, egui::FontFamily::Proportional)),
        );
        ui.label(
            egui::RichText::new("预览（代码）：fn main() { let value = 0; } // 中文 0123")
                .font(egui::FontId::new(font_size, egui::FontFamily::Monospace)),
        );

        if self.system_fonts.is_empty() {
            ui.weak("字体列表为空：可能是读取系统字体注册表失败，见日志");
        } else {
            ui.weak(format!(
                "系统字体 {} 个（其中等宽 {} 个）；新装字体请点「刷新列表」",
                self.system_fonts.len(),
                self.system_fonts
                    .iter()
                    .filter(|font| font.monospace)
                    .count()
            ));
        }
    }

    /// 首次进入设置页时枚举系统字体（避免启动时多花时间）
    fn ensure_fonts_loaded(&mut self) {
        if !self.fonts_loaded {
            self.reload_fonts();
        }
    }

    /// 重新枚举系统字体（新装字体后可点刷新按钮）
    fn reload_fonts(&mut self) {
        self.system_fonts = enumerate_system_fonts();
        self.fonts_loaded = true;
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
    let exe_path = match std::env::current_exe() {
        Ok(path) => path,
        Err(error) => {
            log::error!("获取当前程序路径失败，未修改跟随系统启动设置: {}", error);
            return;
        }
    };
    let exe_str = auto_start_value(&exe_path);

    // SAFETY: 调用 Windows API 操作注册表，子键名、值名与数据均在调用期间保持有效。
    unsafe {
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

/// 构建注册表 `Run` 项的值。
///
/// 路径必须加引号：`Run` 项按命令行解析，含空格的路径（如 `C:\Program Files\...`）
/// 不加引号会被拆成「C:\Program」加参数，导致开机启动静默失败。
fn auto_start_value(exe_path: &Path) -> String {
    format!("\"{}\"", exe_path.display())
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{
        AppSettings, MAX_SIDEBAR_WIDTH, MIN_SIDEBAR_WIDTH, SIDEBAR_WIDTH_INPUT_ID, SettingsChange,
        SettingsUi, auto_start_value, commit_sidebar_width_input, format_sidebar_width,
    };

    #[test]
    fn auto_start_value_quotes_path_with_spaces() {
        let path = Path::new("C:\\Program Files\\Tools-box\\tools-box.exe");

        assert_eq!(
            auto_start_value(path),
            "\"C:\\Program Files\\Tools-box\\tools-box.exe\""
        );
    }

    /// 渲染设置面板一帧：字体区域不应 panic，首次渲染即枚举系统字体，默认不产生字体变更
    #[test]
    fn renders_font_section_and_loads_system_fonts() {
        let ctx = egui::Context::default();
        let mut settings_ui = SettingsUi::new(AppSettings::default(), vec!["工具".to_string()]);

        assert!(!settings_ui.fonts_loaded, "渲染前不应枚举字体");

        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1200.0, 900.0),
            )),
            ..Default::default()
        };
        let mut change = SettingsChange::default();
        let _ = ctx.run(input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                change = settings_ui.render(ui);
            });
        });

        assert!(
            !change.font_changed,
            "仅渲染不应报告字体变更（未点选任何字体）"
        );
        assert!(settings_ui.fonts_loaded, "首次渲染设置面板应枚举系统字体");
        assert!(
            !settings_ui.system_fonts.is_empty(),
            "Windows 环境应至少枚举到一个系统字体"
        );
        assert!(
            settings_ui.system_fonts.iter().any(|font| font.monospace),
            "应至少识别出一个等宽字体（代码字体下拉需要）"
        );
        assert!(
            settings_ui
                .system_fonts
                .iter()
                .all(|font| font.path.is_absolute()),
            "枚举结果必须带绝对路径"
        );
    }

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
