pub mod diff_viewer;
pub mod hosts_manager;
pub mod json_editor;
pub mod password_manager;

use crate::plugin::Plugin;

/// 创建并注册所有插件
///
/// 新增插件时在此函数中添加即可。
pub fn register_all_plugins() -> Vec<Box<dyn Plugin>> {
    let plugins: Vec<Box<dyn Plugin>> = vec![
        Box::new(password_manager::PasswordManagerPlugin::new()),
        Box::new(json_editor::JsonEditorPlugin::new()),
        Box::new(hosts_manager::HostsManagerPlugin::new()),
        Box::new(diff_viewer::DiffViewerPlugin::new()),
    ];

    log::info!("已注册 {} 个插件", plugins.len());
    plugins
}
