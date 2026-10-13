//! The shutdown signal.
//!
//! Ctrl+C, SIGTERM and the "exit service" request all trigger the same [`Shutdown`]. The
//! HTTP server stops accepting connections, open event streams end, and then the database
//! is closed.

use tokio::sync::watch;

/// A one-way switch from running to shutting down. Clones share the switch.
#[derive(Clone, Debug)]
pub struct Shutdown {
    state: watch::Sender<bool>,
}

impl Default for Shutdown {
    fn default() -> Self {
        Self::new()
    }
}

impl Shutdown {
    #[must_use]
    pub fn new() -> Self {
        Self {
            state: watch::Sender::new(false),
        }
    }

    /// Starts shutting down. Triggering again has no further effect.
    pub fn trigger(&self) {
        // `send_replace` updates the value even when nobody is waiting yet.
        self.state.send_replace(true);
    }

    #[must_use]
    pub fn is_triggered(&self) -> bool {
        *self.state.borrow()
    }

    /// Completes once shutdown has been triggered.
    pub async fn triggered(&self) {
        let mut receiver = self.state.subscribe();
        // The sender lives in `self`, so the channel cannot close while this waits.
        let _ = receiver.wait_for(|triggered| *triggered).await;
    }

    /// Triggers shutdown when the process receives Ctrl+C or, on Unix, SIGTERM.
    pub fn trigger_on_signals(&self) {
        let shutdown = self.clone();
        tokio::spawn(async move {
            terminate_signal().await;
            shutdown.trigger();
        });
    }
}

async fn terminate_signal() {
    let ctrl_c = async {
        if let Err(error) = tokio::signal::ctrl_c().await {
            tracing::error!(%error, "cannot listen for Ctrl+C");
            std::future::pending::<()>().await;
        }
    };
    #[cfg(unix)]
    let sigterm = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut signal) => {
                signal.recv().await;
            }
            Err(error) => {
                tracing::error!(%error, "cannot listen for SIGTERM");
                std::future::pending::<()>().await;
            }
        }
    };
    #[cfg(not(unix))]
    let sigterm = std::future::pending::<()>();
    tokio::select! {
        () = ctrl_c => {}
        () = sigterm => {}
    }
}
