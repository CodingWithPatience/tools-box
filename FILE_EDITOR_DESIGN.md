# 常用文件编辑器插件设计方案

## 一、功能需求

| 功能 | 说明 | 优先级 |
|------|------|--------|
| 目录管理 | 新增、重命名、删除自定义目录（分组），用于归类常用文件 | P0 |
| 文件条目管理 | 在目录下添加常用文件、移除条目、重命名显示名 | P0 |
| 文件选择添加 | 通过系统文件选择器（rfd）选择文件，也支持粘贴/手输路径 | P0 |
| 打开编辑 | 选中左侧文件后在右侧编辑区显示内容并可直接修改 | P0 |
| 保存写回 | 点击保存按钮把编辑内容写回原文件（原子替换） | P0 |
| Ctrl+S 保存 | 编辑器聚焦时按 Ctrl+S 保存当前标签 | P0 |
| 多标签编辑 | 同时打开多个文件，标签可切换、可关闭，行为对齐 VS Code | P0 |
| 未保存关闭提示 | 关闭脏标签时弹窗询问「保存 / 不保存 / 取消」 | P0 |
| 左右两栏可调宽度 | 左列表与右编辑区之间的分割线可拖动，宽度持久化 | P0 |
| 语法高亮 | 按扩展名复用 `utils::highlight`（syntect）着色，未知类型回退纯文本 | P1 |
| 外部修改检测 | 打开的文件被外部改动（mtime/大小变化）时提示重新加载 | P1 |
| 编码与换行符保持 | 保留 BOM 与主导换行符（CRLF/LF），不做无提示的有损转换 | P1 |
| 手动重载 | 放弃编辑内容、重新从磁盘读取当前标签 | P1 |
| 查找替换 | 当前标签内查找/替换文本 | P2 |
| 另存为 | 把当前内容另存到其它路径 | P2 |
| 最近打开排序 | 按 `last_opened_at` 排序或置顶最近使用 | P2 |

> 边界说明：`hosts` 文件不在本插件中编辑（已有专门的 hosts 管理器负责环境合并写入），
> 添加该路径时给出引导提示，避免绕过 hosts 管理逻辑写坏系统文件。

---

## 二、界面设计

### 2.1 主界面布局

```
┌──────────────────────────────────────────────────────────────────────────────┐
│  📄 常用文件编辑器  [ ＋ 添加文件 ] [ 🗂 目录管理 ] [ 💾 保存 ] [ ⟳ 重载 ] [ 📄 另存为 ] [ ⇅ 最近打开 ] │
│  🔍 [key] [↑] [↓]  3/17  ☐区分大小写  替换为 [param]  [替换] [全部替换]  第 10 行            │
├────────────────────┬─────────────────────────────────────────────────────────┤
│  目录 / 文件列表     │  ┌─ 标签栏 ───────────────────────────────────────────┐  │
│                    │  │ .gitconfig ✕ │ config.toml ● │ settings.json ✕    │  │
│  ▸ 📁 常用配置 (3)  │  └────────────────────────────────────────────────────┘  │
│      .gitconfig    │                                                         │
│      .npmrc        │  [user]                                                 │
│      config.toml   │      name = zhang                                        │
│  ▸ 📁 项目文件 (2)  │      email = zhang@example.com                           │
│      Cargo.toml    │  [core]                                                  │
│      settings.json │      editor = code --wait                                │
│  ▸ 📁 未分组 (1)    │                                                          │
│      .editorconfig │                                                          │
│                    │                                                          │
│                    │  ─────────────────────────────────────────────────────  │
│  ── 拖动此处分隔 ──  │  路径: C:\Users\zhang\.gitconfig   修改时间: 10:32  保存于 10:41 │
├────────────────────┴─────────────────────────────────────────────────────────┤
│  已打开 3 个文件，1 个未保存 | 共 6 个常用文件                                  │
└──────────────────────────────────────────────────────────────────────────────┘
```

要点：

- 左右两栏用 `egui::SidePanel::left("file_editor_list").resizable(true)` 实现，
  默认宽度 260px，宽度写入插件自己的设置表（§4），下次启动沿用。
- 标签栏中脏标签在文件名后显示圆点（`●`），未保存状态一眼可见。
- 左列表条目右键菜单：打开、重命名显示名、从列表移除、在资源管理器中显示。
- 列表底部状态栏显示打开数量与未保存数量；编辑区底部显示文件路径、磁盘修改时间、最近保存时间。
- 编辑区左侧有固定宽度的**行号列**（宽度按实际行数位数自适应、颜色取主题弱化色、不带分隔符），
  与文本滚动区并排 ⇒ 横向滚动长行时行号保持钉住；实现细节见 §8 技术要点 17。

### 2.2 添加 / 编辑文件条目弹窗

```
┌───────────────────────────────────────────────┐
│  添加常用文件                          [ ✕ ]  │
├───────────────────────────────────────────────┤
│  文件路径：[ C:\Users\zhang\.gitconfig     ]  │
│            [ 📂 浏览… ]                       │
│  显示名  ：[ .gitconfig（可留空）           ]  │
│  所属目录：[ 📁 常用配置 ▼ ]                   │
│                                               │
│  ⚠ 该文件已存在列表中（重复路径会被拒绝）       │
│                                               │
│              [ 确定 ]   [ 取消 ]               │
└───────────────────────────────────────────────┘
```

### 2.3 未保存关闭确认弹窗

```
┌───────────────────────────────────────────────┐
│  关闭标签                              [ ✕ ]  │
├───────────────────────────────────────────────┤
│  文件 config.toml 有未保存的修改。             │
│  关闭前是否保存？                              │
│                                               │
│     [ 💾 保存 ]  [ 不保存 ]  [ 取消 ]          │
└───────────────────────────────────────────────┘
```

### 2.4 目录管理弹窗

```
┌───────────────────────────────────────────────┐
│  目录管理                              [ ✕ ]  │
├───────────────────────────────────────────────┤
│  目录名称：[ 新目录名                      ]  │
│                                               │
│  ┌─────────────────────────────────────────┐ │
│  │ 📁 常用配置        （3 个文件）           │ │
│  │ 📁 项目文件        （2 个文件）           │ │
│  │ 📁 未分组          （1 个文件）           │ │
│  └─────────────────────────────────────────┘ │
│                                               │
│  [ 新建 ] [ 重命名选中 ] [ 删除选中 ]          │
│  删除目录时其下文件移入「未分组」，不删除磁盘文件 │
└───────────────────────────────────────────────┘
```

---

## 三、数据结构设计

```rust
/// 自定义目录（分组）
#[derive(Debug, Clone)]
pub struct FileDirectory {
    pub id: i64,
    pub name: String,
    pub sort_order: i32,
    pub created_at: String,
}

/// 常用文件条目（只保存路径与显示名，不保存内容）
#[derive(Debug, Clone)]
pub struct FavoriteFile {
    pub id: i64,
    /// 所属目录，None 表示未分组
    pub directory_id: Option<i64>,
    /// 磁盘绝对路径
    pub path: String,
    /// 列表显示名，None 时取文件名
    pub alias: Option<String>,
    pub sort_order: i32,
    pub created_at: String,
    pub last_opened_at: Option<String>,
}

/// 文本编码（只做无损处理：不认识的编码拒绝编辑而不是猜）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextEncoding {
    /// UTF-8 无 BOM
    Utf8,
    /// UTF-8 带 BOM，保存时保留 BOM
    Utf8Bom,
}

impl TextEncoding {
    /// 探测字节流编码；非 UTF-8 返回 `None`（调用方据此拒绝编辑）
    pub fn detect(bytes: &[u8]) -> Option<Self>;
    /// 按编码写回字节序列（带 BOM 的编码写回 BOM）
    pub fn encode(self, text: &str) -> Vec<u8>;
    /// 状态栏显示名（UTF-8 / UTF-8 BOM）
    pub fn label(self) -> &'static str;
}

/// 换行符风格，保存时按原文件主导风格写回
///
/// 混用（同时存在 CRLF 与 LF）不单独建状态：`detect` 取数量多的一方，
/// 由 `is_mixed` 判断是否需要提示，保存时统一为主导风格。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineEnding {
    Lf,
    CrLf,
}

impl LineEnding {
    /// 探测主导换行符风格（混用时取多数）
    pub fn detect(text: &str) -> Self;
    /// 文本是否同时包含 CRLF 与 LF
    pub fn is_mixed(text: &str) -> bool;
    /// 状态栏显示名（LF / CRLF）
    pub fn label(self) -> &'static str;
    /// 把文本统一为指定换行符风格
    pub fn apply(self, text: &str) -> String;
}

/// 磁盘文件状态，用于外部修改检测
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DiskState {
    pub modified: Option<SystemTime>,
    pub len: u64,
}

/// 一次文件读取的结果（`document::load`）
pub struct LoadedFile {
    /// 编辑器内容（换行符已统一为 LF）
    pub text: String,
    pub encoding: TextEncoding,
    pub line_ending: LineEnding,
    /// 原文件是否混用 CRLF 与 LF
    pub mixed_line_endings: bool,
    pub disk: DiskState,
    /// 只读（文件过大或不可无损编辑）
    pub read_only: bool,
    /// 附加提示（只读原因等）
    pub message: Option<String>,
}

/// 一个已打开的文件标签
#[derive(Debug, Clone)]
pub struct EditorTab {
    /// 对应列表条目 id，`None` 表示未纳入列表
    pub file_id: Option<i64>,
    pub path: PathBuf,
    pub display_name: String,
    /// 编辑器内容（换行符已统一为 LF，保存时按 `line_ending` 写回）
    pub text: String,
    /// 是否有未保存改动（编辑事件置位，保存 / 重载复位）
    pub dirty: bool,
    /// 载入或保存时记录的磁盘状态
    pub disk: DiskState,
    pub encoding: TextEncoding,
    pub line_ending: LineEnding,
    /// 只读（文件过大或不可无损编辑）
    pub read_only: bool,
    /// 提示信息（只读原因、混用换行符、外部修改、保存失败）
    pub message: Option<String>,
}

impl EditorTab {
    /// 标签标题：脏标签追加未保存圆点
    pub fn title(&self) -> String;
}

/// 关闭标签时的用户选择
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseDecision {
    Save,
    Discard,
    Cancel,
}

/// 关闭脏标签是否需要弹窗确认
pub fn close_requires_prompt(dirty: bool) -> bool;

/// 判断路径是否为系统 hosts 文件（引导到 hosts 管理器编辑，避免绕过其合并逻辑）
pub fn is_hosts_file(path: &Path) -> bool;

/// 按扩展名 / 文件名推断语言名（当前用于状态栏展示，阶段四接入着色）
pub fn highlight_language(path: &Path) -> Option<String>;

/// 单文件可编辑大小上限（`document::MAX_EDITABLE_BYTES`，2 MB）
/// 左侧列表默认宽度（`store::DEFAULT_LIST_WIDTH`，260 px）
```

