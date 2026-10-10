//! 常用文件编辑器界面渲染
//!
//! 左侧为目录 / 常用文件列表（分隔线可拖动调宽），右侧为多标签文本编辑器。
//! 所有交互（打开、保存、关闭、重载）都通过渲染期间收集的延迟动作执行，
//! 避免 egui 渲染过程中的借用冲突。

use std::path::{Path, PathBuf};

use egui::{Context, RichText, Ui};
use rusqlite::Connection;

use super::document;
use super::{models, store};

/// 左侧列表最小宽度（像素）
const LIST_MIN_WIDTH: f32 = 180.0;
/// 左侧列表最大宽度（像素）
const LIST_MAX_WIDTH: f32 = 520.0;
/// 标签栏、列表等控件的唯一标识前缀
const LIST_PANEL_ID: &str = "file_editor_list";
/// 编辑器区域可编辑行数（撑开滚动区）
const EDITOR_DESIRED_ROWS: usize = 24;

/// 当前打开的弹窗
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Dialog {
    /// 无弹窗
    None,
    /// 添加常用文件
    AddFile,
    /// 重命名显示名
    RenameFile,
    /// 目录管理
    ManageDirectories,
}

/// 添加文件表单
#[derive(Debug, Clone, Default)]
struct AddFileForm {
    /// 文件路径输入
    path: String,
    /// 显示名输入
    alias: String,
    /// 所属目录
    directory_id: Option<i64>,
    /// 校验错误提示
    error: Option<String>,
}

/// 目录管理表单
#[derive(Debug, Clone, Default)]
struct DirectoryForm {
    /// 新建目录名称
    new_name: String,
    /// 正在重命名的目录 id
    rename_target: Option<i64>,
    /// 重命名输入
    rename_name: String,
    /// 校验错误提示
    error: Option<String>,
}

/// 渲染期间收集、渲染后统一执行的延迟动作
#[derive(Debug, Clone)]
enum Action {
    /// 打开文件（必要时新建标签）
    Open {
        file_id: i64,
        path: PathBuf,
        display_name: String,
    },
    /// 激活标签
    ActivateTab(usize),
    /// 请求关闭标签（脏标签会先弹确认框）
    CloseTab(usize),
    /// 保存当前标签
    SaveActive,
    /// 放弃修改并重新读取当前标签
    ReloadActive,
    /// 从常用列表移除条目
    RemoveFile(i64),
    /// 在资源管理器中显示
    Reveal(PathBuf),
    /// 打开添加文件弹窗
    StartAddFile(Option<i64>),
    /// 打开重命名显示名弹窗（条目 id、当前显示名）
    StartRenameFile(i64, Option<String>),
    /// 打开目录管理弹窗
    StartManageDirectories,
    /// 关闭确认弹窗的选择结果
    FinishClose(models::CloseDecision),
}

/// 关闭某个标签后应激活的标签下标
///
/// 关闭的是当前标签时优先激活右邻标签，没有右邻则回到最后一个标签。
fn active_after_close(active: Option<usize>, closed: usize, remaining: usize) -> Option<usize> {
    if remaining == 0 {
        return None;
    }

    match active {
        Some(current) if current < closed => Some(current),
        Some(current) if current > closed => Some(current - 1),
        _ => Some(closed.min(remaining - 1)),
    }
}

/// 是否应该把当前左栏宽度写回数据库
///
/// 拖动过程中（指针按下）只更新内存宽度，指针抬起后再落库，避免每帧写库；
/// 比较对象是"上次已保存的宽度"，因此松手那一帧即使宽度不再变化也会触发保存。
fn should_persist_width(pointer_down: bool, current: f32, last_saved: f32) -> bool {
    !pointer_down && (current - last_saved).abs() >= 0.5
}

/// 常用文件编辑器界面状态
pub struct FileEditorUi {
    /// 自定义目录
    directories: Vec<models::FileDirectory>,
    /// 常用文件条目
    files: Vec<models::FavoriteFile>,
    /// 磁盘上已不存在的条目 id
    missing_files: Vec<i64>,
    /// 已打开的标签
    tabs: Vec<models::EditorTab>,
    /// 当前激活标签
    active_tab: Option<usize>,
    /// 列表选中条目
    selected_file_id: Option<i64>,
    /// 待确认关闭的标签下标
    pending_close: Option<usize>,
    /// 当前弹窗
    dialog: Dialog,
    /// 添加文件表单
    add_form: AddFileForm,
    /// 重命名显示名表单（条目 id、输入内容）
    rename_form: Option<(i64, String)>,
    /// 目录管理表单
    dir_form: DirectoryForm,
    /// 左栏宽度
    list_width: f32,
    /// 最近一次写入数据库的左栏宽度
    last_saved_width: f32,
    /// 状态栏文本
    status: String,
    /// 数据库是否可写（只读时在顶部显示警告）
    storage_writable: bool,
    /// 是否已从数据库加载
    initialized: bool,
}

impl FileEditorUi {
    /// 创建界面状态
    pub fn new() -> Self {
        Self {
            directories: Vec::new(),
            files: Vec::new(),
            missing_files: Vec::new(),
            tabs: Vec::new(),
            active_tab: None,
            selected_file_id: None,
            pending_close: None,
            dialog: Dialog::None,
            add_form: AddFileForm::default(),
            rename_form: None,
            dir_form: DirectoryForm::default(),
            list_width: store::DEFAULT_LIST_WIDTH,
            last_saved_width: store::DEFAULT_LIST_WIDTH,
            status: "就绪".to_string(),
            storage_writable: true,
            initialized: false,
        }
    }

    /// 首次进入插件时加载数据库中的目录、文件条目与界面宽度
    pub fn init(&mut self, conn: &Connection) {
        self.reload_data(conn);

        match store::load_list_width(conn) {
            Ok(width) => {
                self.list_width = width;
                self.last_saved_width = width;
            }
            Err(error) => log::warn!("读取列表宽度失败，使用默认值: {}", error),
        }

        self.initialized = true;
    }

