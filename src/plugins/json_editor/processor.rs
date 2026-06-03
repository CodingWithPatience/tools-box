use serde_json::Value;

/// JSON 处理结果
pub struct ProcessResult {
    pub output: String,
    pub is_error: bool,
    pub message: String,
}

impl ProcessResult {
    fn success(output: String) -> Self {
        Self {
            output,
            is_error: false,
            message: String::new(),
        }
    }

    fn error(message: String) -> Self {
        Self {
            output: String::new(),
            is_error: true,
            message,
        }
    }
}

/// 格式化美化 JSON（带缩进）
pub fn format_json(input: &str) -> ProcessResult {
    match serde_json::from_str::<Value>(input) {
        Ok(value) => match serde_json::to_string_pretty(&value) {
            Ok(formatted) => ProcessResult::success(formatted),
            Err(e) => ProcessResult::error(format!("格式化失败: {}", e)),
        },
        Err(e) => ProcessResult::error(format!("JSON 解析错误: {}", e)),
    }
}

/// 压缩 JSON（去除空白）
pub fn minify_json(input: &str) -> ProcessResult {
    match serde_json::from_str::<Value>(input) {
        Ok(value) => match serde_json::to_string(&value) {
            Ok(minified) => ProcessResult::success(minified),
            Err(e) => ProcessResult::error(format!("压缩失败: {}", e)),
        },
        Err(e) => ProcessResult::error(format!("JSON 解析错误: {}", e)),
    }
}

/// 转义 JSON 为字符串形式
///
/// 将 JSON 对象转为一个被引号包裹的字符串，内部所有特殊字符被转义。
/// 例如: `{"a":1}` → `"{\"a\":1}"`
pub fn escape_json(input: &str) -> ProcessResult {
    // 先验证是否为合法 JSON
    if serde_json::from_str::<Value>(input).is_err() {
        // 不是合法 JSON，直接转义为 JSON 字符串
        let escaped = serde_json::to_string(input).unwrap_or_else(|_| {
            format!("\"{}\"", input.replace('\\', "\\\\").replace('"', "\\\""))
        });
        return ProcessResult::success(escaped);
    }

    // 合法 JSON，转义为字符串形式
    let escaped = serde_json::to_string(input).unwrap_or_else(|_| {
        format!("\"{}\"", input.replace('\\', "\\\\").replace('"', "\\\""))
    });
    ProcessResult::success(escaped)
}

/// 反转义：将 JSON 字符串形式还原为 JSON 对象
///
/// 例如: `"{\"a\":1}"` → `{"a":1}`
pub fn unescape_json(input: &str) -> ProcessResult {
    let trimmed = input.trim();

    // 尝试作为 JSON 字符串解析
    if trimmed.starts_with('"') && trimmed.ends_with('"') {
        match serde_json::from_str::<String>(trimmed) {
            Ok(unescaped) => {
                // 尝试将反转义后的内容格式化输出
                if let Ok(value) = serde_json::from_str::<Value>(&unescaped) {
                    match serde_json::to_string_pretty(&value) {
                        Ok(pretty) => return ProcessResult::success(pretty),
                        Err(_) => return ProcessResult::success(unescaped),
                    }
                }
                ProcessResult::success(unescaped)
            }
            Err(e) => ProcessResult::error(format!("反转义失败: {}", e)),
        }
    } else {
        ProcessResult::error("输入不是有效的 JSON 字符串形式（需以引号包裹）".to_string())
    }
}

/// 校验 JSON 并返回错误位置信息
pub fn validate_json(input: &str) -> ProcessResult {
    match serde_json::from_str::<Value>(input) {
        Ok(_) => ProcessResult {
            output: input.to_string(),
            is_error: false,
            message: "✓ JSON 有效".to_string(),
        },
        Err(e) => {
            let line = e.line();
            let col = e.column();
            ProcessResult {
                output: input.to_string(),
                is_error: true,
                message: format!("✗ 错误: {} (行 {}, 列 {})", e, line, col),
            }
        }
    }
}

/// 统计 JSON 信息
pub fn json_stats(input: &str) -> (usize, usize, bool) {
    let bytes = input.len();
    let lines = input.lines().count();
    let valid = serde_json::from_str::<Value>(input).is_ok();
    (bytes, lines, valid)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_json() {
        let r = format_json(r#"{"name":"test","value":42}"#);
        assert!(!r.is_error);
        assert!(r.output.contains('\n'));
    }

    #[test]
    fn test_minify_json() {
        let input = "{\n  \"name\": \"test\"\n}";
        let r = minify_json(input);
        assert!(!r.is_error);
        assert_eq!(r.output, r#"{"name":"test"}"#);
    }

    #[test]
    fn test_escape_unescape() {
        let input = r#"{"key":"value"}"#;
        let escaped = escape_json(input);
        assert!(!escaped.is_error);

        let unescaped = unescape_json(&escaped.output);
        assert!(!unescaped.is_error);
    }

    #[test]
    fn test_validate_valid() {
        let r = validate_json(r#"{"ok":true}"#);
        assert!(!r.is_error);
    }

    #[test]
    fn test_validate_invalid() {
        let r = validate_json(r#"{"broken":}"#);
        assert!(r.is_error);
    }
}
