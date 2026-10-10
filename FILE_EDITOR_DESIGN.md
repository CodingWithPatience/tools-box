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
│  📄 常用文件编辑器   [ + 添加文件 ] [ + 新建目录 ] [ 🗂 目录管理 ] [ 💾 保存 ] [ ⟳ 重载 ] │
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

/// 换行符风格，保存时按原文件风格写回
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineEnding {
    Lf,
    CrLf,
    /// 混用：以多数行为准，保存时统一为主导风格并在状态栏提示
    Mixed,
}

/// 一个已打开的文件标签
#[derive(Debug, Clone)]
pub struct EditorTab {
    /// 对应列表条目 id，None 表示未纳入列表（例如临时打开）
    pub file_id: Option<i64>,
    pub path: PathBuf,
    pub display_name: String,
    /// 编辑器当前内容（含原始换行符）
    pub text: String,
    /// 内容与磁盘版本是否不一致
    pub dirty: bool,
    /// 打开/保存时记录的磁盘状态，用于外部修改检测
    pub disk_modified: Option<SystemTime>,
    pub disk_len: u64,
    pub encoding: TextEncoding,
    pub line_ending: LineEnding,
    /// 高亮语言（由扩展名推断），None 为纯文本
    pub highlight: Option<String>,
    /// 读取/保存失败原因，失败后标签保留内容但只读提示
    pub error: Option<String>,
}

/// 关闭标签时的用户选择
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseDecision {
    Save,
    Discard,
    Cancel,
}

/// 待确认关闭的标签（弹窗状态）
#[derive(Debug, Clone, Copy)]
pub struct PendingClose {
    pub tab_index: usize,
}

