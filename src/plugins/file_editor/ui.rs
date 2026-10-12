//! 常用文件编辑器界面渲染
//!
//! 左侧为目录 / 常用文件列表（分隔线可拖动调宽），右侧为多标签文本编辑器。
//! 所有交互（打开、保存、关闭、重载）都通过渲染期间收集的延迟动作执行，
//! 避免 egui 渲染过程中的借用冲突。

use std::path::{Path, PathBuf};
use std::sync::Arc;

use egui::text::LayoutJob;
use egui::{Context, RichText, Ui};
use rusqlite::Connection;

use crate::utils::highlight::SyntaxHighlighter;

use super::document;
use super::{models, store};

/// 左侧列表最小宽度（像素）
const LIST_MIN_WIDTH: f32 = 180.0;
/// 左侧列表最大宽度（像素）
const LIST_MAX_WIDTH: f32 = 520.0;
/// 标签栏、列表等控件的唯一标识前缀
const LIST_PANEL_ID: &str = "file_editor_list";
/// 编辑器在缺少可用高度时的默认行数（撑开滚动区）
const EDITOR_DESIRED_ROWS: usize = 24;
/// 编辑区最少显示行数（可用高度很小时避免塌缩）
const MIN_EDITOR_ROWS: f32 = 6.0;
/// 编辑区最多显示行数（可用高度异常大时的保护上限）
const MAX_EDITOR_ROWS: f32 = 400.0;
/// 参与语法着色的最大文本长度（字节）：超过后按纯文本布局，避免拖慢输入
const HIGHLIGHT_MAX_BYTES: usize = 128 * 1024;
/// 单帧最多高亮的匹配数量（极端情况下避免绘制过多矩形）
const MAX_HIGHLIGHTED_MATCHES: usize = 1000;

/// 当前打开的弹窗
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Dialog {
    /// 无弹窗
    None,
    /// 添加常用文件
    AddFile,
    /// 重命名显示名
    RenameFile,
    /// 目录管理
    ManageDirectories,
}

/// 添加文件表单
#[derive(Debug, Clone, Default)]
struct AddFileForm {
    /// 文件路径输入
    path: String,
    /// 显示名输入
    alias: String,
    /// 所属目录
    directory_id: Option<i64>,
    /// 校验错误提示
    error: Option<String>,
}

/// 目录管理表单
#[derive(Debug, Clone, Default)]
struct DirectoryForm {
    /// 新建目录名称
    new_name: String,
    /// 正在重命名的目录 id
    rename_target: Option<i64>,
    /// 重命名输入
    rename_name: String,
    /// 校验错误提示
    error: Option<String>,
}

/// 渲染期间收集、渲染后统一执行的延迟动作
#[derive(Debug, Clone)]
enum Action {
    /// 打开文件（必要时新建标签）
    Open {
        file_id: i64,
        path: PathBuf,
        display_name: String,
    },
    /// 激活标签
    ActivateTab(usize),
    /// 请求关闭标签（脏标签会先弹确认框）
    CloseTab(usize),
    /// 保存当前标签
    SaveActive,
    /// 另存为：写入用户选择的新路径并切换标签到新文件
    SaveAsActive,
    /// 展开 / 收起查找替换栏
    ToggleFind,
    /// 查找下一个匹配
    FindNext,
    /// 查找上一个匹配
    FindPrev,
    /// 替换当前匹配
    ReplaceOne,
    /// 替换全部匹配
    ReplaceAll,
    /// 放弃修改并重新读取当前标签
    ReloadActive,
    /// 从常用列表移除条目
    RemoveFile(i64),
    /// 在资源管理器中显示
    Reveal(PathBuf),
    /// 打开添加文件弹窗
    StartAddFile(Option<i64>),
    /// 打开重命名显示名弹窗（条目 id、当前显示名）
    StartRenameFile(i64, Option<String>),
    /// 打开目录管理弹窗
    StartManageDirectories,
    /// 关闭确认弹窗的选择结果
    FinishClose(models::CloseDecision),
}

/// 关闭某个标签后应激活的标签下标
///
/// 关闭的是当前标签时优先激活右邻标签，没有右邻则回到最后一个标签。
fn active_after_close(active: Option<usize>, closed: usize, remaining: usize) -> Option<usize> {
    if remaining == 0 {
        return None;
    }

    match active {
        Some(current) if current < closed => Some(current),
        Some(current) if current > closed => Some(current - 1),
        _ => Some(closed.min(remaining - 1)),
    }
}

/// 是否应该把当前左栏宽度写回数据库
///
/// 拖动过程中（指针按下）只更新内存宽度，指针抬起后再落库，避免每帧写库；
/// 比较对象是"上次已保存的宽度"，因此松手那一帧即使宽度不再变化也会触发保存。
fn should_persist_width(pointer_down: bool, current: f32, last_saved: f32) -> bool {
    !pointer_down && (current - last_saved).abs() >= 0.5
}

/// 该长度的文本是否参与语法着色
fn should_highlight_text(len: usize) -> bool {
    len <= HIGHLIGHT_MAX_BYTES
}

/// 一次查找命中的位置（同时给出字节与字符区间）
///
/// 字节区间用于文本切片，字符区间用于设置编辑器光标（`CCursor` 按字符计数）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct MatchRange {
    /// 起始字节偏移
    start_byte: usize,
    /// 结束字节偏移（不含）
    end_byte: usize,
    /// 起始字符下标
    start_char: usize,
    /// 结束字符下标（不含）
    end_char: usize,
}

/// 判断 `byte` 处是否匹配 `query`，返回命中的结束字节偏移
///
/// `byte` 必须是字符边界（调用方均来自 `char_indices()` 或既有命中位置）。
fn match_at(text: &str, byte: usize, query_chars: &[char], match_case: bool) -> Option<usize> {
    let mut end = byte;
    let mut chars = text[byte..].chars();

    for expected in query_chars {
        let actual = chars.next()?;
        let equal = if match_case {
            actual == *expected
        } else if actual.is_ascii() && expected.is_ascii() {
            // ASCII 快路径：避免每字符两次 to_lowercase 折叠（大文本查找会明显变慢）
            actual.eq_ignore_ascii_case(expected)
        } else {
            actual.to_lowercase().eq(expected.to_lowercase())
        };
        if !equal {
            return None;
        }
        end += actual.len_utf8();
    }

    Some(end)
}

/// 收集文本中的全部命中（按出现顺序，不重叠）
///
/// 查找栏需要"匹配总数"与"高亮所有匹配"，因此一次扫描出全部命中并缓存；
/// 命中区间与 [`match_at`] 使用同一套比较规则，保证与替换行为一致。
///
/// 字符下标在扫描过程中**递推**得到（`char_index`）：若对每个命中回头执行
/// `text[..byte].chars().count()`，复杂度会退化为 O(命中数 × 文本长度)，
/// 在 2 MB 文本 + 单字符查询下会造成秒级卡顿。
fn find_all(text: &str, query: &str, match_case: bool) -> Vec<MatchRange> {
    if query.is_empty() {
        return Vec::new();
    }
    let query_chars: Vec<char> = query.chars().collect();
    let mut matches = Vec::new();
    let mut cursor = 0;

    for (char_index, (byte, _)) in text.char_indices().enumerate() {
        if byte >= cursor
            && let Some(end) = match_at(text, byte, &query_chars, match_case)
        {
            matches.push(MatchRange {
                start_byte: byte,
                end_byte: end,
                start_char: char_index,
                // 一次命中恰好覆盖查询的字符数
                end_char: char_index + query_chars.len(),
            });
            cursor = end;
        }
    }

    matches
}

/// 全部替换（不重叠），返回新文本与替换次数
fn replace_all(text: &str, query: &str, replacement: &str, match_case: bool) -> (String, usize) {
    if query.is_empty() {
        return (text.to_string(), 0);
    }
    let query_chars: Vec<char> = query.chars().collect();
    let mut result = String::with_capacity(text.len());
    let mut cursor = 0;
    let mut count = 0;

    for (byte, _) in text.char_indices() {
        if byte < cursor {
            continue;
        }
        if let Some(end) = match_at(text, byte, &query_chars, match_case) {
            result.push_str(&text[cursor..byte]);
            result.push_str(replacement);
            cursor = end;
            count += 1;
        }
    }

    result.push_str(&text[cursor..]);
    (result, count)
}

/// 判断一段文本是否与查找内容完全一致
fn matches_query(slice: &str, query: &str, match_case: bool) -> bool {
    let query_chars: Vec<char> = query.chars().collect();
    match_at(slice, 0, &query_chars, match_case) == Some(slice.len())
}

/// 计算字节偏移所在的行号（从 1 开始）
///
/// 偏移不是字符边界或超出文本长度时按文本末尾处理，不会 panic。
fn line_number(text: &str, byte: usize) -> usize {
    let end = byte.min(text.len());
    let prefix = text.get(..end).unwrap_or(text);
    prefix.matches('\n').count() + 1
}

/// 编辑区应显示的行数：按可用高度换算，让编辑器占满可见区域
///
/// `f32 → usize` 没有标准转换实现，这里先把结果夹取到
/// `[MIN_EDITOR_ROWS, MAX_EDITOR_ROWS]` 的安全整数区间再取整，
/// 因此不存在截断出异常值或符号问题的可能。
fn editor_rows(available_height: f32, row_height: f32) -> usize {
    if !row_height.is_finite()
        || row_height <= 0.0
        || !available_height.is_finite()
        || available_height <= 0.0
    {
        return EDITOR_DESIRED_ROWS;
    }

    let rows = (available_height / row_height)
        .floor()
        .clamp(MIN_EDITOR_ROWS, MAX_EDITOR_ROWS);
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let rows = rows as usize;

    rows
}

/// 按最近打开时间排序（未打开过的排在最后，其次按路径排序保证稳定）
fn sort_files_by_recent(files: &mut [models::FavoriteFile]) {
    files.sort_by(
        |left, right| match (&right.last_opened_at, &left.last_opened_at) {
            (Some(right_time), Some(left_time)) => right_time
                .cmp(left_time)
                .then_with(|| left.path.cmp(&right.path)),
            (Some(_), None) => std::cmp::Ordering::Greater,
            (None, Some(_)) => std::cmp::Ordering::Less,
            (None, None) => left.path.cmp(&right.path),
        },
    );
}

/// 另存为成功后更新标签的路径与显示名
///
/// 只有目标路径与原路径不同时才解除与原列表条目的关联，
/// 否则"另存为到原路径"会让点列表条目再开一个重复标签。
fn apply_saved_as(tab: &mut models::EditorTab, target: &Path, disk: models::DiskState) {
    let path_changed = tab.path != target;

    tab.display_name = target
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| target.display().to_string());
    tab.path = target.to_path_buf();
    if path_changed {
        tab.file_id = None;
    }
    tab.disk = disk;
    tab.dirty = false;
    tab.message = None;
}

/// 纯文本（不着色）布局
fn plain_layout_job(text: &str, font_size: f32) -> LayoutJob {
    let mut job = LayoutJob::default();
    job.append(
        text,
        0.0,
        egui::TextFormat {
            font_id: egui::FontId::monospace(font_size),
            ..Default::default()
        },
    );
    job
}

/// `Ctrl+F` 是否应由本插件接管
///
/// 该组合键同时是 app 级的"聚焦插件搜索框"快捷键，约定：
/// - 编辑器有焦点（正在编辑文件）或查找栏已打开时由本插件接管；
/// - 焦点在**其它文本输入框**（全局搜索框、别的插件的输入框）时不接管，留给全局搜索；
/// - 焦点不在任何输入上（例如刚点过左侧文件列表）时也接管——此时用户显然在本插件里操作。
///
/// `editor_had_focus` 必须传**上一帧**的状态：app 的 `handle_shortcuts` 在本插件之前执行，
/// 且 egui 的 `request_focus` 当帧立即生效——按 Ctrl+F 的那一帧，焦点已被全局搜索框抢走，
/// 用当帧状态判断会永远判否，查找栏就打不开了。
fn should_capture_find_shortcut(
    editor_had_focus: bool,
    find_visible: bool,
    other_text_input_focused: bool,
) -> bool {
    editor_had_focus || find_visible || !other_text_input_focused
}

/// 一次布局除文本内容外的参数（与文本内容一起构成缓存键）
///
/// 编辑器不做自动换行（长行交给横向滚动条），因此布局宽度恒为无穷大、无需入键。
#[derive(Debug, Clone, Copy, PartialEq)]
struct LayoutParams<'a> {
    /// 高亮语言（`None` 为纯文本）
    language: Option<&'a str>,
    /// 等宽字号
    font_size: f32,
    /// 是否暗色主题
    dark: bool,
}

/// 编辑器语法高亮缓存
///
/// 缓存**布局任务**（`LayoutJob`）而不是布局结果（`Galley`）：
/// syntect 的高亮结果被复用，而 `Galley` 的失效（缩放比例变化、字形图集重建等）
/// 交给 epaint 的字体系统按它自己的规则处理——在外部缓存 `Galley` 会绕过这套失效契约，
/// 把窗口拖到不同缩放比例的显示器后可能出现文字错位。
///
/// 缓存键包含**文本内容本身**（而不是版本计数器）：编辑、重载或切换到内容相同的标签
/// 都能得到与当前文本一致的结果；egui 会把 layouter 的返回值直接用于绘制与光标定位，
/// 键里缺少文本身份会导致显示错内容或光标错位。
#[derive(Default)]
struct HighlightJobCache {
    /// 上次高亮的文本内容
    text: String,
    /// 上次高亮的参数
    params: Option<LayoutParamsOwned>,
    /// 缓存的布局任务
    job: Option<LayoutJob>,
    /// 实际执行高亮的次数（仅供测试断言缓存命中）
    #[cfg(test)]
    highlights: u32,
}

