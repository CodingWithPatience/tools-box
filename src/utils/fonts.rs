//! 系统字体枚举与全局字体应用
//!
//! 字体清单来自注册表（字体名 → 字体文件），是否等宽由**字体文件自身**的
//! `post` / `OS/2` 表判定：只读文件头的少量字节，不依赖 GDI，也能正确处理 `.ttc` 字体集合。
//! 解析逻辑拆成纯函数（`(Read + Seek)` 入参），可用 `std::io::Cursor` 单元测试。

use std::collections::BTreeSet;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use windows_sys::Win32::Foundation::ERROR_SUCCESS;
use windows_sys::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, REG_EXPAND_SZ, REG_SZ, RegCloseKey,
    RegEnumValueW, RegOpenKeyExW, RegQueryValueExW,
};

/// 注册表中登记系统字体的键（每个 hive 一份）
const FONTS_REGISTRY_KEY: &str = "SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion\\Fonts";
/// 系统字体目录的默认位置（`%SystemRoot%` 取不到时回退）
const DEFAULT_SYSTEM_FONT_DIR: &str = r"C:\Windows\Fonts";
/// `post` 表标签（"post" 的大端表示）
const POST_TABLE_TAG: u32 = 0x706F_7374;
/// `OS/2` 表标签（"OS/2" 的大端表示）
const OS2_TABLE_TAG: u32 = 0x4F53_2F32;
/// Panose 中"比例"字段取值：9 表示等宽
const PANOSE_MONOSPACED: u8 = 9;

/// 一个可选的系统字体
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SystemFont {
    /// 字体名（注册表值名去掉后缀与别名，用于显示与写进 `FontDefinitions`）
    pub name: String,
    /// 字体文件路径
    pub path: PathBuf,
    /// 是否等宽（代码字体只允许等宽字体）
    pub monospace: bool,
}

/// 字体版本号：每次应用全局字体时自增
///
/// 各插件的排版缓存（`LayoutJob` / 字形度量）都以它为一个键：
/// 只按"文本 + 字号 + 主题"缓存的话，换字体会继续命中旧字体的排版结果。
static FONT_REVISION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// 当前字体版本号
pub fn font_revision() -> u64 {
    FONT_REVISION.load(std::sync::atomic::Ordering::Acquire)
}

/// 一次枚举解析界面字体与代码字体
///
/// 枚举要读注册表并逐个读字体文件头（本机 385 个），因此两侧只枚举一次；
/// 两者都未选择时**不枚举**（默认配置启动零开销）。
pub fn resolve_selected_fonts(
    ui_font: Option<&str>,
    code_font: Option<&str>,
) -> (Option<SystemFont>, Option<SystemFont>) {
    if ui_font.is_none() && code_font.is_none() {
        return (None, None);
    }

    let fonts = enumerate_system_fonts();
    let find = |name: Option<&str>| -> Option<SystemFont> {
        name.and_then(|name| fonts.iter().find(|font| font.name == name).cloned())
    };

    (find(ui_font), find(code_font))
}

/// 枚举系统已安装字体（当前用户 + 本机），按名称排序
///
/// 读不到注册表或字体文件时跳过该条目并记日志，不影响其它字体。
pub fn enumerate_system_fonts() -> Vec<SystemFont> {
    let mut fonts: Vec<SystemFont> = Vec::new();
    let mut seen: BTreeSet<String> = BTreeSet::new();

    for hive in [HKEY_LOCAL_MACHINE, HKEY_CURRENT_USER] {
        for (value_name, file_name) in read_font_registry_entries(hive) {
            let Some(name) = parse_registry_font_name(&value_name) else {
                continue;
            };
            if !seen.insert(name.clone()) {
                continue;
            }

            let Some(path) = resolve_font_path(&file_name) else {
                log::debug!("字体文件不存在，跳过: {file_name}（{name}）");
                continue;
            };
            let monospace = file_is_monospace(&path).unwrap_or(false);

            fonts.push(SystemFont {
                name,
                path,
                monospace,
            });
        }
    }

    fonts.sort_by(|left, right| left.name.cmp(&right.name));
    log::info!("已枚举系统字体 {} 个", fonts.len());

    fonts
}

