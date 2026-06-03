# Tools Box 项目设计文档

## 1. 项目概述

**项目名称**：Tools Box  
**技术栈**：Rust + egui + SQLite  
**项目定位**：一款类似 Microsoft PowerToys 的桌面工具箱应用，采用插件化架构，集成多种常用开发/运维工具。

---

## 2. 架构设计

### 2.1 整体架构

```
┌─────────────────────────────────────────────────────┐
│                    Main App (egui)                   │
│  ┌──────────┐  ┌──────────────────────────────────┐ │
│  │          │  │                                  │ │
│  │  侧边栏   │  │         插件渲染区域              │ │
│  │ (Plugin  │  │   (根据选中插件动态切换)            │ │
│  │  List)   │  │                                  │ │
│  │          │  │                                  │ │
│  │ ┌──────┐ │  │  ┌────────────────────────────┐  │ │
│  │ │ 密码   │ │  │  │                          │  │ │
│  │ │ 管理器 │ │  │  │    插件 UI 内容            │  │ │
│  │ ├──────┤ │  │  │                          │  │ │
│  │ │ JSON  │ │  │  │                          │  │ │
│  │ │ 编辑器│ │  │  │                          │  │ │
│  │ ├──────┤ │  │  └────────────────────────────┘  │ │
│  │ │ Hosts │ │  │                                  │ │
│  │ │ 管理器│ │  └──────────────────────────────────┘ │
│  │ └──────┘ │                                      │
│  └──────────┘                                      │
└─────────────────────────────────────────────────────┘
         │
         ▼
┌─────────────────────┐
│   Plugin Trait       │  ← 插件统一接口
│   - name()           │
│   - icon()           │
│   - render(&mut self)│
└─────────┬───────────┘
          │
          ▼
┌─────────────────────┐
│   Storage Layer      │  ← SQLite 数据存储
│   (rusqlite)        │
└─────────────────────┘
```

### 2.2 插件系统设计

所有插件实现统一的 `Plugin` trait：

```rust
pub trait Plugin {
    /// 插件名称（用于侧边栏显示和搜索）
    fn name(&self) -> &str;

    /// 插件图标（侧边栏图标）
    fn icon(&self) -> &str;

    /// 插件描述
    fn description(&self) -> &str;

    /// 在 egui 中渲染插件 UI
    fn render(&mut self, ui: &mut egui::Ui);

    /// 插件初始化（可选）
    fn init(&mut self) {}

    /// 插件销毁时的清理（可选）
    fn cleanup(&mut self) {}
}
```

### 2.3 目录结构设计

```
src/
├── main.rs                 # 程序入口，初始化窗口
├── app.rs                  # 主应用结构，管理插件注册和切换
├── plugin.rs               # Plugin trait 定义
├── storage/
│   ├── mod.rs              # 存储模块入口
│   └── database.rs         # SQLite 连接管理与通用操作
├── plugins/
│   ├── mod.rs              # 插件注册入口
│   ├── password_manager/
│   │   ├── mod.rs          # 密码管理器插件入口
│   │   ├── ui.rs           # UI 渲染逻辑
│   │   ├── crypto.rs       # 加密/解密工具
│   │   └── models.rs       # 数据模型
│   ├── json_editor/
│   │   ├── mod.rs          # JSON 编辑器插件入口
│   │   ├── ui.rs           # UI 渲染逻辑
│   │   └── processor.rs    # JSON 处理逻辑（格式化、压缩、转义）
│   └── hosts_manager/
│       ├── mod.rs          # Hosts 管理器插件入口
│       ├── ui.rs           # UI 渲染逻辑
│       └── parser.rs       # Hosts 文件解析
└── utils/
    ├── mod.rs
    └── clipboard.rs        # 剪贴板工具
```

---

## 3. 布局效果设计

### 3.1 主界面布局（ASCII 效果图）