/// 缓存中保存的布局参数（拥有语言字符串）
#[derive(Debug, Clone, PartialEq)]
struct LayoutParamsOwned {
    /// 高亮语言
    language: Option<String>,
    /// 等宽字号
    font_size: f32,
    /// 是否暗色主题
    dark: bool,
}

impl LayoutParamsOwned {
    /// 与本次布局参数是否一致
    fn matches(&self, params: LayoutParams<'_>) -> bool {
        self.language.as_deref() == params.language
            && (self.font_size - params.font_size).abs() <= f32::EPSILON
            && self.dark == params.dark
    }
}

impl HighlightJobCache {
    /// 取得与当前文本和参数匹配的布局任务，必要时重新高亮
    fn job(
        &mut self,
        highlighter: &SyntaxHighlighter,
        text: &str,
        params: LayoutParams<'_>,
    ) -> LayoutJob {
        if let Some(job) = &self.job {
            // 先比较廉价参数（长度不同立即判为未命中），再比较文本内容
            if self.text.len() == text.len()
                && self
                    .params
                    .as_ref()
                    .is_some_and(|saved| saved.matches(params))
                && self.text == text
            {
                return job.clone();
            }
        }

        let mut job = if should_highlight_text(text.len()) {
            highlighter.highlight_to_layout_job(
                text,
                params.language,
                params.font_size,
                params.dark,
            )
        } else {
            plain_layout_job(text, params.font_size)
        };
        // 不自动换行：长行保持单行，由外层 ScrollArea 提供横向滚动条
        job.wrap.max_width = f32::INFINITY;

        self.job = Some(job.clone());
        self.text.clear();
        self.text.push_str(text);
        self.params = Some(LayoutParamsOwned {
            language: params.language.map(str::to_string),
            font_size: params.font_size,
            dark: params.dark,
        });
        #[cfg(test)]
        {
            self.highlights += 1;
        }

        job
    }
}

/// 编辑器的固定标识（查找替换需要读写它的光标状态）
const EDITOR_ID_SALT: &str = "file_editor_editor";
/// 查找输入框的固定标识（需要在渲染它之前判断焦点并消费 ↑/↓/回车）
const FIND_QUERY_ID_SALT: &str = "file_editor_find_query";
/// 查找命中高亮的底色
///
/// 用低透明度底色 + 描边：高亮叠在语法着色文字之上，
/// 底色太实（或用预乘 alpha 传非预乘分量）都会把正文压得看不清。
fn find_highlight_fill(dark: bool) -> egui::Color32 {
    if dark {
        egui::Color32::from_rgba_unmultiplied(0xE0, 0xC0, 0x40, 0x38)
    } else {
        egui::Color32::from_rgba_unmultiplied(0xB0, 0x7A, 0x00, 0x2E)
    }
}

/// 查找命中高亮的描边（低透明度底色下依然能一眼看到命中范围）
fn find_highlight_stroke(dark: bool) -> egui::Color32 {
    if dark {
        egui::Color32::from_rgb(0xE0, 0xC0, 0x40)
    } else {
        egui::Color32::from_rgb(0xA0, 0x6A, 0x00)
    }
}

/// 非当前命中项的底色（比当前项更淡，用于"高亮所有匹配"）
fn find_other_fill(dark: bool) -> egui::Color32 {
    if dark {
        egui::Color32::from_rgba_unmultiplied(0xE0, 0xC0, 0x40, 0x1F)
    } else {
        egui::Color32::from_rgba_unmultiplied(0xB0, 0x7A, 0x00, 0x1A)
    }
}

/// 命中区间在当前布局中的矩形（与光标定位共用同一套字符下标）
fn match_rect(galley: &egui::Galley, range: MatchRange) -> egui::Rect {
    let start = galley.pos_from_ccursor(egui::text::CCursor::new(range.start_char));
    let end = galley.pos_from_ccursor(egui::text::CCursor::new(range.end_char));

    start.union(end)
}

/// 查找 / 替换状态
#[derive(Default)]
struct FindState {
    /// 是否展开查找栏
    visible: bool,
    /// 查找内容
    query: String,
    /// 替换内容
    replace: String,
    /// 是否区分大小写
    match_case: bool,
    /// 当前命中（用于替换与"替换全部"的起点计算）
    current: Option<MatchRange>,
    /// 下一次查找的起始字节偏移（替换后从这里继续）
    anchor: Option<usize>,
    /// 上一次动作是替换（用于把"已替换"与"下一处命中"合并成一条提示）
    replaced_once: bool,
    /// 待应用到编辑器的选区（字符下标区间）
    pending_selection: Option<(usize, usize)>,
    /// 是否需要把选区滚动到可见区域（egui 只在交互改变选区时自动滚动）
    pending_scroll: bool,
    /// 是否需要把键盘焦点交给查找输入框（Ctrl+F 打开或工具条打开时置位）
    focus_requested: bool,
    /// 结果提示
    message: String,
    /// 全部命中（按出现顺序），用于匹配总数、序号与高亮所有匹配
    matches: Vec<MatchRange>,
    /// 命中列表对应的查找内容
    matches_query: String,
    /// 命中列表对应的大小写规则
    matches_match_case: bool,
    /// 命中列表是否已失效（文本被编辑、替换、重载后置位）
    matches_stale: bool,
}

impl FindState {
    /// 按需重算命中列表（文本、查找内容或大小写规则变化时）
    fn ensure_matches(&mut self, text: &str) {
        if !self.matches_stale
            && self.matches_query == self.query
            && self.matches_match_case == self.match_case
        {
            return;
        }

        self.matches = find_all(text, &self.query, self.match_case);
        self.matches_query.clear();
        self.matches_query.push_str(&self.query);
        self.matches_match_case = self.match_case;
        self.matches_stale = false;
    }

    /// 把命中列表标记为失效（下一次查找时重算）
    fn invalidate_matches(&mut self) {
        self.matches_stale = true;
    }

    /// 当前命中在命中列表中的序号（从 1 开始）
    ///
    /// 命中列表按位置有序，用二分查找避免每帧线性扫描。
    fn ordinal(&self) -> Option<usize> {
        self.index_of(self.current?).map(|index| index + 1)
    }

    /// 命中在列表中的下标（列表有序，二分查找）
    fn index_of(&self, range: MatchRange) -> Option<usize> {
        let index = self
            .matches
            .partition_point(|found| found.start_char < range.start_char);
        self.matches
            .get(index)
            .filter(|found| found.start_char == range.start_char)
            .map(|_| index)
    }
}

/// 常用文件编辑器界面状态
pub struct FileEditorUi {
    /// 自定义目录
    directories: Vec<models::FileDirectory>,
    /// 常用文件条目
    files: Vec<models::FavoriteFile>,
    /// 磁盘上已不存在的条目 id
    missing_files: Vec<i64>,
    /// 已打开的标签
    tabs: Vec<models::EditorTab>,
    /// 当前激活标签
    active_tab: Option<usize>,
    /// 列表选中条目
    selected_file_id: Option<i64>,
    /// 待确认关闭的标签下标
    pending_close: Option<usize>,
    /// 当前弹窗
    dialog: Dialog,
    /// 添加文件表单
    add_form: AddFileForm,
    /// 重命名显示名表单（条目 id、输入内容）
    rename_form: Option<(i64, String)>,
    /// 目录管理表单
    dir_form: DirectoryForm,
    /// 左栏宽度
    list_width: f32,
    /// 最近一次写入数据库的左栏宽度
    last_saved_width: f32,
    /// 状态栏文本
    status: String,
    /// 数据库是否可写（只读时在顶部显示警告）
    storage_writable: bool,
    /// 语法高亮器（首次进入插件时懒加载，避免拖慢程序启动）
    highlighter: Option<SyntaxHighlighter>,
    /// 编辑器语法高亮缓存（缓存最近一次的 LayoutJob）
    highlight_job_cache: HighlightJobCache,
    /// 查找 / 替换状态
    find: FindState,
    /// 编辑器的持久 Id（查找结果要读写它的光标状态；同时便于测试断言）
    editor_id: Option<egui::Id>,
    /// 上一帧编辑器是否持有键盘焦点（Ctrl+F 判定必须用它，见 `should_capture_find_shortcut`）
    editor_had_focus: bool,
    /// 列表是否按最近打开时间排序
    sort_by_recent: bool,
    /// 上一帧窗口是否处于焦点（用于重新获得焦点时检查外部修改）
    was_focused: bool,
    /// 是否已从数据库加载
    initialized: bool,
}

impl FileEditorUi {
    /// 创建界面状态
    pub fn new() -> Self {
        Self {
            directories: Vec::new(),
            files: Vec::new(),
            missing_files: Vec::new(),
            tabs: Vec::new(),
            active_tab: None,
            selected_file_id: None,
            pending_close: None,
            dialog: Dialog::None,
            add_form: AddFileForm::default(),
            rename_form: None,
            dir_form: DirectoryForm::default(),
            list_width: store::DEFAULT_LIST_WIDTH,
            last_saved_width: store::DEFAULT_LIST_WIDTH,
            status: "就绪".to_string(),
            storage_writable: true,
            highlighter: None,
            highlight_job_cache: HighlightJobCache::default(),
            find: FindState::default(),
            editor_id: None,
            editor_had_focus: false,
            sort_by_recent: false,
            was_focused: false,
            initialized: false,
        }
    }

    /// 首次进入插件时加载数据库中的目录、文件条目与界面宽度
    pub fn init(&mut self, conn: &Connection) {
        self.reload_data(conn);

        match store::load_list_width(conn) {
            Ok(width) => {
                self.list_width = width;
                self.last_saved_width = width;
            }
            Err(error) => log::warn!("读取列表宽度失败，使用默认值: {}", error),
        }

        self.initialized = true;
    }

    /// 渲染插件界面
    pub fn render(&mut self, ui: &mut Ui, conn: &Connection) {
        let mut actions = Vec::new();

        if !self.storage_writable {
            self.render_storage_warning(ui);
        }

        // 窗口重新获得焦点时检查活动标签的磁盘状态：长时间停留同一标签也能发现外部修改
        let focused = ui
            .ctx()
            .input(|input| input.viewport().focused)
            .unwrap_or(false);
        if focused
            && !self.was_focused
            && let Some(index) = self.active_tab.filter(|index| *index < self.tabs.len())
        {
            self.check_external_change(index);
        }
        self.was_focused = focused;

        if self.can_save_active_tab() && self.save_shortcut_pressed(ui.ctx()) {
            actions.push(Action::SaveActive);
        }

        // Ctrl+F：与 app 级"聚焦插件搜索框"共用该快捷键。
        // 判定用**上一帧**的编辑器焦点：app 的 handle_shortcuts 在本插件之前执行，
        // 且 egui 的 request_focus 当帧立即生效——按键这一帧焦点已被搜索框抢走。
        // 接管后把焦点请求交给查找输入框（本插件渲染更晚，最后一次请求生效）。
        let editor_focused_now = self
            .editor_id
            .is_some_and(|id| ui.ctx().memory(|memory| memory.has_focus(id)));
        let other_text_input_focused =
            ui.ctx().wants_keyboard_input() && !self.editor_had_focus && !self.find.visible;
        if should_capture_find_shortcut(
            self.editor_had_focus,
            self.find.visible,
            other_text_input_focused,
        ) && ui
            .ctx()
            .input_mut(|input| input.consume_key(egui::Modifiers::CTRL, egui::Key::F))
        {
            self.find.visible = true;
            self.find.focus_requested = true;
            log::info!(
                "常用文件编辑器：Ctrl+F 打开查找栏（上一帧编辑器焦点={}, 其它输入框焦点={other_text_input_focused}）",
                self.editor_had_focus
            );
        }
        self.editor_had_focus = editor_focused_now;

        self.render_toolbar(ui, &mut actions);
        ui.separator();

        let panel = egui::SidePanel::left(LIST_PANEL_ID)
            .resizable(true)
            .default_width(self.list_width)
            .width_range(LIST_MIN_WIDTH..=LIST_MAX_WIDTH)
            .show_inside(ui, |ui| {
                self.render_file_list(ui, &mut actions);
            });
        self.sync_list_width(panel.response.rect.width(), ui.ctx(), conn);

        self.render_editor(ui, &mut actions);
        self.render_dialogs(ui.ctx(), conn);

        self.apply_actions(actions, conn);
    }

    /// 设置数据库是否可写（由插件在初始化探测后传入）
    pub fn set_storage_writable(&mut self, writable: bool) {
        self.storage_writable = writable;
    }

    /// 数据库不可写时的顶部警告
    ///
    /// 常见原因：程序运行在带低完整性标签或受限权限的目录下（例如被沙箱标记过的工作区），
    /// 此时低完整性进程无法写入 `%APPDATA%` 下的数据库，SQLite 只报 "readonly database"。
    fn render_storage_warning(&self, ui: &mut Ui) {
        ui.horizontal_wrapped(|ui| {
            ui.colored_label(
                egui::Color32::from_rgb(0xE0, 0x60, 0x60),
                "⚠ 数据库不可写：分组与常用文件列表无法保存。请把程序放在普通目录运行\
                 （例如 C:\\Program Files\\Tools-box），不要运行在带低完整性标签或受限权限的目录下。",
            );
        });
        ui.separator();
    }

    /// 当前是否存在可保存的标签（无标签或只读标签时不应响应保存快捷键）
    fn can_save_active_tab(&self) -> bool {
        self.active_tab
            .and_then(|index| self.tabs.get(index))
            .is_some_and(|tab| !tab.read_only)
    }

    /// Ctrl+S 是否被按下（消费按键，避免继续向下传播）
    fn save_shortcut_pressed(&self, ctx: &Context) -> bool {
        ctx.input_mut(|input| input.consume_key(egui::Modifiers::CTRL, egui::Key::S))
    }