    /// 渲染插件界面
    pub fn render(&mut self, ui: &mut Ui, conn: &Connection) {
        let mut actions = Vec::new();

        if !self.storage_writable {
            self.render_storage_warning(ui);
        }

        if self.can_save_active_tab() && self.save_shortcut_pressed(ui.ctx()) {
            actions.push(Action::SaveActive);
        }

        self.render_toolbar(ui, &mut actions);
        ui.separator();

        let panel = egui::SidePanel::left(LIST_PANEL_ID)
            .resizable(true)
            .default_width(self.list_width)
            .width_range(LIST_MIN_WIDTH..=LIST_MAX_WIDTH)
            .show_inside(ui, |ui| {
                self.render_file_list(ui, &mut actions);
            });
        self.sync_list_width(panel.response.rect.width(), ui.ctx(), conn);

        self.render_editor(ui, &mut actions);
        self.render_dialogs(ui.ctx(), conn);

        self.apply_actions(actions, conn);
    }

    /// 设置数据库是否可写（由插件在初始化探测后传入）
    pub fn set_storage_writable(&mut self, writable: bool) {
        self.storage_writable = writable;
    }

    /// 数据库不可写时的顶部警告
    ///
    /// 常见原因：程序运行在带低完整性标签或受限权限的目录下（例如被沙箱标记过的工作区），
    /// 此时低完整性进程无法写入 `%APPDATA%` 下的数据库，SQLite 只报 "readonly database"。
    fn render_storage_warning(&self, ui: &mut Ui) {
        ui.horizontal_wrapped(|ui| {
            ui.colored_label(
                egui::Color32::from_rgb(0xE0, 0x60, 0x60),
                "⚠ 数据库不可写：分组与常用文件列表无法保存。请把程序放在普通目录运行\
                 （例如 C:\\Program Files\\Tools-box），不要运行在带低完整性标签或受限权限的目录下。",
            );
        });
        ui.separator();
    }

    /// 当前是否存在可保存的标签（无标签或只读标签时不应响应保存快捷键）
    fn can_save_active_tab(&self) -> bool {
        self.active_tab
            .and_then(|index| self.tabs.get(index))
            .is_some_and(|tab| !tab.read_only)
    }

    /// Ctrl+S 是否被按下（消费按键，避免继续向下传播）
    fn save_shortcut_pressed(&self, ctx: &Context) -> bool {
        ctx.input_mut(|input| input.consume_key(egui::Modifiers::CTRL, egui::Key::S))
    }

    /// 重新加载目录与文件条目
    fn reload_data(&mut self, conn: &Connection) {
        match store::load_directories(conn) {
            Ok(directories) => self.directories = directories,
            Err(error) => {
                log::error!("加载常用文件目录失败: {}", error);
                self.status = format!("加载目录失败: {error}");
            }
        }

        match store::load_files(conn) {
            Ok(files) => self.files = files,
            Err(error) => {
                log::error!("加载常用文件列表失败: {}", error);
                self.status = format!("加载文件列表失败: {error}");
            }
        }

        self.refresh_missing_files();
    }

    /// 刷新磁盘上已不存在的条目
    fn refresh_missing_files(&mut self) {
        self.missing_files = self
            .files
            .iter()
            .filter(|file| !Path::new(&file.path).is_file())
            .map(|file| file.id)
            .collect();
    }

    /// 渲染顶部工具条
    fn render_toolbar(&mut self, ui: &mut Ui, actions: &mut Vec<Action>) {
        ui.horizontal(|ui| {
            if ui
                .button("＋ 添加文件")
                .on_hover_text("把常用配置文件加入列表")
                .clicked()
            {
                actions.push(Action::StartAddFile(self.selected_directory()));
            }

            if ui
                .button("🗂 目录管理")
                .on_hover_text("新建、重命名、删除自定义目录")
                .clicked()
            {
                actions.push(Action::StartManageDirectories);
            }

            ui.separator();

            let can_save = self
                .active_tab
                .and_then(|index| self.tabs.get(index))
                .is_some_and(|tab| !tab.read_only);
            if ui
                .add_enabled(can_save, egui::Button::new("💾 保存"))
                .on_hover_text("保存当前标签（Ctrl+S）")
                .clicked()
            {
                actions.push(Action::SaveActive);
            }

            let has_tab = self.active_tab.is_some();
            if ui
                .add_enabled(has_tab, egui::Button::new("⟳ 重载"))
                .on_hover_text("放弃修改并重新从磁盘读取")
                .clicked()
            {
                actions.push(Action::ReloadActive);
            }
        });
    }

    /// 当前选中条目所属目录（用于添加文件时预选目录）
    fn selected_directory(&self) -> Option<i64> {
        self.selected_file_id
            .and_then(|id| self.files.iter().find(|file| file.id == id))
            .and_then(|file| file.directory_id)
    }

