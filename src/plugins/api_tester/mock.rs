use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::thread;

use anyhow::Result;

/// Mock 响应配置
#[derive(Debug, Clone)]
pub struct MockResponse {
    pub status: u16,
    pub headers: HashMap<String, String>,
    pub body: String,
}

impl MockResponse {
    pub fn new(status: u16, body: impl Into<String>) -> Self {
        let mut headers = HashMap::new();
        headers.insert("Content-Type".to_string(), "application/json".to_string());
        headers.insert("Access-Control-Allow-Origin".to_string(), "*".to_string());

        Self {
            status,
            headers,
            body: body.into(),
        }
    }

    /// 创建 JSON 响应
    pub fn json(status: u16, json_value: serde_json::Value) -> Self {
        Self::new(status, serde_json::to_string_pretty(&json_value).unwrap())
    }

    /// 创建成功响应
    pub fn ok(body: impl Into<String>) -> Self {
        Self::new(200, body)
    }

    /// 创建 JSON 成功响应
    pub fn ok_json(json_value: serde_json::Value) -> Self {
        Self::json(200, json_value)
    }

    /// 创建错误响应
    pub fn error(status: u16, message: impl Into<String>) -> Self {
        let body = serde_json::json!({
            "error": message.into(),
            "status": status
        });
        Self::json(status, body)
    }
}

/// Mock 路由配置
#[derive(Debug, Clone)]
pub struct MockRoute {
    pub method: String,
    pub path: String,
    pub response: MockResponse,
}

/// Mock 服务器状态
pub struct MockServer {
    /// 监听端口
    port: u16,
    /// 路由配置
    routes: Arc<Mutex<Vec<MockRoute>>>,
    /// 服务器是否运行
    running: Arc<Mutex<bool>>,
    /// 服务器线程句柄
    handle: Option<thread::JoinHandle<()>>,
}

impl MockServer {
    /// 创建新的 Mock 服务器
    pub fn new(port: u16) -> Self {
        Self {
            port,
            routes: Arc::new(Mutex::new(Vec::new())),
            running: Arc::new(Mutex::new(false)),
            handle: None,
        }
    }

    /// 添加路由
    pub fn add_route(&self, method: &str, path: &str, response: MockResponse) {
        let mut routes = self.routes.lock().unwrap();
        routes.push(MockRoute {
            method: method.to_uppercase(),
            path: path.to_string(),
            response,
        });
    }

    /// 添加预设的 mock 路由
    pub fn add_default_routes(&self) {
        // 用户列表接口
        self.add_route(
            "GET",
            "/api/users",
            MockResponse::ok_json(serde_json::json!({
                "users": [
                    {"id": 1, "name": "张三", "email": "zhangsan@example.com"},
                    {"id": 2, "name": "李四", "email": "lisi@example.com"},
                    {"id": 3, "name": "王五", "email": "wangwu@example.com"}
                ],
                "total": 3
            })),
        );

        // 用户详情接口
        self.add_route(
            "GET",
            "/api/users/1",
            MockResponse::ok_json(serde_json::json!({
                "id": 1,
                "name": "张三",
                "email": "zhangsan@example.com",
                "avatar": "https://example.com/avatar/1.jpg",
                "created_at": "2024-01-15T10:30:00Z"
            })),
        );

        // 创建用户接口
        self.add_route(
            "POST",
            "/api/users",
            MockResponse::new(201, serde_json::json!({
                "id": 4,
                "message": "用户创建成功"
            }).to_string()),
        );

        // 更新用户接口
        self.add_route(
            "PUT",
            "/api/users/1",
            MockResponse::ok_json(serde_json::json!({
                "id": 1,
                "message": "用户更新成功"
            })),
        );

        // 删除用户接口
        self.add_route(
            "DELETE",
            "/api/users/1",
            MockResponse::ok_json(serde_json::json!({
                "message": "用户删除成功"
            })),
        );

        // 健康检查接口
        self.add_route(
            "GET",
            "/api/health",
            MockResponse::ok_json(serde_json::json!({
                "status": "ok",
                "version": "1.0.0",
                "timestamp": chrono::Utc::now().to_rfc3339()
            })),
        );

        // 404 测试接口
        self.add_route(
            "GET",
            "/api/404",
            MockResponse::error(404, "资源不存在"),
        );

        // 500 测试接口
        self.add_route(
            "GET",
            "/api/500",
            MockResponse::error(500, "服务器内部错误"),
        );

        log::info!("已添加 {} 条 mock 路由", self.routes.lock().unwrap().len());
    }

