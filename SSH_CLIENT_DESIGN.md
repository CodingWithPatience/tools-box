# SSH 客户端插件设计文档

## 概述

本文档描述 SSH 客户端插件的设计与实现方案，定位为类 Xshell 的轻量级 SSH 工具，提供基础交互终端、连接配置管理、文件双向传输功能。

---

## 一、功能需求

| 功能 | 说明 | 优先级 |
|------|------|--------|
| 连接配置管理 | 新增/编辑/删除 SSH 连接配置（主机、端口、用户、认证方式） | P0 |
| 配置持久化 | SQLite 存储连接配置 | P0 |
| 密码认证 | 用户名+密码方式连接 SSH | P0 |
| 交互式终端 | 连接后进入交互终端，支持命令输入和输出显示 | P0 |
| ANSI 终端渲染 | 解析 ANSI 转义序列，以带颜色的等宽字体渲染终端内容 | P0 |
| 终端输入 | 捕获键盘输入转发到 SSH channel | P0 |
| 连接状态指示 | 显示连接中 / 已连接 / 断开等状态 | P1 |
| 密钥认证 | SSH 私钥文件认证 | P1 |
| SFTP 文件上传 | 上传本地文件到远程服务器 | P1 |
| SFTP 文件下载 | 从远程服务器下载文件到本地 | P1 |
| 终端字体大小调节 | 支持 Ctrl+滚轮 调整终端字体大小 | P2 |
| 终端日志 | 会话日志记录到本地文件 | P2 |
| 多标签会话 | 同时连接多台服务器，标签切换 | P2 |

---

## 二、界面设计

### 2.1 连接管理界面（主视图）

```
┌─────────────────────────────────────────────────────────────────────┐
│  🖥 SSH 客户端           [ + 新增连接 ]  [ 📂 导入 ] [ 📤 导出 ]      │
├──────────────┬──────────────────────────────────────────────────────┤
│              │                                                      │
│  连接列表：   │  连接详情：                                           │
│  ┌──────────┐│  ┌────────────────────────────────────────────────┐  │
│  │🟢 开发服务器││  │  名称:   开发服务器                             │  │
│  │⚪ 测试服务器││  │  主机:   192.168.1.100                         │  │
│  │⚪ 生产服务器││  │  端口:   22                                    │  │
│  │🔴 旧服务器  ││  │  用户:   root                                  │  │
│  │          ││  │  认证:   密码 / 密钥文件                          │  │
│  │          ││  │                                                │  │
│  │          ││  │  [ 🖥 创建会话 ]  [ ✏️ 编辑 ]  [ 🗑 删除 ]            │  │
│  └──────────┘│  └────────────────────────────────────────────────┘  │
│              │                                                      │
│              │  快速连接：                                            │
│              │  ┌────────────────────────────────────────────────┐  │
│              │  │ ssh [user@host          ] [-p 22]  [ 连接 ]    │  │
│              │  └────────────────────────────────────────────────┘  │
│              │                                                      │
└──────────────┴──────────────────────────────────────────────────────┘
```

### 2.2 终端界面（连接成功后）

```
┌─────────────────────────────────────────────────────────────────────┐
│  🖥 开发服务器 (root@192.168.1.100:22)   🟢 已连接    [ 🔌 断开 ]   │
│  [ 📁 SFTP ] [ 📜 日志 ] [ 🔤 字号+ ] [ 🔤 字号- ]                 │
├─────────────────────────────────────────────────────────────────────┤
│                                                                     │
│  Welcome to Ubuntu 22.04 LTS (GNU/Linux 5.15.0)                    │
│                                                                     │
│  root@dev-server:~# ls -la                                         │
│  total 32                                                           │
│  drwxr-xr-x  5 root root 4096 Jun 10 10:00 .                       │
│  drwxr-xr-x 20 root root 4096 Jun  1 08:30 ..                      │
│  -rw-r--r--  1 root root  310 Jun 10 09:55 .bashrc                 │
│  drwxr-xr-x  2 root root 4096 Jun  5 14:20 logs                   │
│                                                                     │
│  root@dev-server:~# ▌                                               │
│                                                                     │
└─────────────────────────────────────────────────────────────────────┘
```

