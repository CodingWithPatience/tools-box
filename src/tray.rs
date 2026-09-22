//! 系统托盘管理模块（直接使用 Windows API）
//!
//! 通过 Shell_NotifyIconW 创建托盘图标，CreatePopupMenuW / TrackPopupMenu 管理右键菜单。
//! 消息处理通过 eframe 的 winit 消息泵接收 WM_TRAYICON，
//! 右键菜单通过 TPM_RETURNCMD 在模态循环结束后再分发业务事件。

use std::ffi::c_void;
use std::sync::{
    atomic::{AtomicPtr, Ordering},
    mpsc,
};
#[cfg(test)]
use windows_sys::Win32::Foundation::GetLastError;
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Shell::{
    NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW, Shell_NotifyIconW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

/// 自定义窗口消息 ID（托盘图标回调）
const WM_TRAYICON: u32 = WM_APP + 1;
/// 重复启动进程请求显示现有窗口的消息 ID。
const WM_SHOW_EXISTING_INSTANCE: u32 = WM_APP + 2;
/// 托盘消息窗口类名。
const TRAY_WINDOW_CLASS_NAME: &str = "ToolsBoxTrayClass";

/// 菜单项 ID
const MENU_SHOW: usize = 1;
const MENU_QUIT: usize = 2;

/// 窗口属性名（用于存储菜单句柄）
const MENU_PROP: [u16; 3] = [b'T' as u16, b'M' as u16, 0];

/// 托盘事件类型
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayEvent {
    ToggleVisible,
    ShowWindow,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TrayMenuAction {
    ShowWindow,
    Quit,
}

/// 全局 Sender（避免 Box 被移动导致指针失效）
static mut GLOBAL_TX: Option<mpsc::Sender<TrayEvent>> = None;

/// 全局 egui Context（用于在 send_event 中强制请求重绘，唤醒事件循环）
static mut GLOBAL_EGUI_CTX: Option<egui::Context> = None;

/// 主窗口句柄，用于在窗口隐藏时通过原生消息唤醒 eframe 事件循环。
static MAIN_WINDOW_HWND: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());

/// 系统托盘管理器
pub struct TrayManager {
    hwnd: HWND,
    nid: NOTIFYICONDATAW,
    icon_registered: bool,
    #[cfg(test)]
    registration_error: u32,
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
        let class_name = tray_window_class_name();
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
        let mut nid = notify_icon_data(hwnd);
        nid.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP;
        nid.uCallbackMessage = WM_TRAYICON;
        nid.hIcon = icon;
        let tip: Vec<u16> = "Tools Box\0".encode_utf16().collect();
        let tip_len = tip.len().min(127);
        nid.szTip[..tip_len].copy_from_slice(&tip[..tip_len]);

        // SAFETY: nid 结构体字段已正确初始化，Shell_NotifyIconW 会验证参数
        let ok = unsafe { Shell_NotifyIconW(NIM_ADD, &nid) };
        let icon_registered = ok != 0;
        #[cfg(test)]
        let registration_error = if icon_registered {
            0
        } else {
            // SAFETY: 紧接失败的 Win32 调用读取当前线程的错误码。
            unsafe { GetLastError() }
        };
        if icon_registered {
            log::info!("系统托盘图标已创建");
        } else {
            log::error!("创建系统托盘图标失败");
        }

        Self {
            hwnd,
            nid,
            icon_registered,
            #[cfg(test)]
            registration_error,
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
        MAIN_WINDOW_HWND.store(std::ptr::null_mut(), Ordering::Release);

        // SAFETY: 按正确顺序清理资源
        unsafe {
            if self.icon_registered {
                Shell_NotifyIconW(NIM_DELETE, &self.nid);
            }
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
                    // SAFETY: 托盘回调与全局事件发送器均在主线程中使用。
                    unsafe {
                        send_event(TrayEvent::ToggleVisible);
                    }
                }
                WM_RBUTTONUP => {
                    log::info!("[托盘] 右键点击 → 显示菜单");
                    // SAFETY: hwnd 是 Windows 传入的有效托盘消息窗口句柄。
                    unsafe {
                        show_menu(hwnd);
                    }
                }
                _ => {}
            }
            0
        }
        WM_SHOW_EXISTING_INSTANCE => {
            log::info!("[单实例] 收到重复启动请求，显示现有窗口");
            // SAFETY: 托盘消息窗口与全局事件发送器均由主线程创建并使用。
            unsafe {
                send_event(TrayEvent::ShowWindow);
            }
            0
        }
        _ => {
            // SAFETY: 未处理消息按 Win32 约定转发给默认窗口过程。
            unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
        }
    }
}