界面状态（`ui::FileEditorUi`，字段私有，仅暴露 `new` / `init` / `render`）：

```rust
/// 当前打开的弹窗
enum Dialog {
    None,
    AddFile,
    RenameFile,
    ManageDirectories,
}

/// 插件界面状态
pub struct FileEditorUi {
    directories: Vec<FileDirectory>,
    files: Vec<FavoriteFile>,
    /// 磁盘上已不存在的条目 id（列表中显示 ⚠）
    missing_files: Vec<i64>,
    tabs: Vec<EditorTab>,
    active_tab: Option<usize>,
    selected_file_id: Option<i64>,
    /// 待确认关闭的标签下标
    pending_close: Option<usize>,
    dialog: Dialog,
    add_form: AddFileForm,
    /// 重命名显示名表单（条目 id、输入内容）
    rename_form: Option<(i64, String)>,
    dir_form: DirectoryForm,
    /// 左栏宽度，来自插件设置表
    list_width: f32,
    status: String,
    initialized: bool,
}
```

---

## 四、数据库设计

新增 `user_version` 迁移到 **3**，两张业务表 + 一张插件设置表（均为 `CREATE TABLE IF NOT EXISTS`，幂等）：

```sql
-- 自定义目录（分组）
CREATE TABLE IF NOT EXISTS file_editor_directories (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    name       TEXT NOT NULL UNIQUE,
    sort_order INTEGER DEFAULT 0,
    created_at DATETIME DEFAULT CURRENT_TIMESTAMP
);

-- 常用文件条目
CREATE TABLE IF NOT EXISTS file_editor_files (
    id             INTEGER PRIMARY KEY AUTOINCREMENT,
    directory_id   INTEGER,
    path           TEXT NOT NULL UNIQUE,   -- 同一路径只允许添加一次
    alias          TEXT,
    sort_order     INTEGER DEFAULT 0,
    created_at     DATETIME DEFAULT CURRENT_TIMESTAMP,
    last_opened_at DATETIME,
    FOREIGN KEY (directory_id) REFERENCES file_editor_directories(id) ON DELETE SET NULL
);

CREATE INDEX IF NOT EXISTS idx_file_editor_files_dir ON file_editor_files(directory_id);

-- 插件设置（单行）：左右两栏宽度等界面状态
CREATE TABLE IF NOT EXISTS file_editor_settings (
    id         INTEGER PRIMARY KEY DEFAULT 1,
    list_width REAL NOT NULL DEFAULT 260.0,
    updated_at DATETIME DEFAULT CURRENT_TIMESTAMP
);
```

表结构落点：`src/storage/database.rs::init_tables()`（与现有插件表集中创建，`user_version` 2 → 3）。

**不落库的数据**：文件内容、光标位置、撤销历史（不持久化，重启后重新从磁盘读取）。

---

## 五、目录结构

```
src/plugins/file_editor/
├── mod.rs        # 插件入口，Plugin trait 实现（name/icon/description/render/init）
├── ui.rs         # 界面渲染：左侧列表、标签栏、编辑区、弹窗、快捷键
├── models.rs     # FileDirectory / FavoriteFile / EditorTab / 编码与换行符模型
├── store.rs      # SQLite CRUD（目录、文件条目、插件设置）
└── document.rs   # 文件读写：编码探测、换行符探测、原子保存、外部修改检测
```

---

## 六、实现步骤

| 步骤 | 任务 | 说明 | 状态 |
|------|------|------|------|
| 1 | 创建插件骨架 | `file_editor/` 目录、`Plugin` trait 实现、注册到 `plugins/mod.rs` | ✅ |
| 2 | 数据库表与迁移 | 三张表 + `user_version` 3，`store.rs` 提供 CRUD | ✅ |
| 3 | 目录管理 | 新建/重命名/删除目录，删除时文件移入未分组 | ✅ |
| 4 | 文件条目管理 | 添加（rfd 选择 + 手输路径）、显示名、移除、去重校验 | ✅ |
| 5 | 左右两栏布局 | `SidePanel::left(...).resizable(true)` + 宽度持久化 | ✅ |
| 6 | 打开与编辑 | 点击条目读取文件、`TextEdit::multiline` 编辑、状态栏信息 | ✅ |
| 7 | 保存写回 | 原子替换保存、错误处理、脏标记与磁盘状态更新 | ✅ |
| 8 | 多标签 | 标签栏、切换、关闭、激活态与脏标记圆点 | ✅ |
| 9 | 未保存提示 | 关闭脏标签弹窗（保存/不保存/取消） | ✅ |
| 10 | Ctrl+S 快捷键 | 编辑器聚焦时保存当前标签 | ✅ |
| 11 | 默认热键与设置 | `tool_hotkeys` 补第 8 个默认热键，设置面板自动补位 | ✅ |
| 12 | 语法高亮与重载 | 语法高亮（缓存布局）、外部修改检测（打开 / 切换 / 窗口获得焦点）、手动重载 | ✅ |
| 13 | 测试与文档同步 | 单元测试、`cargo build`/`cargo test`、更新本设计文档 | ✅ |

---

## 七、依赖库

无需新增依赖，全部复用现有 crate：

| 依赖 | 用途 |
|------|------|
| `egui` / `eframe` | 左右两栏、标签栏、`TextEdit`、弹窗 |
| `rusqlite` | 目录与文件条目持久化 |
| `rfd` | `pick_file()` / `pick_folder()` 系统文件选择器 |
| `syntect`（经 `utils::highlight`） | 按扩展名语法高亮 |
| `anyhow` / `log` | 错误处理与运行日志（日志已落盘到 `%APPDATA%\tools-box\logs\`） |

---

## 八、技术要点

1. **保存策略（原子替换）**：写入同目录临时文件 `*.toolsbox-tmp` → `std::fs::rename` 覆盖原文件，
   与 `hosts_manager/parser.rs`、`ssh_client/sftp.rs` 的既有做法一致；rename 失败时清理临时文件、
   原文件保持不变，错误写入标签提示。
2. **编码处理**：只接受 UTF-8（含 BOM）。带 BOM 时保存写回 BOM，无 BOM 时不添加。
   探测到非 UTF-8（如 GBK/UTF-16）时**拒绝编辑**并在状态栏提示，不做静默转换，避免写坏配置文件。
3. **换行符保持**：读取时统计 CRLF/LF 主导风格；保存时统一按主导风格写回；
   原文件混用两种换行符时在标签上提示"保存时将统一为 X"。避免"编辑一行、整个文件 diff 翻天"。
4. **脏标记与外部修改检测**：`dirty` 由编辑器 `changed()` 事件置位，保存 / 重载复位；
   标签记录 `DiskState { modified, len }`，在打开文件、切换标签以及**窗口重新获得焦点**时比对磁盘状态
   （焦点检查只在本插件处于渲染状态时执行），未修改则自动重载，
   已有本地修改则提示"文件已被外部修改；点「重载」可放弃本地修改"。
5. **多标签与关闭流程**：`tabs: Vec<EditorTab>` + `active_tab` + `pending_close`，
   关闭前若 `dirty` 则弹确认框；选择「保存」时保存失败则**不关闭**标签并展示错误，
   关闭后激活下标按「优先右邻、否则最后一个」规则调整。
6. **Ctrl+S**：在插件 `render` 内用 `input.consume_key(Modifiers::CTRL, Key::S)` 响应并消费按键，
   仅当存在可保存（非只读）的活动标签时才投递保存动作，只读标签的保存请求记 `info!` 并忽略；
   不与 app 级 `Ctrl+F`/`Esc`（`App::handle_shortcuts`）冲突。
7. **文件大小与 UI 卡顿**：单文件超过 2 MB 时**只读取前 2 MB**（`File::take` 限制读取量，
   不整体读入内存），按 UTF-8 字符边界截断后只读展示，状态栏显示「只读预览」而不是编码 / 换行符。
   I/O 沿用现有插件的同步读写方式（hosts 写入、note_taker 导出均是同步），配合 2 MB 上限把风险控制在有界范围；
   项目 Code Review 规则要求 I/O 使用 `async` + `tokio::time::timeout`，本插件的同步实现已作为**例外**记录在
   `AGENTS.md` / `CLAUDE.md`（本地小文件同步 I/O 允许，但需有大小上限与错误处理；跨网络 / 流式 / 大文件仍必须 async + 超时）。
   保存前检查文件只读属性并拒绝，写入临时文件后 `sync_all()` 再 rename。
8. **路径健壮性**：统一使用 `PathBuf`；路径含空格/中文/长路径正常处理；
   打开时校验存在性与可读性，保存时校验可写性（权限不足时提示，例如 `C:\Program Files`、系统 hosts 目录）。
9. **列表交互细节**：点击已打开的文件直接激活对应标签（不重复读取磁盘）；
   从列表移除条目**不关闭**已打开的标签，也不删除磁盘文件；删除目录时其下条目移入「未分组」。
10. **热键补位**：插件数量从 7 增至 8，`AppSettings::default()` 的 `tool_hotkeys`
    与 `app_settings` 表默认值由 `1234567` 调整为 `12345678`；
    老用户数据加载后若热键数量少于插件数量，按未占用字符自动补位并记录 `info!` 日志，
    用户仍可在设置面板中改键。
11. **错误处理与日志**：所有文件读写与 SQL 操作返回 `anyhow::Result`；
    保存成功、失败、重载、编码拒绝等关键动作记录 `info!`/`warn!`/`error!`
    （日志已同时落盘到 `%APPDATA%\tools-box\logs\tools-box.log`，便于事后排查）。
12. **公共 API 文档注释**：新增的 `pub` 结构体与方法统一写 `///` 文档注释，
    `use` 语句集中在文件顶部，遵循 `cargo fmt` 与现有命名风格。
