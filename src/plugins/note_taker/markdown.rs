use egui::RichText;

/// Markdown 渲染器
///
/// 将 Markdown 文本转换为 egui 可显示的富文本段落
pub struct MarkdownRenderer;

impl MarkdownRenderer {
    /// 渲染 Markdown 文本为纯文本（简化版本）
    ///
    /// 由于 egui 原生不支持完整 HTML 渲染，
    /// 这里提供一个简化版本，保留 Markdown 结构但以纯文本显示
    pub fn render_plain(markdown: &str) -> String {
        let mut result = String::new();
        let mut in_code_block = false;

        for line in markdown.lines() {
            // 代码块处理
            if line.starts_with("```") {
                in_code_block = !in_code_block;
                if in_code_block {
                    result.push_str("--- 代码块 ---\n");
                } else {
                    result.push_str("--- /代码块 ---\n");
                }
                continue;
            }

            if in_code_block {
                result.push_str(line);
                result.push('\n');
                continue;
            }

            // 标题处理
            if line.starts_with("# ") {
                result.push_str(&format!["📌 {}\n", &line[2..]]);
            } else if line.starts_with("## ") {
                result.push_str(&format!["📌 {}\n", &line[3..]]);
            } else if line.starts_with("### ") {
                result.push_str(&format!["📌 {}\n", &line[4..]]);
            } else if line.starts_with("#### ") {
                result.push_str(&format!["📌 {}\n", &line[5..]]);
            }
            // 列表处理
            else if line.starts_with("- ") || line.starts_with("* ") {
                result.push_str(&format!["  • {}\n", &line[2..]]);
            } else if line.starts_with("  - ") || line.starts_with("  * ") {
                result.push_str(&format!["    ◦ {}\n", &line[4..]]);
            }
            // 有序列表
            else if let Some(pos) = line.find(". ") {
                if line[..pos].chars().all(|c| c.is_ascii_digit()) {
                    result.push_str(&format!["  {}\n", line]);
                } else {
                    result.push_str(line);
                    result.push('\n');
                }
            }
            // 引用
            else if line.starts_with("> ") {
                result.push_str(&format!["  │ {}\n", &line[2..]]);
            }
            // 分割线
            else if line.starts_with("---") || line.starts_with("***") || line.starts_with("___") {
                result.push_str("────────────────────────────\n");
            }
            // 普通行
            else {
                result.push_str(line);
                result.push('\n');
            }
        }

        result
    }

    /// 渲染 Markdown 为 RichText 段落列表
    ///
    /// 返回 (文本, 是否代码块) 的列表
    pub fn render_to_segments(markdown: &str) -> Vec<(String, bool)> {
        let mut segments = Vec::new();
        let mut current_segment = String::new();
        let mut in_code_block = false;

        for line in markdown.lines() {
            if line.starts_with("```") {
                // 保存当前段落
                if !current_segment.is_empty() {
                    segments.push((current_segment.clone(), false));
                    current_segment.clear();
                }
                in_code_block = !in_code_block;
                continue;
            }

            if in_code_block {
                current_segment.push_str(line);
                current_segment.push('\n');
            } else {
                // 标题前添加空行
                if line.starts_with('#') && !current_segment.is_empty() {
                    segments.push((current_segment.clone(), false));
                    current_segment.clear();
                    current_segment.push('\n');
                }

                current_segment.push_str(line);
                current_segment.push('\n');

                // 标题后添加空行
                if line.starts_with('#') {
                    segments.push((current_segment.clone(), false));
                    current_segment.clear();
                }
            }
        }

        if !current_segment.is_empty() {
            segments.push((current_segment, in_code_block));
        }

        segments
    }

    /// 创建标题样式
    pub fn heading_text(text: &str) -> RichText {
        RichText::new(text).strong().size(16.0)
    }

    /// 创建二级标题样式
    pub fn heading2_text(text: &str) -> RichText {
        RichText::new(text).strong().size(14.0)
    }

    /// 创建代码样式
    pub fn code_text(text: &str) -> RichText {
        RichText::new(text).monospace()
    }

    /// 创建引用样式
    pub fn quote_text(text: &str) -> RichText {
        RichText::new(text).italics()
    }
}