# 新增插件设计文档

## 概述

本文档描述两个新增插件的设计与实现方案：
1. **Diff 文本代码对比工具** - 对比两段文本/代码的差异
2. **API 调试工具** - 轻量级 HTTP API 调试工具（类 Postman）

---

## 一、Diff 文本代码对比工具

### 1.1 功能需求

| 功能 | 说明 | 优先级 |
|------|------|--------|
| 双栏文本输入 | 左右两个文本输入区域 | P0 |
| 差异对比 | 计算并显示两段文本的差异 | P0 |
| Split Layout 视图 | 左右并排显示差异（类似 GitHub） | P0 |
| Unified 视图 | 传统单栏差异视图 | P0 |
| 差异高亮 | 新增/删除/修改的行用不同颜色标记 | P0 |
| 行号显示 | 显示每行的行号 | P1 |
| 同步滚动 | 左右面板同步滚动（Split 视图） | P1 |
| 交换内容 | 一键交换左右文本 | P2 |
| 从文件加载 | 支持从文件读取文本 | P2 |
| 复制差异 | 复制差异部分到剪贴板 | P2 |
| 编辑区语法高亮 | 编辑输入区域支持实时语法高亮，支持 25+ 种语言，带哈希缓存优化 | P1 |
| 横向滚动条 | 编辑区禁用自动换行，支持横向滚动查看长行代码 | P1 |
| 编辑区行号 | 编辑区左侧独立行号面板，固定宽度，与文本垂直同步滚动 | P1 |

### 1.2 界面设计

#### 1.2.1 编辑模式（输入文本）

```
┌─────────────────────────────────────────────────────────────────────┐
│  📝 文本对比工具           [ 交换 ] [ 清空 ] [ 从文件加载 ]          │
├─────────────────────────────────────────────────────────────────────┤
│                                                                     │
│  原始文本 (左侧)                    对比文本 (右侧)                  │
│  ┌─────────────────────────┐       ┌─────────────────────────┐     │
│  │                         │       │                         │     │
│  │  在此输入原始文本...      │       │  在此输入对比文本...      │     │
│  │                         │       │                         │     │
│  │                         │       │                         │     │
│  │                         │       │                         │     │
│  └─────────────────────────┘       └─────────────────────────┘     │
│                                                                     │
│                            [ 开始对比 ]                              │
│                                                                     │
└─────────────────────────────────────────────────────────────────────┘
```

#### 1.2.2 Split Layout 差异显示模式（类似 GitHub）

```
┌─────────────────────────────────────────────────────────────────────┐
│  📝 文本对比工具    [ 返回编辑 ] [ 交换 ] [ 统一视图 ] [ Split 视图 ] │
├────────────────────────────────┬────────────────────────────────────┤
│  原始文本 (左侧)               │  对比文本 (右侧)                    │
├────────────────────────────────┼────────────────────────────────────┤
│                                │                                    │
│  1  │ function hello() {       │  1  │ function hello() {           │
│  2  │   console.log("Hi")      │  2  │   console.log("Hi")          │
│     │                          │     │                              │
│  3  │ - return true            │  3  │ + console.log("Bye")         │
│     │   (红色背景 - 删除行)     │     │   (绿色背景 - 新增行)         │
│     │                          │     │                              │
│  4  │ - }                      │  4  │ + return false               │
│     │   (红色背景)              │     │   (绿色背景)                  │
│     │                          │     │                              │
│     │                          │  5  │ + }                          │
│     │                          │     │   (绿色背景)                  │
│     │                          │     │                              │
├────────────────────────────────┴────────────────────────────────────┤
│  统计：新增 2 行 | 删除 2 行 | 相似度：50%                            │
└─────────────────────────────────────────────────────────────────────┘

说明：
- 左侧显示原始文本，红色背景标记删除的行
- 右侧显示对比文本，绿色背景标记新增的行
- 修改的行：左侧红色背景（旧行），右侧绿色背景（新行）
- 相同的行正常显示，保持左右对齐
- 左右两侧同步滚动
```

#### 1.2.3 Unified 视图模式（传统差异视图）

