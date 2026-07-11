# 全局快捷键唤出 + 系统托盘 + 设置面板 设计文档

## 概述

本文档描述 Tools Box 的三个核心功能增强：
1. **系统托盘支持** — 关闭按钮最小化到托盘，托盘图标菜单控制显示/退出
2. **全局热键唤出** — Win+Alt+<键> 组合键随时唤出窗口并跳转到指定工具
3. **设置插件** — 统一管理快捷键自定义、字体大小、主题设置，持久化到 SQLite

---

## 一、功能需求

### 1.1 系统托盘

| 功能 | 说明 | 优先级 |
|------|------|--------|
| 关闭最小化到托盘 | 点击 ✕ 按钮隐藏窗口而非退出程序 | P0 |
| 托盘图标 | 任务栏托盘区域显示 Tools Box 图标 | P0 |
| 左键切换显示 | 左键单击托盘图标显示/隐藏窗口 | P0 |
| 右键菜单 | 右键弹出菜单：显示窗口、退出 | P0 |
| 退出程序 | 托盘菜单"退出"完全关闭程序和后台线程 | P0 |

### 1.2 全局热键

| 功能 | 说明 | 优先级 |
|------|------|--------|
| 唤出工具集 | Win+Alt+Space 显示窗口并跳转到最近使用的工具 | P0 |
| 唤出指定工具 | Win+Alt+1~7 显示窗口并跳转到对应插件 | P0 |
| 唤出设置 | Win+Alt+, 显示窗口并跳转到设置面板 | P0 |
| 快捷键自定义 | 设置面板中可修改修饰键和字母键 | P1 |
| 热键动态更新 | 设置变更后立即重新注册热键，无需重启 | P1 |

### 1.3 设置插件

| 功能 | 说明 | 优先级 |
|------|------|--------|
| 主题设置 | 亮色/暗色主题切换 | P0 |
| 字体大小设置 | 字体大小调节 (10-24) | P0 |
| 快捷键配置 | 修饰键和工具快捷键字母自定义 | P0 |
| 设置持久化 | SQLite 存储，重启后保持 | P0 |
| 恢复默认 | 一键恢复所有设置为默认值 | P1 |

---

## 二、架构设计

### 2.1 整体架构

```
┌──────────────────────────────────────────────────────────────────┐
│                         main.rs                                    │
│  1. 初始化数据库                                                    │
│  2. 加载设置 (AppSettings)                                        │
│  3. 创建事件 channel (热键 + 托盘)                                  │
│  4. 启动热键监听线程 (hotkey_watcher)                               │
│  5. 创建系统托盘 (TrayManager)                                     │
│  6. 启动 eframe 事件循环 (run_and_return: true)                    │
│  7. 退出时清理热键和托盘                                            │
└──────────────────────────────────────────────────────────────────┘

┌──────────────────────┐     mpsc::Sender      ┌───────────────────┐
│   热键监听线程         │ ───────────────────→  │   主线程 (egui)    │
│   (hotkey::watcher)   │   HotkeyEvent         │   App::update()   │
│                       │                       │                   │
│  RegisterHotKey()     │                       │  每帧检查 channel  │
│  GetMessageW() 循环   │                       │  处理热键事件      │
│  解析 WM_HOTKEY       │                       │  ShowWindow()     │
└──────────────────────┘                       └───────────────────┘
         ↑
         │  WM_HOTKEY
         │
┌──────────────────────┐
│  Windows OS           │
│  RegisterHotKey API   │
└──────────────────────┘
```

### 2.2 线程模型

