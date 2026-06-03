use crate::plugin::Plugin;
use crate::plugins;
use crate::storage::Database;
use egui::FontFamily;

/// 主应用状态
pub struct App {
    /// 已注册的插件列表
    plugins: Vec<Box<dyn Plugin>>,
    /// 当前选中的插件索引
    selected: usize,
    /// 侧边栏搜索关键词
    search_query: String,
    /// 数据库连接（预留，后续插件使用）
    _db: Database,
    /// 状态栏消息
    status_message: String,
}

/// 配置中文字体
///
/// 从 Windows 系统字体目录加载 Microsoft YaHei，确保中文正常显示。
pub fn setup_chinese_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();

    // 尝试加载 Microsoft YaHei 字体
    let font_paths = [
        r"C:\Windows\Fonts\msyh.ttc",
        r"C:\Windows\Fonts\msyhbd.ttc",
        r"C:\Windows\Fonts\simhei.ttf",
        r"C:\Windows\Fonts\simsun.ttc",
    ];

    let mut loaded = false;
    for path in &font_paths {
        if let Ok(font_data) = std::fs::read(path) {
            fonts
                .font_data
                .insert("chinese".to_owned(), egui::FontData::from_owned(font_data).into());

            // 将中文字体设为 Proportional 和 Monospace 的首选 fallback
            if let Some(family) = fonts.families.get_mut(&FontFamily::Proportional) {
                family.insert(0, "chinese".to_owned());
            }
            if let Some(family) = fonts.families.get_mut(&FontFamily::Monospace) {
                family.push("chinese".to_owned());
            }

            log::info!("已加载中文字体: {}", path);
            loaded = true;
            break;
        }
    }

    if !loaded {
        log::warn!("未找到中文字体，中文可能显示为乱码");
    }

    ctx.set_fonts(fonts);
}

impl App {
    pub fn new(db: Database) -> Self {
        let mut plugins = plugins::register_all_plugins();

        // 初始化所有插件
        for plugin in plugins.iter_mut() {
            plugin.init();
        }

        Self {
            plugins,
            selected: 0,
            search_query: String::new(),
            _db: db,
            status_message: "就绪".to_string(),
        }
    }

    /// 渲染顶部标题栏
    fn render_top_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.heading("🛠 Tools Box");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(&self.status_message);
            });
        });
        ui.separator();
    }

    /// 渲染左侧边栏
    fn render_sidebar(&mut self, ui: &mut egui::Ui) {
        // 搜索框
        ui.horizontal(|ui| {
            ui.label("🔍");
            ui.text_edit_singleline(&mut self.search_query);
        });

        ui.add_space(8.0);

        // 过滤后的插件索引列表
        let query = self.search_query.to_lowercase();
        let filtered: Vec<usize> = self
            .plugins
            .iter()
            .enumerate()
            .filter(|(_, p)| {
                query.is_empty()
                    || p.name().to_lowercase().contains(&query)
                    || p.description().to_lowercase().contains(&query)
            })
            .map(|(i, _)| i)
            .collect();

        // 插件列表
        egui::ScrollArea::vertical()
            .id_salt("sidebar_plugin_list")
            .show(ui, |ui| {
            for &idx in &filtered {
                let plugin = &self.plugins[idx];
                let is_selected = self.selected == idx;

                let text = format!("{} {}", plugin.icon(), plugin.name());

                let response = ui.add_sized(
                    [ui.available_width(), 36.0],
                    egui::SelectableLabel::new(is_selected, text),
                );

                if response.clicked() {
                    self.selected = idx;
                }

                // 鼠标悬停时显示描述
                if response.hovered() {
                    response.on_hover_text(plugin.description());
                }
            }
        });
    }

    /// 渲染右侧插件内容区
    fn render_plugin_content(&mut self, ui: &mut egui::Ui) {
        if self.plugins.is_empty() {
            ui.centered_and_justified(|ui| {
                ui.label("暂无可用插件");
            });
            return;
        }

        // 确保 selected 索引有效
        if self.selected >= self.plugins.len() {
            self.selected = 0;
        }

        let plugin = &mut self.plugins[self.selected];
        plugin.render(ui);
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // 顶部面板
        egui::TopBottomPanel::top("top_panel").show(ctx, |ui| {
            self.render_top_bar(ui);
        });

        // 底部状态栏
        egui::TopBottomPanel::bottom("status_bar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label(format!(
                    "就绪 | 已注册插件: {}",
                    self.plugins.len()
                ));
            });
        });

        // 左侧边栏
        egui::SidePanel::left("sidebar")
            .resizable(true)
            .default_width(200.0)
            .min_width(150.0)
            .show(ctx, |ui| {
                self.render_sidebar(ui);
            });

        // 中央内容区
        egui::CentralPanel::default().show(ctx, |ui| {
            self.render_plugin_content(ui);
        });
    }
}
