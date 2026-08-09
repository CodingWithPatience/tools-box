# 临时笔记插件设计方案

## 一、功能需求

| 功能 | 说明 | 优先级 |
|------|------|--------|
| 目录分类 | 支持创建多级目录，按目录组织笔记 | P0 |
| 笔记管理 | 创建、编辑、删除笔记 | P0 |
| Markdown 支持 | 支持 Markdown 格式编辑和预览 | P0 |
| 搜索功能 | 按标题和内容搜索笔记 | P1 |
| 收藏功能 | 标记重要笔记为收藏 | P1 |
| 标签功能 | 为笔记添加标签分类 | P1 |
| 排序功能 | 按创建时间、更新时间、标题排序 | P2 |
| 导入导出 | 批量导入导出笔记 | P2 |

---

## 二、界面设计

### 2.1 主界面布局

```
┌─────────────────────────────────────────────────────────────────────┐
│  📝 临时笔记                [ + 新建笔记 ] [ + 新建目录 ] [ 🔍 搜索 ] │
├──────────────┬──────────────────────────────────────────────────────┤
│              │                                                      │
│  目录树：     │  标题：[ 我的笔记标题                               ] │
│  ┌────────┐  │                                                      │
│  │ 📁 全部  │  │  [ Markdown 编辑 ] [ 预览 ]                         │
│  │ 📁 收藏  │  │  ┌──────────────────────────────────────────────┐ │
│  │ 📁 工作  │  │  │                                              │ │
│  │  ├─ 会议  │  │  │  # 标题                                       │ │
│  │  └─ 待办  │  │  │                                              │ │
│  │ 📁 学习  │  │  │  这是一段 **Markdown** 文本...               │ │
│  │  ├─ Rust  │  │  │                                              │ │
│  │  └─ 前端  │  │  │                                              │ │
│  │ 📁 个人  │  │  │                                              │ │
│  └────────┘  │  │  └──────────────────────────────────────────────┘ │
│              │                                                      │
│  最近笔记：   │  目录：[📁 工作 > 会议 ▼]                           │
│  ┌────────┐  │  标签：[Rust] [会议] [+]                            │
│  │ 会议记录  │                                                      │
│  │ 学习笔记  │  [ 💾 保存 ] [ 🗑 删除 ] [ ⭐ 收藏 ]                │
│  │ ...      │                                                      │
│  └────────┘  │                                                      │
│              │                                                      │
├──────────────┴──────────────────────────────────────────────────────┤
│  共 12 条笔记 | 最后更新：2024-01-15 10:30                          │
└─────────────────────────────────────────────────────────────────────┘
```

### 2.2 目录管理弹窗

```
┌───────────────────────────────────────┐
│  目录管理                    [ ✕ ]    │
├───────────────────────────────────────┤
│                                       │
│  目录名称：[ 新目录名              ]   │
│  上级目录：[ 根目录 ▼ ]               │
│                                       │
│  现有目录：                            │
│  ┌─────────────────────────────────┐ │
│  │ 📁 工作                         │ │
│  │   📁 会议                       │ │
│  │   📁 待办                       │ │
│  │ 📁 学习                         │ │
│  │   📁 Rust                       │ │
│  │   📁 前端                       │ │
│  └─────────────────────────────────┘ │
│                                       │
│  [ 确定 ] [ 取消 ] [ 删除选中目录 ]   │
│                                       │
└───────────────────────────────────────┘
```

---

## 三、数据结构设计

### 3.1 核心数据结构