    /// 重新加载目录与文件条目
    fn reload_data(&mut self, conn: &Connection) {
        match store::load_directories(conn) {
            Ok(directories) => self.directories = directories,
            Err(error) => {
                log::error!("加载常用文件目录失败: {}", error);
                self.status = format!("加载目录失败: {error}");
            }
        }

        match store::load_files(conn) {
            Ok(files) => self.files = files,
            Err(error) => {
                log::error!("加载常用文件列表失败: {}", error);
                self.status = format!("加载文件列表失败: {error}");
            }
        }

        self.refresh_missing_files();
    }

    /// 刷新磁盘上已不存在的条目
    fn refresh_missing_files(&mut self) {
        self.missing_files = self
            .files
            .iter()
            .filter(|file| !Path::new(&file.path).is_file())
            .map(|file| file.id)
            .collect();
    }

    /// 渲染顶部工具条
    fn render_toolbar(&mut self, ui: &mut Ui, actions: &mut Vec<Action>) {
        ui.horizontal(|ui| {
            if ui
                .button("＋ 添加文件")
                .on_hover_text("把常用配置文件加入列表")
                .clicked()
            {
                actions.push(Action::StartAddFile(self.selected_directory()));
            }

            if ui
                .button("🗂 目录管理")
                .on_hover_text("新建、重命名、删除自定义目录")
                .clicked()
            {
                actions.push(Action::StartManageDirectories);
            }

            ui.separator();

            let can_save = self
                .active_tab
                .and_then(|index| self.tabs.get(index))
                .is_some_and(|tab| !tab.read_only);
            if ui
                .add_enabled(can_save, egui::Button::new("💾 保存"))
                .on_hover_text("保存当前标签（Ctrl+S）")
                .clicked()
            {
                actions.push(Action::SaveActive);
            }

            let has_tab = self.active_tab.is_some();
            if ui
                .add_enabled(has_tab, egui::Button::new("⟳ 重载"))
                .on_hover_text("放弃修改并重新从磁盘读取")
                .clicked()
            {
                actions.push(Action::ReloadActive);
            }

            // 只读预览（超过 2 MB 只加载了前 2 MB）不能另存为，否则会把截断内容当完整文件写出
            let can_save_as = self
                .active_tab
                .and_then(|index| self.tabs.get(index))
                .is_some_and(|tab| !tab.read_only);
            if ui
                .add_enabled(can_save_as, egui::Button::new("📄 另存为…"))
                .on_hover_text("把当前内容另存到其它路径，并把标签切换到新文件")
                .clicked()
            {
                actions.push(Action::SaveAsActive);
            }

            let find_visible = self.find.visible;
            if ui
                .add_enabled(has_tab, egui::SelectableLabel::new(find_visible, "🔍 查找"))
                .on_hover_text("查找 / 替换（Ctrl+F 保留给全局插件搜索）")
                .clicked()
            {
                actions.push(Action::ToggleFind);
            }

            ui.separator();

            let mut sort_by_recent = self.sort_by_recent;
            if ui
                .checkbox(&mut sort_by_recent, "⇅ 最近打开")
                .on_hover_text("按最近打开时间排序常用文件")
                .changed()
            {
                self.sort_by_recent = sort_by_recent;
            }
        });
    }

    /// 当前选中条目所属目录（用于添加文件时预选目录）
    fn selected_directory(&self) -> Option<i64> {
        self.selected_file_id
            .and_then(|id| self.files.iter().find(|file| file.id == id))
            .and_then(|file| file.directory_id)
    }