```
主线程 (eframe)                 热键监听线程               Windows 消息循环
    │                               │
    │  创建 channel                  │
    │ ──────────────────────────→  │
    │                               │  创建隐藏消息窗口
    │                               │  注册热键 RegisterHotKey()
    │                               │
    │                               │  循环: GetMessageW()
    │                               │     ├─ WM_HOTKEY →
    │                               │     │    tx.send(HotkeyEvent)
    │  每帧检查 rx.try_recv()       │     │
    │  ├─ HotkeyEvent::ShowApp      │     │
    │  │  → ShowWindow() + 跳转     │     │
    │  ├─ HotkeyEvent::ShowTool(i)  │     │
    │  │  → ShowWindow() + 跳转插件 │     │
    │  └─ 无事件 → 继续渲染         │     │
    │                               │     │
    │  窗口关闭事件                  │     │
    │  → CancelClose + 隐藏窗口     │     │
    │  → 不退出程序                  │     │
    │                               │     │
    │  托盘"退出"菜单                │     │
    │  → 注销热键                    │  ← UnregisterHotKey()
    │  → 移除托盘图标                │  ← Shell_NotifyIcon(NIM_DELETE)
    │  → std::process::exit(0)      │     │
```

---

## 三、快捷键设计

### 3.1 默认快捷键映射

| 功能 | 默认快捷键 | RegisterHotKey 参数 |
|------|-----------|---------------------|
| 唤出工具集（最近面板） | `Win+Alt+Space` | MOD_WIN \| MOD_ALT, VK_SPACE |
| 唤出密码管理器 | `Win+Alt+1` | MOD_WIN \| MOD_ALT, '1' |
| 唤出 JSON 编辑器 | `Win+Alt+2` | MOD_WIN \| MOD_ALT, '2' |
| 唤出 Hosts 管理器 | `Win+Alt+3` | MOD_WIN \| MOD_ALT, '3' |
| 唤出 Diff 对比 | `Win+Alt+4` | MOD_WIN \| MOD_ALT, '4' |
| 唤出 API 调试 | `Win+Alt+5` | MOD_WIN \| MOD_ALT, '5' |
| 唤出临时笔记 | `Win+Alt+6` | MOD_WIN \| MOD_ALT, '6' |
| 唤出 SSH 客户端 | `Win+Alt+7` | MOD_WIN \| MOD_ALT, '7' |
| 唤出设置 | `Win+Alt+,` | MOD_WIN \| MOD_ALT, VK_OEM_COMMA |

### 3.2 Fn 键说明

Windows 平台上，Fn 键由键盘固件处理，不向操作系统发送标准按键扫描码，无法通过 `RegisterHotKey` API 注册。因此使用 `Win+Alt` 作为默认修饰键组合。`Win+Alt+<key>` 组合在 Windows 系统中极少被占用，是实用的替代方案。

用户可在"设置"面板中自定义修饰键为其他组合（如 `Ctrl+Alt`、`Ctrl+Shift` 等）。

---

## 四、模块设计

### 4.1 文件清单

```
src/
├── hotkey.rs              # [新增] 全局热键管理（线程 + Windows API 封装）
├── tray.rs                # [新增] 系统托盘管理（tray-icon 封装）
├── app.rs                 # [修改] 集成热键/托盘事件，设置入口
├── main.rs                # [修改] 初始化热键线程和系统托盘
├── plugin.rs              # [修改] Plugin trait 新增 hotkey_char() 方法
├── plugins/
│   ├── mod.rs             # [修改] 注册 settings 插件
│   └── settings/          # [新增] 设置插件目录
│       ├── mod.rs         # 插件入口，Plugin trait 实现
│       ├── ui.rs          # 设置 UI（快捷键配置 / 字体 / 主题）
│       └── models.rs      # 快捷键配置数据结构
├── storage/
│   └── database.rs        # [修改] 新建设置表 app_settings
```

### 4.2 核心模块

#### hotkey.rs — 全局热键管理

- 使用 `windows` crate 调用 Win32 `RegisterHotKey`/`UnregisterHotKey` API
- 独立线程创建隐藏消息窗口，`GetMessageW` 循环接收 `WM_HOTKEY`
- 通过 `mpsc::channel` 将热键事件转发给主线程
- `HotkeyController` 支持运行时动态更新热键绑定

#### tray.rs — 系统托盘管理

- 使用 `tray-icon` 0.21 crate（纯 Rust，`Shell_NotifyIconW` 封装）
- 托盘图标左键单击切换窗口显示/隐藏
- 右键菜单："显示窗口" / "退出"
- 通过 `mpsc::channel` 将托盘事件转发给主线程

