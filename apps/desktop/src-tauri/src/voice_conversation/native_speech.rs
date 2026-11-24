//! P2-01 native output port. The caller owns policy, scheduling and the Phase 1
//! queue. This test-only bridge owns at most one submitted utterance and reports
//! evidence as data. There is no callback into the caller or a follow-up action.
use super::clock::{Deadline, MonotonicClock, MonotonicTime};
use super::limits::{
    checked_total, MAX_AGGREGATE_TURNS, MAX_SPEECH_JOB_TEXT_BYTES, SPEECH_OUTPUT_DEADLINE,
};
use super::model::{AppRunId, ContractIdentity, Generation, JobId, SpeechOutcome};
use super::ContractError;
use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::sync::{Arc, Mutex};

const MAX_VOICES: usize = 512;
const MAX_VOICE_ID_BYTES: usize = 512;
const MAX_VOICE_NAME_BYTES: usize = 256;
const MAX_VOICE_LANGUAGE_BYTES: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Error {
    Contract(ContractError),
    BackendUnavailable,
    Unauthorized,
    NativeFailure,
    InvalidText,
    InvalidVoiceId,
    InvalidVoiceMetadata,
    VoiceUnavailable,
    Busy,
    StaleJob,
    Closed,
    Deadline,
    UnexpectedCallback,
}

impl From<ContractError> for Error {
    fn from(error: ContractError) -> Self {
        Self::Contract(error)
    }
}

pub(super) type Result<T> = std::result::Result<T, Error>;

fn bounded_field(value: &str, max_bytes: usize) -> bool {
    !value.is_empty() && value.len() <= max_bytes && !value.chars().any(char::is_control)
}

/// An exact native string, with no normalization, parsing or fallback.
#[derive(Clone, PartialEq, Eq, Hash)]
pub(super) struct VoiceId(String);

impl VoiceId {
    pub(super) fn new(value: &str) -> Result<Self> {
        if !bounded_field(value, MAX_VOICE_ID_BYTES) {
            return Err(Error::InvalidVoiceId);
        }
        Ok(Self(value.to_owned()))
    }
}

pub(super) struct Voice {
    pub(super) id: VoiceId,
    pub(super) name: String,
    pub(super) language: String,
}