```
┌─────────────────────────────────────────────────────────────────────┐
│  ◉ Tools Box                                            ─ □ ✕     │
├────────────┬────────────────────────────────────────────────────────┤
│            │                                                        │
│ 🔍 搜索插件 │                                                        │
│            │                                                        │
│ ┌────────┐ │                                                        │
│ │🔑 密码  │ │   [ JSON 编辑器 ]                                       │
│ │  管理器  │ │                                                        │
│ ├────────┤ │   输入区：                                               │
│ │📋 JSON  │ │   ┌──────────────────────────────────────────────┐     │
│ │  编辑器  │ │   │ {                                          │     │
│ ├────────┤ │   │   "name": "Tools Box",                      │     │
│ │🌐 Hosts│ │   │   "version": "0.1.0"                       │     │
│ │  管理器  │ │   │ }                                          │     │
│ ├────────┤ │   └──────────────────────────────────────────────┘     │
│ │        │ │                                                        │
│ │  ...   │ │   [ 格式化 ] [ 压缩 ] [ 转义 ] [ 反转义 ] [ 复制]       │
│ │        │ │                                                        │
│ └────────┘ │   输出区：                                               │
│            │   ┌──────────────────────────────────────────────┐     │
│            │   │ {                                            │     │
│            │   │   "name": "Tools Box",                       │     │
│            │   │   "version": "0.1.0"                         │     │
│            │   │ }                                            │     │
│            │   └──────────────────────────────────────────────┘     │
│            │                                                        │
├────────────┴────────────────────────────────────────────────────────┤
│  就绪 | SQLite: 已连接                                               │
└─────────────────────────────────────────────────────────────────────┘
```

### 3.2 各插件界面设计

#### 3.2.1 密码管理器

```
┌─────────────────────────────────────────────────────────────────────┐
│  🔑 密码管理器                    [ + 新增 ]  [ 🔍 搜索... ]       │
├─────────────────────────────────────────────────────────────────────┤
│                                                                     │
│  网站列表：                                                          │
│  ┌─────┬──────────────────┬──────────────────┬──────────┐          │
│  │  #  │ 网站              │ 账号              │ 操作      │          │
│  ├─────┼──────────────────┼──────────────────┼──────────┤          │
│  │  1  │ github.com       │ user@email.com   │ 👁 👋 🗑  │          │
│  │  2  │ google.com       │ user@gmail.com   │ 👁 👋 🗑  │          │
│  │  3  │ stackoverflow.com│ dev_user         │ 👁 👋 🗑  │          │
│  └─────┴──────────────────┴──────────────────┴──────────┘          │
│                                                                     │
│  [👁 查看密码]  [👋 复制密码]  [🗑 删除]                              │
│                                                                     │
└─────────────────────────────────────────────────────────────────────┘
```

#### 3.2.2 JSON 编辑器

```
┌─────────────────────────────────────────────────────────────────────┐
│  📋 JSON 编辑器                                                     │
├─────────────────────────────────────────────────────────────────────┤
│                                                                     │
│  输入：                                                              │
│  ┌─────────────────────────────────────────────────────────────┐   │
│  │  在此粘贴或输入 JSON...                                       │   │
│  └─────────────────────────────────────────────────────────────┘   │
│                                                                     │
│  操作按钮：                                                          │
│  [ 格式化美化 ] [ 压缩 ] [ 转义为字符串 ] [ 反转义 ] [ 清空] [ 复制 ] │
│                                                                     │
│  输出：                                                              │
│  ┌─────────────────────────────────────────────────────────────┐   │
│  │  格式化后的 JSON 结果...                                      │   │
│  └─────────────────────────────────────────────────────────────┘   │
│                                                                     │
│  状态：✓ JSON 有效  |  大小：128 bytes  |  行数：5                    │
│                                                                     │
└─────────────────────────────────────────────────────────────────────┘
```

#### 3.2.3 Hosts 管理器

```
┌─────────────────────────────────────────────────────────────────────┐
│  🌐 Hosts 管理器           [ + 新增环境 ]  [ 💾 保存 ]  [ 🔄 刷新 ] │
├─────────────────────────────────────────────────────────────────────┤
│                                                                     │
│  环境列表：                                                          │
│  ┌──────────────┬────────┬────────────────────────────────────┐    │
│  │ 环境名称       │ 状态    │ 操作                               │    │
│  ├──────────────┼────────┼────────────────────────────────────┤    │
│  │ ☑ 开发环境     │ 已启用  │ [ 编辑 ] [ 切换 ] [ 🗑 删除 ]      │    │
│  │ ☐ 测试环境     │ 已禁用  │ [ 编辑 ] [ 切换 ] [ 🗑 删除 ]      │    │
│  │ ☐ 生产环境     │ 已禁用  │ [ 编辑 ] [ 切换 ] [ 🗑 删除 ]      │    │
│  └──────────────┴────────┴────────────────────────────────────┘    │
│                                                                     │
│  当前启用环境的 Hosts 内容：                                          │
│  ┌─────────────────────────────────────────────────────────────┐   │
│  │  127.0.0.1   localhost                                      │   │
│  │  192.168.1.100  dev.api.example.com                         │   │
│  │  192.168.1.101  dev.db.example.com                          │   │
│  └─────────────────────────────────────────────────────────────┘   │
│                                                                     │
│  ⚠ 注意：切换环境需要管理员权限                                       │
│                                                                     │
└─────────────────────────────────────────────────────────────────────┘
```