#### plugin.rs — Plugin trait 扩展

新增方法：
```rust
/// 默认快捷键字母键（用于全局热键 Win+Alt+<key>）
fn hotkey_char(&self) -> Option<char> {
    None
}
```

#### app.rs — 主应用改造

新增字段：`hotkey_rx`、`tray_rx`、`last_active_tool`、`window_visible`、`is_quitting`、`settings`

关键逻辑：
- `process_hotkey_events()`：处理热键事件，显示窗口 + 跳转插件
- `process_tray_events()`：处理托盘事件，切换显示/退出
- 关闭事件拦截：`CancelClose` + `ShowWindow(SW_HIDE)`
- 窗口显隐：使用原生 `ShowWindow` API 替代 egui ViewportCommand

#### settings 插件

设置面板 UI 布局：
```
┌─ 外观设置 ────────────────────────────────────┐
│  主题：     [ ● 暗色  ○ 亮色 ]                  │
│  字体大小： [ A- ] 14.0 [ A+ ]  [ 重置默认 ]    │
└───────────────────────────────────────────────┘

┌─ 全局快捷键设置 ───────────────────────────────┐
│  唤出修饰键：  ☑ Win  ☑ Alt  ☐ Ctrl  ☐ Shift  │
│  快捷键列表：                                    │
│  ┌──────────────────┬──────────────────┬──────┐ │
│  │ 功能              │ 快捷键            │ 操作  │ │
│  ├──────────────────┼──────────────────┼──────┤ │
│  │ 唤出工具集        │ Win+Alt+Space    │ 修改  │ │
│  │ 密码管理器        │ Win+Alt+1        │ 修改  │ │
│  │ ...              │ ...              │ ...  │ │
│  └──────────────────┴──────────────────┴──────┘ │
└───────────────────────────────────────────────┘
```

---

## 五、数据库设计

```sql
-- 应用设置表（单行配置）
CREATE TABLE IF NOT EXISTS app_settings (
    id              INTEGER PRIMARY KEY DEFAULT 1,
    theme           TEXT NOT NULL DEFAULT 'dark',
    font_size       REAL NOT NULL DEFAULT 14.0,
    hotkey_modifiers TEXT NOT NULL DEFAULT '{"alt":true,"ctrl":false,"shift":false,"win":true}',
    tool_hotkeys    TEXT NOT NULL DEFAULT '[]',
    sidebar_width   REAL NOT NULL DEFAULT 200.0,
    updated_at      DATETIME DEFAULT CURRENT_TIMESTAMP
);
```

---

## 六、依赖变更

```toml
[dependencies]
# Windows API（托盘 + 全局热键）
windows-sys = { version = "0.60", features = [
    "Win32_UI_Shell",
    "Win32_UI_WindowsAndMessaging",
    "Win32_Foundation",
    "Win32_System_LibraryLoader",
] }
```

---

## 七、实施计划

### 阶段一：系统托盘支持

| 步骤 | 任务 | 说明 | 状态 |
|------|------|------|------|
| 1.1 | 添加依赖 | `tray-icon` 加入 Cargo.toml | ✅ |
| 1.2 | 实现 `tray.rs` 模块 | 创建托盘图标、右键菜单、左键切换 | ✅ |
| 1.3 | 改造 `main.rs` | 初始化托盘管理器，run_and_return: true | ✅ |
| 1.4 | 改造 `app.rs` | 拦截关闭事件 → 隐藏窗口，处理托盘事件 | ✅ |

**阶段一产出文件：**
- `Cargo.toml` — 新增 `tray-icon` 依赖
- `src/tray.rs` — 系统托盘管理模块
- `src/main.rs` — 集成托盘初始化
- `src/app.rs` — 窗口隐藏/显示逻辑 + 托盘事件处理

### 阶段二：全局热键支持

