# SSH 客户端插件设计文档

## 概述

本文档描述 SSH 客户端插件的设计与实现方案，定位为类 Xshell 的轻量级 SSH 工具，提供基础交互终端、连接配置管理、文件双向传输功能。

---

## 一、功能需求

| 功能 | 说明 | 优先级 | 状态 |
|------|------|--------|------|
| 连接配置管理 | 新增/编辑/删除 SSH 连接配置（主机、端口、用户、认证方式） | P0 | ✅ |
| 配置持久化 | SQLite 存储连接配置 | P0 | ✅ |
| 密码认证 | 用户名+密码方式连接 SSH | P0 | ✅ |
| 交互式终端 | 连接后进入交互终端，支持命令输入和输出显示 | P0 | ✅ |
| ANSI 终端渲染 | 解析 ANSI 转义序列，以带颜色的等宽字体渲染终端内容 | P0 | ✅ |
| 终端输入 | 捕获键盘输入转发到 SSH channel | P0 | ✅ |
| 终端复制粘贴 | 鼠标拖选文本，支持快捷键和右键菜单复制粘贴 | P0 | ✅ |
| 连接状态指示 | 显示连接中 / 已连接 / 断开等状态 | P1 | ✅ |
| 密钥认证 | SSH 私钥文件认证 | P1 | ✅ |
| SFTP 文件上传 | 上传本地文件到远程服务器 | P1 | ✅ |
| SFTP 文件下载 | 从远程服务器下载文件到本地 | P1 | ✅ |
| 终端字体大小调节 | 支持 Ctrl+滚轮 调整终端字体大小 | P2 | ✅ |
| 终端单词级编辑 | Ctrl+左右箭头按单词移动光标，Ctrl+Backspace 删除前一个单词 | P2 | ✅ |
| 终端编辑键 | Home/End 跳转行首行尾（序列跟随远端 DECCKM），Ctrl+Home/Ctrl+End 跳首尾，Insert/Delete/PageUp/PageDown 与 xterm 对齐 | P2 | ✅ |
| 终端日志 | 会话日志记录到本地文件 | P2 | ⬜ |
| 多标签会话 | 同时连接多台服务器，标签切换（支持 Ctrl+Tab / Ctrl+Shift+Tab） | P2 | ✅ |

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
│  [ 🖥 终端 ] [ 📁 SFTP ]                                          │
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

位置计算（光标、选区、鼠标定位）说明：

- 终端文本由 egui 按各字体实际字形宽度排版：半角字符取等宽字体宽度，宽字符（中文、Emoji）取该字形的真实宽度，二者不满足「宽字符 = 2 × 等宽字符宽度」。
- 因此光标与选区位置**不按「列号 × 等宽字符宽度」估算**，而是用 `TerminalEmulator::rendered_cell_text` 按单元格取出实际渲染的文本，再用 `Fonts::glyph_width` 逐字符累加并按像素网格舍入（`TerminalTextLayout::column_offset` / `column_span`，与 epaint 的排版推进一致）。
- 宽字符的后半单元格不产生渲染宽度（`rendered_cell_text` 返回 `None`），空白单元格以空格占一个等宽字符宽度；鼠标定位按同样的逐单元格宽度反查列号，保证与显示文字对齐。

ANSI 颜色映射（主题 16 色调色板 + 256 色）：

```rust
fn ansi_color_to_egui(color: vt100::Color, default: Color32, palette: &[Color32; 16]) -> Color32 {
    match color {
        vt100::Color::Default      => default,
        vt100::Color::Idx(idx)     => xterm_index_to_egui(idx, palette), // 0..16 调色板 / 16..232 色立方 / 232..256 灰阶
        vt100::Color::Rgb(r, g, b) => Color32::from_rgb(r, g, b),
    }
}

fn ansi_palette(is_dark_mode: bool) -> &'static [Color32; 16] {
    if is_dark_mode { &ANSI_PALETTE_DARK } else { &ANSI_PALETTE_LIGHT }
}
```

- 深色与浅色主题各有一套 16 色调色板：经典调色板的蓝色（`#0000AA`）在深色背景（`#1e1e1e`）
  上对比度仅约 1.25:1，目录等内容几乎无法辨认；深色调色板改用对深色背景友好的取值
  （基于 One Half Dark，蓝色 `#61AFEF` 对比度约 7:1），亮色变体整体更亮以便区分强调内容。
- 256 色索引完整解析：`0..16` 取主题调色板，`16..232` 为标准 6×6×6 色立方
  （通道取值 `0/95/135/175/215/255`），`232..256` 为 24 级灰阶（8 起步、步长 10）。
  PTY 声明 `xterm-256color`，因此 `ls --color`、vim、tmux 等使用扩展色的输出不再丢色。
