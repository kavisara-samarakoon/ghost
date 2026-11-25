//! P1-02 / R01–R12: pure, test-only session/turn bookkeeping.
//! No transport, audio, consent issuer, history store, or runtime registration.
//! Guard facts and release evidence below are simulations, never capabilities.
use super::clock::{Deadline, MonotonicTime};
use super::limits::*;
use super::model::*;
use super::ContractError;
use std::sync::Arc;
use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Error {
    Contract(ContractError),
    InvalidTransition,
    OwnershipMismatch,
    OperationMismatch,
    Unauthorized,
    Inadmissible,
    CleanupPending,
    InvalidAcknowledgement,
}
impl From<ContractError> for Error {
    fn from(value: ContractError) -> Self {
        Self::Contract(value)
    }
}
type Result<T> = std::result::Result<T, Error>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SessionEvent {
    Start,
    Ready,
    Pause,
    Resume,
    End,
    Stop,
    Fault,
    Tick,
    CleanupComplete,
}

/// The explicit session matrix. Tick's deadline decision is separate.
fn session_next(state: SessionState, event: SessionEvent) -> Result<SessionState> {
    use SessionEvent as E;
    use SessionState as S;
    match (state, event) {
        (S::Inactive, E::Start) => Ok(S::Starting),
        (S::Starting, E::Ready) | (S::Paused, E::Resume) => Ok(S::Active),
        (S::Active, E::Pause) => Ok(S::Paused),
        (S::Starting, E::Pause | E::End)
        | (S::Active | S::Paused, E::End)
        | (S::Starting | S::Active | S::Paused, E::Stop) => Ok(S::Stopping),
        (S::Starting | S::Active | S::Paused | S::Stopping, E::Fault) => Ok(S::Faulted),
        (S::Stopping, E::CleanupComplete) => Ok(S::Stopped),
        (S::Inactive | S::Stopping | S::Stopped | S::Expired | S::Faulted, E::Stop)
        | (S::Starting | S::Active | S::Paused | S::Stopping, E::Tick) => Ok(state),
        _ => Err(Error::InvalidTransition),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TurnEvent {
    BeginCapture,
    SpeechOnset,
    FinishCapture,
    AudioReady,
    StartStt,
    SttResult,
    RequestConversation,
    ConversationResult,
    ReplyAccepted,
    ActionReview,
    RequestSpeech,
    SpeechStarted,
    SpeechCompleted,
    TextComplete,
    Cancel,
    Fail,
    RejectValidation,
    Recover,
    ReviewDone,
    Tick,
}

fn turn_terminal(state: TurnState) -> bool {
    matches!(state, TurnState::Canceled | TurnState::Failed)
}

/// The explicit turn matrix. Consent, deadlines and completion retirement are
/// guarded refinements; there is no Canceled/Failed -> Idle reset for an identity.
fn turn_next(state: TurnState, event: TurnEvent) -> Result<TurnState> {
    use TurnEvent as E;
    use TurnState as S;
    match (state, event) {
        (S::Idle, E::BeginCapture) => Ok(S::Listening),
        (S::Listening, E::SpeechOnset) => Ok(S::Listening),
        (S::Listening, E::FinishCapture) => Ok(S::Finalizing),
        (S::Finalizing, E::AudioReady) => Ok(S::Finalizing),
        (S::Finalizing, E::StartStt) => Ok(S::Transcribing),
        (S::Transcribing, E::SttResult) => Ok(S::ValidatingTranscript),
        (S::ValidatingTranscript, E::RequestConversation) => Ok(S::RequestingConversation),
        (S::RequestingConversation, E::ConversationResult) => Ok(S::ValidatingReply),
        (S::ValidatingReply, E::ReplyAccepted) => Ok(S::ReplyReady),
        (S::ValidatingTranscript | S::ValidatingReply, E::ActionReview) => Ok(S::NeedsActionReview),
        (S::ReplyReady, E::RequestSpeech) => Ok(S::ReplyReady),
        (S::ReplyReady, E::SpeechStarted) => Ok(S::Speaking),
        (S::Speaking, E::SpeechCompleted) => Ok(S::FollowUpWindow),
        (S::ReplyReady, E::TextComplete) | (S::NeedsActionReview, E::ReviewDone) => Ok(S::Idle),
        (S::AwaitingConsent, E::Recover) => Ok(S::Idle), // restored checkpoint below
        (_, E::Cancel) if !turn_terminal(state) => Ok(S::Canceled),
        (
            S::Listening
            | S::Finalizing
            | S::Transcribing
            | S::RequestingConversation
            | S::ReplyReady
            | S::Speaking,
            E::Fail,
        )
        | (S::ValidatingTranscript | S::ValidatingReply, E::RejectValidation) => Ok(S::Failed),
        (_, E::Tick) if !turn_terminal(state) => Ok(state),
        _ => Err(Error::InvalidTransition),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Gate {
    Allowed,
    MissingConsent,
    Denied,
}

/// Test-only inputs standing for checks performed by future independent gates.
/// No serde, public constructors, consent creation, or sealed provider jobs.
#[derive(Clone)]
struct GuardFacts {
    gate: Gate,
    binding: ConsentBindingId,
    deadline: Deadline,
    privacy_clear: bool,
    provenance_allowed: bool,
    history_allowed: bool,
    budget_allowed: bool,
}
impl GuardFacts {
    fn eligible(&self, now: &MonotonicTime) -> Result<()> {
        if !self.privacy_clear || !self.provenance_allowed || !self.history_allowed {
            return Err(Error::Inadmissible);
        }
        if !self.budget_allowed {
            return Err(ContractError::LimitExceeded.into());
        }
        if self.deadline.is_expired(now)? || self.gate == Gate::Denied {
            return Err(Error::Unauthorized);
        }
        Ok(())
    }
    fn authorized(&self, now: &MonotonicTime) -> Result<()> {
        self.eligible(now)?;
        if self.gate != Gate::Allowed {
            return Err(Error::Unauthorized);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct SessionKey {
    run: AppRunId,
    interaction: InteractionId,
    session: SessionId,
    epoch: Generation,
    configuration: ConfigurationRevision,
    binding: ConsentBindingId,
}

/// Native ownership is separate from the P1-01 serialized description.
#[derive(Clone, Debug)]
struct SessionOwner {
    key: SessionKey,
    instance: Arc<()>,
}
impl SessionOwner {
    fn matches(&self, other: &Self) -> bool {
        self.key == other.key && Arc::ptr_eq(&self.instance, &other.instance)
    }
}

#[derive(Clone, Debug)]
struct TurnOwner {
    session: SessionOwner,
    turn: TurnId,
    generation: Generation,
    binding: ConsentBindingId,
}
impl TurnOwner {
    fn matches(&self, other: &Self) -> bool {
        self.session.matches(&other.session)
            && self.turn == other.turn
            && self.generation == other.generation
            && self.binding == other.binding
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum OperationRole {
    Startup,
    Capture,
    Stt,
    Conversation,
    Speech,
}

#[derive(Clone, Debug)]
struct Operation {
    owner: SessionOwner,
    turn: Option<TurnOwner>,
    role: OperationRole,
    job: JobId,
    stage: TurnState,
    deadline: Deadline,
}
impl Operation {
    fn matches(&self, other: &Self) -> bool {
        self.owner.matches(&other.owner)
            && match (&self.turn, &other.turn) {
                (Some(a), Some(b)) => a.matches(b),
                (None, None) => true,
                _ => false,
            }
            && self.role == other.role
            && self.job == other.job
            && self.stage == other.stage
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Resource {
    Input,
    Output,
}
impl Resource {
    fn index(self) -> usize {
        match self {
            Self::Input => 0,
            Self::Output => 1,
        }
    }
}

/// Exact fake lease/release ticket; its nonce is never reconstructed from metadata.
#[derive(Clone, Debug)]
struct ReleaseTicket {
    owner: TurnOwner,
    lease: JobId,
    resource: Resource,
    nonce: Arc<()>,
}
struct HeldResource {
    ticket: ReleaseTicket,
    release_requested: bool,
}

/// Simulated evidence only. Tests create it; no claim of physical hardware ACK.
struct FakeReleaseEvidence(ReleaseTicket);

#[derive(Default)]
struct Cleanup {
    resources: [Option<HeldResource>; 2],
}
impl Cleanup {
    fn clear(&self) -> bool {
        self.resources.iter().all(Option::is_none)
    }
    fn hold(&mut self, owner: TurnOwner, lease: JobId, resource: Resource) {
        self.resources[resource.index()] = Some(HeldResource {
            ticket: ReleaseTicket {
                owner,
                lease,
                resource,
                nonce: Arc::new(()),
            },
            release_requested: false,
        });
    }
    fn request(&mut self, resource: Resource) {
        if let Some(held) = &mut self.resources[resource.index()] {
            held.release_requested = true;
        }
    }
    fn request_all(&mut self) {
        self.request(Resource::Input);
        self.request(Resource::Output);
    }
    fn acknowledge(&mut self, evidence: &FakeReleaseEvidence) -> Result<()> {
        let ticket = &evidence.0;
        let held = self.resources[ticket.resource.index()]
            .as_ref()
            .ok_or(Error::InvalidAcknowledgement)?;
        if !held.release_requested
            || !held.ticket.owner.matches(&ticket.owner)
            || held.ticket.lease != ticket.lease
            || !Arc::ptr_eq(&held.ticket.nonce, &ticket.nonce)
        {
            return Err(Error::InvalidAcknowledgement);
        }
        self.resources[ticket.resource.index()] = None;
        Ok(())
    }
}

/// Owns the reviewed ledger across child sessions. No reset, Clone or serde.
struct Interaction {
    run: AppRunId,
    id: InteractionId,
    instance: Arc<()>,
    deadline: Deadline,
    budget: InteractionBudget,
    sessions: u64,
    turns: u64,
    jobs: u64,
    admitted_turns: u64,
}
impl Interaction {
    fn new(run: AppRunId, id: InteractionId, now: MonotonicTime) -> Result<Self> {
        Ok(Self {
            run,
            id,
            instance: Arc::new(()),
            deadline: Deadline::after(now, MAX_CLOUD_INTERACTION)?,
            budget: InteractionBudget::new(id),
            sessions: 0,
            turns: 0,
            jobs: 0,
            admitted_turns: 0,
        })
    }
    fn session(
        &mut self,
        configuration: ConfigurationRevision,
        binding: ConsentBindingId,
        grant_deadline: Deadline,
        now: MonotonicTime,
    ) -> Result<Session> {
        // Validate both clock domains before deciding whether allocation is allowed.
        let interaction_expired = self.deadline.is_expired(&now)?;
        let grant_expired = grant_deadline.is_expired(&now)?;
        if interaction_expired || grant_expired {
            return Err(Error::Unauthorized);
        }
        let sequence = self
            .sessions
            .checked_add(1)
            .ok_or(ContractError::Overflow)?;
        let session = SessionId::from_native_sequence(sequence)?;
        self.sessions = sequence;
        Ok(Session {
            owner: SessionOwner {
                key: SessionKey {
                    run: self.run,
                    interaction: self.id,
                    session,
                    epoch: Generation::from_native_sequence(1)?,
                    configuration,
                    binding,
                },
                instance: Arc::new(()),
            },
            interaction_instance: self.instance.clone(),
            state: SessionState::Inactive,
            child_deadline: None,
            interaction_deadline: self.deadline.clone(),
            grant_deadline,
            idle_deadline: Deadline::after(now.clone(), IDLE_CHILD_TIMEOUT)?,
            last_time: now,
            startup: None,
            turn: None,
            cleanup: Cleanup::default(),
            admitted_turns: 0,
            last_admission: false,
            revoked: false,
        })
    }
    fn next_job(&self) -> Result<(u64, JobId)> {
        let sequence = self.jobs.checked_add(1).ok_or(ContractError::Overflow)?;
        Ok((sequence, JobId::from_native_sequence(sequence)?))
    }
}

struct Turn {
    owner: TurnOwner,
    state: TurnState,
    admitted: bool,
    retired: bool,
    audio_bound: Duration,
    audio_retained: bool,
    capture_started: Option<MonotonicTime>,
    speech_onset: bool,
    transcript_bytes: u64,
    reply_bytes: u64,
    checkpoint: Option<TurnState>,
    operation: Option<Operation>,
    stage_deadline: Option<Deadline>,
    retention_deadline: Option<Deadline>,
    speech_requested: bool,
}

struct Session {
    owner: SessionOwner,
    interaction_instance: Arc<()>,
    state: SessionState,
    child_deadline: Option<Deadline>,
    interaction_deadline: Deadline,
    grant_deadline: Deadline,
    idle_deadline: Deadline,
    last_time: MonotonicTime,
    startup: Option<Operation>,
    turn: Option<Turn>,
    cleanup: Cleanup,
    admitted_turns: u64,
    last_admission: bool,
    revoked: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Outcome {
    Applied,
    Expired,
    GenerationExhausted,
}

impl Session {
    fn check_owner(&self, owner: &SessionOwner) -> Result<()> {
        if !self.owner.matches(owner) {
            return Err(Error::OwnershipMismatch);
        }
        Ok(())
    }
    fn check_interaction(&self, interaction: &Interaction) -> Result<()> {
        if self.owner.key.run != interaction.run
            || self.owner.key.interaction != interaction.id
            || !Arc::ptr_eq(&self.interaction_instance, &interaction.instance)
        {
            return Err(Error::OwnershipMismatch);
        }
        Ok(())
    }
    fn check_time(&self, now: &MonotonicTime) -> Result<()> {
        now.checked_duration_since(&self.last_time)?;
        Ok(())
    }
    fn waiting(&self) -> bool {
        self.state == SessionState::Paused
            || self.turn.as_ref().is_none_or(|t| {
                t.retired
                    || matches!(
                        t.state,
                        TurnState::Idle | TurnState::AwaitingConsent | TurnState::NeedsActionReview
                    )
            })
    }
    fn hard_expired(&self, now: &MonotonicTime) -> Result<bool> {
        let child = self
            .child_deadline
            .as_ref()
            .map(|d| d.is_expired(now))
            .transpose()?
            .unwrap_or(false);
        let interaction = self.interaction_deadline.is_expired(now)?;
        let grant = self.grant_deadline.is_expired(now)?;
        Ok(interaction || grant || child)
    }
    fn expire_if_due(&mut self, now: &MonotonicTime) -> Result<Option<Outcome>> {
        self.check_time(now)?;
        // Check every domain even if another deadline has already expired.
        let child = self
            .child_deadline
            .as_ref()
            .map(|d| d.is_expired(now))
            .transpose()?
            .unwrap_or(false);
        let hard = self.interaction_deadline.is_expired(now)?
            || self.grant_deadline.is_expired(now)?
            || child;
        let idle = self.idle_deadline.is_expired(now)? && self.waiting();
        let retention = self
            .turn
            .as_ref()
            .filter(|t| !t.retired)
            .and_then(|t| t.retention_deadline.as_ref())
            .map(|d| d.is_expired(now))
            .transpose()?
            .unwrap_or(false);
        if hard || idle {
            self.invalidate(SessionState::Expired);
            self.last_time = now.clone();
            return Ok(Some(Outcome::Expired));
        }
        if retention {
            return self.retire(TurnState::Canceled, now).map(Some);
        }
        Ok(None)
    }
    /// No clock, provider, persistence or successful generation allocation required.
    fn invalidate(&mut self, target: SessionState) -> Outcome {
        let next = self.owner.key.epoch.checked_next();
        self.revoked = true;
        self.startup = None;
        self.cleanup.request_all();
        if let Some(turn) = &mut self.turn {
            if !turn_terminal(turn.state) && !turn.retired {
                turn.state = TurnState::Canceled;
            }
            turn.retired = true;
            turn.operation = None;
            turn.audio_retained = false;
            turn.transcript_bytes = 0;
            turn.reply_bytes = 0;
            turn.checkpoint = None;
        }
        match next {
            Ok(epoch) => {
                self.owner.key.epoch = epoch;
                self.state = target;
                Outcome::Applied
            }
            Err(_) => {
                self.state = SessionState::Faulted;
                Outcome::GenerationExhausted
            }
        }
    }
    fn stop(&mut self) -> Outcome {
        if matches!(
            self.state,
            SessionState::Inactive
                | SessionState::Stopping
                | SessionState::Stopped
                | SessionState::Expired
                | SessionState::Faulted
        ) {
            self.revoked = true;
            self.cleanup.request_all();
            return Outcome::Applied;
        }
        self.invalidate(SessionState::Stopping)
    }
    fn control(
        &mut self,
        interaction: &mut Interaction,
        owner: &SessionOwner,
        event: SessionEvent,
        facts: &GuardFacts,
        now: &MonotonicTime,
    ) -> Result<Outcome> {
        self.check_owner(owner)?;
        self.check_interaction(interaction)?;
        let next = session_next(self.state, event)?;
        if matches!(
            event,
            SessionEvent::Stop | SessionEvent::End | SessionEvent::Pause | SessionEvent::Fault
        ) {
            return Ok(if event == SessionEvent::Stop {
                self.stop()
            } else {
                self.invalidate(next)
            });
        }
        if event == SessionEvent::CleanupComplete {
            if !self.cleanup.clear() {
                return Err(Error::CleanupPending);
            }
            self.state = next;
            return Ok(Outcome::Applied);
        }
        if event == SessionEvent::Ready {
            return Err(Error::OperationMismatch);
        } // callback only
        self.check_time(now)?;
        if self.state != SessionState::Inactive {
            if let Some(outcome) = self.expire_if_due(now)? {
                return Ok(outcome);
            }
        }
        match event {
            SessionEvent::Start => {
                facts.authorized(now)?;
                if facts.binding != owner.key.binding {
                    return Err(Error::OwnershipMismatch);
                }
                if self.hard_expired(now)? || self.revoked {
                    return Err(Error::Unauthorized);
                }
                if interaction.admitted_turns >= MAX_AGGREGATE_TURNS {
                    return Err(ContractError::LimitExceeded.into());
                }
                let child = Deadline::after(now.clone(), MAX_CHILD_CONVERSATION)?;
                let idle = Deadline::after(now.clone(), IDLE_CHILD_TIMEOUT)?;
                let (sequence, job) = interaction.next_job()?;
                interaction
                    .budget
                    .reserve_child(owner.key.session)?
                    .consume()?;
                interaction.jobs = sequence;
                self.child_deadline = Some(child);
                self.idle_deadline = idle;
                self.startup = Some(Operation {
                    owner: owner.clone(),
                    turn: None,
                    role: OperationRole::Startup,
                    job,
                    stage: TurnState::Idle,
                    deadline: facts.deadline.clone(),
                });
            }
            SessionEvent::Resume => {
                facts.authorized(now)?;
                if facts.binding != owner.key.binding {
                    return Err(Error::OwnershipMismatch);
                }
                if !self.cleanup.clear() {
                    return Err(Error::CleanupPending);
                }
                self.revoked = false;
                self.turn = None; // canceled identity is never reset/reused
            }
            SessionEvent::Tick => {
                return self.tick(now);
            }
            _ => return Err(Error::InvalidTransition),
        }
        self.state = next;
        self.last_time = now.clone();
        Ok(Outcome::Applied)
    }
    fn ready(
        &mut self,
        operation: &Operation,
        facts: &GuardFacts,
        now: &MonotonicTime,
    ) -> Result<Outcome> {
        session_next(self.state, SessionEvent::Ready)?;
        if !self
            .startup
            .as_ref()
            .is_some_and(|op| op.matches(operation))
        {
            return Err(Error::OperationMismatch);
        }
        let deadline = self.startup.as_ref().unwrap().deadline.clone();
        if let Some(outcome) = self.expire_if_due(now)? {
            return Ok(outcome);
        }
        facts.authorized(now)?;
        if facts.binding != self.owner.key.binding || deadline.is_expired(now)? {
            return Err(Error::Unauthorized);
        }
        self.state = SessionState::Active;
        self.startup = None;
        self.last_time = now.clone();
        Ok(Outcome::Applied)
    }
    fn prepare_turn(
        &mut self,
        interaction: &mut Interaction,
        owner: &SessionOwner,
        now: &MonotonicTime,
    ) -> Result<Outcome> {
        self.check_owner(owner)?;
        self.check_interaction(interaction)?;
        if self.state != SessionState::Active || self.revoked {
            return Err(Error::InvalidTransition);
        }
        if self
            .turn
            .as_ref()
            .is_some_and(|t| !t.retired && t.state != TurnState::FollowUpWindow)
        {
            return Err(Error::InvalidTransition);
        }
        if let Some(outcome) = self.expire_if_due(now)? {
            return Ok(outcome);
        }
        if !self.cleanup.clear() {
            return Err(Error::CleanupPending);
        }
        // Follow-up windows are not renewed by installing the next turn identity.
        let inherited = self
            .turn
            .as_ref()
            .filter(|t| t.state == TurnState::FollowUpWindow)
            .and_then(|t| t.stage_deadline.clone());
        if inherited
            .as_ref()
            .map(|d| d.is_expired(now))
            .transpose()?
            .unwrap_or(false)
        {
            return Err(Error::InvalidTransition);
        }
        let sequence = interaction
            .turns
            .checked_add(1)
            .ok_or(ContractError::Overflow)?;
        let turn = TurnId::from_native_sequence(sequence)?;
        interaction.turns = sequence;
        self.turn = Some(Turn {
            owner: TurnOwner {
                session: owner.clone(),
                turn,
                generation: Generation::from_native_sequence(1)?,
                binding: owner.key.binding,
            },
            state: TurnState::Idle,
            admitted: false,
            retired: false,
            audio_bound: Duration::ZERO,
            audio_retained: false,
            transcript_bytes: 0,
            reply_bytes: 0,
            checkpoint: None,
            capture_started: None,
            speech_onset: false,
            operation: None,
            stage_deadline: inherited,
            retention_deadline: None,
            speech_requested: false,
        });
        self.last_time = now.clone();
        Ok(Outcome::Applied)
    }
    fn check_turn(&self, owner: &TurnOwner) -> Result<()> {
        self.check_owner(&owner.session)?;
        let turn = self.turn.as_ref().ok_or(Error::InvalidTransition)?;
        if !turn.owner.matches(owner) {
            return Err(Error::OwnershipMismatch);
        }
        if turn.retired || self.state != SessionState::Active || self.revoked {
            return Err(Error::InvalidTransition);
        }
        Ok(())
    }
    fn retire(&mut self, state: TurnState, now: &MonotonicTime) -> Result<Outcome> {
        let previous = self.turn.as_ref().ok_or(Error::InvalidTransition)?.state;
        let completed = state == TurnState::Idle
            && matches!(previous, TurnState::Speaking | TurnState::ReplyReady);
        let idle = if completed {
            Deadline::after(now.clone(), IDLE_CHILD_TIMEOUT)?
        } else {
            self.idle_deadline.clone()
        };
        let turn = self.turn.as_mut().ok_or(Error::InvalidTransition)?;
        turn.state = state;
        turn.retired = true;
        turn.operation = None;
        turn.audio_retained = false;
        turn.checkpoint = None;
        turn.transcript_bytes = 0;
        // Speech failures may leave an eligible textual reply; no output starts.
        if state != TurnState::Failed {
            turn.reply_bytes = 0;
        }
        self.cleanup.request_all();
        self.idle_deadline = idle;
        self.last_time = now.clone();
        if self.last_admission {
            self.invalidate(SessionState::Expired);
            return Ok(Outcome::Expired);
        }
        Ok(Outcome::Applied)
    }
    fn bounded_retention(&self, deadline: &Deadline, now: &MonotonicTime) -> Result<Deadline> {
        let remaining = deadline.remaining(now)?;
        let prior = self
            .turn
            .as_ref()
            .and_then(|t| t.retention_deadline.as_ref())
            .map(|d| d.remaining(now))
            .transpose()?
            .unwrap_or(remaining);
        Ok(Deadline::after(now.clone(), remaining.min(prior))?)
    }
    fn wait_for_consent(&mut self, checkpoint: TurnState, deadline: Deadline) {
        let turn = self.turn.as_mut().unwrap();
        turn.state = TurnState::AwaitingConsent;
        turn.checkpoint = Some(checkpoint);
        turn.retention_deadline = Some(deadline);
        // Existing idle/child/grant/interaction deadlines remain unchanged.
    }
    fn command(
        &mut self,
        interaction: &mut Interaction,
        owner: &TurnOwner,
        event: TurnEvent,
        facts: &GuardFacts,
        now: &MonotonicTime,
        audio_bound: Duration,
        route: TranscriptionRoute,
    ) -> Result<Outcome> {
        self.check_interaction(interaction)?;
        self.check_turn(owner)?;
        let state = self.turn.as_ref().unwrap().state;
        let next = turn_next(state, event)?;
        if event == TurnEvent::Cancel {
            // Cancellation is deliberately time-independent, including clock errors.
            return self.cancel_turn();
        }
        if matches!(
            event,
            TurnEvent::SpeechOnset
                | TurnEvent::FinishCapture
                | TurnEvent::AudioReady
                | TurnEvent::SttResult
                | TurnEvent::ConversationResult
                | TurnEvent::SpeechStarted
                | TurnEvent::SpeechCompleted
                | TurnEvent::Fail
        ) {
            return Err(Error::OperationMismatch); // matching callbacks only
        }
        if matches!(
            event,
            TurnEvent::BeginCapture
                | TurnEvent::StartStt
                | TurnEvent::RequestConversation
                | TurnEvent::RequestSpeech
        ) && (self.turn.as_ref().unwrap().operation.is_some()
            || (event == TurnEvent::RequestSpeech && self.turn.as_ref().unwrap().speech_requested))
        {
            return Err(Error::InvalidTransition);
        }
        if let Some(outcome) = self.expire_if_due(now)? {
            return Ok(outcome);
        }
        if event == TurnEvent::Tick {
            return self.tick(now);
        }
        if self
            .turn
            .as_ref()
            .unwrap()
            .stage_deadline
            .as_ref()
            .map(|d| d.is_expired(now))
            .transpose()?
            .unwrap_or(false)
        {
            return self.retire(
                if state == TurnState::Idle {
                    TurnState::Canceled
                } else {
                    TurnState::Failed
                },
                now,
            );
        }
        if event == TurnEvent::Recover {
            facts.authorized(now)?;
            let checkpoint = self
                .turn
                .as_ref()
                .unwrap()
                .checkpoint
                .ok_or(Error::InvalidTransition)?;
            let generation = owner.generation.checked_next()?;
            let turn = self.turn.as_mut().unwrap();
            turn.owner.generation = generation;
            turn.owner.binding = facts.binding;
            turn.state = checkpoint;
            turn.checkpoint = None;
            self.last_time = now.clone();
            return Ok(Outcome::Applied); // never creates an operation
        }
        if event == TurnEvent::RejectValidation {
            return self.retire(TurnState::Failed, now);
        }
        if matches!(event, TurnEvent::TextComplete | TurnEvent::ReviewDone) {
            if self.turn.as_ref().unwrap().operation.is_some() {
                return Err(Error::InvalidTransition);
            }
            return self.retire(next, now);
        }
        if event == TurnEvent::ReplyAccepted || event == TurnEvent::ActionReview {
            facts.eligible(now)?;
            if facts.binding != owner.binding {
                return Err(Error::OwnershipMismatch);
            }
            self.turn.as_mut().unwrap().state = next;
            self.last_time = now.clone();
            return Ok(Outcome::Applied);
        }
        if event == TurnEvent::StartStt
            && route == TranscriptionRoute::Cloud
            && (facts.gate != Gate::Allowed || facts.deadline.is_expired(now)?)
        {
            return self.retire(TurnState::Canceled, now);
        }
        facts.eligible(now)?;
        if facts.binding != owner.binding {
            return Err(Error::OwnershipMismatch);
        }
        if facts.gate == Gate::MissingConsent {
            if event == TurnEvent::StartStt {
                // R05: completed audio is not retained while requesting cloud consent.
                return self.retire(TurnState::Canceled, now);
            }
            if matches!(
                event,
                TurnEvent::BeginCapture | TurnEvent::RequestConversation | TurnEvent::RequestSpeech
            ) {
                let deadline = self.bounded_retention(&facts.deadline, now)?;
                self.wait_for_consent(state, deadline);
                self.last_time = now.clone();
                return Ok(Outcome::Applied);
            }
            return Err(Error::Unauthorized);
        }
        if !self.cleanup.clear() {
            return Err(Error::CleanupPending);
        }
        let (sequence, job) = interaction.next_job()?;
        let role = match event {
            TurnEvent::BeginCapture => OperationRole::Capture,
            TurnEvent::StartStt => OperationRole::Stt,
            TurnEvent::RequestConversation => OperationRole::Conversation,
            TurnEvent::RequestSpeech => OperationRole::Speech,
            _ => return Err(Error::InvalidTransition),
        };
        let turn = self.turn.as_ref().unwrap();
        let follow_up_capture = turn.stage_deadline.is_some();
        let retention = self.bounded_retention(&facts.deadline, now)?;
        if turn.operation.is_some() || (event == TurnEvent::RequestSpeech && turn.speech_requested)
        {
            return Err(Error::InvalidTransition);
        }
        let idle = Deadline::after(now.clone(), IDLE_CHILD_TIMEOUT)?;
        let stage = match event {
            TurnEvent::BeginCapture => {
                if audio_bound.is_zero() || audio_bound > MAX_CAPTURED_TURN {
                    return Err(Error::Inadmissible);
                }
                Some(Deadline::after(
                    now.clone(),
                    if turn.stage_deadline.is_some() {
                        audio_bound
                    } else {
                        SPEECH_ONSET_WAIT.min(audio_bound)
                    },
                )?)
            }
            TurnEvent::RequestSpeech => Some(Deadline::after(now.clone(), SPEECH_OUTPUT_DEADLINE)?),
            _ => None,
        };
        // All fallible guards precede the irreversible P1-01 ledger reservation.
        match event {
            TurnEvent::BeginCapture => {
                interaction
                    .budget
                    .reserve_turn(self.owner.key.session, owner.turn, audio_bound)?
                    .consume()?;
            }
            TurnEvent::StartStt => {
                if !turn.audio_retained {
                    return Err(Error::Inadmissible);
                }
                interaction
                    .budget
                    .reserve_transcription(owner.turn, route)?
                    .consume()?;
            }
            TurnEvent::RequestConversation => {
                if turn.transcript_bytes == 0 {
                    return Err(Error::Inadmissible);
                }
                interaction
                    .budget
                    .reserve_provider(owner.turn, ConversationProviderRole::Conversation)?
                    .consume()?;
            }
            TurnEvent::RequestSpeech => {
                if turn.reply_bytes == 0 {
                    return Err(Error::Inadmissible);
                }
                // Capacity/authorization remain external; this only charges the role.
                interaction
                    .budget
                    .reserve_provider(owner.turn, ConversationProviderRole::Tts)?
                    .consume()?;
            }
            _ => unreachable!(),
        }
        interaction.jobs = sequence;
        let turn = self.turn.as_mut().unwrap();
        turn.state = next;
        turn.stage_deadline = stage;
        turn.retention_deadline = Some(retention);
        turn.operation = Some(Operation {
            owner: self.owner.clone(),
            turn: Some(owner.clone()),
            role,
            job,
            stage: next,
            deadline: facts.deadline.clone(),
        });
        if event == TurnEvent::BeginCapture {
            turn.admitted = true;
            turn.audio_bound = audio_bound;
            turn.capture_started = Some(now.clone());
            turn.speech_onset = follow_up_capture;
            self.admitted_turns += 1;
            interaction.admitted_turns += 1;
            self.last_admission = self.admitted_turns == MAX_CHILD_TURNS
                || interaction.admitted_turns == MAX_AGGREGATE_TURNS;
            self.idle_deadline = idle;
            self.cleanup.hold(owner.clone(), job, Resource::Input);
        } else if event == TurnEvent::StartStt {
            turn.audio_retained = false;
        } else if event == TurnEvent::RequestSpeech {
            turn.speech_requested = true;
            self.cleanup.hold(owner.clone(), job, Resource::Output);
        }
        self.last_time = now.clone();
        Ok(Outcome::Applied)
    }
    fn cancel_turn(&mut self) -> Result<Outcome> {
        let turn = self.turn.as_mut().ok_or(Error::InvalidTransition)?;
        if turn.retired || turn_terminal(turn.state) {
            return Err(Error::InvalidTransition);
        }
        let next = turn.owner.generation.checked_next();
        turn.state = TurnState::Canceled;
        turn.retired = true;
        turn.operation = None;
        turn.audio_retained = false;
        turn.transcript_bytes = 0;
        turn.reply_bytes = 0;
        turn.checkpoint = None;
        self.cleanup.request_all();
        if let Ok(generation) = next {
            turn.owner.generation = generation;
        } else {
            self.invalidate(SessionState::Faulted);
            return Ok(Outcome::GenerationExhausted);
        }
        if self.last_admission {
            return Ok(self.invalidate(SessionState::Expired));
        }
        Ok(Outcome::Applied)
    }
    fn callback(
        &mut self,
        operation: &Operation,
        event: TurnEvent,
        now: &MonotonicTime,
        bytes: u64,
        follow_up: bool,
    ) -> Result<Outcome> {
        let owner = operation.turn.as_ref().ok_or(Error::OperationMismatch)?;
        self.check_turn(owner)?;
        let turn = self.turn.as_ref().unwrap();
        let next = turn_next(turn.state, event)?;
        if !turn
            .operation
            .as_ref()
            .is_some_and(|op| op.matches(operation))
        {
            return Err(Error::OperationMismatch);
        }
        let deadline = turn.operation.as_ref().unwrap().deadline.clone();
        if event == TurnEvent::SpeechOnset && turn.speech_onset {
            return Err(Error::InvalidTransition);
        }
        if event == TurnEvent::AudioReady
            && self.cleanup.resources[Resource::Input.index()].is_some()
        {
            return Err(Error::CleanupPending);
        }
        let expected = match event {
            TurnEvent::SpeechOnset | TurnEvent::FinishCapture | TurnEvent::AudioReady => {
                OperationRole::Capture
            }
            TurnEvent::SttResult => OperationRole::Stt,
            TurnEvent::ConversationResult => OperationRole::Conversation,
            TurnEvent::SpeechStarted | TurnEvent::SpeechCompleted => OperationRole::Speech,
            TurnEvent::Fail => operation.role,
            _ => return Err(Error::OperationMismatch),
        };
        if operation.role != expected {
            return Err(Error::OperationMismatch);
        }
        if let Some(outcome) = self.expire_if_due(now)? {
            return Ok(outcome);
        }
        if deadline.is_expired(now)? {
            return self.retire(TurnState::Canceled, now);
        }
        let turn = self.turn.as_ref().unwrap();
        if turn
            .stage_deadline
            .as_ref()
            .map(|d| d.is_expired(now))
            .transpose()?
            .unwrap_or(false)
        {
            if turn.state == TurnState::Listening && !turn.speech_onset {
                return self.retire(TurnState::Canceled, now);
            }
            if turn.state == TurnState::Listening
                && turn.speech_onset
                && event == TurnEvent::FinishCapture
                && now.checked_duration_since(turn.capture_started.as_ref().unwrap())?
                    == turn.audio_bound
            {
                return self.finish_capture(now);
            }
            return self.retire(TurnState::Failed, now);
        }
        if event == TurnEvent::Fail {
            return self.retire(TurnState::Failed, now);
        }
        if event == TurnEvent::SpeechCompleted {
            self.cleanup.request(Resource::Output);
            if !follow_up || self.last_admission {
                return self.retire(TurnState::Idle, now);
            }
            let deadline = Deadline::after(now.clone(), FOLLOW_UP_WINDOW)?;
            let idle = Deadline::after(now.clone(), IDLE_CHILD_TIMEOUT)?;
            let turn = self.turn.as_mut().unwrap();
            turn.state = next;
            turn.operation = None;
            turn.stage_deadline = Some(deadline);
            turn.audio_retained = false;
            self.idle_deadline = idle;
            self.last_time = now.clone();
            return Ok(Outcome::Applied);
        }
        let stage = if event == TurnEvent::FinishCapture {
            Some(Deadline::after(now.clone(), FINALIZATION_TIMEOUT)?)
        } else {
            self.turn.as_ref().unwrap().stage_deadline.clone()
        };
        let maximum = match event {
            TurnEvent::AudioReady => MAX_ENCODED_AUDIO_BYTES,
            TurnEvent::SttResult => MAX_USER_TRANSCRIPT_BYTES,
            TurnEvent::ConversationResult => MAX_ASSISTANT_REPLY_BYTES,
            _ => 0,
        };
        if maximum != 0 && (bytes == 0 || bytes > maximum) {
            return self.retire(TurnState::Failed, now);
        }
        let turn = self.turn.as_mut().unwrap();
        turn.state = next;
        turn.stage_deadline = stage;
        match event {
            TurnEvent::SpeechOnset => {
                turn.stage_deadline = Some(Deadline::after(
                    turn.capture_started.as_ref().unwrap().clone(),
                    turn.audio_bound,
                )?);
                turn.speech_onset = true;
            }
            TurnEvent::FinishCapture => {
                self.cleanup.request(Resource::Input);
                turn.operation.as_mut().unwrap().stage = next;
            }
            TurnEvent::AudioReady => {
                turn.audio_retained = true;
                turn.operation = None;
            }
            TurnEvent::SttResult => {
                turn.transcript_bytes = bytes;
                turn.operation = None;
                turn.stage_deadline = None;
            }
            TurnEvent::ConversationResult => {
                turn.reply_bytes = bytes;
                turn.operation = None;
            }
            TurnEvent::SpeechStarted => {
                turn.operation.as_mut().unwrap().stage = next;
            }
            _ => return Err(Error::OperationMismatch),
        }
        self.last_time = now.clone();
        Ok(Outcome::Applied)
    }
    fn tick(&mut self, now: &MonotonicTime) -> Result<Outcome> {
        session_next(self.state, SessionEvent::Tick)?;
        if let Some(outcome) = self.expire_if_due(now)? {
            return Ok(outcome);
        }
        if self
            .startup
            .as_ref()
            .map(|op| op.deadline.is_expired(now))
            .transpose()?
            .unwrap_or(false)
        {
            let outcome = self.invalidate(SessionState::Faulted);
            self.last_time = now.clone();
            return Ok(outcome);
        } else if let Some(turn) = self.turn.as_ref().filter(|t| !t.retired) {
            let stage = turn
                .stage_deadline
                .as_ref()
                .map(|d| d.is_expired(now))
                .transpose()?
                .unwrap_or(false);
            let authorization = turn
                .operation
                .as_ref()
                .map(|op| op.deadline.is_expired(now))
                .transpose()?
                .unwrap_or(false);
            if authorization {
                return self.retire(TurnState::Canceled, now);
            }
            if stage {
                if turn.state == TurnState::Listening
                    && turn.speech_onset
                    && now.checked_duration_since(turn.capture_started.as_ref().unwrap())?
                        == turn.audio_bound
                {
                    return self.finish_capture(now);
                }
                return self.retire(
                    if turn.state == TurnState::FollowUpWindow {
                        TurnState::Idle
                    } else if (turn.state == TurnState::Listening && !turn.speech_onset)
                        || turn.state == TurnState::Idle
                    {
                        TurnState::Canceled
                    } else {
                        TurnState::Failed
                    },
                    now,
                );
            }
        }
        self.last_time = now.clone();
        Ok(Outcome::Applied)
    }
    fn finish_capture(&mut self, now: &MonotonicTime) -> Result<Outcome> {
        let deadline = Deadline::after(now.clone(), FINALIZATION_TIMEOUT)?;
        let turn = self.turn.as_mut().unwrap();
        turn.state = TurnState::Finalizing;
        turn.stage_deadline = Some(deadline);
        turn.operation.as_mut().unwrap().stage = TurnState::Finalizing;
        self.cleanup.request(Resource::Input);
        self.last_time = now.clone();
        Ok(Outcome::Applied)
    }
}

#[cfg(test)]
#[path = "session_tests.rs"]
mod tests;