/// 通知已运行的 Tools Box 实例显示主窗口。
///
/// 首个进程可能仍在初始化，因此会在有限时间内等待托盘消息窗口创建。
pub fn notify_existing_instance() -> bool {
    let class_name = tray_window_class_name();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);

    loop {
        // SAFETY: class_name 以空字符结尾且调用期间有效，窗口标题传空表示只按类名查找。
        let hwnd = unsafe { FindWindowW(class_name.as_ptr(), std::ptr::null()) };
        if !hwnd.is_null() {
            let mut process_id = 0;
            // SAFETY: hwnd 由 FindWindowW 返回，process_id 指向有效可写内存。
            unsafe {
                GetWindowThreadProcessId(hwnd, &mut process_id);
                if process_id != 0 {
                    let allowed = AllowSetForegroundWindow(process_id);
                    if allowed == 0 {
                        log::debug!("未能授予现有进程前台激活权限，将继续发送显示请求");
                    }
                }
            }

            // SAFETY: hwnd 由 FindWindowW 返回；消息不携带指针，跨进程异步投递安全。
            let posted = unsafe { PostMessageW(hwnd, WM_SHOW_EXISTING_INSTANCE, 0, 0) };
            if posted != 0 {
                log::info!("已通知现有 Tools Box 实例显示窗口");
                return true;
            }
        }

        if std::time::Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}

fn tray_window_class_name() -> Vec<u16> {
    TRAY_WINDOW_CLASS_NAME
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect()
}