    /// 启动服务器
    pub fn start(&mut self) -> Result<()> {
        let mut running = self.running.lock().unwrap();
        if *running {
            return Ok(());
        }

        let port = self.port;
        let routes = self.routes.clone();
        let running_flag = self.running.clone();

        // 使用简单的 HTTP 服务器
        let handle = thread::spawn(move || {
            Self::run_server(port, routes, running_flag);
        });

        self.handle = Some(handle);
        *running = true;

        log::info!("Mock 服务器已启动，监听端口: {}", self.port);
        Ok(())
    }

    /// 停止服务器
    pub fn stop(&mut self) {
        let mut running = self.running.lock().unwrap();
        if !*running {
            return;
        }

        *running = false;

        if let Some(handle) = self.handle.take() {
            // 注意：这里不能真正停止线程，只能设置标志位
            // 实际的服务器需要检查这个标志位来停止
            let _ = handle;
        }

        log::info!("Mock 服务器已停止");
    }

    /// 检查服务器是否运行
    pub fn is_running(&self) -> bool {
        *self.running.lock().unwrap()
    }

    /// 获取服务器地址
    pub fn base_url(&self) -> String {
        format!("http://localhost:{}", self.port)
    }

    /// 运行服务器（简化版本，使用 TCP 监听）
    fn run_server(port: u16, routes: Arc<Mutex<Vec<MockRoute>>>, running: Arc<Mutex<bool>>) {
        use std::io::{BufRead, BufReader, Write};
        use std::net::TcpListener;

        let listener = match TcpListener::bind(format!("127.0.0.1:{}", port)) {
            Ok(l) => l,
            Err(e) => {
                log::error!("Mock 服务器绑定端口失败: {}", e);
                return;
            }
        };

        listener.set_nonblocking(true).unwrap();

        log::info!("Mock 服务器开始监听 127.0.0.1:{}", port);

        loop {
            // 检查是否应该停止
            if !*running.lock().unwrap() {
                break;
            }

            match listener.accept() {
                Ok((mut stream, _addr)) => {
                    let reader = BufReader::new(stream.try_clone().unwrap());
                    let mut lines = reader.lines();

                    // 读取请求行
                    if let Some(Ok(request_line)) = lines.next() {
                        let parts: Vec<&str> = request_line.split_whitespace().collect();
                        if parts.len() >= 2 {
                            let method = parts[0];
                            let path = parts[1];

                            // 跳过请求头
                            loop {
                                match lines.next() {
                                    Some(Ok(line)) if line.is_empty() => break,
                                    Some(Ok(_)) => continue,
                                    _ => break,
                                }
                            }

                            // 查找匹配的路由
                            let routes = routes.lock().unwrap();
                            let matched = routes.iter().find(|r| {
                                r.method.eq_ignore_ascii_case(method) && r.path == path
                            });

                            let response = if let Some(route) = matched {
                                format!(
                                    "HTTP/1.1 {} OK\r\n\
                                     Content-Type: application/json\r\n\
                                     Access-Control-Allow-Origin: *\r\n\
                                     Content-Length: {}\r\n\
                                     Connection: close\r\n\
                                     \r\n\
                                     {}",
                                    route.response.status,
                                    route.response.body.len(),
                                    route.response.body
                                )
                            } else {
                                let error_body = serde_json::json!({
                                    "error": "Not Found",
                                    "message": format!("{} {} 不存在", method, path)
                                });
                                let error_str = serde_json::to_string(&error_body).unwrap();

                                format!(
                                    "HTTP/1.1 404 Not Found\r\n\
                                     Content-Type: application/json\r\n\
                                     Access-Control-Allow-Origin: *\r\n\
                                     Content-Length: {}\r\n\
                                     Connection: close\r\n\
                                     \r\n\
                                     {}",
                                    error_str.len(),
                                    error_str
                                )
                            };

                            let _ = stream.write_all(response.as_bytes());
                            let _ = stream.flush();
                        }
                    }
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    // 没有连接，休眠一下
                    std::thread::sleep(std::time::Duration::from_millis(100));
                    continue;
                }
                Err(e) => {
                    log::error!("接受连接失败: {}", e);
                    continue;
                }
            }
        }

        log::info!("Mock 服务器已退出");
    }
}

impl Drop for MockServer {
    fn drop(&mut self) {
        self.stop();
    }
}