    /// 渲染左侧目录 / 文件列表
    fn render_file_list(&mut self, ui: &mut Ui, actions: &mut Vec<Action>) {
        let directories = self.directories.clone();
        let mut files = self.files.clone();
        if self.sort_by_recent {
            sort_files_by_recent(&mut files);
        }

        egui::ScrollArea::vertical()
            .id_salt("file_editor_list_scroll")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for directory in &directories {
                    let file_count = files
                        .iter()
                        .filter(|file| file.directory_id == Some(directory.id))
                        .count();
                    egui::CollapsingHeader::new(
                        RichText::new(format!("📁 {}（{file_count}）", directory.name)).strong(),
                    )
                    .id_salt(format!("file_editor_dir_{}", directory.id))
                    .default_open(true)
                    .show(ui, |ui| {
                        let mut empty = true;
                        for file in files
                            .iter()
                            .filter(|file| file.directory_id == Some(directory.id))
                        {
                            empty = false;
                            self.render_file_row(ui, file, actions);
                        }

                        if empty {
                            ui.weak("（空目录）");
                        }
                    });
                }

                let ungrouped: Vec<&models::FavoriteFile> = files
                    .iter()
                    .filter(|file| file.directory_id.is_none())
                    .collect();
                if !ungrouped.is_empty() || directories.is_empty() {
                    egui::CollapsingHeader::new(
                        RichText::new(format!("📁 未分组（{}）", ungrouped.len())).strong(),
                    )
                    .id_salt("file_editor_dir_ungrouped")
                    .default_open(true)
                    .show(ui, |ui| {
                        if ungrouped.is_empty() {
                            ui.weak("点击「＋ 添加文件」加入常用配置文件");
                        }
                        for file in ungrouped {
                            self.render_file_row(ui, file, actions);
                        }
                    });
                }
            });
    }

    /// 渲染单个文件条目
    fn render_file_row(&self, ui: &mut Ui, file: &models::FavoriteFile, actions: &mut Vec<Action>) {
        let selected = self.selected_file_id == Some(file.id);
        let dirty = self
            .tabs
            .iter()
            .any(|tab| tab.file_id == Some(file.id) && tab.dirty);
        let missing = self.missing_files.contains(&file.id);

        let mut label = file.display_name();
        if dirty {
            label = format!("{label} ●");
        }
        if missing {
            label = format!("⚠ {label}");
        }

        let hover = if missing {
            format!("{}（文件不存在）", file.path)
        } else {
            file.path.clone()
        };

        let path = PathBuf::from(&file.path);
        let display_name = file.display_name();
        let file_id = file.id;

        let response = ui.selectable_label(selected, label).on_hover_text(hover);

        if response.clicked() {
            actions.push(Action::Open {
                file_id,
                path: path.clone(),
                display_name: display_name.clone(),
            });
        }

        response.context_menu(|ui| {
            if ui.button("打开").clicked() {
                actions.push(Action::Open {
                    file_id,
                    path: path.clone(),
                    display_name: display_name.clone(),
                });
                ui.close_menu();
            }

            if ui.button("重命名显示名").clicked() {
                actions.push(Action::StartRenameFile(file_id, file.alias.clone()));
                ui.close_menu();
            }

            if ui.button("在资源管理器中显示").clicked() {
                actions.push(Action::Reveal(path.clone()));
                ui.close_menu();
            }

            if ui.button("从列表移除").clicked() {
                actions.push(Action::RemoveFile(file_id));
                ui.close_menu();
            }
        });
    }

    /// 拖动结束后把左栏宽度写回数据库
    fn sync_list_width(&mut self, width: f32, ctx: &Context, conn: &Connection) {
        self.list_width = width;

        let pointer_down = ctx.input(|input| input.pointer.any_down());
        if !should_persist_width(pointer_down, width, self.last_saved_width) {
            return;
        }

        match store::save_list_width(conn, width) {
            Ok(()) => {
                self.last_saved_width = width;
                log::debug!("已保存列表宽度: {width}");
            }
            Err(error) => log::error!("保存列表宽度失败: {}", error),
        }
    }

    /// 渲染右侧编辑区
    fn render_editor(&mut self, ui: &mut Ui, actions: &mut Vec<Action>) {
        let active = self.active_tab.filter(|index| *index < self.tabs.len());

        egui::TopBottomPanel::bottom("file_editor_status").show_inside(ui, |ui| {
            self.render_status(ui, active);
        });

        self.render_tab_bar(ui, actions);

        let Some(active) = active else {
            ui.centered_and_justified(|ui| {
                ui.weak("从左侧选择一个文件开始编辑（支持 Ctrl+S 保存）");
            });
            return;
        };

        self.render_editor_toolbar(ui, active, actions);
        self.render_tab_message(ui, active);

        if self.find.visible {
            self.render_find_bar(ui, actions);
        }

        // 语法高亮：按文本内容 + 字号 / 主题 / 语言缓存布局结果（不做自动换行）
        let language = models::highlight_language(&self.tabs[active].path);
        let font_size = ui
            .style()
            .text_styles
            .get(&egui::TextStyle::Monospace)
            .map(|font| font.size)
            .unwrap_or(14.0);
        let dark = ui.visuals().dark_mode;
        let highlighter = self.highlighter.get_or_insert_with(SyntaxHighlighter::new);
        let cache = &mut self.highlight_job_cache;
        let tabs = &mut self.tabs;
        let find = &mut self.find;
        let editor_id_slot = &mut self.editor_id;

        egui::ScrollArea::both()
            .id_salt("file_editor_scroll")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let tab = &mut tabs[active];
                let editable = !tab.read_only;

                // 应用查找结果：设置编辑器选区（egui 的选区按字符下标计算）
                //
                // 这里算出的 Id 必须与下面 `TextEdit::id(...)` 使用的一致：
                // `TextEdit::id_salt(s)` 内部把 salt 包成 `Id::new(s)` 再经 `Ui::make_persistent_id`，
                // 直接传 `&str` 会得到另一个 Id，于是选区被写到别的状态槽位（编辑器读不到）。
                let editor_id = ui.make_persistent_id(egui::Id::new(EDITOR_ID_SALT));
                *editor_id_slot = Some(editor_id);
                if let Some((start, end)) = find.pending_selection.take() {
                    let mut text_state = egui::text_edit::TextEditState::load(ui.ctx(), editor_id)
                        .unwrap_or_default();
                    text_state
                        .cursor
                        .set_char_range(Some(egui::text::CCursorRange::two(
                            egui::text::CCursor::new(start),
                            egui::text::CCursor::new(end),
                        )));
                    text_state.store(ui.ctx(), editor_id);
                    find.pending_scroll = true;
                }

                // 编辑区占满可见高度：行数按可用高度换算，长文本继续往下滚
                let row_height =
                    ui.fonts(|fonts| fonts.row_height(&egui::FontId::monospace(font_size)));
                let desired_rows = editor_rows(ui.available_height(), row_height);

                let language_ref = language.as_deref();
                let mut layouter = |ui: &Ui, text: &str, _wrap_width: f32| -> Arc<egui::Galley> {
                    let job = cache.job(
                        highlighter,
                        text,
                        LayoutParams {
                            language: language_ref,
                            font_size,
                            dark,
                        },
                    );

                    // 布局交给 egui / epaint：它会按缩放比例与字形图集的变化自行失效重建
                    ui.fonts(|fonts| fonts.layout_job(job))
                };

                let response = ui.add(
                    egui::TextEdit::multiline(&mut tab.text)
                        .code_editor()
                        // 不折行时宽度由内容决定，这里让控件至少铺满可见宽度（点击空白处也能定位光标）
                        .desired_width(f32::INFINITY)
                        .desired_rows(desired_rows)
                        .interactive(editable)
                        .id(editor_id)
                        .layouter(&mut layouter),
                );

                if response.changed() {
                    tab.dirty = true;
                    // 文本被编辑后原命中位置与命中列表都失效，需重新查找
                    find.current = None;
                    find.pending_selection = None;
                    find.invalidate_matches();
                }

                // 命中高亮：egui 只在编辑器获得焦点时绘制自身选区（builder.rs:723-730），
                // 而查找时焦点在查找框里，所以这里在文本之上自绘：
                // 先铺所有匹配（淡色），再把当前选中项画在最上层（浓色 + 描边）。
                // 关栏后不绘制；绘制前按当前文本校正命中列表（命中缓存时为 O(1)）。
                if find.visible {
                    find.ensure_matches(&tab.text);
                }
                if find.visible && !find.matches.is_empty() {
                    let job = cache.job(
                        highlighter,
                        &tab.text,
                        LayoutParams {
                            language: language_ref,
                            font_size,
                            dark,
                        },
                    );
                    let galley = ui.fonts(|fonts| fonts.layout_job(job));
                    let origin = response.rect.min.to_vec2();
                    let painter = ui.painter();
                    let other_fill = find_other_fill(dark);

                    for found in find.matches.iter().take(MAX_HIGHLIGHTED_MATCHES) {
                        if Some(*found) == find.current {
                            continue; // 当前项用浓色单独画，避免两层底色叠加
                        }
                        let rect = match_rect(&galley, *found).translate(origin);
                        if ui.is_rect_visible(rect) {
                            painter.rect_filled(rect, egui::CornerRadius::same(2), other_fill);
                        }
                    }

                    if let Some(current) = find.current {
                        let rect = match_rect(&galley, current).translate(origin);
                        if ui.is_rect_visible(rect) {
                            painter.rect(
                                rect.expand(1.0),
                                egui::CornerRadius::same(2),
                                find_highlight_fill(dark),
                                egui::Stroke::new(1.0, find_highlight_stroke(dark)),
                                egui::StrokeKind::Inside,
                            );
                        }
                    }
                }

                // egui 只在"本帧交互改变选区"时自动滚动，外部设置的查找选区需要手动滚动到可见区域
                if find.pending_scroll {
                    find.pending_scroll = false;
                    if let Some(range) = find.current {
                        let job = cache.job(
                            highlighter,
                            &tab.text,
                            LayoutParams {
                                language: language_ref,
                                font_size,
                                dark,
                            },
                        );
                        let galley = ui.fonts(|fonts| fonts.layout_job(job));
                        let cursor_rect =
                            galley.pos_from_ccursor(egui::text::CCursor::new(range.end_char));
                        ui.scroll_to_rect(cursor_rect.translate(response.rect.min.to_vec2()), None);
                    }
                }
            });
    }

    /// 渲染查找 / 替换栏
    ///
    /// 不做 Ctrl+F 绑定：`Ctrl+F` 由 app 级快捷键用于聚焦全局插件搜索框。
    fn render_find_bar(&mut self, ui: &mut Ui, actions: &mut Vec<Action>) {
        // 只读预览（超过 2 MB 只加载前 2 MB）不允许替换
        let read_only = self
            .active_tab
            .and_then(|index| self.tabs.get(index))
            .is_some_and(|tab| tab.read_only);

        // 让"匹配总数 / 当前序号"随输入即时更新
        if let Some(index) = self.active_tab.filter(|index| *index < self.tabs.len()) {
            let (tabs, find) = (&self.tabs, &mut self.find);
            find.ensure_matches(&tabs[index].text);
        }

        ui.horizontal_wrapped(|ui| {
            ui.label("🔍");

            let query_id = ui.make_persistent_id(egui::Id::new(FIND_QUERY_ID_SALT));

            // 跳转键必须在渲染查找框**之前**消费：单行输入框自己会吃掉回车并主动失焦，
            // 若等控件处理完再判断，回车已经被消费、焦点也已交还，后续按键就失效了。
            // 约定（类似 IDEA）：↓ / 回车 = 下一个，↑ / Shift+回车 = 上一个；
            // 只在查找框持有焦点时处理，编辑器里的 ↑/↓ 仍然是移动光标。
            if ui.ctx().memory(|memory| memory.has_focus(query_id)) {
                let (previous, next) = ui.input_mut(|input| {
                    let previous = input.consume_key(egui::Modifiers::SHIFT, egui::Key::Enter)
                        || input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp);
                    let next = input.consume_key(egui::Modifiers::NONE, egui::Key::Enter)
                        || input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown);
                    (previous, next)
                });

                if previous {
                    actions.push(Action::FindPrev);
                } else if next {
                    actions.push(Action::FindNext);
                }
            }

            let query = ui.add(
                egui::TextEdit::singleline(&mut self.find.query)
                    .id(query_id)
                    .desired_width(180.0)
                    .hint_text("查找内容（↑/↓ 跳转，回车=下一个）"),
            );
            if self.find.focus_requested {
                query.request_focus();
                self.find.focus_requested = false;
            }
            if query.changed() {
                self.find.invalidate_matches();
            }

            if ui.button("↑").on_hover_text("上一个匹配（↑ 键）").clicked() {
                actions.push(Action::FindPrev);
            }
            if ui
                .button("↓")
                .on_hover_text("下一个匹配（↓ 键或回车）")
                .clicked()
            {
                actions.push(Action::FindNext);
            }

            let total = self.find.matches.len();
            let ordinal = self.find.ordinal().unwrap_or(0);
            if total > 0 {
                ui.label(format!("{ordinal}/{total}"));
            } else if !self.find.query.is_empty() {
                ui.weak("0/0");
            }
            if total > MAX_HIGHLIGHTED_MATCHES {
                ui.weak(format!("仅高亮前 {MAX_HIGHLIGHTED_MATCHES} 处"));
            }

            if ui
                .checkbox(&mut self.find.match_case, "区分大小写")
                .changed()
            {
                // 匹配规则变了，旧命中作废
                self.find.current = None;
                self.find.anchor = None;
                self.find.invalidate_matches();
            }
            ui.separator();

            ui.label("替换为");
            let replacement = ui.add(
                egui::TextEdit::singleline(&mut self.find.replace)
                    .desired_width(180.0)
                    .hint_text("替换内容"),
            );
            if replacement.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter)) {
                actions.push(Action::ReplaceOne);
            }

            if ui
                .add_enabled(
                    self.find.current.is_some() && !read_only,
                    egui::Button::new("替换"),
                )
                .on_hover_text("替换当前匹配并跳到下一个")
                .clicked()
            {
                actions.push(Action::ReplaceOne);
            }
            if ui
                .add_enabled(
                    !self.find.query.is_empty() && !read_only,
                    egui::Button::new("全部替换"),
                )
                .clicked()
            {
                actions.push(Action::ReplaceAll);
            }

            if read_only {
                ui.weak("只读预览不可替换");
            } else if !self.find.message.is_empty() {
                ui.weak(&self.find.message);
            }

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.small_button("✕").on_hover_text("关闭查找栏").clicked() {
                    actions.push(Action::ToggleFind);
                }
            });
        });
        ui.separator();
    }

    /// 清除当前查找命中（切换标签或重新打开文件后原位置失效）
    fn clear_find_match(&mut self) {
        self.find.current = None;
        self.find.anchor = None;
        self.find.pending_selection = None;
        self.find.pending_scroll = false;
        self.find.invalidate_matches();
    }

    /// 在当前标签中查找下一个 / 上一个匹配
    ///
    /// 命中列表由 [`FindState::ensure_matches`] 缓存，导航只在列表内移动下标并环绕，
    /// 因此可以同时给出"匹配总数"和"当前序号"。
    fn find_in_active(&mut self, forward: bool) {
        let Some(index) = self.active_tab.filter(|index| *index < self.tabs.len()) else {
            return;
        };
        if self.find.query.is_empty() {
            self.find.message = "请输入查找内容".to_string();
            return;
        }

        let (tabs, find) = (&self.tabs, &mut self.find);
        let text = &tabs[index].text;
        find.ensure_matches(text);

        if find.matches.is_empty() {
            find.message = if find.replaced_once {
                "已替换 1 处 · 未找到更多匹配".to_string()
            } else {
                "未找到匹配".to_string()
            };
            find.replaced_once = false;
            find.current = None;
            return;
        }

        let total = find.matches.len();
        let current_index = find.current.and_then(|current| find.index_of(current));
        let anchor = find.anchor.unwrap_or(if forward { 0 } else { text.len() });

        let next_index = match (current_index, forward) {
            (Some(index), true) => (index + 1) % total,
            (Some(index), false) => (index + total - 1) % total,
            (None, true) => {
                // 锚点之后的第一处；锚点在文末时环绕到第一处
                find.matches
                    .partition_point(|found| found.start_byte < anchor)
                    % total
            }
            (None, false) => {
                // 锚点之前的最后一处；锚点在开头时环绕到最后一处
                let before = find
                    .matches
                    .partition_point(|found| found.end_byte <= anchor);
                if before == 0 { total - 1 } else { before - 1 }
            }
        };

        let found = find.matches[next_index];
        let line = line_number(text, found.start_byte);
        find.message = if find.replaced_once {
            format!("已替换 1 处 · 下一处第 {line} 行")
        } else {
            format!("第 {line} 行")
        };
        find.replaced_once = false;
        find.anchor = Some(if forward {
            found.end_byte
        } else {
            found.start_byte
        });
        find.current = Some(found);
        find.pending_selection = Some((found.start_char, found.end_char));
    }

    /// 替换当前匹配，并把查找起点移到替换内容之后
    fn replace_in_active(&mut self) {
        let Some(index) = self.active_tab.filter(|index| *index < self.tabs.len()) else {
            return;
        };
        if self.find.query.is_empty() {
            self.find.message = "请输入查找内容".to_string();
            return;
        }
        if self.tabs[index].read_only {
            self.find.message = "只读预览不可替换".to_string();
            log::info!(
                "常用文件编辑器忽略只读标签的替换请求: {}",
                self.tabs[index].path.display()
            );
            return;
        }

        let (tabs, find) = (&mut self.tabs, &mut self.find);
        let tab = &mut tabs[index];

        // 只有当前命中仍与查找内容一致时才替换（文本可能已被手动编辑）
        let valid = find.current.filter(|range| {
            tab.text
                .get(range.start_byte..range.end_byte)
                .is_some_and(|slice| matches_query(slice, &find.query, find.match_case))
        });
        let Some(range) = valid else {
            find.current = None;
            find.message = "请先查找匹配".to_string();
            return;
        };

        tab.text
            .replace_range(range.start_byte..range.end_byte, &find.replace);
        tab.dirty = true;
        find.anchor = Some(range.start_byte + find.replace.len());
        find.current = None;
        find.pending_selection = None;
        find.replaced_once = true;
        find.invalidate_matches();
        find.message = "已替换 1 处".to_string();
        log::info!("常用文件编辑器替换一处: {}", tab.path.display());
    }

    /// 替换当前标签中的全部匹配
    fn replace_all_in_active(&mut self) {
        let Some(index) = self.active_tab.filter(|index| *index < self.tabs.len()) else {
            return;
        };
        if self.find.query.is_empty() {
            self.find.message = "请输入查找内容".to_string();
            return;
        }
        if self.tabs[index].read_only {
            self.find.message = "只读预览不可替换".to_string();
            log::info!(
                "常用文件编辑器忽略只读标签的替换请求: {}",
                self.tabs[index].path.display()
            );
            return;
        }

        let (tabs, find) = (&mut self.tabs, &mut self.find);
        let tab = &mut tabs[index];
        let (text, count) = replace_all(&tab.text, &find.query, &find.replace, find.match_case);

        if count == 0 {
            find.current = None;
            find.message = "未找到匹配".to_string();
            return;
        }

        tab.text = text;
        tab.dirty = true;
        find.current = None;
        find.anchor = None;
        find.pending_selection = None;
        find.replaced_once = false;
        find.invalidate_matches();
        find.message = format!("已替换 {count} 处");
        log::info!("常用文件编辑器替换全部: {} 处", count);
    }

    /// 渲染标签栏
    fn render_tab_bar(&mut self, ui: &mut Ui, actions: &mut Vec<Action>) {
        if self.tabs.is_empty() {
            return;
        }

        let titles: Vec<String> = self.tabs.iter().map(|tab| tab.title()).collect();
        egui::ScrollArea::horizontal()
            .id_salt("file_editor_tabs")
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    for (index, title) in titles.iter().enumerate() {
                        let is_active = self.active_tab == Some(index);
                        let label = ui
                            .selectable_label(is_active, title)
                            .on_hover_text(self.tabs[index].path.display().to_string());
                        if label.clicked() {
                            actions.push(Action::ActivateTab(index));
                        }

                        if ui.small_button("×").on_hover_text("关闭标签").clicked() {
                            actions.push(Action::CloseTab(index));
                        }
                    }
                });
            });
        ui.separator();
    }

    /// 渲染当前标签的保存 / 重载操作行
    fn render_editor_toolbar(&mut self, ui: &mut Ui, active: usize, actions: &mut Vec<Action>) {
        let read_only = self.tabs[active].read_only;
        let path = self.tabs[active].path.display().to_string();

        ui.horizontal(|ui| {
            if ui
                .add_enabled(!read_only, egui::Button::new("💾 保存"))
                .on_hover_text("Ctrl+S")
                .clicked()
            {
                actions.push(Action::SaveActive);
            }

            if ui.button("⟳ 重载").clicked() {
                actions.push(Action::ReloadActive);
            }

            ui.separator();
            ui.label(RichText::new(path).weak());
        });
    }

    /// 渲染标签提示（只读、外部修改等）
    fn render_tab_message(&self, ui: &mut Ui, active: usize) {
        let Some(message) = self.tabs[active].message.as_deref() else {
            return;
        };

        ui.horizontal_wrapped(|ui| {
            ui.colored_label(
                egui::Color32::from_rgb(0xE0, 0xA0, 0x30),
                format!("⚠ {message}"),
            );
        });
    }

    /// 渲染状态栏
    fn render_status(&self, ui: &mut Ui, active: Option<usize>) {
        ui.horizontal(|ui| {
            match active.and_then(|index| self.tabs.get(index)) {
                Some(tab) => {
                    let saved = if tab.dirty { "未保存" } else { "已保存" };
                    // 只读标签未判定编码与换行符，避免显示误导性的值
                    let format = if tab.read_only {
                        "只读预览".to_string()
                    } else {
                        format!("{} | {}", tab.encoding.label(), tab.line_ending.label())
                    };
                    let language = models::highlight_language(&tab.path)
                        .map(|language| format!(" | 推断语法 {language}"))
                        .unwrap_or_default();
                    ui.label(format!(
                        "{} | {}{} | {}",
                        tab.path.display(),
                        format,
                        language,
                        saved
                    ));
                }
                None => {
                    ui.weak("未打开文件");
                }
            }

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(RichText::new(&self.status).weak());
            });
        });
    }

    /// 渲染弹窗
    fn render_dialogs(&mut self, ctx: &Context, conn: &Connection) {
        match self.dialog {
            Dialog::AddFile => self.render_add_file_dialog(ctx, conn),
            Dialog::RenameFile => self.render_rename_file_dialog(ctx, conn),
            Dialog::ManageDirectories => self.render_directory_dialog(ctx, conn),
            Dialog::None => {}
        }

        if self.pending_close.is_some() {
            let mut actions = Vec::new();
            self.render_close_dialog(ctx, &mut actions);
            self.apply_actions(actions, conn);
        }
    }

    /// 添加常用文件弹窗
    fn render_add_file_dialog(&mut self, ctx: &Context, conn: &Connection) {
        let mut confirmed = false;
        let mut cancelled = false;
        let directories = self.directories.clone();

        egui::Window::new("添加常用文件")
            .collapsible(false)
            .resizable(false)
            .default_width(520.0)
            .show(ctx, |ui| {
                egui::Grid::new("file_editor_add_grid")
                    .num_columns(2)
                    .spacing([8.0, 8.0])
                    .show(ui, |ui| {
                        ui.label("文件路径：");
                        ui.horizontal(|ui| {
                            ui.add(
                                egui::TextEdit::singleline(&mut self.add_form.path)
                                    .desired_width(320.0)
                                    .hint_text("例如 C:\\Users\\me\\.gitconfig"),
                            );
                            if ui.button("📂 浏览…").clicked()
                                && let Some(path) = rfd::FileDialog::new().pick_file()
                            {
                                self.add_form.path = path.display().to_string();
                            }
                        });
                        ui.end_row();

                        ui.label("显示名：");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.add_form.alias)
                                .desired_width(320.0)
                                .hint_text("留空则显示文件名"),
                        );
                        ui.end_row();

                        ui.label("所属目录：");
                        let selected = directories
                            .iter()
                            .find(|directory| Some(directory.id) == self.add_form.directory_id)
                            .map(|directory| directory.name.clone())
                            .unwrap_or_else(|| "未分组".to_string());
                        egui::ComboBox::from_id_salt("file_editor_add_directory")
                            .selected_text(selected)
                            .show_ui(ui, |ui| {
                                ui.selectable_value(
                                    &mut self.add_form.directory_id,
                                    None,
                                    "未分组",
                                );
                                for directory in &directories {
                                    ui.selectable_value(
                                        &mut self.add_form.directory_id,
                                        Some(directory.id),
                                        &directory.name,
                                    );
                                }
                            });
                        ui.end_row();
                    });

                if let Some(error) = self.add_form.error.as_deref() {
                    ui.add_space(4.0);
                    ui.colored_label(egui::Color32::from_rgb(0xE0, 0x60, 0x60), error);
                }

                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("确定").clicked() {
                        confirmed = true;
                    }
                    if ui.button("取消").clicked() {
                        cancelled = true;
                    }
                });
            });

        if cancelled {
            self.dialog = Dialog::None;
            self.add_form = AddFileForm::default();
            return;
        }

        if confirmed {
            self.submit_add_file(conn);
        }
    }

    /// 校验并提交添加文件表单
    fn submit_add_file(&mut self, conn: &Connection) {
        let raw_path = self.add_form.path.trim().to_string();
        if raw_path.is_empty() {
            self.add_form.error = Some("请填写文件路径".to_string());
            return;
        }

        let path = PathBuf::from(&raw_path);
        if models::is_hosts_file(&path) {
            self.add_form.error =
                Some("hosts 文件请使用 Hosts 管理器插件编辑，避免绕过其环境合并逻辑".to_string());
            return;
        }

        if !path.is_file() {
            self.add_form.error = Some(format!("路径不存在或不是文件: {}", path.display()));
            return;
        }

        let alias = self.add_form.alias.clone();
        let alias = if alias.trim().is_empty() {
            None
        } else {
            Some(alias)
        };
        let directory_id = self.add_form.directory_id;

        match store::insert_file(conn, directory_id, &raw_path, alias.as_deref()) {
            Ok(id) => {
                self.status = format!("已加入常用列表: {raw_path}");
                log::info!("常用文件编辑器新增条目 id={id}, path={raw_path}");
                self.reload_data(conn);
                self.dialog = Dialog::None;
                self.add_form = AddFileForm::default();
            }
            Err(error) => {
                log::error!("常用文件编辑器新增条目失败 {raw_path}: {error}");
                self.add_form.error = Some(error.to_string());
            }
        }
    }

    /// 重命名显示名弹窗
    fn render_rename_file_dialog(&mut self, ctx: &Context, conn: &Connection) {
        let Some(entry) = self.rename_form.as_mut() else {
            self.dialog = Dialog::None;
            return;
        };
        let alias = &mut entry.1;

        let mut confirmed = false;
        let mut cancelled = false;

        egui::Window::new("重命名显示名")
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                ui.label("显示名（留空表示使用文件名）：");
                ui.add(egui::TextEdit::singleline(alias).desired_width(280.0));
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("确定").clicked() {
                        confirmed = true;
                    }
                    if ui.button("取消").clicked() {
                        cancelled = true;
                    }
                });
            });

        if cancelled {
            self.rename_form = None;
            self.dialog = Dialog::None;
            return;
        }

        if !confirmed {
            return;
        }

        let Some((id, alias)) = self.rename_form.take() else {
            return;
        };
        let alias = alias.trim().to_string();
        let alias = if alias.is_empty() { None } else { Some(alias) };

        match store::update_alias(conn, id, alias.as_deref()) {
            Ok(()) => {
                self.status = "已更新显示名".to_string();
                log::info!("常用文件编辑器更新显示名 id={id}");
                self.reload_data(conn);
            }
            Err(error) => {
                self.status = format!("更新显示名失败: {error}");
                log::error!("常用文件编辑器更新显示名失败 id={id}: {error}");
            }
        }

        self.dialog = Dialog::None;
    }

    /// 目录管理弹窗
    fn render_directory_dialog(&mut self, ctx: &Context, conn: &Connection) {
        let mut create = false;
        let mut rename: Option<i64> = None;
        let mut delete: Option<i64> = None;
        let mut close = false;
        let mut select_rename: Option<(i64, String)> = None;
        let directories = self.directories.clone();

        egui::Window::new("目录管理")
            .collapsible(false)
            .resizable(false)
            .default_width(460.0)
            .show(ctx, |ui| {
                ui.label("现有目录：");
                egui::ScrollArea::vertical()
                    .id_salt("file_editor_directory_list")
                    .max_height(200.0)
                    .show(ui, |ui| {
                        for directory in &directories {
                            let count = self
                                .files
                                .iter()
                                .filter(|file| file.directory_id == Some(directory.id))
                                .count();
                            ui.horizontal(|ui| {
                                ui.label(format!("📁 {}（{count} 个文件）", directory.name));
                                if ui.small_button("重命名").clicked() {
                                    select_rename = Some((directory.id, directory.name.clone()));
                                }
                                if ui.small_button("删除").clicked() {
                                    delete = Some(directory.id);
                                }
                            });
                        }
                    });

                ui.separator();

                if let Some((id, name)) = select_rename {
                    self.dir_form.rename_target = Some(id);
                    self.dir_form.rename_name = name;
                }

                match self.dir_form.rename_target {
                    Some(id) => {
                        ui.horizontal(|ui| {
                            ui.label("新名称：");
                            ui.add(
                                egui::TextEdit::singleline(&mut self.dir_form.rename_name)
                                    .desired_width(200.0),
                            );
                            if ui.button("保存名称").clicked() {
                                rename = Some(id);
                            }
                            if ui.button("取消重命名").clicked() {
                                self.dir_form.rename_target = None;
                                self.dir_form.rename_name.clear();
                            }
                        });
                    }
                    None => {
                        ui.horizontal(|ui| {
                            ui.label("新建目录：");
                            ui.add(
                                egui::TextEdit::singleline(&mut self.dir_form.new_name)
                                    .desired_width(200.0)
                                    .hint_text("目录名称"),
                            );
                            if ui.button("新建").clicked() {
                                create = true;
                            }
                        });
                    }
                }

                if let Some(error) = self.dir_form.error.as_deref() {
                    ui.add_space(4.0);
                    ui.colored_label(egui::Color32::from_rgb(0xE0, 0x60, 0x60), error);
                }

                ui.add_space(6.0);
                ui.weak("删除目录只删除分组，其下文件会移入「未分组」，不会删除磁盘文件。");

                ui.add_space(6.0);
                if ui.button("关闭").clicked() {
                    close = true;
                }
            });

        if create {
            let name = self.dir_form.new_name.clone();
            match store::insert_directory(conn, &name) {
                Ok(id) => {
                    self.status = format!("已新建目录: {name}");
                    log::info!("常用文件编辑器新建目录 id={id}, name={name}");
                    self.dir_form.new_name.clear();
                    self.dir_form.error = None;
                    self.reload_data(conn);
                }
                Err(error) => {
                    log::error!("常用文件编辑器新建目录失败: {error}");
                    self.dir_form.error = Some(error.to_string());
                }
            }
        } else if let Some(id) = rename {
            let name = self.dir_form.rename_name.clone();
            match store::rename_directory(conn, id, &name) {
                Ok(()) => {
                    self.status = format!("已重命名目录: {name}");
                    log::info!("常用文件编辑器重命名目录 id={id}, name={name}");
                    self.dir_form.rename_target = None;
                    self.dir_form.rename_name.clear();
                    self.dir_form.error = None;
                    self.reload_data(conn);
                }
                Err(error) => {
                    log::error!("常用文件编辑器重命名目录失败: {error}");
                    self.dir_form.error = Some(error.to_string());
                }
            }
        } else if let Some(id) = delete {
            match store::delete_directory(conn, id) {
                Ok(()) => {
                    self.status = "已删除目录（其下文件已移入未分组）".to_string();
                    log::info!("常用文件编辑器删除目录 id={id}");
                    self.dir_form.error = None;
                    self.reload_data(conn);
                }
                Err(error) => {
                    log::error!("常用文件编辑器删除目录失败: {error}");
                    self.dir_form.error = Some(error.to_string());
                }
            }
        }

        if close {
            self.dialog = Dialog::None;
            self.dir_form = DirectoryForm::default();
        }
    }

    /// 未保存关闭确认弹窗
    fn render_close_dialog(&mut self, ctx: &Context, actions: &mut Vec<Action>) {
        let Some(index) = self.pending_close else {
            return;
        };
        let Some(tab) = self.tabs.get(index) else {
            self.pending_close = None;
            return;
        };
        let display_name = tab.display_name.clone();

        let mut decision = None;
        egui::Window::new("关闭标签")
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                ui.label(format!("文件 {display_name} 有未保存的修改。"));
                ui.label("关闭前是否保存？");
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("💾 保存").clicked() {
                        decision = Some(models::CloseDecision::Save);
                    }
                    if ui.button("不保存").clicked() {
                        decision = Some(models::CloseDecision::Discard);
                    }
                    if ui.button("取消").clicked() {
                        decision = Some(models::CloseDecision::Cancel);
                    }
                });
            });

        if let Some(decision) = decision {
            actions.push(Action::FinishClose(decision));
        }
    }

    /// 执行渲染期间收集的动作
    fn apply_actions(&mut self, actions: Vec<Action>, conn: &Connection) {
        for action in actions {
            match action {
                Action::Open {
                    file_id,
                    path,
                    display_name,
                } => self.open_file(conn, file_id, &path, display_name),
                Action::ActivateTab(index) => {
                    if index < self.tabs.len() {
                        self.active_tab = Some(index);
                        self.clear_find_match();
                        self.check_external_change(index);
                    }
                }
                Action::CloseTab(index) => self.request_close(index),
                Action::SaveActive => {
                    if let Some(index) = self.active_tab {
                        self.save_tab(index);
                    }
                }
                Action::SaveAsActive => {
                    if let Some(index) = self.active_tab {
                        self.save_tab_as(index);
                    }
                }
                Action::ReloadActive => {
                    if let Some(index) = self.active_tab {
                        self.reload_tab(index);
                    }
                }
                Action::ToggleFind => {
                    let closing = self.find.visible;
                    self.find.visible = !closing;
                    if closing {
                        // 关栏即结束查找：清掉命中，避免高亮残留或画在旧位置上
                        self.clear_find_match();
                        self.find.matches.clear();
                    } else {
                        // 打开查找栏时把输入焦点交给查找框
                        self.find.focus_requested = true;
                    }
                }
                Action::FindNext => self.find_in_active(true),
                Action::FindPrev => self.find_in_active(false),
                Action::ReplaceOne => {
                    self.replace_in_active();
                    self.find_in_active(true);
                }
                Action::ReplaceAll => self.replace_all_in_active(),
                Action::RemoveFile(id) => self.remove_file(conn, id),
                Action::Reveal(path) => self.reveal_in_explorer(&path),
                Action::StartAddFile(directory_id) => {
                    self.add_form = AddFileForm {
                        directory_id,
                        ..AddFileForm::default()
                    };
                    self.dialog = Dialog::AddFile;
                }
                Action::StartRenameFile(id, alias) => {
                    self.rename_form = Some((id, alias.unwrap_or_default()));
                    self.dialog = Dialog::RenameFile;
                }
                Action::StartManageDirectories => {
                    self.dir_form = DirectoryForm::default();
                    self.dialog = Dialog::ManageDirectories;
                }
                Action::FinishClose(decision) => self.finish_close(decision),
            }
        }
    }

    /// 打开文件：已打开则激活，否则新建标签
    fn open_file(&mut self, conn: &Connection, file_id: i64, path: &Path, display_name: String) {
        self.selected_file_id = Some(file_id);

        if let Some(index) = self
            .tabs
            .iter()
            .position(|tab| tab.file_id == Some(file_id) || tab.path == path)
        {
            self.active_tab = Some(index);
            self.clear_find_match();
            self.check_external_change(index);
            return;
        }

        if models::is_hosts_file(path) {
            self.status = "hosts 文件请使用 Hosts 管理器插件编辑".to_string();
            return;
        }

        match document::load(path) {
            Ok(loaded) => {
                if let Err(error) = store::touch_last_opened(conn, file_id) {
                    log::warn!("更新最近打开时间失败: {}", error);
                }

                let message = match (loaded.message, loaded.mixed_line_endings) {
                    (Some(message), _) => Some(message),
                    (None, true) => Some(format!(
                        "文件混用 CRLF 与 LF，保存时将统一为 {}",
                        loaded.line_ending.label()
                    )),
                    (None, false) => None,
                };
                self.tabs.push(models::EditorTab {
                    file_id: Some(file_id),
                    path: path.to_path_buf(),
                    display_name,
                    text: loaded.text,
                    dirty: false,
                    disk: loaded.disk,
                    encoding: loaded.encoding,
                    line_ending: loaded.line_ending,
                    read_only: loaded.read_only,
                    message,
                });
                self.active_tab = Some(self.tabs.len() - 1);
                self.clear_find_match();
                self.status = format!("已打开 {}", path.display());
                log::info!("常用文件编辑器打开文件: {}", path.display());
            }
            Err(error) => {
                self.status = format!("打开失败: {error}");
                log::error!("常用文件编辑器打开失败 {}: {error}", path.display());
            }
        }
    }

    /// 从常用列表移除条目（不关闭标签、不删除磁盘文件）
    fn remove_file(&mut self, conn: &Connection, id: i64) {
        match store::delete_file(conn, id) {
            Ok(()) => {
                self.status = "已从常用列表移除（磁盘文件未删除）".to_string();
                self.reload_data(conn);
                log::info!("常用文件编辑器移除条目 id={id}");
            }
            Err(error) => {
                self.status = format!("移除失败: {error}");
                log::error!("常用文件编辑器移除条目失败 id={id}: {error}");
            }
        }
    }

    /// 在资源管理器中定位文件
    fn reveal_in_explorer(&mut self, path: &Path) {
        if let Err(error) = std::process::Command::new("explorer")
            .arg(format!("/select,{}", path.display()))
            .spawn()
        {
            self.status = format!("无法打开资源管理器: {error}");
            log::warn!("无法在资源管理器中显示 {}: {error}", path.display());
        }
    }

    /// 保存指定标签
    fn save_tab(&mut self, index: usize) {
        if index >= self.tabs.len() {
            return;
        }

        // 只读标签是预期内不会保存的状态，按信息级记录，避免污染错误日志
        if self.tabs[index].read_only {
            self.status = "只读标签不可保存".to_string();
            log::info!(
                "常用文件编辑器忽略只读标签的保存请求: {}",
                self.tabs[index].path.display()
            );
            return;
        }

        let outcome: std::result::Result<String, String> = {
            let tab = &mut self.tabs[index];
            match document::save(&tab.path, &tab.text, tab.encoding, tab.line_ending) {
                Ok(disk) => {
                    tab.disk = disk;
                    tab.dirty = false;
                    tab.message = None;
                    Ok(tab.path.display().to_string())
                }
                Err(error) => {
                    let message = format!("保存失败: {error}");
                    tab.message = Some(message.clone());
                    Err(message)
                }
            }
        };

        match outcome {
            Ok(path) => {
                self.status = format!("已保存 {path}");
                log::info!("常用文件编辑器已保存: {path}");
            }
            Err(message) => {
                self.status = message.clone();
                log::error!("常用文件编辑器{message}");
            }
        }
    }

    /// 另存为：把当前内容写入用户选择的新路径，并把标签切换到新文件
    fn save_tab_as(&mut self, index: usize) {
        if index >= self.tabs.len() {
            return;
        }

        // 只读预览只加载了前 2 MB，另存为会写出截断内容，直接拒绝
        if self.tabs[index].read_only {
            self.status = "只读预览不可另存为（文件超过 2 MB 上限）".to_string();
            log::info!(
                "常用文件编辑器忽略只读标签的另存为请求: {}",
                self.tabs[index].path.display()
            );
            return;
        }

        let file_name = self.tabs[index]
            .path
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_default();
        let Some(target) = rfd::FileDialog::new().set_file_name(&file_name).save_file() else {
            return;
        };

        if models::is_hosts_file(&target) {
            self.status = "hosts 文件请使用 Hosts 管理器插件编辑".to_string();
            return;
        }

        let (text, encoding, line_ending) = {
            let tab = &self.tabs[index];
            (tab.text.clone(), tab.encoding, tab.line_ending)
        };

        match document::save(&target, &text, encoding, line_ending) {
            Ok(disk) => {
                // 目标已是列表中的条目时重新绑定，避免重复添加同一个文件
                let target_key = target.display().to_string();
                let existing = self
                    .files
                    .iter()
                    .find(|file| file.path.eq_ignore_ascii_case(&target_key))
                    .map(|file| file.id);

                let tab = &mut self.tabs[index];
                apply_saved_as(tab, &target, disk);
                if let Some(id) = existing {
                    tab.file_id = Some(id);
                }

                self.status = format!("已另存为 {}", target.display());
                log::info!("常用文件编辑器另存为: {}", target.display());
            }
            Err(error) => {
                let message = format!("另存为失败: {error}");
                self.tabs[index].message = Some(message.clone());
                self.status = message.clone();
                log::error!("常用文件编辑器另存为失败 {}: {error}", target.display());
            }
        }
    }

    /// 放弃修改并重新读取标签内容
    fn reload_tab(&mut self, index: usize) {
        if index >= self.tabs.len() {
            return;
        }

        // 文本将被整体替换，旧查找命中作废
        self.clear_find_match();

        let path = self.tabs[index].path.clone();
        match document::load(&path) {
            Ok(loaded) => {
                let tab = &mut self.tabs[index];
                tab.text = loaded.text;
                tab.dirty = false;
                tab.disk = loaded.disk;
                tab.encoding = loaded.encoding;
                tab.line_ending = loaded.line_ending;
                tab.read_only = loaded.read_only;
                tab.message = loaded.message;
                self.status = format!("已重新加载 {}", path.display());
                log::info!("常用文件编辑器已重新加载: {}", path.display());
            }
            Err(error) => {
                self.tabs[index].message = Some(format!("重载失败: {error}"));
                self.status = format!("重载失败: {error}");
                log::error!("常用文件编辑器重载失败 {}: {error}", path.display());
            }
        }
    }

    /// 检测标签对应的文件是否被外部修改
    fn check_external_change(&mut self, index: usize) {
        let Some((path, baseline)) = self.tabs.get(index).map(|tab| (tab.path.clone(), tab.disk))
        else {
            return;
        };

        match document::disk_state(&path) {
            Ok(latest) if document::changed_on_disk(baseline, latest) => {
                if self.tabs[index].dirty {
                    self.tabs[index].message =
                        Some("文件已被外部修改；点「重载」可放弃本地修改".to_string());
                } else {
                    log::info!("常用文件编辑器检测到外部修改，自动重载: {}", path.display());
                    self.reload_tab(index);
                }
            }
            Ok(_) => {}
            Err(error) => {
                self.tabs[index].message = Some(format!("文件不可访问: {error}"));
            }
        }
    }

    /// 请求关闭标签：脏标签先弹确认框
    fn request_close(&mut self, index: usize) {
        if index >= self.tabs.len() {
            return;
        }

        if models::close_requires_prompt(self.tabs[index].dirty) {
            self.pending_close = Some(index);
        } else {
            self.close_tab(index);
        }
    }

    /// 处理关闭确认结果
    fn finish_close(&mut self, decision: models::CloseDecision) {
        let Some(index) = self.pending_close else {
            return;
        };

        match decision {
            models::CloseDecision::Save => {
                self.save_tab(index);
                if self.tabs.get(index).is_some_and(|tab| !tab.dirty) {
                    self.pending_close = None;
                    self.close_tab(index);
                }
            }
            models::CloseDecision::Discard => {
                self.pending_close = None;
                self.close_tab(index);
            }
            models::CloseDecision::Cancel => {
                self.pending_close = None;
            }
        }
    }

    /// 关闭标签并调整激活下标
    fn close_tab(&mut self, index: usize) {
        if index >= self.tabs.len() {
            return;
        }

        self.clear_find_match();
        self.tabs.remove(index);
        self.active_tab = active_after_close(self.active_tab, index, self.tabs.len());
    }
}

