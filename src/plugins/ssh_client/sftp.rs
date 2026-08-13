use std::collections::{HashSet, VecDeque};
use std::io::{Read, Write};
use std::net::{SocketAddr, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use ssh2::{Session, Sftp};

use super::crypto;
use super::models::{AuthMethod, FileEntry, SftpControl, SftpRequest, SftpResponse, TransferTask};

/// SFTP 传输缓冲区大小（128 KB）
const TRANSFER_BUFFER_SIZE: usize = 128 * 1024;
/// 传输进度最小上报间隔
const PROGRESS_INTERVAL: Duration = Duration::from_millis(100);
/// SFTP 连接与读写超时
const SFTP_TIMEOUT: Duration = Duration::from_secs(30);

/// SFTP 客户端管理器
pub struct SftpClient;

impl SftpClient {
    /// 连接 SSH 并启动 SFTP 服务，返回 (请求发送端, 响应接收端)
    ///
    /// 使用独立的 SSH 连接（与终端连接分开），避免线程安全问题。
    pub fn connect(
        host: &str,
        port: u16,
        username: &str,
        auth: &AuthMethod,
    ) -> Result<(
        mpsc::SyncSender<SftpRequest>,
        mpsc::Sender<SftpControl>,
        mpsc::Receiver<SftpResponse>,
    )> {
        let addr = format!("{}:{}", host, port);
        let uname = username.to_string();
        let auth_clone = auth.clone();

        let (req_tx, req_rx) = mpsc::sync_channel::<SftpRequest>(64);
        let (control_tx, control_rx) = mpsc::channel::<SftpControl>();
        let (resp_tx, resp_rx) = mpsc::channel::<SftpResponse>();

        thread::Builder::new()
            .name("sftp-io".to_string())
            .spawn(move || {
                if let Err(e) =
                    Self::session_loop(&addr, &uname, &auth_clone, &req_rx, &control_rx, &resp_tx)
                {
                    let _ = resp_tx.send(SftpResponse::Error(format!("SFTP 连接错误: {}", e)));
                    log::error!("SFTP 会话线程异常退出: {}", e);
                }
            })
            .context("创建 SFTP I/O 线程失败")?;

        Ok((req_tx, control_tx, resp_rx))
    }

    /// SFTP 会话主循环
    fn session_loop(
        addr: &str,
        username: &str,
        auth: &AuthMethod,
        req_rx: &mpsc::Receiver<SftpRequest>,
        control_rx: &mpsc::Receiver<SftpControl>,
        resp_tx: &mpsc::Sender<SftpResponse>,
    ) -> Result<()> {
        // 建立独立的 SSH 连接
        let socket_addr = resolve_socket_addr(addr)?;
        let tcp = std::net::TcpStream::connect_timeout(&socket_addr, SFTP_TIMEOUT)
            .with_context(|| format!("SFTP: 无法连接到 {}", addr))?;
        tcp.set_read_timeout(Some(SFTP_TIMEOUT))
            .context("SFTP: 设置 TCP 超时失败")?;
        tcp.set_write_timeout(Some(SFTP_TIMEOUT))
            .context("SFTP: 设置 TCP 写超时失败")?;

        let mut session = Session::new().context("SFTP: 创建 SSH 会话失败")?;
        session.set_tcp_stream(tcp);
        session.handshake().context("SFTP: SSH 握手失败")?;

        // 认证（复用与终端相同的认证逻辑）
        match auth {
            AuthMethod::Password {
                encrypted_password,
                iv,
                salt,
            } => {
                let password = crypto::decrypt_password(encrypted_password, iv, salt)
                    .context("SFTP: 解密密码失败")?;
                session
                    .userauth_password(username, &password)
                    .with_context(|| format!("SFTP: 密码认证失败 (用户: {})", username))?;
            }
            AuthMethod::KeyFile {
                private_key_path,
                encrypted_passphrase,
            } => {
                let passphrase = if let Some((ct, iv, salt)) = encrypted_passphrase {
                    Some(
                        crypto::decrypt_password(ct, iv, salt)
                            .context("SFTP: 解密密钥密码短语失败")?,
                    )
                } else {
                    None
                };
                session
                    .userauth_pubkey_file(
                        username,
                        None,
                        Path::new(private_key_path),
                        passphrase.as_deref(),
                    )
                    .with_context(|| {
                        format!(
                            "SFTP: 密钥认证失败 (用户: {}, 密钥: {})",
                            username, private_key_path
                        )
                    })?;
            }
        }

        if !session.authenticated() {
            anyhow::bail!("SFTP: SSH 认证未通过");
        }

        let sftp = session.sftp().context("SFTP: 初始化 SFTP 子系统失败")?;
        log::info!("SFTP 连接已建立: {}@{}", username, addr);
        let _ = resp_tx.send(SftpResponse::Connected);

        // 请求处理循环。传输期间收到的非取消请求会暂存并保持原顺序。
        let mut pending_requests = VecDeque::new();
        let mut cancelled_task_ids = HashSet::new();
        loop {
            if process_idle_control(
                control_rx,
                &mut pending_requests,
                &mut cancelled_task_ids,
                resp_tx,
            ) {
                break;
            }
            let request = match pending_requests.pop_front() {
                Some(request) => Ok(request),
                None => match req_rx.recv_timeout(Duration::from_millis(100)) {
                    Ok(request) => Ok(request),
                    Err(mpsc::RecvTimeoutError::Timeout) => continue,
                    Err(mpsc::RecvTimeoutError::Disconnected) => Err(mpsc::RecvError),
                },
            };
            match request {
                Ok(SftpRequest::ListDirectory(path)) => {
                    Self::handle_list_dir(&sftp, &path, resp_tx);
                }
                Ok(SftpRequest::Upload(task)) => {
                    if cancelled_task_ids.remove(&task.id) {
                        finish_cancelled_task(task, resp_tx);
                        continue;
                    }
                    if Self::handle_upload(
                        &sftp,
                        task,
                        req_rx,
                        control_rx,
                        &mut pending_requests,
                        &mut cancelled_task_ids,
                        resp_tx,
                    ) {
                        break;
                    }
                }
                Ok(SftpRequest::Download(task)) => {
                    if cancelled_task_ids.remove(&task.id) {
                        finish_cancelled_task(task, resp_tx);
                        continue;
                    }
                    if Self::handle_download(
                        &sftp,
                        task,
                        req_rx,
                        control_rx,
                        &mut pending_requests,
                        &mut cancelled_task_ids,
                        resp_tx,
                    ) {
                        break;
                    }
                }
                Err(mpsc::RecvError) => {
                    log::info!("SFTP: 请求通道已关闭");
                    break;
                }
            }
        }

        let _ = resp_tx.send(SftpResponse::Disconnected);
        log::info!("SFTP 连接已断开");
        Ok(())
    }

    /// 列出远程目录
    fn handle_list_dir(sftp: &Sftp, path: &str, resp_tx: &mpsc::Sender<SftpResponse>) {
        match sftp.readdir(Path::new(path)) {
            Ok(entries) => {
                let mut file_list: Vec<FileEntry> = entries
                    .iter()
                    .filter_map(|(p, stat)| {
                        let name = p.file_name()?.to_string_lossy().to_string();
                        // 跳过隐藏文件中的 . 和 ..
                        if name == "." || name == ".." {
                            return None;
                        }
                        Some(FileEntry {
                            name,
                            path: p.to_string_lossy().to_string(),
                            is_dir: stat.is_dir(),
                            size: stat.size.unwrap_or(0),
                            size_known: stat.size.is_some(),
                            // SAFETY: mtime 是 Unix 时间戳，u64→i64 仅在 2262 年后溢出
                            modified: stat.mtime.map(|t| t.min(i64::MAX as u64) as i64),
                        })
                    })
                    .collect();

                // 目录在前，文件在后，同类型按名称排序
                file_list.sort_by(|a, b| match (a.is_dir, b.is_dir) {
                    (true, false) => std::cmp::Ordering::Less,
                    (false, true) => std::cmp::Ordering::Greater,
                    _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
                });

                let _ = resp_tx.send(SftpResponse::DirectoryList(path.to_string(), file_list));
            }
            Err(e) => {
                let _ = resp_tx.send(SftpResponse::DirectoryError(
                    path.to_string(),
                    format!("无法列出目录 '{}': {}", path, e),
                ));
            }
        }
    }

    /// 上传文件
    fn handle_upload(
        sftp: &Sftp,
        mut task: TransferTask,
        req_rx: &mpsc::Receiver<SftpRequest>,
        control_rx: &mpsc::Receiver<SftpControl>,
        pending_requests: &mut VecDeque<SftpRequest>,
        cancelled_task_ids: &mut HashSet<String>,
        resp_tx: &mpsc::Sender<SftpResponse>,
    ) -> bool {
        let local_path = task.source.clone();
        let remote_path = task.destination.clone();
        let temporary_path = remote_temporary_path(&remote_path, &task.id);
        let mut local_file = match std::fs::File::open(&local_path) {
            Ok(f) => f,
            Err(e) => {
                finish_task_with_error(&mut task, format!("无法打开本地文件: {}", e), resp_tx);
                return false;
            }
        };

        task.total_size = match local_file.metadata() {
            Ok(metadata) => Some(metadata.len()),
            Err(e) => {
                finish_task_with_error(&mut task, format!("无法读取文件大小: {}", e), resp_tx);
                return false;
            }
        };

        let mut remote_file = match sftp.create(Path::new(&temporary_path)) {
            Ok(f) => f,
            Err(e) => {
                finish_task_with_error(&mut task, format!("无法创建远程临时文件: {}", e), resp_tx);
                return false;
            }
        };

        send_progress(resp_tx, &task);
        let mut buf = vec![0u8; TRANSFER_BUFFER_SIZE];
        let mut last_progress = Instant::now();
        loop {
            let control = poll_transfer_control(
                &task.id,
                req_rx,
                control_rx,
                pending_requests,
                cancelled_task_ids,
                resp_tx,
            );
            if control.cancelled || control.disconnect {
                drop(remote_file);
                let _ = sftp.unlink(Path::new(&temporary_path));
                finish_task_with_error(&mut task, "传输已取消".to_string(), resp_tx);
                return control.disconnect;
            }
            match local_file.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    if let Err(e) = remote_file.write_all(&buf[..n]) {
                        drop(remote_file);
                        let _ = sftp.unlink(Path::new(&temporary_path));
                        finish_task_with_error(
                            &mut task,
                            format!("写入远程文件失败: {}", e),
                            resp_tx,
                        );
                        return false;
                    }
                    task.transferred = task
                        .transferred
                        .saturating_add(u64::try_from(n).unwrap_or(0));
                    report_progress_if_due(resp_tx, &task, &mut last_progress);
                }
                Err(e) => {
                    drop(remote_file);
                    let _ = sftp.unlink(Path::new(&temporary_path));
                    finish_task_with_error(&mut task, format!("读取本地文件失败: {}", e), resp_tx);
                    return false;
                }
            }
        }

        if let Err(error) = validate_transfer_size(task.total_size, task.transferred) {
            drop(remote_file);
            let cleanup_error = cleanup_remote_temporary_file(sftp, &temporary_path);
            finish_task_with_error(
                &mut task,
                combine_transfer_and_cleanup_error(error, cleanup_error),
                resp_tx,
            );
            return false;
        }
        let control = poll_transfer_control(
            &task.id,
            req_rx,
            control_rx,
            pending_requests,
            cancelled_task_ids,
            resp_tx,
        );
        if control.cancelled || control.disconnect {
            drop(remote_file);
            let cleanup_error = cleanup_remote_temporary_file(sftp, &temporary_path);
            finish_task_with_error(
                &mut task,
                combine_transfer_and_cleanup_error("传输已取消".to_string(), cleanup_error),
                resp_tx,
            );
            return control.disconnect;
        }
        if let Err(e) = remote_file.flush() {
            drop(remote_file);
            let cleanup_error = cleanup_remote_temporary_file(sftp, &temporary_path);
            finish_task_with_error(
                &mut task,
                combine_transfer_and_cleanup_error(
                    format!("刷新远程文件失败: {}", e),
                    cleanup_error,
                ),
                resp_tx,
            );
            return false;
        }
        drop(remote_file);
        let control = poll_transfer_control(
            &task.id,
            req_rx,
            control_rx,
            pending_requests,
            cancelled_task_ids,
            resp_tx,
        );
        if control.cancelled || control.disconnect {
            let cleanup_error = cleanup_remote_temporary_file(sftp, &temporary_path);
            finish_task_with_error(
                &mut task,
                combine_transfer_and_cleanup_error("传输已取消".to_string(), cleanup_error),
                resp_tx,
            );
            return control.disconnect;
        }
        if let Err(e) = sftp.rename(
            Path::new(&temporary_path),
            Path::new(&remote_path),
            Some(ssh2::RenameFlags::ATOMIC | ssh2::RenameFlags::OVERWRITE),
        ) {
            let _ = sftp.unlink(Path::new(&temporary_path));
            finish_task_with_error(&mut task, format!("提交远程文件失败: {}", e), resp_tx);
            return false;
        }

        task.done = true;
        send_final_response(resp_tx, SftpResponse::OperationDone(task));
        log::info!("SFTP 上传完成: {} → {}", local_path, remote_path);
        false
    }

    /// 下载文件
    fn handle_download(
        sftp: &Sftp,
        mut task: TransferTask,
        req_rx: &mpsc::Receiver<SftpRequest>,
        control_rx: &mpsc::Receiver<SftpControl>,
        pending_requests: &mut VecDeque<SftpRequest>,
        cancelled_task_ids: &mut HashSet<String>,
        resp_tx: &mpsc::Sender<SftpResponse>,
    ) -> bool {
        let remote_path = task.source.clone();
        let local_path = task.destination.clone();
        let temporary_path = local_temporary_path(Path::new(&local_path), &task.id);
        let mut remote_file = match sftp.open(Path::new(&remote_path)) {
            Ok(f) => f,
            Err(e) => {
                finish_task_with_error(&mut task, format!("无法打开远程文件: {}", e), resp_tx);
                return false;
            }
        };

        if task.total_size.is_none() {
            task.total_size = remote_file.stat().ok().and_then(|stat| stat.size);
        }

        let mut local_file = match std::fs::File::create(&temporary_path) {
            Ok(f) => f,
            Err(e) => {
                finish_task_with_error(&mut task, format!("无法创建本地临时文件: {}", e), resp_tx);
                return false;
            }
        };

        send_progress(resp_tx, &task);
        let mut buf = vec![0u8; TRANSFER_BUFFER_SIZE];
        let mut last_progress = Instant::now();
        loop {
            let control = poll_transfer_control(
                &task.id,
                req_rx,
                control_rx,
                pending_requests,
                cancelled_task_ids,
                resp_tx,
            );
            if control.cancelled || control.disconnect {
                drop(local_file);
                let _ = std::fs::remove_file(&temporary_path);
                finish_task_with_error(&mut task, "传输已取消".to_string(), resp_tx);
                return control.disconnect;
            }
            match remote_file.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    if let Err(e) = local_file.write_all(&buf[..n]) {
                        drop(local_file);
                        let _ = std::fs::remove_file(&temporary_path);
                        finish_task_with_error(
                            &mut task,
                            format!("写入本地文件失败: {}", e),
                            resp_tx,
                        );
                        return false;
                    }
                    task.transferred = task
                        .transferred
                        .saturating_add(u64::try_from(n).unwrap_or(0));
                    report_progress_if_due(resp_tx, &task, &mut last_progress);
                }
                Err(e) => {
                    drop(local_file);
                    let _ = std::fs::remove_file(&temporary_path);
                    finish_task_with_error(&mut task, format!("读取远程文件失败: {}", e), resp_tx);
                    return false;
                }
            }
        }

        if let Err(error) = validate_transfer_size(task.total_size, task.transferred) {
            drop(local_file);
            let cleanup_error = cleanup_local_temporary_file(&temporary_path);
            finish_task_with_error(
                &mut task,
                combine_transfer_and_cleanup_error(error, cleanup_error),
                resp_tx,
            );
            return false;
        }
        let control = poll_transfer_control(
            &task.id,
            req_rx,
            control_rx,
            pending_requests,
            cancelled_task_ids,
            resp_tx,
        );
        if control.cancelled || control.disconnect {
            drop(local_file);
            let cleanup_error = cleanup_local_temporary_file(&temporary_path);
            finish_task_with_error(
                &mut task,
                combine_transfer_and_cleanup_error("传输已取消".to_string(), cleanup_error),
                resp_tx,
            );
            return control.disconnect;
        }
        if let Err(e) = local_file.sync_all() {
            drop(local_file);
            let cleanup_error = cleanup_local_temporary_file(&temporary_path);
            finish_task_with_error(
                &mut task,
                combine_transfer_and_cleanup_error(
                    format!("刷新本地文件失败: {}", e),
                    cleanup_error,
                ),
                resp_tx,
            );
            return false;
        }
        drop(local_file);
        let control = poll_transfer_control(
            &task.id,
            req_rx,
            control_rx,
            pending_requests,
            cancelled_task_ids,
            resp_tx,
        );
        if control.cancelled || control.disconnect {
            let cleanup_error = cleanup_local_temporary_file(&temporary_path);
            finish_task_with_error(
                &mut task,
                combine_transfer_and_cleanup_error("传输已取消".to_string(), cleanup_error),
                resp_tx,
            );
            return control.disconnect;
        }
        if let Err(e) = replace_local_file(&temporary_path, Path::new(&local_path), &task.id) {
            let cleanup_error = cleanup_local_temporary_file(&temporary_path);
            finish_task_with_error(
                &mut task,
                combine_transfer_and_cleanup_error(
                    format!("提交本地文件失败: {}", e),
                    cleanup_error,
                ),
                resp_tx,
            );
            return false;
        }

        task.done = true;
        send_final_response(resp_tx, SftpResponse::OperationDone(task));
        log::info!("SFTP 下载完成: {} → {}", remote_path, local_path);
        false
    }
}