```
┌─────────────────────────────────────────────────────────────────────┐
│  📝 文本对比工具    [ 返回编辑 ] [ 交换 ] [ 统一视图 ] [ Split 视图 ] │
├─────────────────────────────────────────────────────────────────────┤
│  差异结果（统一视图）：                                               │
│  ┌─────────────────────────────────────────────────────────────┐   │
│  │  @@ -1,4 +1,5 @@                                             │   │
│  │    function hello() {                                         │   │
│  │  2     console.log("Hi")      // 相同行（无背景）               │   │
│  │ -3     return true            // 删除行（红色背景）             │   │
│  │ +3     console.log("Bye")     // 新增行（绿色背景）             │   │
│  │ -4     }                      // 删除行（红色背景）             │   │
│  │ +4     return false           // 新增行（绿色背景）             │   │
│  │ +5     }                      // 新增行（绿色背景）             │   │
│  └─────────────────────────────────────────────────────────────┘   │
│                                                                     │
│  统计：新增 2 行 | 删除 2 行 | 相似度：50%                            │
│                                                                     │
└─────────────────────────────────────────────────────────────────────┘
```

### 1.3 技术方案

#### 1.3.1 差异算法选择

| 算法 | 优点 | 缺点 | 推荐 |
|------|------|------|------|
| Myers 算法 | 经典算法，结果最优 | 实现复杂 | 使用库 |
| diff crate | Rust 原生实现 | 功能基础 | 推荐 |
| similar crate | 功能丰富，支持多种格式 | 依赖较大 | 备选 |

**推荐方案：** 使用 `similar` crate，它提供了：
- 行级差异对比
- 字符级差异对比
- 统一格式输出（unified diff）
- 文本相似度计算

#### 1.3.2 数据结构

```rust
/// 差异类型
#[derive(Debug, Clone, PartialEq)]
pub enum DiffType {
    Equal,    // 相同
    Added,    // 新增
    Removed,  // 删除
    Modified, // 修改（旧行删除 + 新行新增）
}

/// 视图模式
#[derive(Debug, Clone, PartialEq)]
pub enum ViewMode {
    Edit,     // 编辑模式（输入文本）
    Split,    // Split Layout 差异视图（左右并排）
    Unified,  // Unified 差异视图（传统单栏）
}

/// 单行差异（用于 Unified 视图）
#[derive(Debug, Clone)]
pub struct DiffLine {
    pub line_number_left: Option<usize>,   // 左侧行号
    pub line_number_right: Option<usize>,  // 右侧行号
    pub content: String,                   // 行内容
    pub diff_type: DiffType,              // 差异类型
}

/// Split 视图的单行数据
#[derive(Debug, Clone)]
pub struct SplitLine {
    pub left_line_number: Option<usize>,   // 左侧行号（None 表示该行为空）
    pub left_content: Option<String>,      // 左侧内容（None 表示该行为空）
    pub left_type: DiffType,              // 左侧差异类型
    pub right_line_number: Option<usize>,  // 右侧行号
    pub right_content: Option<String>,     // 右侧内容
    pub right_type: DiffType,             // 右侧差异类型
}

/// 差异结果
#[derive(Debug, Clone)]
pub struct DiffResult {
    pub unified_lines: Vec<DiffLine>,      // Unified 视图数据
    pub split_lines: Vec<SplitLine>,       // Split 视图数据
    pub added_count: usize,                // 新增行数
    pub removed_count: usize,              // 删除行数
    pub modified_count: usize,             // 修改行数
    pub similarity: f64,                   // 相似度 (0.0 - 1.0)
}
```

#### 1.3.3 Split Layout 核心算法

Split Layout 的关键是将差异结果转换为左右对齐的两列数据：

```rust
/// 将差异结果转换为 Split 视图格式
fn build_split_lines(changes: &[Change]) -> Vec<SplitLine> {
    let mut split_lines = Vec::new();
    let mut left_line = 1;
    let mut right_line = 1;

    // 使用类似 GitHub 的对齐算法：
    // 1. 相同行：左右都显示
    // 2. 删除行：只在左侧显示，右侧留空
    // 3. 新增行：只在右侧显示，左侧留空
    // 4. 修改行：左侧显示旧行（红色），右侧显示新行（绿色）
    //    如果修改行数不对等，用空行填充保持对齐

    for change in changes {
        match change {
            Change::Equal(content) => {
                // 相同行，左右都显示
                split_lines.push(SplitLine {
                    left_line_number: Some(left_line),
                    left_content: Some(content.clone()),
                    left_type: DiffType::Equal,
                    right_line_number: Some(right_line),
                    right_content: Some(content.clone()),
                    right_type: DiffType::Equal,
                });
                left_line += 1;
                right_line += 1;
            }
            Change::Delete(content) => {
                // 删除行，只在左侧显示
                split_lines.push(SplitLine {
                    left_line_number: Some(left_line),
                    left_content: Some(content.clone()),
                    left_type: DiffType::Removed,
                    right_line_number: None,
                    right_content: None,
                    right_type: DiffType::Equal,
                });
                left_line += 1;
            }
            Change::Insert(content) => {
                // 新增行，只在右侧显示
                split_lines.push(SplitLine {
                    left_line_number: None,
                    left_content: None,
                    left_type: DiffType::Equal,
                    right_line_number: Some(right_line),
                    right_content: Some(content.clone()),
                    right_type: DiffType::Added,
                });
                right_line += 1;
            }
        }
    }

    split_lines
}
```

