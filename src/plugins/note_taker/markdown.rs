use egui::{Color32, RichText, Ui};
use pulldown_cmark::{Options, Parser, Event, Tag, TagEnd};

/// Markdown 渲染器
///
/// 使用 pulldown-cmark 解析 Markdown，用 egui RichText 渲染
pub struct MarkdownRenderer {
    /// 是否在代码块中
    in_code_block: bool,
    /// 当前代码块内容
    code_buffer: String,
}

impl MarkdownRenderer {
    /// 创建新的渲染器实例
    pub fn new() -> Self {
        Self {
            in_code_block: false,
            code_buffer: String::new(),
        }
    }

    /// 渲染 Markdown 内容到 egui UI
    pub fn render(&mut self, ui: &mut Ui, markdown: &str) {
        let options = Options::ENABLE_STRIKETHROUGH
            | Options::ENABLE_TABLES
            | Options::ENABLE_TASKLISTS;

        let parser = Parser::new_ext(markdown, options);
        let mut current_text = String::new();
        let mut heading_level = 0;
        let mut list_depth = 0;
        let mut in_blockquote = false;
        let mut in_emphasis = false;
        let mut in_strong = false;

        for event in parser {
            match event {
                Event::Start(tag) => {
                    // 先输出之前累积的文本
                    if !current_text.is_empty() {
                        self.render_text(ui, &current_text, in_emphasis, in_strong, in_blockquote);
                        current_text.clear();
                    }

                    match tag {
                        Tag::Heading { level, .. } => {
                            heading_level = level as u32;
                        }
                        Tag::List(_) => {
                            list_depth += 1;
                        }
                        Tag::Item => {
                            let indent = "  ".repeat(list_depth - 1);
                            current_text.push_str(&format!("{}• ", indent));
                        }
                        Tag::BlockQuote(_) => {
                            in_blockquote = true;
                        }
                        Tag::Emphasis => {
                            in_emphasis = true;
                        }
                        Tag::Strong => {
                            in_strong = true;
                        }
                        Tag::CodeBlock(_) => {
                            self.in_code_block = true;
                            ui.separator();
                            ui.label(RichText::new("代码块:").strong().small());
                        }
                        Tag::Paragraph => {}
                        _ => {}
                    }
                }
                Event::End(tag_end) => {
                    match tag_end {
                        TagEnd::Heading(_) => {
                            if !current_text.is_empty() {
                                self.render_heading(ui, &current_text, heading_level);
                                current_text.clear();
                            }
                            heading_level = 0;
                        }
                        TagEnd::List(_) => {
                            list_depth -= 1;
                            if list_depth == 0 {
                                ui.add_space(4.0);
                            }
                        }
                        TagEnd::Item => {
                            if !current_text.is_empty() {
                                self.render_text(ui, &current_text, false, false, false);
                                current_text.clear();
                            }
                        }
                        TagEnd::BlockQuote(_) => {
                            in_blockquote = false;
                        }
                        TagEnd::Emphasis => {
                            in_emphasis = false;
                        }
                        TagEnd::Strong => {
                            in_strong = false;
                        }
                        TagEnd::CodeBlock => {
                            self.in_code_block = false;
                            if !self.code_buffer.is_empty() {
                                ui.add(
                                    egui::TextEdit::multiline(&mut self.code_buffer.as_str())
                                        .code_editor()
                                        .desired_width(f32::INFINITY),
                                );
                                self.code_buffer.clear();
                            }
                            ui.separator();
                        }
                        TagEnd::Paragraph => {
                            if !current_text.is_empty() {
                                self.render_text(ui, &current_text, in_emphasis, in_strong, in_blockquote);
                                current_text.clear();
                            }
                        }
                        _ => {}
                    }
                }
                Event::Text(text) => {
                    if self.in_code_block {
                        self.code_buffer.push_str(&text);
                    } else {
                        current_text.push_str(&text);
                    }
                }
                Event::Code(code) => {
                    if !current_text.is_empty() {
                        self.render_text(ui, &current_text, false, false, in_blockquote);
                        current_text.clear();
                    }
                    ui.label(RichText::new(code.to_string()).monospace().background_color(Color32::from_rgb(240, 240, 240)));
                }
                Event::Rule => {
                    if !current_text.is_empty() {
                        self.render_text(ui, &current_text, false, false, in_blockquote);
                        current_text.clear();
                    }
                    ui.separator();
                }
                Event::SoftBreak | Event::HardBreak => {
                    current_text.push('\n');
                }
                _ => {}
            }
        }

        // 输出剩余文本
        if !current_text.is_empty() {
            self.render_text(ui, &current_text, in_emphasis, in_strong, in_blockquote);
        }
    }

    /// 渲染标题
    fn render_heading(&self, ui: &mut Ui, text: &str, level: u32) {
        let rich_text = match level {
            1 => RichText::new(text).strong().size(24.0),
            2 => RichText::new(text).strong().size(20.0),
            3 => RichText::new(text).strong().size(16.0),
            4 => RichText::new(text).strong().size(14.0),
            _ => RichText::new(text).strong().size(13.0),
        };
        ui.add_space(8.0);
        ui.label(rich_text);
        ui.add_space(4.0);
    }

    /// 渲染普通文本
    fn render_text(
        &self,
        ui: &mut Ui,
        text: &str,
        emphasis: bool,
        strong: bool,
        blockquote: bool,
    ) {
        let mut rich_text = RichText::new(text.to_string());

        if strong {
            rich_text = rich_text.strong();
        }
        if emphasis {
            rich_text = rich_text.italics();
        }
        if blockquote {
            rich_text = rich_text.italics().color(Color32::GRAY);
            ui.horizontal(|ui| {
                ui.label("│");
                ui.label(rich_text);
            });
        } else {
            ui.label(rich_text);
        }
    }
}