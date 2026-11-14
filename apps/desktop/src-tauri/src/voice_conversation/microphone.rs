//! P1-04: pure, test-only ownership and cleanup bookkeeping.
//! Tickets are not consent, egress capabilities or evidence of real hardware state.
use super::clock::{Deadline, MonotonicTime};
use super::limits::{MAX_CAPTURED_TURN, MAX_WAKE_ARMING};
use super::model::*;
use super::ContractError;
use std::num::NonZeroU64;
use std::sync::Arc;
use std::time::Duration;

const RELEASE_WARNING_AFTER: Duration = Duration::from_secs(2);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Error {
    Contract(ContractError),
    InvalidOwner,
    Busy,
    Stale,
    Unauthorized,
    Expired,
    ReleaseUnconfirmed,
    CleanupUnconfirmed,
    Closed,
}
impl From<ContractError> for Error {
    fn from(error: ContractError) -> Self {
        Self::Contract(error)
    }
}
type Result<T> = std::result::Result<T, Error>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct RequestId(NonZeroU64);
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct LeaseId(NonZeroU64);

#[derive(Clone, Debug)]
struct RequestTicket {
    id: RequestId,
    identity: ContractIdentity,
    owner: MicrophoneOwner,
    generation: Generation,
    native_instance: Arc<()>,
    nonce: Arc<()>,
}
impl RequestTicket {
    fn matches(&self, other: &Self) -> bool {
        self.id == other.id
            && self.identity == other.identity
            && self.owner == other.owner
            && self.generation == other.generation
            && Arc::ptr_eq(&self.native_instance, &other.native_instance)
            && Arc::ptr_eq(&self.nonce, &other.nonce)
    }
}

#[derive(Clone, Debug)]
struct Lease {
    id: LeaseId,
    request: RequestTicket,
    nonce: Arc<()>,
}
impl Lease {
    fn matches(&self, other: &Self) -> bool {
        self.id == other.id
            && self.request.matches(&other.request)
            && Arc::ptr_eq(&self.nonce, &other.nonce)
    }
}

#[derive(Clone, Debug)]
struct ReleaseTicket {
    lease: Lease,
    nonce: Arc<()>,
}
impl ReleaseTicket {
    fn matches(&self, other: &Self) -> bool {
        self.lease.matches(&other.lease) && Arc::ptr_eq(&self.nonce, &other.nonce)
    }
}

#[derive(Clone, Debug)]
struct CleanupTicket {
    release: ReleaseTicket,
    nonce: Arc<()>,
}
impl CleanupTicket {
    fn matches(&self, other: &Self) -> bool {
        self.release.matches(&other.release) && Arc::ptr_eq(&self.nonce, &other.nonce)
    }
}

/// Synthetic facts supplied by future independent validation, never a grant issuer.
struct Validation {
    identity: ContractIdentity,
    deadline: Deadline,
    activation_allowed: bool,
    background_allowed: bool,
    explicit_manual_start: bool,
    foreground: bool,
    privacy_clear: bool,
    output_idle: bool,
}
impl Validation {
    fn allowed(&self, owner: MicrophoneOwner) -> bool {
        self.activation_allowed
            && self.privacy_clear
            && self.output_idle
            && match owner {
                MicrophoneOwner::Off => false,
                MicrophoneOwner::ManualForeground => self.explicit_manual_start && self.foreground,
                _ => self.foreground || self.background_allowed,
            }
    }
}

struct Request {
    ticket: RequestTicket,
    deadline: Deadline,
    grant_deadline: Deadline,
}

enum Phase {
    Acquiring,
    Owned,
    Stopping {
        ticket: ReleaseTicket,
        // Clock-independent Stop leaves this unset until a valid tick samples it.
        since: Option<MonotonicTime>,
    },
    Cleaning(CleanupTicket),
}

struct Occupancy {
    lease: Lease,
    deadline: Deadline,
    grant_deadline: Deadline,
    phase: Phase,
}

