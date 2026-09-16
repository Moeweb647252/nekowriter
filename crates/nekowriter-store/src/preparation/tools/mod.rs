//! 工具层子模块：工具体的类型、参数解析与结果构造。
//!
//! 按 `docs/08` 的 Phase 3 分批从 `preparation/mod.rs` 搬出来，本目录只放工具层的文件；
//! `preparation/mod.rs` 负责把各子模块的名字再导出，兄弟模块的 `super::preparation::…` 引用不变。

pub(crate) mod model;

// 自由工具与提问工具的执行入口搬到 `tools::free`（`docs/08` 的 P3.5）。
pub(crate) mod free;

// 正文工具与暂停边界搬到 `tools::held`（`docs/08` 的 P3.6）。
pub(crate) mod held;
