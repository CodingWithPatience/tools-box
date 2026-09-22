# 全局快捷键唤出 + 系统托盘 + 设置面板 设计文档

## 概述

本文档描述 Tools Box 的三个核心功能增强：
1. **系统托盘支持** — 关闭按钮最小化到托盘，托盘图标菜单控制显示/退出
2. **全局热键唤出** — Ctrl+Alt+<键> 组合键随时唤出窗口并跳转到指定工具
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
| 唤出工具集 | Ctrl+Alt+Space 唤出窗口并跳转到最近使用的工具 | P0 |
| 唤出指定工具 | Ctrl+Alt+1~7 唤出窗口并跳转到对应插件 | P0 |
| 唤出设置 | Ctrl+Alt+, 唤出窗口并跳转到设置面板 | P0 |
| 唤出到最前层 | 无论窗口处于隐藏、最小化到任务栏还是被其他程序覆盖状态，唤出后都位于桌面最前层 | P0 |
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
    │  │  → 还原 + 置顶抬升 + 跳转   │     │
    │  ├─ HotkeyEvent::ShowTool(i)  │     │
    │  │  → 还原 + 置顶抬升 + 跳转   │     │
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
| 唤出工具集（最近面板） | `Ctrl+Alt+Space` | MOD_WIN \| MOD_ALT, VK_SPACE |
| 唤出密码管理器 | `Ctrl+Alt+1` | MOD_WIN \| MOD_ALT, '1' |
| 唤出 JSON 编辑器 | `Ctrl+Alt+2` | MOD_WIN \| MOD_ALT, '2' |
| 唤出 Hosts 管理器 | `Ctrl+Alt+3` | MOD_WIN \| MOD_ALT, '3' |
| 唤出 Diff 对比 | `Ctrl+Alt+4` | MOD_WIN \| MOD_ALT, '4' |
| 唤出 API 调试 | `Ctrl+Alt+5` | MOD_WIN \| MOD_ALT, '5' |
| 唤出临时笔记 | `Ctrl+Alt+6` | MOD_WIN \| MOD_ALT, '6' |
| 唤出 SSH 客户端 | `Ctrl+Alt+7` | MOD_WIN \| MOD_ALT, '7' |
| 唤出设置 | `Ctrl+Alt+,` | MOD_WIN \| MOD_ALT, VK_OEM_COMMA |

### 3.2 Fn 键说明

Windows 平台上，Fn 键由键盘固件处理，不向操作系统发送标准按键扫描码，无法通过 `RegisterHotKey` API 注册。因此使用 `Ctrl+Alt` 作为默认修饰键组合。`Ctrl+Alt+<key>` 组合在 Windows 系统中极少被占用，是实用的替代方案。

用户可在"设置"面板中自定义修饰键为其他组合（如 `Ctrl+Alt`、`Ctrl+Shift` 等）。

### 3.3 唤出行为（总是桌面最前层）

热键唤出统一走「还原/显示 → 置顶抬升 → 前台激活」流程。窗口状态由热键监听线程在按下瞬间采样
（`window_in_front` = 可见、未最小化且为前台窗口），判定含义为「是否已位于桌面最前层」：

| 按键 | 窗口当前状态 | 动作 |
|------|--------------|------|
| Ctrl+Alt+Space | 已位于桌面最前层 | 隐藏到系统托盘 |
| Ctrl+Alt+Space | 隐藏到托盘 / 最小化到任务栏 / 被其他程序覆盖 | 唤出到桌面最前层（跳转最近使用的工具） |
| Ctrl+Alt+工具热键（默认 1~7） | 已位于桌面最前层 | 仅跳转到对应插件，不触碰窗口 |
| Ctrl+Alt+工具热键（默认 1~7） | 隐藏到托盘 / 最小化到任务栏 / 被其他程序覆盖 | 唤出到桌面最前层并跳转到对应插件 |

