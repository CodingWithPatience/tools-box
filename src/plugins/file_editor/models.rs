//! 常用文件编辑器数据模型
//!
//! 定义自定义目录、常用文件条目、编辑器标签，以及文本编码与换行符模型。
//! 编码与换行符只做无损处理：不认识的编码拒绝编辑，换行符按主导风格写回。

use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// UTF-8 BOM 字节序列。
const UTF8_BOM: [u8; 3] = [0xEF, 0xBB, 0xBF];

/// 自定义目录（分组）
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileDirectory {
    /// 主键
    pub id: i64,
    /// 目录名称（唯一）
    pub name: String,
    /// 排序序号
    pub sort_order: i32,
    /// 创建时间
    pub created_at: String,
}

/// 常用文件条目
///
/// 只保存磁盘路径与显示名，不缓存文件内容，内容始终以磁盘为准。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FavoriteFile {
    /// 主键
    pub id: i64,
    /// 所属目录，`None` 表示未分组
    pub directory_id: Option<i64>,
    /// 磁盘绝对路径（唯一）
    pub path: String,
    /// 列表显示名，`None` 时取文件名
    pub alias: Option<String>,
    /// 排序序号
    pub sort_order: i32,
    /// 最近一次打开时间
    pub last_opened_at: Option<String>,
}

impl FavoriteFile {
    /// 列表显示名：优先使用自定义显示名，否则取路径文件名
    pub fn display_name(&self) -> String {
        match self.alias.as_deref() {
            Some(alias) if !alias.trim().is_empty() => alias.trim().to_string(),
            _ => Path::new(&self.path)
                .file_name()
                .map(|name| name.to_string_lossy().to_string())
                .unwrap_or_else(|| self.path.clone()),
        }
    }
}

/// 文本编码（只支持无损编辑的两种）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextEncoding {
    /// UTF-8 无 BOM
    Utf8,
    /// UTF-8 带 BOM，保存时写回 BOM
    Utf8Bom,
}

impl TextEncoding {
    /// 探测字节流编码；非 UTF-8 返回 `None`，调用方据此拒绝编辑
    pub fn detect(bytes: &[u8]) -> Option<Self> {
        let has_bom = bytes.starts_with(&UTF8_BOM);
        let body = if has_bom {
            &bytes[UTF8_BOM.len()..]
        } else {
            bytes
        };
        std::str::from_utf8(body).ok()?;

        Some(if has_bom { Self::Utf8Bom } else { Self::Utf8 })
    }

    /// 按编码把文本编码为字节序列（带 BOM 的编码会写回 BOM）
    pub fn encode(self, text: &str) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(text.len() + UTF8_BOM.len());
        if self == Self::Utf8Bom {
            bytes.extend_from_slice(&UTF8_BOM);
        }
        bytes.extend_from_slice(text.as_bytes());
        bytes
    }

    /// 状态栏显示名
    pub fn label(self) -> &'static str {
        match self {
            Self::Utf8 => "UTF-8",
            Self::Utf8Bom => "UTF-8 BOM",
        }
    }
}

/// 换行符风格
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineEnding {
    /// `\n`
    Lf,
    /// `\r\n`
    CrLf,
}

impl LineEnding {
    /// 探测主导换行符风格：CRLF 与 LF 混用时取数量多的一方
    pub fn detect(text: &str) -> Self {
        let (crlf, lf) = Self::count(text);
        if crlf > lf { Self::CrLf } else { Self::Lf }
    }

    /// 文本是否同时包含 CRLF 与 LF（保存时会被统一为主导风格）
    pub fn is_mixed(text: &str) -> bool {
        let (crlf, lf) = Self::count(text);
        crlf > 0 && lf > 0
    }

    /// 状态栏显示名
    pub fn label(self) -> &'static str {
        match self {
            Self::Lf => "LF",
            Self::CrLf => "CRLF",
        }
    }

    /// 把文本统一为指定换行符风格
    pub fn apply(self, text: &str) -> String {
        let unified = text.replace("\r\n", "\n").replace('\r', "\n");
        match self {
            Self::Lf => unified,
            Self::CrLf => unified.replace('\n', "\r\n"),
        }
    }

    /// 统计 CRLF 与独立 LF 的数量
    fn count(text: &str) -> (usize, usize) {
        let crlf = text.matches("\r\n").count();
        let lf = text.matches('\n').count().saturating_sub(crlf);
        (crlf, lf)
    }
}

