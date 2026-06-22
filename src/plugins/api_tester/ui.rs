use egui::{Color32, RichText, Ui};

use super::client::{HttpClient, format_json};
use super::mock::MockServer;
use super::models::{
    ApiRequest, ApiResponse, BodyType, HeaderEntry, HttpMethod, RequestHistory, RequestTab,
    ResponseTab,
};
use super::store::ApiStore;

/// Mock 服务器默认端口
const MOCK_SERVER_PORT: u16 = 8089;

/// API 调试工具 UI
pub struct ApiTesterUi {
    /// 当前请求配置
    request: ApiRequest,
    /// 当前响应
    response: Option<ApiResponse>,
    /// 请求头编辑临时数据
    headers: Vec<HeaderEntry>,
    /// 请求体类型
    body_type: BodyType,
    /// 请求体内容
    body: String,
    /// 当前请求标签页
    request_tab: RequestTab,
    /// 当前响应标签页
    response_tab: ResponseTab,
    /// HTTP 客户端
    client: Option<HttpClient>,
    /// 错误信息
    error: Option<String>,
    /// 历史记录
    history: Vec<RequestHistory>,
    /// 是否显示历史面板
    show_history: bool,
    /// 是否正在发送请求
    is_sending: bool,
    /// Mock 服务器（仅 debug 模式）
    #[cfg(debug_assertions)]
    mock_server: Option<MockServer>,
}

impl ApiTesterUi {
    pub fn new() -> Self {
        Self {
            request: ApiRequest::new(),
            response: None,
            headers: vec![
                HeaderEntry::new("Content-Type", "application/json"),
                HeaderEntry::new("Accept", "*/*"),
            ],
            body_type: BodyType::None,
            body: String::new(),
            request_tab: RequestTab::Headers,
            response_tab: ResponseTab::Body,
            client: None,
            error: None,
            history: Vec::new(),
            show_history: false,
            is_sending: false,
            #[cfg(debug_assertions)]
            mock_server: None,
        }
    }

    /// 初始化
    pub fn init(&mut self, conn: &rusqlite::Connection) {
        // 初始化 HTTP 客户端
        match HttpClient::new() {
            Ok(client) => self.client = Some(client),
            Err(e) => self.error = Some(format!("初始化客户端失败: {}", e)),
        }

        // 加载历史记录
        self.load_history(conn);
    }

    /// 加载历史记录
    fn load_history(&mut self, conn: &rusqlite::Connection) {
        let store = ApiStore::new(conn);
        match store.get_recent_history(50) {
            Ok(history) => self.history = history,
            Err(e) => self.error = Some(format!("加载历史记录失败: {}", e)),
        }
    }

