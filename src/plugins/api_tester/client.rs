use anyhow::{Context, Result};
use reqwest::blocking::Client;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use reqwest::redirect::Policy;
use std::time::Instant;

use super::models::{ApiRequest, ApiResponse, BodyType, HttpMethod};

/// 解析 form 数据，失败时返回错误
fn parse_form_data(body: &str) -> Result<Vec<(String, String)>> {
    serde_json::from_str(body).context("解析 Form 数据失败，请检查 JSON 格式")
}

/// HTTP 客户端封装
pub struct HttpClient {
    client: Client,
}

impl HttpClient {
    /// 创建新的 HTTP 客户端
    pub fn new() -> Result<Self> {
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .redirect(Policy::limited(10))
            .build()
            .context("创建 HTTP 客户端失败")?;

        Ok(Self { client })
    }

    /// 发送 HTTP 请求
    pub fn send(&self, request: &ApiRequest) -> Result<ApiResponse> {
        let start = Instant::now();

        // 构建请求
        let method = match request.method {
            HttpMethod::Get => reqwest::Method::GET,
            HttpMethod::Post => reqwest::Method::POST,
            HttpMethod::Put => reqwest::Method::PUT,
            HttpMethod::Delete => reqwest::Method::DELETE,
            HttpMethod::Patch => reqwest::Method::PATCH,
            HttpMethod::Head => reqwest::Method::HEAD,
            HttpMethod::Options => reqwest::Method::OPTIONS,
        };

        let mut req_builder = self.client.request(method, &request.url);

        // 添加请求头
        let headers = self.build_headers(&request.headers)?;
        req_builder = req_builder.headers(headers);

        // 添加请求体
        if request.method != HttpMethod::Get && request.method != HttpMethod::Head {
            match request.body_type {
                BodyType::Json => {
                    req_builder = req_builder
                        .header("Content-Type", "application/json")
                        .body(request.body.clone());
                }
                BodyType::Form => {
                    // 解析 form 数据
                    let form_data = parse_form_data(&request.body)?;
                    req_builder = req_builder.form(&form_data);
                }
                BodyType::Raw => {
                    req_builder = req_builder.body(request.body.clone());
                }
                BodyType::None => {}
            }
        }

        // 发送请求
        let response = req_builder.send().context("发送请求失败")?;

        let elapsed = start.elapsed();

        // 获取响应信息
        let status_code = response.status().as_u16();
        let status_text = response
            .status()
            .canonical_reason()
            .unwrap_or("Unknown")
            .to_string();

        // 获取响应头
        let headers: Vec<(String, String)> = response
            .headers()
            .iter()
            .map(|(k, v)| {
                (
                    k.to_string(),
                    v.to_str().unwrap_or("<非 UTF-8 值>").to_string(),
                )
            })
            .collect();

        // 获取响应体
        let body = response.text().context("读取响应体失败")?;
        let size_bytes = body.len();

        Ok(ApiResponse {
            status_code,
            status_text,
            headers,
            body,
            elapsed_ms: elapsed.as_millis() as u64,
            size_bytes,
        })
    }

    /// 构建请求头
    fn build_headers(&self, headers: &[super::models::HeaderEntry]) -> Result<HeaderMap> {
        let mut header_map = HeaderMap::new();

        for entry in headers {
            if !entry.enabled || entry.key.is_empty() {
                continue;
            }

            let name = HeaderName::from_bytes(entry.key.as_bytes())
                .context(format!("无效的请求头名称: {}", entry.key))?;
            let value = HeaderValue::from_str(&entry.value)
                .context(format!("无效的请求头值: {}", entry.value))?;

            header_map.insert(name, value);
        }

        Ok(header_map)
    }
}

/// 格式化 JSON 字符串
pub fn format_json(json_str: &str) -> String {
    match serde_json::from_str::<serde_json::Value>(json_str) {
        Ok(value) => serde_json::to_string_pretty(&value).unwrap_or_else(|_| json_str.to_string()),
        Err(_) => json_str.to_string(),
    }
}