impl Voice {
    fn new(id: VoiceId, name: String, language: String) -> Result<Self> {
        if !bounded_field(&name, MAX_VOICE_NAME_BYTES)
            || !bounded_field(&language, MAX_VOICE_LANGUAGE_BYTES)
        {
            return Err(Error::InvalidVoiceMetadata);
        }
        Ok(Self { id, name, language })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum StopAttempt {
    Accepted,
    Rejected,
    Failed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Evidence {
    Requested,
    Started,
    Finished,
    Cancelled { stop_requested: bool },
    CancellationUnconfirmed(StopAttempt),
    Failure(Error),
}

impl Evidence {
    /// A terminal candidate for the caller's independent identity and policy
    /// checks. This port does not advance the Phase 1 controller.
    pub(super) fn outcome(self) -> Option<SpeechOutcome> {
        match self {
            Self::Finished => Some(SpeechOutcome::NormalCompletion),
            Self::Cancelled {
                stop_requested: true,
            } => Some(SpeechOutcome::Stop),
            Self::Cancelled {
                stop_requested: false,
            }
            | Self::Failure(_) => Some(SpeechOutcome::Failure),
            Self::Requested | Self::Started | Self::CancellationUnconfirmed(_) => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Notice {
    pub(super) identity: ContractIdentity,
    pub(super) evidence: Evidence,
}

/// Native-owned scope. A route revision is a binding supplied by the owner,
/// not evidence that an OS route has been inspected.
#[derive(Clone)]
pub(super) struct OutputScope {
    authority: Arc<()>,
    run: AppRunId,
    epoch: Generation,
    route: Generation,
}

impl OutputScope {
    pub(super) fn new(run: AppRunId, route: Generation) -> Self {
        Self {
            authority: Arc::new(()),
            run,
            epoch: Generation::from_native_sequence(1).expect("nonzero initial epoch"),
            route,
        }
    }

    pub(super) fn next_owner(&self) -> Result<Self> {
        Ok(Self {
            epoch: self.epoch.checked_next()?,
            ..self.clone()
        })
    }

    pub(super) fn bind(&self, identity: ContractIdentity, operation: u64) -> FlightBinding {
        FlightBinding {
            scope: self.clone(),
            identity,
            operation,
            nonce: Arc::new(()),
        }
    }

    fn matches(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.authority, &other.authority)
            && self.run == other.run
            && self.epoch == other.epoch
            && self.route == other.route
    }

    pub(super) fn accepts(&self, binding: &FlightBinding) -> bool {
        self.matches(&binding.scope)
            && self.run == binding.identity.app_run_id
            && binding.operation != 0
    }
}

#[derive(Clone)]
pub(super) struct FlightBinding {
    scope: OutputScope,
    identity: ContractIdentity,
    operation: u64,
    nonce: Arc<()>,
}

impl FlightBinding {
    pub(super) fn identity(&self) -> ContractIdentity {
        self.identity
    }

    pub(super) fn matches(&self, other: &Self) -> bool {
        self.scope.matches(&other.scope)
            && self.identity == other.identity
            && self.operation == other.operation
            && Arc::ptr_eq(&self.nonce, &other.nonce)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Submission {
    NeverSubmitted,
    PossiblySubmitted,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum StopOrigin {
    User,
    CoordinatorFailure,
    SubmissionFailure,
    Deadline,
    InvalidCallback,
    Shutdown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Terminal {
    Finished,
    Cancelled { stop_requested: bool },
    Unresolved,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Attachment {
    NeverAttempted,
    MayHaveAttached,
    Attached,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Detach {
    NotNeeded,
    VerifiedNil,
    ReadbackUnknown,
    SetterFailed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum RetirementUncertainty {
    NoSubmissionOrDelegate,
    DelegateAssociationUnresolved,
    OutputReleaseUnverified,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum CallbackDisposition {
    NoDelegateAttempted,
    CallbackMayStillBeReturning,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ResourceRetirement {
    OwnedFlightRelinquished,
}

/// Diagnostic facts. Only the opaque report carries their originating flight.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct RetirementFacts {
    pub(super) submission: Submission,
    pub(super) first_stop: Option<(StopOrigin, StopAttempt)>,
    pub(super) terminal: Terminal,
    pub(super) attachment: Attachment,
    pub(super) detach: Detach,
    pub(super) callbacks: CallbackDisposition,
    pub(super) resources: ResourceRetirement,
    pub(super) uncertainty: RetirementUncertainty,
}

/// Issued only after the bridge has relinquished its flight resources.
/// Terminal and delegate facts describe different events, and neither
/// establishes physical output release. A callback already inside the
/// delegate may still return after nil readback; it sees a closed/old mailbox.
pub(super) struct RetirementReport {
    binding: FlightBinding,
    facts: RetirementFacts,
}

impl RetirementReport {
    pub(super) fn matches(&self, binding: &FlightBinding) -> bool {
        self.binding.matches(binding)
    }

    pub(super) fn facts(&self) -> RetirementFacts {
        self.facts
    }

    /// The report itself is the bridge-owned retirement event. This narrow
    /// result can abandon only a preparation that never attempted speech.
    pub(super) fn can_abandon_unsubmitted(&self) -> bool {
        self.facts().uncertainty == RetirementUncertainty::NoSubmissionOrDelegate
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct NativeObjects {
    synthesizer: usize,
    utterance: usize,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct CallbackKey {
    identity: ContractIdentity,
    objects: NativeObjects,
}

#[derive(Clone, Copy)]
enum Callback {
    Start,
    Finish,
    Cancel,
}

#[derive(Clone, Copy)]
enum Observation {
    Unsubmitted,
    Waiting,
    Started,
    Finished,
    Cancelled { stop_requested: bool },
    Invalid,
    Closed,
}

struct CallbackState {
    observation: Observation,
    start_pending: bool,
    stopping: bool,
    terminal: Terminal,
}

/// One mailbox per utterance. No native references are transported across
/// threads. Native object addresses are compared as opaque values, never dereferenced.
struct Inbox {
    key: CallbackKey,
    state: Mutex<CallbackState>,
}

impl Inbox {
    fn new(key: CallbackKey) -> Self {
        Self {
            key,
            state: Mutex::new(CallbackState {
                observation: Observation::Unsubmitted,
                start_pending: false,
                stopping: false,
                terminal: Terminal::Unresolved,
            }),
        }
    }

    fn arm(&self) -> Result<()> {
        let mut state = self.state.try_lock().map_err(|_| Error::NativeFailure)?;
        state.observation = Observation::Waiting;
        Ok(())
    }

    fn observe(&self, key: CallbackKey, callback: Callback) {
        if key != self.key {
            return;
        }
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        state.observation = match (state.observation, callback, state.stopping) {
            (Observation::Waiting, Callback::Start, false) => {
                state.start_pending = true;
                Observation::Started
            }
            (Observation::Started, Callback::Finish, false) => Observation::Finished,
            (Observation::Started, Callback::Finish, true) => {
                // Preserve a matching terminal fact without restoring a
                // logical Finished notice after Stop.
                if state.terminal == Terminal::Unresolved {
                    state.terminal = Terminal::Finished;
                }
                Observation::Started
            }
            (Observation::Waiting, Callback::Finish, false) => Observation::Invalid,
            (Observation::Waiting | Observation::Started, Callback::Cancel, stopping) => {
                Observation::Cancelled {
                    stop_requested: stopping,
                }
            }
            (current, _, _) => current,
        };
        if state.terminal == Terminal::Unresolved {
            state.terminal = match state.observation {
                Observation::Finished => Terminal::Finished,
                Observation::Cancelled { stop_requested } => Terminal::Cancelled { stop_requested },
                _ => Terminal::Unresolved,
            };
        }
    }

    fn begin_stop(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.stopping = true;
            state.start_pending = false;
            if !matches!(
                state.observation,
                Observation::Started | Observation::Cancelled { .. }
            ) {
                state.observation = Observation::Waiting;
            }
        }
    }

    fn take_evidence(&self) -> Option<Evidence> {
        let Ok(mut state) = self.state.lock() else {
            return Some(Evidence::Failure(Error::NativeFailure));
        };
        if std::mem::take(&mut state.start_pending) {
            return Some(Evidence::Started);
        }
        let evidence = match state.observation {
            Observation::Finished => Evidence::Finished,
            Observation::Cancelled { stop_requested } => Evidence::Cancelled { stop_requested },
            Observation::Invalid => return Some(Evidence::Failure(Error::UnexpectedCallback)),
            _ => return None,
        };
        state.observation = Observation::Closed;
        Some(evidence)
    }

    fn close(&self) -> Terminal {
        if let Ok(mut state) = self.state.lock() {
            state.start_pending = false;
            state.observation = Observation::Closed;
            state.terminal
        } else {
            Terminal::Unresolved
        }
    }
}

trait Backend: Sized {
    type Voice;
    type Utterance;

    fn find_voice(&self, id: &VoiceId) -> Result<Option<(VoiceId, Self::Voice)>>;
    fn prepare(&self, voice: Self::Voice, text: &str) -> Result<(Self::Utterance, NativeObjects)>;
    fn attach(&mut self, inbox: Arc<Inbox>) -> Result<()>;
    fn speak(
        &self,
        utterance: &Self::Utterance,
        before_dispatch: impl FnOnce() -> Result<()>,
    ) -> Result<()>;
    fn stop_immediately(&self) -> Result<bool>;
    fn detach(&mut self) -> Detach;
}

fn validate_inventory(voices: Vec<Voice>) -> Result<Vec<Voice>> {
    if voices.len() > MAX_VOICES {
        return Err(Error::InvalidVoiceMetadata);
    }
    let mut ids = HashSet::with_capacity(voices.len());
    for voice in &voices {
        if !ids.insert(&voice.id) {
            return Err(Error::InvalidVoiceMetadata);
        }
    }
    Ok(voices)
}

/// Independent synthetic authority, with no provider or callback interface.
#[derive(Clone)]
pub(super) struct LocalSpeechGrant {
    pub(super) identity: ContractIdentity,
    pub(super) deadline: Deadline,
    pub(super) speaker: bool,
    pub(super) source: bool,
}

#[derive(Default)]
pub(super) struct LocalAuthorityFacts {
    pub(super) scope: Option<ContractIdentity>,
    pub(super) grants: [Option<LocalSpeechGrant>; MAX_AGGREGATE_TURNS as usize],
}

pub(super) type LocalAuthorityState = RefCell<LocalAuthorityFacts>;

pub(super) fn same_session(scope: ContractIdentity, identity: ContractIdentity) -> bool {
    scope
        == (ContractIdentity {
            turn_id: scope.turn_id,
            job_id: scope.job_id,
            ..identity
        })
}

/// Stack-bound and consumed once. The managed owner supplies the independent
/// outstanding obligation. The coordinator supplies the trusted local state.
pub(super) struct SubmissionCheck<'a> {
    expected: Option<&'a FlightBinding>,
    authority: &'a LocalAuthorityState,
    revoked: &'a Cell<bool>,
    validated_at: &'a MonotonicTime,
    deadlines: [&'a Deadline; 2],
    obligation: Option<&'a RefCell<Option<FlightBinding>>>,
}

impl<'a> SubmissionCheck<'a> {
    pub(super) fn new(
        expected: &'a FlightBinding,
        authority: &'a LocalAuthorityState,
        revoked: &'a Cell<bool>,
        validated_at: &'a MonotonicTime,
        deadlines: [&'a Deadline; 2],
    ) -> Self {
        Self {
            expected: Some(expected),
            authority,
            revoked,
            validated_at,
            deadlines,
            obligation: None,
        }
    }

    pub(super) fn with_obligation(
        mut self,
        obligation: &'a RefCell<Option<FlightBinding>>,
    ) -> Self {
        self.obligation = Some(obligation);
        self
    }

    pub(super) fn check(&mut self, flight: &FlightBinding, now: &MonotonicTime) -> Result<()> {
        let expected = self.expected.take().ok_or(Error::Unauthorized)?;
        let obligation = self.obligation.ok_or(Error::Unauthorized)?;
        if !expected.matches(flight)
            || !obligation
                .try_borrow()
                .map_err(|_| Error::Busy)?
                .as_ref()
                .is_some_and(|pending| pending.matches(flight))
        {
            return Err(Error::Unauthorized);
        }
        now.checked_duration_since(self.validated_at)?;
        let authority = self.authority.try_borrow().map_err(|_| Error::Busy)?;
        if !authority
            .scope
            .is_some_and(|scope| same_session(scope, flight.identity()))
        {
            return Err(Error::Unauthorized);
        }
        let mut matching = authority
            .grants
            .iter()
            .flatten()
            .filter(|grant| grant.identity == flight.identity());
        let grant = matching.next().ok_or(Error::Unauthorized)?;
        if matching.next().is_some() || !grant.speaker || !grant.source {
            return Err(Error::Unauthorized);
        }
        for deadline in self.deadlines.into_iter().chain([&grant.deadline]) {
            if deadline.is_expired(now)? {
                return Err(Error::Deadline);
            }
        }
        if self.revoked.get() {
            return Err(Error::Unauthorized);
        }
        Ok(())
    }
}

pub(super) trait SpeechOutput {
    fn request(
        &mut self,
        binding: FlightBinding,
        sequence: u64,
        text: &str,
        voice: &VoiceId,
        admitted: &MonotonicTime,
        check: SubmissionCheck<'_>,
    ) -> Result<Notice>;
    fn stop(&mut self, identity: ContractIdentity, origin: StopOrigin) -> Result<Notice>;
    fn poll(&mut self) -> Option<Notice>;
    fn shutdown(&mut self) -> Option<Notice>;
}

pub(super) trait RetirementOutput: SpeechOutput {
    fn begin_retirement(&mut self) -> Option<Notice>;
    fn take_retirement_report(&mut self) -> Option<RetirementReport>;
}

struct Flight<U> {
    binding: FlightBinding,
    utterance: U,
    inbox: Arc<Inbox>,
    deadline: Deadline,
    first_stop: Option<(StopOrigin, StopAttempt)>,
    submission: Submission,
}

impl<U> Flight<U> {
    fn notice(&self, evidence: Evidence) -> Notice {
        Notice {
            identity: self.inbox.key.identity,
            evidence,
        }
    }
}

/// One slot for actual output. The caller owns any pending job and must supply
/// its original Phase 1 admission time so preparation does not reset its deadline.
struct Port<B: Backend, C: MonotonicClock> {
    backend: B,
    clock: C,
    scope: OutputScope,
    last_sequence: u64,
    active: Option<Flight<B::Utterance>>,
    closed: bool,
    retired: Option<RetirementReport>,
}

impl<B: Backend, C: MonotonicClock> Port<B, C> {
    fn new(backend: Result<B>, clock: C, scope: OutputScope) -> Result<Self> {
        Ok(Self {
            backend: backend?,
            clock,
            scope,
            last_sequence: 0,
            active: None,
            closed: false,
            retired: None,
        })
    }

    fn request_stop(backend: &B, flight: &mut Flight<B::Utterance>, origin: StopOrigin) -> Notice {
        let (_, attempt) = *flight.first_stop.get_or_insert_with(|| {
            flight.inbox.begin_stop();
            let attempt = match backend.stop_immediately() {
                Ok(true) => StopAttempt::Accepted,
                Ok(false) => StopAttempt::Rejected,
                Err(_) => StopAttempt::Failed,
            };
            (origin, attempt)
        });
        flight.notice(Evidence::CancellationUnconfirmed(attempt))
    }

    fn clear_delegate(&mut self, inbox: &Inbox) -> (Terminal, Detach) {
        let terminal = inbox.close();
        let detach = self.backend.detach();
        if detach != Detach::VerifiedNil {
            self.closed = true;
        }
        (terminal, detach)
    }

    fn retire(&mut self) {
        if let Some(flight) = self.active.take() {
            let (terminal, detach) = self.clear_delegate(&flight.inbox);
            let binding = flight.binding.clone();
            let first_stop = flight.first_stop;
            drop(flight);
            self.retired = Some(RetirementReport {
                binding,
                facts: RetirementFacts {
                    submission: Submission::PossiblySubmitted,
                    first_stop,
                    terminal,
                    attachment: Attachment::Attached,
                    detach,
                    callbacks: CallbackDisposition::CallbackMayStillBeReturning,
                    resources: ResourceRetirement::OwnedFlightRelinquished,
                    uncertainty: RetirementUncertainty::OutputReleaseUnverified,
                },
            });
        }
    }

    fn retire_unsubmitted(&mut self, error: Error) -> Result<Notice> {
        let flight = self.active.take().expect("prepared flight");
        let (_, detach) = self.clear_delegate(&flight.inbox);
        let binding = flight.binding.clone();
        drop(flight);
        self.unsubmitted(binding, Attachment::Attached, detach, error)
    }

    fn unsubmitted(
        &mut self,
        binding: FlightBinding,
        attachment: Attachment,
        detach: Detach,
        error: Error,
    ) -> Result<Notice> {
        self.retired = Some(RetirementReport {
            binding,
            facts: RetirementFacts {
                submission: Submission::NeverSubmitted,
                first_stop: None,
                terminal: Terminal::Unresolved,
                attachment,
                detach,
                callbacks: match attachment {
                    Attachment::NeverAttempted => CallbackDisposition::NoDelegateAttempted,
                    _ => CallbackDisposition::CallbackMayStillBeReturning,
                },
                resources: ResourceRetirement::OwnedFlightRelinquished,
                uncertainty: match detach {
                    Detach::NotNeeded | Detach::VerifiedNil => {
                        RetirementUncertainty::NoSubmissionOrDelegate
                    }
                    Detach::ReadbackUnknown | Detach::SetterFailed => {
                        RetirementUncertainty::DelegateAssociationUnresolved
                    }
                },
            },
        });
        Err(error)
    }

    fn prepare(
        &mut self,
        sequence: u64,
        text: &str,
        voice: &VoiceId,
        admitted: &MonotonicTime,
    ) -> Result<(B::Utterance, NativeObjects, Deadline, MonotonicTime)> {
        let bytes = u64::try_from(text.len()).map_err(|_| ContractError::Overflow)?;
        checked_total(0, bytes, MAX_SPEECH_JOB_TEXT_BYTES)?;
        if text.trim().is_empty() || text.contains('\0') {
            return Err(Error::InvalidText);
        }
        let now = self.clock.now();
        now.checked_duration_since(admitted)?;
        let deadline = Deadline::after(admitted.clone(), SPEECH_OUTPUT_DEADLINE)?;
        if deadline.is_expired(&now)? {
            return Err(Error::Deadline);
        }
        self.last_sequence = sequence;
        let (resolved_id, resolved_voice) = self
            .backend
            .find_voice(voice)?
            .ok_or(Error::VoiceUnavailable)?;
        if &resolved_id != voice {
            return Err(Error::VoiceUnavailable);
        }
        let (utterance, objects) = self.backend.prepare(resolved_voice, text)?;
        if objects.synthesizer == 0 || objects.utterance == 0 {
            return Err(Error::NativeFailure);
        }
        Ok((utterance, objects, deadline, now))
    }
}

impl<B: Backend, C: MonotonicClock> SpeechOutput for Port<B, C> {
    fn request(
        &mut self,
        binding: FlightBinding,
        sequence: u64,
        text: &str,
        voice: &VoiceId,
        admitted: &MonotonicTime,
        mut check: SubmissionCheck<'_>,
    ) -> Result<Notice> {
        if self.closed {
            return Err(Error::Closed);
        }
        if self.active.is_some() {
            return Err(Error::Busy);
        }
        let identity = binding.identity();
        if !self.scope.accepts(&binding)
            || sequence <= self.last_sequence
            || JobId::from_native_sequence(sequence)? != identity.job_id
        {
            return Err(Error::StaleJob);
        }
        if self.retired.is_some() {
            return Err(Error::Busy);
        }

        let (utterance, objects, deadline, prepared_since) =
            match self.prepare(sequence, text, voice, admitted) {
                Ok(prepared) => prepared,
                Err(error) => {
                    return self.unsubmitted(
                        binding,
                        Attachment::NeverAttempted,
                        Detach::NotNeeded,
                        error,
                    )
                }
            };
        let inbox = Arc::new(Inbox::new(CallbackKey { identity, objects }));
        if let Err(error) = self.backend.attach(Arc::clone(&inbox)) {
            let (_, detach) = self.clear_delegate(&inbox);
            drop(utterance);
            drop(inbox);
            return self.unsubmitted(binding, Attachment::MayHaveAttached, detach, error);
        }
        self.active = Some(Flight {
            binding,
            utterance,
            inbox,
            deadline,
            first_stop: None,
            submission: Submission::NeverSubmitted,
        });
        let flight = self.active.as_mut().expect("just installed the flight");
        let result = self.backend.speak(&flight.utterance, || {
            let now = self.clock.now();
            now.checked_duration_since(&prepared_since)?;
            if flight.deadline.is_expired(&now)? {
                return Err(Error::Deadline);
            }
            check.check(&flight.binding, &now)?;
            flight.inbox.arm()?;
            // The backend must dispatch immediately after this marker.
            flight.submission = Submission::PossiblySubmitted;
            Ok(())
        });
        if flight.submission == Submission::NeverSubmitted {
            return self.retire_unsubmitted(result.err().unwrap_or(Error::NativeFailure));
        }
        if let Err(error) = result {
            Self::request_stop(&self.backend, flight, StopOrigin::SubmissionFailure);
            return Ok(flight.notice(Evidence::Failure(error)));
        }
        Ok(flight.notice(Evidence::Requested))
    }

    fn stop(&mut self, identity: ContractIdentity, origin: StopOrigin) -> Result<Notice> {
        let flight = self.active.as_mut().ok_or(Error::StaleJob)?;
        if flight.binding.identity() != identity || flight.submission == Submission::NeverSubmitted
        {
            return Err(Error::StaleJob);
        }
        Ok(Self::request_stop(&self.backend, flight, origin))
    }

    fn poll(&mut self) -> Option<Notice> {
        let flight = self.active.as_mut()?;
        if flight.submission == Submission::NeverSubmitted {
            return None;
        }
        if flight.first_stop.is_none()
            && flight
                .deadline
                .is_expired(&self.clock.now())
                .unwrap_or(true)
        {
            Self::request_stop(&self.backend, flight, StopOrigin::Deadline);
            return Some(flight.notice(Evidence::Failure(Error::Deadline)));
        }
        let evidence = flight.inbox.take_evidence()?;
        let notice = flight.notice(evidence);
        match evidence {
            Evidence::Finished | Evidence::Cancelled { .. } => self.retire(),
            Evidence::Failure(_) if flight.first_stop.is_none() => {
                Self::request_stop(&self.backend, flight, StopOrigin::InvalidCallback);
            }
            Evidence::Failure(_) => return None,
            _ => {}
        }
        Some(notice)
    }

    fn shutdown(&mut self) -> Option<Notice> {
        let unconfirmed = self.begin_retirement()?;
        let flight = self.active.as_ref().expect("retirement requires a flight");
        let notice = match flight.inbox.take_evidence() {
            Some(evidence @ Evidence::Cancelled { .. }) => flight.notice(evidence),
            _ => unconfirmed,
        };
        self.retire();
        Some(notice)
    }
}

impl<B: Backend, C: MonotonicClock> RetirementOutput for Port<B, C> {
    fn begin_retirement(&mut self) -> Option<Notice> {
        self.closed = true;
        let flight = self.active.as_mut()?;
        if flight.submission == Submission::NeverSubmitted {
            let _ = self.retire_unsubmitted(Error::NativeFailure);
            return None;
        }
        Some(Self::request_stop(
            &self.backend,
            flight,
            StopOrigin::Shutdown,
        ))
    }

    fn take_retirement_report(&mut self) -> Option<RetirementReport> {
        self.retired.take()
    }
}

impl<B: Backend, C: MonotonicClock> Drop for Port<B, C> {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}

#[cfg(target_os = "macos")]
#[allow(unused_imports)] // The isolated port has no runtime caller in P2-01.
pub(super) use macos::{installed_voices, open};

#[cfg(target_os = "macos")]
#[allow(dead_code)] // Compiled in tests. No automated test instantiates AVFoundation.
mod macos {
    use super::*;
    use objc2::rc::{autoreleasepool, Retained};
    use objc2::runtime::{AnyClass, AnyObject, AnyProtocol, Bool, NSObject, NSObjectProtocol, Sel};
    use objc2::{
        define_class, extern_protocol, msg_send, sel, AnyThread, DefinedClass, MainThreadMarker,
    };
    use objc2_foundation::{NSArray, NSString, NSUTF8StringEncoding};
    use std::panic::{catch_unwind, AssertUnwindSafe};
    use std::ptr::NonNull;

    #[link(name = "AVFoundation", kind = "framework")]
    extern "C" {}

    // SAFETY: The runtime protocol has NSObject as its only superprotocol.
    // Its optional delegate selectors are implemented with their SDK signatures below.
    extern_protocol!(
        unsafe trait AVSpeechSynthesizerDelegate: NSObjectProtocol {}
    );

    define_class!(
        // SAFETY: NSObject permits subclassing. Immutable ivars hold only an
        // Arc to a synchronized Rust mailbox and are initialized before init.
        #[unsafe(super(NSObject))]
        #[name = "GhostM45BIsolatedSpeechDelegate"]
        #[ivars = Arc<Inbox>]
        struct Delegate;

        // SAFETY: Delegate inherits NSObject's protocol implementations.
        unsafe impl NSObjectProtocol for Delegate {}

        // SAFETY: Delegate implements the three optional methods below; each
        // takes two Objective-C object pointers and returns void.
        unsafe impl AVSpeechSynthesizerDelegate for Delegate {}

        impl Delegate {
            #[unsafe(method(speechSynthesizer:didStartSpeechUtterance:))]
            fn started(&self, synthesizer: *mut AnyObject, utterance: *mut AnyObject) {
                self.record(synthesizer, utterance, Callback::Start);
            }

            #[unsafe(method(speechSynthesizer:didFinishSpeechUtterance:))]
            fn finished(&self, synthesizer: *mut AnyObject, utterance: *mut AnyObject) {
                self.record(synthesizer, utterance, Callback::Finish);
            }

            #[unsafe(method(speechSynthesizer:didCancelSpeechUtterance:))]
            fn cancelled(&self, synthesizer: *mut AnyObject, utterance: *mut AnyObject) {
                self.record(synthesizer, utterance, Callback::Cancel);
            }
        }
    );

    impl Delegate {
        fn record(&self, synthesizer: *mut AnyObject, utterance: *mut AnyObject, event: Callback) {
            if let Err(payload) = catch_unwind(AssertUnwindSafe(|| {
                let inbox = self.ivars();
                let key = CallbackKey {
                    identity: inbox.key.identity,
                    objects: NativeObjects {
                        synthesizer: synthesizer as usize,
                        utterance: utterance as usize,
                    },
                };
                inbox.observe(key, event);
            })) {
                // Even a panic payload with a panicking destructor must stay
                // on this side of the Objective-C callback boundary.
                std::mem::forget(payload);
            }
        }
    }

    struct MacBackend {
        _main_thread: MainThreadMarker,
        synthesizer: Retained<AnyObject>,
        utterance_class: &'static AnyClass,
        voice_class: &'static AnyClass,
        // The framework's delegate property is weak.
        delegate: Option<Retained<Delegate>>,
    }

    struct MacUtterance {
        object: Retained<AnyObject>,
        _voice: Retained<AnyObject>,
    }

    impl MacBackend {
        // Owner entry points and destruction stay on the macOS main thread
        // with its normal run loop. There is no wait for callback delivery.
        fn new(main_thread: MainThreadMarker) -> Result<Self> {
            native_call(|| {
                let synth_class =
                    AnyClass::get(c"AVSpeechSynthesizer").ok_or(Error::BackendUnavailable)?;
                let utterance_class =
                    AnyClass::get(c"AVSpeechUtterance").ok_or(Error::BackendUnavailable)?;
                let voice_class =
                    AnyClass::get(c"AVSpeechSynthesisVoice").ok_or(Error::BackendUnavailable)?;
                AnyProtocol::get(c"AVSpeechSynthesizerDelegate")
                    .ok_or(Error::BackendUnavailable)?;
                // SAFETY: The verified AVSpeechSynthesizer class inherits
                // NSObject's +new. The new family transfers +1 ownership.
                let synthesizer = unsafe { msg_send![synth_class, new] };
                Ok(Self {
                    _main_thread: main_thread,
                    synthesizer: checked_object(synthesizer, synth_class)?,
                    utterance_class,
                    voice_class,
                    delegate: None,
                })
            })
        }
    }

    pub(in crate::voice_conversation) fn open(
        main_thread: MainThreadMarker,
        clock: impl MonotonicClock,
        scope: OutputScope,
    ) -> Result<impl RetirementOutput> {
        Port::new(MacBackend::new(main_thread), clock, scope)
    }

    // Wry already enables objc2's exception API in the locked macOS build.
    // Keep foreign exceptions in that existing handler and discard their data.
    fn native_call<T>(call: impl FnOnce() -> Result<T>) -> Result<T> {
        autoreleasepool(|_| {
            objc2::exception::catch(AssertUnwindSafe(call)).map_err(|_| Error::NativeFailure)?
        })
    }

    fn checked_object(
        object: Option<Retained<AnyObject>>,
        expected: &AnyClass,
    ) -> Result<Retained<AnyObject>> {
        let object = object.ok_or(Error::NativeFailure)?;
        let mut class = Some(object.class());
        while let Some(current) = class {
            if std::ptr::eq(current, expected) {
                return Ok(object);
            }
            class = current.superclass();
        }
        Err(Error::NativeFailure)
    }

    fn native_string(value: &str) -> Result<Retained<NSString>> {
        // SAFETY: initWithBytes:length:encoding: copies exactly the live UTF-8
        // slice, takes NSUInteger/NSStringEncoding and returns an owned NSString
        // or nil. msg_send transfers the NSString allocation to init.
        let string: Option<Retained<NSString>> = unsafe {
            msg_send![NSString::alloc(), initWithBytes: value.as_ptr().cast::<std::ffi::c_void>(),
                length: value.len(), encoding: NSUTF8StringEncoding]
        };
        string.ok_or(Error::NativeFailure)
    }

    fn read_string(object: Option<Retained<AnyObject>>, max: usize) -> Result<String> {
        let string = object
            .and_then(|object| object.downcast::<NSString>().ok())
            .ok_or(Error::InvalidVoiceMetadata)?;
        let bytes = string.len();
        if bytes == 0 || bytes > max {
            return Err(Error::InvalidVoiceMetadata);
        }
        let mut buffer = [0_u8; MAX_VOICE_ID_BYTES + 1];
        let pointer = NonNull::from(&mut buffer[0]).cast();
        // SAFETY: All callers' maxima fit this stack allocation. Foundation
        // writes at most bytes + 1 bytes to the buffer during this call.
        if !unsafe {
            string.getCString_maxLength_encoding(pointer, bytes + 1, NSUTF8StringEncoding)
        } {
            return Err(Error::InvalidVoiceMetadata);
        }
        let value =
            std::str::from_utf8(&buffer[..bytes]).map_err(|_| Error::InvalidVoiceMetadata)?;
        if !bounded_field(value, max) {
            return Err(Error::InvalidVoiceMetadata);
        }
        Ok(value.to_owned())
    }

    fn voice_id(voice: &AnyObject) -> Result<VoiceId> {
        // SAFETY: The caller has verified the AVSpeechSynthesisVoice class.
        // The identifier getter returns an autoreleased/borrowed NSString (+0).
        let id = unsafe { msg_send![voice, identifier] };
        read_string(id, MAX_VOICE_ID_BYTES).map(VoiceId)
    }

    /// This can run off the speech owner thread. It only uses the SDK's sendable
    /// voice objects within the calling thread and returns bounded Rust data.
    pub(in crate::voice_conversation) fn installed_voices() -> Result<Vec<Voice>> {
        native_call(|| {
            let voice_class =
                AnyClass::get(c"AVSpeechSynthesisVoice").ok_or(Error::BackendUnavailable)?;
            // SAFETY: +speechVoices on the verified class returns an NSArray
            // at +0. Check both the array and element types before use.
            let array: Option<Retained<AnyObject>> =
                unsafe { msg_send![voice_class, speechVoices] };
            let array = array
                .and_then(|object| object.downcast::<NSArray>().ok())
                .ok_or(Error::NativeFailure)?;
            let count = array.count();
            if count > MAX_VOICES {
                return Err(Error::InvalidVoiceMetadata);
            }
            let mut voices = Vec::with_capacity(count);
            for index in 0..count {
                let voice = checked_object(Some(array.objectAtIndex(index)), voice_class)?;
                // SAFETY: Both checked voice getters return NSString pointers
                // at +0. Rust retains them until bounded conversion completes.
                let (name, language) =
                    unsafe { (msg_send![&*voice, name], msg_send![&*voice, language]) };
                voices.push(Voice::new(
                    voice_id(&voice)?,
                    read_string(name, MAX_VOICE_NAME_BYTES)?,
                    read_string(language, MAX_VOICE_LANGUAGE_BYTES)?,
                )?);
            }
            validate_inventory(voices)
        })
    }

    impl Backend for MacBackend {
        type Voice = Retained<AnyObject>;
        type Utterance = MacUtterance;

        fn find_voice(&self, id: &VoiceId) -> Result<Option<(VoiceId, Self::Voice)>> {
            native_call(|| {
                let identifier = native_string(&id.0)?;
                // SAFETY: The verified voice class's factory takes a live NSString
                // and returns a nullable voice at +0; nil means unavailable.
                let voice: Option<Retained<AnyObject>> =
                    unsafe { msg_send![self.voice_class, voiceWithIdentifier: &*identifier] };
                voice
                    .map(|voice| {
                        let voice = checked_object(Some(voice), self.voice_class)?;
                        Ok((voice_id(&voice)?, voice))
                    })
                    .transpose()
            })
        }

        fn prepare(
            &self,
            voice: Self::Voice,
            text: &str,
        ) -> Result<(Self::Utterance, NativeObjects)> {
            native_call(|| {
                let text = native_string(text)?;
                // SAFETY: The verified utterance class's plain-text factory
                // takes a live NSString and returns an utterance at +0.
                let utterance =
                    unsafe { msg_send![self.utterance_class, speechUtteranceWithString: &*text] };
                let object = checked_object(utterance, self.utterance_class)?;
                if object
                    .class()
                    .instance_method(sel!(setPrefersAssistiveTechnologySettings:))
                    .is_some()
                {
                    // SAFETY: The SDK's macOS 11+ setter was found on the verified
                    // utterance. A false BOOL preserves explicit voice selection.
                    unsafe {
                        let _: () =
                            msg_send![&*object, setPrefersAssistiveTechnologySettings: Bool::NO];
                    }
                }

                // SAFETY: Both objects have verified classes. setVoice: takes a
                // voice object, retains it and returns void. voice returns +0.
                let selected: Option<Retained<AnyObject>> = unsafe {
                    let _: () = msg_send![&*object, setVoice: &*voice];
                    msg_send![&*object, voice]
                };
                let selected = checked_object(selected, self.voice_class)?;
                if voice_id(&selected)? != voice_id(&voice)? {
                    return Err(Error::VoiceUnavailable);
                }
                let objects = NativeObjects {
                    synthesizer: Retained::as_ptr(&self.synthesizer) as usize,
                    utterance: Retained::as_ptr(&object) as usize,
                };
                Ok((
                    MacUtterance {
                        object,
                        _voice: voice,
                    },
                    objects,
                ))
            })
        }

        fn attach(&mut self, inbox: Arc<Inbox>) -> Result<()> {
            if self.delegate.is_some() {
                return Err(Error::NativeFailure);
            }
            native_call(|| {
                let allocated = Delegate::alloc().set_ivars(inbox);
                // SAFETY: Ivars are initialized; NSObject's inherited init
                // consumes the allocation and returns the retained delegate.
                let delegate: Option<Retained<Delegate>> =
                    unsafe { msg_send![super(allocated), init] };
                self.delegate = Some(delegate.ok_or(Error::NativeFailure)?);
                let delegate = self.delegate.as_deref().ok_or(Error::NativeFailure)?;
                // SAFETY: The retained delegate conforms to the SDK protocol.
                // setDelegate: accepts the Objective-C object and returns void.
                unsafe {
                    let _: () = msg_send![&*self.synthesizer, setDelegate: delegate];
                }
                Ok(())
            })
        }

        fn speak(
            &self,
            utterance: &Self::Utterance,
            before_dispatch: impl FnOnce() -> Result<()>,
        ) -> Result<()> {
            native_call(|| {
                let selector = sel!(speakUtterance:);
                // objc2's debug message path performs method lookup, which
                // can resolve methods. Perform validation before the final gate.
                self.synthesizer
                    .class()
                    .verify_sel::<(*mut AnyObject,), ()>(selector)
                    .map_err(|_| Error::NativeFailure)?;
                let receiver = Retained::as_ptr(&self.synthesizer).cast_mut();
                let argument = Retained::as_ptr(&utterance.object).cast_mut();
                type SendSpeech = unsafe extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject);
                // SAFETY: Apple declares a void object-argument method. Use
                // the same typed objc_msgSend ABI as objc2's Apple dispatch.
                let send: SendSpeech = unsafe {
                    std::mem::transmute(objc2::ffi::objc_msgSend as unsafe extern "C-unwind" fn())
                };
                before_dispatch()?;
                // SAFETY: The selector signature was verified above. Both
                // retained objects outlive the call on the main thread, and
                // the enclosing native_call catches Objective-C exceptions.
                unsafe { send(receiver, selector, argument) };
                Ok(())
            })
        }

        fn stop_immediately(&self) -> Result<bool> {
            native_call(|| {
                // SAFETY: The SDK defines AVSpeechBoundary as NSInteger and
                // Immediate as zero. The return is Objective-C BOOL, not proof
                // that the delegate observed cancellation.
                let accepted: Bool =
                    unsafe { msg_send![&*self.synthesizer, stopSpeakingAtBoundary: 0_isize] };
                Ok(accepted.as_bool())
            })
        }

        fn detach(&mut self) -> Detach {
            let setter = native_call(|| {
                // SAFETY: The SDK declares a weak nullable delegate. The
                // retained synthesizer accepts a nil object and returns void.
                unsafe {
                    let _: () =
                        msg_send![&*self.synthesizer, setDelegate: std::ptr::null::<AnyObject>()];
                }
                Ok(())
            });
            let disposition = match setter {
                Err(_) => Detach::SetterFailed,
                Ok(()) => {
                    let readback = native_call(|| {
                        // SAFETY: The SDK getter returns a nullable Objective-C
                        // object at +0. The bridge still retains its delegate,
                        // so a nil result cannot be due to dropping that retain.
                        let delegate: Option<Retained<AnyObject>> =
                            unsafe { msg_send![&*self.synthesizer, delegate] };
                        Ok(delegate.is_none())
                    });
                    match readback {
                        Ok(true) => Detach::VerifiedNil,
                        Ok(false) | Err(_) => Detach::ReadbackUnknown,
                    }
                }
            };
            // Nil readback does not join callbacks already on the stack. The
            // inbox was closed before detach and stays retained by the delegate.
            self.delegate = None;
            disposition
        }
    }
}

#[path = "native_speech_tests.rs"]
mod tests;

#[cfg(all(test, target_os = "macos"))]
pub(super) use tests::{preparation_fixture, retirement_fixture};
