use std::io::{Read, Write};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result};
use ssh2::Session;

use super::models::{AuthMethod, SshInput, SshOutput};

/// SSH 连接管理
pub struct SshClient;

impl SshClient {
    /// 连接 SSH 服务器，启动 I/O 线程，返回 (命令发送端, 数据接收端)
    ///
    /// 使用有界通道 (sync_channel, bound=256) 防止高频输出时 OOM。
    /// I/O 线程设置为非阻塞模式，避免 write/wait_close 永久挂起。
    pub fn connect(
        host: &str,
        port: u16,
        username: &str,
        auth: &AuthMethod,
        terminal_cols: u16,
        terminal_rows: u16,
    ) -> Result<(mpsc::SyncSender<SshInput>, mpsc::Receiver<SshOutput>)> {
        let addr = format!("{}:{}", host, port);
        let uname = username.to_string();
        let auth_clone = auth.clone();

        // 有界通道：防止 SSH 高频输出时消息堆积导致 OOM
        let (input_tx, input_rx) = mpsc::sync_channel::<SshInput>(256);
        let (output_tx, output_rx) = mpsc::sync_channel::<SshOutput>(256);

        // 线程名截断避免超过系统限制 (14 字符可用)
        let thread_label = if addr.len() > 10 {
            format!("ssh-{}", &addr[..addr.len().min(10)])
        } else {
            format!("ssh-{}", addr)
        };

        thread::Builder::new()
            .name(thread_label)
            .spawn(move || {
                if let Err(e) = Self::session_loop(
                    &addr,
                    &uname,
                    &auth_clone,
                    terminal_cols,
                    terminal_rows,
                    &input_rx,
                    &output_tx,
                ) {
                    let _ = output_tx.send(SshOutput::Error(format!("SSH 连接错误: {}", e)));
                    log::error!("SSH 会话线程异常退出: {}", e);
                }
            })
            .context("创建 SSH I/O 线程失败")?;

        Ok((input_tx, output_rx))
    }

    /// SSH 会话主循环（在独立线程中运行）
    fn session_loop(
        addr: &str,
        username: &str,
        auth: &AuthMethod,
        cols: u16,
        rows: u16,
        input_rx: &mpsc::Receiver<SshInput>,
        output_tx: &mpsc::SyncSender<SshOutput>,
    ) -> Result<()> {
        // 建立 TCP 连接（带读取超时）
        let tcp =
            std::net::TcpStream::connect(addr).with_context(|| format!("无法连接到 {}", addr))?;
        tcp.set_read_timeout(Some(Duration::from_millis(200)))
            .context("设置 TCP 读取超时失败")?;

        // 创建 SSH 会话（握手阶段保持阻塞模式）
        let mut session = Session::new().context("创建 SSH 会话失败")?;
        session.set_tcp_stream(tcp);
        session.handshake().context("SSH 握手失败")?;

        // 认证
        match auth {
            AuthMethod::Password {
                encrypted_password,
                iv,
                salt,
            } => {
                let password = super::crypto::decrypt_password(encrypted_password, iv, salt)
                    .context("解密 SSH 密码失败")?;
                session
                    .userauth_password(username, &password)
                    .with_context(|| format!("密码认证失败 (用户: {})", username))?;
            }
            AuthMethod::KeyFile {
                private_key_path,
                encrypted_passphrase,
            } => {
                let passphrase = if let Some((ct, iv, salt)) = encrypted_passphrase {
                    Some(
                        super::crypto::decrypt_password(ct, iv, salt)
                            .context("解密密钥密码短语失败")?,
                    )
                } else {
                    None
                };
                session
                    .userauth_pubkey_file(
                        username,
                        None,
                        std::path::Path::new(private_key_path),
                        passphrase.as_deref(),
                    )
                    .with_context(|| {
                        format!(
                            "密钥认证失败 (用户: {}, 密钥: {})",
                            username, private_key_path
                        )
                    })?;
            }
        }

        if !session.authenticated() {
            anyhow::bail!("SSH 认证未通过");
        }

        log::info!("SSH 认证成功: {}@{}", username, addr);

        // 请求 PTY + 启动 shell
        let mut channel = session.channel_session().context("创建 SSH channel 失败")?;
        channel
            .request_pty(
                "xterm-256color",
                None,
                Some((cols.into(), rows.into(), 0, 0)),
            )
            .context("请求 PTY 失败")?;
        channel.shell().context("启动 shell 失败")?;

        // 切换到非阻塞模式，避免读写操作永久阻塞
        session.set_blocking(false);

        // 通知主线程连接已就绪
        let _ = output_tx.send(SshOutput::Connected);

        // 主读写循环
        let mut buf = [0u8; 8192];
        loop {
            // 1. 检查来自主线程的消息（非阻塞）
            match input_rx.try_recv() {
                Ok(SshInput::KeyInput(data)) => {
                    let mut remaining = &data[..];
                    while !remaining.is_empty() {
                        match channel.write(remaining) {
                            Ok(n) => remaining = &remaining[n..],
                            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                                // 非阻塞模式下写缓冲区满，丢弃剩余数据
                                break;
                            }
                            Err(e) => {
                                log::warn!("写入 SSH channel 失败: {}", e);
                                let _ = output_tx
                                    .send(SshOutput::Disconnected(format!("写入失败: {}", e)));
                                return Ok(());
                            }
                        }
                    }
                }
                Ok(SshInput::Resize(c, r)) => {
                    if let Err(e) = channel.request_pty_size(c.into(), r.into(), None, None) {
                        log::warn!("调整终端大小失败: {}", e);
                    }
                }
                Ok(SshInput::Disconnect) => {
                    log::info!("主动断开 SSH 连接");
                    let _ = output_tx.send(SshOutput::Disconnected("手动断开".to_string()));
                    break;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    let _ = output_tx.send(SshOutput::Disconnected("主线程已断开".to_string()));
                    break;
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }

            // 2. 读取 SSH 终端输出
            match channel.read(&mut buf) {
                Ok(n) if n > 0 => {
                    // 有界通道：满时丢弃旧数据，避免 OOM
                    let _ = output_tx.send(SshOutput::TerminalData(buf[..n].to_vec()));
                }
                Ok(_) => {
                    // n == 0 表示流结束
                    let _ = output_tx.send(SshOutput::Disconnected("远程主机已断开".to_string()));
                    break;
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(e) => {
                    log::warn!("读取 SSH channel 失败: {}", e);
                    let _ = output_tx.send(SshOutput::Error(format!("读取错误: {}", e)));
                    break;
                }
            }
        }

        // 优雅关闭：先发 EOF，再关闭 channel
        session.set_blocking(true);
        let _ = channel.send_eof();
        let _ = channel.close();
        if let Err(e) = channel.wait_close() {
            // -34 表示 channel 已在其他状态关闭，属正常情况
            log::debug!("SSH channel 关闭: {}", e);
        }
        log::info!("SSH 连接已断开");
        Ok(())
    }
}
