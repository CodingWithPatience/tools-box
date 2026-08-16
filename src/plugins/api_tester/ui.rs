use egui::{Color32, RichText, Ui};
use std::collections::HashMap;

use super::client::{HttpClient, format_json};
use super::mock::MockServer;
use super::models::{
    ApiCollection, ApiRequest, ApiResponse, BodyType, Environment, EnvironmentVariable,
    HeaderEntry, HttpMethod, RequestHistory, RequestTab, ResponseTab, SavedApiRequest,
    VariableReplacer,
};
use super::store::ApiStore;

/// Mock 服务器默认端口
const MOCK_SERVER_PORT: u16 = 8089;

/// 左侧面板视图
#[derive(Debug, Clone, PartialEq)]
enum LeftPanelView {
    Collections,
    History,
}

/// 一个打开的请求标签页状态
#[derive(Clone)]
struct OpenRequestTab {
    request: ApiRequest,
    response: Option<ApiResponse>,
    headers: Vec<HeaderEntry>,
    params: Vec<HeaderEntry>,
    body_type: BodyType,
    body: String,
    request_tab: RequestTab,
    response_tab: ResponseTab,
    saved_request_id: Option<i64>,
    collection_id: Option<i64>,
    is_default_name: bool,
    is_sending: bool,
}

impl OpenRequestTab {
    fn new() -> Self {
        Self {
            request: ApiRequest::new(),
            response: None,
            headers: vec![
                HeaderEntry::new("Content-Type", "application/json"),
                HeaderEntry::new("Accept", "*/*"),
            ],
            params: Vec::new(),
            body_type: BodyType::None,
            body: String::new(),
            request_tab: RequestTab::Headers,
            response_tab: ResponseTab::Body,
            saved_request_id: None,
            collection_id: None,
            is_default_name: true,
            is_sending: false,
        }
    }

    fn from_saved_request(request: SavedApiRequest) -> Self {
        Self {
            request: ApiRequest {
                id: uuid::Uuid::new_v4().to_string(),
                name: request.name,
                method: request.method,
                url: request.url,
                headers: request.headers.clone(),
                params: request.params.clone(),
                body_type: request.body_type.clone(),
                body: request.body.clone(),
            },
            response: None,
            headers: request.headers,
            params: request.params,
            body_type: request.body_type,
            body: request.body,
            request_tab: RequestTab::Headers,
            response_tab: ResponseTab::Body,
            saved_request_id: Some(request.id),
            collection_id: request.collection_id,
            is_default_name: false,
            is_sending: false,
        }
    }
}

/// API 调试工具 UI
pub struct ApiTesterUi {
    /// 当前请求配置
    request: ApiRequest,
    /// 当前响应
    response: Option<ApiResponse>,
    /// 请求头编辑临时数据
    headers: Vec<HeaderEntry>,
    /// 查询参数编辑临时数据
    params: Vec<HeaderEntry>,
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
    /// 是否正在发送请求
    is_sending: bool,
    /// 已打开的请求标签页
    open_tabs: Vec<OpenRequestTab>,
    /// 当前激活的请求标签页索引
    active_tab_index: usize,
    /// 当前请求对应的已保存请求 ID
    saved_request_id: Option<i64>,
    /// 当前请求原本所属的集合 ID
    request_collection_id: Option<i64>,
    /// 当前请求是否仍使用新建请求的默认名称
    is_default_name: bool,
    /// Mock 服务器（仅 debug 模式）
    #[cfg(debug_assertions)]
    mock_server: Option<MockServer>,

    // ========== 集合相关 ==========
    /// 集合列表
    collections: Vec<ApiCollection>,
    /// 保存的请求列表
    saved_requests: Vec<SavedApiRequest>,
    /// 当前选中的集合 ID
    selected_collection_id: Option<i64>,
    /// 左侧面板视图
    left_panel_view: LeftPanelView,
    /// 是否显示集合管理弹窗
    show_collection_dialog: bool,
    /// 集合编辑表单 - 名称
    collection_form_name: String,
    /// 集合编辑表单 - 描述
    collection_form_description: String,
    /// 编辑中的集合 ID
    editing_collection_id: Option<i64>,

    // ========== 环境相关 ==========
    /// 环境列表
    environments: Vec<Environment>,
    /// 当前激活的环境
    active_environment: Option<Environment>,
    /// 环境变量列表
    environment_variables: Vec<EnvironmentVariable>,
    /// 是否显示环境管理弹窗
    show_environment_dialog: bool,
    /// 是否显示环境变量编辑弹窗
    show_env_var_dialog: bool,
    /// 编辑中的环境 ID
    editing_environment_id: Option<i64>,
    /// 环境变量编辑表单
    env_var_form_key: String,
    env_var_form_value: String,
    /// 编辑中的环境变量 ID
    editing_env_var_id: Option<i64>,
    /// 环境名称编辑表单
    env_form_name: String,
    /// 是否显示新增环境弹窗
    show_add_env_dialog: bool,
    /// 是否显示重命名环境弹窗
    show_rename_env_dialog: bool,

    // ========== 布局相关 ==========
    /// 左侧面板宽度
    left_panel_width: f32,
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
            params: Vec::new(),
            body_type: BodyType::None,
            body: String::new(),
            request_tab: RequestTab::Headers,
            response_tab: ResponseTab::Body,
            client: None,
            error: None,
            history: Vec::new(),
            is_sending: false,
            open_tabs: vec![OpenRequestTab::new()],
            active_tab_index: 0,
            saved_request_id: None,
            request_collection_id: None,
            is_default_name: true,
            #[cfg(debug_assertions)]
            mock_server: None,

            collections: Vec::new(),
            saved_requests: Vec::new(),
            selected_collection_id: None,
            left_panel_view: LeftPanelView::Collections,
            show_collection_dialog: false,
            collection_form_name: String::new(),
            collection_form_description: String::new(),
            editing_collection_id: None,