    /// 渲染主界面
    pub fn render(&mut self, ui: &mut Ui, conn: &rusqlite::Connection) {
        // 顶部标题栏
        ui.horizontal(|ui| {
            ui.heading("API 调试工具");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                // Mock 服务控制（仅 debug 模式）
                #[cfg(debug_assertions)]
                {
                    let mock_running = self
                        .mock_server
                        .as_ref()
                        .map(|s| s.is_running())
                        .unwrap_or(false);

                    let mock_btn_text = if mock_running {
                        "停止 Mock"
                    } else {
                        "启动 Mock"
                    };

                    let mock_btn_color = if mock_running {
                        Color32::from_rgb(200, 0, 0)
                    } else {
                        Color32::from_rgb(0, 150, 0)
                    };

                    if ui
                        .button(RichText::new(mock_btn_text).color(mock_btn_color))
                        .on_hover_text("启动/停止 Mock 服务器 (端口 8089)")
                        .clicked()
                    {
                        self.toggle_mock_server();
                    }

                    if mock_running {
                        ui.label(
                            RichText::new(format!("Mock: localhost:{}", MOCK_SERVER_PORT))
                                .color(Color32::from_rgb(0, 150, 0))
                                .small(),
                        );
                    }

                    ui.separator();
                }

                if self.show_history {
                    if ui
                        .button(RichText::new("返回").strong())
                        .on_hover_text("返回主界面")
                        .clicked()
                    {
                        self.show_history = false;
                    }
                } else {
                    if ui
                        .button(RichText::new("历史").strong())
                        .on_hover_text("显示请求历史")
                        .clicked()
                    {
                        self.show_history = true;
                    }
                }
            });
        });
        ui.separator();

        // 主内容区域
        if self.show_history {
            ui.horizontal_top(|ui| {
                // 左侧历史面板
                ui.vertical(|ui| {
                    ui.set_min_width(360.0);
                    ui.label(RichText::new("请求历史").strong());
                    ui.separator();
                    self.render_history_panel(ui, conn);
                });

                ui.separator();

                // 右侧主内容
                ui.vertical(|ui| {
                    self.render_main_content(ui, conn);
                });
            });
        } else {
            self.render_main_content(ui, conn);
        }
    }

    /// 渲染主内容区域
    fn render_main_content(&mut self, ui: &mut Ui, conn: &rusqlite::Connection) {
        // 请求配置区域
        self.render_request_config(ui);

        ui.add_space(10.0);

        // 请求头/体配置
        self.render_request_tabs(ui);

        ui.add_space(10.0);

        // 发送按钮
        ui.horizontal(|ui| {
            let send_btn_text = if self.is_sending {
                "发送中..."
            } else {
                "发送请求"
            };

            let send_btn = ui.add_enabled(
                !self.is_sending && self.client.is_some(),
                egui::Button::new(RichText::new(send_btn_text).strong())
                    .min_size(egui::vec2(120.0, 32.0)),
            );

            if send_btn.clicked() {
                self.send_request(conn);
            }

            if let Some(err) = &self.error {
                ui.label(RichText::new(err).color(Color32::RED));
            }
        });

        ui.add_space(10.0);

        // 响应区域
        if self.response.is_some() || self.error.is_some() {
            ui.separator();
            self.render_response(ui);
        }
    }

    /// 渲染请求配置
    fn render_request_config(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            // HTTP 方法选择
            egui::ComboBox::from_id_salt("http_method")
                .selected_text(self.request.method.as_str())
                .show_ui(ui, |ui| {
                    for method in HttpMethod::all() {
                        ui.selectable_value(
                            &mut self.request.method,
                            method.clone(),
                            method.as_str(),
                        );
                    }
                });

            // URL 输入框
            ui.add(
                egui::TextEdit::singleline(&mut self.request.url)
                    .hint_text("输入请求 URL，例如: https://api.example.com/users")
                    .desired_width(ui.available_width()),
            );
        });
    }

    /// 渲染请求标签页
    fn render_request_tabs(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.request_tab, RequestTab::Headers, "请求头");
            ui.selectable_value(&mut self.request_tab, RequestTab::Body, "请求体");
            ui.selectable_value(&mut self.request_tab, RequestTab::Params, "查询参数");
        });
        ui.separator();

        match self.request_tab {
            RequestTab::Headers => self.render_headers_editor(ui),
            RequestTab::Body => self.render_body_editor(ui),
            RequestTab::Params => self.render_params_editor(ui),
        }
    }

    /// 渲染请求头编辑器
    fn render_headers_editor(&mut self, ui: &mut Ui) {
        let mut to_remove = Vec::new();

        egui::Grid::new("headers_grid")
            .striped(true)
            .num_columns(3)
            .min_col_width(150.0)
            .show(ui, |ui| {
                // 表头
                ui.label(RichText::new("启用").strong());
                ui.label(RichText::new("Key").strong());
                ui.label(RichText::new("Value").strong());
                ui.end_row();

                // 请求头列表
                for (i, header) in self.headers.iter_mut().enumerate() {
                    ui.checkbox(&mut header.enabled, "");
                    ui.add(
                        egui::TextEdit::singleline(&mut header.key)
                            .hint_text("Header Name")
                            .desired_width(200.0),
                    );
                    ui.add(
                        egui::TextEdit::singleline(&mut header.value)
                            .hint_text("Header Value")
                            .desired_width(ui.available_width() - 60.0),
                    );

                    if ui.button("删除").clicked() {
                        to_remove.push(i);
                    }
                    ui.end_row();
                }
            });

        // 删除标记的请求头
        for i in to_remove.into_iter().rev() {
            self.headers.remove(i);
        }

        ui.add_space(5.0);
        if ui.button("添加请求头").clicked() {
            self.headers.push(HeaderEntry::empty());
        }
    }

    /// 渲染请求体编辑器
    fn render_body_editor(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            ui.label("请求体类型:");
            for body_type in BodyType::all() {
                ui.selectable_value(&mut self.body_type, body_type.clone(), body_type.as_str());
            }
        });

        ui.add_space(5.0);

        if self.body_type != BodyType::None {
            // 固定高度，避免占据响应内容区域
            let body_height = 200.0;

            egui::ScrollArea::vertical()
                .id_salt("body_editor_scroll")
                .max_height(body_height)
                .show(ui, |ui| {
                    ui.add_sized(
                        [ui.available_width(), body_height],
                        egui::TextEdit::multiline(&mut self.body)
                            .hint_text(match self.body_type {
                                BodyType::Json => "输入 JSON 请求体...",
                                BodyType::Form => "输入 Form 数据 (JSON 格式)...",
                                BodyType::Raw => "输入请求体...",
                                _ => "",
                            })
                            .code_editor(),
                    );
                });
        } else {
            ui.label("当前请求方法不支持请求体");
        }
    }

    /// 渲染查询参数编辑器
    fn render_params_editor(&mut self, ui: &mut Ui) {
        ui.label("查询参数将自动添加到 URL 中");
        ui.add_space(5.0);

        // TODO: 实现查询参数编辑器
        ui.label("功能开发中...");
    }

    /// 渲染响应区域
    fn render_response(&mut self, ui: &mut Ui) {
        if let Some(response) = &self.response {
            // 响应状态栏
            ui.horizontal(|ui| {
                let status_color = if response.status_code < 300 {
                    Color32::from_rgb(0, 150, 0) // 绿色
                } else if response.status_code < 400 {
                    Color32::from_rgb(200, 200, 0) // 黄色
                } else {
                    Color32::from_rgb(200, 0, 0) // 红色
                };

                ui.label(
                    RichText::new(format!("状态: {}", response.status_display()))
                        .color(status_color)
                        .strong(),
                );
                ui.separator();
                ui.label(format!("耗时: {}", response.elapsed_display()));
                ui.separator();
                ui.label(format!("大小: {}", response.size_display()));
            });

            ui.add_space(5.0);

            // 响应标签页
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.response_tab, ResponseTab::Body, "响应体");
                ui.selectable_value(&mut self.response_tab, ResponseTab::Headers, "响应头");
                ui.selectable_value(&mut self.response_tab, ResponseTab::Cookies, "Cookies");
            });
            ui.separator();

            match self.response_tab {
                ResponseTab::Body => self.render_response_body(ui, response),
                ResponseTab::Headers => self.render_response_headers(ui, response),
                ResponseTab::Cookies => self.render_response_cookies(ui, response),
            }
        }
    }

    /// 渲染响应体
    fn render_response_body(&self, ui: &mut Ui, response: &ApiResponse) {
        let formatted_body = format_json(&response.body);
        let available_height = ui.available_height().max(200.0);

        egui::ScrollArea::vertical()
            .max_height(available_height)
            .show(ui, |ui| {
                ui.add_sized(
                    [ui.available_width(), available_height],
                    egui::TextEdit::multiline(&mut formatted_body.as_str())
                        .code_editor(),
                );
            });
    }

    /// 渲染响应头
    fn render_response_headers(&self, ui: &mut Ui, response: &ApiResponse) {
        egui::Grid::new("response_headers_grid")
            .striped(true)
            .num_columns(2)
            .show(ui, |ui| {
                for (key, value) in &response.headers {
                    ui.label(RichText::new(key).strong());
                    ui.label(value);
                    ui.end_row();
                }
            });
    }

    /// 渲染 Cookies
    fn render_response_cookies(&self, ui: &mut Ui, _response: &ApiResponse) {
        ui.label("Cookies 信息:");
        ui.add_space(5.0);
        // TODO: 解析并显示 cookies
        ui.label("功能开发中...");
    }

    /// 渲染历史面板
    fn render_history_panel(&mut self, ui: &mut Ui, conn: &rusqlite::Connection) {
        let mut to_load: Option<i64> = None;
        let mut to_delete: Option<i64> = None;
        let mut clear_all = false;

        // 工具栏：显示记录数和清空按钮
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(format!("共 {} 条记录", self.history.len()))
                    .color(Color32::GRAY)
                    .small(),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .small_button(RichText::new("清空").color(Color32::from_rgb(200, 0, 0)))
                    .on_hover_text("清空所有历史记录")
                    .clicked()
                {
                    clear_all = true;
                }
            });
        });
        ui.separator();

        // 使用 Grid 实现表格，参考 Hosts 管理器的实现方式
        egui::ScrollArea::vertical()
            .id_salt("api_history_scroll")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                egui::Grid::new("api_history_table")
                    .striped(true)
                    .num_columns(5)
                    .spacing([8.0, 4.0])
                    .min_col_width(40.0)
                    .show(ui, |ui| {
                        // 表头
                        ui.strong("方法");
                        ui.strong("URL");
                        ui.strong("状态");
                        ui.strong("耗时");
                        ui.strong("操作");
                        ui.end_row();

                        // 历史记录列表
                        for history in &self.history {
                            let method_color = match history.method.as_str() {
                                "GET" => Color32::from_rgb(0, 150, 0),
                                "POST" => Color32::from_rgb(0, 0, 200),
                                "PUT" => Color32::from_rgb(200, 150, 0),
                                "DELETE" => Color32::from_rgb(200, 0, 0),
                                "PATCH" => Color32::from_rgb(200, 100, 0),
                                _ => Color32::GRAY,
                            };

                            // 格式化耗时显示
                            let elapsed_display = if let Some(ms) = history.elapsed_ms {
                                if ms < 1000 {
                                    format!("{}ms", ms)
                                } else {
                                    format!("{:.1}s", ms as f64 / 1000.0)
                                }
                            } else {
                                "-".to_string()
                            };

                            // 方法列
                            ui.label(
                                RichText::new(&history.method)
                                    .color(method_color)
                                    .strong()
                                    .monospace()
                                    .small(),
                            );

                            // URL 列（可点击）
                            let url_label = ui
                                .label(
                                    RichText::new(&history.url)
                                        .small()
                                        .color(Color32::from_rgb(100, 149, 237)),
                                )
                                .on_hover_text(&history.url);
                            if url_label.clicked() {
                                to_load = Some(history.id);
                            }

                            // 状态码列
                            if let Some(status) = history.status_code {
                                let status_color = if status < 300 {
                                    Color32::from_rgb(0, 150, 0)
                                } else if status < 400 {
                                    Color32::from_rgb(200, 180, 0)
                                } else {
                                    Color32::from_rgb(200, 0, 0)
                                };
                                ui.label(
                                    RichText::new(status.to_string())
                                        .color(status_color)
                                        .small(),
                                );
                            } else {
                                ui.label(RichText::new("-").color(Color32::GRAY).small());
                            }

                            // 耗时列
                            ui.label(
                                RichText::new(&elapsed_display)
                                    .color(Color32::GRAY)
                                    .small(),
                            );

                            // 删除按钮
                            if ui
                                .small_button(
                                    RichText::new("×")
                                        .color(Color32::from_rgb(200, 0, 0)),
                                )
                                .on_hover_text("删除此记录")
                                .clicked()
                            {
                                to_delete = Some(history.id);
                            }

                            ui.end_row();
                        }
                    });
            });

        // 执行操作
        let store = ApiStore::new(conn);

        if let Some(id) = to_load {
            self.load_history_item(id, conn);
        }

        if let Some(id) = to_delete {
            match store.delete_history(id) {
                Ok(_) => {
                    self.load_history(conn);
                    log::info!("已删除历史记录: id={}", id);
                }
                Err(e) => {
                    self.error = Some(format!("删除历史记录失败: {}", e));
                }
            }
        }

        if clear_all {
            match store.clear_history() {
                Ok(_) => {
                    self.history.clear();
                    log::info!("已清空所有历史记录");
                }
                Err(e) => {
                    self.error = Some(format!("清空历史记录失败: {}", e));
                }
            }
        }
    }

    /// 加载历史记录项
    fn load_history_item(&mut self, id: i64, conn: &rusqlite::Connection) {
        let store = ApiStore::new(conn);
        match store.get_history_by_id(id) {
            Ok(Some((method, url, headers, body))) => {
                if let Some(m) = HttpMethod::from_str(&method) {
                    self.request.method = m;
                }
                self.request.url = url;

                // 解析请求头
                if !headers.is_empty() {
                    if let Ok(parsed_headers) = serde_json::from_str::<Vec<HeaderEntry>>(&headers)
                    {
                        self.headers = parsed_headers;
                    }
                }

                // 解析请求体
                if !body.is_empty() {
                    self.body = body;
                    // 尝试检测请求体类型
                    if serde_json::from_str::<serde_json::Value>(&self.body).is_ok() {
                        self.body_type = BodyType::Json;
                    } else {
                        self.body_type = BodyType::Raw;
                    }
                }
            }
            Ok(None) => {
                self.error = Some("历史记录不存在".to_string());
            }
            Err(e) => {
                self.error = Some(format!("加载历史记录失败: {}", e));
            }
        }
    }

    /// 发送请求
    fn send_request(&mut self, conn: &rusqlite::Connection) {
        if self.request.url.is_empty() {
            self.error = Some("请输入请求 URL".to_string());
            return;
        }

        self.is_sending = true;
        self.error = None;
        self.response = None;

        // 更新请求配置
        self.request.headers = self.headers.clone();
        self.request.body_type = self.body_type.clone();
        self.request.body = self.body.clone();

        // 发送请求
        if let Some(client) = &self.client {
            match client.send(&self.request) {
                Ok(response) => {
                    // 保存到历史记录
                    let store = ApiStore::new(conn);
                    let headers_json =
                        serde_json::to_string(&self.request.headers).unwrap_or_default();

                    match store.save_history(
                        &self.request.id,
                        self.request.method.as_str(),
                        &self.request.url,
                        &headers_json,
                        self.request.body_type.as_str(),
                        &self.request.body,
                        Some(response.status_code as i32),
                        Some(&response.body),
                        Some(response.elapsed_ms as i64),
                    ) {
                        Ok(_) => {
                            // 重新加载历史记录
                            self.load_history(conn);
                        }
                        Err(e) => {
                            self.error = Some(format!("保存历史记录失败: {}", e));
                        }
                    }

                    self.response = Some(response);
                }
                Err(e) => {
                    self.error = Some(format!("请求失败: {}", e));
                }
            }
        } else {
            self.error = Some("HTTP 客户端未初始化".to_string());
        }

        self.is_sending = false;
    }

    /// 切换 Mock 服务器状态（仅 debug 模式）
    #[cfg(debug_assertions)]
    fn toggle_mock_server(&mut self) {
        if let Some(server) = &mut self.mock_server {
            if server.is_running() {
                server.stop();
                log::info!("Mock 服务器已停止");
            } else {
                if let Err(e) = server.start() {
                    self.error = Some(format!("启动 Mock 服务器失败: {}", e));
                }
            }
        } else {
            // 创建并启动 Mock 服务器
            let mut server = MockServer::new(MOCK_SERVER_PORT);
            server.add_default_routes();
            if let Err(e) = server.start() {
                self.error = Some(format!("启动 Mock 服务器失败: {}", e));
            }
            self.mock_server = Some(server);

            // 自动填充 Mock 服务器 URL
            self.request.url = format!("http://localhost:{}/api/users", MOCK_SERVER_PORT);
            self.request.method = HttpMethod::Get;
        }
    }
}