---

## 4. 技术方案

### 4.1 依赖库选型

| 用途 | 库名 | 说明 |
|------|------|------|
| GUI 框架 | `eframe` + `egui` | egui 官方框架，跨平台桌面应用 |
| 数据库 | `rusqlite` | SQLite 绑定，带 `bundled` 特性 |
| 加密 | `aes-gcm` / `ring` | 密码加密存储 |
| 密码生成 | `rand` + `password-hash` | 随机密码生成 |
| 剪贴板 | `arboard` | 跨平台剪贴板操作 |
| JSON 处理 | `serde_json` | JSON 格式化/解析/压缩 |
| 配置存储 | `dirs` | 获取用户目录路径 |
| 错误处理 | `thiserror` + `anyhow` | 错误类型定义与上下文包装 |
| 日志 | `log` + `env_logger` | 日志记录 |

### 4.2 密码管理器加密方案

```
用户设置主密码
       │
       ▼
┌─────────────────┐
│ PBKDF2 派生密钥  │  ← 主密码 + Salt → 256-bit 密钥
└────────┬────────┘
         │
         ▼
┌─────────────────┐
│ AES-256-GCM 加密 │  ← 加密每条密码记录
└────────┬────────┘
         │
         ▼
┌─────────────────┐
│ SQLite 存储      │  ← 存储密文 + IV + Salt
└─────────────────┘
```

### 4.3 Hosts 管理原理

```
┌─────────────────────────────────────────────────┐
│  工具维护一份"基础 hosts" + 多份"环境 hosts"      │
│                                                  │
│  启用环境时：                                      │
│  1. 备份当前系统 hosts                             │
│  2. 合并基础 hosts + 目标环境 hosts                │
│  3. 写入系统 hosts 文件（需管理员权限）              │
│                                                  │
│  禁用环境时：                                      │
│  1. 从合并结果中移除该环境条目                      │
│  2. 写回系统 hosts 文件                             │
└─────────────────────────────────────────────────┘
```

---

## 5. 实施计划

### 阶段一：项目基础搭建（优先级：P0） ✅ 已完成

| 步骤 | 任务 | 说明 | 状态 |
|------|------|------|------|
| 1.1 | 初始化 Cargo 依赖 | 添加 `eframe`, `egui`, `rusqlite` 等基础依赖 | ✅ |
| 1.2 | 实现主窗口框架 | 创建 `App` 结构体，实现 `eframe::App` trait | ✅ |
| 1.3 | 实现侧边栏布局 | 左侧插件导航栏，右侧内容区域 | ✅ |
| 1.4 | 实现 Plugin trait | 定义插件统一接口 | ✅ |
| 1.5 | 实现插件注册机制 | 插件列表管理，支持动态添加/切换 | ✅ |
| 1.6 | SQLite 存储层 | 数据库连接管理，建表，通用 CRUD | ✅ |
| 1.7 | 中文字体支持 | 加载系统微软雅黑字体，解决中文乱码 | ✅ |

**阶段一产出文件：**
- `Cargo.toml` — 依赖配置
- `src/main.rs` — 程序入口（日志/数据库/窗口初始化）
- `src/app.rs` — 主应用结构（侧边栏 + 插件渲染 + 中文字体配置）
- `src/plugin.rs` — Plugin trait 定义
- `src/storage/mod.rs` — 存储模块入口
- `src/storage/database.rs` — SQLite 连接管理与建表
- `src/plugins/mod.rs` — 插件注册入口
- `src/plugins/password_manager/mod.rs` — 密码管理器占位模块
- `src/plugins/json_editor/mod.rs` — JSON 编辑器占位模块
- `src/plugins/hosts_manager/mod.rs` — Hosts 管理器占位模块

### 阶段二：JSON 编辑器插件（优先级：P1） ✅ 已完成