| 步骤 | 任务 | 说明 | 状态 |
|------|------|------|------|
| 2.1 | 添加依赖 | `windows-sys` 已在阶段一添加 | ⏳ |
| 2.2 | 实现 `hotkey.rs` 模块 | RegisterHotKey 封装 + 监听线程 | ⏳ |
| 2.3 | 扩展 `Plugin` trait | 新增 `hotkey_char()` 方法 | ⏳ |
| 2.4 | 各插件实现 `hotkey_char()` | 每个插件返回默认快捷键字母 | ⏳ |
| 2.5 | 集成到 `main.rs` | 启动热键监听线程 | ⏳ |
| 2.6 | 集成到 `app.rs` | 热键事件处理 + last_active_tool | ⏳ |

**阶段二产出文件：**
- `Cargo.toml` — 新增 `windows` 依赖
- `src/hotkey.rs` — 热键管理模块
- `src/plugin.rs` — Plugin trait 扩展
- `src/main.rs` — 集成热键线程
- `src/app.rs` — 热键事件处理
- `src/plugins/*/mod.rs` — 各插件实现 `hotkey_char()`

### 阶段三：设置插件 + 设置持久化

| 步骤 | 任务 | 说明 | 状态 |
|------|------|------|------|
| 3.1 | 新增数据库表 | `app_settings` 表 | ⏳ |
| 3.2 | 创建插件目录 | `src/plugins/settings/` | ⏳ |
| 3.3 | 实现 `models.rs` | AppSettings 数据结构 + 数据库读写 | ⏳ |
| 3.4 | 实现 `ui.rs` | 设置面板 UI | ⏳ |
| 3.5 | 实现 `mod.rs` | Plugin trait 实现 + 注册 | ⏳ |
| 3.6 | 改造 `app.rs` | 加载/保存设置，更新顶部栏 | ⏳ |
| 3.7 | 动态热键更新 | 设置变更后重新注册热键 | ⏳ |
| 3.8 | 顶部栏简化 | 移除字体/主题控件，保留设置入口 | ⏳ |

**阶段三产出文件：**
- `src/storage/database.rs` — 新增 app_settings 表
- `src/plugins/settings/mod.rs` — 设置插件入口
- `src/plugins/settings/ui.rs` — 设置 UI
- `src/plugins/settings/models.rs` — 数据模型
- `src/plugins/mod.rs` — 注册设置插件
- `src/app.rs` — 加载/保存设置

### 阶段四：集成测试与打磨

| 步骤 | 任务 | 说明 | 状态 |
|------|------|------|------|
| 4.1 | 端到端测试 | 托盘隐藏/显示、热键唤出、快捷键跳转 | ⏳ |
| 4.2 | 设置持久化测试 | 重启后设置保持 | ⏳ |
| 4.3 | 热键冲突处理 | 快捷键被占用时警告 | ⏳ |
| 4.4 | 构建验证 | cargo build --release 通过 | ⏳ |

---

## 八、技术注意事项

1. **线程安全**：热键监听线程只写 mpsc channel，主线程只读，无需 Mutex
2. **热键冲突**：RegisterHotKey 失败时记录 warn! 日志，不 panic
3. **托盘图标**：使用程序内置生成的 RGBA pixel buffer 作为图标
4. **窗口焦点**：show_window() 中调用 SetForegroundWindow 带到前台
5. **优雅退出**：托盘"退出"→ 注销热键 → 移除托盘 → 设置 is_quitting → 退出
6. **设置兼容**：app_settings 表不存在时使用 AppSettings::default()
7. **窗口内快捷键保留**：Ctrl+1~9 切换快捷键不受影响，与全局 Win+Alt+1~9 共存

---

## 九、验证方案

1. **系统托盘**：
   - 点击 ✕ → 窗口隐藏，托盘图标出现
   - 左键托盘 → 窗口显示
   - 右键托盘 → 菜单显示、退出有效

2. **全局热键**：
   - 窗口隐藏后 Win+Alt+Space → 窗口显示
   - 窗口隐藏后 Win+Alt+3 → 窗口显示并切换到 Hosts 管理器
   - 其他应用前台时热键仍能唤出

3. **设置插件**：
   - 修改主题/字体/快捷键 → 立即生效
   - 重启 → 设置保持
   - 恢复默认 → 回到初始值
