use anyhow::{Context, Result};
use std::path::PathBuf;

/// Tools Box 管理区域的开始标记
const TOOLS_BOX_START: &str = "# >>> Tools Box START >>>";
/// Tools Box 管理区域的结束标记
const TOOLS_BOX_END: &str = "# <<< Tools Box END <<<";

/// Hosts 条目
#[derive(Debug, Clone)]
pub struct HostsLine {
    pub ip: String,
    pub hostname: String,
    pub comment: Option<String>,
    pub is_active: bool,
}

/// 获取系统 hosts 文件路径
pub fn hosts_file_path() -> PathBuf {
    if cfg!(target_os = "windows") {
        PathBuf::from(r"C:\Windows\System32\drivers\etc\hosts")
    } else {
        PathBuf::from("/etc/hosts")
    }
}

/// 读取系统 hosts 文件
pub fn read_system_hosts() -> Result<String> {
    let path = hosts_file_path();
    std::fs::read_to_string(&path)
        .with_context(|| format!("无法读取 hosts 文件: {}", path.display()))
}

/// 解析 hosts 文件内容
///
/// 支持的格式：
/// - `# 注释`
/// - `127.0.0.1 localhost`
/// - `# 127.0.0.1 localhost`（被注释的条目）
/// - `127.0.0.1 localhost # 备注`
pub fn parse_hosts(content: &str) -> Vec<HostsLine> {
    let mut entries = Vec::new();

    for line in content.lines() {
        let line = line.trim();

        // 跳过空行
        if line.is_empty() {
            continue;
        }

        // 检查是否是被注释的 hosts 条目（# 后面跟着 IP 格式的内容）
        if line.starts_with('#') {
            let uncommented = line[1..].trim();
            // 只有当注释后的内容看起来像 hosts 条目时才解析
            if let Some(entry) = parse_hosts_line(uncommented) {
                let mut entry = entry;
                entry.is_active = false;
                entries.push(entry);
            }
            continue;
        }

        // 解析普通 hosts 条目
        if let Some(entry) = parse_hosts_line(line) {
            entries.push(entry);
        }
    }

    entries
}

/// 解析单行 hosts 条目
fn parse_hosts_line(line: &str) -> Option<HostsLine> {
    let line = line.trim();
    if line.is_empty() {
        return None;
    }

    // 分离注释
    let (content, comment) = if let Some(pos) = line.find('#') {
        let comment = line[pos + 1..].trim();
        let comment = if comment.is_empty() {
            None
        } else {
            Some(comment.to_string())
        };
        (line[..pos].trim(), comment)
    } else {
        (line, None)
    };

    // 分离 IP 和 hostname
    let parts: Vec<&str> = content.split_whitespace().collect();
    if parts.len() >= 2 && is_ip_address(parts[0]) {
        Some(HostsLine {
            ip: parts[0].to_string(),
            hostname: parts[1].to_string(),
            comment,
            is_active: true,
        })
    } else {
        None
    }
}

/// 简单检查是否是 IP 地址格式（IPv4 或 IPv6）
fn is_ip_address(s: &str) -> bool {
    // IPv4: 数字和点
    if s.chars().all(|c| c.is_ascii_digit() || c == '.') && s.contains('.') {
        return true;
    }
    // IPv6: 包含冒号
    if s.contains(':') && s.chars().all(|c| c.is_ascii_hexdigit() || c == ':') {
        return true;
    }
    false
}

/// 生成 Tools Box 管理的 hosts 内容块
///
/// 返回带有标记的内容块，用于追加到系统 hosts 文件
pub fn generate_hosts_block(env_entries: &[HostsLine]) -> String {
    let mut output = String::new();

    // 开始标记
    output.push_str(TOOLS_BOX_START);
    output.push_str("\n");
    output.push_str("# Managed by Tools Box - Do not edit manually\n");
    output.push('\n');

    // 环境条目
    if !env_entries.is_empty() {
        for entry in env_entries {
            if entry.is_active {
                output.push_str(&format_entry(entry));
                output.push('\n');
            } else {
                output.push_str(&format!("# {}", format_entry(entry)));
                output.push('\n');
            }
        }
    }

    // 结束标记
    output.push_str(TOOLS_BOX_END);
    output.push('\n');

    output
}

/// 从 hosts 内容中移除 Tools Box 管理的区域
pub fn remove_tools_box_section(content: &str) -> String {
    let mut result = String::new();
    let mut in_section = false;

    for line in content.lines() {
        if line.trim() == TOOLS_BOX_START {
            in_section = true;
            continue;
        }
        if line.trim() == TOOLS_BOX_END {
            in_section = false;
            continue;
        }
        if !in_section {
            result.push_str(line);
            result.push('\n');
        }
    }

    result
}