| 步骤 | 任务 | 说明 | 状态 |
|------|------|------|------|
| 2.1 | 实现基本 UI | 输入框、输出框、操作按钮布局 | ✅ |
| 2.2 | JSON 格式化美化 | 使用 `serde_json` 格式化输出 | ✅ |
| 2.3 | JSON 压缩 | 去除空白字符的紧凑输出 | ✅ |
| 2.4 | JSON 转义/反转义 | 字符串形式与 JSON 对象互转 | ✅ |
| 2.5 | 复制到剪贴板 | 集成 `arboard` 复制功能 | ✅ |
| 2.6 | JSON 校验提示 | 错误位置高亮/提示 | ✅ |

**阶段二产出文件：**
- `src/plugins/json_editor/mod.rs` — 插件入口，接入 processor 和 ui
- `src/plugins/json_editor/processor.rs` — JSON 处理逻辑（格式化/压缩/转义/校验）
- `src/plugins/json_editor/ui.rs` — UI 渲染（输入输出区、操作按钮、状态栏）
- `Cargo.toml` — 新增 `serde_json`、`arboard` 依赖

**测试覆盖：** 5 个单元测试全部通过（格式化、压缩、转义反转义、校验有效/无效 JSON）

### 阶段三：密码管理器插件（优先级：P1） ✅ 已完成

| 步骤 | 任务 | 说明 | 状态 |
|------|------|------|------|
| 3.1 | 主密码设置/验证 | 首次使用设置主密码，后续验证 | ✅ |
| 3.2 | 密码加密存储 | AES-GCM 加密，SQLite 存储 | ✅ |
| 3.3 | 密码列表展示 | 表格形式展示所有密码条目 | ✅ |
| 3.4 | 新增/编辑/删除 | 密码的增删改操作 | ✅ |
| 3.5 | 查看/复制密码 | 解密显示，一键复制 | ✅ |
| 3.6 | 随机密码生成 | 可配置长度和字符类型的密码生成器 | ✅ |
| 3.7 | 搜索功能 | 按网站名称搜索 | ✅ |

**阶段三产出文件：**
- `src/plugins/password_manager/mod.rs` — 插件入口，数据库连接管理
- `src/plugins/password_manager/crypto.rs` — 加密模块（AES-256-GCM + PBKDF2 密钥派生）
- `src/plugins/password_manager/models.rs` — 数据模型（PasswordEntry、PasswordForm、GeneratorConfig）
- `src/plugins/password_manager/store.rs` — 数据库 CRUD 操作
- `src/plugins/password_manager/ui.rs` — UI 渲染（主密码界面、密码列表、表单、生成器）
- `Cargo.toml` — 新增 `aes-gcm`、`pbkdf2`、`hmac`、`sha2`、`rand`、`serde` 依赖

**测试覆盖：** 5 个单元测试全部通过（加密解密、错误密钥验证、密码生成、密钥派生确定性、主密码哈希验证）

### 阶段四：Hosts 管理器插件（优先级：P2） ✅ 已完成

| 步骤 | 任务 | 说明 | 状态 |
|------|------|------|------|
| 4.1 | Hosts 文件解析 | 读取当前系统 hosts 文件 | ✅ |
| 4.2 | 环境管理 | 新增/编辑/删除环境 | ✅ |
| 4.3 | Hosts 条目管理 | 每个环境的 hosts 条目增删改 | ✅ |
| 4.4 | 环境切换 | 启用/禁用环境（需权限提示） | ✅ |
| 4.5 | 备份机制 | 切换前自动备份，支持恢复 | ✅ |

**阶段四产出文件：**
- `src/plugins/hosts_manager/mod.rs` — 插件入口，数据库连接管理
- `src/plugins/hosts_manager/parser.rs` — Hosts 文件解析/生成，支持读写系统 hosts
- `src/plugins/hosts_manager/store.rs` — 数据库 CRUD 操作（环境和条目管理）
- `src/plugins/hosts_manager/ui.rs` — UI 渲染（环境列表、条目管理、切换、导入导出）
- `Cargo.toml` — 新增 `chrono` 依赖（用于备份时间戳）

**测试覆盖：** 2 个单元测试全部通过（hosts 解析、hosts 生成）

### 阶段五：增强与打磨（优先级：P2） ✅ 部分完成（5.1-5.4）

| 步骤 | 任务 | 说明 | 状态 |
|------|------|------|------|
| 5.1 | 快捷键支持 | 插件唤出快捷键 | ✅ |
| 5.2 | 主题切换 | 亮色/暗色主题 | ✅ |
| 5.3 | 搜索全局插件 | 侧边栏搜索过滤插件 | ✅ |
| 5.4 | 数据导出/导入 | 密码数据的备份恢复 | ✅ |
| 5.5 | 打包发布 | 配置 GitHub Actions，构建 release | ⏳ |

