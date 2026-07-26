//! 全局热键管理模块
//!
//! 使用 Windows RegisterHotKey API 注册全局热键，
//! 在独立线程中监听 WM_HOTKEY 消息，通过 mpsc channel 将事件转发给主线程。

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc,
};
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

/// 全局 egui Context（用于唤醒事件循环）
/// 使用 std::sync::OnceLock 确保线程安全
static HOTKEY_EGUI_CTX: std::sync::OnceLock<egui::Context> = std::sync::OnceLock::new();

/// 转发全局热键事件，并在发送成功后唤醒主窗口。
fn dispatch_hotkey_event(
    tx: &mpsc::Sender<HotkeyEvent>,
    event: HotkeyEvent,
) -> Result<(), mpsc::SendError<HotkeyEvent>> {
    dispatch_hotkey_event_with_wake(tx, event, crate::tray::wake_main_window)
}

fn dispatch_hotkey_event_with_wake(
    tx: &mpsc::Sender<HotkeyEvent>,
    event: HotkeyEvent,
    wake_main_window: impl FnOnce(),
) -> Result<(), mpsc::SendError<HotkeyEvent>> {
    tx.send(event)?;
    wake_main_window();

    if let Some(ctx) = HOTKEY_EGUI_CTX.get() {
        ctx.request_repaint();
    }

    Ok(())
}

/// 全局热键管理器
pub struct HotkeyManager {
    rx: mpsc::Receiver<HotkeyEvent>,
    bindings: Vec<HotkeyBinding>,
    /// 监听线程的 HWND（以 isize 存储，因为 HWND 不是 Send）
    listener_hwnd: isize,
    /// 发送给监听线程的热键更新请求
    update_tx: mpsc::Sender<Vec<HotkeyBinding>>,
    /// 通知监听线程停止。
    shutdown: Arc<AtomicBool>,
    /// 监听线程句柄（用于 Drop 中等待线程退出）。
    handle: Option<std::thread::JoinHandle<()>>,
}

impl HotkeyManager {
    /// 创建热键管理器并启动监听线程
    pub fn new(bindings: Vec<HotkeyBinding>) -> Self {
        let (tx, rx) = mpsc::channel::<HotkeyEvent>();
        let (hwnd_tx, hwnd_rx) = mpsc::channel::<isize>();
        let (update_tx, update_rx) = mpsc::channel::<Vec<HotkeyBinding>>();
        let shutdown = Arc::new(AtomicBool::new(false));
        let listener_shutdown = Arc::clone(&shutdown);

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
                    0,
                    0,
                    0,
                    0,
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
                let ok = unsafe { RegisterHotKey(hwnd, binding.id, binding.modifiers, binding.vk) };
                if ok == 0 {
                    log::warn!(
                        "注册热键失败: id={}, vk={:#x}（可能被其他程序占用）",
                        binding.id,
                        binding.vk
                    );
                } else {
                    log::info!(
                        "注册热键成功: id={}, vk={:#x}, plugin_index={}",
                        binding.id,
                        binding.vk,
                        binding.plugin_index
                    );
                }
            }