    /// 渲染左侧目录 / 文件列表
    fn render_file_list(&mut self, ui: &mut Ui, actions: &mut Vec<Action>) {
        let directories = self.directories.clone();
        let files = self.files.clone();

        egui::ScrollArea::vertical()
            .id_salt("file_editor_list_scroll")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for directory in &directories {
                    let file_count = files
                        .iter()
                        .filter(|file| file.directory_id == Some(directory.id))
                        .count();
                    egui::CollapsingHeader::new(
                        RichText::new(format!("📁 {}（{file_count}）", directory.name)).strong(),
                    )
                    .id_salt(format!("file_editor_dir_{}", directory.id))
                    .default_open(true)
                    .show(ui, |ui| {
                        let mut empty = true;
                        for file in files
                            .iter()
                            .filter(|file| file.directory_id == Some(directory.id))
                        {
                            empty = false;
                            self.render_file_row(ui, file, actions);
                        }

                        if empty {
                            ui.weak("（空目录）");
                        }
                    });
                }

                let ungrouped: Vec<&models::FavoriteFile> = files
                    .iter()
                    .filter(|file| file.directory_id.is_none())
                    .collect();
                if !ungrouped.is_empty() || directories.is_empty() {
                    egui::CollapsingHeader::new(
                        RichText::new(format!("📁 未分组（{}）", ungrouped.len())).strong(),
                    )
                    .id_salt("file_editor_dir_ungrouped")
                    .default_open(true)
                    .show(ui, |ui| {
                        if ungrouped.is_empty() {
                            ui.weak("点击「＋ 添加文件」加入常用配置文件");
                        }
                        for file in ungrouped {
                            self.render_file_row(ui, file, actions);
                        }
                    });
                }
            });
    }

    /// 渲染单个文件条目
    fn render_file_row(&self, ui: &mut Ui, file: &models::FavoriteFile, actions: &mut Vec<Action>) {
        let selected = self.selected_file_id == Some(file.id);
        let dirty = self
            .tabs
            .iter()
            .any(|tab| tab.file_id == Some(file.id) && tab.dirty);
        let missing = self.missing_files.contains(&file.id);

        let mut label = file.display_name();
        if dirty {
            label = format!("{label} ●");
        }
        if missing {
            label = format!("⚠ {label}");
        }

        let hover = if missing {
            format!("{}（文件不存在）", file.path)
        } else {
            file.path.clone()
        };

        let path = PathBuf::from(&file.path);
        let display_name = file.display_name();
        let file_id = file.id;

        let response = ui.selectable_label(selected, label).on_hover_text(hover);

        if response.clicked() {
            actions.push(Action::Open {
                file_id,
                path: path.clone(),
                display_name: display_name.clone(),
            });
        }

        response.context_menu(|ui| {
            if ui.button("打开").clicked() {
                actions.push(Action::Open {
                    file_id,
                    path: path.clone(),
                    display_name: display_name.clone(),
                });
                ui.close_menu();
            }

            if ui.button("重命名显示名").clicked() {
                actions.push(Action::StartRenameFile(file_id, file.alias.clone()));
                ui.close_menu();
            }

            if ui.button("在资源管理器中显示").clicked() {
                actions.push(Action::Reveal(path.clone()));
                ui.close_menu();
            }

            if ui.button("从列表移除").clicked() {
                actions.push(Action::RemoveFile(file_id));
                ui.close_menu();
            }
        });
    }

    /// 拖动结束后把左栏宽度写回数据库
    fn sync_list_width(&mut self, width: f32, ctx: &Context, conn: &Connection) {
        self.list_width = width;

        let pointer_down = ctx.input(|input| input.pointer.any_down());
        if !should_persist_width(pointer_down, width, self.last_saved_width) {
            return;
        }

        match store::save_list_width(conn, width) {
            Ok(()) => {
                self.last_saved_width = width;
                log::debug!("已保存列表宽度: {width}");
            }
            Err(error) => log::error!("保存列表宽度失败: {}", error),
        }
    }

    /// 渲染右侧编辑区
    fn render_editor(&mut self, ui: &mut Ui, actions: &mut Vec<Action>) {
        let active = self.active_tab.filter(|index| *index < self.tabs.len());

        egui::TopBottomPanel::bottom("file_editor_status").show_inside(ui, |ui| {
            self.render_status(ui, active);
        });

        self.render_tab_bar(ui, actions);

        let Some(active) = active else {
            ui.centered_and_justified(|ui| {
                ui.weak("从左侧选择一个文件开始编辑（支持 Ctrl+S 保存）");
            });
            return;
        };

        self.render_editor_toolbar(ui, active, actions);
        self.render_tab_message(ui, active);

        egui::ScrollArea::both()
            .id_salt("file_editor_scroll")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let tab = &mut self.tabs[active];
                let editable = !tab.read_only;
                let response = ui.add(
                    egui::TextEdit::multiline(&mut tab.text)
                        .code_editor()
                        .desired_width(f32::INFINITY)
                        .desired_rows(EDITOR_DESIRED_ROWS)
                        .interactive(editable),
                );

                if response.changed() {
                    tab.dirty = true;
                }
            });
    }

    /// 渲染标签栏
    fn render_tab_bar(&mut self, ui: &mut Ui, actions: &mut Vec<Action>) {
        if self.tabs.is_empty() {
            return;
        }

        let titles: Vec<String> = self.tabs.iter().map(|tab| tab.title()).collect();
        egui::ScrollArea::horizontal()
            .id_salt("file_editor_tabs")
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    for (index, title) in titles.iter().enumerate() {
                        let is_active = self.active_tab == Some(index);
                        let label = ui
                            .selectable_label(is_active, title)
                            .on_hover_text(self.tabs[index].path.display().to_string());
                        if label.clicked() {
                            actions.push(Action::ActivateTab(index));
                        }

                        if ui.small_button("×").on_hover_text("关闭标签").clicked() {
                            actions.push(Action::CloseTab(index));
                        }
                    }
                });
            });
        ui.separator();
    }

    /// 渲染当前标签的保存 / 重载操作行
    fn render_editor_toolbar(&mut self, ui: &mut Ui, active: usize, actions: &mut Vec<Action>) {
        let read_only = self.tabs[active].read_only;
        let path = self.tabs[active].path.display().to_string();

        ui.horizontal(|ui| {
            if ui
                .add_enabled(!read_only, egui::Button::new("💾 保存"))
                .on_hover_text("Ctrl+S")
                .clicked()
            {
                actions.push(Action::SaveActive);
            }

            if ui.button("⟳ 重载").clicked() {
                actions.push(Action::ReloadActive);
            }

            ui.separator();
            ui.label(RichText::new(path).weak());
        });
    }

    /// 渲染标签提示（只读、外部修改等）
    fn render_tab_message(&self, ui: &mut Ui, active: usize) {
        let Some(message) = self.tabs[active].message.as_deref() else {
            return;
        };

        ui.horizontal_wrapped(|ui| {
            ui.colored_label(
                egui::Color32::from_rgb(0xE0, 0xA0, 0x30),
                format!("⚠ {message}"),
            );
        });
    }

    /// 渲染状态栏
    fn render_status(&self, ui: &mut Ui, active: Option<usize>) {
        ui.horizontal(|ui| {
            match active.and_then(|index| self.tabs.get(index)) {
                Some(tab) => {
                    let saved = if tab.dirty { "未保存" } else { "已保存" };
                    // 只读标签未判定编码与换行符，避免显示误导性的值
                    let format = if tab.read_only {
                        "只读预览".to_string()
                    } else {
                        format!("{} | {}", tab.encoding.label(), tab.line_ending.label())
                    };
                    let language = models::highlight_language(&tab.path)
                        .map(|language| format!(" | 推断语法 {language}"))
                        .unwrap_or_default();
                    ui.label(format!(
                        "{} | {}{} | {}",
                        tab.path.display(),
                        format,
                        language,
                        saved
                    ));
                }
                None => {
                    ui.weak("未打开文件");
                }
            }

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(RichText::new(&self.status).weak());
            });
        });
    }

    /// 渲染弹窗
    fn render_dialogs(&mut self, ctx: &Context, conn: &Connection) {
        match self.dialog {
            Dialog::AddFile => self.render_add_file_dialog(ctx, conn),
            Dialog::RenameFile => self.render_rename_file_dialog(ctx, conn),
            Dialog::ManageDirectories => self.render_directory_dialog(ctx, conn),
            Dialog::None => {}
        }

        if self.pending_close.is_some() {
            let mut actions = Vec::new();
            self.render_close_dialog(ctx, &mut actions);
            self.apply_actions(actions, conn);
        }
    }

    /// 添加常用文件弹窗
    fn render_add_file_dialog(&mut self, ctx: &Context, conn: &Connection) {
        let mut confirmed = false;
        let mut cancelled = false;
        let directories = self.directories.clone();

        egui::Window::new("添加常用文件")
            .collapsible(false)
            .resizable(false)
            .default_width(520.0)
            .show(ctx, |ui| {
                egui::Grid::new("file_editor_add_grid")
                    .num_columns(2)
                    .spacing([8.0, 8.0])
                    .show(ui, |ui| {
                        ui.label("文件路径：");
                        ui.horizontal(|ui| {
                            ui.add(
                                egui::TextEdit::singleline(&mut self.add_form.path)
                                    .desired_width(320.0)
                                    .hint_text("例如 C:\\Users\\me\\.gitconfig"),
                            );
                            if ui.button("📂 浏览…").clicked() {
                                if let Some(path) = rfd::FileDialog::new().pick_file() {
                                    self.add_form.path = path.display().to_string();
                                }
                            }
                        });
                        ui.end_row();

                        ui.label("显示名：");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.add_form.alias)
                                .desired_width(320.0)
                                .hint_text("留空则显示文件名"),
                        );
                        ui.end_row();

                        ui.label("所属目录：");
                        let selected = directories
                            .iter()
                            .find(|directory| Some(directory.id) == self.add_form.directory_id)
                            .map(|directory| directory.name.clone())
                            .unwrap_or_else(|| "未分组".to_string());
                        egui::ComboBox::from_id_salt("file_editor_add_directory")
                            .selected_text(selected)
                            .show_ui(ui, |ui| {
                                ui.selectable_value(
                                    &mut self.add_form.directory_id,
                                    None,
                                    "未分组",
                                );
                                for directory in &directories {
                                    ui.selectable_value(
                                        &mut self.add_form.directory_id,
                                        Some(directory.id),
                                        &directory.name,
                                    );
                                }
                            });
                        ui.end_row();
                    });

                if let Some(error) = self.add_form.error.as_deref() {
                    ui.add_space(4.0);
                    ui.colored_label(egui::Color32::from_rgb(0xE0, 0x60, 0x60), error);
                }

                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("确定").clicked() {
                        confirmed = true;
                    }
                    if ui.button("取消").clicked() {
                        cancelled = true;
                    }
                });
            });

        if cancelled {
            self.dialog = Dialog::None;
            self.add_form = AddFileForm::default();
            return;
        }

        if confirmed {
            self.submit_add_file(conn);
        }
    }

    /// 校验并提交添加文件表单
    fn submit_add_file(&mut self, conn: &Connection) {
        let raw_path = self.add_form.path.trim().to_string();
        if raw_path.is_empty() {
            self.add_form.error = Some("请填写文件路径".to_string());
            return;
        }

        let path = PathBuf::from(&raw_path);
        if models::is_hosts_file(&path) {
            self.add_form.error =
                Some("hosts 文件请使用 Hosts 管理器插件编辑，避免绕过其环境合并逻辑".to_string());
            return;
        }

        if !path.is_file() {
            self.add_form.error = Some(format!("路径不存在或不是文件: {}", path.display()));
            return;
        }

        let alias = self.add_form.alias.clone();
        let alias = if alias.trim().is_empty() {
            None
        } else {
            Some(alias)
        };
        let directory_id = self.add_form.directory_id;

        match store::insert_file(conn, directory_id, &raw_path, alias.as_deref()) {
            Ok(id) => {
                self.status = format!("已加入常用列表: {raw_path}");
                log::info!("常用文件编辑器新增条目 id={id}, path={raw_path}");
                self.reload_data(conn);
                self.dialog = Dialog::None;
                self.add_form = AddFileForm::default();
            }
            Err(error) => {
                log::error!("常用文件编辑器新增条目失败 {raw_path}: {error}");
                self.add_form.error = Some(error.to_string());
            }
        }
    }

    /// 重命名显示名弹窗
    fn render_rename_file_dialog(&mut self, ctx: &Context, conn: &Connection) {
        let Some(entry) = self.rename_form.as_mut() else {
            self.dialog = Dialog::None;
            return;
        };
        let alias = &mut entry.1;

        let mut confirmed = false;
        let mut cancelled = false;

        egui::Window::new("重命名显示名")
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                ui.label("显示名（留空表示使用文件名）：");
                ui.add(egui::TextEdit::singleline(alias).desired_width(280.0));
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("确定").clicked() {
                        confirmed = true;
                    }
                    if ui.button("取消").clicked() {
                        cancelled = true;
                    }
                });
            });

        if cancelled {
            self.rename_form = None;
            self.dialog = Dialog::None;
            return;
        }

        if !confirmed {
            return;
        }

        let Some((id, alias)) = self.rename_form.take() else {
            return;
        };
        let alias = alias.trim().to_string();
        let alias = if alias.is_empty() { None } else { Some(alias) };

        match store::update_alias(conn, id, alias.as_deref()) {
            Ok(()) => {
                self.status = "已更新显示名".to_string();
                log::info!("常用文件编辑器更新显示名 id={id}");
                self.reload_data(conn);
            }
            Err(error) => {
                self.status = format!("更新显示名失败: {error}");
                log::error!("常用文件编辑器更新显示名失败 id={id}: {error}");
            }
        }

        self.dialog = Dialog::None;
    }

    /// 目录管理弹窗
    fn render_directory_dialog(&mut self, ctx: &Context, conn: &Connection) {
        let mut create = false;
        let mut rename: Option<i64> = None;
        let mut delete: Option<i64> = None;
        let mut close = false;
        let mut select_rename: Option<(i64, String)> = None;
        let directories = self.directories.clone();

        egui::Window::new("目录管理")
            .collapsible(false)
            .resizable(false)
            .default_width(460.0)
            .show(ctx, |ui| {
                ui.label("现有目录：");
                egui::ScrollArea::vertical()
                    .id_salt("file_editor_directory_list")
                    .max_height(200.0)
                    .show(ui, |ui| {
                        for directory in &directories {
                            let count = self
                                .files
                                .iter()
                                .filter(|file| file.directory_id == Some(directory.id))
                                .count();
                            ui.horizontal(|ui| {
                                ui.label(format!("📁 {}（{count} 个文件）", directory.name));
                                if ui.small_button("重命名").clicked() {
                                    select_rename = Some((directory.id, directory.name.clone()));
                                }
                                if ui.small_button("删除").clicked() {
                                    delete = Some(directory.id);
                                }
                            });
                        }
                    });

                ui.separator();

                if let Some((id, name)) = select_rename {
                    self.dir_form.rename_target = Some(id);
                    self.dir_form.rename_name = name;
                }

                match self.dir_form.rename_target {
                    Some(id) => {
                        ui.horizontal(|ui| {
                            ui.label("新名称：");
                            ui.add(
                                egui::TextEdit::singleline(&mut self.dir_form.rename_name)
                                    .desired_width(200.0),
                            );
                            if ui.button("保存名称").clicked() {
                                rename = Some(id);
                            }
                            if ui.button("取消重命名").clicked() {
                                self.dir_form.rename_target = None;
                                self.dir_form.rename_name.clear();
                            }
                        });
                    }
                    None => {
                        ui.horizontal(|ui| {
                            ui.label("新建目录：");
                            ui.add(
                                egui::TextEdit::singleline(&mut self.dir_form.new_name)
                                    .desired_width(200.0)
                                    .hint_text("目录名称"),
                            );
                            if ui.button("新建").clicked() {
                                create = true;
                            }
                        });
                    }
                }

                if let Some(error) = self.dir_form.error.as_deref() {
                    ui.add_space(4.0);
                    ui.colored_label(egui::Color32::from_rgb(0xE0, 0x60, 0x60), error);
                }

                ui.add_space(6.0);
                ui.weak("删除目录只删除分组，其下文件会移入「未分组」，不会删除磁盘文件。");

                ui.add_space(6.0);
                if ui.button("关闭").clicked() {
                    close = true;
                }
            });

        if create {
            let name = self.dir_form.new_name.clone();
            match store::insert_directory(conn, &name) {
                Ok(id) => {
                    self.status = format!("已新建目录: {name}");
                    log::info!("常用文件编辑器新建目录 id={id}, name={name}");
                    self.dir_form.new_name.clear();
                    self.dir_form.error = None;
                    self.reload_data(conn);
                }
                Err(error) => {
                    log::error!("常用文件编辑器新建目录失败: {error}");
                    self.dir_form.error = Some(error.to_string());
                }
            }
        } else if let Some(id) = rename {
            let name = self.dir_form.rename_name.clone();
            match store::rename_directory(conn, id, &name) {
                Ok(()) => {
                    self.status = format!("已重命名目录: {name}");
                    log::info!("常用文件编辑器重命名目录 id={id}, name={name}");
                    self.dir_form.rename_target = None;
                    self.dir_form.rename_name.clear();
                    self.dir_form.error = None;
                    self.reload_data(conn);
                }
                Err(error) => {
                    log::error!("常用文件编辑器重命名目录失败: {error}");
                    self.dir_form.error = Some(error.to_string());
                }
            }
        } else if let Some(id) = delete {
            match store::delete_directory(conn, id) {
                Ok(()) => {
                    self.status = "已删除目录（其下文件已移入未分组）".to_string();
                    log::info!("常用文件编辑器删除目录 id={id}");
                    self.dir_form.error = None;
                    self.reload_data(conn);
                }
                Err(error) => {
                    log::error!("常用文件编辑器删除目录失败: {error}");
                    self.dir_form.error = Some(error.to_string());
                }
            }
        }

        if close {
            self.dialog = Dialog::None;
            self.dir_form = DirectoryForm::default();
        }
    }

    /// 未保存关闭确认弹窗
    fn render_close_dialog(&mut self, ctx: &Context, actions: &mut Vec<Action>) {
        let Some(index) = self.pending_close else {
            return;
        };
        let Some(tab) = self.tabs.get(index) else {
            self.pending_close = None;
            return;
        };
        let display_name = tab.display_name.clone();

        let mut decision = None;
        egui::Window::new("关闭标签")
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                ui.label(format!("文件 {display_name} 有未保存的修改。"));
                ui.label("关闭前是否保存？");
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("💾 保存").clicked() {
                        decision = Some(models::CloseDecision::Save);
                    }
                    if ui.button("不保存").clicked() {
                        decision = Some(models::CloseDecision::Discard);
                    }
                    if ui.button("取消").clicked() {
                        decision = Some(models::CloseDecision::Cancel);
                    }
                });
            });

        if let Some(decision) = decision {
            actions.push(Action::FinishClose(decision));
        }
    }

    /// 执行渲染期间收集的动作
    fn apply_actions(&mut self, actions: Vec<Action>, conn: &Connection) {
        for action in actions {
            match action {
                Action::Open {
                    file_id,
                    path,
                    display_name,
                } => self.open_file(conn, file_id, &path, display_name),
                Action::ActivateTab(index) => {
                    if index < self.tabs.len() {
                        self.active_tab = Some(index);
                        self.check_external_change(index);
                    }
                }
                Action::CloseTab(index) => self.request_close(index),
                Action::SaveActive => {
                    if let Some(index) = self.active_tab {
                        self.save_tab(index);
                    }
                }
                Action::ReloadActive => {
                    if let Some(index) = self.active_tab {
                        self.reload_tab(index);
                    }
                }
                Action::RemoveFile(id) => self.remove_file(conn, id),
                Action::Reveal(path) => self.reveal_in_explorer(&path),
                Action::StartAddFile(directory_id) => {
                    self.add_form = AddFileForm {
                        directory_id,
                        ..AddFileForm::default()
                    };
                    self.dialog = Dialog::AddFile;
                }
                Action::StartRenameFile(id, alias) => {
                    self.rename_form = Some((id, alias.unwrap_or_default()));
                    self.dialog = Dialog::RenameFile;
                }
                Action::StartManageDirectories => {
                    self.dir_form = DirectoryForm::default();
                    self.dialog = Dialog::ManageDirectories;
                }
                Action::FinishClose(decision) => self.finish_close(decision),
            }
        }
    }

    /// 打开文件：已打开则激活，否则新建标签
    fn open_file(&mut self, conn: &Connection, file_id: i64, path: &Path, display_name: String) {
        self.selected_file_id = Some(file_id);

        if let Some(index) = self
            .tabs
            .iter()
            .position(|tab| tab.file_id == Some(file_id) || tab.path == path)
        {
            self.active_tab = Some(index);
            self.check_external_change(index);
            return;
        }

        if models::is_hosts_file(path) {
            self.status = "hosts 文件请使用 Hosts 管理器插件编辑".to_string();
            return;
        }

        match document::load(path) {
            Ok(loaded) => {
                if let Err(error) = store::touch_last_opened(conn, file_id) {
                    log::warn!("更新最近打开时间失败: {}", error);
                }

                let message = match (loaded.message, loaded.mixed_line_endings) {
                    (Some(message), _) => Some(message),
                    (None, true) => Some(format!(
                        "文件混用 CRLF 与 LF，保存时将统一为 {}",
                        loaded.line_ending.label()
                    )),
                    (None, false) => None,
                };
                self.tabs.push(models::EditorTab {
                    file_id: Some(file_id),
                    path: path.to_path_buf(),
                    display_name,
                    text: loaded.text,
                    dirty: false,
                    disk: loaded.disk,
                    encoding: loaded.encoding,
                    line_ending: loaded.line_ending,
                    read_only: loaded.read_only,
                    message,
                });
                self.active_tab = Some(self.tabs.len() - 1);
                self.status = format!("已打开 {}", path.display());
                log::info!("常用文件编辑器打开文件: {}", path.display());
            }
            Err(error) => {
                self.status = format!("打开失败: {error}");
                log::error!("常用文件编辑器打开失败 {}: {error}", path.display());
            }
        }
    }

    /// 从常用列表移除条目（不关闭标签、不删除磁盘文件）
    fn remove_file(&mut self, conn: &Connection, id: i64) {
        match store::delete_file(conn, id) {
            Ok(()) => {
                self.status = "已从常用列表移除（磁盘文件未删除）".to_string();
                self.reload_data(conn);
                log::info!("常用文件编辑器移除条目 id={id}");
            }
            Err(error) => {
                self.status = format!("移除失败: {error}");
                log::error!("常用文件编辑器移除条目失败 id={id}: {error}");
            }
        }
    }

    /// 在资源管理器中定位文件
    fn reveal_in_explorer(&mut self, path: &Path) {
        if let Err(error) = std::process::Command::new("explorer")
            .arg(format!("/select,{}", path.display()))
            .spawn()
        {
            self.status = format!("无法打开资源管理器: {error}");
            log::warn!("无法在资源管理器中显示 {}: {error}", path.display());
        }
    }

    /// 保存指定标签
    fn save_tab(&mut self, index: usize) {
        if index >= self.tabs.len() {
            return;
        }

        // 只读标签是预期内不会保存的状态，按信息级记录，避免污染错误日志
        if self.tabs[index].read_only {
            self.status = "只读标签不可保存".to_string();
            log::info!(
                "常用文件编辑器忽略只读标签的保存请求: {}",
                self.tabs[index].path.display()
            );
            return;
        }

        let outcome: std::result::Result<String, String> = {
            let tab = &mut self.tabs[index];
            match document::save(&tab.path, &tab.text, tab.encoding, tab.line_ending) {
                Ok(disk) => {
                    tab.disk = disk;
                    tab.dirty = false;
                    tab.message = None;
                    Ok(tab.path.display().to_string())
                }
                Err(error) => {
                    let message = format!("保存失败: {error}");
                    tab.message = Some(message.clone());
                    Err(message)
                }
            }
        };

        match outcome {
            Ok(path) => {
                self.status = format!("已保存 {path}");
                log::info!("常用文件编辑器已保存: {path}");
            }
            Err(message) => {
                self.status = message.clone();
                log::error!("常用文件编辑器{message}");
            }
        }
    }

    /// 放弃修改并重新读取标签内容
    fn reload_tab(&mut self, index: usize) {
        if index >= self.tabs.len() {
            return;
        }

        let path = self.tabs[index].path.clone();
        match document::load(&path) {
            Ok(loaded) => {
                let tab = &mut self.tabs[index];
                tab.text = loaded.text;
                tab.dirty = false;
                tab.disk = loaded.disk;
                tab.encoding = loaded.encoding;
                tab.line_ending = loaded.line_ending;
                tab.read_only = loaded.read_only;
                tab.message = loaded.message;
                self.status = format!("已重新加载 {}", path.display());
                log::info!("常用文件编辑器已重新加载: {}", path.display());
            }
            Err(error) => {
                self.tabs[index].message = Some(format!("重载失败: {error}"));
                self.status = format!("重载失败: {error}");
                log::error!("常用文件编辑器重载失败 {}: {error}", path.display());
            }
        }
    }

    /// 检测标签对应的文件是否被外部修改
    fn check_external_change(&mut self, index: usize) {
        let Some((path, baseline)) = self.tabs.get(index).map(|tab| (tab.path.clone(), tab.disk))
        else {
            return;
        };

        match document::disk_state(&path) {
            Ok(latest) if document::changed_on_disk(baseline, latest) => {
                if self.tabs[index].dirty {
                    self.tabs[index].message =
                        Some("文件已被外部修改；点「重载」可放弃本地修改".to_string());
                } else {
                    log::info!("常用文件编辑器检测到外部修改，自动重载: {}", path.display());
                    self.reload_tab(index);
                }
            }
            Ok(_) => {}
            Err(error) => {
                self.tabs[index].message = Some(format!("文件不可访问: {error}"));
            }
        }
    }

    /// 请求关闭标签：脏标签先弹确认框
    fn request_close(&mut self, index: usize) {
        if index >= self.tabs.len() {
            return;
        }

        if models::close_requires_prompt(self.tabs[index].dirty) {
            self.pending_close = Some(index);
        } else {
            self.close_tab(index);
        }
    }

    /// 处理关闭确认结果
    fn finish_close(&mut self, decision: models::CloseDecision) {
        let Some(index) = self.pending_close else {
            return;
        };

        match decision {
            models::CloseDecision::Save => {
                self.save_tab(index);
                if self.tabs.get(index).is_some_and(|tab| !tab.dirty) {
                    self.pending_close = None;
                    self.close_tab(index);
                }
            }
            models::CloseDecision::Discard => {
                self.pending_close = None;
                self.close_tab(index);
            }
            models::CloseDecision::Cancel => {
                self.pending_close = None;
            }
        }
    }

    /// 关闭标签并调整激活下标
    fn close_tab(&mut self, index: usize) {
        if index >= self.tabs.len() {
            return;
        }

        self.tabs.remove(index);
        self.active_tab = active_after_close(self.active_tab, index, self.tabs.len());
    }
}