/// 解析注册表字体值名 → 字体名
///
/// 值名形如 `Consolas (TrueType)`、`Microsoft YaHei & Microsoft YaHei UI (TrueType)`：
/// 去掉括号后缀，并在 ` & ` 处只保留**第一个**名字——`.ttc` 集合按注册表顺序取第 0 个 face，
/// 保留别名会导致选中别名却加载到第一个 face。
fn parse_registry_font_name(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }

    // 去掉结尾的 " (TrueType)" / " (OpenType)" / " (All res)" 等括号说明
    let without_suffix = match trimmed.rfind(" (") {
        Some(index) if trimmed.ends_with(')') => &trimmed[..index],
        _ => trimmed,
    };

    let primary = match without_suffix.find(" & ") {
        Some(index) => &without_suffix[..index],
        None => without_suffix,
    };

    let name = primary.trim();
    if name.is_empty() {
        None
    } else {
        Some(name.to_owned())
    }
}

/// 系统字体目录
///
/// 按 `%SystemRoot%` 解析（Windows 装在非 C 盘时也能找到），取不到时回退默认路径。
fn system_font_dir() -> PathBuf {
    std::env::var("SystemRoot")
        .map(|root| Path::new(&root).join("Fonts"))
        .unwrap_or_else(|_| PathBuf::from(DEFAULT_SYSTEM_FONT_DIR))
}

/// 字体文件名 → 绝对路径
///
/// 注册表数据通常是文件名（如 `consola.ttf`），少数安装器写绝对路径。
fn resolve_font_path(file_name: &str) -> Option<PathBuf> {
    let value = file_name.trim();
    if value.is_empty() {
        return None;
    }

    let candidate = Path::new(value);
    if candidate.is_absolute() {
        return candidate.exists().then(|| candidate.to_path_buf());
    }

    let system_dir = system_font_dir().join(value);
    if system_dir.exists() {
        return Some(system_dir);
    }

    // 用户字体目录（Win10 1809+）：%LOCALAPPDATA%\Microsoft\Windows\Fonts
    if let Ok(local_app_data) = std::env::var("LOCALAPPDATA") {
        let user_dir = Path::new(&local_app_data)
            .join("Microsoft\\Windows\\Fonts")
            .join(value);
        if user_dir.exists() {
            return Some(user_dir);
        }
    }

    None
}

/// 读取字体文件是否等宽
///
/// 优先 `post` 表的 `isFixedPitch`，缺失或为 0 时再看 `OS/2` 的 Panose 比例字段
/// （不少等宽字体把 `isFixedPitch` 留为 0）。
pub fn file_is_monospace(path: &Path) -> Option<bool> {
    let mut file = File::open(path).ok()?;

    is_monospace(&mut file)
}

/// 从字体字节流判断是否等宽（`Read + Seek` 便于用 `Cursor` 测试）
fn is_monospace(reader: &mut (impl Read + Seek)) -> Option<bool> {
    let base = sfnt_offset(reader)?;
    let post = find_table_offset(reader, base, POST_TABLE_TAG);

    if let Some(post_offset) = post {
        reader.seek(SeekFrom::Start(post_offset + 12)).ok()?;
        let fixed_pitch = read_u32(reader)?;
        if fixed_pitch != 0 {
            return Some(true);
        }
    }

    if let Some(os2_offset) = find_table_offset(reader, base, OS2_TABLE_TAG) {
        // OS/2 表偏移 32 起是 10 字节 Panose，其中第 4 个字节（+35）是比例字段
        reader.seek(SeekFrom::Start(os2_offset + 35)).ok()?;
        let mut panose_proportion = [0_u8; 1];
        reader.read_exact(&mut panose_proportion).ok()?;
        if panose_proportion[0] == PANOSE_MONOSPACED {
            return Some(true);
        }
    }

    // 两个表都读过但都没标记等宽 ⇒ 判定为非等宽；都读不到则无法判定
    if post.is_some() { Some(false) } else { None }
}

