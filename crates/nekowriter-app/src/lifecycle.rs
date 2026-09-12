//! 进程生命周期：日志落地、启动时打开浏览器，以及退出协调。
//!
//! 退出只有一条路径：进程信号与 `POST /api/system/shutdown` 都汇到 [`Shutdown`]，
//! 服务停止接受连接后由 `main` 依次取消在途请求、收尾持久化状态、落盘并关闭数据库。

use std::{path::Path, sync::Arc, time::Duration};

use tokio::sync::watch;
use tracing::{info, warn};
use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

use crate::StartupError;

const DEFAULT_LOG_FILTER: &str = "nekowriter=info,tower_http=info";
/// 数据目录内保留的滚动日志文件数；更早的日志在滚动时删除。
const LOG_FILE_RETENTION: usize = 7;
/// 退出响应先写回客户端，再真正停止服务。
const SHUTDOWN_RESPONSE_GRACE: Duration = Duration::from_millis(100);

/// 把日志同时写到标准输出和数据目录的 `logs/`：按天滚动、只保留有限数量文件。
///
/// 返回值是后台写线程的守卫，必须在进程存活期间持有，否则退出前的日志可能丢失。
pub fn init_logging(
    data_dir: &Path,
) -> Result<tracing_appender::non_blocking::WorkerGuard, StartupError> {
    let logs_dir = data_dir.join("logs");
    std::fs::create_dir_all(&logs_dir)?;
    let appender = RollingFileAppender::builder()
        .rotation(Rotation::DAILY)
        .filename_prefix("nekowriter")
        .filename_suffix("log")
        .max_log_files(LOG_FILE_RETENTION)
        .build(&logs_dir)?;
    let (file_writer, guard) = tracing_appender::non_blocking(appender);
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| DEFAULT_LOG_FILTER.into());
    tracing_subscriber::registry()
        .with(filter)
        .with(
            tracing_subscriber::fmt::layer()
                .with_target(false)
                .compact(),
        )
        .with(
            tracing_subscriber::fmt::layer()
                .with_target(false)
                .with_ansi(false)
                .with_writer(file_writer)
                .compact(),
        )
        .init();
    Ok(guard)
}

/// 用系统默认浏览器打开工作台。
///
/// 打开失败不影响服务：地址已经写进日志与界面，用户手动访问即可。
pub fn open_browser(port: u16) {
    let url = format!("http://127.0.0.1:{port}/");
    match webbrowser::open(&url) {
        Ok(()) => info!(%url, "已在默认浏览器打开工作台"),
        Err(error) => warn!(%url, %error, "无法自动打开浏览器，请手动访问该地址"),
    }
}

/// 退出协调器：接口请求与进程信号共用同一个触发信号。
#[derive(Clone)]
pub struct Shutdown {
    sender: Arc<watch::Sender<bool>>,
}

impl Shutdown {
    #[must_use]
    pub fn new() -> Self {
        let (sender, _) = watch::channel(false);
        Self {
            sender: Arc::new(sender),
        }
    }

    /// 订阅退出信号；已经退出时返回的接收者立刻可见 `true`。
    #[must_use]
    pub fn subscribe(&self) -> watch::Receiver<bool> {
        self.sender.subscribe()
    }

    #[must_use]
    pub fn is_requested(&self) -> bool {
        *self.sender.borrow()
    }

    #[must_use]
    pub fn requested(receiver: &watch::Receiver<bool>) -> bool {
        *receiver.borrow()
    }

    /// 请求退出服务。重复调用无副作用。
    ///
    /// 用 `send_replace` 而不是 `send`：`send` 在没有接收者时既不更新值也不唤醒，
    /// 而退出信号必须先于等待者成立。
    pub fn request(&self) {
        let _ = self.sender.send_replace(true);
    }

    /// 接口请求的退出：先让 HTTP 响应抵达客户端，再触发收尾。
    pub fn request_after_response(&self) {
        let shutdown = self.clone();
        tokio::spawn(async move {
            tokio::time::sleep(SHUTDOWN_RESPONSE_GRACE).await;
            shutdown.request();
        });
    }

    /// 等待 Ctrl+C、SIGTERM 或接口请求，并把触发结果写进同一个信号。
    ///
    /// 信号路径必须在返回前调用 [`Self::request`]：否则 SSE 与其他等待者看不到
    /// 「已请求退出」，持久连接不会结束，优雅退出会一直等它们断开。
    pub async fn wait(&self) {
        let mut receiver = self.sender.subscribe();
        if Self::requested(&receiver) {
            return;
        }
        tokio::select! {
            () = wait_for_process_signal() => info!("收到退出信号，正在停止服务"),
            _ = receiver.changed() => info!("收到界面退出请求，正在停止服务"),
        }
        self.request();
    }
}

impl Default for Shutdown {
    fn default() -> Self {
        Self::new()
    }
}

async fn wait_for_process_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};

        let mut terminate = match signal(SignalKind::terminate()) {
            Ok(stream) => Some(stream),
            Err(error) => {
                warn!(%error, "无法监听 SIGTERM，本次运行只响应 Ctrl+C");
                None
            }
        };
        tokio::select! {
            result = tokio::signal::ctrl_c() => await_interrupt(result).await,
            () = async {
                match terminate.as_mut() {
                    Some(stream) => {
                        stream.recv().await;
                    }
                    None => std::future::pending::<()>().await,
                }
            } => {}
        }
    }
    #[cfg(not(unix))]
    await_interrupt(tokio::signal::ctrl_c().await);
}

/// 监听失败不能当成收到退出信号：否则退出流程会在一个没被请求的时刻启动。
async fn await_interrupt(result: std::io::Result<()>) {
    if let Err(error) = result {
        warn!(%error, "无法监听 Ctrl+C；本次运行只能响应 SIGTERM 或进程终止");
        std::future::pending::<()>().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn shutdown_wakes_every_waiter_and_keeps_satisfying_new_ones() {
        let shutdown = Shutdown::new();
        assert!(!shutdown.is_requested());

        let first = shutdown.clone();
        let first_waiter = tokio::spawn(async move { first.wait().await });
        shutdown.request();
        first_waiter.await.unwrap();
        assert!(shutdown.is_requested());

        // 触发之后才创建的接收者与等待者都立刻看到已退出。
        let receiver = shutdown.subscribe();
        assert!(Shutdown::requested(&receiver));
        shutdown.wait().await;
    }

    #[test]
    fn request_without_any_subscriber_is_still_recorded() {
        let shutdown = Shutdown::new();
        shutdown.request();
        assert!(shutdown.is_requested());
        assert!(Shutdown::requested(&shutdown.subscribe()));
    }

    #[test]
    fn subscriber_created_before_the_request_sees_it() {
        let shutdown = Shutdown::new();
        let mut receiver = shutdown.subscribe();
        shutdown.request();
        assert!(*receiver.borrow_and_update());
    }
}
