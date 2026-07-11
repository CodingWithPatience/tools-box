//! 全局热键管理模块
//!
//! 使用 Windows RegisterHotKey API 注册全局热键，
//! 在独立线程中监听 WM_HOTKEY 消息，通过 mpsc channel 将事件转发给主线程。

use std::sync::mpsc;
use windows_sys::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

/// 热键事件：唤出指定索引的插件（0 = 主窗口/最近工具）
#[derive(Debug, Clone)]
pub struct HotkeyEvent {
    /// 插件索引（0 表示唤出主窗口/最近工具）
    pub plugin_index: usize,
}

/// 热键绑定配置
#[derive(Debug, Clone)]
pub struct HotkeyBinding {
    /// 热键 ID（RegisterHotKey 的 nID 参数）
    pub id: i32,
    /// 修饰键组合
    pub modifiers: HOT_KEY_MODIFIERS,
    /// 虚拟键码
    pub vk: u32,
    /// 对应的插件索引
    pub plugin_index: usize,
}

/// 构建默认热键绑定列表
///
/// 默认快捷键：Win+Alt+Space（主窗口），Win+Alt+1~9（各工具）
pub fn default_bindings(plugin_count: usize) -> Vec<HotkeyBinding> {
    let mut bindings = Vec::new();

    // 主窗口唤出: Ctrl+Alt+Space（plugin_index = usize::MAX 表示恢复最近工具）
    bindings.push(HotkeyBinding {
        id: 1,
        modifiers: MOD_CONTROL | MOD_ALT,
        vk: VK_SPACE as u32,
        plugin_index: usize::MAX,
    });

    // 各工具快捷键: Ctrl+Alt+1~9
    let tool_keys = [
        0x31u32, 0x32, 0x33, 0x34, 0x35, 0x36, 0x37, 0x38, 0x39,
    ]; // '1'~'9'
    for (i, &vk) in tool_keys.iter().enumerate() {
        if i < plugin_count {
            bindings.push(HotkeyBinding {
                id: 2 + i as i32,
                modifiers: MOD_CONTROL | MOD_ALT,
                vk,
                plugin_index: i,
            });
        }
    }

    bindings
}

/// 全局 egui Context（用于唤醒事件循环）
/// 使用 std::sync::OnceLock 确保线程安全
static HOTKEY_EGUI_CTX: std::sync::OnceLock<egui::Context> = std::sync::OnceLock::new();

/// 全局热键管理器
pub struct HotkeyManager {
    rx: mpsc::Receiver<HotkeyEvent>,
    bindings: Vec<HotkeyBinding>,
    /// 监听线程的 HWND（以 isize 存储，因为 HWND 不是 Send）
    listener_hwnd: isize,
    /// 监听线程句柄（用于 Drop 中等待线程退出）
    _handle: std::thread::JoinHandle<()>,
}