**阶段五产出文件：**
- `src/app.rs` — 新增快捷键支持（Ctrl+1-9 切换插件、Ctrl+F 搜索、Esc 清空）、主题切换（亮色/暗色）、搜索增强
- `src/plugins/password_manager/models.rs` — 新增 ExportData、ExportEntry 数据结构
- `src/plugins/password_manager/store.rs` — 新增 export_entries、import_entries 方法
- `src/plugins/password_manager/ui.rs` — 新增导出/导入按钮和处理逻辑

**测试覆盖：** 14 个单元测试全部通过

---

## 6. 数据库设计

### 6.1 密码管理器

```sql
-- 主密码验证信息
CREATE TABLE master_config (
    id         INTEGER PRIMARY KEY DEFAULT 1,
    salt       BLOB NOT NULL,
    verify_hash BLOB NOT NULL,
    created_at DATETIME DEFAULT CURRENT_TIMESTAMP
);

-- 密码条目
CREATE TABLE passwords (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    website    TEXT NOT NULL,
    url        TEXT,
    username   TEXT NOT NULL,
    password   BLOB NOT NULL,     -- AES-GCM 加密后的密文
    iv         BLOB NOT NULL,     -- 初始化向量
    notes      TEXT,
    created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
    updated_at DATETIME DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX idx_passwords_website ON passwords(website);
```

### 6.2 Hosts 管理器

```sql
-- Hosts 环境
CREATE TABLE hosts_environments (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    name       TEXT NOT NULL UNIQUE,
    is_active  BOOLEAN DEFAULT FALSE,
    created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
    updated_at DATETIME DEFAULT CURRENT_TIMESTAMP
);

-- Hosts 条目
CREATE TABLE hosts_entries (
    id             INTEGER PRIMARY KEY AUTOINCREMENT,
    environment_id INTEGER NOT NULL,
    ip_address     TEXT NOT NULL,
    hostname       TEXT NOT NULL,
    comment        TEXT,
    is_enabled     BOOLEAN DEFAULT TRUE,
    sort_order     INTEGER DEFAULT 0,
    FOREIGN KEY (environment_id) REFERENCES hosts_environments(id) ON DELETE CASCADE
);

CREATE INDEX idx_hosts_entries_env ON hosts_entries(environment_id);
```

---

## 7. 开发规范

### 7.1 代码风格

- 遵循 `rustfmt` 默认格式
- 使用 `clippy` 进行代码检查，启用 `pedantic` lint
- 模块间通过 trait 解耦，避免直接依赖具体实现

### 7.2 插件开发约定

- 每个插件作为独立模块位于 `src/plugins/<plugin_name>/`
- 插件 UI 逻辑与业务逻辑分离（`ui.rs` vs 其他）
- 插件通过 `src/plugins/mod.rs` 统一注册
- 新增插件只需：实现 `Plugin` trait → 在 `mod.rs` 注册

### 7.3 提交规范

- 使用 Conventional Commits 格式
- 每个功能阶段完成后提交
- 提交信息使用中文

---

## 8. 构建与运行

```bash
# 开发模式运行
cargo run

# Release 构建
cargo build --release

# 代码检查
cargo clippy -- -W clippy::pedantic

# 格式化
cargo fmt
```

---

## 附录：布局参考图

> 以下为各插件的简化线框图，展示核心交互流程。

### 密码管理器流程

```
[打开插件] → [输入主密码] → [密码列表]
                              ├── [新增] → [填写表单] → [保存]
                              ├── [查看] → [解密显示]
                              ├── [复制] → [剪贴板]
                              └── [删除] → [确认] → [移除]
```

### JSON 编辑器流程

```
[打开插件] → [输入/粘贴 JSON]
                ├── [格式化美化] → [输出美化结果]
                ├── [压缩]       → [输出紧凑结果]
                ├── [转义]       → [输出字符串形式]
                └── [反转义]     → [输出 JSON 对象]
```

### Hosts 管理器流程

```
[打开插件] → [环境列表]
              ├── [新增环境] → [填写名称] → [添加条目] → [保存]
              ├── [编辑环境] → [修改条目] → [保存]
              └── [切换环境] → [确认] → [备份] → [写入系统 hosts]
```
