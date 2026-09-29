//! One blocking session worker and cancellation channel.
use crate::{
    capture::{self, Failure, Outcome},
    cwmp::model::Device,
    domain::serial::Serial,
    error::Error,
    net::Cancel,
    progress::Progress,
    store::Store,
};
use std::{
    sync::{Arc, mpsc},
    thread::{self, JoinHandle},
};

pub(super) struct Worker {
    pub(super) cancel: Cancel,
    pub(super) rx: mpsc::Receiver<Progress>,
    handle: Option<JoinHandle<std::result::Result<Outcome, Failure>>>,
}
impl Worker {
    #[allow(clippy::panic)] // Development-only trigger checks recovery without printing payloads.
    pub(super) fn start(store: Arc<Store>, serial: Serial, device: Arc<Device>) -> Self {
        let cancel = Cancel::default();
        let token = cancel.clone();
        let (tx, rx) = mpsc::channel();
        let handle = thread::spawn(move || {
            #[cfg(feature = "dev-provider-override")]
            if std::env::var_os("YETTEL_CWMP_DEV_WORKER_PANIC").is_some() {
                panic!("synthetic-worker-payload-must-never-appear");
            }
            capture::capture(&store, &serial, &device, &token, |progress| {
                let _ = tx.send(progress);
            })
        });
        Self {
            cancel,
            rx,
            handle: Some(handle),
        }
    }

    /// Join after the progress channel closes, preserving its final safe progress on panic.
    pub(super) fn finish(mut self, progress: Progress) -> std::result::Result<Outcome, Failure> {
        match self.handle.take().map(JoinHandle::join) {
            Some(Ok(result)) => result,
            _ => Err(Failure {
                error: Error::Internal,
                progress,
            }),
        }
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.cancel.cancel();
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}
