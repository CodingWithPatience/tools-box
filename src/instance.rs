//! Windows 单实例进程控制。
//!
//! 通过命名互斥体确保同一 Windows 会话中只运行一个 Tools Box 实例。

use std::io;
use windows_sys::Win32::Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE};
use windows_sys::Win32::System::Threading::CreateMutexW;

/// Tools Box 单实例互斥体名称。
const INSTANCE_MUTEX_NAME: &str =
    "Local\\ToolsBox.SingleInstance.8F938DAF-FF51-44A6-A86E-5D68DBA0863C";

/// 单实例检测结果。
pub enum SingleInstanceState {
    /// 当前进程是首个实例，守卫必须在程序运行期间保持存活。
    Primary(SingleInstanceGuard),
    /// 已有 Tools Box 实例正在运行。
    Existing,
}

/// 首个进程持有的命名互斥体守卫。
pub struct SingleInstanceGuard {
    handle: HANDLE,
}

impl Drop for SingleInstanceGuard {
    fn drop(&mut self) {
        // SAFETY: handle 由 CreateMutexW 成功创建，并且仅在此处关闭一次。
        let closed = unsafe { CloseHandle(self.handle) };
        if closed == 0 {
            log::error!("关闭单实例互斥体失败");
        }
    }
}

/// 获取单实例状态，并在通知已有实例失败后重新尝试成为主实例。
///
/// 已有实例可能在检测完成后、接收通知前退出。此时再次获取命名互斥体，
/// 避免新启动的进程也直接退出，导致系统中没有正在运行的实例。
pub fn acquire_single_instance_or_notify<F>(notify_existing: F) -> io::Result<SingleInstanceState>
where
    F: FnOnce() -> bool,
{
    acquire_named_instance_or_notify(INSTANCE_MUTEX_NAME, notify_existing)
}

fn acquire_named_instance_or_notify<F>(
    name: &str,
    notify_existing: F,
) -> io::Result<SingleInstanceState>
where
    F: FnOnce() -> bool,
{
    match acquire_named_instance(name)? {
        SingleInstanceState::Existing if !notify_existing() => acquire_named_instance(name),
        state => Ok(state),
    }
}

fn acquire_named_instance(name: &str) -> io::Result<SingleInstanceState> {
    let wide_name: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();

    // SAFETY: 使用默认安全属性，不请求初始所有权；名称以空字符结尾且调用期间有效。
    let handle = unsafe { CreateMutexW(std::ptr::null(), 0, wide_name.as_ptr()) };
    if handle.is_null() {
        return Err(io::Error::last_os_error());
    }

    // SAFETY: 紧接 CreateMutexW 调用读取线程错误码，用于判断命名对象是否已存在。
    let already_exists = unsafe { GetLastError() } == ERROR_ALREADY_EXISTS;
    if already_exists {
        // SAFETY: handle 是 CreateMutexW 返回的有效句柄，当前分支不需要继续持有。
        let closed = unsafe { CloseHandle(handle) };
        if closed == 0 {
            log::error!("关闭重复实例互斥体句柄失败");
        }
        return Ok(SingleInstanceState::Existing);
    }

    Ok(SingleInstanceState::Primary(SingleInstanceGuard { handle }))
}

#[cfg(test)]
mod tests {
    use super::{SingleInstanceState, acquire_named_instance, acquire_named_instance_or_notify};

    #[test]
    fn named_mutex_detects_existing_instance_and_releases_on_drop() {
        let name = format!(
            "Local\\ToolsBox.SingleInstance.Test.{}",
            uuid::Uuid::new_v4()
        );

        let first_guard = match acquire_named_instance(&name) {
            Ok(SingleInstanceState::Primary(guard)) => guard,
            Ok(SingleInstanceState::Existing) => {
                panic!("唯一测试互斥体不应已存在");
            }
            Err(error) => {
                panic!("创建首个测试互斥体失败: {error}");
            }
        };

        match acquire_named_instance(&name) {
            Ok(SingleInstanceState::Existing) => {}
            Ok(SingleInstanceState::Primary(_guard)) => {
                panic!("第二次获取同名互斥体时应检测到已有实例");
            }
            Err(error) => {
                panic!("检测已有测试互斥体失败: {error}");
            }
        }

        drop(first_guard);

        match acquire_named_instance(&name) {
            Ok(SingleInstanceState::Primary(_guard)) => {}
            Ok(SingleInstanceState::Existing) => {
                panic!("首个守卫释放后应允许重新成为主实例");
            }
            Err(error) => {
                panic!("重新创建测试互斥体失败: {error}");
            }
        }
    }

    #[test]
    fn notification_failure_retries_mutex_after_existing_instance_exits() {
        let name = format!(
            "Local\\ToolsBox.SingleInstance.RetryTest.{}",
            uuid::Uuid::new_v4()
        );
        let first_guard = match acquire_named_instance(&name) {
            Ok(SingleInstanceState::Primary(guard)) => guard,
            Ok(SingleInstanceState::Existing) => {
                panic!("唯一测试互斥体不应已存在");
            }
            Err(error) => {
                panic!("创建第一个测试互斥体失败: {error}");
            }
        };
        let mut first_guard = Some(first_guard);

        let state = acquire_named_instance_or_notify(&name, || {
            drop(first_guard.take());
            false
        });

        match state {
            Ok(SingleInstanceState::Primary(_guard)) => {}
            Ok(SingleInstanceState::Existing) => {
                panic!("已有实例退出后，当前进程应重新成为主实例");
            }
            Err(error) => {
                panic!("通知失败后重新获取测试互斥体失败: {error}");
            }
        }
    }
}