#### 1.3.3 目录结构

```
src/plugins/diff_viewer/
├── mod.rs          # 插件入口
├── ui.rs           # UI 渲染逻辑（含编辑区语法高亮缓存）
├── differ.rs       # 差异计算核心逻辑
├── highlight.rs    # 语法高亮器（基于 syntect）
└── models.rs       # 数据结构定义
```

### 1.4 实现步骤

| 步骤 | 任务 | 说明 | 状态 |
|------|------|------|------|
| 1.4.1 | 创建插件骨架 | 创建目录结构，实现 Plugin trait | ✅ |
| 1.4.2 | 实现双栏输入 UI | 左右两个 TextEdit 多行输入框 | ✅ |
| 1.4.3 | 实现差异计算 | 使用 `similar` crate 计算行级差异 | ✅ |
| 1.4.4 | 实现 Split Layout 视图 | 左右并排显示差异，类似 GitHub | ✅ |
| 1.4.5 | 实现 Unified 视图 | 传统单栏差异视图 | ✅ |
| 1.4.6 | 实现差异高亮显示 | 用不同颜色标记新增/删除/修改行 | ✅ |
| 1.4.7 | 添加行号显示 | 在文本区域左侧显示行号 | ✅ |
| 1.4.8 | 实现同步滚动 | Split 视图左右面板滚动位置同步 | ✅ |
| 1.4.9 | 添加视图切换 | 支持 Split/Unified 视图切换 | ✅ |
| 1.4.10 | 添加辅助功能 | 交换内容、清空、复制差异 | ✅ |
| 1.4.11 | 添加统计信息 | 显示新增/删除/修改行数和相似度 | ✅ |
| 1.4.12 | 支持文件加载 | 从文件加载文本进行对比 | ✅ |
| 1.4.13 | 语法高亮 | 代码语法高亮显示，支持 25+ 种语言 | ✅ |
| 1.4.14 | 编辑区语法高亮 | 编辑输入区域支持实时语法高亮，带缓存优化 | ✅ |
| 1.4.15 | 横向滚动条 | 编辑区禁用自动换行，支持横向滚动 | ✅ |
| 1.4.16 | 编辑区行号 | 编辑区左侧独立行号面板，与文本垂直同步滚动 | ✅ |

### 1.5 依赖库

```toml
[dependencies]
similar = "2.6"  # 文本差异对比库
rfd = "0.17"  # 文件对话框
syntect = "5.3"  # 语法高亮（可选）
```

---

## 二、API 调试工具（轻量级 Postman）

### 2.1 功能需求

| 功能 | 说明 | 优先级 |
|------|------|--------|
| 请求方法选择 | GET/POST/PUT/DELETE/PATCH | P0 |
| URL 输入 | 请求地址输入框 | P0 |
| 请求头编辑 | Key-Value 形式编辑请求头 | P0 |
| 查询参数编辑 | Key-Value 形式编辑查询参数，自动添加到 URL | P0 |
| 请求体编辑 | JSON/Form/Raw 格式请求体 | P0 |
| 发送请求 | 发送 HTTP 请求 | P0 |
| 响应显示 | 显示响应状态码、响应头、响应体 | P0 |
| 集合管理 | 创建集合/文件夹，保存请求到集合 | P1 |
| 环境变量 | 支持变量替换（如 {{base_url}}） | P1 |
| 请求历史 | 保存最近的请求记录 | P1 |
| 导入/导出 | 导入/导出集合和环境配置 | P2 |

### 2.2 界面设计

