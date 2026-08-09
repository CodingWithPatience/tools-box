use egui::{Color32, RichText, Ui};
use pulldown_cmark::{Alignment, CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag};

/// 解析后的 Markdown 文档。
#[derive(Debug, Clone, PartialEq)]
pub struct MarkdownDocument {
    /// 文档中的块级内容。
    pub blocks: Vec<MarkdownBlock>,
}

/// Markdown 块级内容。
#[derive(Debug, Clone, PartialEq)]
pub enum MarkdownBlock {
    /// 标题。
    Heading {
        /// 标题级别，取值范围为 1 到 6。
        level: u8,
        /// 标题行内内容。
        content: Vec<MarkdownInline>,
    },
    /// 普通段落。
    Paragraph(Vec<MarkdownInline>),
    /// 引用块。
    Quote(Vec<MarkdownBlock>),
    /// 列表。
    List {
        /// 是否为有序列表。
        ordered: bool,
        /// 有序列表的起始编号，无序列表固定为 1。
        start: u64,
        /// 列表项。
        items: Vec<MarkdownListItem>,
    },
    /// 代码块。
    CodeBlock {
        /// 代码语言，未声明语言时为空。
        language: Option<String>,
        /// 代码内容。
        content: String,
    },
    /// 表格。
    Table {
        /// 每一列的对齐方式。
        alignments: Vec<MarkdownTableAlignment>,
        /// 表头单元格。
        headers: Vec<Vec<MarkdownInline>>,
        /// 表格数据行。
        rows: Vec<Vec<Vec<MarkdownInline>>>,
    },
    /// 分割线。
    ThematicBreak,
}

/// Markdown 表格列对齐方式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarkdownTableAlignment {
    /// 未指定对齐方式。
    None,
    /// 左对齐。
    Left,
    /// 居中对齐。
    Center,
    /// 右对齐。
    Right,
}

/// Markdown 列表项。
#[derive(Debug, Clone, PartialEq)]
pub struct MarkdownListItem {
    /// 任务列表状态。普通列表项为空，任务列表项为已完成或未完成状态。
    pub checked: Option<bool>,
    /// 列表项中的块级内容。
    pub blocks: Vec<MarkdownBlock>,
}

/// Markdown 行内内容。
#[derive(Debug, Clone, PartialEq)]
pub enum MarkdownInline {
    /// 普通文本。
    Text(String),
    /// 斜体文本。
    Emphasis(Vec<MarkdownInline>),
    /// 粗体文本。
    Strong(Vec<MarkdownInline>),
    /// 删除线文本。
    Strikethrough(Vec<MarkdownInline>),
    /// 行内代码。
    Code(String),
    /// 链接。
    Link {
        /// 链接地址。
        destination: String,
        /// 链接标题。
        title: Option<String>,
        /// 链接显示内容。
        content: Vec<MarkdownInline>,
    },
    /// 图片。
    Image {
        /// 图片地址。
        destination: String,
        /// 图片标题。
        title: Option<String>,
        /// 图片替代文本。
        alt: String,
    },
    /// 软换行。
    SoftBreak,
    /// 强制换行。
    HardBreak,
    /// 原始 HTML，以文本形式保留，避免在桌面 UI 中执行或解释 HTML。
    Html(String),
}

impl MarkdownDocument {
    /// 使用 pulldown-cmark 将 Markdown 文本解析为文档模型。
    pub fn parse(markdown: &str) -> Self {
        let options =
            Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TABLES | Options::ENABLE_TASKLISTS;
        let parser = Parser::new_ext(markdown, options);
        let root = RawNode::from_events(parser);

        Self {
            blocks: build_blocks(&root.children),
        }
    }
}

