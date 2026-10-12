use std::sync::Arc;

use anyhow::Result;
use tokio::{sync::mpsc, task::JoinHandle};

use crate::{
    app::{Application, CommandRequest},
    miner::Miner,
    twitch::TwitchError,
};

/// The process entry point owns this handle and awaits shutdown before exiting.
pub struct Runtime {
    pub app: Arc<Application>,
    worker: Option<JoinHandle<Result<(), TwitchError>>>,
    notifications: Option<JoinHandle<()>>,
}

impl Runtime {
    #[cfg(feature = "dashboard-fixture")]
    pub fn fixture(app: Arc<Application>, worker: JoinHandle<Result<(), TwitchError>>) -> Self {
        Self {
            app,
            worker: Some(worker),
            notifications: None,
        }
    }

    pub fn start(app: Arc<Application>, commands: mpsc::Receiver<CommandRequest>) -> Self {
        let events = app.notifications.subscribe();
        let notifications = tokio::spawn(crate::notifications::run(app.clone(), events));
        let owned = app.clone();
        let worker = tokio::spawn(async move {
            let result = Miner::new(owned.clone(), commands).run().await;
            owned.shutdown.cancel();
            result
        });
        Self {
            app,
            worker: Some(worker),
            notifications: Some(notifications),
        }
    }

    pub async fn wait(&mut self) -> Result<()> {
        let Some(worker) = &mut self.worker else {
            return Ok(());
        };
        let result = worker.await;
        self.worker = None;
        self.app.shutdown.cancel();
        if let Some(notifications) = self.notifications.take() {
            let _ = notifications.await;
        }
        anyhow::ensure!(matches!(result, Ok(Ok(()))), "mining task failed");
        Ok(())
    }

    pub async fn shutdown(&mut self) -> Result<()> {
        self.app.shutdown.cancel();
        let result = self.wait().await;
        self.app.drain_writes().await;
        result
    }
}

impl Drop for Runtime {
    fn drop(&mut self) {
        // Also cancel on an early-return/panic path; normal exit always awaits shutdown.
        self.app.shutdown.cancel();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};

    #[tokio::test]
    async fn shutdown_drains_worker_and_accepted_writes_and_is_repeatable() {
        let directory = tempfile::tempdir().unwrap();
        let (app, _commands) = Application::open(directory.path().into()).unwrap();
        let written = Arc::new(AtomicBool::new(false));
        let finished = written.clone();
        let cancel = app.shutdown.clone();
        app.writes.spawn(async move {
            cancel.cancelled().await;
            tokio::task::yield_now().await;
            finished.store(true, Ordering::SeqCst);
        });
        let cancel = app.shutdown.clone();
        let worker = tokio::spawn(async move {
            cancel.cancelled().await;
            Ok(())
        });
        let mut runtime = Runtime {
            app,
            worker: Some(worker),
            notifications: None,
        };
        runtime.shutdown().await.unwrap();
        assert!(written.load(Ordering::SeqCst));
        assert!(!runtime.app.has_pending_writes());
        runtime.shutdown().await.unwrap();
    }
}
