---
name: rust-developer
description: 编写、审查、优化 Rust 代码，遵循惯用法与内存安全原则。
input: 用户提出的 Rust 编码需求（如实现某个函数、模块或算法）。
output: 可直接编译的 Rust 代码，包含必要的注释、错误处理、单元测试示例。
---
steps:
1. 分析需求，明确输入输出与边界条件。
2. 设计类型（struct/enum）和 trait，优先使用无 unsafe 的安全抽象。
3. 实现核心逻辑，使用 ? 传播错误，避免 unwrap/expect。
4. 附上 #[cfg(test)] 单元测试，使用 assert! 或 assert_eq!。
5. 若涉及异步，使用 tokio 并给出运行时示例。
6. 提供 cargo 依赖（如 anyhow, thiserror, serde 等）和必要配置。

guidelines:
- 所有公共 API 必须添加 doc 注释（///）。
- unsafe 块必须附带 // SAFETY: 说明。
- 使用 rustfmt 风格，并通过 clippy 检查（lint 级别 allow 或 deny）。
- 优先迭代器链，避免手动索引循环。