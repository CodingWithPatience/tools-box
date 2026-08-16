use egui::{Color32, RichText, Ui};

use super::markdown::MarkdownRenderer;
use super::models::{NoteEntry, NoteFolder, NoteForm, NoteViewMode};
use super::store::NoteStore;

/// UI 操作枚举（用于延迟执行）
enum UiAction {
    CreateNote,
    ReloadNotes,
    SelectNote(i64),
}

/// 临时笔记 UI
pub struct NoteTakerUi {
    /// 所有目录
    folders: Vec<NoteFolder>,
    /// 当前显示的笔记列表
    notes: Vec<NoteEntry>,
    /// 当前选中的笔记 ID
    selected_note_id: Option<i64>,
    /// 当前选中的目录 ID
    selected_folder_id: Option<i64>,
    /// 是否显示收藏夹
    show_favorites: bool,
    /// 笔记表单
    form: NoteForm,
    /// 视图模式
    view_mode: NoteViewMode,
    /// 编辑区纵向滚动偏移
    editor_scroll_offset: f32,
    /// 预览区纵向滚动偏移
    preview_scroll_offset: f32,
    /// 模式切换后待同步的滚动偏移
    pending_scroll_sync: Option<f32>,
    /// 待定位的 Markdown 标题目录索引
    pending_heading_index: Option<usize>,
    /// 搜索关键词
    search_query: String,
    /// 是否显示搜索
    show_search: bool,
    /// 是否显示目录管理弹窗
    show_folder_dialog: bool,
    /// 目录编辑表单
    folder_form_name: String,
    /// 目录编辑表单 - 上级目录
    folder_form_parent_id: Option<i64>,
    /// 编辑中的目录 ID
    editing_folder_id: Option<i64>,
    /// 错误信息
    error: Option<String>,
    /// Markdown 渲染器
    markdown_renderer: MarkdownRenderer,
    /// 左侧面板宽度
    left_panel_width: f32,
}

impl NoteTakerUi {
    pub fn new() -> Self {
        Self {
            folders: Vec::new(),
            notes: Vec::new(),
            selected_note_id: None,
            selected_folder_id: None,
            show_favorites: false,
            form: NoteForm::empty(),
            view_mode: NoteViewMode::Edit,
            editor_scroll_offset: 0.0,
            preview_scroll_offset: 0.0,
            pending_scroll_sync: None,
            pending_heading_index: None,
            search_query: String::new(),
            show_search: false,
            show_folder_dialog: false,
            folder_form_name: String::new(),
            folder_form_parent_id: None,
            editing_folder_id: None,
            error: None,
            markdown_renderer: MarkdownRenderer::new(),
            left_panel_width: 200.0, // 默认宽度
        }
    }

    /// 初始化
    pub fn init(&mut self, conn: &rusqlite::Connection) {
        let store = NoteStore::new(conn);
        match store.init_table() {
            Ok(_) => {
                self.load_folders(conn);
                self.load_notes(conn);
            }
            Err(e) => {
                self.error = Some(format!("初始化数据库失败: {}", e));
            }
        }
    }

    /// 加载目录列表
    fn load_folders(&mut self, conn: &rusqlite::Connection) {
        let store = NoteStore::new(conn);
        match store.get_all_folders() {
            Ok(folders) => self.folders = folders,
            Err(e) => {
                self.error = Some(format!("加载目录失败: {}", e));
            }
        }
    }

    /// 加载笔记列表
    fn load_notes(&mut self, conn: &rusqlite::Connection) {
        let store = NoteStore::new(conn);

        let result = if self.show_favorites {
            store.get_favorite_notes()
        } else if self.show_search && !self.search_query.is_empty() {
            store.search_notes(&self.search_query)
        } else {
            store.get_notes_by_folder(self.selected_folder_id)
        };

        match result {
            Ok(mut notes) => {
                // 按更新时间排序
                notes.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
                self.notes = notes;
            }
            Err(e) => {
                self.error = Some(format!("加载笔记失败: {}", e));
            }
        }
    }

