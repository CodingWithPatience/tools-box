//! 系统托盘管理模块（直接使用 Windows API）
//!
//! 通过 Shell_NotifyIconW 创建托盘图标，CreatePopupMenuW / TrackPopupMenu 管理右键菜单。
//! 消息处理通过 eframe 的 winit 消息泵自动完成（WM_TRAYICON / WM_COMMAND）。

use std::sync::mpsc;
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Shell::{
    NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW, Shell_NotifyIconW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

/// 自定义窗口消息 ID（托盘图标回调）
const WM_TRAYICON: u32 = WM_APP + 1;

/// 菜单项 ID
const MENU_SHOW: usize = 1;
const MENU_QUIT: usize = 2;

/// 窗口属性名（用于存储菜单句柄）
const MENU_PROP: [u16; 3] = [b'T' as u16, b'M' as u16, 0];

/// 托盘事件类型
#[derive(Debug, Clone)]
pub enum TrayEvent {
    ToggleVisible,
    Quit,
}

/// 全局 Sender（避免 Box 被移动导致指针失效）
static mut GLOBAL_TX: Option<mpsc::Sender<TrayEvent>> = None;

/// 全局 egui Context（用于在 send_event 中强制请求重绘，唤醒事件循环）
static mut GLOBAL_EGUI_CTX: Option<egui::Context> = None;

/// 系统托盘管理器
pub struct TrayManager {
    hwnd: HWND,
    nid: NOTIFYICONDATAW,
    hmenu: HMENU,
    icon: HICON,
    rx: mpsc::Receiver<TrayEvent>,
}

impl TrayManager {
    /// 创建系统托盘图标和菜单
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel::<TrayEvent>();

        // 将 Sender 存入全局静态变量（不会被移动，指针始终有效）
        // SAFETY: new() 只在 main 线程调用一次，无竞争
        unsafe {
            GLOBAL_TX = Some(tx);
        }

        // 注册隐藏窗口类
        let class_name: Vec<u16> = "ToolsBoxTrayClass\0".encode_utf16().collect();
        // SAFETY: class_name 在整个函数期间保持有效，wnd_class 字段已正确初始化
        let atom = unsafe {
            let mut wnd_class: WNDCLASSEXW = std::mem::zeroed();
            wnd_class.cbSize = std::mem::size_of::<WNDCLASSEXW>() as u32;
            wnd_class.lpfnWndProc = Some(tray_wnd_proc);
            wnd_class.hInstance = GetModuleHandleW(std::ptr::null());
            wnd_class.lpszClassName = class_name.as_ptr();
            RegisterClassExW(&wnd_class)
        };
        if atom == 0 {
            log::warn!("RegisterClassExW 失败（可能窗口类已注册）");
        }

        // 创建隐藏消息窗口（不需要 lpParam，Sender 在全局静态中）
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

        // 创建托盘菜单
        // SAFETY: CreatePopupMenu 返回有效菜单句柄，AppendMenuW 参数已正确编码
        let hmenu = unsafe { CreatePopupMenu() };
        let show_text: Vec<u16> = "显示窗口\0".encode_utf16().collect();
        let quit_text: Vec<u16> = "退出\0".encode_utf16().collect();
        unsafe {
            AppendMenuW(hmenu, MF_STRING, MENU_SHOW, show_text.as_ptr());
            AppendMenuW(hmenu, MF_SEPARATOR, 0, std::ptr::null());
            AppendMenuW(hmenu, MF_STRING, MENU_QUIT, quit_text.as_ptr());
        }

        // 将菜单句柄存入窗口属性
        // SAFETY: MENU_PROP 是静态生命周期的宽字符串，hmenu 为有效句柄
        unsafe {
            SetPropW(hwnd, MENU_PROP.as_ptr(), hmenu as _);
        }

        // 注册托盘图标
        let icon = create_hicon();
        let mut nid: NOTIFYICONDATAW = unsafe { std::mem::zeroed() };
        nid.hWnd = hwnd;
        nid.uID = 1;
        nid.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP;
        nid.uCallbackMessage = WM_TRAYICON;
        nid.hIcon = icon;
        let tip: Vec<u16> = "Tools Box\0".encode_utf16().collect();
        let tip_len = tip.len().min(127);
        nid.szTip[..tip_len].copy_from_slice(&tip[..tip_len]);

        // SAFETY: nid 结构体字段已正确初始化，Shell_NotifyIconW 会验证参数
        let ok = unsafe { Shell_NotifyIconW(NIM_ADD, &nid) };
        if ok == 1 {
            log::info!("系统托盘图标已创建");
        } else {
            log::error!("创建系统托盘图标失败");
        }

        Self {
            hwnd,
            nid,
            hmenu,
            icon,
            rx,
        }
    }

    /// 轮询托盘事件（非阻塞）
    pub fn poll_events(&self) -> Vec<TrayEvent> {
        let mut events = Vec::new();
        while let Ok(event) = self.rx.try_recv() {
            events.push(event);
        }
        events
    }
}