/// 取第一个字体表的起始偏移：`.ttc` 集合取第 0 个 face，普通字体为 0
fn sfnt_offset(reader: &mut (impl Read + Seek)) -> Option<u64> {
    reader.seek(SeekFrom::Start(0)).ok()?;
    let mut header = [0_u8; 4];
    reader.read_exact(&mut header).ok()?;

    if &header == b"ttcf" {
        // TTC 头：tag(4) version(4) numFonts(4) offsetTable[numFonts]
        // 即 numFonts 位于 8..12，第 0 个 face 偏移位于 12..16
        reader.seek(SeekFrom::Start(8)).ok()?;
        let num_fonts = read_u32(reader)?;
        if num_fonts == 0 {
            return None;
        }

        u64::from(read_u32(reader)?).into()
    } else {
        Some(0)
    }
}

/// 在字体表目录里查标签对应的表偏移
fn find_table_offset(reader: &mut (impl Read + Seek), base: u64, tag: u32) -> Option<u64> {
    reader.seek(SeekFrom::Start(base + 4)).ok()?;
    let num_tables = u32::from(read_u16(reader)?);

    for index in 0..num_tables {
        let record_offset = base + 12 + u64::from(index) * 16;
        reader.seek(SeekFrom::Start(record_offset)).ok()?;
        let record_tag = read_u32(reader)?;
        if record_tag == tag {
            reader.seek(SeekFrom::Start(record_offset + 8)).ok()?;
            return Some(u64::from(read_u32(reader)?));
        }
    }

    None
}

/// 读大端 `u16`
fn read_u16(reader: &mut impl Read) -> Option<u16> {
    let mut buffer = [0_u8; 2];
    reader.read_exact(&mut buffer).ok()?;

    Some(u16::from_be_bytes(buffer))
}

/// 读大端 `u32`
fn read_u32(reader: &mut impl Read) -> Option<u32> {
    let mut buffer = [0_u8; 4];
    reader.read_exact(&mut buffer).ok()?;

    Some(u32::from_be_bytes(buffer))
}

/// 读取注册表字体键的 `(值名, 文件名)` 列表
fn read_font_registry_entries(hive: HKEY) -> Vec<(String, String)> {
    let mut entries = Vec::new();
    let key_path: Vec<u16> = FONTS_REGISTRY_KEY
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();

    // SAFETY: 键路径以空字符结尾且在调用期间有效；句柄仅在成功打开后使用并在结束前关闭。
    unsafe {
        let mut hkey: HKEY = std::ptr::null_mut();
        let result = RegOpenKeyExW(hive, key_path.as_ptr(), 0, KEY_READ, &mut hkey);
        if result != ERROR_SUCCESS {
            log::debug!("无法打开字体注册表键: {}", result);
            return entries;
        }

        let mut index = 0_u32;
        loop {
            let mut name_buffer = [0_u16; 512];
            let mut name_len = name_buffer.len() as u32;

            // SAFETY: 缓冲区与长度在调用期间有效；不需要类型与数据，传空指针。
            let enum_result = RegEnumValueW(
                hkey,
                index,
                name_buffer.as_mut_ptr(),
                &mut name_len,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            );

            if enum_result != ERROR_SUCCESS {
                break;
            }

            // SAFETY: 值名以空字符结尾（`name_len` 不含结尾空字符）且在调用期间有效。
            if let Some(value_name) = utf16_to_string(&name_buffer[..name_len as usize]) {
                let terminated: Vec<u16> = value_name
                    .encode_utf16()
                    .chain(std::iter::once(0))
                    .collect();
                if let Some(file_name) = read_font_registry_value_data(hkey, &terminated) {
                    entries.push((value_name, file_name));
                }
            }

            index += 1;
        }

        RegCloseKey(hkey);
    }

    entries
}

