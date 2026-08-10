use std::cell::RefCell;
use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};

use egui::text::{LayoutJob, TextFormat};
use egui::{Color32, FontId, RichText, Stroke, Ui};
use pulldown_cmark::{Alignment, CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag};

use crate::utils::highlight::SyntaxHighlighter;

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
pub struct MarkdownRenderer {
    highlighter: SyntaxHighlighter,
    code_highlight_cache: RefCell<HashMap<u64, CodeHighlightCacheEntry>>,
}

/// 单条代码高亮缓存，保存完整上下文以避免哈希碰撞返回错误结果。
struct CodeHighlightCacheEntry {
    content: String,
    language: Option<String>,
    font_size_bits: u32,
    is_dark_mode: bool,
    job: LayoutJob,
}

impl CodeHighlightCacheEntry {
    fn matches(
        &self,
        content: &str,
        language: Option<&str>,
        font_size: f32,
        is_dark_mode: bool,
    ) -> bool {
        self.content == content
            && self.language.as_deref() == language
            && self.font_size_bits == font_size.to_bits()
            && self.is_dark_mode == is_dark_mode
    }
}

const MAX_CODE_HIGHLIGHT_CACHE_ENTRIES: usize = 128;

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
        Self {
            highlighter: SyntaxHighlighter::new(),
            code_highlight_cache: RefCell::new(HashMap::new()),
        }
    }

    /// 渲染 Markdown 内容到 egui UI。
    pub fn render(&mut self, ui: &mut Ui, markdown: &str) {
        let document = MarkdownDocument::parse(markdown);
        if document.blocks.is_empty() {
            let weak_text_color = ui.visuals().weak_text_color();
            ui.vertical_centered(|ui| {
                ui.add_space(32.0);
                ui.label(
                    RichText::new("暂无可预览内容")
                        .color(weak_text_color)
                        .italics(),
                );
            });
            return;
        }
        self.render_blocks(ui, &document.blocks, 0);
    }

    fn render_blocks(&self, ui: &mut Ui, blocks: &[MarkdownBlock], list_depth: usize) {
        for (block_index, block) in blocks.iter().enumerate() {
            let is_last_block = block_index + 1 == blocks.len();
            match block {
                MarkdownBlock::Heading { level, content } => {
                    let style = InlineStyle {
                        strong: true,
                        size: Some(heading_size(*level)),
                        ..InlineStyle::default()
                    };
                    ui.add_space(12.0);
                    self.render_inlines(ui, content, style);
                    if !is_last_block {
                        ui.add_space(6.0);
                    }
                }
                MarkdownBlock::Paragraph(content) => {
                    self.render_inlines(ui, content, InlineStyle::default());
                    if !is_last_block {
                        ui.add_space(8.0);
                    }
                }
                MarkdownBlock::Quote(content) => {
                    let quote_stroke =
                        Stroke::new(1.0, ui.visuals().widgets.noninteractive.bg_stroke.color);
                    let quote_accent = ui.visuals().selection.bg_fill;
                    let quote_content_width = (ui.available_width() - 28.0).max(0.0);
                    let quote_rect = egui::Frame::new()
                        .fill(ui.visuals().faint_bg_color)
                        .stroke(quote_stroke)
                        .inner_margin(egui::Margin::symmetric(14, 8))
                        .show(ui, |ui| {
                            ui.set_min_width(quote_content_width);
                            ui.set_max_width(quote_content_width);
                            self.render_blocks(ui, content, list_depth);
                        })
                        .response
                        .rect;
                    let quote_line_x = quote_rect.left() + 6.0;
                    ui.painter().line_segment(
                        [
                            egui::pos2(quote_line_x, quote_rect.top() + 1.0),
                            egui::pos2(quote_line_x, quote_rect.bottom() - 1.0),
                        ],
                        Stroke::new(2.0, quote_accent),
                    );
                    if !is_last_block {
                        ui.add_space(8.0);
                    }
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
                            ui.label(
                                RichText::new(marker)
                                    .color(ui.visuals().weak_text_color())
                                    .strong(),
                            );
                            ui.vertical(|ui| {
                                self.render_blocks(ui, &item.blocks, list_depth + 1);
                            });
                            ui.add_space(2.0);
                        });
                    }
                    if !is_last_block {
                        ui.add_space(8.0);
                    }
                }
                MarkdownBlock::CodeBlock { language, content } => {
                    let code_stroke =
                        Stroke::new(1.0, ui.visuals().widgets.noninteractive.bg_stroke.color);
                    egui::Frame::new()
                        .fill(ui.visuals().code_bg_color)
                        .stroke(code_stroke)
                        .inner_margin(egui::Margin::symmetric(8, 6))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                if let Some(language) = language {
                                    ui.label(
                                        RichText::new(language)
                                            .small()
                                            .strong()
                                            .color(ui.visuals().weak_text_color()),
                                    );
                                } else {
                                    ui.label(
                                        RichText::new("纯文本")
                                            .small()
                                            .color(ui.visuals().weak_text_color()),
                                    );
                                }
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        if ui.small_button("复制").clicked() {
                                            ui.ctx().copy_text(content.clone());
                                        }
                                    },
                                );
                            });
                            ui.add_space(4.0);
                            let code_job = self.highlight_code(ui, language.as_deref(), content);
                            let scroll_id = code_highlight_cache_key(
                                content,
                                language.as_deref(),
                                monospace_font_size(ui),
                                ui.visuals().dark_mode,
                            );
                            egui::ScrollArea::horizontal()
                                .id_salt((
                                    "markdown_code_block",
                                    list_depth,
                                    block_index,
                                    scroll_id,
                                ))
                                // 仅关闭水平方向收缩，垂直方向必须按代码实际高度收缩，
                                // 否则第一个嵌套水平滚动区会占满父级剩余高度。
                                .auto_shrink([false, true])
                                .show(ui, |ui| {
                                    ui.add(egui::Label::new(code_job).extend());
                                });
                        });
                    if !is_last_block {
                        ui.add_space(8.0);
                    }
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
                    if !is_last_block {
                        ui.add_space(8.0);
                    }
                }
                MarkdownBlock::ThematicBreak => {
                    ui.add_space(4.0);
                    ui.separator();
                    if !is_last_block {
                        ui.add_space(8.0);
                    }
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
                self.render_inlines(
                    ui,
                    content,
                    InlineStyle {
                        strong: header,
                        ..InlineStyle::default()
                    },
                );
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

        let mut x_boundaries = vec![table_rect.left(), table_rect.right()];
        let mut y_boundaries = vec![table_rect.top(), table_rect.bottom()];
        for rect in cell_rects {
            if rect.left() > table_rect.left() + 0.5 && rect.left() < table_rect.right() - 0.5 {
                x_boundaries.push(rect.left());
            }
            if rect.top() > table_rect.top() + 0.5 && rect.top() < table_rect.bottom() - 0.5 {
                y_boundaries.push(rect.top());
            }
        }

        x_boundaries.sort_by(f32::total_cmp);
        y_boundaries.sort_by(f32::total_cmp);
        x_boundaries.dedup_by(|left, right| (*left - *right).abs() < 0.5);
        y_boundaries.dedup_by(|top, bottom| (*top - *bottom).abs() < 0.5);

        for y in y_boundaries {
            painter.line_segment(
                [
                    egui::pos2(table_rect.left(), y),
                    egui::pos2(table_rect.right(), y),
                ],
                stroke,
            );
        }
        for x in x_boundaries {
            painter.line_segment(
                [
                    egui::pos2(x, table_rect.top()),
                    egui::pos2(x, table_rect.bottom()),
                ],
                stroke,
            );
        }
    }

    fn render_inlines(&self, ui: &mut Ui, inlines: &[MarkdownInline], style: InlineStyle) {
        let mut job = LayoutJob::default();
        self.append_inlines_to_job(ui, inlines, style, &mut job);
        ui.add(egui::Label::new(job).wrap());
    }

    fn append_inlines_to_job(
        &self,
        ui: &Ui,
        inlines: &[MarkdownInline],
        style: InlineStyle,
        job: &mut LayoutJob,
    ) {
        for inline in inlines {
            match inline {
                MarkdownInline::Text(text) => self.append_text_to_job(ui, text, style, false, job),
                MarkdownInline::Emphasis(content) => self.append_inlines_to_job(
                    ui,
                    content,
                    InlineStyle {
                        emphasis: true,
                        ..style
                    },
                    job,
                ),
                MarkdownInline::Strong(content) => self.append_inlines_to_job(
                    ui,
                    content,
                    InlineStyle {
                        strong: true,
                        ..style
                    },
                    job,
                ),
                MarkdownInline::Strikethrough(content) => self.append_inlines_to_job(
                    ui,
                    content,
                    InlineStyle {
                        strikethrough: true,
                        ..style
                    },
                    job,
                ),
                MarkdownInline::Code(code) => {
                    self.append_text_to_job(ui, code, style, true, job);
                }
                MarkdownInline::Link { content, .. } => self.append_inlines_to_job(
                    ui,
                    content,
                    InlineStyle {
                        link: true,
                        ..style
                    },
                    job,
                ),
                MarkdownInline::Image {
                    alt, destination, ..
                } => {
                    let text = if alt.is_empty() {
                        format!("[图片: {destination}]")
                    } else {
                        format!("[图片: {alt} · {destination}]")
                    };
                    self.append_text_to_job(ui, &text, style, false, job);
                }
                MarkdownInline::SoftBreak => {
                    self.append_text_to_job(ui, "\n", style, false, job);
                }
                MarkdownInline::HardBreak => {
                    self.append_text_to_job(ui, "\n", style, false, job);
                }
                MarkdownInline::Html(html) => {
                    self.append_text_to_job(ui, html, style, false, job);
                }
            }
        }
    }

    fn highlight_code(&self, ui: &Ui, language: Option<&str>, content: &str) -> LayoutJob {
        let font_size = monospace_font_size(ui);
        let is_dark_mode = ui.visuals().dark_mode;
        let cache_key = code_highlight_cache_key(content, language, font_size, is_dark_mode);

        if let Some(entry) = self.code_highlight_cache.borrow().get(&cache_key) {
            if entry.matches(content, language, font_size, is_dark_mode) {
                return entry.job.clone();
            }
        }

        let job =
            self.highlighter
                .highlight_to_layout_job(content, language, font_size, is_dark_mode);
        let mut cache = self.code_highlight_cache.borrow_mut();
        if !cache.contains_key(&cache_key) && cache.len() >= MAX_CODE_HIGHLIGHT_CACHE_ENTRIES {
            if let Some(evicted_key) = cache.keys().next().copied() {
                cache.remove(&evicted_key);
            }
        }
        cache.insert(
            cache_key,
            CodeHighlightCacheEntry {
                content: content.to_owned(),
                language: language.map(str::to_owned),
                font_size_bits: font_size.to_bits(),
                is_dark_mode,
                job: job.clone(),
            },
        );
        job
    }

    fn append_text_to_job(
        &self,
        ui: &Ui,
        text: &str,
        style: InlineStyle,
        code: bool,
        job: &mut LayoutJob,
    ) {
        let color = if style.link {
            ui.visuals().hyperlink_color
        } else if style.strong {
            ui.visuals().strong_text_color()
        } else {
            ui.visuals().text_color()
        };
        let font_size = style.size.unwrap_or_else(|| {
            ui.style()
                .text_styles
                .get(&egui::TextStyle::Body)
                .map(|font_id| font_id.size)
                .unwrap_or(14.0)
        });
        let mut format = TextFormat {
            font_id: if code {
                FontId::monospace(font_size)
            } else {
                FontId::proportional(font_size)
            },
            color,
            ..TextFormat::default()
        };
        format.italics = style.emphasis;
        if style.strikethrough {
            format.strikethrough = Stroke::new(1.0, color);
        }
        if style.link {
            format.underline = Stroke::new(1.0, color);
        }
        if code {
            format.background = ui.visuals().code_bg_color;
        }
        job.append(text, 0.0, format);
    }
}