```
┌──────────────────────────────────────────────────────────────────────────┐
│  🔍 API 调试工具          [ 环境: 开发 ▼ ] [ + 新建请求 ] [ 历史 ]        │
├──────────────┬───────────────────────────────────────────────────────────┤
│              │                                                           │
│  集合：       │  请求名称: [ 用户列表 API                              ] │
│  ┌────────┐  │                                                           │
│  │📁 用户API│  │  [GET ▼]  [{{base_url}}/users]  [ 发送 ] [ 保存到集合 ]  │
│  │  ├─ 列表 │  │                                                           │
│  │  ├─ 详情 │  │  [ 请求头 ] [ 请求体 ] [ 查询参数 ]                       │
│  │  └─ 创建 │  │  ┌─────────────────────────────────────────────────────┐ │
│  │📁 订单API│  │  │  Key              │ Value                           │ │
│  │  ├─ 列表 │  │  │  ─────────────────┼──────────────────────────────  │ │
│  │  └─ 详情 │  │  │  Content-Type     │ application/json               │ │
│  └────────┘  │  │  Authorization    │ Bearer {{token}}               │ │
│              │  │  [+ 添加请求头]                                       │ │
│  最近请求：   │  └─────────────────────────────────────────────────────┘ │
│  ┌────────┐  │                                                           │
│  │GET /user│  │  响应：                                                   │
│  │POST /ord│  │  ┌─────────────────────────────────────────────────────┐ │
│  └────────┘  │  │  状态：200 OK  |  耗时：125ms  |  大小：1.2 KB        │ │
│              │  │  ┌───────────────────────────────────────────────────┐ │
│              │  │  │ { "id": 1, "name": "John Doe" }                  │ │
│              │  │  └───────────────────────────────────────────────────┘ │
│              │  └─────────────────────────────────────────────────────┘ │
├──────────────┴───────────────────────────────────────────────────────────┤
│  共 5 个集合 | 当前环境: 开发环境                                         │
└──────────────────────────────────────────────────────────────────────────┘
```

### 2.2.1 环境管理弹窗

```
┌─────────────────────────────────────────────────────────────────┐
│  环境管理                                              [ ✕ ]    │
├─────────────────────────────────────────────────────────────────┤
│                                                                 │
│  环境列表：                                                      │
│  ┌──────────────┬────────┬────────────────────────────────────┐ │
│  │ 环境名称       │ 状态    │ 操作                               │ │
│  ├──────────────┼────────┼────────────────────────────────────┤ │
│  │ ☑ 全局         │ 默认   │ [ 编辑 ]                           │ │
│  │ ☐ 开发环境     │ 未选中  │ [ 编辑 ] [ 删除 ]                  │ │
│  │ ☐ 测试环境     │ 未选中  │ [ 编辑 ] [ 删除 ]                  │ │
│  │ ☐ 生产环境     │ 未选中  │ [ 编辑 ] [ 删除 ]                  │ │
│  └──────────────┴────────┴────────────────────────────────────┘ │
│                                                                 │
│  [ + 新增环境 ]                                                   │
│                                                                 │
└─────────────────────────────────────────────────────────────────┘
```

### 2.2.2 环境变量编辑弹窗

```
┌─────────────────────────────────────────────────────────────────┐
│  编辑环境: 开发环境                                      [ ✕ ]   │
├─────────────────────────────────────────────────────────────────┤
│                                                                 │
│  变量列表：                                                      │
│  ┌──────────────────┬──────────────────────────────────────────┐│
│  │ Key              │ Value                                    ││
│  ├──────────────────┼──────────────────────────────────────────┤│
│  │ base_url         │ http://localhost:3000                    ││
│  │ token            │ eyJhbGciOiJIUzI1NiIs...                  ││
│  │ api_key          │ sk-test-1234567890                       ││
│  └──────────────────┴──────────────────────────────────────────┘│
│                                                                 │
│  [ + 添加变量 ]                                                   │
│                                                                 │
│  [ 保存 ] [ 取消 ]                                                │
│                                                                 │
└─────────────────────────────────────────────────────────────────┘
```

### 2.3 技术方案

#### 2.3.1 HTTP 客户端选择

| 库 | 优点 | 缺点 | 推荐 |
|------|------|------|------|
| reqwest | 功能全面，异步支持 | 依赖较大 | 推荐 |
| ureq | 轻量级，同步 API | 功能较少 | 备选 |
| hyper | 底层控制，性能好 | 使用复杂 | 不推荐 |