/// 磁盘文件状态，用于外部修改检测
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DiskState {
    /// 最后修改时间
    pub modified: Option<SystemTime>,
    /// 文件长度
    pub len: u64,
}

/// 一个已打开的文件标签
#[derive(Debug, Clone)]
pub struct EditorTab {
    /// 对应列表条目 id，`None` 表示未纳入列表
    pub file_id: Option<i64>,
    /// 磁盘路径
    pub path: PathBuf,
    /// 标签显示名
    pub display_name: String,
    /// 编辑器内容（换行符已统一为 LF）
    pub text: String,
    /// 内容是否有未保存的改动
    pub dirty: bool,
    /// 载入或保存时记录的磁盘状态
    pub disk: DiskState,
    /// 原文件编码
    pub encoding: TextEncoding,
    /// 原文件换行符风格，保存时按此写回
    pub line_ending: LineEnding,
    /// 是否为只读（文件过大或读取失败）
    pub read_only: bool,
    /// 提示信息（读取失败、外部修改等）
    pub message: Option<String>,
}

impl EditorTab {
    /// 标签标题：脏标签追加未保存圆点
    pub fn title(&self) -> String {
        if self.dirty {
            format!("{} ●", self.display_name)
        } else {
            self.display_name.clone()
        }
    }
}

/// 关闭标签时的用户选择
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseDecision {
    /// 保存后关闭
    Save,
    /// 放弃修改并关闭
    Discard,
    /// 取消关闭
    Cancel,
}

/// 关闭脏标签是否需要弹窗确认
pub fn close_requires_prompt(dirty: bool) -> bool {
    dirty
}

/// 判断路径是否为系统 hosts 文件（应由 hosts 管理器负责，避免绕过其合并逻辑）
pub fn is_hosts_file(path: &Path) -> bool {
    let normalized = path
        .to_string_lossy()
        .replace('/', "\\")
        .to_ascii_lowercase();
    normalized.ends_with("\\drivers\\etc\\hosts")
}