- 取舍说明：扩展色按程序给定值原样呈现（与真实终端一致），若程序使用与背景同色的索引
  （浅色主题下的 231、深色主题下的 16 等），该内容同样不可见；深色调色板的黑/亮黑为暗色
  主题惯例取值，不作为正文前景色使用。

### 3.4 终端输入处理

egui 键盘事件处理策略：

| 输入类型 | 处理方式 |
|---------|---------|
| 可打印字符 (a-z, 0-9, 符号) | 直接发送对应字节到 SSH channel |
| Enter | 发送 `\r` (0x0D) |
| Backspace | 发送 `\x7f` (DEL) |
| Ctrl+Backspace | 发送 `\x17` (Ctrl+W)，由远端 shell 删除光标前一个单词 |
| Tab | 发送 `\t` (0x09) |
| Ctrl+C | 发送 `\x03` |
| Ctrl+Shift+C | 复制鼠标选中文本；无选区时复制当前可见终端内容 |
| Ctrl+Shift+V | 从系统剪贴板读取文本并发送到 SSH channel |
| 系统 Copy/Paste 事件 | 根据 patched `egui-winit` 产生的事件类型区分 `Ctrl+C` 与 `Ctrl+Shift+C/V`；仅接受快捷键或右键请求产生的 Paste |
| Ctrl+D | 发送 `\x04` |
| Ctrl+Z | 发送 `\x1a` |
| Ctrl+L | 发送 `\x0c` |
| Escape | 发送 `\x1b` |
| 方向键 | 发送 ANSI 转义序列 (`\x1b[A` 等) |
| Ctrl+左右箭头 | 发送 `\x1b[1;5D` / `\x1b[1;5C`，按单词移动光标 |
| Home / End | 发送 `\x1b[H` / `\x1b[F`（普通模式）或 `\x1bOH` / `\x1bOF`（应用光标键模式），跳到行首 / 行尾 |
| Ctrl+Home / Ctrl+End | 发送 `\x1b[1;5H` / `\x1b[1;5F`，跳到输入区 / 文档首尾（vim 插入模式等） |
| Insert / Delete | 发送 `\x1b[2~` / `\x1b[3~`，序列固定为 CSI 形式，不受 DECCKM 影响 |
| PageUp / PageDown | 发送 `\x1b[5~` / `\x1b[6~`，翻页由远端程序处理（本地历史仍用滚轮 / 滚动条） |
| Ctrl+滚轮 | 调整终端字体大小（不发送到 SSH） |
| Ctrl+Tab / Ctrl+Shift+Tab | 切换到下一个 / 上一个会话标签（循环，不发送到 SSH） |

说明：Ctrl+Backspace 与 Ctrl+左右箭头的单词级操作依赖远端 shell 处于默认的 emacs 编辑模式（bash/readline、zsh、fish 默认绑定）；vi 编辑模式下不生效。方向键仅识别 Ctrl，Shift/Alt 组合不会改变发送的序列（与改动前一致）。

方向键有意保持 CSI 形式、不跟随 DECCKM（多数全屏程序对两种形式都有兜底识别），本次仅让新增的编辑键跟随远端模式，避免影响既有输入行为。

Home/End 的序列形式跟随远端状态：远端程序（vim、less 等）通过 `smkx` 打开应用光标键模式（DECCKM，由 `vt100` 解析 `\x1b[?1h` / `\x1b[?1l` 得到）后发送 SS3 形式，与 xterm 及 terminfo 的 `khome` / `kend` 保持一致；带 Ctrl 修饰键时统一使用 xterm 的 `CSI 1;5` 形式，不受该模式影响。Insert/Delete/PageUp/PageDown 固定使用 `CSI n ~` 形式。编辑键与方向键一致，只区分无修饰键与 Ctrl 组合：Shift/Alt 组合不改变发送的序列（Shift+Home 等同普通 Home），Ctrl+Delete、Ctrl+PageUp 等不映射。

Ctrl+Insert、Shift+Insert、Shift+Delete 不进入编辑键映射：Windows 下 patched `egui-winit` 会在产生 Key 事件之前把它们转换为系统 Copy/Paste/Cut 事件。其中 Ctrl+Insert 产生的 Copy 事件在终端视图按普通 `Ctrl+C` 处理（即向远端发送中断信号 `0x03`），Shift+Insert 的 Paste 事件与 Shift+Delete 的 Cut 事件在终端视图当前无对应处理，均为本次改动前既有行为，未随本次改动调整。

输入捕获策略：终端区域获得 egui focus 后，通过 `ui.input(|i| i.events.iter())` 捕获所有键盘事件，过滤掉全局快捷键（如 Ctrl+1~9 切换插件），其余转发到 SSH。

