use serde::{Deserialize, Serialize};

/// HTTP 请求方法
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum HttpMethod {
    Get,
    Post,
    Put,
    Delete,
    Patch,
    Head,
    Options,
}

impl HttpMethod {
    /// 获取所有 HTTP 方法
    pub fn all() -> &'static [HttpMethod] {
        &[
            HttpMethod::Get,
            HttpMethod::Post,
            HttpMethod::Put,
            HttpMethod::Delete,
            HttpMethod::Patch,
            HttpMethod::Head,
            HttpMethod::Options,
        ]
    }

    /// 转换为字符串
    pub fn as_str(&self) -> &'static str {
        match self {
            HttpMethod::Get => "GET",
            HttpMethod::Post => "POST",
            HttpMethod::Put => "PUT",
            HttpMethod::Delete => "DELETE",
            HttpMethod::Patch => "PATCH",
            HttpMethod::Head => "HEAD",
            HttpMethod::Options => "OPTIONS",
        }
    }

    /// 从字符串解析
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_uppercase().as_str() {
            "GET" => Some(HttpMethod::Get),
            "POST" => Some(HttpMethod::Post),
            "PUT" => Some(HttpMethod::Put),
            "DELETE" => Some(HttpMethod::Delete),
            "PATCH" => Some(HttpMethod::Patch),
            "HEAD" => Some(HttpMethod::Head),
            "OPTIONS" => Some(HttpMethod::Options),
            _ => None,
        }
    }
}

impl std::fmt::Display for HttpMethod {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// 请求体类型
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum BodyType {
    None,
    Json,
    Form,
    Raw,
}

impl BodyType {
    /// 获取所有请求体类型
    pub fn all() -> &'static [BodyType] {
        &[BodyType::None, BodyType::Json, BodyType::Form, BodyType::Raw]
    }

    /// 转换为字符串
    pub fn as_str(&self) -> &'static str {
        match self {
            BodyType::None => "None",
            BodyType::Json => "JSON",
            BodyType::Form => "Form",
            BodyType::Raw => "Raw",
        }
    }
}

impl std::fmt::Display for BodyType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// 请求头键值对
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HeaderEntry {
    pub key: String,
    pub value: String,
    pub enabled: bool,
}

impl HeaderEntry {
    pub fn new(key: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            value: value.into(),
            enabled: true,
        }
    }

    pub fn empty() -> Self {
        Self {
            key: String::new(),
            value: String::new(),
            enabled: true,
        }
    }
}

/// API 请求配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiRequest {
    pub id: String,
    pub name: String,
    pub method: HttpMethod,
    pub url: String,
    pub headers: Vec<HeaderEntry>,
    pub params: Vec<HeaderEntry>,
    pub body_type: BodyType,
    pub body: String,
}

impl ApiRequest {
    pub fn new() -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            name: "New Request".to_string(),
            method: HttpMethod::Get,
            url: String::new(),
            headers: vec![
                HeaderEntry::new("Content-Type", "application/json"),
            ],
            params: Vec::new(),
            body_type: BodyType::None,
            body: String::new(),
        }
    }

    /// 构建完整的 URL（包含查询参数）
    pub fn build_url(&self) -> String {
        let mut url = self.url.clone();
        let enabled_params: Vec<_> = self.params.iter().filter(|p| p.enabled && !p.key.is_empty()).collect();

        if !enabled_params.is_empty() {
            let query_string: String = enabled_params
                .iter()
                .map(|p| format!("{}={}", urlencoding::encode(&p.key), urlencoding::encode(&p.value)))
                .collect::<Vec<_>>()
                .join("&");

            if url.contains('?') {
                url.push('&');
            } else {
                url.push('?');
            }
            url.push_str(&query_string);
        }

        url
    }
}

/// API 响应结果
#[derive(Debug, Clone)]
pub struct ApiResponse {
    pub status_code: u16,
    pub status_text: String,
    pub headers: Vec<(String, String)>,
    pub body: String,
    pub elapsed_ms: u64,
    pub size_bytes: usize,
}

impl ApiResponse {
    /// 获取状态文本
    pub fn status_display(&self) -> String {
        format!("{} {}", self.status_code, self.status_text)
    }

    /// 获取耗时文本
    pub fn elapsed_display(&self) -> String {
        if self.elapsed_ms < 1000 {
            format!("{}ms", self.elapsed_ms)
        } else {
            format!("{:.2}s", self.elapsed_ms as f64 / 1000.0)
        }
    }

    /// 获取大小文本
    pub fn size_display(&self) -> String {
        if self.size_bytes < 1024 {
            format!("{} B", self.size_bytes)
        } else if self.size_bytes < 1024 * 1024 {
            format!("{:.2} KB", self.size_bytes as f64 / 1024.0)
        } else {
            format!("{:.2} MB", self.size_bytes as f64 / (1024.0 * 1024.0))
        }
    }
}

/// 请求历史记录
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestHistory {
    pub id: i64,
    pub request_id: String,
    pub method: String,
    pub url: String,
    pub status_code: Option<i32>,
    pub elapsed_ms: Option<i64>,
    pub executed_at: String,
}

/// 响应标签页
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResponseTab {
    Body,
    Headers,
    Cookies,
}

/// 请求标签页
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RequestTab {
    Headers,
    Body,
    Params,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_url_no_params() {
        let request = ApiRequest {
            url: "https://example.com/api".to_string(),
            params: vec![],
            ..ApiRequest::new()
        };
        assert_eq!(request.build_url(), "https://example.com/api");
    }

    #[test]
    fn test_build_url_with_params() {
        let request = ApiRequest {
            url: "https://example.com/api".to_string(),
            params: vec![
                HeaderEntry::new("key1", "value1"),
                HeaderEntry::new("key2", "value2"),
            ],
            ..ApiRequest::new()
        };
        assert_eq!(request.build_url(), "https://example.com/api?key1=value1&key2=value2");
    }

    #[test]
    fn test_build_url_with_existing_query() {
        let request = ApiRequest {
            url: "https://example.com/api?existing=param".to_string(),
            params: vec![HeaderEntry::new("key1", "value1")],
            ..ApiRequest::new()
        };
        assert_eq!(request.build_url(), "https://example.com/api?existing=param&key1=value1");
    }

    #[test]
    fn test_build_url_skip_disabled_params() {
        let mut param = HeaderEntry::new("key1", "value1");
        param.enabled = false;

        let request = ApiRequest {
            url: "https://example.com/api".to_string(),
            params: vec![param, HeaderEntry::new("key2", "value2")],
            ..ApiRequest::new()
        };
        assert_eq!(request.build_url(), "https://example.com/api?key2=value2");
    }

    #[test]
    fn test_build_url_skip_empty_key() {
        let request = ApiRequest {
            url: "https://example.com/api".to_string(),
            params: vec![
                HeaderEntry::empty(),
                HeaderEntry::new("key2", "value2"),
            ],
            ..ApiRequest::new()
        };
        assert_eq!(request.build_url(), "https://example.com/api?key2=value2");
    }

    #[test]
    fn test_build_url_encode_special_chars() {
        let request = ApiRequest {
            url: "https://example.com/api".to_string(),
            params: vec![HeaderEntry::new("key", "value with spaces&special=chars")],
            ..ApiRequest::new()
        };
        assert_eq!(request.build_url(), "https://example.com/api?key=value%20with%20spaces%26special%3Dchars");
    }
}