#[cfg(test)]
mod tests {
    use super::{FileEditorUi, active_after_close, should_persist_width};
    use crate::plugins::file_editor::store;
    use rusqlite::Connection;
    use std::fs;
    use std::path::PathBuf;

    /// 建立与 `storage::database` 一致的表结构（内存库）
    fn test_connection() -> Connection {
        let conn = Connection::open_in_memory().expect("创建内存数据库失败");
        conn.execute_batch(
            "PRAGMA foreign_keys = ON;
             CREATE TABLE file_editor_directories (
                 id         INTEGER PRIMARY KEY AUTOINCREMENT,
                 name       TEXT NOT NULL UNIQUE,
                 sort_order INTEGER DEFAULT 0,
                 created_at DATETIME DEFAULT CURRENT_TIMESTAMP
             );
             CREATE TABLE file_editor_files (
                 id             INTEGER PRIMARY KEY AUTOINCREMENT,
                 directory_id   INTEGER,
                 path           TEXT NOT NULL UNIQUE,
                 alias          TEXT,
                 sort_order     INTEGER DEFAULT 0,
                 created_at     DATETIME DEFAULT CURRENT_TIMESTAMP,
                 last_opened_at DATETIME,
                 FOREIGN KEY (directory_id) REFERENCES file_editor_directories(id) ON DELETE SET NULL
             );
             CREATE TABLE file_editor_settings (
                 id         INTEGER PRIMARY KEY DEFAULT 1,
                 list_width REAL NOT NULL DEFAULT 260.0,
                 updated_at DATETIME DEFAULT CURRENT_TIMESTAMP
             );",
        )
        .expect("初始化测试表失败");

        conn
    }