13. **语法高亮**：通过 `TextEdit::layouter` 接入 `utils::highlight`（syntect）；
    `HighlightJobCache` 以**文本内容本身**加（等宽字号、主题、语言）为键缓存 `LayoutJob`，
    同一内容不会每帧重新高亮，而编辑、重载、切换标签等任何文本变化都必然重新高亮
    （egui 会把 layouter 的返回值直接用于绘制与光标定位，缓存键里缺少文本身份会导致显示错内容或光标错位）。
    缓存 `LayoutJob` 而不是 `Galley`：布局与字形图集失效（缩放比例变化、图集重建）交给 epaint 自行处理，
    在外部缓存 `Galley` 会绕过这套失效契约。
    超过 `HIGHLIGHT_MAX_BYTES`（128 KB）的文本按纯文本布局以避免输入卡顿。
    窗口重新获得焦点时会检查活动标签的磁盘状态（仅在本插件处于渲染状态时执行），
    长时间停留同一标签也能发现外部修改。
14. **编辑区尺寸与滚动**：编辑器行数按可用高度换算（`editor_rows`，夹取 6~400 行、异常时回退默认值），
    编辑区始终占满可见区域；布局不做自动换行（`job.wrap.max_width = f32::INFINITY`），
    长行保持单行、宽度随内容增长，由外层 `ScrollArea::both()` 提供横向滚动条，
    窗口缩窄时不会自动折行改变行高。
15. **查找 / 替换**：查找栏由工具条「🔍 查找」或 **Ctrl+F** 打开；
    跳转键（类似 IDEA）：查找框内 **↑ / ↓ = 上一个 / 下一个**，**回车 = 下一个（与 ↓ 一致）**，
    Shift+回车 = 上一个；工具条上的跳转按钮为纯图标 `↑` / `↓`（悬停显示说明与对应按键）。
    跳转键在**渲染查找框之前**消费（`FIND_QUERY_ID_SALT` 记录的固定 Id 判断焦点）：
    单行输入框自己会吃掉回车并主动失焦，等控件处理完再判断就会导致"跳转一次后按键失效"；
    由于按键在控件之前被消费，跳转后焦点始终留在查找框，可以连续按 ↑/↓/回车逐处浏览。
    编辑器里的 ↑/↓ 不受影响（仍用于移动光标），因为查找框没有焦点时不做处理。
    查找按字符比较、可区分大小写（ASCII 走 `eq_ignore_ascii_case` 快路径，其余按 `to_lowercase` 折叠）。
    命中列表由 `find_all()` 一次扫描得出并由 `FindState::ensure_matches()` 缓存，
    仅在**文本 / 查找内容 / 大小写规则**变化时重算（编辑、替换、重载、切换标签都会置失效）；
    扫描过程中**递推**字符下标（`char_index`），不对每个命中回头做 `chars().count()`——
    否则复杂度会退化成 O(命中数 × 文本长度)，2 MB 文本 + 单字符查询可达秒级卡顿；
    关闭查找栏时会清空命中列表，之后既不高亮也不再扫描。
    导航只在列表中移动下标并环绕（末尾下一个回到第一个、开头上一个回到最后一个），
    因此查找栏可以同时显示 **「当前序号 / 匹配总数」**（如 `3/17`，未定位时显示 `0/总数`），
    命中数超过高亮上限时提示「仅高亮前 1000 处」。
    命中位置同时换算为**字符下标**（egui 选区按字符计数）与字节下标，前者写入 `TextEditState` 的选区，
    后者用于文本替换；egui 只在"本帧交互改变选区"时自动滚动，因此外部设置的查找选区需要用
    `galley.pos_from_ccursor()` 自行滚动到可见区域。
    编辑器使用显式 `Id`（`ui.make_persistent_id(Id::new(...))` + `TextEdit::id()`），
    状态读写的 Id 必须与控件实际使用的完全一致，否则选区会写到别的内存槽（`id_salt(&str)` 与
    `make_persistent_id(&str)` 得到的 Id 并不相同）。
    **Ctrl+F 冲突处理**：该组合键同时是 app 级"聚焦插件搜索框"的快捷键，约定为——
    编辑器（上一帧）有焦点、查找栏已打开、或焦点不在任何输入框时由本插件接管
    （`should_capture_find_shortcut()`：消费按键 + 打开查找栏 + 请求聚焦查找框）；
    焦点在**其它文本输入框**（全局搜索框、别的插件的输入框）时不消费，仍归全局搜索。
    判定必须使用**上一帧**的编辑器焦点状态：app 的 `handle_shortcuts` 在本插件之前执行，
    而 egui 的 `Memory::request_focus` 当帧立即生效——按 Ctrl+F 的那一帧焦点已被搜索框抢走，
    用当帧状态判断会永远判否（表现为"Ctrl+F 打不开查找栏，只有已打开时才聚焦查找框"）。
    本插件在 app 之后渲染，请求聚焦查找框的调用更晚，因此最终焦点落在查找框。
    只读预览（>2 MB）不允许替换；替换当前匹配后会继续查找下一处，并在提示中同时给出替换结果与下一处位置；
    文本被手动编辑、切换标签、重新打开文件、重载都会清空旧命中。
16. **命中高亮**：插件自绘**所有匹配**：非当前项用淡色铺底（`find_other_fill()`，每帧最多
    `MAX_HIGHLIGHTED_MATCHES = 1000` 个，避开当前项以免两层底色叠加），
    当前项用浓色 + 1px 描边画在最上层（`find_highlight_fill()` / `find_highlight_stroke()`），
    二者颜色明显区分（如 `3/17` 中被定位的那一处）。矩形由 `galley.pos_from_ccursor()` 求得。
    原因：egui 只在 `TextEdit` **获得焦点**时才绘制自身选区（`text_edit/builder.rs:723-730`），
    而查找时焦点在查找框内，只写 `TextEditState` 用户看不到高亮——
    选区仍会写入（编辑器一旦获得焦点即可直接编辑命中内容），可见性由自绘保证。
    底色必须用**非预乘** alpha 且保持低透明度：高亮叠在语法着色文字之上，底色太实会把正文压得看不清。
    多行命中的高亮为两端光标的包围盒（跨行时是整块区域），不做逐行拆分。
17. **行号列（一期）**：编辑器左侧固定宽度行号列（`LineNumberGutter`），
    做法与文本对比插件一致——行号列是文本滚动区的**兄弟节点**（`ui.horizontal_top` +
    `ui.allocate_space([gutter_width, editor_height])`），因此横向滚动长行时行号保持钉住；
    `TextEdit` 使用固定 `EDITOR_MARGIN = Margin::symmetric(4, 2)`（行号纵向对齐按它换算），
    同一帧用 **`ScrollArea::show` 的返回值**驱动：`state.offset.y` 是本次绘制文本所用的偏移，
    `inner_rect` 是文本视口（比滚动区外框准确——滚动条改成占位样式时外框会比视口高一截），
    因此不再手拼 `Id` 读 `scroll_area::State`，也就没有"Id 对不上导致偏移恒为 0"的隐患；
    行号只绘制可见行（`visible_line_range()`，上万行也只画可见部分）；
    行高按 `pixels_per_point` 取整（`monospace_row_height()`，与 epaint 对每行 y 的取整一致），
    且与 `desired_rows` 共用同一个值；行号列宽度用 `Fonts::glyph_width(&FontId::monospace(size), '0'..='9')`
    **实测字符步进**（不用"字号 × 0.6"估算，估算偏小会裁掉最高位）；
    宽度 = **左侧留白 + 位数 × 步进 + 右侧内缩** `LINE_NUMBER_INSET = 4.0`，
    其中左侧留白取 `内缩 + item_spacing.x`——因为数字右对齐到「列右边界 − 内缩」后，
    它与正文之间还有一条 `item_spacing`，左侧按同一数值留白，两侧看起来才一样宽（§12.13.4）；
    位数按实际行数（`line_number_digits()`，不设最小值）、颜色取主题弱化色、**不带分隔符**。
    行高有两个值，**用途不能混**：`TextEdit` 的 `desired_height = 行数 × row_height` 用的是
    **原始（未取整）值**（`builder.rs:540`），行号列逐行绘制必须用**按像素取整值**
    （epaint 把每行 y 对齐到整像素，`text_layout.rs:608/627-633`）。混用会让算出的行数偏多一行、
    内容比视口高（实测：视口 825、原始行高 16.313 ⇒ 51 行占 831.9 > 825，出现多余滚动条）；
    实现见 `monospace_row_heights()`（同时返回两个值）。
    行数用 `split('\n')` 统计（结尾换行之后的空行同样可以放光标，`lines()` 会漏掉它）。
    **行数换算还必须扣除 `TextEdit` 的上下内边距**（`editor_content_height()`）：控件外高是
    `max(文本高, 行数 × 行高) + 内边距`，直接按视口高度整除会让控件高出内边距之和
    （`Margin::symmetric(4, 2)` ⇒ 4px）。
    方向约定与 egui 自身一致：egui 在宽度方向就是 `available_width - margin.sum().x`
    （`builder.rs:512`），本项目只在高度方向补扣 `top + bottom`，宽度方向无需处理。
    边界说明：视口高度小于 `MIN_EDITOR_ROWS × 行高`（6 行）时最小行数下限会让内容仍然溢出，
    这是有意取舍——窗口太矮本来就该出现滚动条。
    **前提**：编辑器关闭了自动换行 ⇒ 文本行与排版行一一对应；若将来加入软折行，
    行号必须改为按 `galley.rows` 把视觉行映射回文本行。
    另一个已知前提是**等宽字体覆盖**：epaint 的行距取该行所有字形 `line_height` 的最大值
    （`text_layout.rs:605-606`），若某行含等宽字体缺字的字符（CJK / Emoji 走回退字体），
    该行行距会大于 `取整(等宽行高)` ⇒ 行号列从该行之后错位、内容也会略高于 `行数 × 行高`。
    当前配置下日常文本未出现该情况，若将来支持含 CJK 的大文件编辑，需要改为按 `galley.rows` 计算行位。
    当前行高亮与"点击行号选中整行"留待二期。