会话标签切换快捷键在终端输入处理之前消费，因此 `Ctrl+Tab` 不会把 `Tab` 发送到远端；`Ctrl+Tab` 与 `Ctrl+Shift+Tab` 在会话视图的终端、SFTP 子标签下均可用，单标签时保持当前标签不变，长按按系统按键重复速率连续切换。消费事件只负责阻止 `Tab` 进入终端输入；阻止 egui 焦点导航依赖终端自身设置的 focus lock filter，因此在未渲染终端的 SFTP 子标签下，焦点仍可能在面板内移动。

复制粘贴快捷键在普通终端控制键之前分流。项目通过本地 `egui-winit 0.31.1` 最小补丁，使 Windows 的 `Ctrl+Shift+C/V` 保留包含触发时修饰键的 Key 事件，避免依赖帧末按键状态推断；普通 `Ctrl+C/V` 仍使用平台 Copy/Paste 事件。因此 `Ctrl+C` 始终发送远端中断信号，`Ctrl+Shift+C` 执行复制，普通 `Ctrl+V` 不向远端发送内容，只有 `Ctrl+Shift+V` 才显式请求粘贴。右键粘贴同样通过 egui `ViewportCommand::RequestPaste` 读取操作系统剪贴板，可接收浏览器、编辑器等其他程序复制的文本；会话标签使用 2 秒截止时间等待平台返回 Paste，收到首个事件或超时后结束请求。无快捷键授权且无待处理请求的 Paste 事件不会进入终端。SSH 模块不直接执行剪贴板 I/O。

### 3.5 终端文本选择与剪贴板

- 终端标签保存鼠标选择的锚点和焦点单元格，允许正向、反向和跨行拖选。
- 鼠标坐标根据等宽字符宽度、行高和终端行列数映射到 vt100 单元格，并在边界外拖动时钳制到有效范围。
- 选区文本通过 `vt100::Screen::contents_between` 提取，正确处理跨行文本和宽字符；无选区时通过 `Screen::contents` 获取当前可见纯文本。
- 终端区域以半透明背景绘制选区，并提供右键复制/粘贴菜单；顶部不重复显示操作按钮。
- 终端尺寸变化、滚动、键盘输入或新输出到达时清除旧选区，避免复制已经变化的屏幕位置。
- 终端内容区底部固定预留 6px 安全间距，PTY 行数按扣除间距后的高度计算，避免普通窗口下最后一行光标贴近裁剪边界而无法闪烁。
- 光标可见性使用显式终端画布边界判断，不再依赖文本 `LayoutJob` 的浮点 galley 底边；光标矩形上下各内缩 1px，并限定在终端画布内绘制。IME 输入区域同步使用终端画布，查看 scrollback 时将候选框光标位置钳制在可见边界内。

### 3.6 终端滚动与历史浏览

- 滚动模型：内容 = 历史行（scrollback）+ 当前屏幕行；窗口固定为屏幕行数，滚动偏移 `0..=历史行数`
  表示窗口在内容上向上滑动的行数，`0` 为最新内容（跟随输出）。
- 鼠标滚轮：普通滚轮每格滚动 3 行（按 egui 的 `line_scroll_speed` 换算，原生平台一格为 40 点）；
  触控板按滚动距离等比例换算，每次事件至少 1 行，惯性滚动上限 300 行，避免一次滚动跳跃过大。
  `Ctrl+滚轮` 仍用于调整字号。
- 右侧滚动条：有历史内容时显示，滑块高度按「可视行数 / 总行数」折算（最小 12px），拖动滑块按位置
  快速定位；点击滑块上方/下方按一页（可视行数的 90%）翻页；无历史内容或轨道过短时不显示。
- 键盘输入或粘贴完成后自动收回到底部（偏移归零），终端有新输出且偏移非 0 时保持当前查看位置。
- 窗口尺寸变化时历史行随之迁移（本地 `vt100` 补丁）：放大时从历史行回填到屏幕顶部（历史行数随之
  减少，内容全部可见时滚动条自动消失）；缩小时先丢弃光标之下的空白填充行、再按需把顶部行推入
  历史行——因此放大再恢复窗口不会把填充空行变成历史内容，滚动条范围只覆盖真实输出，
  同时光标之下的真实内容与最新输出都不会丢失。
- 依赖补丁：`vt100 0.15.2` 的 `Grid::visible_rows` 在滚动偏移超过屏幕行数时会出现无符号下溢
  （debug 构建 panic、release 构建回绕），本项目以 `vendor/vt100-0.15.2` 本地补丁修正为
  「历史行 + 屏幕行滑动窗口」，并新增 `Screen::scrollback_count` 供滚动条与滚动上限使用；
  补丁同时修正 `Grid::set_size`（放大回填历史行、缩小按需推入历史行），
  详见 `vendor/vt100-0.15.2/TOOLS_BOX_PATCH.md`）。

### 3.7 SFTP 目录刷新与传输可靠性