/// Fake native evidence only. Release includes completion/cancellation of the
/// acquisition operation: there can be no later successful engine start behind ACK.
struct FakeReleaseAck {
    ticket: ReleaseTicket,
    acquisition_settled: bool,
    input_stopped: bool,
}
struct FakeCleanupAck {
    ticket: CleanupTicket,
    frames_cleared: bool,
    queues_cleared: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Status {
    /// The reserved owner is retained through uncertain release and cleanup.
    owner: MicrophoneOwner,
    release: MicrophoneReleaseStatus,
    listening: ListeningStatus,
    cancel_requested: bool,
    stop_input_requested: bool,
    cleanup_requested: bool,
    missing_ack_warning: bool,
    faulted: bool,
}

struct MicrophoneCoordinator {
    run: AppRunId,
    native_instance: Arc<()>,
    generation: Generation,
    request_sequence: u64,
    lease_sequence: u64,
    closed: bool,
    faulted: bool,
    missing_ack_warning: bool,
    last_time: MonotonicTime,
    current: Option<Occupancy>,
    pending: Option<Request>,
}

impl MicrophoneCoordinator {
    fn new(run: AppRunId, now: MonotonicTime) -> Result<Self> {
        Ok(Self {
            run,
            native_instance: Arc::new(()),
            generation: Generation::from_native_sequence(1)?,
            request_sequence: 0,
            lease_sequence: 0,
            closed: false,
            faulted: false,
            missing_ack_warning: false,
            last_time: now,
            current: None,
            pending: None,
        })
    }

    fn check_time(&self, now: &MonotonicTime) -> Result<()> {
        now.checked_duration_since(&self.last_time)?;
        Ok(())
    }

    fn status(&self) -> Status {
        let (owner, release, listening, stopping, cleaning) = match &self.current {
            None => (
                MicrophoneOwner::Off,
                MicrophoneReleaseStatus::NotRequested,
                ListeningStatus::NotListening,
                false,
                false,
            ),
            Some(held) => {
                let owner = held.lease.request.owner;
                match held.phase {
                    Phase::Owned => (
                        owner,
                        MicrophoneReleaseStatus::NotRequested,
                        ListeningStatus::Listening,
                        false,
                        false,
                    ),
                    Phase::Acquiring => (
                        owner,
                        MicrophoneReleaseStatus::NotRequested,
                        ListeningStatus::Unconfirmed,
                        false,
                        false,
                    ),
                    Phase::Stopping { .. } => (
                        owner,
                        MicrophoneReleaseStatus::StoppingUnconfirmed,
                        ListeningStatus::Unconfirmed,
                        true,
                        false,
                    ),
                    Phase::Cleaning(_) => (
                        owner,
                        MicrophoneReleaseStatus::Acknowledged,
                        ListeningStatus::NotListening,
                        false,
                        true,
                    ),
                }
            }
        };
        Status {
            owner,
            release,
            listening,
            cancel_requested: stopping,
            stop_input_requested: stopping,
            cleanup_requested: cleaning,
            missing_ack_warning: self.missing_ack_warning,
            faulted: self.faulted,
        }
    }

    fn request(
        &mut self,
        owner: MicrophoneOwner,
        duration: Duration,
        facts: &Validation,
        now: &MonotonicTime,
    ) -> Result<RequestTicket> {
        if self.closed {
            return Err(Error::Closed);
        }
        if owner == MicrophoneOwner::Off {
            return Err(Error::InvalidOwner);
        }
        if self.pending.is_some() || self.faulted {
            return Err(Error::Busy);
        }
        self.check_time(now)?;
        let expired = facts.deadline.is_expired(now)?;
        if facts.identity.app_run_id != self.run || !facts.allowed(owner) || expired {
            return Err(Error::Unauthorized);
        }
        let maximum = if owner == MicrophoneOwner::WakeDetection {
            MAX_WAKE_ARMING
        } else {
            MAX_CAPTURED_TURN
        };
        if duration.is_zero() || duration > maximum {
            return Err(ContractError::LimitExceeded.into());
        }
        let deadline = Deadline::after(now.clone(), duration)?;
        let sequence = self
            .request_sequence
            .checked_add(1)
            .ok_or(ContractError::Overflow)?;
        let id = RequestId(NonZeroU64::new(sequence).ok_or(ContractError::InvalidIdentifier)?);
        // Revoke before exposing cancel/stop requests or creating the next ticket.
        // Overflow still publishes a stop request, so preserve its validated time.
        self.last_time = now.clone();
        self.revoke(Some(now))?;
        let ticket = RequestTicket {
            id,
            identity: facts.identity,
            owner,
            generation: self.generation,
            native_instance: self.native_instance.clone(),
            nonce: Arc::new(()),
        };
        self.pending = Some(Request {
            ticket: ticket.clone(),
            deadline,
            grant_deadline: facts.deadline.clone(),
        });
        self.request_sequence = sequence;
        self.last_time = now.clone();
        Ok(ticket)
    }