/// 通过全局 Sender 发送事件，并通过 egui Context 强制请求重绘
///
/// SAFETY: GLOBAL_TX 和 GLOBAL_EGUI_CTX 在 App 初始化时设置，在 Drop 中清除。
unsafe fn send_event(event: TrayEvent) {
    // SAFETY: 全局发送器和 egui Context 仅在主线程初始化、读取和清理。
    unsafe {
        if let Some(ref tx) = GLOBAL_TX {
            match tx.send(event) {
                Ok(()) => {
                    log::debug!("[托盘] 事件发送成功");
                    wake_main_window();

                    // 原生窗口消息是主要唤醒手段，egui 重绘请求作为可见窗口的补充。
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
}

/// 登记 eframe 主窗口句柄，供托盘回调在主窗口隐藏时唤醒事件循环。
pub fn set_main_window_handle(hwnd: HWND) {
    let previous = MAIN_WINDOW_HWND.swap(hwnd, Ordering::AcqRel);
    if previous != hwnd {
        log::info!("已登记主窗口句柄，用于托盘事件唤醒");
    }
}

/// 读取已登记的主窗口句柄，尚未登记时返回空句柄。
pub fn main_window_handle() -> HWND {
    MAIN_WINDOW_HWND.load(Ordering::Acquire)
}

/// 使用原生窗口操作唤醒隐藏或最小化的主窗口。
///
/// 托盘事件和全局热键事件均可调用此函数，不依赖 eframe 后续重绘。
/// 最小化到任务栏的窗口也在此还原：`IsWindowVisible` 对最小化窗口仍返回可见，
/// 仅判断可见性会漏掉该状态，导致热键唤出时窗口停留在任务栏。
pub fn wake_main_window() {
    let hwnd = main_window_handle();
    if hwnd.is_null() {
        log::debug!("[托盘] 主窗口句柄尚未登记，使用 egui 重绘请求唤醒");
        return;
    }

    // SAFETY: hwnd 由主窗口在运行期间登记，主窗口生命周期覆盖托盘和热键管理器。
    // ShowWindowAsync 在热键监听线程调用时不会同步等待主窗口线程。
    unsafe {
        let command = if IsIconic(hwnd) != 0 {
            SW_RESTORE
        } else if IsWindowVisible(hwnd) == 0 {
            SW_SHOW
        } else {
            return;
        };

        if ShowWindowAsync(hwnd, command) == 0 {
            log::error!("异步唤醒主窗口失败: command={command}");
        }
    }
}

/// 判断主窗口当前是否已位于桌面最前层（可见、未最小化且为前台窗口）。
///
/// 供热键监听线程在唤出窗口前采样状态：只有窗口已位于桌面最前层时，
/// 切换热键才应隐藏到托盘；隐藏、最小化到任务栏、被其他程序覆盖都应唤出。
/// 必须在 `wake_main_window()` 之前采样，否则唤醒本身会把窗口激活，采样结果失真。
pub fn is_main_window_in_front() -> bool {
    let hwnd = main_window_handle();
    if hwnd.is_null() {
        return false;
    }

    // SAFETY: hwnd 由主窗口在运行期间登记，主窗口生命周期覆盖热键管理器。
    unsafe { IsWindowVisible(hwnd) != 0 && IsIconic(hwnd) == 0 && GetForegroundWindow() == hwnd }
}

/// 将主窗口唤出到桌面最前层：还原/显示 + 置顶抬升 + 前台激活。
///
/// 供主线程（`App::update`）在热键或托盘唤出窗口后调用，保证窗口不被其他程序覆盖。
/// Windows 前台锁定会静默拒绝 `SetForegroundWindow`，因此先置顶再取消置顶：
/// 该操作不受前台锁定限制，可把窗口排到所有非置顶窗口之上。
pub fn bring_main_window_to_front() {
    let hwnd = main_window_handle();
    if hwnd.is_null() {
        log::debug!("[托盘] 主窗口句柄尚未登记，跳过窗口唤出到最前层");
        return;
    }

    // SAFETY: hwnd 由主窗口在运行期间登记，主窗口生命周期覆盖托盘和热键管理器；
    // 本函数由主线程调用，同步 ShowWindow 不会跨线程等待。
    unsafe {
        if IsIconic(hwnd) != 0 {
            ShowWindow(hwnd, SW_RESTORE);
        } else if IsWindowVisible(hwnd) == 0 {
            ShowWindow(hwnd, SW_SHOW);
        }

        let raise_flags = SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_SHOWWINDOW;
        let raised = SetWindowPos(hwnd, HWND_TOPMOST, 0, 0, 0, 0, raise_flags) != 0
            && SetWindowPos(hwnd, HWND_NOTOPMOST, 0, 0, 0, 0, raise_flags) != 0;
        let activated = SetForegroundWindow(hwnd) != 0;

        if raised && activated {
            log::debug!("主窗口已唤出到桌面最前层");
        } else if raised {
            log::warn!("主窗口抢前台失败，已通过置顶抬升保证窗口位于最前层");
        } else {
            log::warn!("主窗口置顶抬升失败，抢前台结果: {activated}");
        }
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
    // SAFETY: hwnd 和窗口属性中的菜单句柄由 TrayManager 创建并在调用期间保持有效。
    unsafe {
        let mut point: POINT = std::mem::zeroed();
        GetCursorPos(&mut point);
        SetForegroundWindow(hwnd);
        let hmenu = GetPropW(hwnd, MENU_PROP.as_ptr()) as HMENU;
        if !hmenu.is_null() {
            let command = TrackPopupMenu(
                hmenu,
                TPM_BOTTOMALIGN | TPM_LEFTALIGN | TPM_NONOTIFY | TPM_RETURNCMD,
                point.x,
                point.y,
                0,
                hwnd,
                std::ptr::null(),
            );
            PostMessageW(hwnd, WM_NULL, 0, 0);

            if let Some(action) = menu_action(command) {
                match action {
                    TrayMenuAction::ShowWindow => {
                        log::info!("[托盘菜单] 显示窗口");
                        send_event(TrayEvent::ShowWindow);
                    }
                    TrayMenuAction::Quit => {
                        log::info!("[托盘菜单] 直接清理托盘图标并退出程序");
                        quit_from_tray(hwnd);
                    }
                }
            }
        }
    }
}

/// 将右键菜单命令转换为菜单操作。
fn menu_action(command: i32) -> Option<TrayMenuAction> {
    match usize::try_from(command).ok()? {
        MENU_SHOW => Some(TrayMenuAction::ShowWindow),
        MENU_QUIT => Some(TrayMenuAction::Quit),
        _ => None,
    }
}

/// 创建具有正确大小和标识的托盘图标数据。
fn notify_icon_data(hwnd: HWND) -> NOTIFYICONDATAW {
    // SAFETY: NOTIFYICONDATAW 是 Win32 POD 结构体，零初始化后再填充必需字段。
    let mut nid: NOTIFYICONDATAW = unsafe { std::mem::zeroed() };
    nid.cbSize = u32::try_from(std::mem::size_of::<NOTIFYICONDATAW>()).unwrap_or(0);
    nid.hWnd = hwnd;
    nid.uID = 1;
    nid
}

/// 从 Windows 通知区域删除托盘图标。
///
/// # Safety
/// `hwnd` 必须是创建该托盘图标时使用的有效消息窗口句柄。
unsafe fn remove_tray_icon(hwnd: HWND) -> bool {
    let nid = notify_icon_data(hwnd);

    // SAFETY: nid 包含托盘图标注册时使用的窗口句柄、ID 和正确的结构体大小。
    unsafe { Shell_NotifyIconW(NIM_DELETE, &nid) != 0 }
}

/// 不依赖 eframe 更新循环，直接清理托盘图标并终止进程。
///
/// # Safety
/// `hwnd` 必须是 TrayManager 创建且仍然有效的托盘消息窗口句柄。
unsafe fn quit_from_tray(hwnd: HWND) -> ! {
    // SAFETY: 调用方保证 hwnd 是有效的托盘消息窗口句柄。
    let removed = unsafe { remove_tray_icon(hwnd) };
    if removed {
        log::info!("系统托盘图标已在退出前主动删除");
    } else {
        log::error!("退出前删除系统托盘图标失败");
    }

    std::process::exit(0);
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
    use std::sync::{Mutex, MutexGuard};

    static MAIN_WINDOW_TEST_LOCK: Mutex<()> = Mutex::new(());

    fn lock_main_window_tests() -> MutexGuard<'static, ()> {
        match MAIN_WINDOW_TEST_LOCK.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }

    fn wait_for_window_state(hwnd: HWND, ready: impl Fn(HWND) -> bool) -> bool {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
        loop {
            // SAFETY: 在创建测试窗口的线程中处理该窗口的消息，MSG 已正确初始化。
            unsafe {
                let mut message: MSG = std::mem::zeroed();
                while PeekMessageW(&mut message, hwnd, 0, 0, PM_REMOVE) != 0 {
                    TranslateMessage(&message);
                    DispatchMessageW(&message);
                }

                if ready(hwnd) {
                    return true;
                }
            }

            if std::time::Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }

    fn wait_for_window_visible(hwnd: HWND) -> bool {
        // SAFETY: 仅查询窗口状态，不解引用任何指针。
        wait_for_window_state(hwnd, |hwnd| unsafe { IsWindowVisible(hwnd) != 0 })
    }

    fn wait_for_window_restored(hwnd: HWND) -> bool {
        // SAFETY: 仅查询窗口状态，不解引用任何指针。
        wait_for_window_state(hwnd, |hwnd| unsafe { IsIconic(hwnd) == 0 })
    }

    struct TestMainWindow(HWND);

    impl TestMainWindow {
        fn new() -> Self {
            let class_name: Vec<u16> = "STATIC\0".encode_utf16().collect();
            let window_name: Vec<u16> = "Tools Box 托盘唤醒测试\0".encode_utf16().collect();

            // SAFETY: 使用系统内置 STATIC 窗口类，字符串均以空字符结尾且在调用期间有效。
            let hwnd = unsafe {
                CreateWindowExW(
                    0,
                    class_name.as_ptr(),
                    window_name.as_ptr(),
                    WS_OVERLAPPED,
                    0,
                    0,
                    100,
                    100,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null(),
                )
            };
            assert!(!hwnd.is_null(), "测试窗口创建失败");
            Self(hwnd)
        }
    }

    impl Drop for TestMainWindow {
        fn drop(&mut self) {
            let _ = MAIN_WINDOW_HWND.compare_exchange(
                self.0,
                std::ptr::null_mut(),
                Ordering::AcqRel,
                Ordering::Acquire,
            );

            // SAFETY: 句柄由 CreateWindowExW 创建，且仅在此处销毁一次。
            unsafe {
                DestroyWindow(self.0);
            }
        }
    }

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

    #[test]
    fn test_menu_action_mapping() {
        assert_eq!(menu_action(1), Some(TrayMenuAction::ShowWindow));
        assert_eq!(menu_action(2), Some(TrayMenuAction::Quit));
        assert_eq!(menu_action(0), None);
        assert_eq!(menu_action(-1), None);
    }

    #[test]
    fn test_existing_instance_message_requests_show_window() {
        let _guard = lock_main_window_tests();
        let manager = TrayManager::new();

        // SAFETY: manager.hwnd 是 TrayManager 创建并在测试期间保持有效的消息窗口。
        let result = unsafe {
            tray_wnd_proc(
                manager.hwnd,
                WM_SHOW_EXISTING_INSTANCE,
                WPARAM::default(),
                LPARAM::default(),
            )
        };

        assert_eq!(result, 0);
        assert_eq!(manager.poll_events(), vec![TrayEvent::ShowWindow]);
    }

    #[test]
    fn test_notify_existing_instance_finds_tray_window_and_posts_show_request() {
        let _guard = lock_main_window_tests();
        let manager = TrayManager::new();

        assert!(
            notify_existing_instance(),
            "应能通过窗口类名找到已运行实例的托盘消息窗口"
        );

        // SAFETY: 在创建托盘消息窗口的线程中处理该窗口收到的异步消息。
        unsafe {
            let mut message: MSG = std::mem::zeroed();
            while PeekMessageW(&mut message, manager.hwnd, 0, 0, PM_REMOVE) != 0 {
                TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }

        assert_eq!(manager.poll_events(), vec![TrayEvent::ShowWindow]);
    }

    #[test]
    fn test_show_event_wakes_hidden_main_window() {
        let _guard = lock_main_window_tests();
        let window = TestMainWindow::new();

        // SAFETY: 句柄在测试期间有效，并由 TestMainWindow 保持存活。
        unsafe {
            ShowWindow(window.0, SW_SHOW);
        }
        set_main_window_handle(window.0);

        // SAFETY: 句柄在测试期间有效，并由 TestMainWindow 保持存活。
        unsafe {
            ShowWindow(window.0, SW_HIDE);
            assert_eq!(IsWindowVisible(window.0), 0, "测试窗口应处于隐藏状态");
        }

        wake_main_window();
        assert!(
            wait_for_window_visible(window.0),
            "托盘事件应原生唤醒隐藏窗口"
        );
    }

    #[test]
    fn test_background_thread_wakes_hidden_main_window() {
        let _guard = lock_main_window_tests();
        let window = TestMainWindow::new();

        // SAFETY: 句柄在测试期间有效，并由 TestMainWindow 保持存活。
        unsafe {
            ShowWindow(window.0, SW_SHOW);
            ShowWindow(window.0, SW_HIDE);
        }
        set_main_window_handle(window.0);

        let wake_thread = std::thread::spawn(wake_main_window);
        assert!(wake_thread.join().is_ok(), "后台唤醒线程不应发生 panic");
        assert!(
            wait_for_window_visible(window.0),
            "后台线程应在超时前恢复隐藏窗口"
        );
    }

    #[test]
    fn test_wake_restores_minimized_main_window() {
        let _guard = lock_main_window_tests();
        let window = TestMainWindow::new();

        // SAFETY: 句柄在测试期间有效，并由 TestMainWindow 保持存活。
        unsafe {
            ShowWindow(window.0, SW_SHOW);
            ShowWindow(window.0, SW_MINIMIZE);
            assert_ne!(IsIconic(window.0), 0, "测试窗口应处于最小化状态");
        }
        set_main_window_handle(window.0);

        wake_main_window();
        assert!(
            wait_for_window_restored(window.0),
            "唤醒应还原最小化到任务栏的主窗口"
        );
    }

    #[test]
    fn test_bring_main_window_to_front_restores_and_keeps_window_unpinned() {
        let _guard = lock_main_window_tests();
        let window = TestMainWindow::new();

        // SAFETY: 句柄在测试期间有效，并由 TestMainWindow 保持存活。
        unsafe {
            ShowWindow(window.0, SW_SHOW);
            ShowWindow(window.0, SW_MINIMIZE);
        }
        set_main_window_handle(window.0);

        bring_main_window_to_front();

        // SAFETY: 仅查询窗口状态与扩展样式，不解引用任何指针。
        let on_screen = wait_for_window_state(window.0, |hwnd| unsafe {
            IsWindowVisible(hwnd) != 0 && IsIconic(hwnd) == 0
        });
        assert!(on_screen, "唤出到最前层时应还原并显示最小化窗口");

        // SAFETY: 仅查询窗口扩展样式。
        let ex_style = unsafe { GetWindowLongW(window.0, GWL_EXSTYLE) };
        assert_eq!(
            ex_style & WS_EX_TOPMOST as i32,
            0,
            "唤出瞬间抬升后不应保留永久置顶属性"
        );
    }

    #[test]
    fn test_notify_icon_data_contains_required_fields() {
        let _guard = lock_main_window_tests();
        let window = TestMainWindow::new();
        let nid = notify_icon_data(window.0);

        assert_eq!(
            nid.cbSize,
            u32::try_from(std::mem::size_of::<NOTIFYICONDATAW>()).unwrap_or(0),
            "托盘数据必须设置正确的结构体大小"
        );
        assert_eq!(nid.hWnd, window.0);
        assert_eq!(nid.uID, 1);
    }

    #[test]
    #[ignore = "需要可访问 Explorer 通知区域的交互式 Windows 会话"]
    fn test_registered_tray_icon_can_be_removed_immediately() {
        let _guard = lock_main_window_tests();
        let manager = TrayManager::new();
        assert!(!manager.hwnd.is_null(), "托盘消息窗口创建失败");
        assert!(
            manager.icon_registered,
            "NIM_ADD 未能注册托盘图标，无法验证删除行为，GetLastError={}",
            manager.registration_error
        );

        // SAFETY: manager.hwnd 是注册托盘图标时使用且仍然有效的消息窗口句柄。
        let removed = unsafe { remove_tray_icon(manager.hwnd) };
        assert!(removed, "NIM_DELETE 应立即成功删除已注册的托盘图标");
    }
}