### 2.3 SFTP 文件传输面板（侧面板）

```
┌─────────────────────────────────────────────────────────────────────┐
│  🖥 开发服务器 (root@192.168.1.100:22)   🟢 已连接                   │
│  [ 终端 ] [ 📁 SFTP ] [ 📜 日志 ]                                    │
├────────────────────────────────┬────────────────────────────────────┤
│  本地文件 (Local)               │  远程文件 (Remote)                  │
│  ┌──────────────────────────┐  │  ┌──────────────────────────────┐  │
│  │ 📁 C:\Users\admin\       │  │  │ 📁 /home/root/               │  │
│  │ ├── 📄 config.json       │  │  │ ├── 📁 logs/                 │  │
│  │ ├── 📄 data.csv          │  │  │ ├── 📄 .bashrc               │  │
│  │ ├── 📁 downloads/        │  │  │ └── 📄 app.log               │  │
│  │ └── 📄 notes.txt         │  │  │                              │  │
│  │                          │  │  │                              │  │
│  │ [ 上传 → ]               │  │  │ [ ← 下载 ]                   │  │
│  └──────────────────────────┘  │  └──────────────────────────────┘  │
│                                │                                    │
│  传输进度：                      │                                    │
│  ┌──────────────────────────┐  │                                    │
│  │ data.csv   ████████░░ 80%│  │                                    │
│  └──────────────────────────┘  │                                    │
└────────────────────────────────┴────────────────────────────────────┘
```

---

## 三、技术方案

### 3.1 依赖库选型

| 用途 | crate | 版本 | 说明 |
|------|-------|------|------|
| SSH 连接 | `ssh2` | 0.9 | 基于 libssh2 的 Rust 绑定，`vendored` feature 从源码编译 libssh2 避免系统依赖 |
| 终端 ANSI 解析 | `vt100` | 0.15 | 轻量 VT100 解析器，解析 ANSI 转义序列为二维字符网格 |
| 终端渲染 | egui 内置 | 0.31 | `LayoutJob` 渲染带颜色的等宽文本 |
| 线程通信 | `std::sync::mpsc` | std | 主线程与 SSH I/O 线程间传递终端数据和用户输入 |
| 文件对话框 | `rfd` | 0.17 | 已有依赖，SFTP 上传/下载时选择本地文件 |
| 加密 | `ssh2` 内置 | — | 支持 RSA/ECDSA/Ed25519 密钥及密码认证 |

新增依赖清单：
```toml
[dependencies]
ssh2 = { version = "0.9", features = ["vendored"] }
vt100 = "0.15"
```

### 3.2 线程模型

```
┌─────────────────────────────────────────────────────────────┐
│  主线程 (egui render loop)                                   │
│                                                             │
│  每帧流程：                                                   │
│  1. 从 mpsc::Receiver 非阻塞读取 SSH 输出数据                  │
│  2. 将数据喂入 vt100::Parser                                 │
│  3. 从 vt100::Screen 读取 cell 网格                          │
│  4. 转换为 egui::LayoutJob 渲染到 ScrollArea                  │
│  5. 收集用户键盘输入 → mpsc::Sender 发送给 SSH I/O 线程        │
│                                                             │
├─────────────────────────────────────────────────────────────┤
│  SSH I/O 线程 (std::thread::spawn)                           │
│                                                             │
│  循环：                                                      │
│  1. ssh2::Channel::read() 读取终端输出                        │
│  2. tx.send(OutputData) 发送给主线程                          │
│  3. rx.recv() 等待用户输入                                    │
│  4. ssh2::Channel::write() 写入用户输入                       │
│                                                             │
└─────────────────────────────────────────────────────────────┘
```

### 3.3 终端渲染流程

```
SSH 服务器
    │
    ▼ (原始字节流, 含 ANSI 转义序列)
vt100::Parser::process(bytes)
    │
    ▼
vt100::Screen
    │
    ▼ 每帧遍历 cell 网格
screen.cell(row, col)
    │ .contents() → char/string
    │ .fgcolor()  → ANSI 颜色
    │ .bgcolor()  → ANSI 颜色
    │ .bold() / .italic() ...
    │
    ▼
ANSI 颜色 → egui::Color32 映射
    │
    ▼
egui::LayoutJob (逐行构建)
    │
    ▼
egui::ScrollArea 内渲染为只读标签
```

