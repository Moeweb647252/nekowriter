//! 共享状态：每个 HTTP 请求（以及每条 SSE 连接）读到的进程级句柄。
//!
//! 从 `app/src/main.rs` 整块搬来（`docs/08` 的 P1.4），代码一行未改，只有可见性从私有放宽到
//! `pub(crate)`（字段要跨模块读）。`main.rs` 用
//! `pub(crate) use http::state::{AppState, EventStreamState};` 把两个名字接回原位，于是
//! `crate::AppState` 的 71 处引用与 `crate::EventStreamState` 的 4 处引用一个字都不用改。

use std::{
    collections::{HashSet, VecDeque},
    sync::Arc,
};

use nekowriter_agent::engine::AccumulationEngine;
use nekowriter_store::{PersistedEvent, Store};
use tokio::sync::{Mutex, Semaphore, broadcast, watch};

use crate::lifecycle::Shutdown;
use crate::scheduler;

#[derive(Clone)]
pub(crate) struct AppState {
    pub(crate) store: Store,
    pub(crate) allowed_write_origins: Arc<HashSet<String>>,
    pub(crate) settings_write_lock: Arc<Mutex<()>>,
    pub(crate) model_request_permit: Arc<Semaphore>,
    pub(crate) scheduler: scheduler::WorkflowScheduler,
    /// 仅供当前 SSE 连接消费的模型字符流；它不进入持久事件日志。
    pub(crate) preview_deltas: broadcast::Sender<scheduler::PreviewDelta>,
    pub(crate) shutdown: Shutdown,
    /// 本次进程实际拥有的知识积累引擎；能力查询与积累入口读同一份。MVP 为空。
    pub(crate) accumulation_engines: &'static [&'static dyn AccumulationEngine],
}

pub(crate) struct EventStreamState {
    pub(crate) store: Store,
    pub(crate) receiver: broadcast::Receiver<()>,
    pub(crate) preview_receiver: broadcast::Receiver<scheduler::PreviewDelta>,
    pub(crate) shutdown: watch::Receiver<bool>,
    pub(crate) cursor: i64,
    pub(crate) pending: VecDeque<PersistedEvent>,
    pub(crate) send_connection_comment: bool,
}