**推荐方案：** 使用 `reqwest` 的 blocking 模式，原因：
- 功能全面，支持各种 HTTP 方法
- 自动处理重定向、Cookie
- 支持 JSON、Form 等多种请求体格式
- 阻塞模式适合 UI 应用（避免异步复杂性）

#### 2.3.2 数据结构

```rust
/// HTTP 请求方法
#[derive(Debug, Clone, PartialEq)]
pub enum HttpMethod {
    Get,
    Post,
    Put,
    Delete,
    Patch,
}

/// 请求头/查询参数键值对
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HeaderEntry {
    pub key: String,
    pub value: String,
    pub enabled: bool,
}

/// 请求配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiRequest {
    pub id: String,                    // 请求 ID
    pub name: String,                  // 请求名称
    pub method: HttpMethod,            // 请求方法
    pub url: String,                   // 请求 URL（可包含 {{variable}}）
    pub headers: Vec<HeaderEntry>,     // 请求头
    pub params: Vec<HeaderEntry>,      // 查询参数
    pub body_type: BodyType,           // 请求体类型
    pub body: String,                  // 请求体内容
}

impl ApiRequest {
    /// 构建完整的 URL（包含查询参数）
    pub fn build_url(&self) -> String { ... }
}

/// 请求体类型
#[derive(Debug, Clone, PartialEq)]
pub enum BodyType {
    None,
    Json,
    Form,
    Raw,
}

/// API 集合（文件夹）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiCollection {
    pub id: i64,
    pub name: String,
    pub parent_id: Option<i64>,        // 支持嵌套集合
    pub description: String,
    pub sort_order: i32,
    pub created_at: String,
}

/// 保存的 API 请求
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SavedApiRequest {
    pub id: i64,
    pub collection_id: Option<i64>,    // 所属集合（None 表示未分类）
    pub name: String,
    pub method: HttpMethod,
    pub url: String,
    pub headers: Vec<HeaderEntry>,
    pub params: Vec<HeaderEntry>,
    pub body_type: BodyType,
    pub body: String,
    pub created_at: String,
    pub updated_at: String,
}

/// 环境配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Environment {
    pub id: i64,
    pub name: String,
    pub is_default: bool,              // 是否为默认（全局）环境
    pub is_active: bool,               // 是否当前选中
    pub created_at: String,
}

/// 环境变量
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnvironmentVariable {
    pub id: i64,
    pub environment_id: i64,
    pub key: String,
    pub value: String,
    pub enabled: bool,
}

/// 环境变量替换
pub struct VariableReplacer {
    variables: HashMap<String, String>,
}

impl VariableReplacer {
    /// 替换字符串中的 {{variable}} 为实际值
    pub fn replace(&self, input: &str) -> String { ... }
}

/// 响应结果
#[derive(Debug, Clone)]
pub struct ApiResponse {
    pub status_code: u16,
    pub status_text: String,
    pub headers: Vec<(String, String)>,
    pub body: String,
    pub elapsed_ms: u64,
    pub size_bytes: usize,
}

/// 请求历史记录
#[derive(Debug, Clone)]
pub struct RequestHistory {
    pub id: i64,
    pub request_id: String,
    pub method: String,
    pub url: String,
    pub status_code: Option<i32>,
    pub elapsed_ms: Option<i64>,
    pub executed_at: String,
}
```

#### 2.3.3 目录结构

```
src/plugins/api_tester/
├── mod.rs          # 插件入口
├── ui.rs           # UI 渲染逻辑
├── client.rs       # HTTP 客户端封装
├── models.rs       # 数据结构定义
└── store.rs        # 请求历史存储（SQLite）
```

### 2.4 实现步骤

| 步骤 | 任务 | 说明 | 状态 |
|------|------|------|------|
| 2.4.1 | 创建插件骨架 | 创建目录结构，实现 Plugin trait | ✅ |
| 2.4.2 | 实现请求配置 UI | URL 输入、方法选择、请求头/体编辑 | ✅ |
| 2.4.3 | 实现查询参数编辑 | Key-Value 形式编辑，自动添加到 URL | ✅ |
| 2.4.4 | 实现 HTTP 客户端 | 使用 reqwest 发送请求 | ✅ |
| 2.4.5 | 实现响应显示 | 状态码、响应头、响应体（JSON 格式化） | ✅ |
| 2.4.6 | 实现请求历史 | SQLite 存储历史记录（含查询参数） | ✅ |
| 2.4.7 | 添加 JSON 格式化 | 响应体 JSON 自动格式化显示 | ✅ |
| 2.4.8 | 实现集合管理 | 创建集合/文件夹，保存请求到集合 | ✅ |
| 2.4.9 | 实现环境变量 | 支持多环境、变量替换 {{variable}} | ✅ |
| 2.4.10 | 实现导入/导出 | 集合和环境配置的导入导出 | ⏳ |

