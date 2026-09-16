# Tools Box 本地补丁说明

来源：`vt100 0.15.2`（MIT）。

SSH 终端需要支持滚动历史并显示滚动条，原实现存在两个问题：

1. `Grid::visible_rows` 用 `rows_len - scrollback_offset` 截取屏幕行；当滚动偏移超过屏幕行数
   （即查看更早的历史）时该减法下溢——debug 构建直接 panic，release 构建回绕后行为虽可滚动
   但两种构建表现不一致。补丁改为在「历史行 + 屏幕行」上滑动窗口并截取屏幕行数
   （`saturating_sub` + `take(rows_len)`），使滚动偏移在 `0..=历史行数` 范围内都能正确取到窗口，
   且偏移不超过屏幕行数时与原实现结果完全一致。
2. 缺少「当前已缓存历史行数」的公开访问接口，调用方无法计算滚动条范围与滚动上限。补丁新增
   `Grid::scrollback_count` 与 `Screen::scrollback_count`。

补丁位置：`src/grid.rs` 的 `visible_rows`、`scrollback_count`，`src/screen.rs` 的
`scrollback_count`。