/// 读取注册表字体值的文件名字符串
///
/// `RegEnumValueW` 只返回值名，值数据需要再查一次；本函数把两者合并成
/// `(字体名, 文件名)`，供枚举时直接解析路径。
fn read_font_registry_value_data(hkey: HKEY, value_name: &[u16]) -> Option<String> {
    // SAFETY: 值名以空字符结尾且在调用期间有效；缓冲区大小按需查询后分配。
    unsafe {
        let mut value_type = 0_u32;
        let mut data_len = 0_u32;
        let probe = RegQueryValueExW(
            hkey,
            value_name.as_ptr(),
            std::ptr::null_mut(),
            &mut value_type,
            std::ptr::null_mut(),
            &mut data_len,
        );
        if probe != ERROR_SUCCESS || data_len == 0 {
            return None;
        }

        // 接受 REG_EXPAND_SZ 但不展开其中的环境变量：字体登记几乎总是纯文件名或绝对路径，
        // 若将来遇到 `%SystemRoot%\Fonts\x.ttf` 形式的登记，这里需要补 ExpandEnvironmentStringsW。
        if value_type != REG_SZ && value_type != REG_EXPAND_SZ {
            return None;
        }

        let mut buffer = vec![0_u8; data_len as usize];
        let query = RegQueryValueExW(
            hkey,
            value_name.as_ptr(),
            std::ptr::null_mut(),
            &mut value_type,
            buffer.as_mut_ptr(),
            &mut data_len,
        );
        if query != ERROR_SUCCESS {
            return None;
        }

        let utf16: Vec<u16> = buffer
            .chunks_exact(2)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .collect();

        utf16_to_string(&utf16)
    }
}

/// UTF-16 切片 → `String`（去掉结尾空字符）
fn utf16_to_string(utf16: &[u16]) -> Option<String> {
    let end = utf16
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(utf16.len());
    if end == 0 {
        return None;
    }

    Some(String::from_utf16_lossy(&utf16[..end]))
}

/// 把选中的字体应用到 egui
///
/// 保持既有行为：微软雅黑作为中文字形来源、Segoe UI Emoji 作为 Emoji 来源；
/// 用户选择的字体插到对应 family 的**首位**，缺字时自动回退到后面的中文字体。
pub fn setup_fonts(
    ctx: &egui::Context,
    ui_font: Option<&SystemFont>,
    code_font: Option<&SystemFont>,
) {
    let fonts = build_font_definitions(ui_font, code_font);
    ctx.set_fonts(fonts);

    // 版本号自增：插件的排版缓存据此失效
    let revision = FONT_REVISION.fetch_add(1, std::sync::atomic::Ordering::AcqRel) + 1;
    log::info!("全局字体已应用（版本 {revision}）");
}

/// 构建字体定义：中文字体与 Emoji 作为兜底，用户字体插到对应 family 首位
///
/// 单独拆出来便于测试"用户字体在首位、兜底字体仍在"这一装配顺序。
pub fn build_font_definitions(
    ui_font: Option<&SystemFont>,
    code_font: Option<&SystemFont>,
) -> egui::FontDefinitions {
    let mut fonts = egui::FontDefinitions::default();

    let chinese_paths = [
        r"C:\Windows\Fonts\msyh.ttc",
        r"C:\Windows\Fonts\msyhbd.ttc",
        r"C:\Windows\Fonts\simhei.ttf",
        r"C:\Windows\Fonts\simsun.ttc",
    ];
    if let Some(path) = load_first_available(&chinese_paths, &mut fonts, "chinese") {
        if let Some(family) = fonts.families.get_mut(&egui::FontFamily::Proportional) {
            family.insert(0, "chinese".to_owned());
        }
        if let Some(family) = fonts.families.get_mut(&egui::FontFamily::Monospace) {
            family.push("chinese".to_owned());
        }
        log::info!("已加载中文字体: {path}");
    } else {
        log::warn!("未找到中文字体，中文可能显示为乱码");
    }

    let emoji_paths = [
        r"C:\Windows\Fonts\seguiemj.ttf",
        r"C:\Windows\Fonts\seguisym.ttf",
    ];
    if let Some(path) = load_first_available(&emoji_paths, &mut fonts, "emoji") {
        for family_key in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
            if let Some(family) = fonts.families.get_mut(&family_key) {
                family.push("emoji".to_owned());
            }
        }
        log::info!("已加载 Emoji 字体: {path}");
    } else {
        log::warn!("未找到 Emoji 字体，部分图标可能显示为方框");
    }

    if let Some(font) = ui_font {
        if load_font_file(font, &mut fonts, "user_ui") {
            if let Some(family) = fonts.families.get_mut(&egui::FontFamily::Proportional) {
                family.insert(0, "user_ui".to_owned());
            }
            log::info!("界面字体已设置为: {}（{}）", font.name, font.path.display());
        } else {
            log::warn!("界面字体加载失败，回退默认字体: {}", font.path.display());
        }
    }

    if let Some(font) = code_font {
        if load_font_file(font, &mut fonts, "user_code") {
            if let Some(family) = fonts.families.get_mut(&egui::FontFamily::Monospace) {
                family.insert(0, "user_code".to_owned());
            }
            log::info!("代码字体已设置为: {}（{}）", font.name, font.path.display());
        } else {
            log::warn!("代码字体加载失败，回退默认字体: {}", font.path.display());
        }
    }

    fonts
}