---

## 九、产出文件

| 文件 | 说明 |
|------|------|
| `src/plugins/file_editor/mod.rs` | 插件入口，`Plugin` trait 实现 |
| `src/plugins/file_editor/models.rs` | 数据结构与编码/换行符模型 |
| `src/plugins/file_editor/store.rs` | SQLite CRUD（目录、条目、插件设置） |
| `src/plugins/file_editor/document.rs` | 文件读取、编码/换行符探测、原子保存、外部修改检测 |
| `src/plugins/file_editor/ui.rs` | 布局、标签栏、编辑器、弹窗、快捷键 |
| `src/plugins/mod.rs` | 注册新插件（第 8 个） |
| `src/storage/database.rs` | 新增三张表与 `user_version` 3 迁移 |
| `src/plugins/settings/models.rs` | 默认 `tool_hotkeys` 补第 8 位 |
| `AGENTS.md` / `CLAUDE.md` | 插件列表新增本插件（本设计阶段已完成） |
| `FILE_EDITOR_DESIGN.md` | 本设计文档（实现完成后追加变更记录） |

---

## 十、测试计划

- [x] 目录新增、重命名、删除（删除后条目移入未分组）正常 —— `store::tests` 覆盖
- [x] 添加文件：重复路径（大小写不敏感）被拒绝、不存在路径有提示 —— 单测 + 表单校验覆盖
- [x] 打开文件：UTF-8 / UTF-8 BOM 内容正确显示，BOM 在保存后保留 —— `document::tests` 往返覆盖
- [x] 换行符：CRLF 与 LF 文件保存后风格不变 —— 单测覆盖（含混用判定）
- [x] 非 UTF-8 文件被拒绝编辑并给出明确提示 —— 单测覆盖
- [x] 保存：内容写回正确、磁盘状态更新、脏标记复位 —— 单测覆盖写回与磁盘长度；界面按钮待人工验证
- [x] 保存失败（只读 / 权限不足）时内容保留、错误可见 —— 代码路径已实现（失败写入标签提示且不关闭标签）
- [x] 多标签：打开、切换、关闭、激活态与脏标记显示正确 —— 关闭后激活下标单测覆盖
- [x] 关闭脏标签弹窗三选项行为正确（保存失败不关闭）—— 已实现，待人工验证
- [x] Ctrl+S 保存当前标签，且不影响 Ctrl+F/Esc 等既有快捷键 —— 插件内响应，未改动 app 级快捷键
- [x] 左栏宽度拖动生效并在重启后保持 —— `should_persist_width` 判定单测覆盖（拖动中不写库、松手后写库），
      读写单测覆盖（`store::tests`），界面拖动待人工验证
- [x] 外部修改检测：另一个程序改动文件后提示重载 / 自动重载 —— 已实现，待人工验证
- [x] 语法高亮：按扩展名 / 文件名着色，缓存布局（文本版本 + 宽度 + 字号 + 主题 + 语言为键），
      超过 128 KB 的文本跳过着色；状态栏同时显示推断出的语言名
- [x] 另存为：写入新路径并把标签切换到新文件 —— 单元测试覆盖标签更新（`apply_saved_as`），对话框待人工验证
- [x] 按最近打开排序：勾选后按 `last_opened_at` 降序、未打开过的按路径排在最后 —— 单元测试覆盖排序规则
- [x] 查找替换：工具条「🔍 查找」展开查找栏，支持上/下查找（环绕）、区分大小写、替换当前与全部替换，
      命中位置以字符下标设置编辑器选区并自动滚动到可见区域 —— 逻辑单测覆盖（含中文的字符/字节下标换算），
      界面交互待人工验证（不做 Ctrl+F 绑定：该组合键由 app 级快捷键用于聚焦全局插件搜索）
- [x] 超过 2 MB 的文件给出"过大只读"提示而非卡死 —— 单测覆盖
- [x] 数据库不可写时给出明确提示而不是隐晦的 `readonly database` 报错 —— 初始化探测 + 界面警告，单测覆盖
- [x] `cargo test`、`cargo build`、`cargo build --release` 通过；新增 40 项测试
      （`file_editor` 37 项 + 设置热键补位 3 项），覆盖编码 / 换行符 / 关闭决策 / 原子保存 / CRUD / 渲染流程 /
      高亮缓存 / 排序 / 另存为

---

## 十一、实施计划

| 阶段 | 内容 | 优先级 | 状态 |
|------|------|--------|------|
| 阶段一 | 插件骨架 + 数据库表 + 目录/文件条目管理 + 左右两栏布局 | P0 | ✅ 已完成 |
| 阶段二 | 单文件打开编辑 + 原子保存 + Ctrl+S + 状态栏信息 | P0 | ✅ 已完成 |
| 阶段三 | 多标签 + 未保存关闭提示 + 左栏宽度持久化 | P0 | ✅ 已完成 |
| 阶段四 | 语法高亮 + 外部修改检测、重载、过大文件保护 | P1 | ✅ 已完成 |
| 阶段五 | 查找替换、另存为、最近打开排序 | P2 | ✅ 已完成 |

阶段推进要求：每阶段完成后同步更新第六节状态列、第十节测试勾选情况，并在第十二节追加变更记录。

---

## 十二、变更记录

### 12.1 初始设计（2026-10-09）

- 完成插件功能需求、界面布局（左右两栏 + 多标签 + 三个弹窗）、数据结构、数据库表（三张表 + `user_version` 3）、
  目录结构、实现步骤、依赖、技术要点、产出文件、测试计划与阶段划分。
- 关键设计决策：
  1. 只做**无损**文本处理（UTF-8/BOM 保留、换行符保持、非 UTF-8 拒绝编辑），避免写坏用户配置文件；
  2. 保存采用**临时文件 + rename** 的原子替换，与 hosts 管理器、SFTP 下载的既有做法一致；
  3. 列表条目与磁盘文件解耦：移除条目不删除文件、不关闭标签；
  4. `hosts` 文件引导到 hosts 管理器编辑，避免绕过其环境合并逻辑；
  5. 左栏宽度等界面状态存入插件自己的设置表，不污染 `app_settings`。
- 同步更新：`AGENTS.md`、`CLAUDE.md` 的插件列表新增「常用文件编辑器」。
- 状态：设计完成，等待确认后进入阶段一实现。

### 12.2 阶段一~三实现完成（2026-10-10）

实现文件：

- `src/plugins/file_editor/mod.rs`：插件入口（`Plugin` trait，名称「常用文件编辑器」、图标 📄、懒加载数据库）
- `src/plugins/file_editor/models.rs`：目录/条目/标签模型、编码与换行符处理、hosts 路径识别、语言推断
- `src/plugins/file_editor/store.rs`：目录、条目、插件设置的 CRUD（路径按 `COLLATE NOCASE` 判重）
- `src/plugins/file_editor/document.rs`：文件读取（编码/换行符探测、2 MB 只读保护）、原子保存、磁盘状态比对
- `src/plugins/file_editor/ui.rs`：左右两栏布局、标签栏、编辑器、四个弹窗、Ctrl+S、延迟动作调度
- `src/plugins/mod.rs` / `src/storage/database.rs` / `src/plugins/settings/models.rs` / `src/main.rs`：注册第 8 个插件、
  三张表与 `user_version` 3 迁移、默认热键补位与启动时自动补齐

与 12.1 设计的差异（本文件已同步为最终形态）：

1. `LineEnding` 去掉 `Mixed` 变体：混用不单独建状态，`detect` 取多数、新增 `is_mixed` 判断并提示；
2. `EditorTab` 用 `disk: DiskState` + `read_only` + `message` 三个字段承载原设计的磁盘状态 / 只读 / 错误信息，
   不再单独保留 `disk_modified`、`disk_len`、`highlight`、`error`；
