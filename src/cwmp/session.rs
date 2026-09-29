//! Synchronous CWMP session loop.
//! Accepted assignments reach the persist callback before the next acknowledgment is sent.

use super::{
    model::{Assignments, DataModel, Device},
    rpc::Cpe,
    soap,
};
use crate::{
    error::{Error, Result},
    net::{Cancel, PostKind, Transport},
    progress::{Progress, Stage},
};
use std::time::{Duration, Instant};

/// Maximum duration of a session, also enforced inside network waits.
pub const SESSION_LIMIT: Duration = Duration::from_secs(15 * 60);

/// Inputs and callbacks for one synchronous protocol session.
pub struct SessionParams<'a, T, P, F> {
    /// Validated transport; receives the absolute session deadline before Inform.
    pub transport: &'a mut T,
    /// Selected provider/device bundle.
    pub device: &'a Device,
    /// Instantiated device parameter model.
    pub model: DataModel,
    /// Shared cancellation token.
    pub cancel: &'a Cancel,
    /// Absolute deadline is computed once from this duration.
    pub limit: Duration,
    /// Persist accepted credentials before acknowledgment.
    pub persist: P,
    /// Receive secret-free progress updates.
    pub progress: F,
}

struct Exchanger<'a, T, F> {
    transport: &'a mut T,
    cancel: &'a Cancel,
    deadline: Instant,
    context: Progress,
    progress: F,
}

impl<T: Transport, F: FnMut(Progress)> Exchanger<'_, T, F> {
    fn post(&mut self, body: &[u8], kind: PostKind) -> Result<Vec<u8>> {
        self.cancel.check()?;
        if Instant::now() >= self.deadline {
            return Err(Error::Deadline);
        }
        self.context.stage = match kind {
            PostKind::Inform => Stage::Inform,
            PostKind::Session => Stage::Session,
        };
        (self.progress)(self.context);
        self.transport.post(body, kind)
    }
}

/// Run one protocol session, persisting accepted rotation before the next acknowledgment.
pub fn run<T, P, F>(params: SessionParams<'_, T, P, F>) -> Result<Assignments>
where
    T: Transport,
    P: FnMut(&Assignments) -> Result<()>,
    F: FnMut(Progress),
{
    let deadline = Instant::now()
        .checked_add(params.limit)
        .ok_or(Error::Internal)?;
    params.transport.set_deadline(Some(deadline));
    let mut exchange = Exchanger {
        transport: params.transport,
        cancel: params.cancel,
        deadline,
        context: Progress::default(),
        progress: params.progress,
    };
    let mut persist = params.persist;
    let mut cpe = Cpe::new(params.model);
    let reply = exchange.post(
        &soap::inform(&cpe.model, params.device.inform_parameters())?,
        PostKind::Inform,
    )?;
    let parsed = soap::parse(&reply)?;
    if !parsed
        .method()?
        .has_tag_name((soap::CWMP, "InformResponse"))
    {
        return Err(Error::Protocol);
    }
    let mut outgoing = vec![];
    loop {
        let reply = match exchange.post(&outgoing, PostKind::Session) {
            Ok(reply) => reply,
            // Cancellation, the deadline or a lost connection stops network work, but
            // must not discard an accepted, complete set of internet settings. Capture
            // will save it locally. Explicit provider and identity failures still fail.
            Err(
                Error::Cancelled
                | Error::Deadline
                | Error::Dns
                | Error::Connect
                | Error::Timeout
                | Error::Network,
            ) if params.device.export(&cpe.received).is_ok() => {
                return Ok(cpe.received);
            }
            Err(error) => return Err(error),
        };
        if reply.iter().all(u8::is_ascii_whitespace) {
            return Ok(cpe.received);
        }
        let handled = cpe.handle(&reply);
        exchange.context.rpc = handled.rpc;
        (exchange.progress)(exchange.context);
        outgoing = handled.reply?;
        // Do not check cancellation or deadline between accepting the RPC and this save.
        let saved = persist(&cpe.received);
        // A replacement may already have happened even when durability failed.
        // Publish that fact before propagating the storage error.
        (exchange.progress)(exchange.context);
        saved?;
    }
}