struct TransferControl {
    cancelled: bool,
    disconnect: bool,
}

fn poll_transfer_control(
    task_id: &str,
    req_rx: &mpsc::Receiver<SftpRequest>,
    control_rx: &mpsc::Receiver<SftpControl>,
    pending_requests: &mut VecDeque<SftpRequest>,
    cancelled_task_ids: &mut HashSet<String>,
    resp_tx: &mpsc::Sender<SftpResponse>,
) -> TransferControl {
    let mut control = TransferControl {
        cancelled: false,
        disconnect: false,
    };
    loop {
        match control_rx.try_recv() {
            Ok(SftpControl::CancelTransfer(id)) if id == task_id => control.cancelled = true,
            Ok(SftpControl::CancelTransfer(id)) => {
                if !cancel_pending_transfer(pending_requests, &id, resp_tx) {
                    cancelled_task_ids.insert(id);
                }
            }
            Ok(SftpControl::Disconnect) => control.disconnect = true,
            Err(mpsc::TryRecvError::Empty) => break,
            Err(mpsc::TryRecvError::Disconnected) => {
                control.disconnect = true;
                break;
            }
        }
    }
    loop {
        match req_rx.try_recv() {
            Ok(request) => pending_requests.push_back(request),
            Err(mpsc::TryRecvError::Empty) => break,
            Err(mpsc::TryRecvError::Disconnected) => {
                control.disconnect = true;
                break;
            }
        }
    }
    control
}