ANSI 颜色映射表（标准 16 色 + 216 色调色板）：

```rust
fn ansi_to_egui(color: vt100::Color) -> Color32 {
    match color {
        vt100::Color::Default => Color32::from_rgb(0xd0, 0xd0, 0xd0),
        vt100::Color::Idx(i)  => ANSI_PALETTE[i as usize],
        vt100::Color::Rgb(r, g, b) => Color32::from_rgb(r, g, b),
    }
}
```

### 3.4 终端输入处理

egui 键盘事件处理策略：

| 输入类型 | 处理方式 |
|---------|---------|
| 可打印字符 (a-z, 0-9, 符号) | 直接发送对应字节到 SSH channel |
| Enter | 发送 `\r` (0x0D) |
| Backspace | 发送 `\x7f` (DEL) 或 `\x08` (BS) |
| Tab | 发送 `\t` (0x09) |
| Ctrl+C | 发送 `\x03` |
| Ctrl+D | 发送 `\x04` |
| Ctrl+Z | 发送 `\x1a` |
| 方向键 | 发送 ANSI 转义序列 (`\x1b[A` 等) |
| Ctrl+滚轮 | 调整终端字体大小（不发送到 SSH） |

输入捕获策略：终端区域获得 egui focus 后，通过 `ui.input(|i| i.events.iter())` 捕获所有键盘事件，过滤掉全局快捷键（如 Ctrl+1~9 切换插件），其余转发到 SSH。

---

## 四、数据结构

### 4.1 核心结构体

```rust
/// SSH 连接配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SshSession {
    pub id: i64,
    /// 连接名称（显示用）
    pub name: String,
    /// 远程主机地址
    pub host: String,
    /// SSH 端口（默认 22）
    pub port: u16,
    /// 登录用户名
    pub username: String,
    /// 认证方式
    pub auth_method: AuthMethod,
    /// 排序顺序
    pub sort_order: i32,
    /// 创建时间
    pub created_at: String,
    /// 更新时间
    pub updated_at: String,
}

/// 认证方式
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum AuthMethod {
    /// 密码认证（密码明文存储需加密，使用 aes-gcm 已有依赖）
    Password { encrypted_password: Vec<u8>, iv: Vec<u8> },
    /// 密钥文件认证
    KeyFile { private_key_path: String, passphrase_encrypted: Option<(Vec<u8>, Vec<u8>)> },
}

/// 会话连接状态
#[derive(Debug, Clone, PartialEq)]
pub enum SessionState {
    /// 未连接
    Disconnected,
    /// 连接中
    Connecting,
    /// 已连接
    Connected,
    /// 连接失败
    Error(String),
}
```

### 4.2 线程间通信消息

```rust
/// SSH I/O 线程 → 主线程的消息
#[derive(Debug)]
pub enum SshOutput {
    /// 终端输出数据（ANSI 字节流）
    TerminalData(Vec<u8>),
    /// 连接已建立
    Connected,
    /// 连接断开
    Disconnected(String),
    /// 发生错误
    Error(String),
}

/// 主线程 → SSH I/O 线程的消息
#[derive(Debug)]
pub enum SshInput {
    /// 用户键盘输入
    KeyInput(Vec<u8>),
    /// 终端窗口大小变更 (cols, rows)
    Resize(u16, u16),
    /// 断开连接
    Disconnect,
}
```

### 4.3 终端渲染器