3. 语法高亮的语言推断保留为 `highlight_language()`（当前用于状态栏展示），着色本身留待阶段四；
4. 左侧目录折叠状态交由 egui 自身的 `CollapsingHeader` 记忆，未额外维护 `expanded` 字段；
5. 只创建 `idx_file_editor_files_dir` 索引，未创建按 `last_opened_at` 的索引（当前无按最近打开的排序需求）；
6. 新增"重命名显示名"弹窗（设计稿仅在右键菜单中提到），使 `update_alias` 有完整入口；
7. 只读文件（超限、非 UTF-8 由读取失败提示）在编辑器内以 `interactive(false)` 呈现，避免误编辑。

测试覆盖（新增 28 项）：

- `file_editor::models`：BOM/非 UTF-8 探测、编码输出、换行符主导风格与混用、标签标题脏标记、
  关闭提示判定、hosts 路径识别、语言推断
- `file_editor::document`：临时文件路径、磁盘状态比对、**BOM + CRLF 往返保存**、非 UTF-8 拒绝、超大文件只读、文件缺失报错
- `file_editor::store`：目录增删改、删除目录后条目移入未分组、路径大小写不敏感判重、显示名与最近打开、宽度默认值与持久化
- `file_editor::ui`：关闭标签后的激活下标规则（优先右邻 / 回退最后一个），以及
  **渲染冒烟测试**（无窗口环境渲染布局，并串起打开 → 置脏 → 关闭确认 → 保存（校验 CRLF 写回）→ 放弃关闭 → 非脏标签直接关闭全流程）
- `settings::models`：热键数量足够时不改动、补齐缺失字符、跳过已被其他工具占用的字符

验证说明：

```text
cargo test                      # 260 通过 / 1 忽略（原 232 + 新增 28）
cargo build                     # 无新增告警
cargo build --release           # 仅 api_tester 既有 7 项告警
rustfmt --edition 2024 <本次改动文件>   # 无差异
```

运行时冒烟验证（启动新构建后读取 `%APPDATA%\tools-box\logs\tools-box.log`）：

- `数据库已迁移到 v3（常用文件编辑器表）` —— 迁移执行成功
- `已注册 8 个插件` + `常用文件编辑器插件已初始化` —— 插件注册与初始化正常
- `已为新增工具补齐热键: pjHDTMZ1` —— 保留既有自定义热键（P/J/H/D/T/M/Z），新插件补到 `1`
- `注册热键成功: id=9, vk=0x31, plugin_index=7` —— 新插件可用 `Ctrl+Alt+1` 唤出

未完成项（阶段四 / 五）：语法高亮着色、查找替换、另存为、按最近打开排序；界面交互项（拖动调宽、
保存按钮、关闭确认弹窗、外部修改提示）已实现但需人工在界面上确认。

代码审查：见本次提交说明中的 code-reviewer 报告。

### 12.3 代码审查修复（2026-10-10）

code-reviewer 子代理审查结论：无阻塞项；2 项必须修复（左栏宽度持久化失效、超大文件只读分支整体读入内存）；
另 1 项规则决策待确认（同步 I/O 例外）；其余 10 条为建议项。

已修复：

1. **左栏宽度持久化失效**（审查问题 1）：原实现以"本帧宽度是否变化"作为落库条件，
   而拖动期间每帧都已更新内存宽度，松手帧宽度不再变化因而永不落库。现新增 `last_saved_width`
   与纯函数 `should_persist_width(pointer_down, current, last_saved)`：拖动中只更新内存，
   指针抬起后与"上次已保存宽度"比较并落库；补 `persists_width_only_after_drag_ends` 单测。
2. **超大文件只读分支整体读入内存**（审查问题 2）：改为 `File::open` + `Read::take(MAX_EDITABLE_BYTES)`
   只读前 2 MB；新增 `truncate_to_char_boundary` 按 UTF-8 字符边界截断（含单测，覆盖完整字符不被误删）；
   状态栏对只读标签显示「只读预览」而非编码 / 换行符；文件大小提示改用整数运算的 `format_megabytes`，
   一并去掉 `u64 → usize`、`u64 → f64` 的 `as` 转换。
3. **保存完整性**（建议 3）：保存前检查只读属性并拒绝（明确提示"文件带只读属性，未执行保存"）；
   临时文件写入后 `sync_all()` 再 rename，降低掉电截断风险。
4. **Ctrl+S 细节**（建议 4）：改用 `consume_key(Modifiers::CTRL, Key::S)` 消费按键；
   无活动标签或只读标签时不投递保存动作，只读标签的保存请求降级为 `info!`。
5. **状态栏易误解**（建议 10）：语言标注改为「推断语法 X」，与"尚未接入着色"的现状一致。
6. **测试补齐**（建议 8）：新增 `read_only_tab_refuses_save`（只读标签保存不写盘）、
   `save_failure_keeps_tab_open`（保存失败不关闭标签、保留确认框与未保存状态）、
   `truncates_incomplete_trailing_character`、`formats_megabytes_without_float_conversion`、
   `refuses_to_save_read_only_file`、`persists_width_only_after_drag_ends` 共 6 项。
7. **文档小疵**（建议 7）：界面效果图按钮改为与实现一致（去掉未实现的 `+ 新建目录`）；
   §8.6 的 Ctrl+S 描述与实现对齐。

待办（不阻塞本次，按审查建议留到阶段四 / 五）：

- **I/O 规则例外待拍板**：同步 I/O + 2 MB 上限（现状，§8.7 记为待确认例外）vs 后台线程 + 超时改造；
  若保留同步，需在 `AGENTS.md` 记录该例外；
- 热路径 `clone()` 优化（目录 / 条目每帧深拷贝，`display_name` 改返回 `Cow<'_, str>`）；
- 路径规范化（`canonicalize` / 统一分隔符）与唯一索引改 `COLLATE NOCASE`；
- 外部修改检测扩展到窗口重新获得焦点或定时比对，脏标签保存前提示"磁盘已变更，是否覆盖"；
- 同帧多动作改用稳定标签标识（路径 / 标签 id）替代下标；
- 语法高亮着色（阶段四）。

验证结果：

```text
cargo test            # 266 通过 / 1 忽略（12.2 的 260 + 本次新增 6）
cargo build           # 无新增告警
cargo build --release # 仅 api_tester 既有 7 项告警
cargo test file_editor # 31 项通过
```

### 12.4 数据库不可写自检与同步 I/O 例外确认（2026-10-10）

现象：在带 Low 完整性标签的工作区里运行 `target\release\tools-box.exe`（低完整性进程）时，
「新建目录」等写库操作报 `attempt to write a readonly database`，而读取与建表迁移都正常。

原因：`%APPDATA%\tools-box\data.db` 等文件没有完整性标签（按 Medium 处理），
低完整性进程受强制完整性策略 no-write-up 限制无法写入，SQLite 只能报只读。
迁移之所以看起来正常，是因为 `CREATE TABLE IF NOT EXISTS`（对象已存在时）与
`PRAGMA journal_mode = WAL`（已经是 WAL）都不产生实际写入，首次 `INSERT` 才真正进入写路径。

处置：

1. `store::probe_writable()`：初始化时用「`BEGIN IMMEDIATE` + 建探测表 + `ROLLBACK`」真实触发写路径，
   不留下任何修改；探测为 false 时插件记录 `error!` 日志并在界面顶部显示红色警告，
   提示把程序放到 `C:\Program Files\Tools-box` 等普通目录运行。新增单测 `detects_read_only_database`。
2. 运行位置：新构建部署到 `C:\Program Files\Tools-box\tools-box.exe`（无完整性标签），
   以普通权限启动后进程完整性为 Medium，实测写库成功（`WRITE_OK`）；
   工作区里的 `target\release\tools-box.exe` 只作为构建产物，不再日常运行。
3. 同步 I/O 例外（方案 A，已确认）：在 `AGENTS.md` 与 `CLAUDE.md` 的 Code Review 规则中记录
   「插件对本地小文件的读写允许在 UI 线程同步执行，但必须有明确的大小上限与错误处理；
   跨网络、流式或大文件 I/O 仍必须 async + 超时」。

验证结果：

```text
cargo test            # 267 通过 / 1 忽略（12.3 的 266 + 本次新增 1）
cargo build           # 无新增告警
cargo build --release # 仅 api_tester 既有 7 项告警
实测                   # C:\Program Files 下 Medium 进程写库成功；工作区 Low 进程写库报 readonly database
```

### 12.5 阶段四与阶段五（部分）实现（2026-10-10）

完成内容：

1. **语法高亮**：接入 `utils::highlight`（syntect），经 `TextEdit::layouter` 自定义布局；
   新增 `HighlightCache`，以**文本内容本身**加（布局宽度、等宽字号、主题、语言）为键缓存 `Arc<Galley>`，
   避免每帧重新高亮整篇文本，同时保证编辑 / 重载 / 切换标签后必定重新布局；
   超过 `HIGHLIGHT_MAX_BYTES`（128 KB）的文本按纯文本布局，防止大文件输入卡顿。
2. **外部修改检测扩展**：除打开文件与切换标签外，**窗口重新获得焦点**时也检查活动标签的磁盘状态，
   长时间停留同一标签也能发现外部改动；未修改自动重载，有本地修改则提示「重载」。
3. **另存为**：工具条新增「📄 另存为…」，写入用户选择的新路径（沿用原子保存与原编码 / 换行符），
   成功后把标签切换到新文件并清除未保存标记（`apply_saved_as`）；目标是 hosts 文件时仍引导到 hosts 管理器。
4. **最近打开排序**：工具条新增「⇅ 最近打开」开关，按 `last_opened_at` 降序排列，
   未打开过的文件按路径稳定排在最后（`sort_files_by_recent`）。