fn process_idle_control(
    control_rx: &mpsc::Receiver<SftpControl>,
    pending_requests: &mut VecDeque<SftpRequest>,
    cancelled_task_ids: &mut HashSet<String>,
    resp_tx: &mpsc::Sender<SftpResponse>,
) -> bool {
    loop {
        match control_rx.try_recv() {
            Ok(SftpControl::CancelTransfer(id)) => {
                if !cancel_pending_transfer(pending_requests, &id, resp_tx) {
                    cancelled_task_ids.insert(id);
                }
            }
            Ok(SftpControl::Disconnect) | Err(mpsc::TryRecvError::Disconnected) => return true,
            Err(mpsc::TryRecvError::Empty) => return false,
        }
    }
}

fn cancel_pending_transfer(
    pending_requests: &mut VecDeque<SftpRequest>,
    task_id: &str,
    resp_tx: &mpsc::Sender<SftpResponse>,
) -> bool {
    if let Some(index) = pending_requests.iter().position(|request| {
        matches!(request, SftpRequest::Upload(task) | SftpRequest::Download(task) if task.id == task_id)
    }) {
        if let Some(request) = pending_requests.remove(index) {
            let mut task = match request {
                SftpRequest::Upload(task) | SftpRequest::Download(task) => task,
                _ => return false,
            };
            finish_task_with_error(&mut task, "传输已取消".to_string(), resp_tx);
            return true;
        }
    }
    false
}