    /// A separate explicit step. ACK/cleanup never starts a pending acquisition.
    fn begin(
        &mut self,
        ticket: &RequestTicket,
        facts: &Validation,
        now: &MonotonicTime,
    ) -> Result<Lease> {
        if self.closed {
            return Err(Error::Closed);
        }
        let pending = self.pending.as_ref().ok_or(Error::Stale)?;
        if !pending.ticket.matches(ticket) || ticket.generation != self.generation {
            return Err(Error::Stale);
        }
        if self.current.is_some() {
            return Err(Error::Busy);
        }
        self.check_time(now)?;
        let expired = pending.deadline.is_expired(now)?;
        let grant_expired = pending.grant_deadline.is_expired(now)?;
        let fresh_expired = facts.deadline.is_expired(now)?;
        if expired || grant_expired || fresh_expired {
            return Err(Error::Expired);
        }
        if facts.identity != ticket.identity || !facts.allowed(ticket.owner) {
            return Err(Error::Unauthorized);
        }
        let grant_deadline = Self::earlier(&pending.grant_deadline, &facts.deadline, now)?;
        let sequence = self
            .lease_sequence
            .checked_add(1)
            .ok_or(ContractError::Overflow)?;
        let lease = Lease {
            id: LeaseId(NonZeroU64::new(sequence).ok_or(ContractError::InvalidIdentifier)?),
            request: ticket.clone(),
            nonce: Arc::new(()),
        };
        let pending = self.pending.take().unwrap();
        self.current = Some(Occupancy {
            lease: lease.clone(),
            deadline: pending.deadline,
            grant_deadline,
            phase: Phase::Acquiring,
        });
        self.lease_sequence = sequence;
        self.last_time = now.clone();
        Ok(lease)
    }

    fn acquired(
        &mut self,
        lease: &Lease,
        ready: bool,
        facts: &Validation,
        now: &MonotonicTime,
    ) -> Result<()> {
        let held = self.current.as_ref().ok_or(Error::Stale)?;
        if !matches!(held.phase, Phase::Acquiring)
            || !held.lease.matches(lease)
            || lease.request.generation != self.generation
            || self.closed
        {
            return Err(Error::Stale);
        }
        self.check_time(now)?;
        let expired = held.deadline.is_expired(now)?;
        let grant_expired = held.grant_deadline.is_expired(now)?;
        let fresh_expired = facts.deadline.is_expired(now)?;
        if facts.identity != lease.request.identity {
            return Err(Error::Stale);
        }
        if !ready
            || expired
            || grant_expired
            || fresh_expired
            || !facts.allowed(lease.request.owner)
        {
            self.last_time = now.clone();
            self.stop()?;
            return Err(Error::Unauthorized);
        }
        let grant_deadline = Self::earlier(&held.grant_deadline, &facts.deadline, now)?;
        let held = self.current.as_mut().unwrap();
        held.grant_deadline = grant_deadline;
        held.phase = Phase::Owned;
        self.last_time = now.clone();
        Ok(())
    }

    fn accepts(&self, lease: &Lease, now: &MonotonicTime) -> Result<bool> {
        let held = self.current.as_ref().ok_or(Error::Stale)?;
        if !matches!(held.phase, Phase::Owned)
            || !held.lease.matches(lease)
            || lease.request.generation != self.generation
            || self.closed
        {
            return Err(Error::Stale);
        }
        self.check_time(now)?;
        let expired = held.deadline.is_expired(now)?;
        let grant_expired = held.grant_deadline.is_expired(now)?;
        Ok(!expired && !grant_expired)
    }

    fn earlier(first: &Deadline, second: &Deadline, now: &MonotonicTime) -> Result<Deadline> {
        Ok(if first.remaining(now)? <= second.remaining(now)? {
            first.clone()
        } else {
            second.clone()
        })
    }

    fn revoke(&mut self, at: Option<&MonotonicTime>) -> Result<()> {
        let Some(held) = self.current.as_mut() else {
            return Ok(());
        };
        if matches!(held.phase, Phase::Stopping { .. } | Phase::Cleaning(_)) {
            return Ok(());
        }
        let next = self.generation.checked_next();
        match next {
            Ok(generation) => self.generation = generation,
            Err(_) => self.closed = true,
        }
        held.phase = Phase::Stopping {
            ticket: ReleaseTicket {
                lease: held.lease.clone(),
                nonce: Arc::new(()),
            },
            since: at.cloned(),
        };
        next?;
        Ok(())
    }