测试覆盖（新增 4 项）：`highlights_only_reasonable_text_sizes`、
`highlight_cache_reuses_layout_for_unchanged_text`（`Arc::ptr_eq` + 布局次数断言）、
`sorts_files_by_recent_open_time`、`applies_saved_as_target_to_tab`；
渲染冒烟测试同时断言"内容与宽度未变化时不重复布局"。

验证结果：

```text
cargo test            # 271 通过 / 1 忽略（12.4 的 267 + 本次新增 4）
cargo build           # 无新增告警
cargo build --release # 仅 api_tester 既有 7 项告警
file_editor 测试       # 36 项通过
```

剩余项：查找替换（阶段五）；审查报告中的优化项（热路径深拷贝、路径规范化、脏标签保存前的覆盖提示、
同帧多动作稳定标识等）。

### 12.6 阶段四 / 五 代码审查修复（2026-10-10）

审查结论：3 条阻塞项同一根因——高亮缓存用版本计数器代替"文本身份"；另有 2 条重要项。

已修复：

1. **高亮缓存改为内容键**（阻塞 1/2/3）：缓存键由 `EditorTab::revision` 计数器改为**文本内容本身**加
   （布局宽度、等宽字号、主题、语言）。原实现下：不同标签同为 `revision = 0` 时会命中别的标签的 galley
   而持续显示错误内容；`reload_tab`（手动与自动重载）替换文本却不改计数器，重载后仍显示旧内容；
   egui 在 `ui.add` 内部改动文本后会立刻再调用一次 layouter 并用返回值定位光标，命中旧 galley 会让
   文件末尾连续输入出现字符倒序。改为内容键后三类问题同时消失，`EditorTab::revision` 字段随之删除。
2. **另存为对只读预览禁用**（重要 4）：超过 2 MB 的文件只加载了前 2 MB，另存为会把截断内容写成完整文件；
   现在按钮对只读标签置灰，`save_tab_as` 内也会再次拒绝并提示。
3. **另存为的列表关联**（重要 5）：仅当目标路径与原路径不同才解除 `file_id`；目标命中列表已有条目时重新绑定，
   避免"另存为到原路径 / 已有文件"后点列表条目出现重复标签。
4. **clippy 清理**：合并焦点检测与"浏览…"按钮的 `collapsible_if`（let-chain）；
   两处测试清理代码的 `permissions_set_readonly_false` 显式 `allow` 并注明原因；本次新增代码 0 条 clippy 告警。

新增测试（+1）：`keeps_file_link_when_saved_as_same_path`；并强化 `highlight_cache_reuses_layout_for_unchanged_text`
（同长度不同内容必须重新布局——正是阻塞项的失败模式）与渲染冒烟测试（重载后缓存必须跟随新文本）。

验证结果：

```text
cargo test                 # 272 通过 / 1 忽略
cargo test file_editor      # 37 项通过
cargo build                # 无新增告警
cargo build --release      # 仅 api_tester 既有 7 项告警
cargo clippy --all-targets # file_editor 相关 0 条告警
```

仍待处理（下阶段）：每帧深拷贝（`Cow`）、路径规范化与唯一索引 `COLLATE NOCASE`、
脏标签保存前的"磁盘已变更"覆盖提示、同帧多动作的稳定标签标识、「⇅ 最近打开」开关持久化、
大文件撤销栈内存（egui 默认 100 步、每步持有整份文本）、查找替换；
插件未被渲染期间发生的磁盘改动暂不检查。

### 12.7 编辑区高度与滚动修复（2026-10-10）

界面实测反馈的问题：① 编辑区只占 `desired_rows(24)` 的最小高度，下方留大片空白；
② 窗口缩窄时文本自动换行、高度随内容变化，且不出现横向滚动条。

原因（对照 egui 0.31 源码确认）：`TextEdit` 的内容高度为
`max(galley 高度, desired_rows × row_height)`（`text_edit/builder.rs:540-541`），固定 24 行即最小高度；
多行布局默认按 `wrap_width = desired_width.min(available_width)` 折行
（`builder.rs:512-518`、默认 layouter `:524`）。

修复：

1. 新增 `editor_rows(available_height, row_height)`：按可用高度换算行数（夹取 6~400 行，
   行高或高度异常时回退默认 24 行），编辑区随面板高度铺满，长文本继续纵向滚动；
2. 自定义 layouter 改为 `job.wrap.max_width = f32::INFINITY`（不折行）：长行保持单行、宽度随内容增长，
   由外层 `ScrollArea::both()` 给出横向滚动条；
3. 布局宽度因此恒为无穷大，从高亮缓存键中移除 `width` 维度（`LayoutParams` 只保留语言 / 字号 / 主题）；
4. 同一轮代码审查发现：外部缓存 `Galley` 会绕过 epaint 的失效契约
   （`pixels_per_point` / `max_texture_size` 变化、字形图集重建时 epaint 会整体重建字体系统并丢弃 galley 缓存，
   见 `epaint-0.31.1/src/text/text_layout_types.rs:486-492`、`fonts.rs:447-462`），
   跨不同缩放比例的显示器拖窗口可能出现文字错位。因此缓存对象由 `Galley` 改为 `LayoutJob`
   （`HighlightJobCache`）：高亮结果仍被复用，布局与失效交给 epaint；每帧成本为 job 克隆与哈希。

测试：新增 `editor_rows_fills_available_height`（高度换算 / 最小行数 / 上限 / 负高度 / NaN / ∞ / 0 行高回退），
高亮缓存测试改名 `highlight_cache_reuses_job_for_unchanged_text` 并覆盖
"文本变化 / 语言变化必须重新高亮"与"长行不折行（galley 宽度 > 1000px）"。

验证：

```text
cargo test                 # 273 通过 / 1 忽略（file_editor 38 项）
cargo build                # 无新增告警
cargo build --release      # 仅 api_tester 既有 7 项告警
cargo clippy --all-targets # file_editor 相关 0 条告警
```

仍待处理（本插件下阶段）：极长单行（压缩 JSON 等）在不折行下的超宽 galley 与绘制精度，
可考虑加"软折行"开关或行长上限；其余见 §12.6 的待办清单。

### 12.8 查找替换（阶段五收尾，2026-10-10）

完成内容：

1. **查找栏**：工具条「🔍 查找」切换展开/收起，栏内含查找输入框、上/下一个、区分大小写、
   替换输入框、替换、全部替换与结果提示；输入框内回车即查找/替换。
   **不绑定 Ctrl+F**：该组合键由 app 级快捷键用于聚焦全局插件搜索框（`app.rs:592-593`），保持既有行为。
2. **查找逻辑**：按字符比较（ASCII 走 `eq_ignore_ascii_case`，其余按 `to_lowercase` 折叠），
   向下查找到末尾后环绕、向上查找到开头后环绕；命中位置同时给出**字符下标**（供 egui 选区）与字节下标（供替换）。
3. **选区与滚动**：把字符区间写入编辑器 `TextEditState`（编辑器使用固定 `id_salt` 以便定位状态）；
   由于 egui 只在"本帧交互改变选区"时自动滚动，外部设置的查找选区用 `galley.pos_from_ccursor()`
   计算光标位置后调用 `ui.scroll_to_rect()` 自行滚动到可见区域。
4. **替换**：替换当前匹配后把查找起点移到替换内容之后并继续查找；「全部替换」一次替换所有不重叠匹配并报告数量；
   替换会标记标签为未保存。文本被手动编辑、切换标签或重新打开文件时丢弃旧命中，避免替换到错误位置。

测试（新增 8 项）：`finds_matches_with_and_without_case`、`find_next_wraps_around_and_reports_char_indices`
（覆盖中文的字符/字节下标换算）、`find_prev_returns_nearest_before_and_wraps`、
`replaces_all_matches_without_overlap`、`counts_lines_for_match_position`、
`find_and_replace_flow_updates_tab`（打开 → 查找 → 渲染应用选区 → **断言 `TextEditState` 真收到选区** →
替换并跳下一处 → 全部替换）、`read_only_tab_refuses_replace`、`replaces_multibyte_match_and_advances_anchor`。
（其中 `find_next_wraps_around_and_reports_char_indices`、`find_prev_returns_nearest_before_and_wraps`
两个用例在 §12.9 改为命中列表后由 `find_all` 系列用例取代。）

代码审查修复（同一轮）：

1. **阻塞项**：选区写入用的 `Id` 与 `TextEdit` 实际使用的 `Id` 不一致 ——
   `TextEdit::id_salt(&str)` 内部是 `Id::new(salt)` 再经 `Ui::make_persistent_id`，
   而 `ui.make_persistent_id(&str)` 得到的是另一个 Id，导致命中处只滚动、不选中。
   现改为一次性算出 `editor_id = ui.make_persistent_id(Id::new(EDITOR_ID_SALT))`，
   控件用 `TextEdit::id(editor_id)`，状态读写与控件共用同一个 Id；
   并把该 Id 记录在界面状态里，使上述断言测试可复现该问题。
2. **只读标签不允许替换**：按钮按 `!read_only` 置灰并在处理函数内再次拒绝（否则会改写 2 MB 截断预览、
   置为未保存却无法保存），提示"只读预览不可替换"。
3. **大小写不敏感查找的 ASCII 快路径**：两边都是 ASCII 时只走 `eq_ignore_ascii_case`，
   避免在大文本上按字符做两次 `to_lowercase` 折叠（此前实现与本文档描述不符，现已一致）。
4. **命中状态清理**：`open_file`（命中已打开标签）、`reload_tab`、`close_tab` 统一调用 `clear_find_match()`，
   避免在另一份文本上设置错误选区；切换「区分大小写」也会清空旧命中。
5. 替换与"下一处命中"的提示合并为一条（如"已替换 1 处 · 下一处第 3 行"）；
   `line_number` 对非字符边界/越界偏移回退到文本末尾而不 panic。