impl HotkeyManager {
    /// 创建热键管理器并启动监听线程
    pub fn new(bindings: Vec<HotkeyBinding>) -> Self {
        let (tx, rx) = mpsc::channel::<HotkeyEvent>();
        let (hwnd_tx, hwnd_rx) = mpsc::channel::<isize>();

        // 将绑定列表克隆到监听线程
        let bindings_clone = bindings.clone();

        // 在独立线程中创建隐藏窗口并监听 WM_HOTKEY
        let handle = std::thread::spawn(move || {
            // 注册窗口类
            let class_name: Vec<u16> = "ToolsBoxHotkeyListener\0".encode_utf16().collect();
            // SAFETY: 窗口类注册参数已正确初始化
            unsafe {
                let mut wnd_class: WNDCLASSEXW = std::mem::zeroed();
                wnd_class.cbSize = std::mem::size_of::<WNDCLASSEXW>() as u32;
                wnd_class.lpfnWndProc = Some(hotkey_wnd_proc);
                wnd_class.hInstance = GetModuleHandleW(std::ptr::null());
                wnd_class.lpszClassName = class_name.as_ptr();
                RegisterClassExW(&wnd_class);
            }

            // 创建隐藏消息窗口
            // SAFETY: 窗口类已注册，参数正确
            let hwnd = unsafe {
                CreateWindowExW(
                    WS_EX_NOACTIVATE,
                    class_name.as_ptr(),
                    std::ptr::null(),
                    WS_OVERLAPPED,
                    0, 0, 0, 0,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    GetModuleHandleW(std::ptr::null()),
                    std::ptr::null(),
                )
            };

            // 将 HWND 发送回主线程
            let _ = hwnd_tx.send(hwnd as isize);

            // 注册所有热键
            for binding in &bindings_clone {
                // SAFETY: RegisterHotKey 参数已正确初始化
                let ok = unsafe {
                    RegisterHotKey(hwnd, binding.id, binding.modifiers, binding.vk)
                };
                if ok == 0 {
                    log::warn!(
                        "注册热键失败: id={}, vk={:#x}（可能被其他程序占用）",
                        binding.id, binding.vk
                    );
                } else {
                    log::info!(
                        "注册热键成功: id={}, vk={:#x}, plugin_index={}",
                        binding.id, binding.vk, binding.plugin_index
                    );
                }
            }

            // 消息循环
            let mut msg: MSG = unsafe { std::mem::zeroed() };
            loop {
                // SAFETY: GetMessageW 会阻塞直到收到消息
                let ret = unsafe { GetMessageW(&mut msg, hwnd, 0, 0) };
                if ret == 0 || ret == -1 {
                    break;
                }
                if msg.message == WM_HOTKEY {
                    let hotkey_id = msg.wParam as i32;
                    // 查找对应的绑定
                    if let Some(binding) = bindings_clone.iter().find(|b| b.id == hotkey_id) {
                        let event = HotkeyEvent {
                            plugin_index: binding.plugin_index,
                        };
                        let _ = tx.send(event);
                        // 唤醒 eframe 事件循环
                        if let Some(ctx) = HOTKEY_EGUI_CTX.get() {
                            ctx.request_repaint();
                        }
                    }
                }
            }

            // 清理：注销所有热键并销毁窗口
            for binding in &bindings_clone {
                unsafe { UnregisterHotKey(hwnd, binding.id); }
            }
            unsafe { DestroyWindow(hwnd); }
            log::info!("热键监听线程已退出");
        });

        // 等待子线程返回 HWND（最多等待 2 秒）
        let listener_hwnd = hwnd_rx
            .recv_timeout(std::time::Duration::from_secs(2))
            .unwrap_or_else(|e| {
                log::error!("获取热键监听窗口句柄失败: {:?}", e);
                0
            });

        Self {
            rx,
            bindings,
            listener_hwnd,
            _handle: handle,
        }
    }

    /// 设置全局 egui Context（用于唤醒事件循环）
    pub fn set_egui_ctx(ctx: egui::Context) {
        let _ = HOTKEY_EGUI_CTX.set(ctx);
    }

    /// 轮询热键事件（非阻塞）
    pub fn poll_events(&self) -> Vec<HotkeyEvent> {
        let mut events = Vec::new();
        while let Ok(event) = self.rx.try_recv() {
            events.push(event);
        }
        events
    }

    /// 获取当前绑定列表
    pub fn bindings(&self) -> &[HotkeyBinding] {
        &self.bindings
    }
}

impl Drop for HotkeyManager {
    fn drop(&mut self) {
        // 发送 WM_QUIT 退出监听线程的消息循环
        if self.listener_hwnd != 0 {
            // SAFETY: PostMessageW 参数正确
            unsafe {
                PostMessageW(self.listener_hwnd as HWND, WM_QUIT, 0, 0);
            }
        }
        // _handle 的 Drop 会自动 join 线程
    }
}

/// 热键监听窗口的消息处理过程
///
/// SAFETY: 此函数由 Windows 消息泵调用，hwnd 为有效窗口句柄。
unsafe extern "system" fn hotkey_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> windows_sys::Win32::Foundation::LRESULT {
    match msg {
        WM_HOTKEY => {
            // 热键消息由 GetMessageW 循环处理，此处不需要额外逻辑
            0
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_bindings_count() {
        let bindings = default_bindings(7);
        // 1 个主窗口 + 7 个工具 = 8 个绑定
        assert_eq!(bindings.len(), 8);
    }

    #[test]
    fn test_default_bindings_ids() {
        let bindings = default_bindings(3);
        assert_eq!(bindings[0].id, 1); // Space
        assert_eq!(bindings[1].id, 2); // '1'
        assert_eq!(bindings[2].id, 3); // '2'
        assert_eq!(bindings[3].id, 4); // '3'
    }

    #[test]
    fn test_default_bindings_modifiers() {
        let bindings = default_bindings(1);
        // Ctrl+Alt
        assert_eq!(bindings[0].modifiers, MOD_CONTROL | MOD_ALT);
    }
}
