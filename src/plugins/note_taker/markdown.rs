use std::cell::RefCell;
use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::io::Cursor;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use egui::load::{ImageLoadResult, ImageLoader, ImagePoll, LoadError, SizeHint};
use egui::text::{LayoutJob, TextFormat};
use egui::{Color32, Context, FontId, RichText, Stroke, Ui};
use pulldown_cmark::{Alignment, CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag};
use tokio::io::AsyncReadExt;

use crate::utils::highlight::SyntaxHighlighter;

/// 解析后的 Markdown 文档。
#[derive(Debug, Clone, PartialEq)]
pub struct MarkdownDocument {
    /// 文档中的块级内容。
    pub blocks: Vec<MarkdownBlock>,
}

/// Markdown 标题目录项。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarkdownHeading {
    /// 标题级别，取值范围为 1 到 6。
    pub level: u8,
    /// 标题显示文本。
    pub title: String,
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
#[derive(Debug, Clone, PartialEq, Hash)]
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

    /// 提取文档中的标题目录，保留标题层级和文档顺序。
    pub fn headings(&self) -> Vec<MarkdownHeading> {
        let mut headings = Vec::new();
        collect_headings(&self.blocks, &mut headings);
        headings
    }
}

fn collect_headings(blocks: &[MarkdownBlock], headings: &mut Vec<MarkdownHeading>) {
    for block in blocks {
        match block {
            MarkdownBlock::Heading { level, content } => headings.push(MarkdownHeading {
                level: *level,
                title: inline_text(content),
            }),
            MarkdownBlock::Quote(content) => collect_headings(content, headings),
            MarkdownBlock::List { items, .. } => {
                for item in items {
                    collect_headings(&item.blocks, headings);
                }
            }
            MarkdownBlock::Paragraph(_)
            | MarkdownBlock::CodeBlock { .. }
            | MarkdownBlock::Table { .. }
            | MarkdownBlock::ThematicBreak => {}
        }
    }
}

fn inline_text(inlines: &[MarkdownInline]) -> String {
    let mut text = String::new();
    for inline in inlines {
        match inline {
            MarkdownInline::Text(value)
            | MarkdownInline::Code(value)
            | MarkdownInline::Html(value) => text.push_str(value),
            MarkdownInline::Emphasis(content)
            | MarkdownInline::Strong(content)
            | MarkdownInline::Strikethrough(content)
            | MarkdownInline::Link { content, .. } => text.push_str(&inline_text(content)),
            MarkdownInline::Image { alt, .. } => text.push_str(alt),
            MarkdownInline::SoftBreak | MarkdownInline::HardBreak => text.push(' '),
        }
    }
    text
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
    parsed_document_cache: Option<ParsedDocumentCache>,
    inline_layout_cache: RefCell<HashMap<u64, InlineLayoutCacheEntry>>,
    code_highlight_cache: RefCell<HashMap<u64, CodeHighlightCacheEntry>>,
}

/// 最近一次 Markdown 内容对应的解析结果。
struct ParsedDocumentCache {
    content: String,
    document: Rc<MarkdownDocument>,
}

const MAX_MARKDOWN_IMAGE_BYTES: u64 = 20 * 1024 * 1024;
const MAX_MARKDOWN_IMAGE_CACHE_ENTRIES: usize = 64;
const MAX_MARKDOWN_IMAGE_CACHE_BYTES: usize = 64 * 1024 * 1024;
const MAX_MARKDOWN_IMAGE_DIMENSION: u32 = 8192;
const MAX_MARKDOWN_IMAGE_ACTIVE: usize = 8;
const MARKDOWN_IMAGE_LOAD_TIMEOUT: Duration = Duration::from_secs(10);

enum MarkdownImageState {
    Pending(u64),
    Ready(Arc<egui::ColorImage>),
    Failed { error: String, retry_at: Instant },
}

/// 安装 Markdown 图片加载器。
///
/// 加载器支持 `file://`、`http://` 和 `https://` 图片地址，并在后台线程执行
/// 文件读取、网络请求和图片解码，避免阻塞 egui UI 线程。
pub fn install_markdown_image_loader(ctx: &Context) {
    ctx.add_image_loader(Arc::new(MarkdownImageLoader::default()));
}