fn finish_cancelled_task(mut task: TransferTask, resp_tx: &mpsc::Sender<SftpResponse>) {
    finish_task_with_error(&mut task, "传输已取消".to_string(), resp_tx);
}

fn report_progress_if_due(
    resp_tx: &mpsc::Sender<SftpResponse>,
    task: &TransferTask,
    last_progress: &mut Instant,
) {
    if last_progress.elapsed() >= PROGRESS_INTERVAL {
        send_progress(resp_tx, task);
        *last_progress = Instant::now();
    }
}

fn send_progress(resp_tx: &mpsc::Sender<SftpResponse>, task: &TransferTask) {
    let _ = resp_tx.send(SftpResponse::TransferProgress(task.clone()));
}

fn send_final_response(resp_tx: &mpsc::Sender<SftpResponse>, response: SftpResponse) {
    let _ = resp_tx.send(response);
}

fn finish_task_with_error(
    task: &mut TransferTask,
    error: String,
    resp_tx: &mpsc::Sender<SftpResponse>,
) {
    task.done = true;
    task.error = Some(error);
    send_final_response(resp_tx, SftpResponse::TransferProgress(task.clone()));
}

fn validate_transfer_size(expected: Option<u64>, actual: u64) -> Result<(), String> {
    if expected.is_some_and(|expected| expected != actual) {
        return Err(format!(
            "传输大小不一致，期望 {} 字节，实际 {} 字节",
            expected.unwrap_or_default(),
            actual
        ));
    }
    Ok(())
}