#[derive(Debug, Clone)]
enum RawKind {
    Root,
    Transparent,
    Paragraph,
    Heading(u8),
    Quote,
    CodeBlock(Option<String>),
    List(Option<u64>),
    Item,
    Emphasis,
    Strong,
    Strikethrough,
    Link {
        destination: String,
        title: Option<String>,
    },
    Image {
        destination: String,
        title: Option<String>,
    },
    Table(Vec<MarkdownTableAlignment>),
    TableHead,
    TableRow,
    TableCell,
    Text(String),
    Code(String),
    SoftBreak,
    HardBreak,
    TaskListMarker(bool),
    Rule,
    Html(String),
}

#[derive(Debug, Clone)]
struct RawNode {
    kind: RawKind,
    children: Vec<RawNode>,
}

impl RawNode {
    fn new(kind: RawKind) -> Self {
        Self {
            kind,
            children: Vec::new(),
        }
    }

    fn from_events<'a>(events: impl IntoIterator<Item = Event<'a>>) -> Self {
        let mut stack = vec![Self::new(RawKind::Root)];

        for event in events {
            match event {
                Event::Start(tag) => stack.push(Self::new(raw_kind_from_tag(tag))),
                Event::End(_) => {
                    if stack.len() > 1 {
                        if let Some(node) = stack.pop() {
                            if let Some(parent) = stack.last_mut() {
                                parent.children.push(node);
                            }
                        }
                    }
                }
                Event::Text(text) => append_leaf(&mut stack, RawKind::Text(text.into_string())),
                Event::Code(code) => append_leaf(&mut stack, RawKind::Code(code.into_string())),
                Event::SoftBreak => append_leaf(&mut stack, RawKind::SoftBreak),
                Event::HardBreak => append_leaf(&mut stack, RawKind::HardBreak),
                Event::Rule => append_leaf(&mut stack, RawKind::Rule),
                Event::TaskListMarker(checked) => {
                    append_leaf(&mut stack, RawKind::TaskListMarker(checked));
                }
                Event::Html(html) | Event::InlineHtml(html) => {
                    append_leaf(&mut stack, RawKind::Html(html.into_string()));
                }
                Event::InlineMath(math) => {
                    append_leaf(&mut stack, RawKind::Text(format!("${}$", math)));
                }
                Event::DisplayMath(math) => {
                    append_leaf(&mut stack, RawKind::Text(format!("$${}$$", math)));
                }
                Event::FootnoteReference(reference) => {
                    append_leaf(&mut stack, RawKind::Text(format!("[^{reference}]")));
                }
            }
        }

        match stack.into_iter().next() {
            Some(root) => root,
            None => Self::new(RawKind::Root),
        }
    }
}

fn append_leaf(stack: &mut [RawNode], kind: RawKind) {
    if let Some(parent) = stack.last_mut() {
        parent.children.push(RawNode::new(kind));
    }
}

fn raw_kind_from_tag(tag: Tag<'_>) -> RawKind {
    match tag {
        Tag::Paragraph => RawKind::Paragraph,
        Tag::Heading { level, .. } => RawKind::Heading(heading_level(level)),
        Tag::BlockQuote(_) => RawKind::Quote,
        Tag::CodeBlock(kind) => RawKind::CodeBlock(match kind {
            CodeBlockKind::Fenced(language) if !language.is_empty() => Some(language.into_string()),
            CodeBlockKind::Fenced(_) | CodeBlockKind::Indented => None,
        }),
        Tag::List(start) => RawKind::List(start),
        Tag::Item => RawKind::Item,
        Tag::Emphasis => RawKind::Emphasis,
        Tag::Strong => RawKind::Strong,
        Tag::Strikethrough => RawKind::Strikethrough,
        Tag::Link {
            dest_url, title, ..
        } => RawKind::Link {
            destination: dest_url.into_string(),
            title: optional_string(title),
        },
        Tag::Image {
            dest_url, title, ..
        } => RawKind::Image {
            destination: dest_url.into_string(),
            title: optional_string(title),
        },
        Tag::Table(alignments) => {
            RawKind::Table(alignments.into_iter().map(table_alignment).collect())
        }
        Tag::TableHead => RawKind::TableHead,
        Tag::TableRow => RawKind::TableRow,
        Tag::TableCell => RawKind::TableCell,
        _ => RawKind::Transparent,
    }
}