```rust
/// 终端渲染器（封装 vt100 Parser + Screen）
pub struct Terminal {
    /// vt100 ANSI 解析器
    parser: vt100::Parser,
    /// 字体大小（可动态调整）
    font_size: f32,
    /// 终端列数
    cols: u16,
    /// 终端行数
    rows: u16,
}

impl Terminal {
    /// 将原始字节喂入解析器
    pub fn process(&mut self, data: &[u8]) {
        self.parser.process(data);
    }

    /// 生成当前帧的 LayoutJob
    pub fn render_to_layout_job(&self, is_dark_mode: bool) -> egui::text::LayoutJob {
        let screen = self.parser.screen();
        let mut job = egui::text::LayoutJob::default();
        job.wrap.max_width = f32::INFINITY;

        for row in 0..screen.rows() {
            for col in 0..screen.cols() {
                let cell = screen.cell(row, col);
                let text = cell.contents();
                let fg = ansi_to_egui(cell.fgcolor());
                let bg = ansi_to_egui(cell.bgcolor());
                // bold/italic 处理...

                job.append(
                    text,
                    0.0,
                    egui::TextFormat {
                        font_id: egui::FontId::monospace(self.font_size),
                        color: fg,
                        background: if bg == default_bg { Color32::TRANSPARENT } else { bg },
                        ..Default::default()
                    },
                );
            }
            // 每行结束添加换行
        }
        job
    }
}
```

---

## 五、数据库设计

```sql
-- SSH 连接配置表
CREATE TABLE ssh_sessions (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    name            TEXT NOT NULL,
    host            TEXT NOT NULL,
    port            INTEGER NOT NULL DEFAULT 22,
    username        TEXT NOT NULL,
    auth_type       TEXT NOT NULL DEFAULT 'password',  -- 'password' | 'keyfile'
    auth_data       TEXT,                               -- JSON: 加密后的认证数据
    sort_order      INTEGER DEFAULT 0,
    created_at      DATETIME DEFAULT CURRENT_TIMESTAMP,
    updated_at      DATETIME DEFAULT CURRENT_TIMESTAMP
);

-- SSH 连接历史日志（可选）
CREATE TABLE ssh_history (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id      INTEGER NOT NULL,
    connected_at    DATETIME DEFAULT CURRENT_TIMESTAMP,
    disconnected_at DATETIME,
    FOREIGN KEY (session_id) REFERENCES ssh_sessions(id) ON DELETE CASCADE
);
```

认证数据 `auth_data` 以 JSON 格式存储：

```json
// auth_type = "password"
{
    "encrypted_password": "<base64>",
    "iv": "<base64>"
}

// auth_type = "keyfile"
{
    "private_key_path": "C:\\Users\\admin\\.ssh\\id_rsa",
    "passphrase_encrypted": "<base64>",
    "passphrase_iv": "<base64>"
}
```

密码和密钥短语使用项目已有的 `aes-gcm` 加密，主密钥从密码管理器的主密码派生或使用独立的本地密钥。

---

## 六、目录结构

```
src/plugins/ssh_client/
├── mod.rs              # 插件入口，实现 Plugin trait
├── ui.rs               # UI 渲染（连接管理 + 终端 + SFTP 面板）
├── client.rs           # SSH 连接管理（连接/断开/认证/channel 管理）
├── terminal.rs         # 终端渲染（vt100 封装 + ANSI→egui 颜色映射）
├── sftp.rs             # SFTP 文件传输（上传/下载/目录浏览）
├── models.rs           # 数据结构定义（SshSession, AuthMethod, SessionState...）
└── store.rs            # 数据库 CRUD 操作（连接配置持久化）
```

---

## 七、实现步骤

### 第一阶段：插件骨架 + 连接管理（步骤 1-3）

| 步骤 | 任务 | 说明 | 预计产出 |
|------|------|------|---------|
| 1.1 | 创建插件骨架 | 目录结构、Plugin trait 实现、注册到 mod.rs | `mod.rs` |
| 1.2 | 新增依赖 | `ssh2` + `vt100` 加入 Cargo.toml | `Cargo.toml` |
| 1.3 | 数据库表 | SQLite 建表，密码加密存储 | `store.rs` + `database.rs` |
| 1.4 | 连接列表 UI | 左侧列表 + 右侧详情面板 | `ui.rs` 连接管理部分 |
| 1.5 | 新增/编辑/删除会话 | 表单对话框，数据持久化 | `ui.rs` + `store.rs` |

### 第二阶段：SSH 连接 + 交互终端（步骤 4-7）