#[cfg(test)]
mod tests {
    use super::{
        EDITOR_DESIRED_ROWS, FileEditorUi, HIGHLIGHT_MAX_BYTES, HighlightJobCache, LayoutParams,
        active_after_close, apply_saved_as, editor_rows, find_all, find_highlight_fill,
        find_other_fill, line_number, replace_all, should_capture_find_shortcut,
        should_highlight_text, should_persist_width, sort_files_by_recent,
    };
    use crate::plugins::file_editor::store;
    use crate::utils::highlight::SyntaxHighlighter;
    use rusqlite::Connection;
    use std::fs;
    use std::path::PathBuf;

    /// 建立与 `storage::database` 一致的表结构（内存库）
    fn test_connection() -> Connection {
        let conn = Connection::open_in_memory().expect("创建内存数据库失败");
        conn.execute_batch(
            "PRAGMA foreign_keys = ON;
             CREATE TABLE file_editor_directories (
                 id         INTEGER PRIMARY KEY AUTOINCREMENT,
                 name       TEXT NOT NULL UNIQUE,
                 sort_order INTEGER DEFAULT 0,
                 created_at DATETIME DEFAULT CURRENT_TIMESTAMP
             );
             CREATE TABLE file_editor_files (
                 id             INTEGER PRIMARY KEY AUTOINCREMENT,
                 directory_id   INTEGER,
                 path           TEXT NOT NULL UNIQUE,
                 alias          TEXT,
                 sort_order     INTEGER DEFAULT 0,
                 created_at     DATETIME DEFAULT CURRENT_TIMESTAMP,
                 last_opened_at DATETIME,
                 FOREIGN KEY (directory_id) REFERENCES file_editor_directories(id) ON DELETE SET NULL
             );
             CREATE TABLE file_editor_settings (
                 id         INTEGER PRIMARY KEY DEFAULT 1,
                 list_width REAL NOT NULL DEFAULT 260.0,
                 updated_at DATETIME DEFAULT CURRENT_TIMESTAMP
             );",
        )
        .expect("初始化测试表失败");

        conn
    }