置顶抬升通过 `SetWindowPos(HWND_TOPMOST)` → `SetWindowPos(HWND_NOTOPMOST)` 实现：
该操作不受 Windows 前台锁定限制，可把窗口排到所有非置顶窗口之上；随后 `SetForegroundWindow`
抢占前台焦点。抬升是瞬时行为，不保留 `WS_EX_TOPMOST` 永久置顶属性，不影响与其他窗口并排使用。
窗口已位于最前层时不做任何抬升与抢前台，避免仅切换插件时产生 Z 序闪动。

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
/// 默认快捷键字母键（用于全局热键 Ctrl+Alt+<key>）
fn hotkey_char(&self) -> Option<char> {
    None
}
```

#### app.rs — 主应用改造

新增字段：`hotkey_manager`、`tray_manager`、`last_active_tool`、`window_visible`、
`main_window_registered`、`settings_plugin`

关键逻辑：
- `process_hotkey_events()`：处理热键事件，还原窗口 + 置顶抬升到桌面最前层 + 跳转插件
- `process_tray_events()`：处理托盘事件，切换显示/退出
- `register_main_window()`：首帧登记主窗口句柄，供热键与托盘线程唤出窗口使用
- `sync_window_visibility()`：每帧以原生窗口状态（可见且未最小化）双向校准 `window_visible`
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
│  │ 唤出工具集        │ Ctrl+Alt+Space    │ 修改  │ │
│  │ 密码管理器        │ Ctrl+Alt+1        │ 修改  │ │
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
| 2.1 | 添加依赖 | `windows-sys` 已在阶段一添加 | ✅ |
| 2.2 | 实现 `hotkey.rs` 模块 | RegisterHotKey 封装 + 监听线程 + mpsc channel | ✅ |
| 2.3 | 扩展 `Plugin` trait | 新增 `hotkey_char()` 方法 | ✅ |
| 2.4 | 各插件实现 `hotkey_char()` | 每个插件返回默认快捷键字符 | ✅ |
| 2.5 | 集成到 `main.rs` | 启动热键监听线程 | ✅ |
| 2.6 | 集成到 `app.rs` | 热键事件处理 + last_active_tool + 侧边栏提示 | ✅ |

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
| 3.1 | 新增数据库表 | `app_settings` 表 | ✅ |
| 3.2 | 创建插件目录 | `src/plugins/settings/` | ✅ |
| 3.3 | 实现 `models.rs` | AppSettings 数据结构 + 数据库读写 | ✅ |
| 3.4 | 实现 `ui.rs` | 设置面板 UI（主题/字体/侧边栏/热键/自启） | ✅ |
| 3.5 | 实现 `mod.rs` | Plugin trait 实现 | ✅ |
| 3.6 | 改造 `app.rs` | 加载设置，设置变更即时生效，保存到数据库 | ✅ |
| 3.7 | 自定义热键 | 支持为每个工具设置 Ctrl+Alt+任意字母/数字 | ✅ |
| 3.8 | 顶部栏简化 | 移除字体/主题控件，⚙ 设置入口 + 返回按钮 | ✅ |

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
4. **窗口焦点**：热键线程先在唤醒窗口前采样主窗口状态（`is_main_window_in_front()`），
   再按需 `ShowWindowAsync(SW_RESTORE/SW_SHOW)` 还原窗口（热键线程拥有前台激活权限），
   主线程最后 `bring_main_window_to_front()` 置顶抬升 + `SetForegroundWindow`；
   窗口已在前台且按下的是切换热键时不唤醒窗口，避免唤醒先把窗口激活而使隐藏判定失真
5. **窗口状态同步**：`window_visible` 每帧以原生窗口状态（可见且未最小化）为准校准，
   最小化到任务栏会同步为「不可见」，避免热键唤出被误判为「已显示」而停在任务栏
6. **句柄登记时机**：主窗口句柄在首帧 `App::update()` 即登记，而不是等到隐藏到托盘，
   否则窗口最小化在任务栏时线程侧拿不到句柄，无法还原窗口
7. **优雅退出**：托盘"退出"→ 注销热键 → 移除托盘 → 设置 is_quitting → 退出
8. **设置兼容**：app_settings 表不存在时使用 AppSettings::default()
9. **窗口内快捷键保留**：Ctrl+1~9 切换快捷键不受影响，与全局 Ctrl+Alt+1~9 共存

---

## 九、验证方案

1. **系统托盘**：
   - 点击 ✕ → 窗口隐藏，托盘图标出现
   - 左键托盘 → 窗口显示
   - 右键托盘 → 菜单显示、退出有效

2. **全局热键**：
   - 窗口隐藏到托盘后 Ctrl+Alt+Space → 窗口显示并位于桌面最前层
   - 窗口最小化到任务栏后 Ctrl+Alt+Space → 窗口还原并位于桌面最前层（不被其他程序覆盖）
   - 窗口隐藏/最小化到任务栏后 Ctrl+Alt+3 → 窗口唤出到最前层并切换到 Hosts 管理器
   - 窗口已在前台时 Ctrl+Alt+Space → 隐藏到系统托盘
   - 其他应用前台时热键仍能唤出，且唤出后窗口位于其他程序之上

3. **设置插件**：
   - 修改主题/字体/快捷键 → 立即生效
   - 重启 → 设置保持
   - 恢复默认 → 回到初始值

---

## 十、变更记录

### 10.1 热键唤出总是位于桌面最前层（2026-09-22）

问题：程序窗口最小化在任务栏时，热键唤出不会出现在桌面顶层，会被其他程序覆盖；
只有窗口隐藏到系统托盘时，热键唤出才会出现在桌面顶层。

原因：

- `App.window_visible` 只记录「是否隐藏到系统托盘」，窗口被最小化到任务栏时仍为 `true`：
  `Ctrl+Alt+1~7` 的 `if !self.window_visible` 分支不成立，根本不还原窗口；
  `Ctrl+Alt+Space` 反而判定为「已显示」，把窗口隐藏掉。
- `tray::wake_main_window()` 只在 `IsWindowVisible == 0` 时 `ShowWindowAsync(SW_SHOW)`，
  而最小化窗口仍带 `WS_VISIBLE`（只是 `IsIconic`），因此热键线程不会还原它——
  只有隐藏到托盘那条路径能生效，与「仅托盘态唤出才在最顶层」的现象一致。
- 主窗口句柄只在 `hide_window()` 中登记，未隐藏过托盘时句柄为空，
  `wake_main_window()` 直接返回。
- `show_window()` 仅调用 `SetForegroundWindow`，会被 Windows 前台锁定静默拒绝，
  且没有任何 Z 序抬升动作，窗口还原后仍被其他程序覆盖。

完成结果：

- `tray.rs`：`wake_main_window()` 增加最小化分支（`IsIconic` → `ShowWindowAsync(SW_RESTORE)`），
  新增 `main_window_handle()`、`is_main_window_in_front()` 与 `bring_main_window_to_front()`；
  后者先还原/显示窗口，再 `SetWindowPos(HWND_TOPMOST)` → `SetWindowPos(HWND_NOTOPMOST)` 抬升 Z 序
  （不受前台锁定限制），最后 `SetForegroundWindow` 抢前台焦点；两次抬升与抢前台结果分别校验，
  抬升成功仅抢前台失败时降级告警，抬升本身失败单独告警；抬升为瞬时行为，
  不保留 `WS_EX_TOPMOST` 永久置顶属性。
- `hotkey.rs`：`HotkeyEvent` 新增 `window_in_front`，由热键监听线程在唤醒窗口**之前**采样；
  窗口已在前台且按下的是切换热键时不唤醒窗口（否则唤醒会先激活窗口，使主线程的隐藏判定失真）。
- `app.rs`：主窗口句柄在首帧 `update()` 即登记（不再依赖隐藏到托盘）；
  `window_visible` 每帧与原生状态双向校准（可见且未最小化），最小化到任务栏同步为隐藏、
  从任务栏还原同步为可见并恢复渲染；`Ctrl+Alt+Space` 仅在窗口已位于桌面最前层时隐藏到托盘，
  隐藏/最小化/被覆盖时统一唤出到最前层；工具热键在窗口未位于最前层时走 `show_window()`
  （还原 + 置顶 + 抢前台），已位于最前层时只切换插件，不做多余抬升与抢前台。

测试覆盖（新增 7 项）：

- `tray::tests::test_wake_restores_minimized_main_window`：最小化窗口唤醒后不再 `IsIconic`
- `tray::tests::test_bring_main_window_to_front_restores_and_keeps_window_unpinned`：
  唤出后窗口可见且未最小化，且不残留 `WS_EX_TOPMOST`
- `hotkey::tests::test_toggle_hotkey_in_front_does_not_wake_main_window`：
  切换热键在前台时不再唤醒窗口，插件热键与后台/最小化状态仍需唤醒
- `hotkey::tests::test_toggle_hotkey_in_front_forwards_event_without_waking`：
  不唤醒分支仍必须把事件投递给主线程，且事件保留采样到的前台状态
- `app::tests::window_on_screen_detects_hidden_and_minimized_states`：
  最小化窗口仍 `IsWindowVisible`，`is_window_on_screen` 正确判定为不在桌面
- `app::tests::hotkey_toggle_hides_only_when_window_is_in_front`：Space 唤出动作判定
- `app::tests::window_visibility_flag_follows_native_window_state`：可见性标记双向同步

验证说明：`cargo test` 共 224 项测试（223 项通过、1 项因需要交互式 Windows 会话而忽略），
`cargo test app::tests` 8 项、`cargo test tray::tests` 11 项（1 项忽略）、
`cargo test hotkey::tests` 5 项全部通过；`cargo build`、`cargo build --release` 通过；
`rustfmt --check` 对本次改动文件无差异，`cargo clippy --all-targets` 无新增告警
（bin 66 项、test 69 项均为改动前既有告警）。

代码审查（code-reviewer 子代理）结论：通过；已按审查意见修正"工具热键在窗口已位于最前层时
仍强制抬升"的行为回归、补充不唤醒分支的事件投递测试、按抬升结果分别校验并调整日志级别与措辞。
抬升效果本身（窗口确实排到其他窗口之上）依赖真实前台激活，单元测试无法稳定断言，
保留"还原 + 不残留置顶"的断言并交由人工验证。