### 2.5 依赖库

```toml
[dependencies]
reqwest = { version = "0.12", features = ["blocking", "json"] }  # HTTP 客户端
uuid = { version = "1.0", features = ["v4"] }  # 生成唯一 ID
urlencoding = "2.1"  # URL 参数编码
```

### 2.6 数据库设计

```sql
-- API 集合
CREATE TABLE api_collections (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    name         TEXT NOT NULL,
    parent_id    INTEGER,                -- 支持嵌套集合
    description  TEXT DEFAULT '',
    sort_order   INTEGER DEFAULT 0,
    created_at   DATETIME DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY (parent_id) REFERENCES api_collections(id) ON DELETE CASCADE
);

-- 保存的 API 请求
CREATE TABLE api_saved_requests (
    id             INTEGER PRIMARY KEY AUTOINCREMENT,
    collection_id  INTEGER,              -- 所属集合
    name           TEXT NOT NULL,
    method         TEXT NOT NULL,
    url            TEXT NOT NULL,
    headers        TEXT,                  -- JSON
    params         TEXT,                  -- JSON
    body_type      TEXT DEFAULT 'none',
    body           TEXT,
    sort_order     INTEGER DEFAULT 0,
    created_at     DATETIME DEFAULT CURRENT_TIMESTAMP,
    updated_at     DATETIME DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY (collection_id) REFERENCES api_collections(id) ON DELETE SET NULL
);

CREATE INDEX idx_api_saved_requests_collection ON api_saved_requests(collection_id);

-- 环境配置
CREATE TABLE api_environments (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    name         TEXT NOT NULL,
    is_default   BOOLEAN DEFAULT FALSE,  -- 是否为全局环境
    is_active    BOOLEAN DEFAULT FALSE,  -- 是否当前选中
    created_at   DATETIME DEFAULT CURRENT_TIMESTAMP
);

-- 环境变量
CREATE TABLE api_environment_variables (
    id             INTEGER PRIMARY KEY AUTOINCREMENT,
    environment_id INTEGER NOT NULL,
    key            TEXT NOT NULL,
    value          TEXT DEFAULT '',
    enabled        BOOLEAN DEFAULT TRUE,
    FOREIGN KEY (environment_id) REFERENCES api_environments(id) ON DELETE CASCADE
);

CREATE INDEX idx_api_env_vars_environment ON api_environment_variables(environment_id);

-- API 请求历史
CREATE TABLE api_history (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    request_id   TEXT NOT NULL,
    method       TEXT NOT NULL,
    url          TEXT NOT NULL,
    headers      TEXT,
    params       TEXT,
    body_type    TEXT DEFAULT 'none',
    body         TEXT,
    status_code  INTEGER,
    response     TEXT,
    elapsed_ms   INTEGER,
    executed_at  DATETIME DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX idx_api_history_request_id ON api_history(request_id);
CREATE INDEX idx_api_history_executed_at ON api_history(executed_at);
```

---

## 三、实施计划

### 3.1 总体计划

| 阶段 | 插件 | 预计工作量 | 优先级 |
|------|------|-----------|--------|
| 阶段六 | Diff 文本对比工具 | 2-3 天 | P1 |
| 阶段七 | API 调试工具 | 3-4 天 | P1 |

### 3.2 阶段六：Diff 文本对比工具 ✅ 已完成

| 步骤 | 任务 | 说明 | 状态 |
|------|------|------|------|
| 6.1 | 创建插件骨架 | 目录结构、Plugin trait 实现 | ✅ |
| 6.2 | 实现双栏输入 UI | 左右两个多行文本输入框 | ✅ |
| 6.3 | 集成 similar crate | 实现行级差异计算 | ✅ |
| 6.4 | 实现 Split Layout 视图 | 左右并排显示差异（类似 GitHub） | ✅ |
| 6.5 | 实现 Unified 视图 | 传统单栏差异视图 | ✅ |
| 6.6 | 实现差异高亮 | 新增/删除/修改行的颜色标记 | ✅ |
| 6.7 | 添加行号和统计 | 行号显示、差异统计信息 | ✅ |
| 6.8 | 实现同步滚动 | Split 视图左右面板同步滚动 | ✅ |
| 6.9 | 添加视图切换 | Split/Unified 视图切换按钮 | ✅ |
| 6.10 | 实现辅助功能 | 交换、清空、复制差异 | ✅ |

