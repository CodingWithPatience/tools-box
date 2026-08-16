# Tools Box 本地补丁说明

来源：`egui-winit 0.31.1`（MIT OR Apache-2.0）。

Windows 平台原实现会把 `Ctrl+Shift+C/V` 与普通 `Ctrl+C/V` 一样转换为不携带修饰键的
`egui::Event::Copy/Paste`。SSH 终端需要区分远端中断、终端复制和终端粘贴，因此本地补丁在
Windows 下让 `Ctrl+Shift+C/V` 保留为带修饰键的 `Event::Key`，普通 `Ctrl+C/V` 行为不变。

补丁位置：`src/lib.rs` 的 `is_copy_command`、`is_paste_command`，并附带两项回归测试。