```rust
/// 目录结构
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NoteFolder {
    pub id: i64,
    pub name: String,
    pub parent_id: Option<i64>,  // None 表示根目录
    pub sort_order: i32,
    pub created_at: String,
}

/// 笔记条目
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NoteEntry {
    pub id: i64,
    pub title: String,
    pub content: String,
    pub folder_id: Option<i64>,  // None 表示未分类
    pub is_favorite: bool,
    pub tags: Vec<String>,
    pub created_at: String,
    pub updated_at: String,
}

/// 笔记表单（编辑用）
#[derive(Debug, Clone)]
pub struct NoteForm {
    pub title: String,
    pub content: String,
    pub folder_id: Option<i64>,
    pub tags: String,  // 逗号分隔
}

/// 排序方式
#[derive(Debug, Clone, PartialEq)]
pub enum SortBy {
    CreatedAt,
    UpdatedAt,
    Title,
}

/// 视图模式
#[derive(Debug, Clone, PartialEq)]
pub enum NoteViewMode {
    Edit,    // 编辑模式
    Preview, // Markdown 预览
}
```

---

## 四、数据库设计

```sql
-- 笔记目录
CREATE TABLE note_folders (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    name       TEXT NOT NULL,
    parent_id  INTEGER,
    sort_order INTEGER DEFAULT 0,
    created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY (parent_id) REFERENCES note_folders(id) ON DELETE CASCADE
);

-- 笔记条目
CREATE TABLE note_entries (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    title      TEXT NOT NULL,
    content    TEXT NOT NULL DEFAULT '',
    folder_id  INTEGER,
    is_favorite BOOLEAN DEFAULT FALSE,
    tags       TEXT DEFAULT '',  -- 逗号分隔的标签
    created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
    updated_at DATETIME DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY (folder_id) REFERENCES note_folders(id) ON DELETE SET NULL
);

CREATE INDEX idx_note_entries_folder ON note_entries(folder_id);
CREATE INDEX idx_note_entries_favorite ON note_entries(is_favorite);
CREATE INDEX idx_note_entries_updated ON note_entries(updated_at);
```

---

## 五、目录结构

```
src/plugins/note_taker/
├── mod.rs          # 插件入口
├── ui.rs           # UI 渲染逻辑
├── models.rs       # 数据结构定义
├── store.rs        # 数据库 CRUD 操作
└── markdown.rs     # Markdown 渲染
```

---

## 六、实现步骤

| 步骤 | 任务 | 说明 | 状态 |
|------|------|------|------|
| 1 | 创建插件骨架 | 目录结构、Plugin trait 实现 | ✅ |
| 2 | 数据库初始化 | 创建 note_folders 和 note_entries 表 | ✅ |
| 3 | 实现目录管理 | 目录树展示、新增/编辑/删除目录 | ✅ |
| 4 | 实现笔记列表 | 按目录筛选笔记列表 | ✅ |
| 5 | 实现笔记编辑 | 标题、内容编辑器 | ✅ |
| 6 | 实现 Markdown 预览 | 使用 pulldown-cmark 解析，并按阶段完善文档模型和 egui 渲染 | 🚧 阶段 1 已完成 |
| 7 | 实现搜索功能 | 按标题和内容搜索 | ✅ |
| 8 | 实现收藏功能 | 收藏/取消收藏 | ✅ |
| 9 | 实现标签功能 | 添加/删除标签 | ✅ |
| 10 | 实现排序功能 | 按时间/标题排序 | ✅ |
| 11 | 注册插件 | 在 plugins/mod.rs 中注册 | ✅ |

---

## 七、依赖库

```toml
[dependencies]
pulldown-cmark = "0.13.4"  # Markdown 解析渲染
```

其他使用现有依赖：
- `rusqlite`: SQLite 数据库
- `serde/serde_json`: 序列化
- `anyhow`: 错误处理

---

## 八、技术要点

1. **目录树实现**：使用递归方式渲染目录树，支持无限层级
2. **Markdown 渲染**：使用 `pulldown-cmark` 解析 Markdown，先转换为内部文档模型，再通过 egui 渲染
   - `MarkdownDocument` 保存块级节点
   - `MarkdownBlock` 表示标题、段落、引用、列表、代码块、表格和分割线
   - `MarkdownInline` 表示文本、嵌套样式、行内代码、链接、图片和换行
   - 原始 HTML 以文本形式保留，不执行或解释 HTML
   - 解析与 UI 渲染分离，为后续主题、交互和缓存优化提供基础