            environments: Vec::new(),
            active_environment: None,
            environment_variables: Vec::new(),
            show_environment_dialog: false,
            show_env_var_dialog: false,
            editing_environment_id: None,
            env_var_form_key: String::new(),
            env_var_form_value: String::new(),
            editing_env_var_id: None,
            env_form_name: String::new(),
            show_add_env_dialog: false,
            show_rename_env_dialog: false,
            left_panel_width: 250.0,
        }
    }

    /// 初始化
    pub fn init(&mut self, conn: &rusqlite::Connection) {
        // 初始化 HTTP 客户端
        match HttpClient::new() {
            Ok(client) => self.client = Some(client),
            Err(e) => self.error = Some(format!("初始化客户端失败: {}", e)),
        }

        // 加载数据
        self.load_collections(conn);
        self.load_environments(conn);
        self.load_history(conn);
    }

    /// 将当前编辑器状态写回激活的请求标签页
    fn save_active_tab(&mut self) {
        if let Some(tab) = self.open_tabs.get_mut(self.active_tab_index) {
            tab.request = self.request.clone();
            tab.response = self.response.clone();
            tab.headers = self.headers.clone();
            tab.params = self.params.clone();
            tab.body_type = self.body_type.clone();
            tab.body = self.body.clone();
            tab.request_tab = self.request_tab.clone();
            tab.response_tab = self.response_tab.clone();
            tab.saved_request_id = self.saved_request_id;
            tab.collection_id = self.request_collection_id;
            tab.is_default_name = self.is_default_name;
            tab.is_sending = self.is_sending;
        }
    }

    /// 加载指定请求标签页的编辑器状态
    fn load_tab(&mut self, index: usize) {
        if let Some(tab) = self.open_tabs.get(index).cloned() {
            self.active_tab_index = index;
            self.request = tab.request;
            self.response = tab.response;
            self.headers = tab.headers;
            self.params = tab.params;
            self.body_type = tab.body_type;
            self.body = tab.body;
            self.request_tab = tab.request_tab;
            self.response_tab = tab.response_tab;
            self.saved_request_id = tab.saved_request_id;
            self.request_collection_id = tab.collection_id;
            self.is_default_name = tab.is_default_name;
            self.is_sending = tab.is_sending;
        }
    }

    /// 新建请求标签页
    fn create_request_tab(&mut self) {
        self.save_active_tab();
        self.open_tabs.push(OpenRequestTab::new());
        self.load_tab(self.open_tabs.len() - 1);
        self.error = None;
    }

    /// 打开已保存请求标签页；重复打开同一请求时切换到已有标签页
    fn open_saved_request(&mut self, request: SavedApiRequest) {
        self.save_active_tab();

        if let Some(index) = self
            .open_tabs
            .iter()
            .position(|tab| tab.saved_request_id == Some(request.id))
        {
            self.load_tab(index);
            return;
        }

        self.open_tabs
            .push(OpenRequestTab::from_saved_request(request));
        self.load_tab(self.open_tabs.len() - 1);
        self.error = None;
    }

    /// 切换请求标签页
    fn activate_request_tab(&mut self, index: usize) {
        if index == self.active_tab_index || index >= self.open_tabs.len() {
            return;
        }

        self.save_active_tab();
        self.load_tab(index);
        self.error = None;
    }

    /// 关闭请求标签页
    fn close_request_tab(&mut self, index: usize) {
        if index >= self.open_tabs.len() {
            return;
        }

        self.save_active_tab();
        self.open_tabs.remove(index);

        if self.open_tabs.is_empty() {
            self.open_tabs.push(OpenRequestTab::new());
            self.load_tab(0);
            return;
        }

        let next_index = if index < self.active_tab_index {
            self.active_tab_index - 1
        } else if index == self.active_tab_index {
            self.active_tab_index.min(self.open_tabs.len() - 1)
        } else {
            self.active_tab_index
        };
        self.load_tab(next_index);
    }

    /// 将请求编辑器字段同步到请求模型
    fn sync_request_fields(&mut self) {
        self.request.headers = self.headers.clone();
        self.request.params = self.params.clone();
        self.request.body_type = self.body_type.clone();
        self.request.body = self.body.clone();
    }

    /// 加载历史记录
    fn load_history(&mut self, conn: &rusqlite::Connection) {
        let store = ApiStore::new(conn);
        match store.get_recent_history(50) {
            Ok(history) => self.history = history,
            Err(e) => self.error = Some(format!("加载历史记录失败: {}", e)),
        }
    }

    /// 加载集合列表
    fn load_collections(&mut self, conn: &rusqlite::Connection) {
        let store = ApiStore::new(conn);
        match store.get_all_collections() {
            Ok(collections) => self.collections = collections,
            Err(e) => self.error = Some(format!("加载集合失败: {}", e)),
        }
    }

    /// 加载集合下的请求
    fn load_saved_requests(&mut self, conn: &rusqlite::Connection) {
        let store = ApiStore::new(conn);
        match store.get_requests_by_collection(self.selected_collection_id) {
            Ok(requests) => self.saved_requests = requests,
            Err(e) => self.error = Some(format!("加载请求失败: {}", e)),
        }
    }

    /// 加载环境列表
    fn load_environments(&mut self, conn: &rusqlite::Connection) {
        let store = ApiStore::new(conn);
        match store.get_all_environments() {
            Ok(environments) => {
                self.environments = environments;
                self.active_environment = self.environments.iter().find(|e| e.is_active).cloned();
            }
            Err(e) => self.error = Some(format!("加载环境失败: {}", e)),
        }
    }

    /// 加载环境变量
    fn load_environment_variables(&mut self, conn: &rusqlite::Connection) {
        let store = ApiStore::new(conn);
        match store.get_all_active_variables() {
            Ok(variables) => self.environment_variables = variables,
            Err(e) => self.error = Some(format!("加载环境变量失败: {}", e)),
        }
    }

    /// 获取变量替换器
    fn get_variable_replacer(&self) -> VariableReplacer {
        let mut variables = HashMap::new();
        for var in &self.environment_variables {
            if var.enabled {
                variables.insert(var.key.clone(), var.value.clone());
            }
        }
        VariableReplacer::new(variables)
    }

    /// 替换请求中的变量
    fn replace_variables_in_request(&self, request: &ApiRequest) -> ApiRequest {
        let replacer = self.get_variable_replacer();
        let mut replaced = request.clone();
        replaced.url = replacer.replace(&request.url);
        replaced.body = replacer.replace(&request.body);

        // 替换请求头中的变量
        for header in &mut replaced.headers {
            header.key = replacer.replace(&header.key);
            header.value = replacer.replace(&header.value);
        }

        // 替换查询参数中的变量
        for param in &mut replaced.params {
            param.key = replacer.replace(&param.key);
            param.value = replacer.replace(&param.value);
        }

        replaced
    }

    /// 渲染主界面
    pub fn render(&mut self, ui: &mut Ui, conn: &rusqlite::Connection) {
        // 顶部标题栏
        ui.horizontal(|ui| {
            ui.heading("🔍 API 调试工具");

            // 环境选择
            ui.separator();
            let env_name = self
                .active_environment
                .as_ref()
                .map(|e| e.name.clone())
                .unwrap_or_else(|| "全局".to_string());
            let envs = self.environments.clone();
            let mut env_to_activate: Option<i64> = None;
            let mut show_env_dialog = false;

            egui::ComboBox::from_id_salt("environment_select")
                .selected_text(format!("环境: {}", env_name))
                .show_ui(ui, |ui| {
                    for env in &envs {
                        if ui.selectable_label(env.is_active, &env.name).clicked() {
                            env_to_activate = Some(env.id);
                        }
                    }
                    ui.separator();
                    if ui.button("管理环境...").clicked() {
                        show_env_dialog = true;
                    }
                });

            if let Some(env_id) = env_to_activate {
                let store = ApiStore::new(conn);
                if let Err(e) = store.activate_environment(env_id) {
                    self.error = Some(format!("切换环境失败: {}", e));
                } else {
                    self.load_environments(conn);
                    self.load_environment_variables(conn);
                }
            }
            if show_env_dialog {
                self.show_environment_dialog = true;
            }

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

                // 新建请求按钮
                if ui.button(RichText::new("+ 新建请求").strong()).clicked() {
                    self.create_request_tab();
                }

                // 历史按钮
                if self.left_panel_view == LeftPanelView::History {
                    if ui
                        .button(RichText::new("集合").strong())
                        .on_hover_text("显示集合")
                        .clicked()
                    {
                        self.left_panel_view = LeftPanelView::Collections;
                        self.load_collections(conn);
                        self.load_saved_requests(conn);
                    }
                } else {
                    if ui
                        .button(RichText::new("历史").strong())
                        .on_hover_text("显示请求历史")
                        .clicked()
                    {
                        self.left_panel_view = LeftPanelView::History;
                        self.load_history(conn);
                    }
                }
            });
        });
        ui.separator();

        // 主内容区域 - 左右分栏（可拖拽调整宽度）
        let available_width = ui.available_width();
        // 历史列表所在的左侧区域最多占插件内容宽度的一半
        let max_left_width = (available_width * 0.5)
            .min(400.0);
        let min_left_width = max_left_width.min(180.0);

        ui.horizontal_top(|ui| {
            // 左侧面板（使用固定宽度）
            let left_width = self.left_panel_width.clamp(min_left_width, max_left_width);
            ui.vertical(|ui| {
                ui.set_min_width(left_width);
                ui.set_max_width(left_width);

                match self.left_panel_view {
                    LeftPanelView::Collections => {
                        self.render_collections_panel(ui, conn);
                    }
                    LeftPanelView::History => {
                        self.render_history_panel(ui, conn);
                    }
                }
            });

            // 可拖拽的分隔线
            let separator_rect = ui.available_rect_before_wrap();
            let separator_x = separator_rect.left();
            let separator_response = ui.allocate_rect(
                egui::Rect::from_min_max(
                    egui::pos2(separator_x, separator_rect.top()),
                    egui::pos2(separator_x + 8.0, separator_rect.bottom()),
                ),
                egui::Sense::drag(),
            );

            // 绘制分隔线
            let painter = ui.painter();
            let line_color = if separator_response.hovered() || separator_response.dragged() {
                ui.visuals().selection.bg_fill
            } else {
                ui.visuals().widgets.noninteractive.bg_stroke.color
            };
            painter.line_segment(
                [
                    egui::pos2(separator_x + 4.0, separator_rect.top()),
                    egui::pos2(separator_x + 4.0, separator_rect.bottom()),
                ],
                egui::Stroke::new(2.0, line_color),
            );

            // 处理拖拽
            if separator_response.dragged() {
                let delta = separator_response.drag_delta().x;
                self.left_panel_width =
                    (self.left_panel_width + delta).clamp(min_left_width, max_left_width);
            }

            // 鼠标样式
            if separator_response.hovered() || separator_response.dragged() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeColumn);
            }

            ui.add_space(4.0);

            // 右侧主内容
            ui.vertical(|ui| {
                self.render_main_content(ui, conn);
            });
        });

        // 弹窗
        if self.show_collection_dialog {
            self.render_collection_dialog(ui, conn);
        }
        if self.show_environment_dialog {
            self.render_environment_dialog(ui, conn);
        }
        if self.show_add_env_dialog {
            self.render_add_env_dialog(ui, conn);
        }
        if self.show_rename_env_dialog {
            self.render_rename_env_dialog(ui, conn);
        }
        if self.show_env_var_dialog {
            self.render_env_var_dialog(ui, conn);
        }
    }

    /// 渲染集合面板
    fn render_collections_panel(&mut self, ui: &mut Ui, conn: &rusqlite::Connection) {
        ui.horizontal(|ui| {
            ui.label(RichText::new("集合").strong());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.small_button("+").on_hover_text("新建集合").clicked() {
                    self.editing_collection_id = None;
                    self.collection_form_name.clear();
                    self.collection_form_description.clear();
                    self.show_collection_dialog = true;
                }
            });
        });
        ui.separator();

        egui::ScrollArea::vertical()
            .id_salt("collections_scroll")
            .show(ui, |ui| {
                // 全部请求（未分类）
                let all_selected = self.selected_collection_id.is_none();
                if ui.selectable_label(all_selected, "📁 全部请求").clicked() {
                    self.selected_collection_id = None;
                    self.load_saved_requests(conn);
                }

                ui.add_space(4.0);

                // 渲染集合树
                self.render_collection_tree(ui, conn, None, 0);
            });

        ui.add_space(10.0);

        // 显示当前集合下的请求
        ui.label(RichText::new("请求列表").strong());
        ui.separator();

        let saved_requests = self.saved_requests.clone();
        let mut request_to_load: Option<SavedApiRequest> = None;
        let mut request_to_delete: Option<i64> = None;

        egui::ScrollArea::vertical()
            .id_salt("requests_scroll")
            .show(ui, |ui| {
                if saved_requests.is_empty() {
                    ui.label(RichText::new("暂无保存的请求").color(Color32::GRAY).small());
                } else {
                    for request in &saved_requests {
                        let method_color = match request.method.as_str() {
                            "GET" => Color32::from_rgb(0, 150, 0),
                            "POST" => Color32::from_rgb(0, 0, 200),
                            "PUT" => Color32::from_rgb(200, 150, 0),
                            "DELETE" => Color32::from_rgb(200, 0, 0),
                            "PATCH" => Color32::from_rgb(200, 100, 0),
                            _ => Color32::GRAY,
                        };

                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new(request.method.as_str())
                                    .color(method_color)
                                    .strong()
                                    .monospace()
                                    .small(),
                            );

                            let request_label = ui
                                .add(
                                    egui::Label::new(&request.name)
                                        .sense(egui::Sense::click()),
                                )
                                .on_hover_text(&format!("{} {}", request.method, request.url));
                            if request_label.clicked() {
                                request_to_load = Some(request.clone());
                            }

                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if ui.small_button("×").on_hover_text("删除").clicked() {
                                        request_to_delete = Some(request.id);
                                    }
                                },
                            );
                        });
                    }
                }
            });

        if let Some(request) = request_to_load {
            self.open_saved_request(request);
        }

        if let Some(id) = request_to_delete {
            let store = ApiStore::new(conn);
            if let Err(e) = store.delete_saved_request(id) {
                self.error = Some(format!("删除请求失败: {}", e));
            } else {
                for tab in &mut self.open_tabs {
                    if tab.saved_request_id == Some(id) {
                        tab.saved_request_id = None;
                        tab.collection_id = None;
                    }
                }
                if self.saved_request_id == Some(id) {
                    self.saved_request_id = None;
                    self.request_collection_id = None;
                }
                self.load_saved_requests(conn);
            }
        }
    }

    /// 渲染集合树（递归）
    fn render_collection_tree(
        &mut self,
        ui: &mut Ui,
        conn: &rusqlite::Connection,
        parent_id: Option<i64>,
        depth: usize,
    ) {
        let collections: Vec<_> = self
            .collections
            .iter()
            .filter(|c| c.parent_id == parent_id)
            .cloned()
            .collect();

        let mut collection_to_select: Option<i64> = None;
        let mut collection_to_edit: Option<ApiCollection> = None;

        for collection in &collections {
            let is_selected = self.selected_collection_id == Some(collection.id);
            let indent = "  ".repeat(depth);
            let label_text = format!("{}📁 {}", indent, collection.name);

            ui.horizontal(|ui| {
                if ui.selectable_label(is_selected, &label_text).clicked() {
                    collection_to_select = Some(collection.id);
                }

                // 编辑按钮
                if ui.small_button("...").clicked() {
                    collection_to_edit = Some(collection.clone());
                }
            });

            // 递归渲染子集合
            self.render_collection_tree(ui, conn, Some(collection.id), depth + 1);
        }

        if let Some(id) = collection_to_select {
            self.selected_collection_id = Some(id);
            self.load_saved_requests(conn);
        }

        if let Some(collection) = collection_to_edit {
            self.editing_collection_id = Some(collection.id);
            self.collection_form_name = collection.name;
            self.collection_form_description = collection.description;
            self.show_collection_dialog = true;
        }
    }

    /// 获取集合及其所有后代集合的 ID
    fn collection_and_descendant_ids(&self, collection_id: i64) -> Vec<i64> {
        let mut ids = vec![collection_id];
        let mut index = 0;

        while index < ids.len() {
            let parent_id = ids[index];
            for collection in &self.collections {
                if collection.parent_id == Some(parent_id) && !ids.contains(&collection.id) {
                    ids.push(collection.id);
                }
            }
            index += 1;
        }

        ids
    }

    /// 渲染集合管理弹窗
    fn render_collection_dialog(&mut self, ui: &mut Ui, conn: &rusqlite::Connection) {
        egui::Window::new(if self.editing_collection_id.is_some() {
            "编辑集合"
        } else {
            "新建集合"
        })
        .collapsible(false)
        .resizable(false)
        .show(ui.ctx(), |ui| {
            ui.horizontal(|ui| {
                ui.label("集合名称:");
                ui.text_edit_singleline(&mut self.collection_form_name);
            });

            ui.horizontal(|ui| {
                ui.label("描述:");
                ui.text_edit_singleline(&mut self.collection_form_description);
            });

            ui.add_space(10.0);

            ui.horizontal(|ui| {
                if ui.button("保存").clicked() {
                    if self.collection_form_name.is_empty() {
                        self.error = Some("集合名称不能为空".to_string());
                    } else {
                        let store = ApiStore::new(conn);
                        if let Some(id) = self.editing_collection_id {
                            if let Err(e) = store.update_collection(
                                id,
                                &self.collection_form_name,
                                &self.collection_form_description,
                            ) {
                                self.error = Some(format!("更新集合失败: {}", e));
                            }
                        } else if let Err(e) =
                            store.create_collection(&self.collection_form_name, None)
                        {
                            self.error = Some(format!("创建集合失败: {}", e));
                        }
                        self.load_collections(conn);
                        self.show_collection_dialog = false;
                    }
                }

                if self.editing_collection_id.is_some() {
                    if ui
                        .button(RichText::new("删除").color(Color32::from_rgb(200, 0, 0)))
                        .clicked()
                    {
                        if let Some(id) = self.editing_collection_id {
                            let deleted_collection_ids =
                                self.collection_and_descendant_ids(id);
                            let store = ApiStore::new(conn);
                            if let Err(e) = store.delete_collection(id) {
                                self.error = Some(format!("删除集合失败: {}", e));
                            } else {
                                for tab in &mut self.open_tabs {
                                    if tab
                                        .collection_id
                                        .is_some_and(|tab_id| deleted_collection_ids.contains(&tab_id))
                                    {
                                        tab.collection_id = None;
                                    }
                                }
                                if self.request_collection_id.is_some_and(|tab_id| {
                                    deleted_collection_ids.contains(&tab_id)
                                }) {
                                    self.request_collection_id = None;
                                }
                                self.load_collections(conn);
                                self.show_collection_dialog = false;
                            }
                        }
                    }
                }

                if ui.button("取消").clicked() {
                    self.show_collection_dialog = false;
                }
            });
        });
    }

    /// 渲染环境管理弹窗
    fn render_environment_dialog(&mut self, ui: &mut Ui, conn: &rusqlite::Connection) {
        let mut env_to_delete: Option<i64> = None;
        let mut close_dialog = false;

        egui::Window::new("环境管理")
            .collapsible(false)
            .resizable(true)
            .default_width(500.0)
            .show(ui.ctx(), |ui| {
                ui.label("环境列表:");
                ui.separator();

                egui::Grid::new("environments_grid")
                    .striped(true)
                    .num_columns(3)
                    .show(ui, |ui| {
                        // 表头
                        ui.label(RichText::new("环境名称").strong());
                        ui.label(RichText::new("状态").strong());
                        ui.label(RichText::new("操作").strong());
                        ui.end_row();

                        let envs = self.environments.clone();
                        for env in &envs {
                            ui.label(&env.name);

                            let status = if env.is_active {
                                "当前选中"
                            } else if env.is_default {
                                "默认"
                            } else {
                                ""
                            };
                            ui.label(status);

                            ui.horizontal(|ui| {
                                if ui.small_button("编辑变量").clicked() {
                                    self.editing_environment_id = Some(env.id);
                                    self.show_env_var_dialog = true;
                                    self.load_environment_variables(conn);
                                }
                                if !env.is_default {
                                    if ui.small_button("重命名").clicked() {
                                        self.editing_environment_id = Some(env.id);
                                        self.env_form_name = env.name.clone();
                                        self.show_rename_env_dialog = true;
                                    }
                                    if ui
                                        .small_button(
                                            RichText::new("删除")
                                                .color(Color32::from_rgb(200, 0, 0)),
                                        )
                                        .clicked()
                                    {
                                        env_to_delete = Some(env.id);
                                    }
                                }
                            });

                            ui.end_row();
                        }
                    });

                ui.add_space(10.0);

                ui.horizontal(|ui| {
                    if ui.button("+ 新增环境").clicked() {
                        self.show_add_env_dialog = true;
                        self.env_form_name = "新环境".to_string();
                        self.editing_environment_id = None;
                    }

                    if ui.button("关闭").clicked() {
                        close_dialog = true;
                    }
                });
            });

        // 处理删除环境
        if let Some(id) = env_to_delete {
            let store = ApiStore::new(conn);
            if let Err(e) = store.delete_environment(id) {
                self.error = Some(format!("删除环境失败: {}", e));
            } else {
                self.load_environments(conn);
            }
        }

        if close_dialog {
            self.show_environment_dialog = false;
        }
    }

    /// 渲染新增环境弹窗
    fn render_add_env_dialog(&mut self, ui: &mut Ui, conn: &rusqlite::Connection) {
        let mut close_dialog = false;
        egui::Window::new("新增环境")
            .collapsible(false)
            .resizable(false)
            .show(ui.ctx(), |ui| {
                ui.horizontal(|ui| {
                    ui.label("环境名称:");
                    ui.text_edit_singleline(&mut self.env_form_name);
                });
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if ui.button("创建").clicked() {
                        if self.env_form_name.is_empty() {
                            self.error = Some("环境名称不能为空".to_string());
                        } else {
                            let store = ApiStore::new(conn);
                            match store.create_environment(&self.env_form_name) {
                                Ok(_) => {
                                    self.load_environments(conn);
                                    self.env_form_name.clear();
                                    close_dialog = true;
                                }
                                Err(e) => {
                                    self.error = Some(format!("创建环境失败: {}", e));
                                }
                            }
                        }
                    }
                    if ui.button("取消").clicked() {
                        self.env_form_name.clear();
                        close_dialog = true;
                    }
                });
            });
        if close_dialog {
            self.show_add_env_dialog = false;
        }
    }

    /// 渲染重命名环境弹窗
    fn render_rename_env_dialog(&mut self, ui: &mut Ui, conn: &rusqlite::Connection) {
        let mut close_dialog = false;
        egui::Window::new("重命名环境")
            .collapsible(false)
            .resizable(false)
            .show(ui.ctx(), |ui| {
                ui.horizontal(|ui| {
                    ui.label("环境名称:");
                    ui.text_edit_singleline(&mut self.env_form_name);
                });
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if ui.button("保存").clicked() {
                        if self.env_form_name.is_empty() {
                            self.error = Some("环境名称不能为空".to_string());
                        } else {
                            let store = ApiStore::new(conn);
                            if let Some(id) = self.editing_environment_id {
                                if let Err(e) = store.update_environment(id, &self.env_form_name) {
                                    self.error = Some(format!("更新环境名称失败: {}", e));
                                } else {
                                    self.load_environments(conn);
                                    self.env_form_name.clear();
                                    self.editing_environment_id = None;
                                    close_dialog = true;
                                }
                            }
                        }
                    }
                    if ui.button("取消").clicked() {
                        self.env_form_name.clear();
                        self.editing_environment_id = None;
                        close_dialog = true;
                    }
                });
            });
        if close_dialog {
            self.show_rename_env_dialog = false;
        }
    }

    /// 渲染环境变量编辑弹窗
    fn render_env_var_dialog(&mut self, ui: &mut Ui, conn: &rusqlite::Connection) {
        let env_id = self.editing_environment_id.unwrap_or(0);
        let env_name = self
            .environments
            .iter()
            .find(|e| e.id == env_id)
            .map(|e| e.name.clone())
            .unwrap_or_default();

        egui::Window::new(format!("编辑环境: {}", env_name))
            .collapsible(false)
            .resizable(true)
            .default_width(500.0)
            .show(ui.ctx(), |ui| {
                // 环境名称编辑（非默认环境）
                let is_default = self
                    .environments
                    .iter()
                    .find(|e| e.id == env_id)
                    .map(|e| e.is_default)
                    .unwrap_or(false);

                if !is_default {
                    ui.horizontal(|ui| {
                        ui.label("环境名称:");
                        // 这里可以添加名称编辑功能
                    });
                    ui.add_space(5.0);
                }

                ui.label("变量列表:");
                ui.separator();

                egui::Grid::new("env_vars_grid")
                    .striped(true)
                    .num_columns(4)
                    .min_col_width(100.0)
                    .show(ui, |ui| {
                        // 表头
                        ui.label(RichText::new("启用").strong());
                        ui.label(RichText::new("Key").strong());
                        ui.label(RichText::new("Value").strong());
                        ui.label(RichText::new("操作").strong());
                        ui.end_row();

                        let vars = self.environment_variables.clone();
                        for var in &vars {
                            if var.environment_id == env_id {
                                let mut enabled = var.enabled;
                                ui.checkbox(&mut enabled, "");
                                // TODO: 更新 enabled 状态
                                ui.label(&var.key);
                                ui.label(&var.value);

                                ui.horizontal(|ui| {
                                    if ui.small_button("编辑").clicked() {
                                        self.editing_env_var_id = Some(var.id);
                                        self.env_var_form_key = var.key.clone();
                                        self.env_var_form_value = var.value.clone();
                                    }
                                    if ui
                                        .small_button(
                                            RichText::new("删除")
                                                .color(Color32::from_rgb(200, 0, 0)),
                                        )
                                        .clicked()
                                    {
                                        let store = ApiStore::new(conn);
                                        if let Err(e) = store.delete_environment_variable(var.id) {
                                            self.error = Some(format!("删除变量失败: {}", e));
                                        } else {
                                            self.load_environment_variables(conn);
                                        }
                                    }
                                });

                                ui.end_row();
                            }
                        }
                    });

                ui.add_space(10.0);

                // 添加/编辑变量
                ui.horizontal(|ui| {
                    ui.label("Key:");
                    ui.text_edit_singleline(&mut self.env_var_form_key);
                    ui.label("Value:");
                    ui.text_edit_singleline(&mut self.env_var_form_value);

                    let btn_text = if self.editing_env_var_id.is_some() {
                        "更新"
                    } else {
                        "添加"
                    };

                    if ui.button(btn_text).clicked() {
                        if self.env_var_form_key.is_empty() {
                            self.error = Some("变量名不能为空".to_string());
                        } else {
                            let store = ApiStore::new(conn);
                            if let Some(var_id) = self.editing_env_var_id {
                                if let Err(e) = store.update_environment_variable(
                                    var_id,
                                    &self.env_var_form_key,
                                    &self.env_var_form_value,
                                    true,
                                ) {
                                    self.error = Some(format!("更新变量失败: {}", e));
                                }
                            } else if let Err(e) = store.create_environment_variable(
                                env_id,
                                &self.env_var_form_key,
                                &self.env_var_form_value,
                            ) {
                                self.error = Some(format!("添加变量失败: {}", e));
                            }
                            self.env_var_form_key.clear();
                            self.env_var_form_value.clear();
                            self.editing_env_var_id = None;
                            self.load_environment_variables(conn);
                        }
                    }

                    if self.editing_env_var_id.is_some() && ui.button("取消").clicked() {
                        self.env_var_form_key.clear();
                        self.env_var_form_value.clear();
                        self.editing_env_var_id = None;
                    }
                });

                ui.add_space(10.0);

                if ui.button("关闭").clicked() {
                    self.show_env_var_dialog = false;
                    self.editing_env_var_id = None;
                    self.env_var_form_key.clear();
                    self.env_var_form_value.clear();
                }
            });
    }

    /// 渲染主内容区域
    fn render_main_content(&mut self, ui: &mut Ui, conn: &rusqlite::Connection) {
        self.render_open_request_tabs(ui);

        // 请求配置区域
        self.render_request_config(ui);

        ui.add_space(10.0);

        // 请求头/体配置
        self.render_request_tabs(ui);

        ui.add_space(10.0);

        // 发送和保存按钮
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

            // 保存到集合按钮
            let save_btn_text = if self.selected_collection_id.is_some() {
                "保存到集合"
            } else {
                "保存请求"
            };

            if ui.button(RichText::new(save_btn_text).strong()).clicked() {
                self.save_request_to_collection(conn);
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
            ui.label("请求名称:");
            let name_response = ui.add(
                egui::TextEdit::singleline(&mut self.request.name)
                    .hint_text("输入请求名称")
                    .desired_width(260.0),
            );
            if name_response.changed() {
                self.is_default_name = false;
            }
        });

        ui.add_space(5.0);

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

    /// 渲染已打开的请求标签页
    fn render_open_request_tabs(&mut self, ui: &mut Ui) {
        let tab_names: Vec<String> = self
            .open_tabs
            .iter()
            .enumerate()
            .map(|(index, tab)| {
                if index == self.active_tab_index {
                    Self::request_display_name(&self.request, self.is_default_name)
                } else {
                    Self::request_display_name(&tab.request, tab.is_default_name)
                }
            })
            .collect();
        let mut tab_to_activate = None;
        let mut tab_to_close = None;

        egui::ScrollArea::horizontal()
            .id_salt("open_request_tabs")
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    for (index, name) in tab_names.iter().enumerate() {
                        let is_active = index == self.active_tab_index;
                        let response = ui
                            .selectable_label(is_active, name)
                            .on_hover_text(name);
                        if response.clicked() {
                            tab_to_activate = Some(index);
                        }

                        if ui
                            .small_button("×")
                            .on_hover_text("关闭请求标签页")
                            .clicked()
                        {
                            tab_to_close = Some(index);
                        }
                    }
                });
            });
        ui.separator();

        if let Some(index) = tab_to_close {
            self.close_request_tab(index);
        } else if let Some(index) = tab_to_activate {
            self.activate_request_tab(index);
        }
    }

    /// 获取请求标签页及集合列表使用的完整显示名称
    fn request_display_name(request: &ApiRequest, is_default_name: bool) -> String {
        if request.name.trim().is_empty() || (is_default_name && request.name == "New Request") {
            if request.url.is_empty() {
                "新建请求".to_string()
            } else {
                request.url.clone()
            }
        } else {
            request.name.clone()
        }
    }

    /// 获取保存到数据库的请求名称，不截断 URL 或用户输入
    fn saved_request_name(request: &ApiRequest, is_default_name: bool) -> String {
        if request.name.trim().is_empty() || (is_default_name && request.name == "New Request") {
            if request.url.is_empty() {
                "未命名请求".to_string()
            } else {
                request.url.clone()
            }
        } else {
            request.name.trim().to_string()
        }
    }

    /// 按显示字符数省略过长 URL，并保留完整 URL 作为悬浮提示
    fn truncate_url(url: &str, max_chars: usize) -> String {
        let chars: Vec<char> = url.chars().collect();
        if chars.len() <= max_chars {
            return url.to_string();
        }

        if max_chars <= 3 {
            return chars.into_iter().take(max_chars).collect();
        }

        let prefix: String = chars.into_iter().take(max_chars - 3).collect();
        format!("{}...", prefix)
    }

    /// 根据历史列表宽度计算 URL 列宽和近似可显示字符数
    fn history_url_layout(available_width: f32) -> (f32, usize) {
        let column_width = (available_width * 0.32).clamp(40.0, 240.0);
        let max_chars = if column_width < 80.0 {
            10
        } else if column_width < 120.0 {
            16
        } else if column_width < 180.0 {
            24
        } else {
            36
        };
        (column_width, max_chars)
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
        let mut to_remove = Vec::new();

        egui::Grid::new("params_grid")
            .striped(true)
            .num_columns(3)
            .min_col_width(150.0)
            .show(ui, |ui| {
                // 表头
                ui.label(RichText::new("启用").strong());
                ui.label(RichText::new("Key").strong());
                ui.label(RichText::new("Value").strong());
                ui.end_row();

                // 查询参数列表
                for (i, param) in self.params.iter_mut().enumerate() {
                    ui.checkbox(&mut param.enabled, "");
                    ui.add(
                        egui::TextEdit::singleline(&mut param.key)
                            .hint_text("Parameter Name")
                            .desired_width(200.0),
                    );
                    ui.add(
                        egui::TextEdit::singleline(&mut param.value)
                            .hint_text("Parameter Value")
                            .desired_width(ui.available_width() - 60.0),
                    );

                    if ui.button("删除").clicked() {
                        to_remove.push(i);
                    }
                    ui.end_row();
                }
            });

        // 删除标记的查询参数
        for i in to_remove.into_iter().rev() {
            self.params.remove(i);
        }

        ui.add_space(5.0);
        if ui.button("添加查询参数").clicked() {
            self.params.push(HeaderEntry::empty());
        }
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
                    egui::TextEdit::multiline(&mut formatted_body.as_str()).code_editor(),
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
        let (url_column_width, url_max_chars) = Self::history_url_layout(ui.available_width());

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
                    .num_columns(6)
                    .spacing([8.0, 4.0])
                    .min_col_width(40.0)
                    .show(ui, |ui| {
                        // 表头
                        ui.strong("名称");
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

                            // 名称列
                            let name_label = ui
                                .add(
                                    egui::Label::new(&history.name)
                                        .sense(egui::Sense::click()),
                                )
                                .on_hover_text(&history.name);
                            if name_label.clicked() {
                                to_load = Some(history.id);
                            }

                            // 方法列
                            ui.label(
                                RichText::new(&history.method)
                                    .color(method_color)
                                    .strong()
                                    .monospace()
                                    .small(),
                            );

                            // URL 列（可点击）
                            let url_display = Self::truncate_url(&history.url, url_max_chars);
                            let url_label = ui
                                .add_sized(
                                    [url_column_width, 20.0],
                                    egui::Label::new(
                                        RichText::new(url_display)
                                            .small()
                                            .color(Color32::from_rgb(100, 149, 237)),
                                    )
                                    .sense(egui::Sense::click()),
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
                            ui.label(RichText::new(&elapsed_display).color(Color32::GRAY).small());

                            // 删除按钮
                            if ui
                                .small_button(
                                    RichText::new("×").color(Color32::from_rgb(200, 0, 0)),
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
            Ok(Some((name, method, url, headers, params, body))) => {
                self.create_request_tab();
                self.request.name = if name.is_empty() { url.clone() } else { name };
                self.saved_request_id = None;
                self.request_collection_id = None;
                self.is_default_name = false;

                if let Some(m) = HttpMethod::from_str(&method) {
                    self.request.method = m;
                }
                self.request.url = url;

                // 解析请求头
                if !headers.trim().is_empty() {
                    if let Ok(parsed_headers) = serde_json::from_str::<Vec<HeaderEntry>>(&headers)
                    {
                        self.headers = parsed_headers;
                    }
                }

                // 解析查询参数
                if !params.trim().is_empty() {
                    if let Ok(parsed_params) = serde_json::from_str::<Vec<HeaderEntry>>(&params) {
                        self.params = parsed_params;
                    }
                }

                // 解析请求体
                self.body = body;
                if self.body.is_empty() {
                    self.body_type = BodyType::None;
                } else {
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
        self.sync_request_fields();

        // 替换环境变量
        let replaced_request = self.replace_variables_in_request(&self.request);

        // 发送请求（使用 build_url 构建包含查询参数的完整 URL）
        if let Some(client) = &self.client {
            let mut request_with_params = replaced_request.clone();
            request_with_params.url = replaced_request.build_url();

            match client.send(&request_with_params) {
                Ok(response) => {
                    // 保存到历史记录
                    let store = ApiStore::new(conn);
                    let headers_json =
                        serde_json::to_string(&self.request.headers).unwrap_or_default();
                    let params_json =
                        serde_json::to_string(&self.request.params).unwrap_or_default();

                    match store.save_history(
                        &self.request.id,
                        &Self::request_display_name(&self.request, self.is_default_name),
                        self.request.method.as_str(),
                        &self.request.url,
                        &headers_json,
                        &params_json,
                        self.request.body_type.as_str(),
                        &self.request.body,
                        Some(i32::from(response.status_code)),
                        Some(&response.body),
                        Some(i64::try_from(response.elapsed_ms).unwrap_or(i64::MAX)),
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

    /// 保存请求到集合
    fn save_request_to_collection(&mut self, conn: &rusqlite::Connection) {
        // 更新请求配置
        self.sync_request_fields();

        let store = ApiStore::new(conn);
        let headers_json = serde_json::to_string(&self.request.headers).unwrap_or_default();
        let params_json = serde_json::to_string(&self.request.params).unwrap_or_default();

        let name = Self::saved_request_name(&self.request, self.is_default_name);

        let collection_id = if self.saved_request_id.is_some() {
            self.request_collection_id
        } else {
            self.selected_collection_id
        };
        let result = if let Some(id) = self.saved_request_id {
            store.update_request(
                id,
                collection_id,
                &name,
                self.request.method.as_str(),
                &self.request.url,
                &headers_json,
                &params_json,
                self.request.body_type.as_str(),
                &self.request.body,
            )
        } else {
            match store.save_request(
                collection_id,
                &name,
                self.request.method.as_str(),
                &self.request.url,
                &headers_json,
                &params_json,
                self.request.body_type.as_str(),
                &self.request.body,
            ) {
                Ok(id) => {
                    self.saved_request_id = Some(id);
                    self.request_collection_id = collection_id;
                    Ok(())
                }
                Err(e) => Err(e),
            }
        };

        match result {
            Ok(()) => {
                self.request.name = name;
                self.is_default_name = false;
                self.load_saved_requests(conn);
                self.save_active_tab();
                log::info!("请求已保存到集合");
            }
            Err(e) => {
                self.error = Some(format!("保存请求失败: {}", e));
            }
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_display_name_keeps_full_custom_name() {
        let long_name = "这是一个超过原有限制的完整请求名称，用于验证不会被截断";
        let request = ApiRequest {
            name: long_name.to_string(),
            ..ApiRequest::new()
        };

        assert_eq!(
            ApiTesterUi::request_display_name(&request, false),
            long_name
        );
        assert_eq!(
            ApiTesterUi::saved_request_name(&request, false),
            long_name
        );
    }

    #[test]
    fn request_name_falls_back_to_full_url() {
        let long_url = "https://example.com/api/requests/with/a/very/long/path?param=full-value";
        let request = ApiRequest {
            url: long_url.to_string(),
            ..ApiRequest::new()
        };

        assert_eq!(
            ApiTesterUi::request_display_name(&request, true),
            long_url
        );
        assert_eq!(ApiTesterUi::saved_request_name(&request, true), long_url);
    }

    #[test]
    fn custom_new_request_name_is_not_replaced_by_url() {
        let request = ApiRequest {
            name: "New Request".to_string(),
            url: "https://example.com/api".to_string(),
            ..ApiRequest::new()
        };

        assert_eq!(
            ApiTesterUi::saved_request_name(&request, false),
            "New Request"
        );
    }

    #[test]
    fn long_url_is_truncated_with_ellipsis() {
        assert_eq!(
            ApiTesterUi::truncate_url("https://example.com/very-long-path", 20),
            "https://example.c..."
        );
        assert_eq!(ApiTesterUi::truncate_url("short", 20), "short");
    }
}