/// 插件整体状态
pub struct FileEditorState {
    pub directories: Vec<FileDirectory>,
    pub files: Vec<FavoriteFile>,
    /// 左侧列表展开的目录 id
    pub expanded: Vec<i64>,
    pub tabs: Vec<EditorTab>,
    pub active_tab: Option<usize>,
    pub selected_file_id: Option<i64>,
    pub pending_close: Option<PendingClose>,
    pub dialog: FileDialogState,
    pub status: String,
    /// 左栏宽度，来自插件设置表
    pub list_width: f32,
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
CREATE INDEX IF NOT EXISTS idx_file_editor_files_opened ON file_editor_files(last_opened_at);

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
| 1 | 创建插件骨架 | `file_editor/` 目录、`Plugin` trait 实现、注册到 `plugins/mod.rs` | ⬜ |
| 2 | 数据库表与迁移 | 三张表 + `user_version` 3，`store.rs` 提供 CRUD | ⬜ |
| 3 | 目录管理 | 新建/重命名/删除目录，删除时文件移入未分组 | ⬜ |
| 4 | 文件条目管理 | 添加（rfd 选择 + 手输路径）、显示名、移除、去重校验 | ⬜ |
| 5 | 左右两栏布局 | `SidePanel::left(...).resizable(true)` + 宽度持久化 | ⬜ |
| 6 | 打开与编辑 | 点击条目读取文件、`TextEdit::multiline` 编辑、状态栏信息 | ⬜ |
| 7 | 保存写回 | 原子替换保存、错误处理、脏标记与磁盘状态更新 | ⬜ |
| 8 | 多标签 | 标签栏、切换、关闭、激活态与脏标记圆点 | ⬜ |
| 9 | 未保存提示 | 关闭脏标签弹窗（保存/不保存/取消） | ⬜ |
| 10 | Ctrl+S 快捷键 | 编辑器聚焦时保存当前标签 | ⬜ |
| 11 | 默认热键与设置 | `tool_hotkeys` 补第 8 个默认热键，设置面板自动补位 | ⬜ |
| 12 | 语法高亮与重载 | 扩展名 → `utils::highlight`，外部修改检测与手动重载 | ⬜ |
| 13 | 测试与文档同步 | 单元测试、`cargo build`/`cargo test`、更新本设计文档 | ⬜ |

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

1. **保存策略（原子替换）**：写入同目录临时文件 `*.tmp-toolsbox` → `std::fs::rename` 覆盖原文件，
   与 `hosts_manager/parser.rs`、`ssh_client/sftp.rs` 的既有做法一致；失败时保留临时文件路径与错误原因，
   编辑器内容不丢。
2. **编码处理**：只接受 UTF-8（含 BOM）。带 BOM 时保存写回 BOM，无 BOM 时不添加。
   探测到非 UTF-8（如 GBK/UTF-16）时**拒绝编辑**并在状态栏提示，不做静默转换，避免写坏配置文件。
3. **换行符保持**：读取时统计 CRLF/LF 主导风格；保存时统一按主导风格写回（混用时提示已统一）。
   避免"编辑一行、整个文件 diff 翻天"。
4. **脏标记与外部修改检测**：`dirty = 编辑内容 != 磁盘内容`（保存/重载后复位）；
   打开时记录 `disk_modified`/`disk_len`，切换到该标签或窗口重新获得焦点时比对，
   变化则提示「文件已被外部修改，是否重新加载？」，用户可选择保留本地编辑。
5. **多标签与关闭流程**：`tabs: Vec<EditorTab>` + `active_tab`，关闭前若 `dirty` 则进入
   `PendingClose` 弹窗；选择「保存」时保存失败则**不关闭**标签并展示错误。
6. **Ctrl+S**：在插件 `render` 内响应 `i.key_pressed(egui::Key::S) && i.modifiers.ctrl`，
   仅在存在活动标签时消费；不与 app 级 `Ctrl+F`/`Esc`（`App::handle_shortcuts`）冲突。
7. **文件大小与 UI 卡顿**：单文件超过 2 MB 时提示"文件过大，本插件不提供编辑"（只读展示前 2 MB），
   避免同步 I/O 与 egui 布局卡死。
   I/O 沿用现有插件的同步读写方式（hosts 写入、note_taker 导出均是同步）；
   若后续需要大文件或网络路径（`\\server\share`），改为后台线程 + 结果回传（参考 note_taker 图片加载）。
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

- [ ] 目录新增、重命名、删除（删除后条目移入未分组）正常
- [ ] 添加文件：文件选择器与手输路径均可用；重复路径被拒绝；不存在路径有提示
- [ ] 打开文件：UTF-8 / UTF-8 BOM 内容正确显示，BOM 在保存后保留
- [ ] 换行符：CRLF 与 LF 文件保存后风格不变
- [ ] 非 UTF-8 文件被拒绝编辑并给出明确提示
- [ ] 保存：内容写回正确，磁盘 mtime 更新，脏标记复位
- [ ] 保存失败（只读/权限不足）时内容保留、错误可见
- [ ] 多标签：打开、切换、关闭、激活态与脏标记显示正确
- [ ] 关闭脏标签弹窗三选项行为正确（保存失败不关闭）
- [ ] Ctrl+S 保存当前标签，且不影响 Ctrl+F/Esc 等既有快捷键
- [ ] 左栏宽度拖动生效并在重启后保持
- [ ] 外部修改检测：另一个程序改动文件后提示重新加载
- [ ] 语法高亮：`.toml`/`.json`/`.rs` 等着色，未知扩展名回退纯文本
- [ ] 超过 2 MB 的文件给出"过大只读"提示而非卡死
- [ ] `cargo test`、`cargo build`、`cargo build --release` 通过；新增测试覆盖编码/换行符/脏标记/关闭决策

---

## 十一、实施计划

| 阶段 | 内容 | 优先级 |
|------|------|--------|
| 阶段一 | 插件骨架 + 数据库表 + 目录/文件条目管理 + 左右两栏布局 | P0 |
| 阶段二 | 单文件打开编辑 + 原子保存 + Ctrl+S + 状态栏信息 | P0 |
| 阶段三 | 多标签 + 未保存关闭提示 + 左栏宽度持久化 | P0 |
| 阶段四 | 语法高亮 + 外部修改检测 + 重载 + 过大文件保护 | P1 |
| 阶段五 | 查找替换、另存为、最近打开排序、文档同步与打磨 | P2 |

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
