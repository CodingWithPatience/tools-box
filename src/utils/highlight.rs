use egui::text::LayoutJob;
use egui::{Color32, FontId, TextFormat};
use syntect::easy::HighlightLines;
use syntect::highlighting::{Style, ThemeSet};
use syntect::parsing::SyntaxSet;
use syntect::util::LinesWithEndings;

/// 语法高亮器
pub struct SyntaxHighlighter {
    syntax_set: SyntaxSet,
    theme_set: ThemeSet,
}

impl SyntaxHighlighter {
    /// 创建新的语法高亮器
    pub fn new() -> Self {
        Self {
            syntax_set: SyntaxSet::load_defaults_newlines(),
            theme_set: ThemeSet::load_defaults(),
        }
    }

    /// 根据亮/暗模式获取主题，若指定主题不存在则回退到第一个可用主题
    fn resolve_theme(&self, is_dark_mode: bool) -> Option<&syntect::highlighting::Theme> {
        let theme_name = if is_dark_mode {
            "base16-ocean.dark"
        } else {
            "base16-ocean.light"
        };
        self.theme_set.themes.get(theme_name).or_else(|| {
            log::warn!("主题 '{}' 未找到，回退到首个可用主题", theme_name);
            self.theme_set.themes.values().next()
        })
    }

    /// 高亮代码并返回 LayoutJob
    pub fn highlight_to_layout_job(
        &self,
        code: &str,
        syntax_name: Option<&str>,
        font_size: f32,
        is_dark_mode: bool,
    ) -> LayoutJob {
        if code.is_empty() {
            return LayoutJob::default();
        }

        let syntax = syntax_name
            .and_then(|name| self.find_syntax_by_name_fuzzy(name))
            .unwrap_or_else(|| self.syntax_set.find_syntax_plain_text());

        let Some(theme) = self.resolve_theme(is_dark_mode) else {
            log::error!("无可用语法高亮主题，回退为纯文本");
            let mut job = LayoutJob::default();
            job.append(
                code,
                0.0,
                TextFormat {
                    font_id: FontId::monospace(font_size),
                    ..Default::default()
                },
            );
            return job;
        };

        let mut highlighter = HighlightLines::new(syntax, theme);
        let mut job = LayoutJob::default();

        for line in LinesWithEndings::from(code) {
            let ranges = highlighter
                .highlight_line(line, &self.syntax_set)
                .unwrap_or_else(|e| {
                    log::warn!("语法高亮失败，使用默认样式回退: {}", e);
                    vec![(Style::default(), line)]
                });

            for (style, text) in ranges {
                let fg_color =
                    Color32::from_rgb(style.foreground.r, style.foreground.g, style.foreground.b);

                let format = TextFormat {
                    font_id: FontId::monospace(font_size),
                    color: fg_color,
                    ..Default::default()
                };

                job.append(text, 0.0, format);
            }
        }

        job
    }

    /// 高亮单行代码
    pub fn highlight_line(
        &self,
        line: &str,
        syntax_name: Option<&str>,
        _font_size: f32,
        is_dark_mode: bool,
    ) -> Vec<(Color32, String)> {
        let syntax = syntax_name
            .and_then(|name| self.find_syntax_by_name_fuzzy(name))
            .unwrap_or_else(|| self.syntax_set.find_syntax_plain_text());

        let Some(theme) = self.resolve_theme(is_dark_mode) else {
            log::error!("无可用语法高亮主题，回退为纯文本");
            let fg = Color32::from_rgb(0xd0, 0xd0, 0xd0);
            return vec![(fg, line.to_string())];
        };

        let mut highlighter = HighlightLines::new(syntax, theme);
        let mut result = Vec::new();

        let ranges = highlighter
            .highlight_line(line, &self.syntax_set)
            .unwrap_or_else(|e| {
                log::warn!("单行语法高亮失败，使用默认样式回退: {}", e);
                vec![(Style::default(), line)]
            });

        for (style, text) in ranges {
            let fg_color =
                Color32::from_rgb(style.foreground.r, style.foreground.g, style.foreground.b);
            result.push((fg_color, text.to_string()));
        }

        result
    }

    /// 根据语言名称获取语法定义（模糊匹配）
    pub fn find_syntax_by_name_fuzzy(
        &self,
        name: &str,
    ) -> Option<&syntect::parsing::SyntaxReference> {
        if let Some(syntax) = self.syntax_set.find_syntax_by_name(name) {
            return Some(syntax);
        }

        let name_lower = name.to_lowercase();
        self.syntax_set.syntaxes().iter().find(|s| {
            s.name.to_lowercase() == name_lower
                || s.file_extensions
                    .iter()
                    .any(|ext| ext.to_lowercase() == name_lower)
        })
    }

    /// 根据语言名称获取语法名称
    pub fn get_syntax_name_for_extension(&self, extension: &str) -> Option<String> {
        self.syntax_set
            .find_syntax_by_extension(extension)
            .map(|s| s.name.clone())
    }
}
