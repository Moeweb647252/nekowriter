//! The agent runtime (spec/03): a resumable state machine over a persisted, append-only
//! conversation.
//!
//! Layers:
//! - [`session`] — row-level reads, the DTOs and **the only** session writer: [`session::transition`]
//!   plus the column-wise updates guarded by the expected status (spec/03 §3, §14, D-034).
//! - [`runner`] — `drive`: the loop, `step_model`, `step_tool` (prepare outside any transaction,
//!   execute inside), sealing, finalize and the circuit breakers (spec/03 §5).
//! - [`manager`] — at most one runner per session, cancellation with a reason, startup recovery
//!   and graceful shutdown (spec/01 §5 step 8, spec/03 §14).
//! - [`tools`] — the tool framework (spec/05 §1–§2).
//! - [`context`] — kickoff assembly, request building and rollover (spec/03 §9–§11).
//! - [`preview`] — the incremental tool-argument extractor (spec/03 §12).
//! - [`report`] — edit reports from `view_snapshot` (spec/03 §7).
//! - [`seal`] — the canonical composition of a sealed result (spec/03 §5.3).
//! - [`completion`] — the domain effects of a completing call (spec/03 §5.4).

pub mod completion;
pub mod context;
pub mod manager;
pub mod preview;
pub mod report;
pub mod runner;
pub mod seal;
pub mod session;
pub mod tools;

pub use manager::{CancelReason, RecoveryReport, SessionManager, recover};
pub use session::transition;