    /// 渲染一帧界面：无窗口环境下验证布局与交互代码不会 panic
    fn render_frame(ctx: &egui::Context, state: &mut FileEditorUi, conn: &Connection) {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1000.0, 700.0),
            )),
            ..Default::default()
        };

        let _ = ctx.run(input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                state.render(ui, conn);
            });
        });
    }

    /// 生成唯一的临时文件路径
    fn temp_file(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "tools-box-file-editor-ui-test-{}-{name}",
            std::process::id()
        ))
    }

    #[test]
    fn active_tab_after_close_prefers_right_neighbour() {
        assert_eq!(active_after_close(Some(1), 1, 3), Some(1));
        assert_eq!(active_after_close(Some(2), 1, 3), Some(1));
        assert_eq!(active_after_close(Some(0), 2, 3), Some(0));
        assert_eq!(active_after_close(Some(0), 1, 2), Some(0));
    }

    #[test]
    fn active_tab_after_close_falls_back_to_last_tab() {
        assert_eq!(active_after_close(Some(2), 2, 2), Some(1));
        assert_eq!(active_after_close(Some(0), 0, 1), Some(0));
        assert_eq!(active_after_close(Some(0), 0, 0), None);
        assert_eq!(active_after_close(None, 0, 2), Some(0));
    }

    #[test]
    fn renders_layout_and_handles_open_save_close_flow() {
        let conn = test_connection();
        let ctx = egui::Context::default();
        let path = temp_file("flow.toml");
        fs::write(&path, "key = 1\r\n").expect("写入测试文件失败");

        let mut state = FileEditorUi::new();
        state.init(&conn);

        // 空列表 / 无标签时渲染
        render_frame(&ctx, &mut state, &conn);

        // 数据库不可写时的警告横幅也必须能正常渲染
        state.set_storage_writable(false);
        render_frame(&ctx, &mut state, &conn);
        state.set_storage_writable(true);

        let id = store::insert_file(&conn, None, &path.display().to_string(), None)
            .expect("新增条目失败");
        state.reload_data(&conn);
        assert_eq!(state.files.len(), 1);

        // 打开文件 → 渲染 → 置脏 → 关闭确认 → 保存 → 放弃关闭
        state.open_file(&conn, id, &path, "flow.toml".to_string());
        render_frame(&ctx, &mut state, &conn);
        assert_eq!(state.tabs.len(), 1);
        assert_eq!(state.tabs[0].line_ending, super::models::LineEnding::CrLf);
        assert!(!state.tabs[0].dirty);

        state.tabs[0].text = "key = 2\n".to_string();
        state.tabs[0].dirty = true;
        render_frame(&ctx, &mut state, &conn);

        state.request_close(0);
        assert_eq!(state.pending_close, Some(0), "脏标签关闭前应弹确认框");
        render_frame(&ctx, &mut state, &conn);

        state.save_tab(0);
        assert!(!state.tabs[0].dirty, "保存后脏标记应复位");
        assert_eq!(
            fs::read_to_string(&path).expect("读取保存结果失败"),
            "key = 2\r\n",
            "保存必须按原文件换行符风格写回"
        );

        state.finish_close(super::models::CloseDecision::Discard);
        assert!(state.tabs.is_empty(), "选择不保存后应关闭标签");
        assert_eq!(state.pending_close, None);
        render_frame(&ctx, &mut state, &conn);

        // 再次打开后关闭非脏标签：不弹确认框
        state.open_file(&conn, id, &path, "flow.toml".to_string());
        state.request_close(0);
        assert!(state.tabs.is_empty());
        assert_eq!(state.pending_close, None);

        let _ = fs::remove_file(&path);
    }

    #[test]
    fn persists_width_only_after_drag_ends() {
        assert!(
            !should_persist_width(true, 300.0, 260.0),
            "拖动过程中不应写库"
        );
        assert!(
            should_persist_width(false, 300.0, 260.0),
            "松手后即使宽度不再变化也必须写库"
        );
        assert!(
            !should_persist_width(false, 260.2, 260.0),
            "小于 0.5px 的变化不写库"
        );
        assert!(
            !should_persist_width(false, 300.0, 300.0),
            "宽度未变化时不写库"
        );
    }

    #[test]
    fn read_only_tab_refuses_save() {
        let conn = test_connection();
        let path = temp_file("huge.toml");
        let limit =
            usize::try_from(super::document::MAX_EDITABLE_BYTES).expect("上限应能转换为 usize");
        fs::write(&path, vec![b'a'; limit + 8]).expect("写入测试文件失败");

        let mut state = FileEditorUi::new();
        state.init(&conn);
        let id = store::insert_file(&conn, None, &path.display().to_string(), None)
            .expect("新增条目失败");
        state.reload_data(&conn);
        state.open_file(&conn, id, &path, "huge.toml".to_string());

        assert_eq!(state.tabs.len(), 1);
        assert!(state.tabs[0].read_only, "超过 2 MB 的文件应只读");
        assert!(
            !state.can_save_active_tab(),
            "只读标签不应允许 Ctrl+S 产生保存动作"
        );

        let original_len = fs::metadata(&path).expect("读取属性失败").len();
        state.save_tab(0);
        assert_eq!(
            fs::metadata(&path).expect("读取属性失败").len(),
            original_len,
            "只读标签的保存必须不写磁盘"
        );

        let _ = fs::remove_file(&path);
    }

    #[test]
    fn save_failure_keeps_tab_open() {
        let conn = test_connection();
        let path = temp_file("readonly-attr.toml");
        fs::write(&path, "key = 1\n").expect("写入测试文件失败");
        let mut permissions = fs::metadata(&path).expect("读取属性失败").permissions();
        permissions.set_readonly(true);
        fs::set_permissions(&path, permissions).expect("设置只读属性失败");

        let mut state = FileEditorUi::new();
        state.init(&conn);
        let id = store::insert_file(&conn, None, &path.display().to_string(), None)
            .expect("新增条目失败");
        state.reload_data(&conn);
        state.open_file(&conn, id, &path, "readonly-attr.toml".to_string());
        assert_eq!(state.tabs.len(), 1);
        assert!(
            !state.tabs[0].read_only,
            "只读属性不影响能否编辑，只在保存时拒绝"
        );

        state.tabs[0].text = "key = 2\n".to_string();
        state.tabs[0].dirty = true;
        state.request_close(0);
        assert_eq!(state.pending_close, Some(0));

        state.finish_close(super::models::CloseDecision::Save);

        assert_eq!(state.tabs.len(), 1, "保存失败时不得关闭标签");
        assert!(state.tabs[0].dirty, "保存失败后应保持未保存状态");
        assert_eq!(state.pending_close, Some(0), "确认框应保留以便重试或取消");
        assert!(
            state.tabs[0]
                .message
                .as_deref()
                .is_some_and(|message| message.contains("只读")),
            "应提示保存失败原因"
        );

        let mut permissions = fs::metadata(&path).expect("读取属性失败").permissions();
        permissions.set_readonly(false);
        let _ = fs::set_permissions(&path, permissions);
        let _ = fs::remove_file(&path);
    }
}
