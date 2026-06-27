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

    /// 根据文件扩展名获取语法定义
    pub fn get_syntax_for_extension(&self, extension: &str) -> Option<&syntect::parsing::SyntaxReference> {
        self.syntax_set.find_syntax_by_extension(extension)
    }

    /// 根据文件名获取语法定义
    pub fn get_syntax_for_file(&self, filename: &str) -> Option<&syntect::parsing::SyntaxReference> {
        self.syntax_set.find_syntax_by_extension(filename)
            .or_else(|| self.syntax_set.find_syntax_by_first_line(filename))
    }

    /// 高亮代码并返回 LayoutJob
    pub fn highlight_to_layout_job(
        &self,
        code: &str,
        syntax_name: Option<&str>,
        font_size: f32,
        is_dark_mode: bool,
    ) -> LayoutJob {
        let syntax = syntax_name
            .and_then(|name| self.find_syntax_by_name_fuzzy(name))
            .unwrap_or_else(|| self.syntax_set.find_syntax_plain_text());

        let theme_name = if is_dark_mode {
            "base16-ocean.dark"
        } else {
            "base16-ocean.light"
        };
        let theme = &self.theme_set.themes[theme_name];

        let mut highlighter = HighlightLines::new(syntax, theme);
        let mut job = LayoutJob::default();

        for line in LinesWithEndings::from(code) {
            let ranges = highlighter.highlight_line(line, &self.syntax_set)
                .unwrap_or_else(|e| {
                    log::warn!("语法高亮失败，使用默认样式回退: {}", e);
                    vec![(Style::default(), line)]
                });

            for (style, text) in ranges {
                let fg_color = Color32::from_rgb(
                    style.foreground.r,
                    style.foreground.g,
                    style.foreground.b,
                );

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
        font_size: f32,
        is_dark_mode: bool,
    ) -> Vec<(Color32, String)> {
        let syntax = syntax_name
            .and_then(|name| self.find_syntax_by_name_fuzzy(name))
            .unwrap_or_else(|| self.syntax_set.find_syntax_plain_text());

        let theme_name = if is_dark_mode {
            "base16-ocean.dark"
        } else {
            "base16-ocean.light"
        };
        let theme = &self.theme_set.themes[theme_name];

        let mut highlighter = HighlightLines::new(syntax, theme);
        let mut result = Vec::new();

        let ranges = highlighter.highlight_line(line, &self.syntax_set)
            .unwrap_or_else(|e| {
                log::warn!("单行语法高亮失败，使用默认样式回退: {}", e);
                vec![(Style::default(), line)]
            });

        for (style, text) in ranges {
            let fg_color = Color32::from_rgb(
                style.foreground.r,
                style.foreground.g,
                style.foreground.b,
            );
            result.push((fg_color, text.to_string()));
        }

        result
    }

    /// 获取支持的语言列表
    pub fn get_supported_languages(&self) -> Vec<String> {
        self.syntax_set
            .syntaxes()
            .iter()
            .map(|s| s.name.clone())
            .collect()
    }

    /// 根据语言名称获取语法定义（模糊匹配）
    pub fn find_syntax_by_name_fuzzy(&self, name: &str) -> Option<&syntect::parsing::SyntaxReference> {
        // 先尝试精确匹配
        if let Some(syntax) = self.syntax_set.find_syntax_by_name(name) {
            return Some(syntax);
        }

        // 尝试不区分大小写匹配
        let name_lower = name.to_lowercase();
        self.syntax_set.syntaxes().iter().find(|s| {
            s.name.to_lowercase() == name_lower
            || s.file_extensions.iter().any(|ext| ext.to_lowercase() == name_lower)
        })
    }

    /// 根据语言名称获取语法名称
    pub fn get_syntax_name_for_extension(&self, extension: &str) -> Option<String> {
        self.syntax_set
            .find_syntax_by_extension(extension)
            .map(|s| s.name.clone())
    }
}