    /// No clock/provider/storage dependency. Repeated Stop preserves ACK tickets.
    fn stop(&mut self) -> Result<()> {
        self.pending = None;
        self.revoke(None)
    }

    fn cancel_pending(&mut self, ticket: &RequestTicket) -> Result<()> {
        if self
            .pending
            .as_ref()
            .is_some_and(|p| p.ticket.matches(ticket))
        {
            self.pending = None;
            return Ok(());
        }
        if self.current.as_ref().is_some_and(|held| {
            matches!(held.phase, Phase::Acquiring) && held.lease.request.matches(ticket)
        }) {
            return self.stop();
        }
        Err(Error::Stale)
    }

    fn release_ticket(&self) -> Option<ReleaseTicket> {
        match &self.current.as_ref()?.phase {
            Phase::Stopping { ticket, .. } => Some(ticket.clone()),
            _ => None,
        }
    }
    fn cleanup_ticket(&self) -> Option<CleanupTicket> {
        match &self.current.as_ref()?.phase {
            Phase::Cleaning(ticket) => Some(ticket.clone()),
            _ => None,
        }
    }

    fn release_ack(&mut self, ack: &FakeReleaseAck) -> Result<()> {
        let expected = self.release_ticket().ok_or(Error::Stale)?;
        if !expected.matches(&ack.ticket) {
            return Err(Error::Stale);
        }
        if !ack.acquisition_settled || !ack.input_stopped {
            self.faulted = true;
            self.pending = None;
            return Err(Error::ReleaseUnconfirmed);
        }
        self.current.as_mut().unwrap().phase = Phase::Cleaning(CleanupTicket {
            release: expected,
            nonce: Arc::new(()),
        });
        self.missing_ack_warning = false;
        Ok(())
    }

    fn cleanup_ack(&mut self, ack: &FakeCleanupAck) -> Result<()> {
        let expected = self.cleanup_ticket().ok_or(Error::Stale)?;
        if !expected.matches(&ack.ticket) {
            return Err(Error::Stale);
        }
        if !ack.frames_cleared || !ack.queues_cleared {
            self.faulted = true;
            self.pending = None;
            return Err(Error::CleanupUnconfirmed);
        }
        self.current = None;
        self.faulted = false;
        self.missing_ack_warning = false;
        Ok(())
    }

    fn tick(&mut self, now: &MonotonicTime) -> Result<()> {
        if let Err(error) = self.check_time(now) {
            let _ = self.stop(); // fail closed even if generation allocation fails
            return Err(error);
        }
        let pending_expired = self
            .pending
            .as_ref()
            .map(|p| {
                let deadline = p.deadline.is_expired(now)?;
                let grant = p.grant_deadline.is_expired(now)?;
                Ok::<_, ContractError>(deadline || grant)
            })
            .transpose()?
            .unwrap_or(false);
        let current_expired = self
            .current
            .as_ref()
            .map(|held| {
                let deadline = held.deadline.is_expired(now)?;
                let grant = held.grant_deadline.is_expired(now)?;
                Ok::<_, ContractError>(deadline || grant)
            })
            .transpose()?
            .unwrap_or(false);
        if pending_expired {
            self.pending = None;
        }
        self.last_time = now.clone();
        if current_expired
            && self
                .current
                .as_ref()
                .is_some_and(|h| matches!(h.phase, Phase::Acquiring | Phase::Owned))
        {
            self.pending = None;
            self.revoke(Some(now))?;
        }
        if let Some(Occupancy {
            phase: Phase::Stopping { since, .. },
            ..
        }) = &mut self.current
        {
            let since = since.get_or_insert_with(|| now.clone());
            self.missing_ack_warning = now.checked_duration_since(since)? >= RELEASE_WARNING_AFTER;
        }
        Ok(())
    }

    /// Manual M45 never gains background scope. No host event starts acquisition.
    fn foreground_lost(&mut self) -> Result<()> {
        let manual_current = self
            .current
            .as_ref()
            .is_some_and(|held| held.lease.request.owner == MicrophoneOwner::ManualForeground);
        let manual_pending = self
            .pending
            .as_ref()
            .is_some_and(|p| p.ticket.owner == MicrophoneOwner::ManualForeground);
        if manual_current {
            return self.stop();
        }
        if manual_pending {
            self.pending = None;
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "microphone_tests.rs"]
mod tests;