fn heading_level(level: HeadingLevel) -> u8 {
    match level {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

fn optional_string(value: pulldown_cmark::CowStr<'_>) -> Option<String> {
    if value.is_empty() {
        None
    } else {
        Some(value.into_string())
    }
}

fn table_alignment(alignment: Alignment) -> MarkdownTableAlignment {
    match alignment {
        Alignment::None => MarkdownTableAlignment::None,
        Alignment::Left => MarkdownTableAlignment::Left,
        Alignment::Center => MarkdownTableAlignment::Center,
        Alignment::Right => MarkdownTableAlignment::Right,
    }
}

fn build_blocks(nodes: &[RawNode]) -> Vec<MarkdownBlock> {
    let mut blocks = Vec::new();

    for node in nodes {
        match &node.kind {
            RawKind::Paragraph => {
                blocks.push(MarkdownBlock::Paragraph(build_inlines(&node.children)));
            }
            RawKind::Heading(level) => {
                blocks.push(MarkdownBlock::Heading {
                    level: *level,
                    content: build_inlines(&node.children),
                });
            }
            RawKind::Quote => {
                blocks.push(MarkdownBlock::Quote(build_blocks(&node.children)));
            }
            RawKind::List(ordered) => {
                let items = node.children.iter().filter_map(build_list_item).collect();
                blocks.push(MarkdownBlock::List {
                    ordered: ordered.is_some(),
                    start: ordered.unwrap_or(1),
                    items,
                });
            }
            RawKind::CodeBlock(language) => {
                blocks.push(MarkdownBlock::CodeBlock {
                    language: language.clone(),
                    content: collect_text(node),
                });
            }
            RawKind::Table(alignments) => {
                if let Some(table) = build_table(node, alignments) {
                    blocks.push(table);
                }
            }
            RawKind::Rule => blocks.push(MarkdownBlock::ThematicBreak),
            RawKind::Transparent => blocks.extend(build_blocks(&node.children)),
            RawKind::Root => blocks.extend(build_blocks(&node.children)),
            RawKind::Text(_) | RawKind::Html(_) => {
                let content = build_inlines(std::slice::from_ref(node));
                if !content.is_empty() {
                    blocks.push(MarkdownBlock::Paragraph(content));
                }
            }
            RawKind::Item
            | RawKind::TableHead
            | RawKind::TableRow
            | RawKind::TableCell
            | RawKind::Emphasis
            | RawKind::Strong
            | RawKind::Strikethrough
            | RawKind::Link { .. }
            | RawKind::Image { .. }
            | RawKind::Code(_)
            | RawKind::SoftBreak
            | RawKind::HardBreak
            | RawKind::TaskListMarker(_) => {}
        }
    }

    blocks
}

fn build_list_item(node: &RawNode) -> Option<MarkdownListItem> {
    if !matches!(node.kind, RawKind::Item) {
        return None;
    }

    let checked = find_task_marker(&node.children);
    Some(MarkdownListItem {
        checked,
        blocks: build_blocks(&node.children),
    })
}

fn find_task_marker(nodes: &[RawNode]) -> Option<bool> {
    for node in nodes {
        match node.kind {
            RawKind::TaskListMarker(checked) => return Some(checked),
            RawKind::List(_) => continue,
            _ => {}
        }

        if let Some(checked) = find_task_marker(&node.children) {
            return Some(checked);
        }
    }

    None
}

fn build_table(node: &RawNode, alignments: &[MarkdownTableAlignment]) -> Option<MarkdownBlock> {
    let mut headers = Vec::new();
    let mut rows = Vec::new();

    for child in &node.children {
        match child.kind {
            RawKind::TableHead => headers = build_table_row(&child.children),
            RawKind::TableRow => rows.push(build_table_row(&child.children)),
            _ => {}
        }
    }

    if headers.is_empty() && rows.is_empty() {
        None
    } else {
        Some(MarkdownBlock::Table {
            alignments: alignments.to_vec(),
            headers,
            rows,
        })
    }
}

fn build_table_row(nodes: &[RawNode]) -> Vec<Vec<MarkdownInline>> {
    nodes
        .iter()
        .filter_map(|node| match node.kind {
            RawKind::TableCell => Some(build_inlines(&node.children)),
            _ => None,
        })
        .collect()
}

fn build_inlines(nodes: &[RawNode]) -> Vec<MarkdownInline> {
    let mut inlines = Vec::new();

    for node in nodes {
        let inline = match &node.kind {
            RawKind::Text(text) => Some(MarkdownInline::Text(text.clone())),
            RawKind::Emphasis => Some(MarkdownInline::Emphasis(build_inlines(&node.children))),
            RawKind::Strong => Some(MarkdownInline::Strong(build_inlines(&node.children))),
            RawKind::Strikethrough => {
                Some(MarkdownInline::Strikethrough(build_inlines(&node.children)))
            }
            RawKind::Code(code) => Some(MarkdownInline::Code(code.clone())),
            RawKind::Link { destination, title } => Some(MarkdownInline::Link {
                destination: destination.clone(),
                title: title.clone(),
                content: build_inlines(&node.children),
            }),
            RawKind::Image { destination, title } => Some(MarkdownInline::Image {
                destination: destination.clone(),
                title: title.clone(),
                alt: collect_text(node),
            }),
            RawKind::SoftBreak => Some(MarkdownInline::SoftBreak),
            RawKind::HardBreak => Some(MarkdownInline::HardBreak),
            RawKind::Html(html) => Some(MarkdownInline::Html(html.clone())),
            RawKind::Transparent => {
                inlines.extend(build_inlines(&node.children));
                None
            }
            RawKind::TaskListMarker(_) => None,
            RawKind::Root => {
                inlines.extend(build_inlines(&node.children));
                None
            }
            RawKind::Paragraph
            | RawKind::Heading(_)
            | RawKind::Quote
            | RawKind::List(_)
            | RawKind::Item
            | RawKind::CodeBlock(_)
            | RawKind::Table(_)
            | RawKind::TableHead
            | RawKind::TableRow
            | RawKind::TableCell
            | RawKind::Rule => {
                inlines.extend(build_inlines(&node.children));
                None
            }
        };

        if let Some(inline) = inline {
            inlines.push(inline);
        }
    }

    inlines
}

fn collect_text(node: &RawNode) -> String {
    let mut text = String::new();
    collect_text_into(node, &mut text);
    text
}

fn collect_text_into(node: &RawNode, output: &mut String) {
    match &node.kind {
        RawKind::Text(value) | RawKind::Code(value) | RawKind::Html(value) => {
            output.push_str(value);
        }
        RawKind::SoftBreak | RawKind::HardBreak => output.push('\n'),
        RawKind::TaskListMarker(checked) => {
            output.push_str(if *checked { "[x] " } else { "[ ] " });
        }
        _ => {
            for child in &node.children {
                collect_text_into(child, output);
            }
        }
    }
}

/// Markdown 渲染器。
///
/// 渲染器负责将 [`MarkdownDocument`] 映射为 egui 控件，解析逻辑由文档模型负责。
pub struct MarkdownRenderer;

#[derive(Clone, Copy, Default)]
struct InlineStyle {
    emphasis: bool,
    strong: bool,
    strikethrough: bool,
    link: bool,
    size: Option<f32>,
}

impl MarkdownRenderer {
    /// 创建新的渲染器实例。
    pub fn new() -> Self {
        Self
    }

    /// 渲染 Markdown 内容到 egui UI。
    pub fn render(&mut self, ui: &mut Ui, markdown: &str) {
        let document = MarkdownDocument::parse(markdown);
        self.render_blocks(ui, &document.blocks, 0);
    }

    fn render_blocks(&self, ui: &mut Ui, blocks: &[MarkdownBlock], list_depth: usize) {
        for block in blocks {
            match block {
                MarkdownBlock::Heading { level, content } => {
                    let style = InlineStyle {
                        strong: true,
                        size: Some(heading_size(*level)),
                        ..InlineStyle::default()
                    };
                    ui.add_space(8.0);
                    ui.horizontal_wrapped(|ui| self.render_inlines(ui, content, style));
                    ui.add_space(4.0);
                }
                MarkdownBlock::Paragraph(content) => {
                    ui.horizontal_wrapped(|ui| {
                        self.render_inlines(ui, content, InlineStyle::default());
                    });
                    ui.add_space(6.0);
                }
                MarkdownBlock::Quote(content) => {
                    ui.horizontal_top(|ui| {
                        ui.label(RichText::new("│").color(Color32::GRAY));
                        ui.vertical(|ui| self.render_blocks(ui, content, list_depth));
                    });
                    ui.add_space(6.0);
                }
                MarkdownBlock::List {
                    ordered,
                    start,
                    items,
                } => {
                    for (index, item) in items.iter().enumerate() {
                        ui.horizontal_top(|ui| {
                            ui.add_space(list_indent(list_depth));
                            let marker = if let Some(checked) = item.checked {
                                if checked { "☑" } else { "☐" }
                            } else if *ordered {
                                ""
                            } else {
                                "•"
                            };
                            let marker = if *ordered && item.checked.is_none() {
                                let item_number =
                                    start.saturating_add(u64::try_from(index).unwrap_or(u64::MAX));
                                format!("{}. ", item_number)
                            } else {
                                format!("{} ", marker)
                            };
                            ui.label(marker);
                            ui.vertical(|ui| {
                                self.render_blocks(ui, &item.blocks, list_depth + 1);
                            });
                        });
                    }
                    ui.add_space(4.0);
                }
                MarkdownBlock::CodeBlock { language, content } => {
                    ui.group(|ui| {
                        if let Some(language) = language {
                            ui.label(RichText::new(language).small().strong());
                        }
                        let mut code = content.clone();
                        ui.add(
                            egui::TextEdit::multiline(&mut code)
                                .code_editor()
                                .interactive(false)
                                .desired_width(f32::INFINITY),
                        );
                    });
                    ui.add_space(6.0);
                }
                MarkdownBlock::Table {
                    alignments: _,
                    headers,
                    rows,
                } => {
                    let mut cell_rects = Vec::new();
                    egui::Grid::new(ui.next_auto_id())
                        .spacing(egui::vec2(0.0, 0.0))
                        .show(ui, |ui| {
                            if !headers.is_empty() {
                                for cell in headers {
                                    cell_rects.push(self.render_table_cell(ui, cell, true));
                                }
                                ui.end_row();
                            }
                            for row in rows {
                                for cell in row {
                                    cell_rects.push(self.render_table_cell(ui, cell, false));
                                }
                                ui.end_row();
                            }
                        });
                    self.paint_table_grid(ui, &cell_rects);
                    ui.add_space(6.0);
                }
                MarkdownBlock::ThematicBreak => {
                    ui.separator();
                    ui.add_space(4.0);
                }
            }
        }
    }

    fn render_table_cell(
        &self,
        ui: &mut Ui,
        content: &[MarkdownInline],
        header: bool,
    ) -> egui::Rect {
        let fill = if header {
            ui.visuals().faint_bg_color
        } else {
            Color32::TRANSPARENT
        };
        let response = egui::Frame::new()
            .fill(fill)
            .inner_margin(egui::Margin::symmetric(6, 4))
            .show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    self.render_inlines(
                        ui,
                        content,
                        InlineStyle {
                            strong: header,
                            ..InlineStyle::default()
                        },
                    );
                });
            });
        response.response.rect
    }

    fn paint_table_grid(&self, ui: &mut Ui, cell_rects: &[egui::Rect]) {
        let Some(table_rect) = cell_rects
            .iter()
            .copied()
            .reduce(|current, cell| current.union(cell))
        else {
            return;
        };
        let stroke = ui.visuals().widgets.noninteractive.bg_stroke;
        let painter = ui.painter();

        for rect in cell_rects {
            painter.line_segment(
                [
                    egui::pos2(rect.left(), rect.top()),
                    egui::pos2(rect.right(), rect.top()),
                ],
                stroke,
            );
            painter.line_segment(
                [
                    egui::pos2(rect.left(), rect.top()),
                    egui::pos2(rect.left(), rect.bottom()),
                ],
                stroke,
            );
        }
        painter.line_segment(
            [
                egui::pos2(table_rect.right(), table_rect.top()),
                egui::pos2(table_rect.right(), table_rect.bottom()),
            ],
            stroke,
        );
        painter.line_segment(
            [
                egui::pos2(table_rect.left(), table_rect.bottom()),
                egui::pos2(table_rect.right(), table_rect.bottom()),
            ],
            stroke,
        );
    }

    fn render_inlines(&self, ui: &mut Ui, inlines: &[MarkdownInline], style: InlineStyle) {
        for inline in inlines {
            match inline {
                MarkdownInline::Text(text) => self.render_text(ui, text, style, false),
                MarkdownInline::Emphasis(content) => self.render_inlines(
                    ui,
                    content,
                    InlineStyle {
                        emphasis: true,
                        ..style
                    },
                ),
                MarkdownInline::Strong(content) => self.render_inlines(
                    ui,
                    content,
                    InlineStyle {
                        strong: true,
                        ..style
                    },
                ),
                MarkdownInline::Strikethrough(content) => self.render_inlines(
                    ui,
                    content,
                    InlineStyle {
                        strikethrough: true,
                        ..style
                    },
                ),
                MarkdownInline::Code(code) => self.render_text(ui, code, style, true),
                MarkdownInline::Link { content, .. } => self.render_inlines(
                    ui,
                    content,
                    InlineStyle {
                        link: true,
                        ..style
                    },
                ),
                MarkdownInline::Image {
                    alt, destination, ..
                } => {
                    let text = if alt.is_empty() {
                        format!("[图片: {destination}]")
                    } else {
                        format!("[图片: {alt} · {destination}]")
                    };
                    self.render_text(ui, &text, style, false);
                }
                MarkdownInline::SoftBreak => {
                    ui.label(" ");
                }
                MarkdownInline::HardBreak => {
                    ui.label("\n");
                }
                MarkdownInline::Html(html) => self.render_text(ui, html, style, false),
            }
        }
    }

    fn render_text(&self, ui: &mut Ui, text: &str, style: InlineStyle, code: bool) {
        let mut rich_text = RichText::new(text.to_owned());
        if style.strong {
            rich_text = rich_text.strong();
        }
        if style.emphasis {
            rich_text = rich_text.italics();
        }
        if style.strikethrough {
            rich_text = rich_text.strikethrough();
        }
        if style.link {
            rich_text = rich_text.color(ui.visuals().hyperlink_color).underline();
        }
        if code {
            rich_text = rich_text
                .monospace()
                .background_color(ui.visuals().faint_bg_color);
        }
        if let Some(size) = style.size {
            rich_text = rich_text.size(size);
        }
        ui.label(rich_text);
    }
}