| 步骤 | 任务 | 说明 | 预计产出 |
|------|------|------|---------|
| 2.1 | SSH 连接管理 | `ssh2::Session` 创建/认证/断开 | `client.rs` |
| 2.2 | SSH I/O 线程 | 独立线程读写 channel，mpsc 通信 | `client.rs` I/O 循环 |
| 2.3 | ANSI 终端解析 | vt100 Parser 封装 | `terminal.rs` |
| 2.4 | 终端 egui 渲染 | LayoutJob 生成 + ScrollArea 渲染 | `terminal.rs` + `ui.rs` |
| 2.5 | 键盘输入转发 | 捕获键盘事件 → channel.write() | `ui.rs` 输入处理 |
| 2.6 | 连接状态 UI | 连接中/已连接/断开 状态指示 | `ui.rs` |

### 第三阶段：SFTP + 增强功能（步骤 8-10）

| 步骤 | 任务 | 说明 | 预计产出 |
|------|------|------|---------|
| 3.1 | SFTP 本地文件浏览 | 左侧面板浏览本地文件系统 | `sftp.rs` + `ui.rs` |
| 3.2 | SFTP 远程文件浏览 | ssh2 sftp 列出远程目录 | `sftp.rs` |
| 3.3 | 文件上传/下载 | 进度条显示传输进度 | `sftp.rs` + `ui.rs` |
| 3.4 | 密钥认证 | 支持私钥文件 + 密码短语 | `client.rs` |
| 3.5 | 终端字号调节 | Ctrl+滚轮调整，持久化偏好 | `terminal.rs` |

---

## 八、测试计划

| 测试项 | 说明 |
|--------|------|
| SSH 连接成功 | 模拟或真实 SSH 服务器连接成功 |
| SSH 密码错误 | 密码认证失败返回明确错误信息 |
| SSH 密钥认证 | 私钥文件认证通过 |
| 终端基本输出 | 简单命令 (echo/ls) 输出正确显示 |
| 终端 ANSI 颜色 | 带颜色输出的命令 (ls --color) 正确渲染颜色 |
| 终端中文输出 | 含中文的终端输出不截断不乱码 |
| 终端宽字符 | emoji 等宽字符占位正确 |
| SFTP 上传 | 本地文件成功传输到远程 |
| SFTP 下载 | 远程文件成功下载到本地 |
| 连接 CRUD | 新增/编辑/删除连接配置持久化正确 |
| vt100 空屏幕 | 空终端不 panic |
| 断线处理 | SSH 连接意外断开时 UI 正确提示 |

---

## 九、风险与应对

| 风险 | 影响 | 概率 | 应对 |
|------|------|------|------|
| `ssh2` vendored 编译失败 | 阻塞开发 | 低 | 备选：切换 `russh` 纯 Rust 实现 |
| ANSI 复杂转义序列（vim/htop） | 终端渲染异常 | 中 | 逐步适配，MVP 先保证基础 shell 可用 |
| SSH I/O 线程 panic | 终端卡死 | 中 | 线程内 `catch_unwind`，panic 时通知主线程 |
| libssh2 不支持新版 OpenSSL | 编译失败 | 低 | `vendored` + `vendored-openssl` feature 锁定版本 |
| vt100 宽字符边界 | 中文/emoji 对齐问题 | 低 | vt100 原生支持宽字符，配合等宽字体验证 |

---

## 十、依赖清单

```toml
[dependencies]
# 新增依赖
ssh2 = { version = "0.9", features = ["vendored"] }  # SSH 连接 + SFTP
vt100 = "0.15"                                        # ANSI 终端解析
```

无需引入新的加密/序列化/数据库依赖，全部复用项目现有依赖。

---

## 附录：参考项目

- [ssh2-rs](https://github.com/rust-lang/git2-rs) — libssh2 官方 Rust 绑定
- [vt100](https://github.com/oxidecomputer/vt100) — Oxide 公司 Rust 实现的 VT100 终端解析器
- [wezterm](https://github.com/wez/wezterm) — Rust 实现的 GPU 加速终端模拟器（参考 ANSI 处理逻辑）