    /// 渲染主界面
    pub fn render(&mut self, ui: &mut Ui, conn: &rusqlite::Connection) {
        // 处理待执行的操作
        let mut action = None;

        // 顶部标题栏
        ui.horizontal(|ui| {
            ui.heading("📝 临时笔记");

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                // 搜索按钮
                if ui
                    .button(RichText::new("🔍").strong())
                    .on_hover_text("搜索笔记")
                    .clicked()
                {
                    self.show_search = !self.show_search;
                    if !self.show_search {
                        self.search_query.clear();
                        action = Some(UiAction::ReloadNotes);
                    }
                }

                // 新建目录按钮
                if ui.button(RichText::new("+ 新建目录").strong()).clicked() {
                    self.editing_folder_id = None;
                    self.folder_form_name.clear();
                    self.folder_form_parent_id = None;
                    self.show_folder_dialog = true;
                }

                // 新建笔记按钮
                if ui.button(RichText::new("+ 新建笔记").strong()).clicked() {
                    action = Some(UiAction::CreateNote);
                }
            });
        });
        ui.separator();

        // 执行操作
        if let Some(act) = action {
            match act {
                UiAction::CreateNote => self.create_new_note(conn),
                UiAction::ReloadNotes => self.load_notes(conn),
                _ => {}
            }
        }

        // 主内容区域 - 左右分栏（可拖拽调整宽度）
        let available_width = ui.available_width();
        let min_left_width = 150.0;
        let max_left_width = (available_width * 0.5).min(400.0);

        ui.horizontal_top(|ui| {
            // 左侧面板（使用固定宽度）
            let left_width = self.left_panel_width.clamp(min_left_width, max_left_width);
            ui.vertical(|ui| {
                ui.set_min_width(left_width);
                ui.set_max_width(left_width);
                self.render_left_panel(ui, conn);
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

            // 右侧编辑区
            ui.vertical(|ui| {
                self.render_right_panel(ui, conn);
            });
        });

        // 状态栏
        ui.separator();
        self.render_status_bar(ui);

        // 目录管理弹窗
        if self.show_folder_dialog {
            self.render_folder_dialog(ui, conn);
        }
    }

    /// 渲染左侧面板
    fn render_left_panel(&mut self, ui: &mut Ui, conn: &rusqlite::Connection) {
        // 搜索框
        let mut action = None;
        if self.show_search {
            ui.horizontal(|ui| {
                ui.label("🔍");
                let search_response = ui.text_edit_singleline(&mut self.search_query);
                if search_response.changed() {
                    action = Some(UiAction::ReloadNotes);
                }
                if ui.small_button("✕").on_hover_text("关闭搜索").clicked() {
                    self.show_search = false;
                    self.search_query.clear();
                    action = Some(UiAction::ReloadNotes);
                }
            });
            ui.add_space(5.0);
        }

        // 目录树
        ui.label(RichText::new("目录").strong());
        ui.separator();

        egui::ScrollArea::vertical()
            .id_salt("folder_scroll")
            .max_height(ui.available_height() * 0.4)
            .show(ui, |ui| {
                // 未分类笔记
                let uncategorized_selected =
                    self.selected_folder_id.is_none() && !self.show_favorites;
                if ui
                    .selectable_label(uncategorized_selected, "📁 未分类")
                    .clicked()
                {
                    self.selected_folder_id = None;
                    self.show_favorites = false;
                    action = Some(UiAction::ReloadNotes);
                }

                // 收藏夹
                let fav_selected = self.show_favorites;
                if ui.selectable_label(fav_selected, "⭐ 收藏").clicked() {
                    self.show_favorites = true;
                    self.selected_folder_id = None;
                    action = Some(UiAction::ReloadNotes);
                }

                ui.add_space(5.0);

                // 渲染目录树
                self.render_folder_tree(ui, conn, None, 0);
            });

        ui.add_space(10.0);

        // 最近笔记
        ui.label(RichText::new("最近笔记").strong());
        ui.separator();

        egui::ScrollArea::vertical()
            .id_salt("recent_notes_scroll")
            .show(ui, |ui| {
                let recent_notes: Vec<_> = self.notes.iter().take(10).collect();
                for note in recent_notes {
                    let is_selected = self.selected_note_id == Some(note.id);
                    let label_text = if note.title.is_empty() {
                        "无标题".to_string()
                    } else {
                        note.title.clone()
                    };

                    let mut label = RichText::new(&label_text).small();
                    if note.is_favorite {
                        label = label.color(Color32::from_rgb(255, 200, 0));
                    }

                    if ui.selectable_label(is_selected, label).clicked() {
                        action = Some(UiAction::SelectNote(note.id));
                    }
                }
            });

        // 执行延迟操作
        if let Some(act) = action {
            match act {
                UiAction::ReloadNotes => self.load_notes(conn),
                UiAction::SelectNote(id) => self.select_note(id, conn),
                _ => {}
            }
        }
    }

    /// 渲染目录树（递归）
    fn render_folder_tree(
        &mut self,
        ui: &mut Ui,
        conn: &rusqlite::Connection,
        parent_id: Option<i64>,
        depth: usize,
    ) {
        // 收集当前层级的目录信息
        let folder_info: Vec<_> = self
            .folders
            .iter()
            .filter(|f| f.parent_id == parent_id)
            .map(|f| (f.id, f.name.clone(), f.parent_id))
            .collect();

        for (folder_id, folder_name, folder_parent_id) in folder_info {
            let is_selected = self.selected_folder_id == Some(folder_id) && !self.show_favorites;
            let indent = "  ".repeat(depth);
            let label_text = format!("{}📁 {}", indent, folder_name);

            let mut clicked = false;
            let mut edit_clicked = false;

            ui.horizontal(|ui| {
                if ui.selectable_label(is_selected, &label_text).clicked() {
                    clicked = true;
                }

                // 编辑按钮（使用 "..." 代替特殊字符）
                if ui.small_button("...").clicked() {
                    edit_clicked = true;
                }
            });

            if clicked {
                self.selected_folder_id = Some(folder_id);
                self.show_favorites = false;
                self.load_notes(conn);
            }

            if edit_clicked {
                self.editing_folder_id = Some(folder_id);
                self.folder_form_name = folder_name;
                self.folder_form_parent_id = folder_parent_id;
                self.show_folder_dialog = true;
            }

            // 递归渲染子目录
            self.render_folder_tree(ui, conn, Some(folder_id), depth + 1);
        }
    }

    /// 渲染右侧面板
    fn render_right_panel(&mut self, ui: &mut Ui, conn: &rusqlite::Connection) {
        if let Some(note_id) = self.selected_note_id {
            // 笔记编辑区
            self.render_note_editor(ui, conn, note_id);
        } else {
            // 未选中笔记时显示提示
            ui.centered_and_justified(|ui| {
                ui.label(
                    RichText::new("选择或创建一条笔记开始编辑")
                        .color(Color32::GRAY)
                        .size(16.0),
                );
            });
        }
    }

    /// 渲染笔记编辑器
    fn render_note_editor(&mut self, ui: &mut Ui, conn: &rusqlite::Connection, note_id: i64) {
        // 标题编辑
        ui.horizontal(|ui| {
            ui.label("标题:");
            let title_width = (ui.available_width() - 280.0).max(120.0);
            ui.add(
                egui::TextEdit::singleline(&mut self.form.title)
                    .hint_text("输入笔记标题")
                    .desired_width(title_width),
            );

            // 视图切换按钮
            if ui
                .selectable_label(self.view_mode == NoteViewMode::Edit, "编辑")
                .clicked()
            {
                self.set_view_mode(NoteViewMode::Edit);
            }
            if ui
                .selectable_label(self.view_mode == NoteViewMode::Preview, "预览")
                .clicked()
            {
                self.set_view_mode(NoteViewMode::Preview);
            }
            if ui
                .selectable_label(self.view_mode == NoteViewMode::Split, "分栏")
                .clicked()
            {
                self.set_view_mode(NoteViewMode::Split);
            }
            if ui
                .button("复制 Markdown")
                .on_hover_text("复制当前笔记的 Markdown 原文")
                .clicked()
            {
                ui.ctx().copy_text(self.form.content.clone());
            }
        });
        ui.add_space(5.0);

        // 内容编辑区（占据剩余空间）
        let bottom_bar_height = 40.0;
        let content_height = (ui.available_height() - bottom_bar_height).max(200.0);

        if let Some(offset) = self.pending_scroll_sync.take() {
            match self.view_mode {
                NoteViewMode::Edit => self.editor_scroll_offset = offset,
                NoteViewMode::Preview => self.preview_scroll_offset = offset,
                NoteViewMode::Split => {
                    self.editor_scroll_offset = offset;
                    self.preview_scroll_offset = offset;
                }
            }
        }

        let pending_heading_index = self.pending_heading_index.take();

        match self.view_mode {
            NoteViewMode::Edit => {
                let output = egui::ScrollArea::vertical()
                    .id_salt("note_content_scroll")
                    .max_height(content_height)
                    .vertical_scroll_offset(self.editor_scroll_offset)
                    .show(ui, |ui| {
                        // multiline TextEdit 内容自然撑开，由外层 ScrollArea 统一管理滚动。
                        let editor_width = ui.available_width();
                        ui.add(
                            egui::TextEdit::multiline(&mut self.form.content)
                                .hint_text("输入笔记内容，支持 Markdown 格式")
                                .desired_width(editor_width)
                                .min_size(egui::vec2(editor_width, content_height))
                                .code_editor(),
                        );
                    });
                self.editor_scroll_offset = output.state.offset.y;
            }
            NoteViewMode::Preview => {
                let preview_fill = ui.visuals().faint_bg_color;
                let preview_stroke =
                    egui::Stroke::new(1.0, ui.visuals().widgets.noninteractive.bg_stroke.color);
                egui::Frame::new()
                    .fill(preview_fill)
                    .stroke(preview_stroke)
                    .inner_margin(egui::Margin::symmetric(8, 6))
                    .show(ui, |ui| {
                        let has_heading_selector = self.render_heading_selector(ui);
                        let selector_height = if has_heading_selector { 30.0 } else { 0.0 };
                        let preview_height = (content_height - selector_height - 12.0).max(120.0);
                        ui.set_min_height(content_height - 12.0);

                        let output = egui::ScrollArea::vertical()
                            .id_salt("note_preview_scroll")
                            .max_height(preview_height)
                            .vertical_scroll_offset(self.preview_scroll_offset)
                            .show(ui, |ui| {
                                if let Some(heading_index) = pending_heading_index {
                                    self.markdown_renderer.render_with_heading_target(
                                        ui,
                                        &self.form.content,
                                        Some(heading_index),
                                    );
                                } else {
                                    self.markdown_renderer.render(ui, &self.form.content);
                                }
                            });
                        self.preview_scroll_offset = output.state.offset.y;
                    });
            }
            NoteViewMode::Split => {
                ui.columns(2, |columns| {
                    let editor_width = columns[0].available_width();
                    let previous_editor_scroll_offset = self.editor_scroll_offset;
                    let editor_output = egui::ScrollArea::vertical()
                        .id_salt("note_split_editor_scroll")
                        .max_height(content_height)
                        .vertical_scroll_offset(self.editor_scroll_offset)
                        .show(&mut columns[0], |ui| {
                            // multiline TextEdit 内容自然撑开，由源码栏统一管理滚动。
                            ui.add(
                                egui::TextEdit::multiline(&mut self.form.content)
                                    .hint_text("输入笔记内容，支持 Markdown 格式")
                                    .desired_width(editor_width)
                                    .min_size(egui::vec2(editor_width, content_height))
                                    .code_editor(),
                            );
                        });
                    self.editor_scroll_offset = editor_output.state.offset.y;

                    let editor_scroll_changed = Self::has_scroll_offset_changed(
                        previous_editor_scroll_offset,
                        self.editor_scroll_offset,
                    );
                    if editor_scroll_changed {
                        self.preview_scroll_offset = self.editor_scroll_offset;
                    }

                    let preview_fill = columns[1].visuals().faint_bg_color;
                    let preview_stroke = egui::Stroke::new(
                        1.0,
                        columns[1].visuals().widgets.noninteractive.bg_stroke.color,
                    );
                    egui::Frame::new()
                        .fill(preview_fill)
                        .stroke(preview_stroke)
                        .inner_margin(egui::Margin::symmetric(8, 6))
                        .show(&mut columns[1], |ui| {
                            let has_heading_selector = self.render_heading_selector(ui);
                            let selector_height = if has_heading_selector { 30.0 } else { 0.0 };
                            let preview_height =
                                (content_height - selector_height - 12.0).max(120.0);
                            ui.set_min_height(content_height - 12.0);

                            let preview_output = egui::ScrollArea::vertical()
                                .id_salt("note_split_preview_scroll")
                                .max_height(preview_height)
                                .vertical_scroll_offset(self.preview_scroll_offset)
                                .show(ui, |ui| {
                                    if let Some(heading_index) = pending_heading_index {
                                        self.markdown_renderer.render_with_heading_target(
                                            ui,
                                            &self.form.content,
                                            Some(heading_index),
                                        );
                                    } else {
                                        self.markdown_renderer.render(ui, &self.form.content);
                                    }
                                });
                            // 预览栏保留独立滚动能力，但其滚动位置不会反向修改源码栏。
                            self.preview_scroll_offset = preview_output.state.offset.y;
                        });
                });
            }
        }

        // 底部操作栏（使用 with_layout 推到底部）
        ui.with_layout(egui::Layout::bottom_up(egui::Align::Center), |ui| {
            ui.horizontal(|ui| {
                // 目录选择
                ui.label("目录:");
                egui::ComboBox::from_id_salt("note_folder_select")
                    .selected_text(self.get_folder_display_name())
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut self.form.folder_id, None, "未分类");
                        for folder in &self.folders {
                            let name = if let Some(parent_id) = folder.parent_id {
                                if let Some(parent) =
                                    self.folders.iter().find(|f| f.id == parent_id)
                                {
                                    format!("{} > {}", parent.name, folder.name)
                                } else {
                                    folder.name.clone()
                                }
                            } else {
                                folder.name.clone()
                            };
                            ui.selectable_value(&mut self.form.folder_id, Some(folder.id), &name);
                        }
                    });

                ui.separator();

                // 标签
                ui.label("标签:");
                ui.add(
                    egui::TextEdit::singleline(&mut self.form.tags)
                        .hint_text("标签1, 标签2")
                        .desired_width(150.0),
                );

                ui.separator();

                // 操作按钮
                if ui.button(RichText::new("💾 保存").strong()).clicked() {
                    self.save_note(conn);
                }

                let is_favorite = self
                    .notes
                    .iter()
                    .find(|n| n.id == note_id)
                    .map(|n| n.is_favorite)
                    .unwrap_or(false);

                let fav_text = if is_favorite {
                    "⭐ 已收藏"
                } else {
                    "☆ 收藏"
                };
                if ui.button(fav_text).clicked() {
                    self.toggle_favorite(note_id, conn);
                }

                if ui
                    .button(RichText::new("🗑 删除").color(Color32::from_rgb(200, 0, 0)))
                    .clicked()
                {
                    self.delete_note(note_id, conn);
                }
            });
        });
    }

    /// 渲染状态栏
    fn render_status_bar(&self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(format!("共 {} 条笔记", self.notes.len()))
                    .color(Color32::GRAY)
                    .small(),
            );

            if let Some(err) = &self.error {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(RichText::new(err).color(Color32::RED).small());
                });
            }
        });
    }

    /// 渲染目录管理弹窗
    fn render_folder_dialog(&mut self, ui: &mut Ui, conn: &rusqlite::Connection) {
        egui::Window::new("目录管理")
            .collapsible(false)
            .resizable(false)
            .show(ui.ctx(), |ui| {
                ui.horizontal(|ui| {
                    ui.label("目录名称:");
                    ui.text_edit_singleline(&mut self.folder_form_name);
                });

                ui.horizontal(|ui| {
                    ui.label("上级目录:");
                    egui::ComboBox::from_id_salt("parent_folder_select")
                        .selected_text(
                            self.folder_form_parent_id
                                .and_then(|pid| self.folders.iter().find(|f| f.id == pid))
                                .map(|f| f.name.as_str())
                                .unwrap_or("根目录"),
                        )
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut self.folder_form_parent_id, None, "根目录");
                            for folder in &self.folders {
                                if Some(folder.id) != self.editing_folder_id {
                                    ui.selectable_value(
                                        &mut self.folder_form_parent_id,
                                        Some(folder.id),
                                        &folder.name,
                                    );
                                }
                            }
                        });
                });

                ui.add_space(10.0);

                ui.horizontal(|ui| {
                    let btn_text = if self.editing_folder_id.is_some() {
                        "更新"
                    } else {
                        "创建"
                    };
                    if ui.button(btn_text).clicked() {
                        self.save_folder(conn);
                    }

                    if self.editing_folder_id.is_some() {
                        if ui
                            .button(RichText::new("删除").color(Color32::from_rgb(200, 0, 0)))
                            .clicked()
                        {
                            if let Some(id) = self.editing_folder_id {
                                self.delete_folder(id, conn);
                            }
                        }
                    }

                    if ui.button("取消").clicked() {
                        self.show_folder_dialog = false;
                    }
                });
            });
    }

    // ========== 操作方法 ==========

    /// 选中笔记
    fn select_note(&mut self, note_id: i64, conn: &rusqlite::Connection) {
        let store = NoteStore::new(conn);
        match store.get_note_by_id(note_id) {
            Ok(Some(note)) => {
                self.selected_note_id = Some(note_id);
                self.form = NoteForm::from_entry(&note);
                self.view_mode = NoteViewMode::Edit;
                self.editor_scroll_offset = 0.0;
                self.preview_scroll_offset = 0.0;
                self.pending_scroll_sync = None;
                self.pending_heading_index = None;
            }
            Ok(None) => {
                self.error = Some("笔记不存在".to_string());
            }
            Err(e) => {
                self.error = Some(format!("加载笔记失败: {}", e));
            }
        }
    }

    /// 切换视图模式并同步目标模式的滚动位置
    fn set_view_mode(&mut self, mode: NoteViewMode) {
        if self.view_mode == mode {
            return;
        }

        let offset = match mode {
            NoteViewMode::Edit => self.preview_scroll_offset,
            NoteViewMode::Preview => self.editor_scroll_offset,
            NoteViewMode::Split => match self.view_mode {
                NoteViewMode::Preview => self.preview_scroll_offset,
                NoteViewMode::Edit | NoteViewMode::Split => self.editor_scroll_offset,
            },
        };
        self.view_mode = mode;
        self.pending_scroll_sync = Some(offset);
    }

    fn render_heading_selector(&mut self, ui: &mut Ui) -> bool {
        if self.view_mode == NoteViewMode::Edit {
            return false;
        }

        let headings = self.markdown_renderer.heading_outline(&self.form.content);
        if headings.is_empty() {
            return false;
        }

        ui.horizontal(|ui| {
            ui.label("目录:");
            egui::ComboBox::from_id_salt("note_heading_navigation")
                .selected_text("选择标题")
                .show_ui(ui, |ui| {
                    for (index, heading) in headings.iter().enumerate() {
                        let indent = "  ".repeat(usize::from(heading.level.saturating_sub(1)));
                        let title = if heading.title.is_empty() {
                            "（无标题）"
                        } else {
                            heading.title.as_str()
                        };
                        if ui
                            .selectable_label(false, format!("{indent}{title}"))
                            .clicked()
                        {
                            self.pending_heading_index = Some(index);
                        }
                    }
                });
        });
        true
    }

    /// 判断源码栏滚动位置是否发生变化
    fn has_scroll_offset_changed(previous: f32, current: f32) -> bool {
        (current - previous).abs() > f32::EPSILON
    }

    /// 创建新笔记
    fn create_new_note(&mut self, conn: &rusqlite::Connection) {
        let store = NoteStore::new(conn);
        match store.create_note("无标题", "", self.selected_folder_id, "") {
            Ok(id) => {
                self.load_notes(conn);
                self.select_note(id, conn);
                log::info!("创建新笔记: id={}", id);
            }
            Err(e) => {
                self.error = Some(format!("创建笔记失败: {}", e));
            }
        }
    }

    /// 保存笔记
    fn save_note(&mut self, conn: &rusqlite::Connection) {
        if let Some(note_id) = self.selected_note_id {
            let store = NoteStore::new(conn);
            match store.update_note(
                note_id,
                &self.form.title,
                &self.form.content,
                self.form.folder_id,
                &self.form.tags,
            ) {
                Ok(_) => {
                    self.load_notes(conn);
                    log::info!("保存笔记: id={}", note_id);
                }
                Err(e) => {
                    self.error = Some(format!("保存笔记失败: {}", e));
                }
            }
        }
    }

    /// 删除笔记
    fn delete_note(&mut self, note_id: i64, conn: &rusqlite::Connection) {
        let store = NoteStore::new(conn);
        match store.delete_note(note_id) {
            Ok(_) => {
                self.selected_note_id = None;
                self.form = NoteForm::empty();
                self.load_notes(conn);
                log::info!("删除笔记: id={}", note_id);
            }
            Err(e) => {
                self.error = Some(format!("删除笔记失败: {}", e));
            }
        }
    }

    /// 切换收藏状态
    fn toggle_favorite(&mut self, note_id: i64, conn: &rusqlite::Connection) {
        let store = NoteStore::new(conn);
        match store.toggle_favorite(note_id) {
            Ok(is_favorite) => {
                self.load_notes(conn);
                log::info!("切换收藏状态: id={}, is_favorite={}", note_id, is_favorite);
            }
            Err(e) => {
                self.error = Some(format!("切换收藏失败: {}", e));
            }
        }
    }

    /// 保存目录
    fn save_folder(&mut self, conn: &rusqlite::Connection) {
        if self.folder_form_name.is_empty() {
            self.error = Some("目录名称不能为空".to_string());
            return;
        }

        let store = NoteStore::new(conn);
        let result = if let Some(folder_id) = self.editing_folder_id {
            store.update_folder(
                folder_id,
                &self.folder_form_name,
                self.folder_form_parent_id,
            )
        } else {
            store
                .create_folder(&self.folder_form_name, self.folder_form_parent_id)
                .map(|_| ())
        };

        match result {
            Ok(_) => {
                self.load_folders(conn);
                self.show_folder_dialog = false;
                self.folder_form_name.clear();
                self.folder_form_parent_id = None;
                self.editing_folder_id = None;
                log::info!("保存目录: {}", self.folder_form_name);
            }
            Err(e) => {
                self.error = Some(format!("保存目录失败: {}", e));
            }
        }
    }

    /// 删除目录
    fn delete_folder(&mut self, folder_id: i64, conn: &rusqlite::Connection) {
        let store = NoteStore::new(conn);
        match store.delete_folder(folder_id) {
            Ok(_) => {
                self.load_folders(conn);
                self.show_folder_dialog = false;
                self.editing_folder_id = None;
                // 如果当前选中的目录被删除，重置选择
                if self.selected_folder_id == Some(folder_id) {
                    self.selected_folder_id = None;
                    self.load_notes(conn);
                }
                log::info!("删除目录: id={}", folder_id);
            }
            Err(e) => {
                self.error = Some(format!("删除目录失败: {}", e));
            }
        }
    }

    /// 获取目录显示名称
    fn get_folder_display_name(&self) -> String {
        if let Some(folder_id) = self.form.folder_id {
            self.folders
                .iter()
                .find(|f| f.id == folder_id)
                .map(|f| f.name.clone())
                .unwrap_or_else(|| "未分类".to_string())
        } else {
            "未分类".to_string()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_sync_scroll_position_when_switching_view_mode() {
        let mut ui = NoteTakerUi::new();
        ui.editor_scroll_offset = 120.0;
        ui.preview_scroll_offset = 40.0;

        ui.set_view_mode(NoteViewMode::Preview);
        assert_eq!(ui.pending_scroll_sync, Some(120.0));

        ui.pending_scroll_sync = None;
        ui.set_view_mode(NoteViewMode::Edit);
        assert_eq!(ui.pending_scroll_sync, Some(40.0));
    }

    #[test]
    fn should_detect_editor_scroll_changes() {
        assert!(NoteTakerUi::has_scroll_offset_changed(0.0, 1.0));
        assert!(!NoteTakerUi::has_scroll_offset_changed(12.0, 12.0));
    }
}
