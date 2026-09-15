use anyhow::{Context, Result, bail};
use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};

/// Tools Box 管理区域的开始标记
const TOOLS_BOX_START: &str = "# >>> Tools Box START >>>";
/// Tools Box 管理区域的结束标记
const TOOLS_BOX_END: &str = "# <<< Tools Box END <<<";

/// Hosts 条目
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostsLine {
    /// IP 地址
    pub ip: String,
    /// 主机名
    pub hostname: String,
    /// 行尾备注
    pub comment: Option<String>,
    /// 是否为启用状态（禁用条目以注释形式写入）
    pub is_active: bool,
}

/// 环境分组（环境名称 + 该环境下的条目）
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostsGroup {
    /// 环境名称，作为注释标题写入 hosts 文件
    pub name: String,
    /// 该环境下的条目
    pub entries: Vec<HostsLine>,
}

/// 主机名在某个环境下的映射记录
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostnameMapping {
    /// 环境名称
    pub environment: String,
    /// 该环境下映射到的 IP
    pub ip: String,
}

/// 主机名冲突：同一主机名被映射到多个不同 IP
#[derive(Debug, Clone)]
pub struct HostnameConflict {
    /// 冲突的主机名（保留首次出现的写法，比较时不区分大小写）
    pub hostname: String,
    /// 各环境下的映射记录（按环境名升序）
    pub mappings: Vec<HostnameMapping>,
    /// 冲突是否涉及多个环境（false 表示冲突发生在同一个环境内部）
    pub is_cross_environment: bool,
}