/// 按顺序尝试加载字体文件，成功后以 `key` 存入 `fonts.font_data`
fn load_first_available(
    paths: &[&str],
    fonts: &mut egui::FontDefinitions,
    key: &str,
) -> Option<String> {
    for path in paths {
        if let Ok(data) = std::fs::read(path) {
            fonts
                .font_data
                .insert(key.to_owned(), egui::FontData::from_owned(data).into());

            return Some((*path).to_owned());
        }
    }

    None
}

/// 加载一个系统字体到 `fonts.font_data`
fn load_font_file(font: &SystemFont, fonts: &mut egui::FontDefinitions, key: &str) -> bool {
    match std::fs::read(&font.path) {
        Ok(data) => {
            fonts
                .font_data
                .insert(key.to_owned(), egui::FontData::from_owned(data).into());

            true
        }
        Err(error) => {
            log::warn!("读取字体文件失败 {}: {error}", font.path.display());

            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    /// 构造最小可解析的字体字节：表目录里放一个 `post` 表
    fn font_bytes_with_post(is_fixed_pitch: u32) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&0x0001_0000_u32.to_be_bytes()); // sfntVersion
        bytes.extend_from_slice(&1_u16.to_be_bytes()); // numTables
        bytes.extend_from_slice(&[0; 6]); // searchRange/entrySelector/rangeShift
        // 表记录：tag, checksum, offset, length
        let post_offset = 12 + 16;
        bytes.extend_from_slice(&POST_TABLE_TAG.to_be_bytes());
        bytes.extend_from_slice(&0_u32.to_be_bytes());
        bytes.extend_from_slice(&(post_offset as u32).to_be_bytes());
        bytes.extend_from_slice(&32_u32.to_be_bytes());
        // post 表头（version 3.0：前 32 字节布局固定）
        bytes.extend_from_slice(&0x0003_0000_u32.to_be_bytes());
        bytes.extend_from_slice(&0_u32.to_be_bytes()); // italicAngle
        bytes.extend_from_slice(&0_u16.to_be_bytes()); // underlinePosition
        bytes.extend_from_slice(&0_u16.to_be_bytes()); // underlineThickness
        bytes.extend_from_slice(&is_fixed_pitch.to_be_bytes());

        bytes
    }

    #[test]
    fn parses_registry_font_names() {
        assert_eq!(
            parse_registry_font_name("Consolas (TrueType)"),
            Some("Consolas".to_owned())
        );
        assert_eq!(
            parse_registry_font_name("Microsoft YaHei & Microsoft YaHei UI (TrueType)"),
            Some("Microsoft YaHei".to_owned()),
            "别名只保留第一个，与 .ttc 第 0 个 face 对应"
        );
        assert_eq!(
            parse_registry_font_name("  等线 (TrueType)  "),
            Some("等线".to_owned())
        );
        assert_eq!(parse_registry_font_name(""), None);
        assert_eq!(parse_registry_font_name("   "), None);
    }

    #[test]
    fn detects_monospace_from_post_table() {
        let monospace = font_bytes_with_post(1);
        assert_eq!(is_monospace(&mut Cursor::new(monospace)), Some(true));

        let proportional = font_bytes_with_post(0);
        assert_eq!(is_monospace(&mut Cursor::new(proportional)), Some(false));
    }

    #[test]
    fn detects_monospace_from_ttc_collection() {
        // TTC 头：tag(4) + version(4) + numFonts(4) + offset[1](4) = 16 字节，face 紧随其后。
        // 注意 TTC 中**表偏移是相对文件起点**的绝对值，构造测试数据时必须把
        // face 内的 `post` 表偏移字段改写成绝对偏移（真实字体文件就是这个约定）。
        const FACE_OFFSET: u32 = 16;
        const POST_RECORD_OFFSET_IN_FACE: usize = 12 + 8;
        const POST_OFFSET_IN_FACE: u32 = 12 + 16;

        let mut face = font_bytes_with_post(1);
        let absolute_post_offset = FACE_OFFSET + POST_OFFSET_IN_FACE;
        face[POST_RECORD_OFFSET_IN_FACE..POST_RECORD_OFFSET_IN_FACE + 4]
            .copy_from_slice(&absolute_post_offset.to_be_bytes());

        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"ttcf");
        bytes.extend_from_slice(&0x0001_0000_u32.to_be_bytes());
        bytes.extend_from_slice(&1_u32.to_be_bytes());
        bytes.extend_from_slice(&FACE_OFFSET.to_be_bytes());
        bytes.extend_from_slice(&face);

        let mut cursor = Cursor::new(bytes.clone());
        let base = sfnt_offset(&mut cursor);
        let post = base.and_then(|base| find_table_offset(&mut cursor, base, POST_TABLE_TAG));

        let result = is_monospace(&mut Cursor::new(bytes));
        assert_eq!(
            result,
            Some(true),
            "TTC 解析结果应为等宽，实际 {result:?}（base={base:?}, post={post:?}）"
        );
    }

    #[test]
    fn returns_none_when_no_usable_table() {
        // 空数据 / 只有 sfnt 头但没有表：无法判定
        assert_eq!(is_monospace(&mut Cursor::new(Vec::new())), None);
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&0x0001_0000_u32.to_be_bytes());
        bytes.extend_from_slice(&0_u16.to_be_bytes());
        bytes.extend_from_slice(&[0; 6]);
        assert_eq!(is_monospace(&mut Cursor::new(bytes)), None);
    }

    #[test]
    fn classifies_installed_system_fonts() {
        // 真实环境抽样：Consolas 等宽、Times New Roman 非等宽（不存在则跳过该断言）
        let consolas = system_font_dir().join("consola.ttf");
        if consolas.exists() {
            assert_eq!(
                file_is_monospace(&consolas),
                Some(true),
                "Consolas 应判定为等宽"
            );
        }

        let times = system_font_dir().join("times.ttf");
        if times.exists() {
            assert_eq!(
                file_is_monospace(&times),
                Some(false),
                "Times New Roman 应判定为非等宽"
            );
        }
    }

    #[test]
    fn resolves_font_paths() {
        let consolas = resolve_font_path("consola.ttf");
        if system_font_dir().join("consola.ttf").exists() {
            assert!(consolas.is_some(), "应在系统字体目录找到 consola.ttf");
        }
        assert_eq!(resolve_font_path(""), None);
        assert_eq!(resolve_font_path("definitely-missing-font-file.ttf"), None);
    }

    #[test]
    fn user_fonts_come_first_and_fallbacks_are_kept() {
        let candidates = [
            system_font_dir().join("consola.ttf"),
            system_font_dir().join("msyh.ttc"),
        ];
        let Some(path) = candidates.into_iter().find(|path| path.exists()) else {
            // 非 Windows 或字体缺失环境：无法构造真实字体数据，跳过
            return;
        };

        let ui_font = SystemFont {
            name: "测试界面字体".to_owned(),
            path: path.clone(),
            monospace: false,
        };
        let code_font = SystemFont {
            name: "测试代码字体".to_owned(),
            path,
            monospace: true,
        };

        let fonts = build_font_definitions(Some(&ui_font), Some(&code_font));
        let proportional = fonts
            .families
            .get(&egui::FontFamily::Proportional)
            .expect("应有 Proportional family");
        let monospace = fonts
            .families
            .get(&egui::FontFamily::Monospace)
            .expect("应有 Monospace family");

        assert_eq!(
            proportional.first().map(String::as_str),
            Some("user_ui"),
            "界面字体必须在 Proportional 首位（缺字时才回退到后面的字体）"
        );
        assert_eq!(
            monospace.first().map(String::as_str),
            Some("user_code"),
            "代码字体必须在 Monospace 首位"
        );
        assert!(
            proportional.contains(&"emoji".to_owned())
                || !system_font_dir().join("seguiemj.ttf").exists(),
            "本机存在 Emoji 字体时必须保留其兜底位置"
        );
        assert!(
            monospace.len() >= 2,
            "Monospace 除用户字体外还应保留兜底字体，实际 {monospace:?}"
        );
    }

    #[test]
    fn applying_fonts_changes_layout_metrics() {
        // 至少一种本机字体可用即可（Consolas 拉丁 / 雅黑 中文），否则跳过
        let candidates: Vec<PathBuf> = [
            system_font_dir().join("consola.ttf"),
            system_font_dir().join("msyh.ttc"),
        ]
        .into_iter()
        .filter(|path| path.exists())
        .collect();
        if candidates.is_empty() {
            return;
        }

        // 字体在下一帧开头生效，因此连续跑两帧后取度量
        let measure = |font: Option<&SystemFont>| -> f32 {
            let ctx = egui::Context::default();
            setup_fonts(&ctx, None, font);

            let mut width = 0.0;
            for _ in 0..2 {
                let input = egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(400.0, 200.0),
                    )),
                    ..Default::default()
                };
                let _ = ctx.run(input, |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        width = ui
                            .fonts(|fonts| {
                                fonts.layout_no_wrap(
                                    "MMMM00".to_owned(),
                                    egui::FontId::monospace(14.0),
                                    egui::Color32::WHITE,
                                )
                            })
                            .size()
                            .x;
                    });
                });
            }

            width
        };

        let default_width = measure(None);
        assert!(default_width > 0.0, "默认字体应能排版出非零宽度");

        let differs = candidates.iter().any(|path| {
            let font = SystemFont {
                name: "测试代码字体".to_owned(),
                path: path.clone(),
                monospace: true,
            };

            measure(Some(&font)) != default_width
        });
        assert!(
            differs,
            "应用自定义代码字体后等宽文本宽度应发生变化（证明字体真的生效，而不只是写进定义）"
        );
    }

    #[test]
    fn resolves_selected_fonts_once_and_skips_when_empty() {
        // 两者都未选择：不枚举、直接返回 None（默认配置启动零开销）
        let (ui_font, code_font) = resolve_selected_fonts(None, None);
        assert!(ui_font.is_none() && code_font.is_none());

        // 名称不存在：同样返回 None（不 panic）
        let (ui_font, code_font) =
            resolve_selected_fonts(Some("绝不存在的字体名"), Some("绝不存在的字体名"));
        assert!(ui_font.is_none() && code_font.is_none());

        // 本机存在 Consolas 时应能解析出来，且两个名称指向同一字体文件
        if system_font_dir().join("consola.ttf").exists() {
            let (ui_font, code_font) = resolve_selected_fonts(Some("Consolas"), Some("Consolas"));
            assert_eq!(ui_font.map(|font| font.name), Some("Consolas".to_owned()));
            assert_eq!(code_font.map(|font| font.name), Some("Consolas".to_owned()));
        }
    }

    #[test]
    fn missing_font_file_falls_back_to_default_family() {
        let broken = SystemFont {
            name: "不存在字体".to_owned(),
            path: PathBuf::from(r"C:\Windows\Fonts\definitely-missing-font.ttf"),
            monospace: true,
        };

        let fonts = build_font_definitions(Some(&broken), Some(&broken));
        let proportional = fonts
            .families
            .get(&egui::FontFamily::Proportional)
            .expect("应有 Proportional family");
        let monospace = fonts
            .families
            .get(&egui::FontFamily::Monospace)
            .expect("应有 Monospace family");

        assert!(
            !proportional.contains(&"user_ui".to_owned()),
            "字体文件读取失败时不应把该键写进 Proportional（否则 egui 会因为缺字体 panic 或空白）"
        );
        assert!(
            !monospace.contains(&"user_code".to_owned()),
            "字体文件读取失败时不应把该键写进 Monospace"
        );
    }
}