#[derive(Default)]
struct MarkdownImageLoader {
    cache: Arc<Mutex<HashMap<String, MarkdownImageState>>>,
    next_request_id: AtomicU64,
    active_requests: Arc<AtomicUsize>,
}

struct ActiveImageRequestGuard(Arc<AtomicUsize>);

impl Drop for ActiveImageRequestGuard {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

impl ImageLoader for MarkdownImageLoader {
    fn id(&self) -> &str {
        "tools_box::note_taker::MarkdownImageLoader"
    }

    fn load(&self, ctx: &Context, uri: &str, _size_hint: SizeHint) -> ImageLoadResult {
        if !is_supported_markdown_image_uri(uri) {
            return Err(LoadError::NotSupported);
        }

        let mut cache = self
            .cache
            .lock()
            .map_err(|_| LoadError::Loading("图片缓存锁已损坏".to_string()))?;
        let request_id = self.next_request_id.fetch_add(1, Ordering::Relaxed);
        let is_retry = matches!(
            cache.get(uri),
            Some(MarkdownImageState::Failed { retry_at, .. }) if *retry_at <= Instant::now()
        );
        let needs_request = cache.get(uri).is_none() || is_retry;
        if cache.get(uri).is_none() {
            trim_markdown_image_entries(&mut cache, uri);
            if cache.len() >= MAX_MARKDOWN_IMAGE_CACHE_ENTRIES {
                return Err(LoadError::Loading("图片缓存正在处理其他请求".to_string()));
            }
        }
        if needs_request && !self.try_acquire_image_slot() {
            return Err(LoadError::Loading("图片并发加载数量已达上限".to_string()));
        }
        if let Some(state) = cache.get_mut(uri) {
            match state {
                MarkdownImageState::Pending(_) => return Ok(ImagePoll::Pending { size: None }),
                MarkdownImageState::Ready(image) => {
                    return Ok(ImagePoll::Ready {
                        image: Arc::clone(image),
                    });
                }
                MarkdownImageState::Failed { error, retry_at } if *retry_at > Instant::now() => {
                    return Err(LoadError::Loading(error.clone()));
                }
                MarkdownImageState::Failed { .. } => {
                    *state = MarkdownImageState::Pending(request_id);
                }
            }
        } else {
            cache.insert(uri.to_owned(), MarkdownImageState::Pending(request_id));
        }
        drop(cache);

        let cache = Arc::clone(&self.cache);
        let active_requests = Arc::clone(&self.active_requests);
        let image_uri = uri.to_owned();
        let repaint_context = ctx.clone();
        let spawn_result = thread::Builder::new()
            .name("markdown-image-loader".to_string())
            .spawn(move || {
                let _active_request_guard = ActiveImageRequestGuard(active_requests);
                let result = load_markdown_image(&image_uri).map(Arc::new);
                if let Ok(mut cache) = cache.lock() {
                    let is_current_request = matches!(
                        cache.get(&image_uri),
                        Some(MarkdownImageState::Pending(current_id))
                            if *current_id == request_id
                    );
                    if is_current_request {
                        let state = match result {
                            Ok(image) => {
                                trim_markdown_image_cache(&mut cache, &image_uri, &image);
                                MarkdownImageState::Ready(image)
                            }
                            Err(error) => MarkdownImageState::Failed {
                                error,
                                retry_at: Instant::now() + Duration::from_secs(30),
                            },
                        };
                        cache.insert(image_uri, state);
                    }
                }
                repaint_context.request_repaint();
            });

        if let Err(error) = spawn_result {
            let message = format!("启动图片加载线程失败: {error}");
            self.active_requests.fetch_sub(1, Ordering::AcqRel);
            if let Ok(mut cache) = self.cache.lock() {
                if matches!(
                    cache.get(uri),
                    Some(MarkdownImageState::Pending(current_id))
                        if *current_id == request_id
                ) {
                    cache.insert(
                        uri.to_owned(),
                        MarkdownImageState::Failed {
                            error: message.clone(),
                            retry_at: Instant::now() + Duration::from_secs(30),
                        },
                    );
                }
            }
            return Err(LoadError::Loading(message));
        }

        Ok(ImagePoll::Pending { size: None })
    }