fn cleanup_remote_temporary_file(sftp: &Sftp, temporary_path: &str) -> Option<String> {
    sftp.unlink(Path::new(temporary_path)).err().map(|error| {
        let message = format!("清理远程临时文件失败: {}", error);
        log::warn!("{} ({})", message, temporary_path);
        message
    })
}

fn cleanup_local_temporary_file(temporary_path: &Path) -> Option<String> {
    std::fs::remove_file(temporary_path).err().map(|error| {
        let message = format!("清理本地临时文件失败: {}", error);
        log::warn!("{} ({})", message, temporary_path.display());
        message
    })
}

fn combine_transfer_and_cleanup_error(error: String, cleanup_error: Option<String>) -> String {
    match cleanup_error {
        Some(cleanup_error) => format!("{}；{}", error, cleanup_error),
        None => error,
    }
}

fn remote_temporary_path(remote_path: &str, task_id: &str) -> String {
    format!("{}.tools-box-part-{}", remote_path, task_id)
}

fn local_temporary_path(local_path: &Path, task_id: &str) -> PathBuf {
    let filename = local_path
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| "download".to_string());
    local_path.with_file_name(format!(".{}.tools-box-part-{}", filename, task_id))
}

fn replace_local_file(temporary_path: &Path, destination: &Path, task_id: &str) -> Result<()> {
    if !destination.exists() {
        std::fs::rename(temporary_path, destination).context("重命名本地临时文件失败")?;
        return Ok(());
    }

    replace_existing_local_file(temporary_path, destination, task_id)
}