/// 按路径扩展名推断语法高亮语言名，未知类型返回 `None`
pub fn highlight_language(path: &Path) -> Option<String> {
    // 无扩展名的常见配置文件（.gitconfig / .npmrc 等）按文件名识别
    let file_name = path
        .file_name()
        .map(|name| name.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    match file_name.as_str() {
        ".gitconfig" => return Some("INI".to_string()),
        ".npmrc" | ".editorconfig" | ".gitignore" => return Some("INI".to_string()),
        _ => {}
    }

    let extension = path.extension()?.to_string_lossy().to_ascii_lowercase();
    if extension.is_empty() {
        return None;
    }

    let language = match extension.as_str() {
        "toml" => "TOML",
        "json" => "JSON",
        "yaml" | "yml" => "YAML",
        "ini" | "conf" | "cfg" | "properties" => "INI",
        "md" => "Markdown",
        "rs" => "Rust",
        "js" => "JavaScript",
        "ts" => "TypeScript",
        "py" => "Python",
        "sh" | "bash" => "Bourne Again Shell (bash)",
        "bat" | "cmd" => "Batch File",
        "ps1" => "PowerShell",
        "xml" => "XML",
        "html" | "htm" => "HTML",
        "css" => "CSS",
        "sql" => "SQL",
        "c" | "h" => "C",
        "cpp" | "cc" | "hpp" => "C++",
        "go" => "Go",
        "java" => "Java",
        "lua" => "Lua",
        _ => extension.as_str(),
    };

    Some(language.to_string())
}

#[cfg(test)]
mod tests {
    use super::{
        EditorTab, FavoriteFile, LineEnding, TextEncoding, close_requires_prompt,
        highlight_language, is_hosts_file,
    };
    use std::path::{Path, PathBuf};
    use std::time::SystemTime;

    fn tab(dirty: bool) -> EditorTab {
        EditorTab {
            file_id: None,
            path: PathBuf::from("C:\\temp\\config.toml"),
            display_name: "config.toml".to_string(),
            text: "key = 1\n".to_string(),
            dirty,
            disk: super::DiskState {
                modified: Some(SystemTime::UNIX_EPOCH),
                len: 8,
            },
            encoding: TextEncoding::Utf8,
            line_ending: LineEnding::Lf,
            read_only: false,
            message: None,
        }
    }

    #[test]
    fn detects_utf8_with_and_without_bom() {
        assert_eq!(TextEncoding::detect(b"key = 1"), Some(TextEncoding::Utf8));
        assert_eq!(
            TextEncoding::detect(b"\xEF\xBB\xBFkey = 1"),
            Some(TextEncoding::Utf8Bom)
        );
    }

    #[test]
    fn rejects_non_utf8_content() {
        // GBK 编码的「测试」
        assert_eq!(TextEncoding::detect(&[0xB2, 0xE2, 0xCA, 0xD4]), None);
    }

    #[test]
    fn encodes_bom_only_for_bom_encoding() {
        let text = "a";
        assert_eq!(TextEncoding::Utf8.encode(text), b"a".to_vec());
        assert_eq!(
            TextEncoding::Utf8Bom.encode(text),
            vec![0xEF, 0xBB, 0xBF, b'a']
        );
    }

    #[test]
    fn detects_dominant_line_ending_and_mixed_content() {
        assert_eq!(LineEnding::detect("a\nb\nc"), LineEnding::Lf);
        assert_eq!(LineEnding::detect("a\r\nb\r\nc"), LineEnding::CrLf);
        assert_eq!(LineEnding::detect("a\r\nb\r\nc\nd"), LineEnding::CrLf);
        assert_eq!(LineEnding::detect("a\nb\nc\r\nd"), LineEnding::Lf);
        assert_eq!(LineEnding::detect("no newline"), LineEnding::Lf);

        assert!(!LineEnding::is_mixed("a\r\nb"));
        assert!(!LineEnding::is_mixed("a\nb"));
        assert!(LineEnding::is_mixed("a\r\nb\nc"));
    }

    #[test]
    fn applies_target_line_ending_without_double_conversion() {
        assert_eq!(LineEnding::CrLf.apply("a\nb\n"), "a\r\nb\r\n");
        assert_eq!(LineEnding::CrLf.apply("a\r\nb\r\n"), "a\r\nb\r\n");
        assert_eq!(LineEnding::Lf.apply("a\r\nb\r\n"), "a\nb\n");
        assert_eq!(LineEnding::Lf.apply("a\rb\r"), "a\nb\n");
    }

    #[test]
    fn display_name_prefers_alias_then_file_name() {
        let mut file = FavoriteFile {
            id: 1,
            directory_id: None,
            path: "C:\\Users\\me\\.gitconfig".to_string(),
            alias: None,
            sort_order: 0,
            last_opened_at: None,
        };
        assert_eq!(file.display_name(), ".gitconfig");

        file.alias = Some("git 配置".to_string());
        assert_eq!(file.display_name(), "git 配置");

        file.alias = Some("   ".to_string());
        assert_eq!(file.display_name(), ".gitconfig");
    }

    #[test]
    fn tab_title_marks_unsaved_changes() {
        assert_eq!(tab(false).title(), "config.toml");
        assert_eq!(tab(true).title(), "config.toml ●");
    }

    #[test]
    fn prompts_only_for_dirty_tabs() {
        assert!(!close_requires_prompt(false));
        assert!(close_requires_prompt(true));
    }

    #[test]
    fn recognizes_hosts_file_paths() {
        assert!(is_hosts_file(Path::new(
            "C:\\Windows\\System32\\drivers\\etc\\hosts"
        )));
        assert!(is_hosts_file(Path::new(
            "c:/windows/system32/drivers/etc/hosts"
        )));
        assert!(!is_hosts_file(Path::new("C:\\temp\\hosts")));
    }

    #[test]
    fn infers_language_from_extension_and_file_name() {
        assert_eq!(
            highlight_language(Path::new("C:\\a\\Cargo.toml")).as_deref(),
            Some("TOML")
        );
        assert_eq!(
            highlight_language(Path::new("C:\\Users\\me\\.gitconfig")).as_deref(),
            Some("INI")
        );
        assert_eq!(
            highlight_language(Path::new("C:\\a\\main.rs")).as_deref(),
            Some("Rust")
        );
        assert!(highlight_language(Path::new("C:\\a\\Makefile")).is_none());
    }
}
