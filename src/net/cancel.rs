//! Cooperative cancellation token shared by network and session code.
use crate::error::{Error, Result};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

#[derive(Clone, Default)]
/// Shared flag that lets a session stop blocking network work promptly.
pub struct Cancel(Arc<AtomicBool>);
impl Cancel {
    /// Request cancellation from any thread holding a clone of this token.
    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }
    /// Return whether cancellation has been requested.
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
    /// Return a session cancellation error when the token is set.
    pub fn check(&self) -> Result<()> {
        if self.is_cancelled() {
            Err(Error::Cancelled)
        } else {
            Ok(())
        }
    }
}
