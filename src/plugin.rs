/// 插件统一接口
///
/// 所有工具插件必须实现此 trait，以保证统一的注册、渲染和生命周期管理。
pub trait Plugin {
    /// 插件名称（用于侧边栏显示和搜索匹配）
    fn name(&self) -> &str;

    /// 插件图标（emoji 或文本图标，用于侧边栏显示）
    fn icon(&self) -> &str;

    /// 插件简介描述
    fn description(&self) -> &str;

    /// 在 egui 中渲染插件主界面
    fn render(&mut self, ui: &mut egui::Ui);

    /// 插件初始化回调（可选，默认空实现）
    fn init(&mut self) {}

    /// 插件销毁时的清理回调（可选，默认空实现）
    fn cleanup(&mut self) {}
}