6. **命中高亮改为自绘**（界面实测反馈"查找内容没有高亮成功"）：egui 仅在编辑器**获得焦点**时
   `paint_text_selection`（`builder.rs:723-730`），查找时焦点在查找框，因此只写 `TextEditState` 不可见。
   现由插件在文本之上绘制高亮（`match_rect()` + `find_highlight_fill()` / `find_highlight_stroke()`），
   选区仍写入编辑器状态（获得焦点后可直接编辑命中内容）；
   测试直接断言本帧输出里存在该高亮底色的 `Rect` 图元（`render_frame_output()` 返回 `FullOutput`）。
7. **高亮配色调整为"低透明度底 + 描边"**（界面实测反馈：亮色主题下黄褐色底把语法着色文字压得看不清）：
   底色改为非预乘 alpha 的低透明度色（暗色 `0xE0C040@0x38`、亮色 `0xB07A00@0x2E`）并加 1px 描边，
   既保证命中范围一眼可见，又不影响正文可读性；同时纠正了此前用
   `from_rgba_premultiplied` 传非预乘分量导致的底色过实问题。

验证：

```text
cargo test                 # 281 通过 / 1 忽略（file_editor 46 项）
cargo build                # 无新增告警
cargo build --release      # 仅 api_tester 既有 7 项告警
cargo clippy --all-targets # file_editor 相关 0 条告警
```

至此阶段五（P2）的三项全部完成，插件功能面覆盖阶段一~五；
剩余为工程性优化项，见 §12.6 待办清单与 §12.7 的极长单行说明。

### 12.9 查找体验优化（2026-10-12）

需求：显示匹配总数与当前选中的序号；高亮**所有**匹配项，并与当前选中项用不同颜色区分。

实现：

1. **命中列表**：新增 `find_all(text, query, match_case) -> Vec<MatchRange>`（一次扫描、命中不重叠），
   取代原先"每次查找重新扫描到下一个命中"的 `find_next` / `find_prev`；
   结果由 `FindState::ensure_matches()` 缓存，仅在文本 / 查找内容 / 大小写规则变化时重算
   （编辑、替换、重载、切换标签都会 `invalidate_matches()`）。
2. **序号与总数**：导航改为在命中列表内移动下标并环绕（`(i + 1) % total`、`(i + len - 1) % total`），
   未定位时按锚点选择起点（替换后从替换内容之后继续）；
   查找栏新增 `当前序号/匹配总数` 标签（如 `3/17`，无命中时显示 `0/0`），输入即时更新。
3. **高亮所有匹配**：非当前项用淡色铺底（`find_other_fill()`：暗色 `0xE0C040@9%`、亮色 `0xB07A00@7%`），
   当前项用浓色 + 描边画在最上层，两层颜色明显区分；单帧最多绘制 `MAX_HIGHLIGHTED_MATCHES`（1000）个匹配。

测试更新：`find_all` 覆盖大小写与字符/字节下标（含中文）、命中不重叠；
流程测试新增断言——命中总数为 2、序号依次为 1→2→环绕回 1、
本帧图元中恰好有 **1 个浓色高亮 + (匹配数 − 1) 个淡色高亮**、替换后命中列表重算为 1。

代码审查修复（同一轮）：

1. **阻塞项：`find_all` 的复杂度退化**。原先每个命中都调用 `match_range()` →
   `text[..byte].chars().count()`，整体 O(命中数 × 文本长度)：审查实测 240 KB / 4 万命中已需 317 ms，
   外推到 2 MB + 单字符查询是秒级同步冻结（而查找栏输入即时统计，每敲一个字符触发一次）。
   现改为**扫描时递推字符下标**（`start_char = char_index`、`end_char = char_index + 查询字符数`），
   复杂度回到 O(文本长度)，并删除不再使用的 `match_range()`；
   新增 `find_all_recurses_char_indices_across_many_matches` 用 2000 处命中与中文混排固化递推结果。
2. **重要项：关栏后高亮残留且会画在旧位置**。绘制入口原先只看 `matches` 非空，而 `matches`
   只在查找栏可见时刷新 → 关栏后编辑/切换标签会按旧下标在新布局上画高亮且永不消失。
   现改为：绘制条件加 `find.visible`，绘制前用当前文本 `ensure_matches()` 校正（命中缓存时 O(1)），
   关栏时清空命中列表；新增 `closing_find_bar_clears_highlights` 断言"关栏 → 编辑 → 无高亮图元"。
3. 序号/下标查询由线性 `position` 改为按有序列表 `partition_point` 二分（每帧与每次导航各一次）；
   淡色高亮透明度略微上调（暗色 12%、亮色 10%）以保证亮色主题下可见；
   命中数超过上限时提示"仅高亮前 1000 处"；测试期望值按"实际绘制窗口"计算。

验证：

```text
cargo test                 # 282 通过 / 1 忽略（file_editor 47 项）
cargo build                # 无新增告警
cargo build --release      # 仅 api_tester 既有 7 项告警
cargo clippy --all-targets # file_editor 相关 0 条告警
```

### 12.10 查找交互精简与 Ctrl+F 接管（2026-10-12）

需求：上一个 / 下一个按钮只保留图标；用 ↑ / ↓ 图标按钮跳转；支持 Ctrl+F 打开查找，
并处理好与 app 级"聚焦插件搜索框"共用该快捷键的冲突。

实现：

1. **按钮图标化**：`[↑ 上一个]` / `[↓ 下一个]` → 纯图标 `↑` / `↓`，说明文字移到悬停提示
   （`on_hover_text("上一个匹配")` / `("下一个匹配（回车）")`），查找栏更紧凑。
2. **键盘跳转**：查找输入框内 `回车` = 下一个、`Shift+回车` = 上一个（提示文案同步更新）。
3. **Ctrl+F 接管**：新增 `should_capture_find_shortcut(editor_focused, find_visible)`——
   只有**编辑器有焦点或查找栏已打开**时才由本插件接管（`input.consume_key(CTRL, F)` →
   打开查找栏 → 请求聚焦查找框）；焦点在侧栏 / 搜索框 / 其他插件时不消费，仍归 app 级全局搜索。
   本插件在 `App::handle_shortcuts` 之后渲染，egui 的焦点以**最后一次请求**为准，
   因此同一帧内 app 先请求聚焦搜索框、插件随后请求聚焦查找框，最终焦点落在查找框（无闪烁）。
   工具条「🔍 查找」打开时同样请求聚焦查找框。

测试（新增 2 项）：`captures_find_shortcut_only_in_editor_context`（接管条件）、
`ctrl_f_opens_find_bar_when_editor_focused`（真实注入 `Ctrl+F` 按键事件，
断言查找栏打开且焦点请求被消费）。

验证：

```text
cargo test                 # 284 通过 / 1 忽略（file_editor 49 项）
cargo build                # 无新增告警
cargo build --release      # 仅 api_tester 既有 7 项告警
cargo clippy --all-targets # file_editor 相关 0 条告警
```

### 12.11 Ctrl+F 打不开查找栏的修复（2026-10-12）

界面实测反馈：编辑器有焦点时 Ctrl+F **打不开**查找栏，而查找栏已打开时 Ctrl+F 能聚焦到查找框。

原因：`App::handle_shortcuts` 在本插件之前执行，它一看到 Ctrl+F 就把焦点交给全局搜索框；
egui 的 `Memory::request_focus` 是**当帧立即生效**的（`focus.focused_widget = Some(...)`），
所以轮到本插件渲染时 `has_focus(编辑器)` 已经是 false → 接管判定失败。
查找栏已打开时之所以还能聚焦，是因为判定里的 `find_visible` 命中了。

修复：新增 `FileEditorUi::editor_had_focus` 记录**上一帧**的编辑器焦点状态，
接管判定改用该值；同时把"焦点在其它文本输入框时不接管"的条件改为基于
`ctx.wants_keyboard_input()` 与上一帧状态组合，避免把"刚被搜索框抢走焦点"误判成"用户正在搜索框里输入"。
测试 `ctrl_f_opens_find_bar_when_editor_focused` 升级为**复现该场景**：先让编辑器获得焦点并渲染一帧，
再模拟 app 抢先聚焦搜索框，然后注入 `Ctrl+F`（含 Windows 真实的 `CTRL|COMMAND` 修饰键），
断言查找栏必须打开——修复前该用例失败。

同时新增 `editor_widget_holds_keyboard_focus_with_recorded_id`：给记录的编辑器 Id 聚焦后注入文本事件，
断言输入确实落到编辑器上，用以固化"记录的 Id 与控件真实 Id 一致"这一前提。

验证：

```text
cargo test                 # 285 通过 / 1 忽略（file_editor 50 项）
cargo build                # 无新增告警
cargo build --release      # 仅 api_tester 既有 7 项告警
cargo clippy --all-targets # file_editor 相关 0 条告警
```

### 12.12 查找跳转改为 ↑/↓ 键并保持焦点（2026-10-12）

界面实测反馈：跳转键应为**键盘 ↑/↓**（而非回车 / Shift+回车）；且回车跳转后查找框**失去焦点**，
再按任何键都无效，必须重新点击查找框。

原因：原实现用 `query.lost_focus() && key_pressed(Enter)` 判定——单行 `TextEdit` 处理回车时
会**消费该事件并主动交还焦点**，因此判定只在"已经失焦"的那一帧成立，跳转一次后焦点就不在查找框里了。