impl HostnameConflict {
    /// 生成用于界面与日志展示的冲突描述
    pub fn describe(&self) -> String {
        let mappings = self
            .mappings
            .iter()
            .map(|mapping| format!("{} → {}", mapping.environment, mapping.ip))
            .collect::<Vec<String>>()
            .join("，");

        let scope = if self.is_cross_environment {
            "跨环境"
        } else {
            "同一环境内"
        };

        format!("{}（{}）：{}", self.hostname, scope, mappings)
    }
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
        if let Some(uncommented) = line.strip_prefix('#') {
            // 只有当注释后的内容看起来像 hosts 条目时才解析
            if let Some(mut entry) = parse_hosts_line(uncommented.trim()) {
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
/// 返回带有标记的内容块，用于追加到系统 hosts 文件。
/// 多个启用的环境会依次写入同一个管理区域，每个环境带有独立的注释标题；
/// 没有条目的环境不会产生空标题
pub fn generate_hosts_block(groups: &[HostsGroup]) -> String {
    let mut output = String::new();

    // 开始标记
    output.push_str(TOOLS_BOX_START);
    output.push('\n');
    output.push_str("# Managed by Tools Box - Do not edit manually\n");

    // 按环境分组输出条目
    for group in groups.iter().filter(|group| !group.entries.is_empty()) {
        output.push('\n');
        output.push_str(&format!("# --- {} ---\n", sanitize_host_field(&group.name)));

        for entry in &group.entries {
            if entry.is_active {
                output.push_str(&format_entry(entry));
                output.push('\n');
            } else {
                output.push_str(&format!("# {}\n", format_entry(entry)));
            }
        }
    }

    // 结束标记
    output.push_str(TOOLS_BOX_END);
    output.push('\n');

    output
}

/// 查找主机名冲突
///
/// 只统计已启用条目；同一主机名被映射到多个不同 IP 时，
/// 最终生效的映射取决于系统的解析顺序，需要在应用前提示用户。
/// 主机名比较不区分大小写，冲突既可能发生在多个环境之间，
/// 也可能发生在同一个环境内部（由 `is_cross_environment` 区分）。
pub fn find_hostname_conflicts(groups: &[HostsGroup]) -> Vec<HostnameConflict> {
    // key 为小写主机名，value 为（首次出现的写法, 映射记录）
    let mut records_by_host: BTreeMap<String, (String, Vec<HostnameMapping>)> = BTreeMap::new();

    for group in groups {
        for entry in group.entries.iter().filter(|entry| entry.is_active) {
            let record = records_by_host
                .entry(entry.hostname.to_ascii_lowercase())
                .or_insert_with(|| (entry.hostname.clone(), Vec::new()));

            record.1.push(HostnameMapping {
                environment: group.name.clone(),
                ip: entry.ip.clone(),
            });
        }
    }

    records_by_host
        .into_iter()
        .filter(|(_, (_, mappings))| has_distinct_ip(mappings))
        .map(|(_, (hostname, mut mappings))| {
            // 映射按环境名排序，保证提示内容稳定可复现
            mappings.sort_by(|a, b| a.environment.cmp(&b.environment));

            HostnameConflict {
                is_cross_environment: has_distinct_environment(&mappings),
                hostname,
                mappings,
            }
        })
        .collect()
}

/// 判断同一主机名的映射中是否存在多个不同的 IP
fn has_distinct_ip(mappings: &[HostnameMapping]) -> bool {
    match mappings.first() {
        Some(first) => mappings.iter().any(|mapping| mapping.ip != first.ip),
        None => false,
    }
}

/// 判断同一主机名的映射是否分布在多个环境中
fn has_distinct_environment(mappings: &[HostnameMapping]) -> bool {
    match mappings.first() {
        Some(first) => mappings
            .iter()
            .any(|mapping| mapping.environment != first.environment),
        None => false,
    }
}

/// 从 hosts 内容中移除 Tools Box 管理的区域
///
/// 标记不成对（缺少开始或结束标记、顺序颠倒）时返回错误，
/// 避免误删用户自己维护的 hosts 条目
pub fn remove_tools_box_section(content: &str) -> Result<String> {
    validate_tools_box_markers(content)?;

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

    Ok(result)
}

/// 校验 Tools Box 标记是否成对且顺序正确
fn validate_tools_box_markers(content: &str) -> Result<()> {
    let mut in_section = false;

    for line in content.lines() {
        let line = line.trim();

        if line == TOOLS_BOX_START {
            if in_section {
                bail!("hosts 文件中的 Tools Box 开始标记重复，请手动修复后重试");
            }
            in_section = true;
        } else if line == TOOLS_BOX_END {
            if !in_section {
                bail!("hosts 文件中存在多余的 Tools Box 结束标记，请手动修复后重试");
            }
            in_section = false;
        }
    }

    if in_section {
        bail!("hosts 文件中的 Tools Box 区域缺少结束标记，请手动修复后重试");
    }

    Ok(())
}

/// 格式化单条 hosts 条目
///
/// 写入前做防御性净化，避免历史数据中的换行等字符破坏 hosts 文件结构
fn format_entry(entry: &HostsLine) -> String {
    let ip = sanitize_host_field(&entry.ip);
    let hostname = sanitize_host_field(&entry.hostname);

    if let Some(comment) = &entry.comment {
        format!("{}\t\t{} # {}", ip, hostname, sanitize_field(comment))
    } else {
        format!("{}\t\t{}", ip, hostname)
    }
}

/// 净化条目字段：控制字符（含换行）替换为空格
fn sanitize_field(value: &str) -> String {
    value
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect::<String>()
        .trim()
        .to_string()
}

/// 净化 IP / 主机名字段：井号会截断条目内容，一并替换
fn sanitize_host_field(value: &str) -> String {
    sanitize_field(value).replace('#', " ")
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
/// 保留系统原有内容，只更新 Tools Box 管理的区域；
/// 传入的所有环境分组会被同时写入该区域
pub fn append_to_system_hosts(groups: &[HostsGroup]) -> Result<()> {
    let path = hosts_file_path();

    // 读取现有内容
    let existing_content = std::fs::read_to_string(&path)
        .with_context(|| format!("无法读取 hosts 文件: {}", path.display()))?;

    // 移除旧的 Tools Box 区域
    let clean_content = remove_tools_box_section(&existing_content)?;

    // 生成新的 Tools Box 区域
    let tools_box_block = generate_hosts_block(groups);

    // 合并内容：原有内容 + Tools Box 区域
    let mut final_content = clean_content.trim_end().to_string();
    final_content.push_str("\n\n");
    final_content.push_str(&tools_box_block);

    // 写入文件
    write_hosts_file(&path, &final_content)?;

    log::info!("已追加更新系统 hosts 文件");
    Ok(())
}

/// 原子写入 hosts 文件
///
/// 优先写入同目录下的临时文件再重命名覆盖，避免写入中断导致 hosts 内容残缺；
/// 目录不可写等场景下回退为直接覆盖写入（调用方已完成备份）
fn write_hosts_file(path: &Path, content: &str) -> Result<()> {
    match write_hosts_file_atomic(path, content) {
        Ok(()) => Ok(()),
        Err(e) => {
            log::warn!("原子写入 hosts 文件失败，回退为直接写入: {}", e);
            std::fs::write(path, content).with_context(|| {
                format!(
                    "无法写入 hosts 文件: {}（原子写入失败: {}）",
                    path.display(),
                    e
                )
            })
        }
    }
}

/// 通过临时文件 + 重命名原子替换 hosts 文件
fn write_hosts_file_atomic(path: &Path, content: &str) -> Result<()> {
    let temp_path = temp_hosts_path(path);

    // Unix 下记录原文件权限（如 /etc/hosts 的 644），替换成功后再恢复；
    // Windows 下权限继承自目录，且只读属性会让重命名失败，因此不做处理
    #[cfg(unix)]
    let original_permissions = std::fs::metadata(path).ok().map(|meta| meta.permissions());

    let mut file = std::fs::File::create(&temp_path)
        .with_context(|| format!("无法创建临时文件: {}", temp_path.display()))?;

    if let Err(e) = file
        .write_all(content.as_bytes())
        .and_then(|()| file.sync_all())
    {
        let _ = std::fs::remove_file(&temp_path);
        return Err(e).with_context(|| format!("无法写入临时文件: {}", temp_path.display()));
    }
    drop(file);

    if let Err(e) = std::fs::rename(&temp_path, path) {
        let _ = std::fs::remove_file(&temp_path);
        return Err(e).with_context(|| format!("无法替换 hosts 文件: {}", path.display()));
    }

    #[cfg(unix)]
    if let Some(permissions) = original_permissions {
        if let Err(e) = std::fs::set_permissions(path, permissions) {
            log::warn!("无法保留 hosts 文件权限: {}", e);
        }
    }

    Ok(())
}

/// 生成与 hosts 文件同目录的临时文件路径（带进程号，避免多实例互相干扰）
fn temp_hosts_path(path: &Path) -> PathBuf {
    path.with_extension(format!("{}.tools-box.tmp", std::process::id()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 构造一条启用状态的 hosts 条目
    fn active_line(ip: &str, hostname: &str) -> HostsLine {
        HostsLine {
            ip: ip.to_string(),
            hostname: hostname.to_string(),
            comment: None,
            is_active: true,
        }
    }

    /// 移除 Tools Box 区域并返回结果内容（失败时终止测试）
    fn remove_section(content: &str) -> String {
        match remove_tools_box_section(content) {
            Ok(result) => result,
            Err(e) => panic!("移除 Tools Box 区域失败: {}", e),
        }
    }

    /// 创建测试专用的临时目录
    fn temp_test_dir(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("tools-box-{}-{}", name, std::process::id()))
    }

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
        let groups = vec![HostsGroup {
            name: "dev".to_string(),
            entries: vec![
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
            ],
        }];

        let output = generate_hosts_block(&groups);
        assert!(output.contains(TOOLS_BOX_START));
        assert!(output.contains(TOOLS_BOX_END));
        assert!(output.contains("# --- dev ---"));
        assert!(output.contains("192.168.1.100"));
        assert!(output.contains("# 192.168.1.101"));
    }

    #[test]
    fn test_generate_hosts_block_multiple_groups() {
        let groups = vec![
            HostsGroup {
                name: "dev".to_string(),
                entries: vec![active_line("192.168.1.100", "dev.api.com")],
            },
            HostsGroup {
                name: "test".to_string(),
                entries: vec![active_line("10.0.0.10", "test.api.com")],
            },
        ];

        let output = generate_hosts_block(&groups);
        assert!(output.contains("# --- dev ---"));
        assert!(output.contains("# --- test ---"));
        assert!(output.contains("192.168.1.100"));
        assert!(output.contains("10.0.0.10"));

        // 两个环境都在同一个管理区域内
        assert_eq!(output.matches(TOOLS_BOX_START).count(), 1);
        assert_eq!(output.matches(TOOLS_BOX_END).count(), 1);
        match (output.find(TOOLS_BOX_START), output.find(TOOLS_BOX_END)) {
            (Some(start), Some(end)) => assert!(start < end, "开始标记应位于结束标记之前"),
            _ => panic!("生成的内容缺少 Tools Box 标记"),
        }
    }

    #[test]
    fn test_generate_hosts_block_skips_empty_group() -> Result<()> {
        let groups = vec![
            HostsGroup {
                name: "empty".to_string(),
                entries: Vec::new(),
            },
            HostsGroup {
                name: "dev".to_string(),
                entries: vec![active_line("192.168.1.100", "dev.api.com")],
            },
        ];

        let output = generate_hosts_block(&groups);
        assert!(!output.contains("# --- empty ---"));
        assert!(output.contains("# --- dev ---"));

        // 空标题不会残留，管理区域仍然标记成对
        assert!(validate_tools_box_markers(&output).is_ok());

        Ok(())
    }

    #[test]
    fn test_generate_hosts_block_sanitizes_group_name() -> Result<()> {
        // 历史数据中的异常名称不能破坏管理区域结构
        let groups = vec![HostsGroup {
            name: "dev\n# <<< Tools Box END <<<".to_string(),
            entries: vec![active_line("192.168.1.100", "dev.api.com")],
        }];

        let output = generate_hosts_block(&groups);
        assert_eq!(output.matches(TOOLS_BOX_END).count(), 1);
        assert!(validate_tools_box_markers(&output).is_ok());
        // 名称中的控制字符与井号被替换为空格
        assert!(output.contains("# --- dev   <<< Tools Box END <<< ---"));

        Ok(())
    }

    #[test]
    fn test_generate_and_remove_roundtrip() {
        let original = "# Original hosts\n127.0.0.1       localhost\n";
        let groups = vec![
            HostsGroup {
                name: "dev".to_string(),
                entries: vec![
                    active_line("192.168.1.100", "api.example.com"),
                    active_line("192.168.1.101", "db.example.com"),
                ],
            },
            HostsGroup {
                name: "test".to_string(),
                entries: vec![active_line("10.0.0.10", "api.example.com")],
            },
        ];

        let written = format!("{}\n{}", original.trim_end(), generate_hosts_block(&groups));

        // 新格式（含环境标题行）写入后应能被完整移除，不残留条目
        let cleaned = remove_section(&written);
        assert_eq!(cleaned.trim_end(), original.trim_end());

        // 管理区域内的标题行不会被解析成 hosts 条目
        let entries = parse_hosts(&generate_hosts_block(&groups));
        assert_eq!(entries.len(), 3);
        assert!(
            entries
                .iter()
                .all(|entry| entry.hostname.ends_with("example.com"))
        );
    }

    #[test]
    fn test_find_hostname_conflicts() {
        let groups = vec![
            HostsGroup {
                name: "dev".to_string(),
                entries: vec![
                    active_line("192.168.1.100", "api.example.com"),
                    // 已禁用条目不参与冲突检测
                    HostsLine {
                        ip: "192.168.1.200".to_string(),
                        hostname: "db.example.com".to_string(),
                        comment: None,
                        is_active: false,
                    },
                ],
            },
            HostsGroup {
                name: "test".to_string(),
                entries: vec![
                    active_line("10.0.0.10", "api.example.com"),
                    active_line("10.0.0.11", "db.example.com"),
                ],
            },
        ];

        let conflicts = find_hostname_conflicts(&groups);
        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].hostname, "api.example.com");
        assert!(conflicts[0].is_cross_environment);
        assert_eq!(
            conflicts[0].mappings,
            vec![
                HostnameMapping {
                    environment: "dev".to_string(),
                    ip: "192.168.1.100".to_string(),
                },
                HostnameMapping {
                    environment: "test".to_string(),
                    ip: "10.0.0.10".to_string(),
                },
            ]
        );
        assert!(conflicts[0].describe().contains("跨环境"));
    }

    #[test]
    fn test_find_hostname_conflicts_same_ip() {
        let groups = vec![
            HostsGroup {
                name: "dev".to_string(),
                entries: vec![active_line("127.0.0.1", "localhost")],
            },
            HostsGroup {
                name: "test".to_string(),
                entries: vec![active_line("127.0.0.1", "localhost")],
            },
        ];

        // 同一主机名映射到相同 IP 时不算冲突
        assert!(find_hostname_conflicts(&groups).is_empty());
    }

    #[test]
    fn test_find_hostname_conflicts_ignore_ascii_case() {
        let groups = vec![
            HostsGroup {
                name: "dev".to_string(),
                entries: vec![active_line("192.168.1.100", "API.example.com")],
            },
            HostsGroup {
                name: "test".to_string(),
                entries: vec![active_line("10.0.0.10", "api.example.com")],
            },
        ];

        // 主机名比较不区分大小写，展示时保留首次出现的写法
        let conflicts = find_hostname_conflicts(&groups);
        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].hostname, "API.example.com");
        assert!(conflicts[0].is_cross_environment);
    }

