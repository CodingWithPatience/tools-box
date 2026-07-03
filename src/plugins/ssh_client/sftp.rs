use std::io::{Read, Write};
use std::path::Path;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result};
use ssh2::{Session, Sftp};

use super::crypto;
use super::models::{
    AuthMethod, FileEntry, SftpRequest, SftpResponse, TransferDirection, TransferTask,
};

/// SFTP 传输缓冲区大小（128 KB）
const TRANSFER_BUFFER_SIZE: usize = 128 * 1024;

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
    ) -> Result<(mpsc::SyncSender<SftpRequest>, mpsc::Receiver<SftpResponse>)> {
        let addr = format!("{}:{}", host, port);
        let uname = username.to_string();
        let auth_clone = auth.clone();

        let (req_tx, req_rx) = mpsc::sync_channel::<SftpRequest>(64);
        let (resp_tx, resp_rx) = mpsc::sync_channel::<SftpResponse>(256);

        thread::Builder::new()
            .name("sftp-io".to_string())
            .spawn(move || {
                if let Err(e) = Self::session_loop(&addr, &uname, &auth_clone, &req_rx, &resp_tx) {
                    let _ = resp_tx.send(SftpResponse::Error(format!("SFTP 连接错误: {}", e)));
                    log::error!("SFTP 会话线程异常退出: {}", e);
                }
            })
            .context("创建 SFTP I/O 线程失败")?;

        Ok((req_tx, resp_rx))
    }

    /// SFTP 会话主循环
    fn session_loop(
        addr: &str,
        username: &str,
        auth: &AuthMethod,
        req_rx: &mpsc::Receiver<SftpRequest>,
        resp_tx: &mpsc::SyncSender<SftpResponse>,
    ) -> Result<()> {
        // 建立独立的 SSH 连接
        let tcp = std::net::TcpStream::connect(addr)
            .with_context(|| format!("SFTP: 无法连接到 {}", addr))?;
        tcp.set_read_timeout(Some(Duration::from_secs(30)))
            .context("SFTP: 设置 TCP 超时失败")?;

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

        // 请求处理循环
        loop {
            match req_rx.recv() {
                Ok(SftpRequest::ListDirectory(path)) => {
                    Self::handle_list_dir(&sftp, &path, resp_tx);
                }
                Ok(SftpRequest::Upload(local_path, remote_path)) => {
                    Self::handle_upload(&sftp, &local_path, &remote_path, resp_tx);
                }
                Ok(SftpRequest::Download(remote_path, local_path)) => {
                    Self::handle_download(&sftp, &remote_path, &local_path, resp_tx);
                }
                Ok(SftpRequest::MkDir(path)) => {
                    Self::handle_mkdir(&sftp, &path, resp_tx);
                }
                Ok(SftpRequest::Delete(path)) => {
                    Self::handle_delete(&sftp, &path, resp_tx);
                }
                Ok(SftpRequest::Disconnect) => {
                    log::info!("SFTP: 收到断开请求");
                    break;
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
    fn handle_list_dir(sftp: &Sftp, path: &str, resp_tx: &mpsc::SyncSender<SftpResponse>) {
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
                            // SAFETY: mtime 是 Unix 时间戳，u64→i64 仅在 2262 年后溢出
                            modified: stat.mtime.map(|t| t.min(i64::MAX as u64) as i64),
                            permissions: stat.perm,
                        })
                    })
                    .collect();

                // 目录在前，文件在后，同类型按名称排序
                file_list.sort_by(|a, b| {
                    match (a.is_dir, b.is_dir) {
                        (true, false) => std::cmp::Ordering::Less,
                        (false, true) => std::cmp::Ordering::Greater,
                        _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
                    }
                });

                let _ = resp_tx.send(SftpResponse::DirectoryList(path.to_string(), file_list));
            }
            Err(e) => {
                let _ = resp_tx.send(SftpResponse::Error(format!(
                    "无法列出目录 '{}': {}",
                    path, e
                )));
            }
        }
    }

    /// 上传文件
    fn handle_upload(
        sftp: &Sftp,
        local_path: &str,
        remote_path: &str,
        resp_tx: &mpsc::SyncSender<SftpResponse>,
    ) {
        let filename = Path::new(local_path)
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| local_path.to_string());

        // 读取本地文件
        let mut local_file = match std::fs::File::open(local_path) {
            Ok(f) => f,
            Err(e) => {
                let _ = resp_tx.send(SftpResponse::Error(format!(
                    "无法打开本地文件 '{}': {}",
                    local_path, e
                )));
                return;
            }
        };

        let total_size = match local_file.metadata() {
            Ok(m) => m.len(),
            Err(e) => {
                let _ = resp_tx.send(SftpResponse::Error(format!(
                    "无法读取文件大小: {}",
                    e
                )));
                return;
            }
        };

        // 创建远程文件
        let mut remote_file = match sftp.create(Path::new(remote_path)) {
            Ok(f) => f,
            Err(e) => {
                let _ = resp_tx.send(SftpResponse::Error(format!(
                    "无法创建远程文件 '{}': {}",
                    remote_path, e
                )));
                return;
            }
        };

        let mut task = TransferTask {
            source: local_path.to_string(),
            destination: remote_path.to_string(),
            direction: TransferDirection::Upload,
            total_size,
            transferred: 0,
            filename: filename.clone(),
            done: false,
            error: None,
        };

        let _ = resp_tx.send(SftpResponse::TransferProgress(task.clone()));

        // 分块传输
        let mut buf = vec![0u8; TRANSFER_BUFFER_SIZE];
        loop {
            match local_file.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    if let Err(e) = remote_file.write_all(&buf[..n]) {
                        task.done = true;
                        task.error = Some(format!("写入远程文件失败: {}", e));
                        let _ = resp_tx.send(SftpResponse::TransferProgress(task));
                        return;
                    }
                    // SAFETY: usize 到 u64 始终是安全的拓宽转换
                    task.transferred += n as u64;
                    let _ = resp_tx.send(SftpResponse::TransferProgress(task.clone()));
                }
                Err(e) => {
                    task.done = true;
                    task.error = Some(format!("读取本地文件失败: {}", e));
                    let _ = resp_tx.send(SftpResponse::TransferProgress(task));
                    return;
                }
            }
        }

        task.done = true;
        let _ = resp_tx.send(SftpResponse::TransferProgress(task));
        let _ = resp_tx.send(SftpResponse::OperationDone(format!(
            "上传完成: {} → {}",
            local_path, remote_path
        )));
        log::info!("SFTP 上传完成: {} → {}", local_path, remote_path);
    }

    /// 下载文件
    fn handle_download(
        sftp: &Sftp,
        remote_path: &str,
        local_path: &str,
        resp_tx: &mpsc::SyncSender<SftpResponse>,
    ) {
        let filename = Path::new(remote_path)
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| remote_path.to_string());

        // 打开远程文件
        let mut remote_file = match sftp.open(Path::new(remote_path)) {
            Ok(f) => f,
            Err(e) => {
                let _ = resp_tx.send(SftpResponse::Error(format!(
                    "无法打开远程文件 '{}': {}",
                    remote_path, e
                )));
                return;
            }
        };

        // 获取远程文件大小
        let total_size = match remote_file.stat() {
            Ok(stat) => stat.size.unwrap_or(0),
            Err(_) => 0,
        };

        // 创建本地文件
        let mut local_file = match std::fs::File::create(local_path) {
            Ok(f) => f,
            Err(e) => {
                let _ = resp_tx.send(SftpResponse::Error(format!(
                    "无法创建本地文件 '{}': {}",
                    local_path, e
                )));
                return;
            }
        };

        let mut task = TransferTask {
            source: remote_path.to_string(),
            destination: local_path.to_string(),
            direction: TransferDirection::Download,
            total_size,
            transferred: 0,
            filename: filename.clone(),
            done: false,
            error: None,
        };

        let _ = resp_tx.send(SftpResponse::TransferProgress(task.clone()));

        // 分块传输
        let mut buf = vec![0u8; TRANSFER_BUFFER_SIZE];
        loop {
            match remote_file.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    if let Err(e) = local_file.write_all(&buf[..n]) {
                        task.done = true;
                        task.error = Some(format!("写入本地文件失败: {}", e));
                        let _ = resp_tx.send(SftpResponse::TransferProgress(task));
                        return;
                    }
                    // SAFETY: usize 到 u64 始终是安全的拓宽转换
                    task.transferred += n as u64;
                    let _ = resp_tx.send(SftpResponse::TransferProgress(task.clone()));
                }
                Err(e) => {
                    task.done = true;
                    task.error = Some(format!("读取远程文件失败: {}", e));
                    let _ = resp_tx.send(SftpResponse::TransferProgress(task));
                    return;
                }
            }
        }

        task.done = true;
        let _ = resp_tx.send(SftpResponse::TransferProgress(task));
        let _ = resp_tx.send(SftpResponse::OperationDone(format!(
            "下载完成: {} → {}",
            remote_path, local_path
        )));
        log::info!("SFTP 下载完成: {} → {}", remote_path, local_path);
    }

    /// 创建远程目录
    fn handle_mkdir(sftp: &Sftp, path: &str, resp_tx: &mpsc::SyncSender<SftpResponse>) {
        match sftp.mkdir(Path::new(path), 0o755) {
            Ok(()) => {
                let _ = resp_tx.send(SftpResponse::OperationDone(format!(
                    "目录已创建: {}",
                    path
                )));
            }
            Err(e) => {
                let _ = resp_tx.send(SftpResponse::Error(format!(
                    "创建目录失败 '{}': {}",
                    path, e
                )));
            }
        }
    }

    /// 删除远程文件或目录
    fn handle_delete(sftp: &Sftp, path: &str, resp_tx: &mpsc::SyncSender<SftpResponse>) {
        let p = Path::new(path);
        // 先尝试作为文件删除
        match sftp.unlink(p) {
            Ok(()) => {
                let _ = resp_tx.send(SftpResponse::OperationDone(format!(
                    "已删除: {}",
                    path
                )));
            }
            Err(_) => {
                // 尝试作为目录删除（仅空目录）
                match sftp.rmdir(p) {
                    Ok(()) => {
                        let _ = resp_tx.send(SftpResponse::OperationDone(format!(
                            "目录已删除: {}",
                            path
                        )));
                    }
                    Err(e) => {
                        let _ = resp_tx.send(SftpResponse::Error(format!(
                            "删除失败 '{}': {}",
                            path, e
                        )));
                    }
                }
            }
        }
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
            modified: metadata
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                // SAFETY: Unix 时间戳到 2262 年才会溢出 i64
                .map(|d| d.as_secs().min(i64::MAX as u64) as i64),
            permissions: None,
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

/// 获取本地根目录（Windows 返回盘符列表，Unix 返回 "/"）
pub fn root_dir() -> String {
    #[cfg(target_os = "windows")]
    {
        "C:\\".to_string()
    }
    #[cfg(not(target_os = "windows"))]
    {
        "/".to_string()
    }
}
