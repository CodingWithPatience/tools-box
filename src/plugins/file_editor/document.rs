//! 常用文件编辑器的文件读写
//!
//! 负责编码与换行符探测、只读保护、原子保存以及外部修改检测。
//! 所有文本处理都是无损的：不认识的编码拒绝编辑，换行符按原文件主导风格写回。

use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use super::models::{DiskState, LineEnding, TextEncoding};

/// 单文件可编辑大小上限（字节）：超过后只读展示，避免同步 I/O 与界面卡顿。
pub const MAX_EDITABLE_BYTES: u64 = 2 * 1024 * 1024;

/// 保存用临时文件后缀（与源文件同目录，保证同卷 rename 的原子性）。
const TEMP_SUFFIX: &str = ".toolsbox-tmp";

/// 一次文件读取的结果
#[derive(Debug)]
pub struct LoadedFile {
    /// 编辑器内容（换行符已统一为 LF）
    pub text: String,
    /// 原文件编码
    pub encoding: TextEncoding,
    /// 原文件主导换行符
    pub line_ending: LineEnding,
    /// 原文件是否混用了 CRLF 与 LF
    pub mixed_line_endings: bool,
    /// 读取时的磁盘状态
    pub disk: DiskState,
    /// 是否只读（文件过大或内容不可无损编辑）
    pub read_only: bool,
    /// 附加提示（只读原因等）
    pub message: Option<String>,
}

/// 读取文件并转换为可编辑文本
///
/// 超过 [`MAX_EDITABLE_BYTES`] 的文件只读展示前 2 MB；非 UTF-8 文件直接报错。
pub fn load(path: &Path) -> Result<LoadedFile> {
    let metadata =
        fs::metadata(path).with_context(|| format!("无法读取文件信息: {}", path.display()))?;
    if metadata.is_dir() {
        bail!("路径是目录而不是文件: {}", path.display());
    }

    let disk = DiskState {
        modified: metadata.modified().ok(),
        len: metadata.len(),
    };

    if metadata.len() > MAX_EDITABLE_BYTES {
        // 只读取前 2 MB：大文件（例如 1 GB 日志）不能整体读入内存
        let file =
            fs::File::open(path).with_context(|| format!("打开文件失败: {}", path.display()))?;
        let mut buffer = Vec::new();
        file.take(MAX_EDITABLE_BYTES)
            .read_to_end(&mut buffer)
            .with_context(|| format!("读取文件失败: {}", path.display()))?;
        truncate_to_char_boundary(&mut buffer);

        let preview = String::from_utf8_lossy(&buffer).to_string();

        return Ok(LoadedFile {
            text: LineEnding::Lf.apply(&preview),
            // 只读预览不做编码与换行符判定，状态栏对只读标签显示「只读预览」
            encoding: TextEncoding::Utf8,
            line_ending: LineEnding::Lf,
            mixed_line_endings: false,
            disk,
            read_only: true,
            message: Some(format!(
                "文件大小 {} 超过 2 MB 上限，仅只读展示前 2 MB（未判定编码与换行符）",
                format_megabytes(metadata.len())
            )),
        });
    }

    let bytes = fs::read(path).with_context(|| format!("读取文件失败: {}", path.display()))?;
    let Some(encoding) = TextEncoding::detect(&bytes) else {
        bail!(
            "文件不是 UTF-8 编码，本插件不做有损转换: {}",
            path.display()
        );
    };

    let body = if encoding == TextEncoding::Utf8Bom {
        &bytes[3..]
    } else {
        &bytes[..]
    };
    let raw =
        std::str::from_utf8(body).with_context(|| format!("UTF-8 解码失败: {}", path.display()))?;

    Ok(LoadedFile {
        text: LineEnding::Lf.apply(raw),
        encoding,
        line_ending: LineEnding::detect(raw),
        mixed_line_endings: LineEnding::is_mixed(raw),
        disk,
        read_only: false,
        message: None,
    })
}

/// 原子保存：先写同目录临时文件，再用 rename 覆盖原文件
///
/// 保存失败时临时文件会被清理，原文件保持不变。
pub fn save(
    path: &Path,
    text: &str,
    encoding: TextEncoding,
    line_ending: LineEnding,
) -> Result<DiskState> {
    let metadata =
        fs::metadata(path).with_context(|| format!("无法读取文件信息: {}", path.display()))?;
    if metadata.permissions().readonly() {
        bail!("文件带只读属性，未执行保存: {}", path.display());
    }

    let content = line_ending.apply(text);
    let bytes = encoding.encode(&content);
    let temp = temp_path(path)?;

    {
        let mut file = fs::File::create(&temp)
            .with_context(|| format!("写入临时文件失败: {}", temp.display()))?;
        file.write_all(&bytes)
            .with_context(|| format!("写入临时文件失败: {}", temp.display()))?;
        // 先落盘再替换，降低掉电后原文件被截断的风险
        file.sync_all()
            .with_context(|| format!("刷新临时文件失败: {}", temp.display()))?;
    }

    if let Err(error) = fs::rename(&temp, path) {
        let _ = fs::remove_file(&temp);
        return Err(error).with_context(|| format!("替换原文件失败: {}", path.display()));
    }

    disk_state(path)
}