impl Drop for TrayManager {
    fn drop(&mut self) {
        // SAFETY: 按正确顺序清理资源
        unsafe {
            Shell_NotifyIconW(NIM_DELETE, &self.nid);
            DestroyIcon(self.icon);
            DestroyMenu(self.hmenu);
            DestroyWindow(self.hwnd);
            GLOBAL_TX = None;
        }
        log::info!("系统托盘已清理");
    }
}

/// 托盘隐藏窗口的消息处理过程
///
/// SAFETY: 此函数由 Windows 消息泵调用，hwnd 为有效窗口句柄。
unsafe extern "system" fn tray_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_TRAYICON => {
            let mouse_msg = (lparam & 0xFFFF) as u32;
            match mouse_msg {
                WM_LBUTTONUP => {
                    log::info!("[托盘] 左键点击 → 切换窗口");
                    send_event(TrayEvent::ToggleVisible);
                }
                WM_RBUTTONUP => {
                    log::info!("[托盘] 右键点击 → 显示菜单");
                    show_menu(hwnd);
                }
                _ => {}
            }
            0
        }
        WM_COMMAND => {
            let menu_id = wparam & 0xFFFF;
            match menu_id as usize {
                MENU_SHOW => {
                    log::info!("[托盘菜单] 显示窗口");
                    send_event(TrayEvent::ToggleVisible);
                }
                MENU_QUIT => {
                    log::info!("[托盘菜单] 退出");
                    send_event(TrayEvent::Quit);
                }
                _ => {}
            }
            0
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

/// 通过全局 Sender 发送事件，并通过 egui Context 强制请求重绘
///
/// SAFETY: GLOBAL_TX 和 GLOBAL_EGUI_CTX 在 App 初始化时设置，在 Drop 中清除。
unsafe fn send_event(event: TrayEvent) {
    if let Some(ref tx) = GLOBAL_TX {
        match tx.send(event) {
            Ok(()) => {
                log::debug!("[托盘] 事件发送成功");
                // 通过 egui Context 强制请求重绘
                // TrackPopupMenu 的模态循环会干扰 winit，导致 update() 停止调用
                // request_repaint 通过 EventLoopProxy 唤醒 winit 事件循环
                if let Some(ref ctx) = GLOBAL_EGUI_CTX {
                    ctx.request_repaint();
                }
            }
            Err(e) => log::error!("[托盘] 事件发送失败: {:?}", e),
        }
    } else {
        log::error!("[托盘] GLOBAL_TX 未初始化！");
    }
}

/// 设置全局 egui Context（在 App::new 中调用）
///
/// SAFETY: 只在 App::new 中调用一次，无竞争。
pub fn set_egui_ctx(ctx: egui::Context) {
    unsafe {
        GLOBAL_EGUI_CTX = Some(ctx);
    }
}

/// 显示托盘右键菜单
///
/// SAFETY: hwnd 为有效窗口句柄，hmenu 通过窗口属性存储且生命周期与窗口一致。
unsafe fn show_menu(hwnd: HWND) {
    let mut point: POINT = std::mem::zeroed();
    GetCursorPos(&mut point);
    SetForegroundWindow(hwnd);
    let hmenu = GetPropW(hwnd, MENU_PROP.as_ptr()) as HMENU;
    if !hmenu.is_null() {
        TrackPopupMenu(
            hmenu,
            TPM_BOTTOMALIGN | TPM_LEFTALIGN,
            point.x,
            point.y,
            0,
            hwnd,
            std::ptr::null(),
        );
        PostMessageW(hwnd, WM_NULL, 0, 0);
    }
}

/// 创建程序化 HICON（蓝色工具箱图标，16x16）
fn create_hicon() -> HICON {
    let size = 16u32;
    let pixel_count = (size * size) as usize;
    let bytes_per_row = ((size + 7) / 8) as usize;
    let mask_byte_count = bytes_per_row * size as usize;

    let mut and_mask = vec![0xFFu8; mask_byte_count];
    let mut xor_mask = vec![0u8; pixel_count * 4];

    for y in 0..size {
        for x in 0..size {
            let pix_idx = (y * size + x) as usize * 4;
            let mask_idx = y as usize * bytes_per_row + (x / 8) as usize;
            let mask_bit = 1u8 << (7 - (x % 8));

            debug_assert!(mask_idx < and_mask.len());
            debug_assert!(pix_idx + 3 < xor_mask.len());

            if is_icon_corner(x, y) {
                and_mask[mask_idx] |= mask_bit;
            } else if is_icon_border(x, y) {
                and_mask[mask_idx] &= !mask_bit;
                xor_mask[pix_idx] = 200;
                xor_mask[pix_idx + 1] = 130;
                xor_mask[pix_idx + 2] = 70;
            } else {
                and_mask[mask_idx] &= !mask_bit;
                xor_mask[pix_idx] = 170;
                xor_mask[pix_idx + 1] = 100;
                xor_mask[pix_idx + 2] = 50;
            }
        }
    }

    // SAFETY: CreateIcon 的 and_mask/xor_mask 指针在函数调用期间有效
    unsafe {
        CreateIcon(
            std::ptr::null_mut(),
            size as i32,
            size as i32,
            1,
            32,
            and_mask.as_ptr(),
            xor_mask.as_ptr(),
        )
    }
}

fn is_icon_corner(x: u32, y: u32) -> bool {
    let r = 2i32;
    let rsq = r * r;
    let checks = [
        (0u32, 0u32, r, r),
        (15, 0, 13, r),
        (0, 15, r, 13),
        (15, 15, 13, 13),
    ];
    for (bx, by, cx, cy) in checks {
        if x == bx && y == by {
            let dx = cx - x as i32;
            let dy = cy - y as i32;
            return dx * dx + dy * dy > rsq;
        }
    }
    false
}

fn is_icon_border(x: u32, y: u32) -> bool {
    x == 0 || x == 15 || y == 0 || y == 15
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_icon_corner() {
        assert!(is_icon_corner(0, 0));
        assert!(is_icon_corner(15, 0));
        assert!(is_icon_corner(0, 15));
        assert!(is_icon_corner(15, 15));
        assert!(!is_icon_corner(8, 8));
        assert!(!is_icon_corner(0, 8));
        assert!(!is_icon_corner(8, 0));
    }

    #[test]
    fn test_is_icon_border() {
        assert!(is_icon_border(0, 5));
        assert!(is_icon_border(15, 5));
        assert!(is_icon_border(5, 0));
        assert!(is_icon_border(5, 15));
        assert!(!is_icon_border(8, 8));
    }

    #[test]
    fn test_create_hicon_not_null() {
        let icon = create_hicon();
        assert!(!icon.is_null());
        // SAFETY: icon 由 CreateIcon 创建，可以安全销毁
        unsafe {
            DestroyIcon(icon);
        }
    }
}