fn code_highlight_cache_key(
    content: &str,
    language: Option<&str>,
    font_size: f32,
    is_dark_mode: bool,
) -> u64 {
    let mut hasher = DefaultHasher::new();
    content.hash(&mut hasher);
    language.hash(&mut hasher);
    font_size.to_bits().hash(&mut hasher);
    is_dark_mode.hash(&mut hasher);
    hasher.finish()
}

fn monospace_font_size(ui: &Ui) -> f32 {
    ui.style()
        .text_styles
        .get(&egui::TextStyle::Monospace)
        .map(|font_id| font_id.size)
        .unwrap_or(14.0)
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
    use super::{
        MarkdownBlock, MarkdownDocument, MarkdownInline, MarkdownTableAlignment, SyntaxHighlighter,
        code_highlight_cache_key,
    };

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
    fn should_separate_code_highlight_cache_by_render_context() {
        let base = code_highlight_cache_key("fn main() {}", Some("rust"), 14.0, true);

        assert_eq!(
            base,
            code_highlight_cache_key("fn main() {}", Some("rust"), 14.0, true)
        );
        assert_ne!(
            base,
            code_highlight_cache_key("fn main() { println!(); }", Some("rust"), 14.0, true)
        );
        assert_ne!(
            base,
            code_highlight_cache_key("fn main() {}", Some("python"), 14.0, true)
        );
        assert_ne!(
            base,
            code_highlight_cache_key("fn main() {}", Some("rust"), 16.0, true)
        );
        assert_ne!(
            base,
            code_highlight_cache_key("fn main() {}", Some("rust"), 14.0, false)
        );
    }

    #[test]
    fn should_fallback_to_plain_text_for_unknown_code_language() {
        let highlighter = SyntaxHighlighter::new();
        let job = highlighter.highlight_to_layout_job(
            "这是一段未知语言代码",
            Some("unknown-language"),
            14.0,
            true,
        );

        assert_eq!(job.text, "这是一段未知语言代码");
    }

    #[test]
    fn should_parse_soft_break_without_inserting_space() {
        let document = MarkdownDocument::parse("第一行\n第二行");

        assert_eq!(
            document.blocks[0],
            MarkdownBlock::Paragraph(vec![
                MarkdownInline::Text("第一行".to_string()),
                MarkdownInline::SoftBreak,
                MarkdownInline::Text("第二行".to_string()),
            ])
        );
    }

    #[test]
    fn should_keep_blank_line_between_paragraphs_without_leading_space() {
        let document = MarkdownDocument::parse("第一段\n\n第二段");

        assert_eq!(
            document.blocks,
            vec![
                MarkdownBlock::Paragraph(vec![MarkdownInline::Text("第一段".to_string())]),
                MarkdownBlock::Paragraph(vec![MarkdownInline::Text("第二段".to_string())]),
            ]
        );
    }

    #[test]
    fn should_parse_quote_and_horizontal_rule() {
        let document = MarkdownDocument::parse("> 引用\n\n---");

        assert!(matches!(&document.blocks[0], MarkdownBlock::Quote(blocks)
            if blocks.len() == 1));
        assert_eq!(document.blocks[1], MarkdownBlock::ThematicBreak);
    }
}