/// 以 MB 为单位格式化字节数
///
/// 使用整数运算，避免大整数转浮点的精度损失。
fn format_megabytes(bytes: u64) -> String {
    const MEBIBYTE: u64 = 1024 * 1024;
    let whole = bytes / MEBIBYTE;
    let tenths = (bytes % MEBIBYTE) * 10 / MEBIBYTE;

    format!("{whole}.{tenths} MB")
}

/// 把字节缓冲截断到 UTF-8 字符边界（丢弃末尾不完整的字符）
///
/// 只读预览按字节数截断时可能落在多字节字符中间，直接解码会产生替换字符；
/// 这里回退到最后一个字符的起始字节，只有该字符确实不完整时才丢弃它。
/// 全是续字节的异常数据不做处理，交给有损解码显示为替换字符。
fn truncate_to_char_boundary(bytes: &mut Vec<u8>) {
    let mut start = bytes.len();
    while start > 0 && (bytes[start - 1] & 0b1100_0000) == 0b1000_0000 {
        start -= 1;
    }
    if start == 0 {
        return;
    }

    let lead = bytes[start - 1];
    let expected = match lead {
        0x00..=0x7F => 1,
        0xC0..=0xDF => 2,
        0xE0..=0xEF => 3,
        0xF0..=0xF7 => 4,
        _ => 1,
    };
    let actual = bytes.len() - (start - 1);

    if actual < expected {
        bytes.truncate(start - 1);
    }
}

/// 读取当前磁盘状态
pub fn disk_state(path: &Path) -> Result<DiskState> {
    let metadata =
        fs::metadata(path).with_context(|| format!("无法读取文件信息: {}", path.display()))?;

    Ok(DiskState {
        modified: metadata.modified().ok(),
        len: metadata.len(),
    })
}

/// 磁盘状态是否已相对记录的基线发生变化
pub fn changed_on_disk(baseline: DiskState, latest: DiskState) -> bool {
    baseline.len != latest.len || baseline.modified != latest.modified
}

/// 临时文件路径：源文件同目录 + `.toolsbox-tmp` 后缀
pub fn temp_path(path: &Path) -> Result<PathBuf> {
    let file_name = path
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .with_context(|| format!("路径缺少文件名: {}", path.display()))?;

    Ok(path.with_file_name(format!("{file_name}{TEMP_SUFFIX}")))
}

#[cfg(test)]
mod tests {
    use super::{
        MAX_EDITABLE_BYTES, changed_on_disk, format_megabytes, load, save, temp_path,
        truncate_to_char_boundary,
    };
    use crate::plugins::file_editor::models::{DiskState, LineEnding, TextEncoding};
    use std::fs;
    use std::path::PathBuf;

