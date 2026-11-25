//! P1-10 deterministic, test-only independent Stop contract.
//!
//! Participants below are SYNTHETIC: existing session/audio/wake/speech/consent/
//! egress/history/adapter controllers keep private cancellation boundaries. This
//! module neither calls them nor claims runtime fanout. Tickets confer no real
//! consent, hardware ownership, provider access or action authority.
//!
//! Stop owns only bounded native bookkeeping and a shared revocation latch. It
//! takes no provider/renderer/storage locks, invokes no external code, allocates
//! no sequence, samples no clock and waits for no cleanup. A future bridge must
//! forward its requests through the existing independent native gates.
use super::clock::MonotonicTime;
use super::model::*;
use super::ContractError;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

const RELEASE_WARNING_AFTER: Duration = Duration::from_secs(2);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Error {
    Contract(ContractError),
    Stale,
    Closed,
    Busy,
    Unconfirmed,
}
impl From<ContractError> for Error {
    fn from(value: ContractError) -> Self {
        Self::Contract(value)
    }
}
type Result<T> = std::result::Result<T, Error>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StopReason {
    Explicit,
    Lock,
    Sleep,
    Logout,
    Quit,
    PermissionRevoked,
    DeviceLost,
    PrivacyRevoked,
    ConfigurationChanged,
    ClockFailure,
    CounterExhaustion,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Participant {
    Session,
    Wake,
    Stt,
    Conversation,
    Planning,
    Tts,
    Speech,
    PendingSpeech,
    ConsentPreview,
    History,
}
impl Participant {
    const ALL: [Self; 10] = [
        Self::Session,
        Self::Wake,
        Self::Stt,
        Self::Conversation,
        Self::Planning,
        Self::Tts,
        Self::Speech,
        Self::PendingSpeech,
        Self::ConsentPreview,
        Self::History,
    ];
    fn index(self) -> usize {
        self as usize
    }
    fn provider(self) -> bool {
        matches!(
            self,
            Self::Stt | Self::Conversation | Self::Planning | Self::Tts
        )
    }
}

/// Three independently described synthetic eligibility domains. A single
/// irreversible latch revokes all of them at Stop's first instruction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Eligibility {
    NewWork,
    Consent,
    Egress,
}

/// Observed synthetic lifecycle metadata, not a second transition machine.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stage {
    Session(SessionState),
    Turn(TurnState),
    Wake(WakeState),
    Speech(SpeechState),
    Planning,
    Consent,
    History,
}

