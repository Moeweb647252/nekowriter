use std::{net::SocketAddr, sync::Arc};

mod dashboard;
mod http;
mod knowledge;
mod lifecycle;
mod manuscript;
mod model_settings;
mod operations;
mod prompts;
mod recycle;
mod scheduler;
mod startup;
mod templates;

use clap::Parser;
use nekowriter_store::InterruptionSource;

use crate::lifecycle::Shutdown;

use startup::{Cli, StartupError, prepare};
use tokio::sync::{Mutex, Semaphore, broadcast};
use tracing::{info, warn};

pub(crate) use http::error::ApiError;
pub(crate) use http::gate::allowed_write_origins;
pub(crate) use http::router;
pub(crate) use http::state::AppState;

#[tokio::main]
async fn main() -> Result<(), StartupError> {
    let cli = Cli::parse();
    // `startup` 必须存活到进程结束：日志守卫与进程锁都在它里面。
    let startup = prepare(&cli).await?;
    let store = startup.store.clone();
    let data_dir = startup.data_dir.clone();
    let web_dir = startup.web_dir.clone();
    if !web_dir.exists() {
        warn!(path = %web_dir.display(), "未找到前端构建产物；可使用 pnpm --dir webui build 后重启，或在开发模式下运行 Vite");
    }

    let shutdown = Shutdown::new();
    let model_request_permit = Arc::new(Semaphore::new(1));
    let settings_write_lock = Arc::new(Mutex::new(()));
    let (preview_deltas, _) = broadcast::channel(256);
    let state = AppState {
        scheduler: scheduler::WorkflowScheduler::with_preview_deltas(
            store.clone(),
            model_request_permit.clone(),
            settings_write_lock.clone(),
            preview_deltas.clone(),
        ),
        store,
        allowed_write_origins: Arc::new(allowed_write_origins(cli.port)),
        settings_write_lock,
        model_request_permit,
        preview_deltas,
        shutdown: shutdown.clone(),
        accumulation_engines: nekowriter_agent::engine::ACCUMULATION_ENGINES,
    };
    // 上次进程退出时仍在 running 的索引任务重新排队；向量本身可由正文重建。
    let requeued = state.store.requeue_running_index_tasks().await?;
    if requeued > 0 {
        info!(requeued, "已重新排队未完成的知识索引任务");
    }
    let indexer_store = state.store.clone();
    let indexer = tokio::spawn(async move {
        knowledge::KnowledgeIndexer::new(indexer_store).run().await;
    });
    let app = router(state.clone(), &web_dir);
    let address = SocketAddr::from(([127, 0, 0, 1], cli.port));
    let listener = tokio::net::TcpListener::bind(address).await?;
    info!(address = %address, data_dir = %data_dir.display(), "NekoWriter 已启动");
    // 长期增长口径：请求快照、命令回执与消息账本是审计事实，不自动回收；把各表当前的
    // 行数与重文本字节数写到日志，长年使用后的体积因此可诊断（见 docs/02 §11.3）。
    match state.store.storage_growth().await {
        Ok(growth) => info!(
            table = %growth
                .iter()
                .map(|table| format!("{}={}行/{}字节", table.table, table.rows, table.text_bytes))
                .collect::<Vec<_>>()
                .join(" "),
            "本地数据规模"
        ),
        Err(error) => warn!(%error, "本地数据规模统计失败"),
    }
    if !cli.no_browser {
        lifecycle::open_browser(cli.port);
    }

    // 退出信号（Ctrl+C / SIGTERM / `POST /api/system/shutdown`）一到达，服务器就
    // 停止接受新连接，因此退出流程不再产生新的生成请求。
    let serve = {
        let shutdown = shutdown.clone();
        async move { shutdown.wait().await }
    };
    axum::serve(listener, app)
        .with_graceful_shutdown(serve)
        .await?;

    info!("正在取消在途请求并收尾状态");
    state.scheduler.cancel_in_flight().await;
    indexer.abort();
    let outcome = state
        .store
        .interrupt_active_runs(InterruptionSource::ProcessShutdown)
        .await?;
    state.store.close().await?;
    info!(
        writing_runs = outcome.writing_runs,
        review_delegations = outcome.review_delegations,
        "NekoWriter 已退出"
    );
    Ok(())
}