3. **标签管理**：标签存储为逗号分隔字符串，解析为 Vec 显示
4. **延迟操作模式**：使用 UiAction 枚举避免 egui 渲染期间的借用冲突

---

## 九、产出文件

| 文件 | 说明 |
|------|------|
| `src/plugins/note_taker/mod.rs` | 插件入口，Plugin trait 实现 |
| `src/plugins/note_taker/ui.rs` | UI 渲染逻辑（目录树、笔记编辑器） |
| `src/plugins/note_taker/models.rs` | 数据结构定义 |
| `src/plugins/note_taker/store.rs` | 数据库 CRUD 操作 |
| `src/plugins/note_taker/markdown.rs` | Markdown 解析模型和 egui 渲染 |
| `src/plugins/mod.rs` | 注册新插件 |

---

## 十、测试计划

- [x] 目录创建、编辑、删除正常
- [x] 目录树层级显示正确
- [x] 笔记创建、编辑、删除正常
- [x] 笔记按目录筛选正确
- [x] Markdown 预览渲染正确
- [x] 搜索功能正常
- [x] 收藏功能正常
- [x] 标签添加删除正常
- [x] 排序功能正常

---

## 十一、Markdown 预览分阶段实施记录

### 阶段 1：解析中间模型与核心语义（已完成）

实现文件：`src/plugins/note_taker/markdown.rs`

已完成内容：

- 增加 `MarkdownDocument`、`MarkdownBlock`、`MarkdownListItem` 和 `MarkdownInline` 模型
- 使用事件树将 `pulldown-cmark` 事件转换为拥有所有权的内部模型
- 支持标题、段落、嵌套粗体/斜体/删除线、行内代码、换行和分割线
- 支持有序列表、无序列表、嵌套列表和任务列表状态
- 支持引用块、表格结构和列对齐信息、链接、图片和代码块语言标识
- 链接当前保留目标并使用超链接样式，跳转交互留待后续阶段；图片当前显示带地址的占位文本
- 原始 HTML 按文本保留，避免渲染层执行 HTML
- 现有 `MarkdownRenderer` 已改为消费 `MarkdownDocument`
- 增加解析模型单元测试，覆盖嵌套样式、列表、任务列表、表格、链接、图片、代码语言、引用和分割线

验证方式：

```text
rustfmt --edition 2024 --check src/plugins/note_taker/markdown.rs
cargo test
cargo build
```

说明：仓库级 `cargo fmt --check` 当前还会报告其他既有文件的格式差异，本阶段未修改这些无关文件。

### 阶段 2：预览视觉样式（进行中：已完成表格边框）

下一阶段只处理渲染表现，不改变已完成的解析模型：

- 已完成：表格使用统一绘制的共享网格线、单元格内边距和表头背景，颜色跟随 egui 当前主题；不使用相邻单元格重复边框
- 统一标题、段落和列表间距
- 使用 egui 主题颜色适配浅色和深色主题
- 优化引用、表格、行内代码和代码块容器
- 明确空文档和解析内容为空时的预览表现

阶段 2 完成后，需要补充视觉渲染相关验证，再进入阶段 3。

### 阶段 3：代码块增强（待实施）

- 使用代码块语言标识接入 `syntect`
- 增加只读代码块、复制代码和长代码水平滚动
- 缓存语法高亮结果

### 阶段 4：编辑与预览交互（待实施）

- 增加编辑、预览和左右分栏模式
- 增加实时预览和滚动同步
- 增加复制 Markdown 原文等操作

### 阶段 5：性能与扩展（待实施）

- 按内容变化缓存 Markdown 解析结果
- 主题或窗口宽度变化时重新计算渲染数据
- 后续评估目录导航、图片加载和导出能力

后续实施必须从本节最后一个已完成阶段继续，并在对应阶段完成后同步更新状态、实现文件、测试命令和下一阶段入口。