/// 格式化单条 hosts 条目
fn format_entry(entry: &HostsLine) -> String {
    if let Some(comment) = &entry.comment {
        format!("{}\t\t{} # {}", entry.ip, entry.hostname, comment)
    } else {
        format!("{}\t\t{}", entry.ip, entry.hostname)
    }
}

/// 备份 hosts 文件
pub fn backup_hosts() -> Result<PathBuf> {
    let path = hosts_file_path();
    let content = std::fs::read_to_string(&path)
        .with_context(|| format!("无法读取 hosts 文件: {}", path.display()))?;

    let backup_dir = dirs::data_dir()
        .context("无法获取数据目录")?
        .join("tools-box")
        .join("hosts_backups");

    std::fs::create_dir_all(&backup_dir)
        .with_context(|| format!("无法创建备份目录: {}", backup_dir.display()))?;

    let timestamp = chrono::Local::now().format("%Y%m%d_%H%M%S");
    let backup_path = backup_dir.join(format!("hosts_{}.bak", timestamp));

    std::fs::write(&backup_path, &content)
        .with_context(|| format!("无法写入备份文件: {}", backup_path.display()))?;

    log::info!("已备份 hosts 文件到: {}", backup_path.display());
    Ok(backup_path)
}

/// 以追加方式更新系统 hosts 文件
///
/// 保留系统原有内容，只更新 Tools Box 管理的区域
pub fn append_to_system_hosts(env_entries: &[HostsLine]) -> Result<()> {
    let path = hosts_file_path();

    // 读取现有内容
    let existing_content = std::fs::read_to_string(&path)
        .with_context(|| format!("无法读取 hosts 文件: {}", path.display()))?;

    // 移除旧的 Tools Box 区域
    let clean_content = remove_tools_box_section(&existing_content);

    // 生成新的 Tools Box 区域
    let tools_box_block = generate_hosts_block(env_entries);

    // 合并内容：原有内容 + Tools Box 区域
    let mut final_content = clean_content.trim_end().to_string();
    final_content.push_str("\n\n");
    final_content.push_str(&tools_box_block);

    // 写入文件
    std::fs::write(&path, &final_content)
        .with_context(|| format!("无法写入 hosts 文件: {}", path.display()))?;

    log::info!("已追加更新系统 hosts 文件");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_hosts() {
        let content = r#"# Hosts file
127.0.0.1       localhost
::1             localhost

# Development
192.168.1.100   dev.api.example.com
192.168.1.101   dev.db.example.com # Database server

# Commented out
# 10.0.0.1      old.server.com
"#;

        let entries = parse_hosts(content);
        assert_eq!(entries.len(), 5);

        assert_eq!(entries[0].ip, "127.0.0.1");
        assert_eq!(entries[0].hostname, "localhost");
        assert!(entries[0].is_active);

        assert_eq!(entries[2].ip, "192.168.1.100");
        assert_eq!(entries[2].hostname, "dev.api.example.com");

        assert_eq!(entries[3].comment.as_deref(), Some("Database server"));

        // 被注释的条目
        assert_eq!(entries[4].ip, "10.0.0.1");
        assert_eq!(entries[4].hostname, "old.server.com");
        assert!(!entries[4].is_active);
    }

    #[test]
    fn test_generate_hosts_block() {
        let env = vec![
            HostsLine {
                ip: "192.168.1.100".to_string(),
                hostname: "dev.api.com".to_string(),
                comment: Some("API server".to_string()),
                is_active: true,
            },
            HostsLine {
                ip: "192.168.1.101".to_string(),
                hostname: "dev.db.com".to_string(),
                comment: None,
                is_active: false,
            },
        ];

        let output = generate_hosts_block(&env);
        assert!(output.contains(TOOLS_BOX_START));
        assert!(output.contains(TOOLS_BOX_END));
        assert!(output.contains("192.168.1.100"));
        assert!(output.contains("# 192.168.1.101"));
    }

    #[test]
    fn test_remove_tools_box_section() {
        let content = r#"# Original hosts
127.0.0.1       localhost

# >>> Tools Box START >>>
# Managed by Tools Box - Do not edit manually

192.168.1.100   dev.api.com
# <<< Tools Box END <<<

# Other entries
10.0.0.1        other.com
"#;

        let result = remove_tools_box_section(content);
        assert!(!result.contains("Tools Box START"));
        assert!(!result.contains("Tools Box END"));
        assert!(!result.contains("192.168.1.100"));
        assert!(result.contains("127.0.0.1"));
        assert!(result.contains("10.0.0.1"));
    }

    #[test]
    fn test_remove_tools_box_section_no_section() {
        let content = r#"# Original hosts
127.0.0.1       localhost
"#;

        let result = remove_tools_box_section(content);
        assert!(result.contains("127.0.0.1"));
    }
}