**阶段六产出文件：**
- `src/plugins/diff_viewer/mod.rs` — 插件入口
- `src/plugins/diff_viewer/ui.rs` — UI 渲染（包含 Split 和 Unified 两种视图）
- `src/plugins/diff_viewer/differ.rs` — 差异计算核心（生成 split_lines 和 unified_lines）
- `src/plugins/diff_viewer/models.rs` — 数据结构（包含 SplitLine、DiffLine 等）
- `Cargo.toml` — 新增 `similar` 依赖
- `src/plugins/mod.rs` — 注册新插件

### 3.3 阶段七：API 调试工具 ✅ 已完成

| 步骤 | 任务 | 说明 | 状态 |
|------|------|------|------|
| 7.1 | 创建插件骨架 | 目录结构、Plugin trait 实现 | ✅ |
| 7.2 | 实现请求配置 UI | URL、方法、请求头、请求体编辑 | ✅ |
| 7.3 | 集成 reqwest | 实现 HTTP 请求发送 | ✅ |
| 7.4 | 实现响应显示 | 状态码、响应头、响应体格式化 | ✅ |
| 7.5 | 实现请求历史 | SQLite 存储、历史列表展示 | ✅ |
| 7.6 | 添加导入导出 | 请求配置的导入导出 | ⏳ |
| 7.7 | 实现环境变量 | {{variable}} 变量替换 | ⏳ |

**阶段七产出文件：**
- `src/plugins/api_tester/mod.rs` — 插件入口
- `src/plugins/api_tester/ui.rs` — UI 渲染
- `src/plugins/api_tester/client.rs` — HTTP 客户端封装
- `src/plugins/api_tester/models.rs` — 数据结构
- `src/plugins/api_tester/store.rs` — 请求历史存储
- `Cargo.toml` — 新增 `reqwest`、`uuid` 依赖
- `src/plugins/mod.rs` — 注册新插件
- `src/storage/database.rs` — 新增 api_history 表

---

## 四、技术风险与应对

### 4.1 Diff 工具

| 风险 | 影响 | 应对方案 |
|------|------|---------|
| 大文件性能 | 内存占用高 | 限制文件大小（如 1MB） |
| 特殊字符编码 | 显示异常 | 统一使用 UTF-8 |
| 差异算法准确性 | 结果不理想 | 使用成熟的 similar crate |

### 4.2 API 工具

| 风险 | 影响 | 应对方案 |
|------|------|---------|
| 网络超时 | 请求卡住 | 设置默认超时（30秒） |
| 大响应体 | 内存占用高 | 限制响应体大小（如 10MB） |
| SSL 证书 | 请求失败 | 支持忽略证书验证选项 |
| 异步阻塞 UI | 界面卡顿 | 使用 reqwest blocking 模式 + 线程池 |

---

## 五、测试计划

### 5.1 Diff 工具测试

- [ ] 相同文本对比结果为空
- [ ] 完全不同的文本显示全部为新增/删除
- [ ] 部分修改正确识别修改行
- [ ] 空文本处理正确
- [ ] 大文件（>10000行）性能可接受
- [ ] Split 视图左右对齐正确
- [ ] Split 视图同步滚动正常
- [ ] Unified 视图显示正确
- [ ] 视图切换功能正常
- [ ] 行号显示正确

### 5.2 API 工具测试

- [ ] GET 请求正常发送和接收
- [ ] POST 请求（JSON body）正常工作
- [ ] 请求头正确添加
- [ ] 响应状态码正确显示
- [ ] 响应体 JSON 正确格式化
- [ ] 请求历史正确保存和加载
- [ ] 网络超时正确处理

---

## 六、后续扩展（可选）

### 6.1 Diff 工具扩展

- ✅ 支持文件对比（从文件加载）
- 支持目录对比
- ✅ 支持语法高亮（支持 25+ 种语言，自动检测文件类型）
- 支持三方合并（merge）

### 6.2 API 工具扩展

- 请求集合管理
- 环境变量集合
- 自动化测试脚本
- WebSocket 支持
- GraphQL 支持