fn heading_size(level: u8) -> f32 {
    match level {
        1 => 24.0,
        2 => 20.0,
        3 => 16.0,
        4 => 14.0,
        _ => 13.0,
    }
}

fn list_indent(depth: usize) -> f32 {
    let depth = u16::try_from(depth).unwrap_or(u16::MAX).min(32);
    f32::from(depth) * 16.0
}

#[cfg(test)]
mod tests {
    use super::{MarkdownBlock, MarkdownDocument, MarkdownInline, MarkdownTableAlignment};

    #[test]
    fn should_parse_nested_inline_styles() {
        let document = MarkdownDocument::parse("**粗体 *斜体*** 和 ~~删除~~ 与 `代码`");

        assert_eq!(document.blocks.len(), 1);
        assert_eq!(
            document.blocks[0],
            MarkdownBlock::Paragraph(vec![
                MarkdownInline::Strong(vec![
                    MarkdownInline::Text("粗体 ".to_string()),
                    MarkdownInline::Emphasis(vec![MarkdownInline::Text("斜体".to_string())]),
                ]),
                MarkdownInline::Text(" 和 ".to_string()),
                MarkdownInline::Strikethrough(vec![MarkdownInline::Text("删除".to_string())]),
                MarkdownInline::Text(" 与 ".to_string()),
                MarkdownInline::Code("代码".to_string()),
            ])
        );
    }