    fn forget(&self, uri: &str) {
        if let Ok(mut cache) = self.cache.lock() {
            cache.remove(uri);
        }
    }

    fn forget_all(&self) {
        if let Ok(mut cache) = self.cache.lock() {
            cache.clear();
        }
    }

    fn byte_size(&self) -> usize {
        self.cache
            .lock()
            .map(|cache| {
                cache
                    .values()
                    .map(|state| match state {
                        MarkdownImageState::Ready(image) => markdown_image_size(image),
                        MarkdownImageState::Failed { error, .. } => error.len(),
                        MarkdownImageState::Pending(_) => 0,
                    })
                    .sum()
            })
            .unwrap_or(0)
    }
}

impl MarkdownImageLoader {
    fn try_acquire_image_slot(&self) -> bool {
        let mut active = self.active_requests.load(Ordering::Acquire);
        loop {
            if active >= MAX_MARKDOWN_IMAGE_ACTIVE {
                return false;
            }
            match self.active_requests.compare_exchange(
                active,
                active + 1,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => return true,
                Err(current) => active = current,
            }
        }
    }
}

fn markdown_image_size(image: &egui::ColorImage) -> usize {
    image.pixels.len() * std::mem::size_of::<egui::Color32>()
}

fn trim_markdown_image_cache(
    cache: &mut HashMap<String, MarkdownImageState>,
    current_uri: &str,
    incoming_image: &egui::ColorImage,
) {
    let incoming_size = markdown_image_size(incoming_image);
    trim_markdown_image_entries(cache, current_uri);
    while cache_size(cache).saturating_add(incoming_size) > MAX_MARKDOWN_IMAGE_CACHE_BYTES
        && cache.len() > 1
    {
        let Some(key) = cache.iter().find_map(|(key, state)| {
            if key != current_uri && !matches!(state, MarkdownImageState::Pending(_)) {
                Some(key.clone())
            } else {
                None
            }
        }) else {
            break;
        };
        cache.remove(&key);
    }
}

fn trim_markdown_image_entries(cache: &mut HashMap<String, MarkdownImageState>, current_uri: &str) {
    while cache.len() >= MAX_MARKDOWN_IMAGE_CACHE_ENTRIES {
        let Some(key) = cache.iter().find_map(|(key, state)| {
            if key != current_uri && !matches!(state, MarkdownImageState::Pending(_)) {
                Some(key.clone())
            } else {
                None
            }
        }) else {
            break;
        };
        cache.remove(&key);
    }
}

fn cache_size(cache: &HashMap<String, MarkdownImageState>) -> usize {
    cache
        .values()
        .map(|state| match state {
            MarkdownImageState::Ready(image) => markdown_image_size(image),
            MarkdownImageState::Failed { error, .. } => error.len(),
            MarkdownImageState::Pending(_) => 0,
        })
        .sum()
}

fn is_supported_markdown_image_uri(uri: &str) -> bool {
    uri.get(..7)
        .map(|scheme| scheme.eq_ignore_ascii_case("file://"))
        .unwrap_or(false)
        || uri
            .get(..7)
            .map(|scheme| scheme.eq_ignore_ascii_case("http://"))
            .unwrap_or(false)
        || uri
            .get(..8)
            .map(|scheme| scheme.eq_ignore_ascii_case("https://"))
            .unwrap_or(false)
}

fn load_markdown_image(uri: &str) -> Result<egui::ColorImage, String> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| format!("创建图片加载运行时失败: {error}"))?;
    runtime.block_on(load_markdown_image_async(uri))
}