    /// 生成唯一的临时文件路径
    fn temp_file(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "tools-box-file-editor-test-{}-{name}",
            std::process::id()
        ))
    }

    #[test]
    fn temp_path_keeps_directory_and_adds_suffix() {
        let path = PathBuf::from("C:\\temp\\config.toml");
        let temp = temp_path(&path).expect("应能推导临时文件路径");

        assert_eq!(temp, PathBuf::from("C:\\temp\\config.toml.toolsbox-tmp"));
    }

    #[test]
    fn detects_disk_state_changes() {
        let baseline = DiskState {
            modified: None,
            len: 10,
        };
        let same = DiskState {
            modified: None,
            len: 10,
        };
        let grown = DiskState {
            modified: None,
            len: 11,
        };

        assert!(!changed_on_disk(baseline, same));
        assert!(changed_on_disk(baseline, grown));
    }

    #[test]
    fn round_trip_preserves_bom_and_crlf() {
        let path = temp_file("bom-crlf.toml");
        let original = b"\xEF\xBB\xBFkey = 1\r\nother = 2\r\n";
        fs::write(&path, original).expect("写入测试文件失败");

        let loaded = load(&path).expect("读取测试文件失败");
        assert_eq!(loaded.encoding, TextEncoding::Utf8Bom);
        assert_eq!(loaded.line_ending, LineEnding::CrLf);
        assert!(!loaded.mixed_line_endings);
        assert!(!loaded.read_only);
        assert_eq!(loaded.text, "key = 1\nother = 2\n");

        // 编辑后保存：BOM 与 CRLF 必须原样写回
        let edited = "key = 42\nother = 2\n";
        let saved = save(&path, edited, loaded.encoding, loaded.line_ending).expect("保存失败");
        let raw = fs::read(&path).expect("读取保存结果失败");
        assert_eq!(raw, b"\xEF\xBB\xBFkey = 42\r\nother = 2\r\n");
        assert_eq!(saved.len, raw.len() as u64);

        let _ = fs::remove_file(&path);
    }

    #[test]
    fn rejects_non_utf8_file() {
        let path = temp_file("gbk.conf");
        // GBK 编码的「测试」
        fs::write(&path, [0xB2, 0xE2, 0xCA, 0xD4]).expect("写入测试文件失败");

        let error = load(&path).expect_err("非 UTF-8 文件应被拒绝");
        assert!(error.to_string().contains("UTF-8"));

        let _ = fs::remove_file(&path);
    }

    #[test]
    fn oversized_file_is_read_only() {
        let path = temp_file("huge.txt");
        let limit = usize::try_from(MAX_EDITABLE_BYTES).expect("上限应能转换为 usize");
        let content = vec![b'a'; limit + 16];
        fs::write(&path, &content).expect("写入测试文件失败");

        let loaded = load(&path).expect("读取超大文件失败");
        assert!(loaded.read_only);
        assert!(
            loaded
                .message
                .as_deref()
                .is_some_and(|message| message.contains("只读展示")),
            "应提示只读原因"
        );
        assert_eq!(loaded.text.len(), limit, "只读预览必须截断到上限字节数");

        let _ = fs::remove_file(&path);
    }

    #[test]
    fn truncates_incomplete_trailing_character() {
        // 完整汉字「中」= E4 B8 AD
        let mut complete = vec![0xE4, 0xB8, 0xAD];
        truncate_to_char_boundary(&mut complete);
        assert_eq!(complete, vec![0xE4, 0xB8, 0xAD], "完整字符不应被截断");

        let mut half = vec![0xE4, 0xB8];
        truncate_to_char_boundary(&mut half);
        assert!(half.is_empty(), "缺少后续字节的起始字节应被丢弃");

        let mut lead_only = vec![0xE4];
        truncate_to_char_boundary(&mut lead_only);
        assert!(lead_only.is_empty(), "只有起始字节时应被丢弃");

        let mut ascii_then_half = b"ab".to_vec();
        ascii_then_half.extend_from_slice(&[0xE4, 0xB8]);
        truncate_to_char_boundary(&mut ascii_then_half);
        assert_eq!(ascii_then_half, b"ab".to_vec(), "ASCII 内容应保持不变");
    }

    #[test]
    fn formats_megabytes_without_float_conversion() {
        const MEBIBYTE: u64 = 1024 * 1024;

        assert_eq!(format_megabytes(0), "0.0 MB");
        assert_eq!(format_megabytes(MEBIBYTE), "1.0 MB");
        assert_eq!(format_megabytes(MEBIBYTE + MEBIBYTE / 2), "1.5 MB");
        assert_eq!(format_megabytes(2 * MEBIBYTE + 16), "2.0 MB");
    }

    #[test]
    // 测试清理：Windows 下删除只读文件前需要清掉只读属性
    #[allow(clippy::permissions_set_readonly_false)]
    fn refuses_to_save_read_only_file() {
        let path = temp_file("readonly.conf");
        fs::write(&path, "key = 1\n").expect("写入测试文件失败");
        let mut permissions = fs::metadata(&path).expect("读取属性失败").permissions();
        permissions.set_readonly(true);
        fs::set_permissions(&path, permissions).expect("设置只读属性失败");

        let error = save(&path, "key = 2\n", TextEncoding::Utf8, LineEnding::Lf)
            .expect_err("只读文件不应被保存");
        assert!(error.to_string().contains("只读"));
        assert_eq!(
            fs::read_to_string(&path).expect("读取原文件失败"),
            "key = 1\n",
            "拒绝保存时原文件必须保持不变"
        );

        let mut permissions = fs::metadata(&path).expect("读取属性失败").permissions();
        permissions.set_readonly(false);
        let _ = fs::set_permissions(&path, permissions);
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn missing_file_reports_error() {
        let path = temp_file("missing-not-exists.conf");
        let _ = fs::remove_file(&path);

        assert!(load(&path).is_err());
    }
}