            // 消息循环（使用 PeekMessageW 非阻塞，以便检查更新请求）
            let mut current_bindings = bindings_clone;
            let mut msg: MSG = unsafe { std::mem::zeroed() };
            while !listener_shutdown.load(Ordering::Acquire) {
                // 检查是否有热键更新请求
                if let Ok(new_bindings) = update_rx.try_recv() {
                    // 注销旧热键
                    for binding in &current_bindings {
                        unsafe {
                            UnregisterHotKey(hwnd, binding.id);
                        }
                    }
                    // 注册新热键
                    for binding in &new_bindings {
                        let ok = unsafe {
                            RegisterHotKey(hwnd, binding.id, binding.modifiers, binding.vk)
                        };
                        if ok == 0 {
                            log::warn!("更新热键失败: id={}, vk={:#x}", binding.id, binding.vk);
                        } else {
                            log::info!("更新热键成功: id={}, vk={:#x}", binding.id, binding.vk);
                        }
                    }
                    current_bindings = new_bindings;
                }

                // 非阻塞获取消息
                let ret = unsafe { PeekMessageW(&mut msg, hwnd, 0, 0, PM_REMOVE) };
                if ret != 0 {
                    if msg.message == WM_QUIT {
                        break;
                    }
                    if msg.message == WM_HOTKEY {
                        let hotkey_id = msg.wParam as i32;
                        if let Some(binding) = current_bindings.iter().find(|b| b.id == hotkey_id) {
                            let event = HotkeyEvent {
                                plugin_index: binding.plugin_index,
                            };
                            match dispatch_hotkey_event(&tx, event) {
                                Ok(()) => {}
                                Err(error) => {
                                    log::error!("发送全局热键事件失败: {}", error);
                                }
                            }
                        }
                    }
                } else {
                    // 无消息时等待唤醒或超时，避免空转并允许析构立即停止监听。
                    std::thread::park_timeout(std::time::Duration::from_millis(50));
                }
            }

            // 清理：注销所有热键并销毁窗口
            for binding in &current_bindings {
                unsafe {
                    UnregisterHotKey(hwnd, binding.id);
                }
            }
            unsafe {
                DestroyWindow(hwnd);
            }
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
            update_tx,
            shutdown,
            handle: Some(handle),
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

    /// 动态更新热键绑定
    ///
    /// 注销旧热键，注册新热键，立即生效。
    pub fn update_bindings(&mut self, new_bindings: Vec<HotkeyBinding>) {
        if let Err(e) = self.update_tx.send(new_bindings.clone()) {
            log::error!("发送热键更新请求失败: {}", e);
        } else {
            self.bindings = new_bindings;
            log::info!("热键更新请求已发送");
        }
    }
}

impl Drop for HotkeyManager {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Release);

        if let Some(handle) = self.handle.take() {
            handle.thread().unpark();
            if handle.join().is_err() {
                log::error!("等待热键监听线程退出时发生 panic");
            } else {
                log::debug!("热键监听窗口已退出: hwnd={}", self.listener_hwnd);
            }
        }
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
        _ => {
            // SAFETY: 未处理消息按 Win32 约定转发给默认窗口过程。
            unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn test_hotkey_event_wakes_main_window_after_forwarding() {
        let (tx, rx) = mpsc::channel();
        let wake_called = Cell::new(false);

        let result = dispatch_hotkey_event_with_wake(
            &tx,
            HotkeyEvent {
                plugin_index: usize::MAX,
            },
            || wake_called.set(true),
        );

        assert!(result.is_ok(), "热键事件应成功转发");
        assert!(wake_called.get(), "转发热键事件后必须唤醒主窗口");
        match rx.try_recv() {
            Ok(event) => assert_eq!(event.plugin_index, usize::MAX),
            Err(error) => panic!("未收到热键事件: {error}"),
        }
    }

    #[test]
    fn test_failed_hotkey_forward_does_not_wake_main_window() {
        let (tx, rx) = mpsc::channel();
        drop(rx);
        let wake_called = Cell::new(false);

        let result = dispatch_hotkey_event_with_wake(
            &tx,
            HotkeyEvent {
                plugin_index: usize::MAX,
            },
            || wake_called.set(true),
        );

        assert!(result.is_err(), "接收端关闭后事件转发应失败");
        assert!(!wake_called.get(), "事件未转发时不应唤醒主窗口");
    }

    #[test]
    fn test_hotkey_manager_drop_waits_for_listener_window_cleanup() {
        let manager = HotkeyManager::new(Vec::new());
        let listener_hwnd = manager.listener_hwnd;
        assert_ne!(listener_hwnd, 0, "热键监听窗口应创建成功");

        drop(manager);

        // SAFETY: 仅查询已记录的窗口句柄是否仍有效，不解引用任何指针。
        assert_eq!(
            unsafe { IsWindow(listener_hwnd as HWND) },
            0,
            "HotkeyManager 析构完成后监听窗口应已销毁"
        );
    }
}