修复：跳转键改为在**渲染查找框之前**处理——用 `FIND_QUERY_ID_SALT` 的固定 Id 判断查找框是否持有焦点，
是则先 `consume_key` 掉 `↑`/`↓`/`回车`/`Shift+回车`（控件因此看不到这些键，不会再失焦），
再投递上/下跳转动作；查找框自身用 `.id(query_id)` 固定 Id。
约定与 IDEA 一致：**↓ / 回车 = 下一个，↑ / Shift+回车 = 上一个**；焦点全程留在查找框，可连续按键逐处浏览。
编辑器里的 ↑/↓ 不受影响（查找框无焦点时不处理）。

测试（新增 1 项）：`find_shortcuts_keep_focus_and_jump_repeatedly` —— Ctrl+F 打开查找栏后
**连续三次回车**依次命中 1/2/3（同时验证了"焦点没有丢"），再用 `↓` 环绕到第 1 个、`↑` 环绕到最后一个，
并断言跳转后焦点没有跑到编辑器。修复前该用例在第一次回车后即失败。

验证：

```text
cargo test                 # 286 通过 / 1 忽略（file_editor 51 项）
cargo build                # 无新增告警
cargo build --release      # 仅 api_tester 既有 7 项告警
cargo clippy --all-targets # file_editor 相关 0 条告警
```

### 12.13 行号列（一期，2026-10-12）

需求与已确认的设计点：编辑器左侧显示行号；**不带 `│` 分隔符**；宽度**按实际位数**（不设最小值）；
**当前行高亮放到二期**；与查找序号无冲突（两套绘制互不干扰）。

实现（复用文本对比插件已跑通的做法，见 §8 技术要点 17）：
与文本滚动区并排的行号列（横向滚动时钉住）、固定 `EDITOR_MARGIN`、同一帧读回滚动偏移、
等行高只画可见行、行高按像素密度取整、行号用主题弱化色、行数按 `split('\n')` 统计。

测试（新增 5 项）：`counts_lines_including_trailing_empty_line`、
`line_number_digits_use_actual_count`、`gutter_width_grows_with_digits`、
`visible_line_range_only_covers_visible_rows`（含 NaN / 零行高 / 零高度 / 滚过末尾等边界）、
`renders_line_numbers_for_open_tab`（渲染一帧后断言图元里确实画出了 `1`/`2`/`3`/`4` 号行号，且正文仍在）。

验证：

```text
cargo test                 # 295 通过 / 1 忽略（file_editor 56 项）
cargo build                # 无新增告警
cargo build --release      # 仅 api_tester 既有 7 项告警
cargo clippy --all-targets # file_editor 相关 0 条告警
```

遗留（二期）：当前行高亮、点击行号选中整行、相对行号；以及"未来若加入软折行需改按 `galley.rows` 映射"。

#### 12.13.1 审查反馈修复（同轮）

code review 结论为"可提交，无阻塞"，其中重要项与若干建议已在同轮落实：

1. **重要项：补"行号与文本对齐"的位置断言**。原用例只断言图元里出现行号文本，不断言坐标，
   三类真实缺陷会静默通过（滚动偏移没用上、内边距不一致、行高未取整导致逐行漂移）。
   现断言：行号 `1` 与正文首行 y 相差 ≤1px、行号 x 在正文左侧、相邻行号间距完全一致。
   **反证检查**：把行号顶部偏移故意改成 `6.0` 后该用例立刻失败
   （`行号与正文首行必须对齐：行号 y=95, 正文 y=91`），确认断言不是空断言。
2. **改用 `ScrollArea::show` 的返回值**驱动行号（`state.offset.y` + `inner_rect`），
   删掉手拼 `Id` 读 `scroll_area::State` 与"用滚动区外框高度"的隐含前提——
   一旦滚动条改成占位样式（`solid()/thin()`），外框会比文本视口高一截、在滚动条区域多画一条行号。
3. 横向布局改为显式 `ui.horizontal_top`（原来靠子项恰好等高才等价于顶对齐）。
4. 行号列宽度改用 `Fonts::glyph_width` 实测字符步进（原 0.6 估算偏小会裁掉最高位）。
5. `LINE_NUMBER_GAP` 改名 `LINE_NUMBER_RIGHT_INSET = 4.0`（原注释说"间隙 8px"却只用了一半）
   ——该常量后续又改名为 `LINE_NUMBER_INSET`（左右各留一份），见 §12.13.3。
6. 浮点上限常量由整数上限派生（`MAX_REASONABLE_LINES as f32`），避免两个常量失配。
7. 行高只算一次，供行号列与 `desired_rows` 共用（原来分别用了取整/未取整两个值）。

未采纳：行数每帧全文扫描（2 MB 只读预览约 1ms/帧）暂未缓存，列为本插件后续可选优化。

验证：

```text
cargo test                 # 295 通过 / 1 忽略（file_editor 56 项；新增能力补齐在既有用例内）
cargo build                # 无新增告警
cargo clippy --all-targets # file_editor 相关 0 条告警
```

#### 12.13.2 修复内容高度多出内边距导致的常驻滚动条（2026-10-12）

界面实测反馈：**内容明显没占满时，编辑区右侧仍常驻一条纵向滚动条**。

原因：`editor_rows()` 按视口高度整除得到行数，而 `TextEdit` 的控件外高是
`max(文本高, 行数 × 行高) + 内边距`（`egui-0.31.1/src/widgets/text_edit/builder.rs:540-543`），
于是控件每帧都比视口高出一个内边距之和（`EDITOR_MARGIN = Margin::symmetric(4, 2)` ⇒ 4px），
滚动区据此判定"内容超高"，滚动条因此常驻（文本很短时滑块几乎占满轨道）。
该问题与行号无关且早于行号改动存在（`Margin::symmetric(4, 2)` 本就是 egui 的默认内边距），
只是行号列上线后被一并观察到。

修复分两步（实测确认第二步才是决定性的）：

1. **扣除上下内边距**：新增 `editor_content_height(viewport_height, margin)`，行数换算改用
   扣除上下内边距后的高度；
2. **行数换算改用 `TextEdit` 内部的原始行高**：实测日志（1920×1080@100%，编辑器视口 825）：

   ```text
   [编辑器] editor_height=825.0 inner_available=825.0 row_height=16.000 rows=51
            content=1495.4x835.9 viewport=1495.4x825.0 行数=25
   ```

   内容高 835.9 − 4(内边距) = 831.9 = **51 × 16.313**——即 `TextEdit` 的
   `desired_height = 行数 × row_height` 用的是**未取整**的 16.313，而项目里按像素取整成 16.0 算行数，
   于是 51 行实际占 831.9 > 825，滚动条常驻。现改为 `monospace_row_heights()` 同时返回两个值，
   行数换算取**两者的较大者**（`TextEdit` 的 desired_height 用原始值、实际排版行距是取整值，
   两者都要受控），行号列逐行绘制固定用取整值。这也解释了"默认高度没事、最大化后出现"：
   只有视口高度落在 `行数 × 取整行高 < 视口 < 行数 × 原始行高` 这个窗口时才会溢出。
   反向窗口（原始行高小数 ≥ 0.5，取整值更大）同样被"取较大者"覆盖。

测试（新增 2 项）：`editor_content_height_reserves_text_edit_margin`（正常值、恰好等于内边距、
小于内边距、`NaN`、负值归零）、`editor_rows_with_margin_never_overflows_viewport`
（回归断言：`行数 × 行高 + 内边距 ≤ 视口高度`，含用户机器的真实参数 `视口 825 / 行高 16.313`
与行高整数倍视口 `561 = 17 × 33`，后者可捕获"忘记扣除内边距"的回退）。

验证：

```text
cargo test                 # 297 通过 / 1 忽略（file_editor 58 项）
cargo build                # 无新增告警
cargo clippy --all-targets # file_editor 相关 0 条告警
```

已部署（`C:\Program Files\Tools-box\tools-box.exe`），**最大化的目视确认待用户反馈**；
底部留白可接受：用户参数下行数 50 ⇒ 内容 819.65 vs 视口 825，余量不足一行。

#### 12.13.3 行号列左侧补留白（2026-10-12）

界面反馈（滚动条问题解决后）：行号数字**紧贴左侧分割线**，左侧没有留白。

修复：`LINE_NUMBER_RIGHT_INSET` 改为 `LINE_NUMBER_INSET = 4.0` 并在**列宽左右各用一份**
（`gutter_width = 内缩 × 2 + 位数 × 字符步进`），数字仍右对齐到「列右边界 − 内缩」，
左侧因此获得与右侧相同的 4px 留白。测试断言同步更新为"一位数字时宽度 = 步进 + 左右各一份内缩"。

验证：`cargo test` 297 通过 / 1 忽略（file_editor 58 项）、`cargo clippy --all-targets`
file_editor 相关 0 条告警、`cargo build` 无新增告警。

#### 12.13.4 行号列左侧留白改为与"数字到正文的间距"等宽（2026-10-12）

界面反馈（§12.13.3 之后）：左侧留白看起来仍比右侧窄——数字与分割线的距离明显小于数字到正文的距离。

原因：§12.13.3 只让**数字自身**左右各留 4px，但数字右边缘与**正文**之间还有一条
`item_spacing.x`（默认 8px），因此视觉上"右侧间距"是 `4 + 8 = 12px`，左侧只有 4px。

修复：`line_number_gutter_width(digits, char_width, item_spacing_x)` 的左侧留白改为
`LINE_NUMBER_INSET + item_spacing_x`（数字右对齐落点不变，仍为「列右边界 − 内缩」），
使**左侧留白 == 数字到正文的视觉间距**，两侧看起来一样宽；`line_number_x()` 与
`line_number_x_keeps_symmetric_inset` 测试同步按该口径断言。

验证：`cargo test` 298 通过 / 1 忽略（file_editor 59 项）、`cargo clippy --all-targets`
file_editor 相关 0 条告警、`cargo build` 无新增告警。