#[derive(Clone, Debug)]
struct Ticket {
    native: Arc<AtomicBool>,
    nonce: Arc<()>,
    identity: ContractIdentity,
    sequence: u64,
    participant: Participant,
    stage: Stage,
}
impl Ticket {
    fn matches(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.native, &other.native)
            && Arc::ptr_eq(&self.nonce, &other.nonce)
            && self.identity == other.identity
            && self.sequence == other.sequence
            && self.participant == other.participant
            && self.stage == other.stage
    }
    fn eligible(&self, _domain: Eligibility) -> bool {
        self.native.load(Ordering::Acquire)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WorkState {
    Pending,
    Completed,
    Canceled,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EgressState {
    Reserved,
    Attempted,
    Uncertain,
    Canceled,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Cancellation {
    Confirmed,
    Failed,
    Uncertain,
}
struct Work {
    ticket: Ticket,
    state: WorkState,
    egress: Option<EgressState>,
    cancel_requested: bool,
    cancellation: Option<Cancellation>,
}

#[derive(Clone, Debug)]
struct InputLease {
    ticket: Ticket,
    owner: MicrophoneOwner,
    release_nonce: Arc<()>,
    cleanup_nonce: Arc<()>,
}
impl InputLease {
    fn matches(&self, other: &Self) -> bool {
        self.ticket.matches(&other.ticket)
            && self.owner == other.owner
            && Arc::ptr_eq(&self.release_nonce, &other.release_nonce)
            && Arc::ptr_eq(&self.cleanup_nonce, &other.cleanup_nonce)
    }
}
#[derive(Clone, Debug)]
struct ReleaseTicket(InputLease);
#[derive(Clone, Debug)]
struct FrameCleanupTicket(InputLease);

enum InputPhase {
    Acquiring,
    Capturing,
    ReleaseRequested,
    Cleaning,
}
struct Input {
    lease: InputLease,
    phase: InputPhase,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Cleanup {
    Output,
    Workers,
    Audit,
}
impl Cleanup {
    const ALL: [Self; 3] = [Self::Output, Self::Workers, Self::Audit];
}
#[derive(Clone, Debug)]
struct CleanupTicket {
    native: Arc<AtomicBool>,
    identity: ContractIdentity,
    kind: Cleanup,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Status {
    revoked: bool,
    local_cleanup_complete: bool,
    owner: MicrophoneOwner,
    release: MicrophoneReleaseStatus,
    listening: ListeningStatus,
    cancel_input_requested: bool,
    stop_input_requested: bool,
    frame_cleanup_requested: bool,
    missing_ack_warning: bool,
    cleanup_failed: bool,
    audit_failed: bool,
    generation_exhausted: bool,
}

struct StopSupervisor {
    identity: ContractIdentity,
    generation: Generation,
    native: Arc<AtomicBool>,
    sequence: u64,
    reason: Option<StopReason>,
    work: [Option<Work>; Participant::ALL.len()],
    input: Option<Input>,
    // Fixed one-shot cleanup requests. Provider results and audit never gate
    // local cleanup completion; output/owned-worker ACKs remain independent.
    cleanup_requested: [bool; Cleanup::ALL.len()],
    cleanup_result: [Option<bool>; Cleanup::ALL.len()],
    cleanup_failed: bool,
    audit_failed: bool,
    generation_exhausted: bool,
    last_time: MonotonicTime,
    warning_since: Option<MonotonicTime>,
    missing_ack_warning: bool,
    ephemeral_references: u64,
    reservations_consumed: u64,
}

impl StopSupervisor {
    fn synthetic(identity: ContractIdentity, now: MonotonicTime) -> Self {
        Self {
            identity,
            generation: identity.generation,
            native: Arc::new(AtomicBool::new(true)),
            sequence: 0,
            reason: None,
            work: std::array::from_fn(|_| None),
            input: None,
            cleanup_requested: [false; 3],
            cleanup_result: [None; 3],
            cleanup_failed: false,
            audit_failed: false,
            generation_exhausted: false,
            last_time: now,
            warning_since: None,
            missing_ack_warning: false,
            ephemeral_references: 0,
            reservations_consumed: 0,
        }
    }

    fn check_scope(&self, identity: &ContractIdentity) -> Result<()> {
        if !self.native.load(Ordering::Acquire) {
            return Err(Error::Closed);
        }
        // Turn/job may differ; every other native scope field must match.
        let mut expected = self.identity;
        expected.turn_id = identity.turn_id;
        expected.job_id = identity.job_id;
        if *identity != expected || identity.generation != self.generation {
            return Err(Error::Stale);
        }
        Ok(())
    }

    fn next_ticket(
        &mut self,
        participant: Participant,
        identity: ContractIdentity,
        stage: Stage,
    ) -> Result<Ticket> {
        self.check_scope(&identity)?;
        let Some(sequence) = self.sequence.checked_add(1) else {
            self.stop(StopReason::CounterExhaustion);
            return Err(ContractError::Overflow.into());
        };
        self.sequence = sequence;
        Ok(Ticket {
            native: self.native.clone(),
            nonce: Arc::new(()),
            identity,
            sequence,
            participant,
            stage,
        })
    }

    /// Bounded synthetic ingress, not an adapter/consent/provenance bridge.
    /// Slots are never reused in this cancellation epoch, even after completion.
    fn synthetic_work(
        &mut self,
        participant: Participant,
        identity: ContractIdentity,
        stage: Stage,
    ) -> Result<Ticket> {
        self.check_scope(&identity)?;
        if self.work[participant.index()].is_some() {
            return Err(Error::Busy);
        }
        let ticket = self.next_ticket(participant, identity, stage)?;
        self.work[participant.index()] = Some(Work {
            ticket: ticket.clone(),
            state: WorkState::Pending,
            egress: participant.provider().then_some(EgressState::Reserved),
            cancel_requested: false,
            cancellation: None,
        });
        // At most ten metadata references and four synthetic role reservations.
        self.ephemeral_references += 1;
        self.reservations_consumed += u64::from(participant.provider());
        Ok(ticket)
    }

    fn work_mut(&mut self, ticket: &Ticket) -> Result<&mut Work> {
        self.work[ticket.participant.index()]
            .as_mut()
            .filter(|w| w.ticket.matches(ticket))
            .ok_or(Error::Stale)
    }

    fn synthetic_dispatch(&mut self, ticket: &Ticket, uncertain: bool) -> Result<()> {
        self.check_scope(&ticket.identity)?;
        let work = self.work_mut(ticket)?;
        if work.state != WorkState::Pending || work.egress != Some(EgressState::Reserved) {
            return Err(Error::Stale);
        }
        work.egress = Some(if uncertain {
            EgressState::Uncertain
        } else {
            EgressState::Attempted
        });
        Ok(())
    }

    /// Success is metadata only; it cannot start capture, speech or planning.
    fn callback(&mut self, ticket: &Ticket) -> Result<()> {
        self.check_scope(&ticket.identity)?;
        let work = self.work_mut(ticket)?;
        if work.state != WorkState::Pending {
            return Err(Error::Stale);
        }
        work.state = WorkState::Completed;
        Ok(())
    }

    fn synthetic_input(
        &mut self,
        identity: ContractIdentity,
        owner: MicrophoneOwner,
        acquiring: bool,
    ) -> Result<InputLease> {
        self.check_scope(&identity)?;
        if self.input.is_some() || owner == MicrophoneOwner::Off {
            return Err(Error::Busy);
        }
        let lease = InputLease {
            ticket: self.next_ticket(
                Participant::Session,
                identity,
                Stage::Turn(TurnState::Listening),
            )?,
            owner,
            release_nonce: Arc::new(()),
            cleanup_nonce: Arc::new(()),
        };
        self.input = Some(Input {
            lease: lease.clone(),
            phase: if acquiring {
                InputPhase::Acquiring
            } else {
                InputPhase::Capturing
            },
        });
        Ok(lease)
    }

    /// Independent, clock-free, infallible, irreversible. No locks, callbacks,
    /// I/O, audit calls, provider cancellation calls or waits occur here.
    fn stop(&mut self, reason: StopReason) -> Status {
        self.native.store(false, Ordering::Release);
        if self.reason.is_none() {
            self.reason = Some(reason);
            // Revocation already took effect, including on generation overflow.
            match self.generation.checked_next() {
                Ok(generation) => self.generation = generation,
                Err(_) => self.generation_exhausted = true,
            }
            for work in self.work.iter_mut().flatten() {
                work.cancel_requested = work.state == WorkState::Pending;
                work.state = WorkState::Canceled;
                if work.egress.is_some() {
                    work.egress = Some(EgressState::Canceled);
                }
            }
            self.ephemeral_references = 0;
            if let Some(input) = &mut self.input {
                input.phase = InputPhase::ReleaseRequested;
            }
            self.cleanup_requested = [
                self.work[Participant::Speech.index()].is_some()
                    || self.work[Participant::PendingSpeech.index()].is_some()
                    || self.work[Participant::Tts.index()].is_some(),
                // Conservatively request owned-worker termination on all Stops.
                true,
                true,
            ];
        }
        self.status()
    }

    fn cancellation_result(&mut self, ticket: &Ticket, outcome: Cancellation) -> Result<()> {
        let work = self.work_mut(ticket)?;
        if !work.cancel_requested || work.cancellation.is_some() {
            return Err(Error::Stale);
        }
        work.cancellation = Some(outcome);
        // Never refund a reservation, restore authority, or enqueue a retry.
        Ok(())
    }

    fn release_ticket(&self) -> Option<ReleaseTicket> {
        self.input.as_ref().and_then(|input| {
            matches!(input.phase, InputPhase::ReleaseRequested)
                .then(|| ReleaseTicket(input.lease.clone()))
        })
    }

    /// Synthetic native evidence must settle a pending acquisition as well as
    /// stop input. Old generations remain valid ONLY for exact cleanup evidence.
    fn release_ack(
        &mut self,
        ticket: &ReleaseTicket,
        acquisition_settled: bool,
        input_stopped: bool,
    ) -> Result<()> {
        let expected = self.release_ticket().ok_or(Error::Stale)?;
        if !expected.0.matches(&ticket.0) {
            return Err(Error::Stale);
        }
        if !acquisition_settled || !input_stopped {
            self.cleanup_failed = true;
            return Err(Error::Unconfirmed);
        }
        self.input.as_mut().unwrap().phase = InputPhase::Cleaning;
        self.missing_ack_warning = false;
        Ok(())
    }

    fn frame_cleanup_ticket(&self) -> Option<FrameCleanupTicket> {
        self.input.as_ref().and_then(|input| {
            matches!(input.phase, InputPhase::Cleaning)
                .then(|| FrameCleanupTicket(input.lease.clone()))
        })
    }

    fn frame_cleanup_ack(
        &mut self,
        ticket: &FrameCleanupTicket,
        frames_cleared: bool,
        queues_cleared: bool,
    ) -> Result<()> {
        let expected = self.frame_cleanup_ticket().ok_or(Error::Stale)?;
        if !expected.0.matches(&ticket.0) {
            return Err(Error::Stale);
        }
        if !frames_cleared || !queues_cleared {
            self.cleanup_failed = true;
            return Err(Error::Unconfirmed);
        }
        self.input = None;
        Ok(())
    }

    fn cleanup_ticket(&self, kind: Cleanup) -> Option<CleanupTicket> {
        (self.cleanup_requested[kind as usize] && self.cleanup_result[kind as usize] != Some(true))
            .then(|| CleanupTicket {
                native: self.native.clone(),
                identity: self.identity,
                kind,
            })
    }

    fn cleanup_ack(&mut self, ticket: &CleanupTicket, success: bool) -> Result<()> {
        if !Arc::ptr_eq(&self.native, &ticket.native)
            || self.identity != ticket.identity
            || self.cleanup_ticket(ticket.kind).is_none()
        {
            return Err(Error::Stale);
        }
        self.cleanup_result[ticket.kind as usize] = Some(success);
        if !success {
            if ticket.kind == Cleanup::Audit {
                self.audit_failed = true;
            } else {
                self.cleanup_failed = true;
            }
            return Err(Error::Unconfirmed);
        }
        Ok(())
    }

    /// Passive warning evaluation only: never starts a timer or treats time as
    /// release evidence. Clock errors also revoke, without obstructing ACKs.
    fn tick(&mut self, now: &MonotonicTime) -> Result<()> {
        if let Err(error) = now.checked_duration_since(&self.last_time) {
            self.stop(StopReason::ClockFailure);
            return Err(error.into());
        }
        self.last_time = now.clone();
        if self.release_ticket().is_some() {
            let since = self.warning_since.get_or_insert_with(|| now.clone());
            self.missing_ack_warning = now.checked_duration_since(since)? >= RELEASE_WARNING_AFTER;
        }
        Ok(())
    }

    fn status(&self) -> Status {
        let revoked = self.reason.is_some();
        let (owner, release, listening, stopping, cleaning) = match &self.input {
            None => (
                MicrophoneOwner::Off,
                MicrophoneReleaseStatus::NotRequested,
                ListeningStatus::NotListening,
                false,
                false,
            ),
            Some(input) => match input.phase {
                InputPhase::Acquiring | InputPhase::Capturing => (
                    input.lease.owner,
                    MicrophoneReleaseStatus::NotRequested,
                    if matches!(input.phase, InputPhase::Capturing) {
                        ListeningStatus::Listening
                    } else {
                        ListeningStatus::Unconfirmed
                    },
                    false,
                    false,
                ),
                InputPhase::ReleaseRequested => (
                    input.lease.owner,
                    MicrophoneReleaseStatus::StoppingUnconfirmed,
                    ListeningStatus::Unconfirmed,
                    true,
                    false,
                ),
                InputPhase::Cleaning => (
                    input.lease.owner,
                    MicrophoneReleaseStatus::Acknowledged,
                    ListeningStatus::NotListening,
                    false,
                    true,
                ),
            },
        };
        Status {
            revoked,
            local_cleanup_complete: revoked
                && self.input.is_none()
                && [Cleanup::Output, Cleanup::Workers].iter().all(|kind| {
                    !self.cleanup_requested[*kind as usize]
                        || self.cleanup_result[*kind as usize] == Some(true)
                }),
            owner,
            release,
            listening,
            cancel_input_requested: stopping,
            stop_input_requested: stopping,
            frame_cleanup_requested: cleaning,
            missing_ack_warning: self.missing_ack_warning,
            cleanup_failed: self.cleanup_failed,
            audit_failed: self.audit_failed,
            generation_exhausted: self.generation_exhausted,
        }
    }
}

#[cfg(test)]
#[path = "stop_tests.rs"]
mod tests;