    /// 渲染一帧界面并返回输出（可检查实际绘制内容）
    fn render_frame_output(
        ctx: &egui::Context,
        state: &mut FileEditorUi,
        conn: &Connection,
    ) -> egui::FullOutput {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1000.0, 700.0),
            )),
            ..Default::default()
        };

        ctx.run(input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                state.render(ui, conn);
            });
        })
    }

    /// 渲染一帧界面：无窗口环境下验证布局与交互代码不会 panic
    fn render_frame(ctx: &egui::Context, state: &mut FileEditorUi, conn: &Connection) {
        let _ = render_frame_output(ctx, state, conn);
    }

    /// 生成唯一的临时文件路径
    fn temp_file(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "tools-box-file-editor-ui-test-{}-{name}",
            std::process::id()
        ))
    }

    #[test]
    fn active_tab_after_close_prefers_right_neighbour() {
        assert_eq!(active_after_close(Some(1), 1, 3), Some(1));
        assert_eq!(active_after_close(Some(2), 1, 3), Some(1));
        assert_eq!(active_after_close(Some(0), 2, 3), Some(0));
        assert_eq!(active_after_close(Some(0), 1, 2), Some(0));
    }

    #[test]
    fn active_tab_after_close_falls_back_to_last_tab() {
        assert_eq!(active_after_close(Some(2), 2, 2), Some(1));
        assert_eq!(active_after_close(Some(0), 0, 1), Some(0));
        assert_eq!(active_after_close(Some(0), 0, 0), None);
        assert_eq!(active_after_close(None, 0, 2), Some(0));
    }

    #[test]
    fn renders_layout_and_handles_open_save_close_flow() {
        let conn = test_connection();
        let ctx = egui::Context::default();
        let path = temp_file("flow.toml");
        fs::write(&path, "key = 1\r\n").expect("写入测试文件失败");

        let mut state = FileEditorUi::new();
        state.init(&conn);

        // 空列表 / 无标签时渲染
        render_frame(&ctx, &mut state, &conn);

        // 数据库不可写时的警告横幅也必须能正常渲染
        state.set_storage_writable(false);
        render_frame(&ctx, &mut state, &conn);
        state.set_storage_writable(true);

        let id = store::insert_file(&conn, None, &path.display().to_string(), None)
            .expect("新增条目失败");
        state.reload_data(&conn);
        assert_eq!(state.files.len(), 1);

        // 打开文件 → 渲染 → 置脏 → 关闭确认 → 保存 → 放弃关闭
        state.open_file(&conn, id, &path, "flow.toml".to_string());
        render_frame(&ctx, &mut state, &conn);
        assert_eq!(state.tabs.len(), 1);
        assert_eq!(state.tabs[0].line_ending, super::models::LineEnding::CrLf);
        assert!(!state.tabs[0].dirty);

        // 语法高亮缓存：内容与参数未变化时不应重复高亮
        let highlights_before = state.highlight_job_cache.highlights;
        render_frame(&ctx, &mut state, &conn);
        render_frame(&ctx, &mut state, &conn);
        assert_eq!(
            state.highlight_job_cache.highlights, highlights_before,
            "内容与参数未变化时不应重复高亮"
        );

        state.tabs[0].text = "key = 2\n".to_string();
        state.tabs[0].dirty = true;
        render_frame(&ctx, &mut state, &conn);

        state.request_close(0);
        assert_eq!(state.pending_close, Some(0), "脏标签关闭前应弹确认框");
        render_frame(&ctx, &mut state, &conn);

        state.save_tab(0);
        assert!(!state.tabs[0].dirty, "保存后脏标记应复位");
        assert_eq!(
            fs::read_to_string(&path).expect("读取保存结果失败"),
            "key = 2\r\n",
            "保存必须按原文件换行符风格写回"
        );

        state.finish_close(super::models::CloseDecision::Discard);
        assert!(state.tabs.is_empty(), "选择不保存后应关闭标签");
        assert_eq!(state.pending_close, None);
        render_frame(&ctx, &mut state, &conn);

        // 再次打开后关闭非脏标签：不弹确认框
        state.open_file(&conn, id, &path, "flow.toml".to_string());
        state.request_close(0);
        assert!(state.tabs.is_empty());
        assert_eq!(state.pending_close, None);

        // 重载后高亮缓存必须跟随新内容（否则界面继续显示重载前的文本）
        state.open_file(&conn, id, &path, "flow.toml".to_string());
        fs::write(&path, "key = 77\r\n").expect("改写测试文件失败");
        state.reload_tab(0);
        render_frame(&ctx, &mut state, &conn);
        assert_eq!(
            state.highlight_job_cache.text, state.tabs[0].text,
            "重载后高亮缓存必须与标签文本一致"
        );
        assert_eq!(state.tabs[0].text, "key = 77\n");

        let _ = fs::remove_file(&path);
    }

    #[test]
    fn highlights_only_reasonable_text_sizes() {
        assert!(should_highlight_text(0));
        assert!(should_highlight_text(HIGHLIGHT_MAX_BYTES));
        assert!(!should_highlight_text(HIGHLIGHT_MAX_BYTES + 1));
    }

    #[test]
    fn highlight_cache_reuses_job_for_unchanged_text() {
        let ctx = egui::Context::default();
        let highlighter = SyntaxHighlighter::new();
        let mut cache = HighlightJobCache::default();
        let params = |language: Option<&'static str>| LayoutParams {
            language,
            font_size: 14.0,
            dark: true,
        };
        let long_text = "x".repeat(400);

        let first = cache.job(&highlighter, "key = 1\n", params(Some("TOML")));
        let second = cache.job(&highlighter, "key = 1\n", params(Some("TOML")));
        assert_eq!(cache.highlights, 1, "文本与参数都不变时不应重新高亮");
        assert_eq!(first.text, second.text);

        // 长度相同但内容不同（等价于"同版本号、不同文本"，必须重新高亮）
        let same_length = cache.job(&highlighter, "key = 2\n", params(Some("TOML")));
        assert_eq!(cache.highlights, 2, "文本内容变化后必须重新高亮");
        assert_ne!(first.text, same_length.text);

        // 内容相同但语言不同也必须重新高亮
        let other_language = cache.job(&highlighter, "key = 2\n", params(None));
        assert_eq!(cache.highlights, 3, "语言变化后必须重新高亮");
        assert_eq!(other_language.text, same_length.text);

        // 长行不换行：宽度随内容增长，由外层滚动条负责横向滚动
        let long_job = cache.job(&highlighter, &long_text, params(Some("TOML")));
        assert_eq!(cache.highlights, 4, "长文本应重新高亮一次");
        assert_eq!(cache.text, long_text, "缓存应记录最近一次高亮的文本");
        assert!(
            long_job.wrap.max_width.is_infinite(),
            "布局任务必须不折行（max_width 为无穷大）"
        );

        // 布局必须在帧内进行（首帧之前字体系统尚未就绪）
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(600.0, 400.0),
            )),
            ..Default::default()
        };
        let mut galley_width = 0.0;
        let _ = ctx.run(input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                galley_width = ui
                    .fonts(|fonts| fonts.layout_job(long_job.clone()))
                    .size()
                    .x;
            });
        });

        assert!(
            galley_width > 1000.0,
            "长行不应自动换行（否则不会出现横向滚动条），实际宽度 {galley_width}"
        );
    }

    #[test]
    fn finds_all_matches_with_and_without_case() {
        let text = "Name = a\nname = b\n";

        let insensitive = find_all(text, "name", false);
        assert_eq!(insensitive.len(), 2, "不区分大小写应命中两处");
        assert_eq!(
            &text[insensitive[0].start_byte..insensitive[0].end_byte],
            "Name"
        );
        assert_eq!(
            &text[insensitive[1].start_byte..insensitive[1].end_byte],
            "name"
        );

        let sensitive = find_all(text, "name", true);
        assert_eq!(sensitive.len(), 1, "区分大小写只命中一处");
        assert_eq!(
            &text[sensitive[0].start_byte..sensitive[0].end_byte],
            "name"
        );

        assert!(find_all(text, "missing", true).is_empty());
        assert!(find_all(text, "", true).is_empty(), "空查询不应命中");
    }

    #[test]
    fn find_all_reports_char_indices_and_skips_overlaps() {
        let text = "配置 = 1\n键 = 2\n";

        let matches = find_all(text, "键", true);
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].start_byte, 11, "字节偏移");
        assert_eq!(matches[0].start_char, 7, "字符下标（供编辑器选区使用）");

        let overlaps = find_all("aaa", "aa", true);
        assert_eq!(overlaps.len(), 1, "命中之间不重叠");
    }

    #[test]
    fn captures_find_shortcut_only_in_editor_context() {
        assert!(
            should_capture_find_shortcut(true, false, true),
            "编辑器聚焦时接管"
        );
        assert!(
            should_capture_find_shortcut(false, true, true),
            "查找栏已打开时接管"
        );
        assert!(
            should_capture_find_shortcut(false, false, false),
            "焦点不在任何输入框（例如刚点过文件列表）时也接管"
        );
        assert!(
            !should_capture_find_shortcut(false, false, true),
            "焦点在其它输入框（全局搜索框、别的插件）时留给全局搜索"
        );
    }

    #[test]
    fn editor_widget_holds_keyboard_focus_with_recorded_id() {
        let conn = test_connection();
        let ctx = egui::Context::default();
        let path = temp_file("focus-id.toml");
        fs::write(&path, "key = 1\n").expect("写入测试文件失败");

        let mut state = FileEditorUi::new();
        state.init(&conn);
        let id = store::insert_file(&conn, None, &path.display().to_string(), None)
            .expect("新增条目失败");
        state.reload_data(&conn);
        state.open_file(&conn, id, &path, "focus-id.toml".to_string());

        render_frame(&ctx, &mut state, &conn);
        let editor_id = state.editor_id.expect("应记录编辑器 id");
        ctx.memory_mut(|memory| memory.request_focus(editor_id));
        assert!(ctx.memory(|memory| memory.has_focus(editor_id)));

        // 注入文本事件：只有记录下来的 Id 与控件真实 Id 一致时，输入才会落到编辑器上
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1000.0, 700.0),
            )),
            events: vec![egui::Event::Text("x".to_string())],
            ..Default::default()
        };
        let _ = ctx.run(input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                state.render(ui, &conn);
            });
        });

        assert!(
            state.tabs[0].text.contains('x'),
            "编辑器控件应使用记录下来的 Id 接收键盘输入，实际文本={:?}",
            state.tabs[0].text
        );
        assert!(state.tabs[0].dirty, "输入后应标记未保存");

        let _ = fs::remove_file(&path);
    }

    #[test]
    fn ctrl_f_opens_find_bar_when_editor_focused() {
        let conn = test_connection();
        let ctx = egui::Context::default();
        let path = temp_file("find-shortcut.toml");
        fs::write(&path, "key = 1\n").expect("写入测试文件失败");

        let mut state = FileEditorUi::new();
        state.init(&conn);
        let id = store::insert_file(&conn, None, &path.display().to_string(), None)
            .expect("新增条目失败");
        state.reload_data(&conn);
        state.open_file(&conn, id, &path, "find-shortcut.toml".to_string());

        // 先渲染一帧拿到编辑器 id，再让它获得焦点，并再渲染一帧记录该状态
        render_frame(&ctx, &mut state, &conn);
        let editor_id = state.editor_id.expect("应记录编辑器 id");
        ctx.memory_mut(|memory| memory.request_focus(editor_id));
        render_frame(&ctx, &mut state, &conn);
        assert!(state.editor_had_focus, "应记录到编辑器持有焦点");

        // 模拟 app 的 handle_shortcuts 在本帧抢先聚焦全局搜索框
        // （egui 的 request_focus 当帧立即生效，此时编辑器已不再持有焦点）
        let search_box_id = egui::Id::new("global_search_box");
        ctx.memory_mut(|memory| memory.request_focus(search_box_id));

        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1000.0, 700.0),
            )),
            events: vec![egui::Event::Key {
                key: egui::Key::F,
                physical_key: None,
                pressed: true,
                repeat: false,
                // Windows 下真实事件带 command（随 ctrl 一起置位）
                modifiers: egui::Modifiers::CTRL | egui::Modifiers::COMMAND,
            }],
            ..Default::default()
        };
        let _ = ctx.run(input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                state.render(ui, &conn);
            });
        });

        assert!(
            state.find.visible,
            "上一帧编辑器有焦点时 Ctrl+F 必须打开查找栏（即使本帧焦点已被搜索框抢走）"
        );
        assert!(!state.find.focus_requested, "渲染查找栏时应消费掉焦点请求");

        let _ = fs::remove_file(&path);
    }

    /// 构造一个按键事件
    fn key_event(key: egui::Key, modifiers: egui::Modifiers) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        }
    }

    /// 渲染一帧并注入单个按键事件
    fn run_key_frame(
        ctx: &egui::Context,
        state: &mut FileEditorUi,
        conn: &Connection,
        event: egui::Event,
    ) -> egui::FullOutput {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1000.0, 700.0),
            )),
            events: vec![event],
            ..Default::default()
        };

        ctx.run(input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                state.render(ui, conn);
            });
        })
    }

    #[test]
    fn find_shortcuts_keep_focus_and_jump_repeatedly() {
        let conn = test_connection();
        let ctx = egui::Context::default();
        let path = temp_file("find-keys.toml");
        fs::write(&path, "key = 1\nkey = 2\nkey = 3\n").expect("写入测试文件失败");

        let mut state = FileEditorUi::new();
        state.init(&conn);
        let id = store::insert_file(&conn, None, &path.display().to_string(), None)
            .expect("新增条目失败");
        state.reload_data(&conn);
        state.open_file(&conn, id, &path, "find-keys.toml".to_string());
        state.find.query = "key".to_string();

        // 让编辑器先持焦，再用 Ctrl+F 打开查找栏（查找框会请求焦点）
        render_frame(&ctx, &mut state, &conn);
        let editor_id = state.editor_id.expect("应记录编辑器 id");
        ctx.memory_mut(|memory| memory.request_focus(editor_id));
        render_frame(&ctx, &mut state, &conn);
        let _ = run_key_frame(
            &ctx,
            &mut state,
            &conn,
            key_event(egui::Key::F, egui::Modifiers::CTRL),
        );
        assert!(state.find.visible, "Ctrl+F 应打开查找栏");

        // 回车 = 下一个：连续按三次必须依次命中 1/2/3（焦点留在查找框，否则后续按键会失效）
        for expected in 1..=3 {
            let _ = run_key_frame(
                &ctx,
                &mut state,
                &conn,
                key_event(egui::Key::Enter, egui::Modifiers::NONE),
            );
            assert_eq!(
                state.find.ordinal(),
                Some(expected),
                "回车应连续跳到下一个匹配（第 {expected} 次）"
            );
        }

        // ↓ 与回车一致（末尾环绕到第 1 个），↑ 回到上一个（开头环绕到最后一个）
        let _ = run_key_frame(
            &ctx,
            &mut state,
            &conn,
            key_event(egui::Key::ArrowDown, egui::Modifiers::NONE),
        );
        assert_eq!(state.find.ordinal(), Some(1), "↓ 应跳到下一个并环绕");

        let _ = run_key_frame(
            &ctx,
            &mut state,
            &conn,
            key_event(egui::Key::ArrowUp, egui::Modifiers::NONE),
        );
        assert_eq!(state.find.ordinal(), Some(3), "↑ 应跳到上一个并环绕");

        // 跳转不应把焦点交给编辑器
        let focused = ctx.memory(|memory| memory.focused());
        assert!(focused.is_some(), "跳转后应有控件保持焦点");
        assert_ne!(focused, Some(editor_id), "跳转不应把焦点移到编辑器");

        let _ = fs::remove_file(&path);
    }

    #[test]
    fn find_all_recurses_char_indices_across_many_matches() {
        // 中等规模输入：断言递推得到的字符下标正确（不依赖时间断言，避免 CI 抖动）
        let text = "key = 1\n".repeat(2_000);
        let matches = find_all(&text, "key", true);
        assert_eq!(matches.len(), 2_000);
        assert_eq!(matches[1].start_byte, 8);
        assert_eq!(matches[1].start_char, 8);
        assert_eq!(&text[matches[1].start_byte..matches[1].end_byte], "key");

        // 中文混排：字节与字符下标必须同步递推
        let mixed = "配置 = 1\n".repeat(100);
        let matches = find_all(&mixed, "配置", true);
        assert_eq!(matches.len(), 100);
        assert_eq!(matches[1].start_byte, 11);
        assert_eq!(matches[1].start_char, 7);
    }

    #[test]
    fn closing_find_bar_clears_highlights() {
        let conn = test_connection();
        let ctx = egui::Context::default();
        let path = temp_file("find-close.toml");
        fs::write(&path, "key = 1\nkey = 2\n").expect("写入测试文件失败");

        let mut state = FileEditorUi::new();
        state.init(&conn);
        let id = store::insert_file(&conn, None, &path.display().to_string(), None)
            .expect("新增条目失败");
        state.reload_data(&conn);
        state.open_file(&conn, id, &path, "find-close.toml".to_string());

        state.find.visible = true;
        state.find.query = "key".to_string();
        state.find_in_active(true);

        let dark_mode = ctx.style().visuals.dark_mode;
        let current_fill = find_highlight_fill(dark_mode);
        let other_fill = find_other_fill(dark_mode);
        let count_highlights = |output: &egui::FullOutput| {
            output
                .shapes
                .iter()
                .filter(|clipped| {
                    matches!(&clipped.shape, egui::Shape::Rect(rect)
                        if rect.fill == current_fill || rect.fill == other_fill)
                })
                .count()
        };

        let output = render_frame_output(&ctx, &mut state, &conn);
        assert!(count_highlights(&output) > 0, "查找栏可见时应绘制高亮");

        // 关栏后：命中清空、编辑文本、再次渲染都不应出现高亮（否则会残留在错误位置）
        state.apply_actions(vec![super::Action::ToggleFind], &conn);
        assert!(!state.find.visible);
        assert!(state.find.matches.is_empty(), "关栏应清空命中列表");
        state.tabs[0].text.push('x');

        let output = render_frame_output(&ctx, &mut state, &conn);
        assert_eq!(count_highlights(&output), 0, "关闭查找栏后不应再绘制高亮");

        let _ = fs::remove_file(&path);
    }

    #[test]
    fn replaces_all_matches_without_overlap() {
        let (text, count) = replace_all("aaa", "aa", "b", true);
        assert_eq!((text.as_str(), count), ("ba", 1), "替换不重叠");

        let (text, count) = replace_all("Key = 1\nkey = 2\n", "key", "param", false);
        assert_eq!(text, "param = 1\nparam = 2\n");
        assert_eq!(count, 2);

        let (text, count) = replace_all("nothing", "zz", "x", false);
        assert_eq!((text.as_str(), count), ("nothing", 0));
    }

    #[test]
    fn counts_lines_for_match_position() {
        assert_eq!(line_number("a\nb\nc", 0), 1);
        assert_eq!(line_number("a\nb\nc", 2), 2);
        assert_eq!(line_number("a\nb\nc", 4), 3);
        assert_eq!(line_number("a\nb\nc", 999), 3, "越界偏移按文本末尾计算");
    }

    #[test]
    fn find_and_replace_flow_updates_tab() {
        let conn = test_connection();
        let ctx = egui::Context::default();
        let path = temp_file("find-flow.toml");
        fs::write(&path, "key = 1\nother = 2\nkey = 3\n").expect("写入测试文件失败");

        let mut state = FileEditorUi::new();
        state.init(&conn);
        let id = store::insert_file(&conn, None, &path.display().to_string(), None)
            .expect("新增条目失败");
        state.reload_data(&conn);
        state.open_file(&conn, id, &path, "find-flow.toml".to_string());

        state.find.visible = true;
        state.find.query = "key".to_string();

        state.find_in_active(true);
        let first = state.find.current.expect("应命中第一处");
        assert_eq!(first.start_byte, 0);
        assert_eq!(state.find.matches.len(), 2, "应统计出全部匹配");
        assert_eq!(state.find.ordinal(), Some(1), "当前为第 1 处");
        assert!(state.find.message.contains("第 1 行"));

        // 再按"下一个"应移动到第 2 处，再按一次环绕回第 1 处
        state.find_in_active(true);
        assert_eq!(state.find.ordinal(), Some(2));
        state.find_in_active(true);
        assert_eq!(state.find.ordinal(), Some(1), "到末尾后应环绕");

        render_frame(&ctx, &mut state, &conn);
        assert!(
            state.find.pending_selection.is_none(),
            "渲染后查找选区应已应用到编辑器"
        );

        // 关键回归：选区必须真正写进编辑器的 TextEditState（Id 不一致时这一步会失败）
        let editor_id = state.editor_id.expect("渲染后应记录编辑器 id");
        let text_state =
            egui::text_edit::TextEditState::load(&ctx, editor_id).expect("应存在编辑器状态");
        let applied = text_state.cursor.char_range().expect("应写入查找选区");
        let mut applied_indices = [applied.primary.index, applied.secondary.index];
        applied_indices.sort_unstable();
        assert_eq!(
            applied_indices,
            [first.start_char, first.end_char],
            "命中区间必须写入编辑器状态（否则只是滚动、没有选中）"
        );

        // 命中高亮必须真的画出来：焦点在查找框时 egui 不会绘制编辑器自身选区；
        // 非当前匹配用淡色铺底、当前匹配用浓色 + 描边，两种颜色都要出现
        let output = render_frame_output(&ctx, &mut state, &conn);
        let dark_mode = ctx.style().visuals.dark_mode;
        let current_fill = find_highlight_fill(dark_mode);
        let other_fill = find_other_fill(dark_mode);
        let count_fill = |wanted: egui::Color32| {
            output
                .shapes
                .iter()
                .filter(|clipped| {
                    matches!(&clipped.shape, egui::Shape::Rect(rect) if rect.fill == wanted)
                })
                .count()
        };
        assert_eq!(count_fill(current_fill), 1, "当前匹配应有一个浓色高亮");
        assert_eq!(
            count_fill(other_fill),
            state.find.matches.len().min(super::MAX_HIGHLIGHTED_MATCHES) - 1,
            "其余匹配应用淡色高亮区分"
        );

        // 替换当前匹配后应自动跳到下一处
        state.find.replace = "param".to_string();
        state.replace_in_active();
        state.find_in_active(true);
        assert!(state.tabs[0].dirty, "替换后应标记未保存");
        let second = state.find.current.expect("应命中下一处");
        assert_eq!(
            &state.tabs[0].text[second.start_byte..second.end_byte],
            "key"
        );
        assert_eq!(state.find.matches.len(), 1, "替换后命中列表应重算");

        state.replace_all_in_active();
        assert_eq!(state.tabs[0].text, "param = 1\nother = 2\nparam = 3\n");
        assert!(state.find.message.contains("1 处"));

        let _ = fs::remove_file(&path);
    }

    #[test]
    fn read_only_tab_refuses_replace() {
        let conn = test_connection();
        let path = temp_file("huge-replace.toml");
        let limit =
            usize::try_from(super::document::MAX_EDITABLE_BYTES).expect("上限应能转换为 usize");
        fs::write(&path, vec![b'a'; limit + 8]).expect("写入测试文件失败");

        let mut state = FileEditorUi::new();
        state.init(&conn);
        let id = store::insert_file(&conn, None, &path.display().to_string(), None)
            .expect("新增条目失败");
        state.reload_data(&conn);
        state.open_file(&conn, id, &path, "huge-replace.toml".to_string());
        assert!(state.tabs[0].read_only);

        state.find.query = "a".to_string();
        state.find.replace = "b".to_string();
        state.replace_all_in_active();

        assert_eq!(state.tabs[0].text.len(), limit, "只读标签内容不应被替换");
        assert!(!state.tabs[0].dirty, "只读标签不应被标记未保存");
        assert!(state.find.message.contains("只读"));

        let _ = fs::remove_file(&path);
    }

    #[test]
    fn replaces_multibyte_match_and_advances_anchor() {
        let conn = test_connection();
        let path = temp_file("cn-replace.toml");
        fs::write(&path, "键 = 1\n键 = 2\n").expect("写入测试文件失败");

        let mut state = FileEditorUi::new();
        state.init(&conn);
        let id = store::insert_file(&conn, None, &path.display().to_string(), None)
            .expect("新增条目失败");
        state.reload_data(&conn);
        state.open_file(&conn, id, &path, "cn-replace.toml".to_string());

        state.find.query = "键".to_string();
        state.find.replace = "参数".to_string();

        state.find_in_active(true);
        state.replace_in_active();
        assert_eq!(state.tabs[0].text, "参数 = 1\n键 = 2\n", "多字节替换");

        // 替换后起点应移到替换内容之后，继续替换第二处
        state.find_in_active(true);
        state.replace_in_active();
        assert_eq!(state.tabs[0].text, "参数 = 1\n参数 = 2\n");
        assert!(state.tabs[0].dirty);
        assert!(state.find.message.contains("已替换"));

        let _ = fs::remove_file(&path);
    }

    #[test]
    fn sorts_files_by_recent_open_time() {
        let file = |id: i64, path: &str, opened: Option<&str>| super::models::FavoriteFile {
            id,
            directory_id: None,
            path: path.to_string(),
            alias: None,
            sort_order: 0,
            last_opened_at: opened.map(str::to_string),
        };

        let mut files = vec![
            file(1, "C:\\a.toml", None),
            file(2, "C:\\b.toml", Some("2026-10-10 10:00:00")),
            file(3, "C:\\c.toml", Some("2026-10-11 10:00:00")),
            file(4, "C:\\d.toml", None),
        ];
        sort_files_by_recent(&mut files);

        let order: Vec<i64> = files.iter().map(|file| file.id).collect();
        assert_eq!(
            order,
            vec![3, 2, 1, 4],
            "最近打开的排在前面，未打开过的按路径稳定排在最后"
        );
    }

    #[test]
    fn applies_saved_as_target_to_tab() {
        let mut tab = super::models::EditorTab {
            file_id: Some(7),
            path: PathBuf::from("C:\\old.toml"),
            display_name: "old.toml".to_string(),
            text: "key = 1\n".to_string(),
            dirty: true,
            disk: super::models::DiskState {
                modified: None,
                len: 8,
            },
            encoding: super::models::TextEncoding::Utf8,
            line_ending: super::models::LineEnding::Lf,
            read_only: false,
            message: Some("旧提示".to_string()),
        };
        let target = PathBuf::from("C:\\new\\config.toml");
        let disk = super::models::DiskState {
            modified: None,
            len: 16,
        };

        apply_saved_as(&mut tab, &target, disk);

        assert_eq!(tab.path, target);
        assert_eq!(tab.display_name, "config.toml");
        assert_eq!(tab.disk, disk);
        assert_eq!(tab.file_id, None, "另存到新路径后不应再关联原列表条目");
        assert!(!tab.dirty, "另存为成功后应清除未保存标记");
        assert_eq!(tab.message, None, "另存为成功后应清除提示");
    }

    #[test]
    fn keeps_file_link_when_saved_as_same_path() {
        let mut tab = super::models::EditorTab {
            file_id: Some(7),
            path: PathBuf::from("C:\\old.toml"),
            display_name: "old.toml".to_string(),
            text: "key = 1\n".to_string(),
            dirty: true,
            disk: super::models::DiskState {
                modified: None,
                len: 8,
            },
            encoding: super::models::TextEncoding::Utf8,
            line_ending: super::models::LineEnding::Lf,
            read_only: false,
            message: None,
        };
        let same = PathBuf::from("C:\\old.toml");

        apply_saved_as(
            &mut tab,
            &same,
            super::models::DiskState {
                modified: None,
                len: 8,
            },
        );

        assert_eq!(
            tab.file_id,
            Some(7),
            "另存为到原路径时应保持与列表条目的关联"
        );
        assert_eq!(tab.display_name, "old.toml");
        assert!(!tab.dirty);
    }

    #[test]
    fn editor_rows_fills_available_height() {
        assert_eq!(editor_rows(400.0, 20.0), 20, "按可用高度换算行数");
        assert_eq!(editor_rows(10.0, 20.0), 6, "可用高度过小时保留最小行数");
        assert_eq!(editor_rows(100_000.0, 20.0), 400, "行数受上限保护");
        assert_eq!(
            editor_rows(400.0, 0.0),
            EDITOR_DESIRED_ROWS,
            "行高异常时回退默认行数"
        );
        assert_eq!(
            editor_rows(f32::INFINITY, 20.0),
            EDITOR_DESIRED_ROWS,
            "可用高度异常时回退默认行数"
        );
        assert_eq!(
            editor_rows(0.0, 20.0),
            EDITOR_DESIRED_ROWS,
            "无可用高度时回退默认行数"
        );
        assert_eq!(
            editor_rows(-100.0, 20.0),
            EDITOR_DESIRED_ROWS,
            "负可用高度时回退默认行数"
        );
        assert_eq!(
            editor_rows(400.0, f32::NAN),
            EDITOR_DESIRED_ROWS,
            "行高为 NaN 时回退默认行数"
        );
        assert_eq!(
            editor_rows(f32::NAN, 20.0),
            EDITOR_DESIRED_ROWS,
            "可用高度为 NaN 时回退默认行数"
        );
    }

    #[test]
    fn persists_width_only_after_drag_ends() {
        assert!(
            !should_persist_width(true, 300.0, 260.0),
            "拖动过程中不应写库"
        );
        assert!(
            should_persist_width(false, 300.0, 260.0),
            "松手后即使宽度不再变化也必须写库"
        );
        assert!(
            !should_persist_width(false, 260.2, 260.0),
            "小于 0.5px 的变化不写库"
        );
        assert!(
            !should_persist_width(false, 300.0, 300.0),
            "宽度未变化时不写库"
        );
    }

    #[test]
    fn read_only_tab_refuses_save() {
        let conn = test_connection();
        let path = temp_file("huge.toml");
        let limit =
            usize::try_from(super::document::MAX_EDITABLE_BYTES).expect("上限应能转换为 usize");
        fs::write(&path, vec![b'a'; limit + 8]).expect("写入测试文件失败");

        let mut state = FileEditorUi::new();
        state.init(&conn);
        let id = store::insert_file(&conn, None, &path.display().to_string(), None)
            .expect("新增条目失败");
        state.reload_data(&conn);
        state.open_file(&conn, id, &path, "huge.toml".to_string());

        assert_eq!(state.tabs.len(), 1);
        assert!(state.tabs[0].read_only, "超过 2 MB 的文件应只读");
        assert!(
            !state.can_save_active_tab(),
            "只读标签不应允许 Ctrl+S 产生保存动作"
        );

        let original_len = fs::metadata(&path).expect("读取属性失败").len();
        state.save_tab(0);
        assert_eq!(
            fs::metadata(&path).expect("读取属性失败").len(),
            original_len,
            "只读标签的保存必须不写磁盘"
        );

        let _ = fs::remove_file(&path);
    }

    #[test]
    // 测试清理：Windows 下删除只读文件前需要清掉只读属性
    #[allow(clippy::permissions_set_readonly_false)]
    fn save_failure_keeps_tab_open() {
        let conn = test_connection();
        let path = temp_file("readonly-attr.toml");
        fs::write(&path, "key = 1\n").expect("写入测试文件失败");
        let mut permissions = fs::metadata(&path).expect("读取属性失败").permissions();
        permissions.set_readonly(true);
        fs::set_permissions(&path, permissions).expect("设置只读属性失败");

        let mut state = FileEditorUi::new();
        state.init(&conn);
        let id = store::insert_file(&conn, None, &path.display().to_string(), None)
            .expect("新增条目失败");
        state.reload_data(&conn);
        state.open_file(&conn, id, &path, "readonly-attr.toml".to_string());
        assert_eq!(state.tabs.len(), 1);
        assert!(
            !state.tabs[0].read_only,
            "只读属性不影响能否编辑，只在保存时拒绝"
        );

        state.tabs[0].text = "key = 2\n".to_string();
        state.tabs[0].dirty = true;
        state.request_close(0);
        assert_eq!(state.pending_close, Some(0));

        state.finish_close(super::models::CloseDecision::Save);

        assert_eq!(state.tabs.len(), 1, "保存失败时不得关闭标签");
        assert!(state.tabs[0].dirty, "保存失败后应保持未保存状态");
        assert_eq!(state.pending_close, Some(0), "确认框应保留以便重试或取消");
        assert!(
            state.tabs[0]
                .message
                .as_deref()
                .is_some_and(|message| message.contains("只读")),
            "应提示保存失败原因"
        );

        let mut permissions = fs::metadata(&path).expect("读取属性失败").permissions();
        permissions.set_readonly(false);
        let _ = fs::set_permissions(&path, permissions);
        let _ = fs::remove_file(&path);
    }
}
