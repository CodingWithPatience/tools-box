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

    /// 默认快捷键字符（用于全局热键 Ctrl+Alt+<key>）
    /// 返回 None 表示不参与全局热键唤出
    fn hotkey_char(&self) -> Option<char> {
        None
    }

    /// 返回 self 的可变引用作为 Any trait object（用于向下转型）
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        // 默认实现：不允许向下转型
        // 需要向下转型的插件应覆盖此方法
        unimplemented!("此插件未实现 as_any_mut")
    }
}