    #[test]
    fn test_find_hostname_conflicts_within_single_environment() {
        let groups = vec![HostsGroup {
            name: "dev".to_string(),
            entries: vec![
                active_line("192.168.1.100", "api.example.com"),
                active_line("192.168.1.101", "api.example.com"),
            ],
        }];

        let conflicts = find_hostname_conflicts(&groups);
        assert_eq!(conflicts.len(), 1);
        assert!(!conflicts[0].is_cross_environment);
        assert!(conflicts[0].describe().contains("同一环境内"));
    }

    #[test]
    fn test_find_hostname_conflicts_empty_and_single_group() {
        assert!(find_hostname_conflicts(&[]).is_empty());

        let groups = vec![HostsGroup {
            name: "dev".to_string(),
            entries: vec![active_line("192.168.1.100", "api.example.com")],
        }];
        assert!(find_hostname_conflicts(&groups).is_empty());
    }

    #[test]
    fn test_find_hostname_conflicts_three_environments() {
        // 故意乱序传入，验证映射结果按环境名排序
        let groups = vec![
            HostsGroup {
                name: "test".to_string(),
                entries: vec![active_line("10.0.0.11", "api.example.com")],
            },
            HostsGroup {
                name: "prod".to_string(),
                entries: vec![active_line("10.0.0.10", "api.example.com")],
            },
            HostsGroup {
                name: "dev".to_string(),
                entries: vec![active_line("192.168.1.100", "api.example.com")],
            },
        ];

        let conflicts = find_hostname_conflicts(&groups);
        assert_eq!(conflicts.len(), 1);
        assert_eq!(
            conflicts[0]
                .mappings
                .iter()
                .map(|mapping| mapping.environment.as_str())
                .collect::<Vec<_>>(),
            vec!["dev", "prod", "test"]
        );
    }

