//! 正文域子模块：正文流转与意见的控制入口。
//!
//! 按 `docs/08` 的 Phase 3 分批从 `preparation/mod.rs` 搬出来；本目录只放正文域的文件，
//! `preparation/mod.rs` 负责把各子模块的名字再导出。

// 正文流转与意见搬到 `body::control`（`docs/08` 的 P3.8）。
pub(crate) mod control;

// 正文写入与提交搬到 `body::edit`（`docs/08` 的 P3.9）。
pub(crate) mod edit;

// 终稿提交搬到 `body::commit`（`docs/08` 的 P3.10）。
pub(crate) mod commit;