- 本地目录选择弹窗、地址栏回车、上级目录和主目录操作统一触发文件列表刷新；无效路径恢复为当前目录并显示错误。
- 远程地址栏回车、刷新、上级目录和双击目录统一发送 `ListDirectory` 请求，仅在服务器成功返回后更新当前目录；失败时保留原列表并显示错误。
- 远程路径始终使用 POSIX 语义进行上级目录计算和词法规范化，不依赖 Windows 本地路径规则。
- 上传/下载任务使用唯一任务 ID，支持取消、断开连接收尾、重复任务拦截、未知文件大小进度和完成历史清理。
- 控制请求使用独立通道，取消和断开操作不会被普通 SFTP 操作队列阻塞。
- 下载先写入目标目录内的临时文件，完成并校验后再替换目标文件；上传和下载均校验源文件大小变化，降低半成品覆盖和静默截断风险。
- SFTP TCP 连接及读写设置超时，传输进度按最小时间间隔上报，避免高频刷新 UI。

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

        // 主题调色板每帧解析一次，逐单元格复用
        let palette = ansi_palette(is_dark_mode);

        for row in 0..screen.rows() {
            for col in 0..screen.cols() {
                let cell = screen.cell(row, col);
                let text = cell.contents();
                let fg = ansi_color_to_egui(cell.fgcolor(), default_fg, palette);
                let bg = ansi_color_to_egui(cell.bgcolor(), default_bg, palette);
                // bold/italic 处理...

                job.append(
                    text,
                    0.0,
                    egui::TextFormat {
                        font_id: egui::FontId::monospace(self.font_size),
                        color: fg,
                        background: if matches!(cell.bgcolor(), vt100::Color::Default) {
                            Color32::TRANSPARENT
                        } else {
                            bg
                        },
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

| 步骤 | 任务 | 说明 | 预计产出 | 状态 |
|------|------|------|---------|------|
| 1.1 | 创建插件骨架 | 目录结构、Plugin trait 实现、注册到 mod.rs | `mod.rs` | ✅ |
| 1.2 | 新增依赖 | `ssh2` + `vt100` 加入 Cargo.toml | `Cargo.toml` | ✅ |
| 1.3 | 数据库表 | SQLite 建表，密码加密存储 | `store.rs` + `database.rs` | ✅ |
| 1.4 | 连接列表 UI | 左侧列表 + 右侧详情面板 | `ui.rs` 连接管理部分 | ✅ |
| 1.5 | 新增/编辑/删除会话 | 表单对话框，数据持久化 | `ui.rs` + `store.rs` | ✅ |

### 第二阶段：SSH 连接 + 交互终端（步骤 4-7）

| 步骤 | 任务 | 说明 | 预计产出 | 状态 |
|------|------|------|---------|------|
| 2.1 | SSH 连接管理 | `ssh2::Session` 创建/认证/断开 | `client.rs` | ✅ |
| 2.2 | SSH I/O 线程 | 独立线程读写 channel，mpsc 通信 | `client.rs` I/O 循环 | ✅ |
| 2.3 | ANSI 终端解析 | vt100 Parser 封装 | `terminal.rs` | ✅ |
| 2.4 | 终端 egui 渲染 | LayoutJob 生成 + ScrollArea 渲染 | `terminal.rs` + `ui.rs` | ✅ |
| 2.5 | 键盘输入转发 | 捕获键盘事件 → channel.write() | `ui.rs` 输入处理 | ✅ |
| 2.6 | 连接状态 UI | 连接中/已连接/断开 状态指示 | `ui.rs` | ✅ |
| 2.7 | 终端复制粘贴 | 鼠标拖选、快捷键及右键菜单 | `terminal.rs` + `models.rs` + `ui.rs` | ✅ |
| 2.8 | 终端单词级编辑 | Ctrl+左右箭头按单词移动、Ctrl+Backspace 删除前一个单词 | `ui.rs` | ✅ |
| 2.9 | 会话标签快捷键 | Ctrl+Tab / Ctrl+Shift+Tab 循环切换标签，不向远端发送 Tab | `ui.rs` | ✅ |

### 第三阶段：SFTP + 增强功能（步骤 8-10）

| 步骤 | 任务 | 说明 | 预计产出 | 状态 |
|------|------|------|---------|------|
| 3.1 | SFTP 本地文件浏览 | 左侧面板浏览本地文件系统 | `sftp.rs` + `ui.rs` | ✅ |
| 3.2 | SFTP 远程文件浏览 | ssh2 sftp 列出远程目录 | `sftp.rs` | ✅ |
| 3.3 | 文件上传/下载 | 进度条显示传输进度 | `sftp.rs` + `ui.rs` | ✅ |
| 3.4 | 密钥认证 | 支持私钥文件 + 密码短语 | `client.rs` | ✅ |
| 3.5 | 终端字号调节 | Ctrl+滚轮调整，持久化偏好 | `terminal.rs` | ✅ |
| 3.6 | SFTP 目录刷新 | 本地/远程路径切换后统一刷新并处理失败回滚 | `ui.rs` + `sftp.rs` | ✅ |
| 3.7 | 传输任务可靠性 | 任务 ID、取消/断开控制、重复拦截、完成历史 | `models.rs` + `sftp.rs` + `ui.rs` | ✅ |
| 3.8 | 文件落盘安全 | 临时文件、大小校验、原子替换和 I/O 超时 | `sftp.rs` | ✅ |

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
| 终端复制快捷键 | Ctrl+Shift+C 复制选区，无选区时复制可见内容；Ctrl+C 仍发送中断信号 |
| 终端粘贴快捷键 | Ctrl+Shift+V 和系统 Paste 事件只向远端发送一次剪贴板文本 |
| 终端单词级编辑 | Ctrl+左右箭头按单词移动光标；Ctrl+Backspace 删除光标前一个单词 |
| 终端宽字符定位 | 光标、选区与鼠标定位按真实字形宽度计算，中文输入不偏移 |
| 终端配色 | 深/浅主题各一套 16 色调色板，支持 256 色索引与灰阶 |
| 终端滚动与历史 | 滚轮每格滚动 3 行，右侧滚动条支持拖动定位与翻页 |
| 会话标签快捷键 | Ctrl+Tab 下一个标签、Ctrl+Shift+Tab 上一个标签，循环切换且不向远端发送 Tab |
| 终端鼠标选区 | 正向、反向、跨行拖选及越界钳制正确 |
| 终端底部光标 | 普通窗口与最大化窗口中最后一行光标均有安全间距并正常闪烁 |
| SFTP 上传 | 本地文件成功传输到远程 |
| SFTP 下载 | 远程文件成功下载到本地 |
| SFTP 本地目录刷新 | 选择目录、地址栏回车、上级目录和主目录均刷新列表 |
| SFTP 远程目录刷新 | 地址栏回车、刷新、上级目录和双击目录成功/失败状态正确 |
| SFTP 传输可靠性 | 取消、断开、重复任务、未知大小、临时文件替换和大小变化检测正确 |
| 终端滚动 | 滚轮每格 3 行、触控板按比例、偏移钳制到历史行数且窗口正确滑动 |
| 终端滚动条 | 拖动定位、点击翻页、无历史时不显示 |
| 终端配色 | 深/浅主题调色板按主题选择，256 色索引解析为色立方与灰阶 |
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

## 十一、已完成修改记录

### 11.1 SFTP 目录刷新与传输可靠性修复（2026-08-14）

对应提交：`3be1369 修复SFTP目录刷新与文件传输可靠性问题`。

完成结果：

- 修复本地文件选择弹窗选中目录后文件列表不刷新，以及本地地址栏回车不刷新问题。
- 修复远程地址栏回车、上级目录、刷新和双击目录后列表状态不一致问题，并增加目录加载失败响应。
- 增加传输任务唯一 ID、取消操作、独立控制通道、重复任务拦截和断开连接收尾。
- 增加未知文件大小显示、完成任务历史保留及手动清理。
- 增加 SFTP 连接/读写超时、临时文件安全替换、文件大小一致性校验和进度节流。
- 补充 POSIX 路径、目录请求、任务状态、取消/断开、临时文件替换和大小校验等测试。

### 11.2 SSH 终端复制粘贴（2026-08-16）

完成结果：

- 支持鼠标拖选终端文本，并以半透明背景显示选区。
- 支持 `Ctrl+Shift+C` 复制、`Ctrl+Shift+V` 粘贴，同时保留 `Ctrl+C` 远端中断语义。
- 支持读取操作系统剪贴板中的外部来源文本，并通过系统 Paste 事件和终端右键复制/粘贴菜单操作；界面不额外占用顶部按钮空间。
- 终端底部预留 6px，并按可用内容高度重新计算远端 PTY 行数，修复普通窗口最后一行光标被裁剪问题。
- 光标改用显式终端画布边界和内缩矩形绘制，避免文本 galley 底边的浮点误差错误隐藏最后一行光标。
- 无选区时复制当前可见终端内容；粘贴发送失败会显示状态并记录日志。
- 补充同帧 Key+Paste 去重、跨帧待处理 Paste、原生 Copy/Paste 事件分流、普通 `Ctrl+V` 拒绝、空剪贴板状态、快捷键按下/释放/repeat、输入队列异常、鼠标单元格映射、分数像素行高、scrollback 饱和、IME 光标钳制、最后一行光标画布边界、正反向选区、跨行文本、宽字符和边界钳制测试；SSH 客户端相关测试 47 项全部通过。

验证说明：2026-08-16 最终执行 `cargo test --all-targets`，共 141 项测试，结果为 140 项通过、0 项失败、1 项因需要交互式 Windows 会话而忽略；SSH 客户端相关测试 47 项全部通过，本地 `egui-winit` 补丁专用测试 2 项全部通过。`cargo build` 与独立目标目录的 release 构建均通过。

### 11.3 会话标签切换与终端单词级编辑（2026-09-16）

完成结果：

- 会话标签支持 `Ctrl+Tab` 切换到下一个标签、`Ctrl+Shift+Tab` 切换到上一个标签，索引循环回绕，单标签时保持不变。
- 标签切换快捷键在终端输入处理之前消费，`Tab` 不会被当作终端输入发送到远端；终端与 SFTP 子标签下均可使用，长按按系统按键重复速率连续切换。
- 终端支持 `Ctrl+左/右箭头` 按单词移动光标，分别发送 xterm 的 `\x1b[1;5D` 与 `\x1b[1;5C`。
- 终端支持 `Ctrl+Backspace` 删除光标前一个单词，发送 `\x17`（Ctrl+W），由远端 shell 的默认绑定完成删除；普通 `Backspace` 仍发送 `\x7f`。
- 补充快捷键识别（含修饰键校验、按下/松开/repeat）、循环切换索引边界、消费后事件列表、方向键转义序列与删除键序列测试。

验证说明：`cargo test` 共 186 项测试，结果为 185 项通过、0 项失败、1 项因需要交互式 Windows 会话而忽略；SSH 客户端相关测试 54 项全部通过。`cargo build` 通过，`cargo check --release` 与独立目标目录的 release 构建均通过（本机 release 二进制被运行中的程序占用，故未覆盖默认目标目录）。`cargo fmt --check` 与 `cargo clippy --all-targets` 对本次新增代码无差异、无告警。

遗留说明：`Ctrl+左右箭头` 与 `Ctrl+Backspace` 依赖远端 shell 的 emacs 编辑模式绑定，尚未在真实 bash/zsh/fish 会话中做真机回归测试。

### 11.4 终端宽字符（中文）光标位置修正（2026-09-16）

问题：终端输入中文后，光标位置比文字实际位置偏右，并且随中文字符数量累积；拖选与鼠标定位同样偏移。

原因：`char_width` 取自等宽字体 `M` 的字形宽度，光标、选区与鼠标定位都按「列号 × char_width」计算；而中文字符由 Microsoft YaHei 渲染，其字形宽度并不是两个等宽字符宽度（14px 字号、pixels_per_point = 1 下实测：`M` 为 8.28px，取整后 `char_width` = 8px，两个单元格即 16px；而汉字为 13.64px），因此每个汉字累积约 2.4px 的右偏。

完成结果：

- `terminal.rs` 新增 `rendered_cell_text`，按与渲染一致的取字规则返回单元格实际渲染的文本（空白单元格返回空格、宽字符后半单元格与越界单元格返回 `None`）。
- `ui.rs` 新增 `TerminalTextLayout` 度量（等宽字符宽度、行高、字体、像素比例）与 `terminal_text_width` / `terminal_text_width_from`：逐字符累加 `Fonts::glyph_width` 并按像素网格舍入，与 epaint 的排版推进逐字等价（新增用例直接与 `fonts.layout_job` 排版出的字形 x 对照）。
- 光标矩形改为按渲染宽度定位（`column_offset` → `terminal_cursor_rect(x, width)`），宽度取光标所在单元格的字形宽度；选区按行取渲染起止位置（`column_span` 单次遍历该行前缀）；鼠标点击/拖选按逐单元格渲染宽度反查列号。
- 选区绘制改为「在字体锁内计算各行位置、锁外绘制」，避免在 `ui.fonts` 闭包内访问画布造成界面卡死（新增整帧绘制用例覆盖）。
- 删除不再使用的 `cursor_char_width`。
- 新增 6 个用例（像素舍入、指针映射与钳制、宽字符列位置、空白单元格占位、光标矩形坐标、中文字体全链路对照、选区整帧绘制），并扩展 4 个既有用例（单元格取字规则、指针定位签名等）；SSH 客户端相关测试 61 项全部通过。

验证说明：`cargo test` 共 193 项测试，结果为 192 项通过、0 项失败、1 项因需要交互式 Windows 会话而忽略。`cargo build` 通过；`cargo fmt --check` 与 `cargo clippy --all-targets` 对本次新增代码无差异、无新增告警。

### 11.5 终端滚动优化与滚动条（2026-09-16）

问题：终端滚轮每格只滚动 1 行，浏览长输出很慢；且滚动偏移超过屏幕行数时 `vt100` 内部会无符号下溢（debug 构建 panic、release 构建取到错误窗口），无法真正滚动到较早的历史内容。

完成结果：

- 本地补丁 `vendor/vt100-0.15.2`：`Grid::visible_rows` 改为在「历史行 + 屏幕行」上滑动窗口并截取屏幕行数
  （`saturating_sub` + `take(rows_len)`），偏移不超过屏幕行数时与原实现结果完全一致；新增
  `Screen::scrollback_count` 暴露已缓存历史行数（Cargo.toml 增加对应的 `[patch.crates-io]`）。
- 滚轮：每格滚动 3 行（按 egui 的 `line_scroll_speed` 换算，不再写死点数）；触控板按滚动距离等比
  换算，每次事件至少 1 行、上限 300 行；滚动上限由 `scrollback_count()` 决定（不再使用固定的缓冲区容量）。
- 新增终端右侧滚动条：轨道锚定在终端画布右缘的留白内（不遮挡文字），拖动滑块按位置比例快速定位
  历史（滑块中心跟随指针），点击轨道空白处按可视行数的一页翻页，无历史内容或轨道过短时不显示；
  几何与映射拆成 `terminal_scrollbar_handle` / `terminal_scrollbar_ratios` /
  `terminal_scrollbar_offset` / `page_scroll_lines` 便于测试。
- 新增 7 个测试（滚轮行数换算、滚动条比例、轨道位置、滑块几何与最短轨道、拖动位置映射、翻页行数、
  历史窗口滑动与偏移钳制）；SSH 客户端相关测试 68 项全部通过。

验证说明：`cargo test` 与 `cargo test --release` 均通过（共 200 项测试：199 项通过、1 项因需要交互式
Windows 会话而忽略），其中历史窗口用例在 debug 构建下验证了原下溢路径已修复；`cargo build`、
`cargo check --release` 通过；`cargo fmt --check` 与 `cargo clippy --all-targets` 对本次新增代码
无差异、无新增告警。

### 11.6 终端配色：主题调色板与 256 色支持（2026-09-16）

问题：深色主题下目录的蓝色高亮（ANSI 4，`#0000AA`）在 `#1e1e1e` 背景上对比度仅约 1.25:1，几乎无法
辨认；同时索引色 ≥ 16 一律回退为默认前景色，使用 256 色的输出会丢失颜色。

完成结果：

- 调色板按主题拆分：新增 `ANSI_PALETTE_DARK`（基于 One Half Dark，蓝色 `#61AFEF` 对比度约 7:1，
  亮色变体整体更亮）与 `ANSI_PALETTE_LIGHT`（保留经典 16 色，适配浅色背景），由
  `ansi_palette(is_dark_mode)` 选择，`render_to_layout_job` 逐帧取一次并在单元格间复用。
- 新增 `xterm_index_to_egui`：`0..16` 取主题调色板、`16..232` 按 xterm 标准 6×6×6 色立方
  （通道取值 `0/95/135/175/215/255`）、`232..256` 为 24 级灰阶（8 起步、步长 10），
  `ls --color`、vim、tmux 等扩展色输出不再丢色；`Color::Rgb` 直通不变。
- 未经显式设置背景色的单元格按语义判断（`vt100::Color::Default`）保持透明，避免调色板取值与
  默认底色相同时误判为「无背景」。
- 新增 4 个测试（主题调色板选择与亮/暗变体特征、深色主题前景色的 WCAG AA 对比度锁定、
  256 色索引解析含色立方与灰阶端点、SGR `01;34` 端到端取色与透明背景）；SSH 客户端相关测试
  72 项全部通过。

验证说明：`cargo test` 共 204 项测试（203 项通过、1 项因需要交互式 Windows 会话而忽略），
`cargo test ssh_client` 72 项全部通过；`cargo build`、独立目标目录 release 构建通过；
`cargo fmt --check` 与 `cargo clippy --all-targets` 对本次新增代码无差异、无新增告警。

### 11.7 窗口缩放时滚动条与历史行同步（2026-09-16）

问题：改变窗口大小时滚动条状态不跟随——最大化后内容已全部可见，滚动条仍然显示；缩小窗口则会
直接丢失最新输出。

原因：`vt100` 的 `Grid::set_size` 只对屏幕行做 `Vec::resize`：放大仅追加空行（历史行数不变，
滚动条判据 `scrollback_count() > 0` 恒成立），缩小直接截断屏幕底部（最新输出丢失，且未进入历史行）。
随后发现第二个问题：放大时补出的空行位于光标之下，恢复窗口时它们会被当作屏幕顶部行推入历史行，
使滚动条范围包含并无远程内容的空行。

完成结果：

- 扩展本地补丁 `vendor/vt100-0.15.2`：新增 `Grid::resize_rows` 并在 `set_size` 中调用——
  放大时从滚动缓冲区尾部回填历史行到屏幕顶部（历史行数随之减少），光标行相应下移；
  缩小时先丢弃光标之下的空白填充行（新增 `Row::is_blank` 判定），若仍需缩小则按底部对齐把顶部行
  推入滚动缓冲区；滚动偏移以「距内容底部的行数」计量，内容底部未变，故偏移保持不变、
  仅按新的历史行数收敛。
- 效果：历史行数随窗口大小动态变化（滚动条显隐与滑块比例实时更新）；放大到内容全部可见时
  历史行数归零、滚动条自动消失；缩小窗口时最新输出不再丢失；**放大后再恢复原尺寸不会把填充
  空行计入历史行，滚动条范围只覆盖远程真实输出**；光标之下的真实内容（如 shell 预测行、
  光标定位输出）保留在屏幕内。
- 新增 6 个测试（放大回填历史行并使滚动条区间归零、缩小推入历史行且最新内容保留、放大后恢复
  不产生填充历史行、缩小只上移保持光标可见所需行数、缩小保留光标之下真实内容、缩放后滚动偏移
  不超出历史行数）；SSH 客户端相关测试 78 项全部通过。

验证说明：`cargo test` 共 210 项测试（209 项通过、1 项因需要交互式 Windows 会话而忽略），
`cargo test ssh_client` 78 项全部通过；`cargo build`、独立目标目录 release 构建通过；
`cargo fmt --check` 与 `cargo clippy --all-targets` 对本次改动无差异、无新增告警。

### 11.8 终端编辑键支持：Home/End 跳转行首行尾（2026-09-22）

问题：终端不响应 Home/End（同样不响应 Insert/Delete/PageUp/PageDown）。按键在本地被完全丢弃，
远端既收不到序列、光标也不移动。

原因：`process_terminal_input` 的 `Event::Key` 分支只覆盖 Enter、Backspace、Tab、Escape、
Ctrl+D/Z/L 与方向键，`egui::Key::Home`/`Key::End` 等编辑键没有任何匹配分支，事件既不发送也不消费。
egui 侧无干扰（`EventFilter` 只涉及 tab、方向键与 escape，不会吞掉编辑键）。

完成结果：

- `terminal.rs`：新增 `TerminalEmulator::application_cursor()`，封装 vt100 的
  `Screen::application_cursor()`，读取远端是否通过 `smkx` 打开应用光标键模式（DECCKM）。
- `ui.rs`：新增 `terminal_edit_key_sequence(key, ctrl, application_cursor)` 映射，并在
  `process_terminal_input` 中按方向键相同的路径发送（`consume_key` + `had_input`，
  输入后视图自动回到底部）：
  - Home / End：普通模式 `\x1b[H` / `\x1b[F`，应用光标键模式 `\x1bOH` / `\x1bOF`
    （与 xterm 及 terminfo 的 `khome` / `kend` 一致，bash/readline 与 vim/less 均可识别）；
  - Ctrl+Home / Ctrl+End：`\x1b[1;5H` / `\x1b[1;5F`，带修饰键时统一为 CSI 1;5 形式，不受 DECCKM 影响；
  - Insert / Delete / PageUp / PageDown：`\x1b[2~` / `\x1b[3~` / `\x1b[5~` / `\x1b[6~`，固定 CSI 形式；
  - Ctrl+Insert、Shift+Insert、Shift+Delete 由平台层转换为系统 Copy/Paste/Cut 事件，不发送到远端。
- 本地历史滚动不受影响：PageUp/PageDown 发送远端序列，本地仍由滚轮与滚动条控制。

测试覆盖（新增 4 项）：

- `terminal::tests::application_cursor_follows_decckm_mode`：`\x1b[?1h` / `\x1b[?1l` 正确切换
  DECCKM 状态
- `ui::tests::terminal_home_end_follow_application_cursor_mode`：普通模式 CSI 形式、应用模式 SS3
  形式、Ctrl 修饰键的 CSI 1;5 形式
- `ui::tests::terminal_edit_keys_send_xterm_sequences`：Insert/Delete/PageUp/PageDown 的 `CSI n ~`
  序列，以及 Ctrl 组合与普通字母键返回 `None`
- `ui::tests::terminal_home_end_are_forwarded_to_remote`：经真实 `process_terminal_input` 管线驱动，
  断言普通模式发送 `\x1b[H` / `\x1b[F`、远端开启 DECCKM 后发送 `\x1bOH` / `\x1bOF`

验证说明：`cargo test` 共 228 项测试（227 项通过、1 项因需要交互式 Windows 会话而忽略），
`cargo test ssh_client` 82 项全部通过；`cargo build`、`cargo build --release` 通过；
`rustfmt --check` 对本次改动文件无差异，`cargo clippy --all-targets` 无新增告警
（bin 66 项、test 69 项与改动前一致）。

---

## 附录：参考项目

- [ssh2-rs](https://github.com/rust-lang/git2-rs) — libssh2 官方 Rust 绑定
- [vt100](https://github.com/oxidecomputer/vt100) — Oxide 公司 Rust 实现的 VT100 终端解析器
- [wezterm](https://github.com/wez/wezterm) — Rust 实现的 GPU 加速终端模拟器（参考 ANSI 处理逻辑）