#[cfg(windows)]
fn replace_existing_local_file(
    temporary_path: &Path,
    destination: &Path,
    _task_id: &str,
) -> Result<()> {
    use std::os::windows::ffi::OsStrExt;

    use windows_sys::Win32::Storage::FileSystem::{
        MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
    };

    let source: Vec<u16> = temporary_path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let target: Vec<u16> = destination
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    // SAFETY: 两个路径都编码为以 NUL 结尾且在调用期间有效的 UTF-16 缓冲区。
    let result = unsafe {
        MoveFileExW(
            source.as_ptr(),
            target.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if result == 0 {
        return Err(std::io::Error::last_os_error()).context("原子替换本地文件失败");
    }
    Ok(())
}

#[cfg(not(windows))]
fn replace_existing_local_file(
    temporary_path: &Path,
    destination: &Path,
    _task_id: &str,
) -> Result<()> {
    std::fs::rename(temporary_path, destination).context("原子替换本地文件失败")
}

fn resolve_socket_addr(addr: &str) -> Result<SocketAddr> {
    addr.to_socket_addrs()
        .with_context(|| format!("SFTP: 无法解析地址 {}", addr))?
        .next()
        .with_context(|| format!("SFTP: 地址没有可用端点 {}", addr))
}

#[cfg(test)]
mod tests {
    use std::collections::{HashSet, VecDeque};
    use std::fs;
    use std::sync::mpsc;

    use super::{
        local_temporary_path, process_idle_control, remote_temporary_path, replace_local_file,
        validate_transfer_size,
    };
    use crate::plugins::ssh_client::models::{
        SftpControl, SftpRequest, SftpResponse, TransferDirection, TransferTask,
    };

    #[test]
    fn temporary_paths_stay_next_to_destination() {
        assert_eq!(
            remote_temporary_path("/home/user/file.txt", "task-1"),
            "/home/user/file.txt.tools-box-part-task-1"
        );
        assert_eq!(
            local_temporary_path(std::path::Path::new(r"C:\data\file.txt"), "task-1"),
            std::path::PathBuf::from(r"C:\data\.file.txt.tools-box-part-task-1")
        );
    }

    #[test]
    fn replace_local_file_preserves_old_file_until_new_file_is_ready() {
        let test_dir =
            std::env::temp_dir().join(format!("tools-box-sftp-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&test_dir).expect("创建测试目录失败");
        let destination = test_dir.join("file.txt");
        let temporary = test_dir.join(".file.txt.part");
        fs::write(&destination, "旧内容").expect("写入旧文件失败");
        fs::write(&temporary, "新内容").expect("写入临时文件失败");

        replace_local_file(&temporary, &destination, "task-1").expect("替换文件失败");

        assert_eq!(
            fs::read_to_string(&destination).expect("读取结果失败"),
            "新内容"
        );
        assert!(!temporary.exists());
        fs::remove_dir_all(&test_dir).expect("清理测试目录失败");
    }

    #[test]
    fn cancel_control_arriving_before_task_is_remembered() {
        let (control_tx, control_rx) = mpsc::channel();
        let (response_tx, _response_rx) = mpsc::channel();
        let mut pending_requests = VecDeque::new();
        let mut cancelled_task_ids = HashSet::new();
        control_tx
            .send(SftpControl::CancelTransfer("task-1".to_string()))
            .expect("发送取消请求失败");

        assert!(!process_idle_control(
            &control_rx,
            &mut pending_requests,
            &mut cancelled_task_ids,
            &response_tx
        ));
        assert!(cancelled_task_ids.contains("task-1"));
    }

    #[test]
    fn cancel_control_removes_pending_task_and_emits_terminal_state() {
        let (control_tx, control_rx) = mpsc::channel();
        let (response_tx, response_rx) = mpsc::channel();
        let task = TransferTask::new(
            "/source".to_string(),
            "/target".to_string(),
            TransferDirection::Upload,
            Some(10),
        );
        let task_id = task.id.clone();
        let mut pending_requests = VecDeque::from([SftpRequest::Upload(task)]);
        let mut cancelled_task_ids = HashSet::new();
        control_tx
            .send(SftpControl::CancelTransfer(task_id))
            .expect("发送取消请求失败");

        assert!(!process_idle_control(
            &control_rx,
            &mut pending_requests,
            &mut cancelled_task_ids,
            &response_tx
        ));
        assert!(pending_requests.is_empty());
        match response_rx.try_recv() {
            Ok(SftpResponse::TransferProgress(task)) => {
                assert!(task.done);
                assert_eq!(task.error.as_deref(), Some("传输已取消"));
            }
            response => panic!("收到非预期响应: {:?}", response),
        }
    }

    #[test]
    fn disconnect_control_is_not_blocked_by_operation_queue() {
        let (control_tx, control_rx) = mpsc::channel();
        let (response_tx, _response_rx) = mpsc::channel();
        let task = TransferTask::new(
            "/source".to_string(),
            "/target".to_string(),
            TransferDirection::Download,
            Some(10),
        );
        let mut pending_requests = VecDeque::from([SftpRequest::Download(task)]);
        let mut cancelled_task_ids = HashSet::new();
        control_tx
            .send(SftpControl::Disconnect)
            .expect("发送断开请求失败");

        assert!(process_idle_control(
            &control_rx,
            &mut pending_requests,
            &mut cancelled_task_ids,
            &response_tx
        ));
        assert_eq!(pending_requests.len(), 1);
    }

    #[test]
    fn transfer_size_validation_rejects_short_or_growing_sources() {
        assert!(validate_transfer_size(Some(1024), 1024).is_ok());
        assert!(validate_transfer_size(None, 2048).is_ok());
        assert!(validate_transfer_size(Some(0), 0).is_ok());
        assert!(validate_transfer_size(Some(0), 1).is_err());
        assert!(validate_transfer_size(Some(1024), 512).is_err());
        assert!(validate_transfer_size(Some(1024), 2048).is_err());
    }
}

/// 列出本地目录内容
pub fn list_local_dir(path: &str) -> Result<Vec<FileEntry>> {
    let dir_path = Path::new(path);
    if !dir_path.is_dir() {
        anyhow::bail!("'{}' 不是有效目录", path);
    }

    let mut entries = Vec::new();
    for entry in std::fs::read_dir(dir_path).context("读取目录失败")? {
        let entry = entry.context("读取目录条目失败")?;
        let metadata = entry.metadata().context("读取文件元数据失败")?;
        let name = entry.file_name().to_string_lossy().to_string();
        let full_path = entry.path().to_string_lossy().to_string();

        entries.push(FileEntry {
            name,
            path: full_path,
            is_dir: metadata.is_dir(),
            size: metadata.len(),
            size_known: true,
            modified: metadata
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                // SAFETY: Unix 时间戳到 2262 年才会溢出 i64
                .map(|d| d.as_secs().min(i64::MAX as u64) as i64),
        });
    }

    // 目录在前，文件在后
    entries.sort_by(|a, b| match (a.is_dir, b.is_dir) {
        (true, false) => std::cmp::Ordering::Less,
        (false, true) => std::cmp::Ordering::Greater,
        _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
    });

    Ok(entries)
}

/// 获取本地用户的主目录
pub fn home_dir() -> String {
    dirs::home_dir()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|| "/".to_string())
}