    #[test]
    fn should_parse_lists_and_task_markers() {
        let document = MarkdownDocument::parse("1. 第一项\n2. 第二项\n\n- [ ] 待办\n- [x] 完成");

        assert_eq!(document.blocks.len(), 2);
        assert!(matches!(
            &document.blocks[0],
            MarkdownBlock::List {
                ordered: true,
                items,
                ..
            } if items.len() == 2
        ));
        assert!(matches!(
            &document.blocks[1],
            MarkdownBlock::List {
                ordered: false,
                items,
                ..
            } if items.iter().map(|item| item.checked).collect::<Vec<_>>()
                == vec![Some(false), Some(true)]
        ));
    }

    #[test]
    fn should_not_inherit_nested_task_marker_to_parent_item() {
        let document = MarkdownDocument::parse("- 父项\n  - [x] 子项");

        let MarkdownBlock::List { items, .. } = &document.blocks[0] else {
            panic!("应解析为列表");
        };
        assert_eq!(items[0].checked, None);
        assert!(matches!(
            &items[0].blocks[1],
            MarkdownBlock::List { items, .. }
                if items[0].checked == Some(true)
        ));
    }

    #[test]
    fn should_parse_tables_links_images_and_code_language() {
        let document = MarkdownDocument::parse(
            "| 名称 | 值 |\n| --- | --- |\n| [Rust](https://www.rust-lang.org) | ![标志](logo.png) |\n\n```rust\nfn main() {}\n```",
        );

        assert!(matches!(&document.blocks[0], MarkdownBlock::Table {
            alignments,
            headers,
            rows,
        } if alignments == &vec![MarkdownTableAlignment::None, MarkdownTableAlignment::None]
            && headers.len() == 2
            && rows.len() == 1));
        let MarkdownBlock::Table { rows, .. } = &document.blocks[0] else {
            panic!("应解析为表格");
        };
        assert!(matches!(
            &rows[0][0][0],
            MarkdownInline::Link { destination, .. }
                if destination == "https://www.rust-lang.org"
        ));
        assert!(matches!(
            &rows[0][1][0],
            MarkdownInline::Image { destination, alt, .. }
                if destination == "logo.png" && alt == "标志"
        ));
        assert!(matches!(
            &document.blocks[1],
            MarkdownBlock::CodeBlock { language, content }
                if language.as_deref() == Some("rust") && content.contains("fn main")
        ));
    }

    #[test]
    fn should_parse_quote_and_horizontal_rule() {
        let document = MarkdownDocument::parse("> 引用\n\n---");

        assert!(matches!(&document.blocks[0], MarkdownBlock::Quote(blocks)
            if blocks.len() == 1));
        assert_eq!(document.blocks[1], MarkdownBlock::ThematicBreak);
    }
}