async fn load_markdown_image_async(uri: &str) -> Result<egui::ColorImage, String> {
    let bytes = if uri
        .get(..7)
        .map(|scheme| scheme.eq_ignore_ascii_case("http://"))
        .unwrap_or(false)
        || uri
            .get(..8)
            .map(|scheme| scheme.eq_ignore_ascii_case("https://"))
            .unwrap_or(false)
    {
        let client = reqwest::Client::builder()
            .timeout(MARKDOWN_IMAGE_LOAD_TIMEOUT)
            .build()
            .map_err(|error| format!("创建图片 HTTP 客户端失败: {error}"))?;
        let mut response =
            tokio::time::timeout(MARKDOWN_IMAGE_LOAD_TIMEOUT, client.get(uri).send())
                .await
                .map_err(|_| "下载图片超时".to_string())?
                .map_err(|error| format!("下载图片失败: {error}"))?
                .error_for_status()
                .map_err(|error| format!("图片 HTTP 响应失败: {error}"))?;
        if response.content_length().unwrap_or(0) > MAX_MARKDOWN_IMAGE_BYTES {
            return Err("图片大小超过 20 MB 限制".to_string());
        }
        let mut bytes = Vec::new();
        loop {
            let chunk = tokio::time::timeout(MARKDOWN_IMAGE_LOAD_TIMEOUT, response.chunk())
                .await
                .map_err(|_| "读取图片响应超时".to_string())?
                .map_err(|error| format!("读取图片响应失败: {error}"))?;
            let Some(chunk) = chunk else {
                break;
            };
            let chunk_size = u64::try_from(chunk.len()).unwrap_or(u64::MAX);
            let current_size = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
            if current_size.saturating_add(chunk_size) > MAX_MARKDOWN_IMAGE_BYTES {
                return Err("图片大小超过 20 MB 限制".to_string());
            }
            bytes.extend_from_slice(&chunk);
        }
        bytes
    } else {
        let path = markdown_file_path(uri);
        let metadata =
            tokio::time::timeout(MARKDOWN_IMAGE_LOAD_TIMEOUT, tokio::fs::metadata(&path))
                .await
                .map_err(|_| "读取图片文件信息超时".to_string())?
                .map_err(|error| format!("读取图片文件信息失败: {}: {error}", path.display()))?;
        if metadata.len() > MAX_MARKDOWN_IMAGE_BYTES {
            return Err("图片大小超过 20 MB 限制".to_string());
        }
        let mut file =
            tokio::time::timeout(MARKDOWN_IMAGE_LOAD_TIMEOUT, tokio::fs::File::open(&path))
                .await
                .map_err(|_| "打开图片文件超时".to_string())?
                .map_err(|error| format!("打开图片文件失败: {}: {error}", path.display()))?;
        let mut bytes = Vec::new();
        let mut buffer = vec![0_u8; 8192];
        loop {
            let read_size =
                tokio::time::timeout(MARKDOWN_IMAGE_LOAD_TIMEOUT, file.read(&mut buffer))
                    .await
                    .map_err(|_| "读取图片文件超时".to_string())?
                    .map_err(|error| format!("读取图片文件失败: {}: {error}", path.display()))?;
            if read_size == 0 {
                break;
            }

            let current_size = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
            let read_size_u64 = u64::try_from(read_size).unwrap_or(u64::MAX);
            if current_size.saturating_add(read_size_u64) > MAX_MARKDOWN_IMAGE_BYTES {
                return Err("图片大小超过 20 MB 限制".to_string());
            }
            bytes.extend_from_slice(&buffer[..read_size]);
        }
        bytes
    };

    let mut reader = image::ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|error| format!("识别图片格式失败: {error}"))?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_MARKDOWN_IMAGE_DIMENSION);
    limits.max_image_height = Some(MAX_MARKDOWN_IMAGE_DIMENSION);
    limits.max_alloc = Some(u64::try_from(MAX_MARKDOWN_IMAGE_CACHE_BYTES).unwrap_or(u64::MAX));
    reader.limits(limits);
    let decoded = reader
        .decode()
        .map_err(|error| format!("解码图片失败: {error}"))?;
    let width = usize::try_from(decoded.width()).map_err(|_| "图片宽度超出支持范围".to_string())?;
    let height =
        usize::try_from(decoded.height()).map_err(|_| "图片高度超出支持范围".to_string())?;
    let image_size = width
        .checked_mul(height)
        .and_then(|pixels| pixels.checked_mul(std::mem::size_of::<egui::Color32>()))
        .ok_or_else(|| "图片解码后的尺寸超出支持范围".to_string())?;
    if image_size > MAX_MARKDOWN_IMAGE_CACHE_BYTES {
        return Err("图片解码后的尺寸超过 64 MB 限制".to_string());
    }
    let rgba = decoded.to_rgba8();
    Ok(egui::ColorImage::from_rgba_unmultiplied(
        [width, height],
        rgba.as_raw(),
    ))
}