    #[test]
    fn test_write_hosts_file_replaces_content() -> Result<()> {
        let dir = temp_test_dir("hosts-write");
        std::fs::create_dir_all(&dir)?;
        let path = dir.join("hosts");
        std::fs::write(&path, "127.0.0.1 localhost\n")?;

        write_hosts_file(&path, "10.0.0.1 example.com\n")?;

        assert_eq!(std::fs::read_to_string(&path)?, "10.0.0.1 example.com\n");
        // 临时文件不残留
        assert_eq!(std::fs::read_dir(&dir)?.count(), 1);

        let _ = std::fs::remove_dir_all(&dir);
        Ok(())
    }

    #[test]
    fn test_write_hosts_file_creates_missing_file() -> Result<()> {
        let dir = temp_test_dir("hosts-create");
        std::fs::create_dir_all(&dir)?;
        let path = dir.join("hosts");

        write_hosts_file(&path, "# new hosts\n")?;

        assert_eq!(std::fs::read_to_string(&path)?, "# new hosts\n");

        let _ = std::fs::remove_dir_all(&dir);
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn test_write_hosts_file_keeps_permissions() -> Result<()> {
        use std::os::unix::fs::PermissionsExt;

        let dir = temp_test_dir("hosts-permissions");
        std::fs::create_dir_all(&dir)?;
        let path = dir.join("hosts");
        std::fs::write(&path, "old\n")?;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;

        write_hosts_file(&path, "new\n")?;

        let mode = std::fs::metadata(&path)?.permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "替换后应保留原文件权限");

        let _ = std::fs::remove_dir_all(&dir);
        Ok(())
    }

    #[test]
    fn test_format_entry_sanitizes_invalid_fields() {
        let entry = HostsLine {
            ip: "192.168.1.100".to_string(),
            hostname: "dev.api.com\n# <<< Tools Box END <<<".to_string(),
            comment: Some("备注\n第二行".to_string()),
            is_active: true,
        };

        let line = format_entry(&entry);
        assert_eq!(line.lines().count(), 1, "条目必须只占一行");
        assert!(!line.contains('\n'));
        assert!(line.contains("dev.api.com"));
        assert!(line.contains("备注 第二行"));
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

        let result = remove_section(content);
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

        let result = remove_section(content);
        assert!(result.contains("127.0.0.1"));
    }

    #[test]
    fn test_remove_tools_box_section_rejects_unpaired_markers() {
        // 缺少结束标记时不能删除标记之后的内容，否则会误删用户条目
        let missing_end = "# Original hosts\n127.0.0.1 localhost\n# >>> Tools Box START >>>\n192.168.1.100   dev.api.com\n";
        assert!(remove_tools_box_section(missing_end).is_err());

        // 多余的结束标记同样中止处理
        let extra_end = "# Original hosts\n127.0.0.1 localhost\n# <<< Tools Box END <<<\n";
        assert!(remove_tools_box_section(extra_end).is_err());

        // 标记顺序颠倒
        let reversed = "# <<< Tools Box END <<<\n# >>> Tools Box START >>>\n";
        assert!(remove_tools_box_section(reversed).is_err());

        // 开始标记重复
        let duplicated =
            "# >>> Tools Box START >>>\n# >>> Tools Box START >>>\n# <<< Tools Box END <<<\n";
        assert!(remove_tools_box_section(duplicated).is_err());
    }

    #[test]
    fn test_remove_tools_box_section_keeps_other_content() {
        let content = r#"127.0.0.1       localhost

# >>> Tools Box START >>>
# --- dev ---
192.168.1.100   dev.api.com
# <<< Tools Box END <<<

# >>> Tools Box START >>>
# --- test ---
10.0.0.10       test.api.com
# <<< Tools Box END <<<

10.0.0.1        other.com
"#;

        // 存在多个管理区域时应全部移除
        let result = remove_section(content);
        assert!(!result.contains("dev.api.com"));
        assert!(!result.contains("test.api.com"));
        assert!(result.contains("127.0.0.1"));
        assert!(result.contains("10.0.0.1"));
    }
}