fn markdown_file_path(uri: &str) -> PathBuf {
    let path = if uri.len() >= 7 && uri[..7].eq_ignore_ascii_case("file://") {
        &uri[7..]
    } else {
        uri
    };
    let path = urlencoding::decode(path)
        .map(|decoded| decoded.into_owned())
        .unwrap_or_else(|_| path.to_owned());
    let path = if path.starts_with('/') && path.as_bytes().get(2).copied() == Some(b':') {
        &path[1..]
    } else {
        path.as_str()
    };
    PathBuf::from(path)
}

/// 行内布局缓存的渲染上下文。
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct InlineLayoutContext {
    available_width_bits: u32,
    body_font_size_bits: u32,
    monospace_font_size_bits: u32,
    text_color: [u8; 4],
    strong_text_color: [u8; 4],
    hyperlink_color: [u8; 4],
    code_background_color: [u8; 4],
}

/// 单条行内布局缓存，保存完整输入以避免哈希碰撞返回错误结果。
struct InlineLayoutCacheEntry {
    inlines: Vec<MarkdownInline>,
    style: InlineStyle,
    context: InlineLayoutContext,
    job: LayoutJob,
}

impl InlineLayoutCacheEntry {
    fn matches(
        &self,
        inlines: &[MarkdownInline],
        style: InlineStyle,
        context: InlineLayoutContext,
    ) -> bool {
        self.inlines == inlines && self.style == style && self.context == context
    }
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
const MAX_INLINE_LAYOUT_CACHE_ENTRIES: usize = 256;

#[derive(Clone, Copy, Default, PartialEq)]
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
            parsed_document_cache: None,
            inline_layout_cache: RefCell::new(HashMap::new()),
            code_highlight_cache: RefCell::new(HashMap::new()),
        }
    }

    /// 渲染 Markdown 内容到 egui UI。
    pub fn render(&mut self, ui: &mut Ui, markdown: &str) {
        self.render_with_heading_target(ui, markdown, None);
    }

    /// 渲染 Markdown 内容，并可将指定标题滚动到预览区域顶部。
    pub fn render_with_heading_target(
        &mut self,
        ui: &mut Ui,
        markdown: &str,
        target_heading: Option<usize>,
    ) {
        let document = self.get_cached_document(markdown);
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
        let mut heading_index = 0;
        self.render_blocks(ui, &document.blocks, 0, &mut heading_index, target_heading);
    }

    /// 获取 Markdown 标题目录，复用当前内容的解析缓存。
    pub fn heading_outline(&mut self, markdown: &str) -> Vec<MarkdownHeading> {
        self.get_cached_document(markdown).headings()
    }

    fn get_cached_document(&mut self, markdown: &str) -> Rc<MarkdownDocument> {
        if let Some(cache) = &self.parsed_document_cache {
            if cache.content == markdown {
                return Rc::clone(&cache.document);
            }
        }

        let document = Rc::new(MarkdownDocument::parse(markdown));
        self.parsed_document_cache = Some(ParsedDocumentCache {
            content: markdown.to_owned(),
            document: Rc::clone(&document),
        });
        document
    }

    fn render_blocks(
        &self,
        ui: &mut Ui,
        blocks: &[MarkdownBlock],
        list_depth: usize,
        heading_index: &mut usize,
        target_heading: Option<usize>,
    ) {
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
                    let current_heading_index = *heading_index;
                    *heading_index += 1;
                    let heading_response = self.render_inlines(ui, content, style);
                    if target_heading == Some(current_heading_index) {
                        ui.scroll_to_rect(heading_response.rect, Some(egui::Align::TOP));
                    }
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
                            self.render_blocks(
                                ui,
                                content,
                                list_depth,
                                heading_index,
                                target_heading,
                            );
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
                                self.render_blocks(
                                    ui,
                                    &item.blocks,
                                    list_depth + 1,
                                    heading_index,
                                    target_heading,
                                );
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

    fn render_inlines(
        &self,
        ui: &mut Ui,
        inlines: &[MarkdownInline],
        style: InlineStyle,
    ) -> egui::Response {
        if inlines
            .iter()
            .any(|inline| matches!(inline, MarkdownInline::Image { .. }))
        {
            let mut response: Option<egui::Response> = None;
            let mut text_inlines = Vec::new();

            for inline in inlines {
                let MarkdownInline::Image {
                    destination,
                    title,
                    alt,
                } = inline
                else {
                    text_inlines.push(inline.clone());
                    continue;
                };

                if !text_inlines.is_empty() {
                    let text_response = self.render_text_inlines(ui, &text_inlines, style);
                    response = Some(match response {
                        Some(response) => response.union(text_response),
                        None => text_response,
                    });
                    text_inlines.clear();
                }

                let image_response = self.render_image(ui, destination, title.as_deref(), alt);
                response = Some(match response {
                    Some(response) => response.union(image_response),
                    None => image_response,
                });
            }

            if !text_inlines.is_empty() {
                let text_response = self.render_text_inlines(ui, &text_inlines, style);
                response = Some(match response {
                    Some(response) => response.union(text_response),
                    None => text_response,
                });
            }

            if let Some(response) = response {
                return response;
            }
        }

        self.render_text_inlines(ui, inlines, style)
    }

    fn render_text_inlines(
        &self,
        ui: &mut Ui,
        inlines: &[MarkdownInline],
        style: InlineStyle,
    ) -> egui::Response {
        let context = inline_layout_context(ui);
        let cache_key = inline_layout_cache_key(inlines, style, context);
        if let Some(entry) = self.inline_layout_cache.borrow().get(&cache_key) {
            if entry.matches(inlines, style, context) {
                return ui.add(egui::Label::new(entry.job.clone()).wrap());
            }
        }

        let mut job = LayoutJob::default();
        self.append_inlines_to_job(ui, inlines, style, &mut job);
        let mut cache = self.inline_layout_cache.borrow_mut();
        if !cache.contains_key(&cache_key) && cache.len() >= MAX_INLINE_LAYOUT_CACHE_ENTRIES {
            if let Some(evicted_key) = cache.keys().next().copied() {
                cache.remove(&evicted_key);
            }
        }
        cache.insert(
            cache_key,
            InlineLayoutCacheEntry {
                inlines: inlines.to_vec(),
                style,
                context,
                job: job.clone(),
            },
        );
        ui.add(egui::Label::new(job).wrap())
    }

    fn render_image(
        &self,
        ui: &mut Ui,
        destination: &str,
        title: Option<&str>,
        alt: &str,
    ) -> egui::Response {
        let image_uri = markdown_image_uri(destination);
        let alt_text = if alt.is_empty() { "图片" } else { alt };
        let max_width = ui.available_width().max(1.0);
        let response = ui.add(
            egui::Image::from_uri(image_uri)
                .max_width(max_width)
                .maintain_aspect_ratio(true)
                .alt_text(alt_text),
        );

        if let Some(title) = title.filter(|title| !title.is_empty()) {
            response.on_hover_text(title)
        } else {
            response
        }
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

fn inline_layout_cache_key(
    inlines: &[MarkdownInline],
    style: InlineStyle,
    context: InlineLayoutContext,
) -> u64 {
    let mut hasher = DefaultHasher::new();
    inlines.hash(&mut hasher);
    style.emphasis.hash(&mut hasher);
    style.strong.hash(&mut hasher);
    style.strikethrough.hash(&mut hasher);
    style.link.hash(&mut hasher);
    style.size.map(f32::to_bits).hash(&mut hasher);
    context.hash(&mut hasher);
    hasher.finish()
}

fn inline_layout_context(ui: &Ui) -> InlineLayoutContext {
    let visuals = ui.visuals();
    InlineLayoutContext {
        available_width_bits: ui.available_width().to_bits(),
        body_font_size_bits: body_font_size(ui).to_bits(),
        monospace_font_size_bits: monospace_font_size(ui).to_bits(),
        text_color: visuals.text_color().to_array(),
        strong_text_color: visuals.strong_text_color().to_array(),
        hyperlink_color: visuals.hyperlink_color.to_array(),
        code_background_color: visuals.code_bg_color.to_array(),
    }
}

fn body_font_size(ui: &Ui) -> f32 {
    ui.style()
        .text_styles
        .get(&egui::TextStyle::Body)
        .map(|font_id| font_id.size)
        .unwrap_or(14.0)
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

fn markdown_image_uri(destination: &str) -> String {
    let destination = destination.trim();
    let has_uri_scheme = ["data:", "file:", "http:", "https:", "bytes:"]
        .iter()
        .any(|scheme| {
            destination.len() >= scheme.len()
                && destination
                    .get(..scheme.len())
                    .is_some_and(|prefix| prefix.eq_ignore_ascii_case(scheme))
        });

    if has_uri_scheme {
        destination.to_owned()
    } else {
        format!("file://{destination}")
    }
}

#[cfg(test)]
mod tests {
    use super::{
        InlineLayoutContext, InlineStyle, MarkdownBlock, MarkdownDocument, MarkdownInline,
        MarkdownRenderer, MarkdownTableAlignment, SyntaxHighlighter, code_highlight_cache_key,
        inline_layout_cache_key, markdown_image_uri,
    };

    fn render_markdown_frame(ctx: &egui::Context, renderer: &mut MarkdownRenderer, width: f32) {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(width, 600.0),
            )),
            ..Default::default()
        };
        let _ = ctx.run(input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                renderer.render(ui, "普通文本 **粗体** [链接](https://example.com)");
            });
        });
    }

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
    fn should_collect_heading_outline_in_document_order() {
        let document =
            MarkdownDocument::parse("# 第一章\n\n## 第二节\n\n> ### 引用标题\n\n- #### 列表标题");

        assert_eq!(
            document.headings(),
            vec![
                super::MarkdownHeading {
                    level: 1,
                    title: "第一章".to_string(),
                },
                super::MarkdownHeading {
                    level: 2,
                    title: "第二节".to_string(),
                },
                super::MarkdownHeading {
                    level: 3,
                    title: "引用标题".to_string(),
                },
                super::MarkdownHeading {
                    level: 4,
                    title: "列表标题".to_string(),
                },
            ]
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
    fn should_normalize_relative_image_destination_to_file_uri() {
        assert_eq!(
            markdown_image_uri("images/logo.png"),
            "file://images/logo.png"
        );
        assert_eq!(
            markdown_image_uri("  images/logo.png  "),
            "file://images/logo.png"
        );
        assert_eq!(markdown_image_uri("图片/截图.png"), "file://图片/截图.png");
    }

    #[test]
    fn should_preserve_image_uri_with_supported_scheme() {
        assert_eq!(
            markdown_image_uri("https://example.com/logo.png"),
            "https://example.com/logo.png"
        );
        assert_eq!(
            markdown_image_uri("FILE://C:/images/logo.png"),
            "FILE://C:/images/logo.png"
        );
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
    fn should_reuse_parsed_document_until_content_changes() {
        let mut renderer = MarkdownRenderer::new();
        let first = renderer.get_cached_document("# 标题");
        let same_content = renderer.get_cached_document("# 标题");
        let changed_content = renderer.get_cached_document("## 新标题");

        assert!(std::rc::Rc::ptr_eq(&first, &same_content));
        assert!(!std::rc::Rc::ptr_eq(&same_content, &changed_content));
    }

    #[test]
    fn should_recalculate_inline_layout_when_render_context_changes() {
        let inlines = vec![MarkdownInline::Text("正文".to_string())];
        let style = InlineStyle::default();
        let context = InlineLayoutContext {
            available_width_bits: 800.0_f32.to_bits(),
            body_font_size_bits: 14.0_f32.to_bits(),
            monospace_font_size_bits: 14.0_f32.to_bits(),
            text_color: [220, 220, 220, 255],
            strong_text_color: [255, 255, 255, 255],
            hyperlink_color: [90, 170, 255, 255],
            code_background_color: [64, 64, 64, 255],
        };
        let base = inline_layout_cache_key(&inlines, style, context);

        let mut narrow_context = context;
        narrow_context.available_width_bits = 480.0_f32.to_bits();
        assert_ne!(
            base,
            inline_layout_cache_key(&inlines, style, narrow_context)
        );

        let mut light_theme_context = context;
        light_theme_context.text_color = [30, 30, 30, 255];
        assert_ne!(
            base,
            inline_layout_cache_key(&inlines, style, light_theme_context)
        );

        let mut large_font_context = context;
        large_font_context.body_font_size_bits = 16.0_f32.to_bits();
        large_font_context.monospace_font_size_bits = 16.0_f32.to_bits();
        assert_ne!(
            base,
            inline_layout_cache_key(&inlines, style, large_font_context)
        );

        let mut strong_color_context = context;
        strong_color_context.strong_text_color = [255, 220, 120, 255];
        assert_ne!(
            base,
            inline_layout_cache_key(&inlines, style, strong_color_context)
        );

        let mut hyperlink_color_context = context;
        hyperlink_color_context.hyperlink_color = [80, 220, 160, 255];
        assert_ne!(
            base,
            inline_layout_cache_key(&inlines, style, hyperlink_color_context)
        );

        let mut code_background_context = context;
        code_background_context.code_background_color = [230, 230, 230, 255];
        assert_ne!(
            base,
            inline_layout_cache_key(&inlines, style, code_background_context)
        );

        let strong_style = InlineStyle {
            strong: true,
            ..style
        };
        assert_ne!(
            base,
            inline_layout_cache_key(&inlines, strong_style, context)
        );
    }

    #[test]
    fn should_rebuild_inline_layout_when_width_or_theme_changes() {
        let ctx = egui::Context::default();
        let mut renderer = MarkdownRenderer::new();

        render_markdown_frame(&ctx, &mut renderer, 800.0);
        let initial_entries = renderer.inline_layout_cache.borrow().len();
        assert!(initial_entries > 0);

        render_markdown_frame(&ctx, &mut renderer, 800.0);
        assert_eq!(renderer.inline_layout_cache.borrow().len(), initial_entries);

        render_markdown_frame(&ctx, &mut renderer, 480.0);
        let narrow_entries = renderer.inline_layout_cache.borrow().len();
        assert!(narrow_entries > initial_entries);

        ctx.set_visuals(egui::Visuals::light());
        render_markdown_frame(&ctx, &mut renderer, 480.0);
        let light_theme_entries = renderer.inline_layout_cache.borrow().len();
        assert!(light_theme_entries > narrow_entries);
    }

    #[test]
    fn should_scroll_to_selected_heading() {
        let ctx = egui::Context::default();
        let mut renderer = MarkdownRenderer::new();
        let markdown =
            "# 第一标题\n\n第一段内容\n\n第二段内容\n\n第三段内容\n\n# 目标标题\n\n目标内容";
        let mut scroll_offset = 0.0;

        for _ in 0..2 {
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(600.0, 120.0),
                )),
                ..Default::default()
            };
            let _ = ctx.run(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let output = egui::ScrollArea::vertical()
                        .id_salt("markdown_heading_navigation_test")
                        .max_height(100.0)
                        .show(ui, |ui| {
                            renderer.render_with_heading_target(ui, markdown, Some(1));
                        });
                    scroll_offset = output.state.offset.y;
                });
            });
        }

        assert!(scroll_offset > 0.0);
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
