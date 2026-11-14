//! P1-09 isolated, deterministic fake role ports. Test-only synthetic jobs and
//! responses; no runtime wiring to consent, egress, provenance, speech or capture.
//! Fixture jobs are NOT sealed egress jobs or proof of any independent approval.
//! All returned content is untrusted data. Nothing here sends, executes or plays.
use super::clock::{Deadline, MonotonicTime};
use super::limits::*;
use super::model::*;
use super::ContractError;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::marker::PhantomData;
use std::num::NonZeroU64;
use std::sync::Arc;
use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Error {
    Contract(ContractError),
    Binding,
    Stale,
    Replay,
    Busy,
    Stopped,
    Expired,
    NoScript,
    OutOfOrder,
    NotDue,
    Malformed,
    UntrustedJob,
}
impl From<ContractError> for Error {
    fn from(value: ContractError) -> Self {
        Self::Contract(value)
    }
}
type Result<T> = std::result::Result<T, Error>;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
struct Target {
    provider: NonZeroU64,
    model: NonZeroU64,
    voice: Option<NonZeroU64>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
struct Binding {
    identity: ContractIdentity,
    role: ProviderRole,
    target: Target,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    AwaitingStart,
    Receiving,
    Succeeded,
    Failed,
    TimedOut,
    Canceled,
    Uncertain,
    Malformed,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Failure {
    Unavailable,
    Rejected,
}
#[derive(Clone)]
enum Event {
    Started,
    Chunk(Vec<u8>),
    Finished,
    Failed(Failure),
    Uncertain,
}
#[derive(Clone)]
struct Scheduled {
    after: Duration,
    event: Event,
}
#[derive(Clone)]
struct Script {
    events: Vec<Scheduled>,
}

// Auxiliary fake-port states only. These do not extend product lifecycle enums.
enum Update<T> {
    Started,
    Progress,
    Succeeded(Completion<T>),
    Ended(State),
}
struct Completion<T> {
    binding: Binding,
    output: T,
}
struct Operation<R> {
    adapter: Arc<()>,
    nonce: Arc<()>,
    binding: Binding,
    sequence: u64,
    role: PhantomData<R>,
}
struct Callback<R> {
    operation: Operation<R>,
    index: u64,
}
impl<R> Operation<R> {
    fn copy_for_callback(&self) -> Self {
        Self {
            adapter: self.adapter.clone(),
            nonce: self.nonce.clone(),
            binding: self.binding,
            sequence: self.sequence,
            role: PhantomData,
        }
    }
}
struct Attempt {
    binding: Binding,
    state: State,
}
struct Active<R> {
    operation: Operation<R>,
    slot: usize,
    started_at: MonotonicTime,
    deadline: Deadline,
    script: Script,
    cursor: u64,
    received: u64,
    buffer: Vec<u8>,
    project: Option<String>,
}

/// These marker types provide four distinct input/output ports. Conversation
/// cannot submit a planning job or return a plan through its typed interface.
trait Role {
    const ROLE: ProviderRole;
    type Input: Serialize;
    type Output;
    fn validate_input(input: &Self::Input) -> Result<()>;
    fn decode(bytes: &[u8], project: Option<&str>) -> Result<Self::Output>;
    fn audio(_input: &Self::Input) -> Duration {
        Duration::ZERO
    }
    fn project(_input: &Self::Input) -> Option<&str> {
        None
    }
}
struct Stt;
struct Conversation;
struct Planning;
struct Tts;
type FakeStt = FakeAdapter<Stt>;
type FakeConversation = FakeAdapter<Conversation>;
type FakePlanning = FakeAdapter<Planning>;
type FakeTts = FakeAdapter<Tts>;

#[derive(Serialize)]
struct SyntheticAudio {
    zeros: Vec<u8>,
    duration: Duration,
}
#[derive(Serialize)]
struct SyntheticConversation {
    text: String,
}
#[derive(Serialize)]
struct SyntheticPlanning {
    reviewed_text: String,
    project_alias: Option<String>,
}
#[derive(Serialize)]
struct SyntheticSpeechText {
    text: String,
}

struct UntrustedTranscript {
    text: String,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ReplyKind {
    Answer,
    Clarify,
    NeedsActionReview,
}
struct UntrustedReply {
    kind: ReplyKind,
    text: String,
}
struct UntrustedProposal {
    plan: crate::jarvis::model::Plan,
}
struct SyntheticAudioOutput {
    zeros: Vec<u8>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TranscriptWire {
    version: ContractVersion,
    text: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReplyWire {
    version: ContractVersion,
    kind: ReplyKind,
    text: String,
}

fn bounded_text(text: &str, maximum: u64) -> Result<()> {
    checked_total(
        0,
        text.len().try_into().map_err(|_| ContractError::Overflow)?,
        maximum,
    )?;
    if text.trim().is_empty()
        || text
            .chars()
            .any(|c| c.is_control() && c != '\n' && c != '\t')
    {
        return Err(Error::Malformed);
    }
    Ok(())
}
fn reject_confirmation_tokens(text: &str) -> Result<()> {
    let words: Vec<_> = text.split_whitespace().collect();
    if words.windows(2).any(|w| w[0] == "APPLY")
        || words.windows(4).any(|w| {
            (w[0] == "RUN" && w[2] == "PLAN")
                || (w[0] == "SEND" && w[2] == "TO" && w[3] == "OPENAI")
        })
        || words
            .iter()
            .any(|w| w.len() == 64 && w.bytes().all(|b| b.is_ascii_hexdigit()))
    {
        return Err(Error::Malformed);
    }
    Ok(())
}
impl Role for Stt {
    const ROLE: ProviderRole = ProviderRole::Stt;
    type Input = SyntheticAudio;
    type Output = UntrustedTranscript;
    fn validate_input(input: &Self::Input) -> Result<()> {
        checked_total(
            0,
            input
                .zeros
                .len()
                .try_into()
                .map_err(|_| ContractError::Overflow)?,
            MAX_ENCODED_AUDIO_BYTES,
        )?;
        checked_duration_total(Duration::ZERO, input.duration, MAX_CAPTURED_TURN)?;
        if input.zeros.is_empty() || input.duration.is_zero() || input.zeros.iter().any(|b| *b != 0)
        {
            return Err(Error::Malformed);
        }
        Ok(())
    }
    fn decode(bytes: &[u8], _project: Option<&str>) -> Result<Self::Output> {
        let wire: TranscriptWire = serde_json::from_slice(bytes).map_err(|_| Error::Malformed)?;
        let ContractVersion::V1 = wire.version;
        bounded_text(&wire.text, MAX_USER_TRANSCRIPT_BYTES)?;
        Ok(UntrustedTranscript { text: wire.text })
    }
    fn audio(input: &Self::Input) -> Duration {
        input.duration
    }
}
impl Role for Conversation {
    const ROLE: ProviderRole = ProviderRole::Conversation;
    type Input = SyntheticConversation;
    type Output = UntrustedReply;
    fn validate_input(input: &Self::Input) -> Result<()> {
        bounded_text(&input.text, MAX_CONVERSATION_INPUT_BYTES)
    }
    fn decode(bytes: &[u8], _project: Option<&str>) -> Result<Self::Output> {
        let wire: ReplyWire = serde_json::from_slice(bytes).map_err(|_| Error::Malformed)?;
        let ContractVersion::V1 = wire.version;
        bounded_text(&wire.text, MAX_ASSISTANT_REPLY_BYTES)?;
        reject_confirmation_tokens(&wire.text)?;
        Ok(UntrustedReply {
            kind: wire.kind,
            text: wire.text,
        })
    }
}
impl Role for Planning {
    const ROLE: ProviderRole = ProviderRole::Planning;
    type Input = SyntheticPlanning;
    type Output = UntrustedProposal;
    fn validate_input(input: &Self::Input) -> Result<()> {
        bounded_text(&input.reviewed_text, MAX_CONVERSATION_INPUT_BYTES)?;
        if let Some(alias) = &input.project_alias {
            crate::jarvis::model::alias(alias).map_err(|_| Error::Malformed)?;
        }
        Ok(())
    }
    fn decode(bytes: &[u8], project: Option<&str>) -> Result<Self::Output> {
        let plan: crate::jarvis::model::Plan =
            serde_json::from_slice(bytes).map_err(|_| Error::Malformed)?;
        // Fixed synthetic calendar data for Jarvis's pure validator; never a wall
        // clock reading. All event scheduling/expiry uses MonotonicTime only.
        let synthetic_date = chrono::DateTime::from_timestamp(0, 0).expect("fixed synthetic date");
        plan.validate(project, synthetic_date)
            .map_err(|_| Error::Malformed)?;
        Ok(UntrustedProposal { plan })
    }
    fn project(input: &Self::Input) -> Option<&str> {
        input.project_alias.as_deref()
    }
}
const FAKE_AUDIO_HEADER: &[u8] = b"GHOST_SYNTHETIC_AUDIO_V1\0";
impl Role for Tts {
    const ROLE: ProviderRole = ProviderRole::Tts;
    type Input = SyntheticSpeechText;
    type Output = SyntheticAudioOutput;
    fn validate_input(input: &Self::Input) -> Result<()> {
        bounded_text(&input.text, MAX_SPEECH_JOB_TEXT_BYTES)
    }
    fn decode(bytes: &[u8], _project: Option<&str>) -> Result<Self::Output> {
        // The common 256 KiB full-response cap applies to synthetic audio too;
        // it can be stricter than this independent 8 MiB speech-audio ceiling.
        let zeros = bytes
            .strip_prefix(FAKE_AUDIO_HEADER)
            .ok_or(Error::Malformed)?;
        checked_total(
            0,
            zeros
                .len()
                .try_into()
                .map_err(|_| ContractError::Overflow)?,
            MAX_SPEECH_JOB_AUDIO_BYTES,
        )?;
        if zeros.is_empty() || zeros.iter().any(|b| *b != 0) {
            return Err(Error::Malformed);
        }
        Ok(SyntheticAudioOutput {
            zeros: zeros.into(),
        })
    }
}

/// Scripted simulation only: one in-flight job per role port. Attempt/audio
/// accounting is local test bookkeeping, not an egress reservation or consent.
/// Real grants, exact private review and provenance validation remain external.
struct FakeAdapter<R: Role> {
    owner: Arc<()>,
    scope: ContractIdentity,
    target: Target,
    stopped: bool,
    last_time: MonotonicTime,
    deadline: Deadline,
    sequence: u64,
    attempts: u64,
    audio: Duration,
    ledger: Vec<Attempt>,
    scripts: VecDeque<Script>,
    active: Option<Active<R>>,
}
impl<R: Role> FakeAdapter<R> {
    fn new(
        scope: ContractIdentity,
        target: Target,
        scripts: Vec<Script>,
        now: MonotonicTime,
    ) -> Result<Self> {
        if (R::ROLE == ProviderRole::Tts) != target.voice.is_some() {
            return Err(Error::Binding);
        }
        checked_total(
            0,
            scripts
                .len()
                .try_into()
                .map_err(|_| ContractError::Overflow)?,
            MAX_PROVIDER_REQUESTS_PER_ROLE_PER_INTERACTION,
        )?;
        for script in &scripts {
            // Bound script metadata too: at most one event per response byte plus
            // start/end. Tests can still script malformed/oversized wire chunks.
            checked_total(
                0,
                script
                    .events
                    .len()
                    .try_into()
                    .map_err(|_| ContractError::Overflow)?,
                MAX_PROVIDER_RESPONSE_BYTES
                    .checked_add(2)
                    .ok_or(ContractError::Overflow)?,
            )?;
            let mut prior = Duration::ZERO;
            for event in &script.events {
                if event.after < prior {
                    return Err(Error::OutOfOrder);
                }
                prior = event.after;
            }
        }
        Ok(Self {
            owner: Arc::new(()),
            scope,
            target,
            stopped: false,
            deadline: Deadline::after(now.clone(), MAX_CLOUD_INTERACTION)?,
            last_time: now,
            sequence: 0,
            attempts: 0,
            audio: Duration::ZERO,
            ledger: Vec::new(),
            scripts: scripts.into(),
            active: None,
        })
    }
    fn observe(&mut self, now: &MonotonicTime) -> Result<()> {
        if self.stopped {
            return Err(Error::Stopped);
        }
        now.checked_duration_since(&self.last_time)?;
        self.last_time = now.clone();
        if self.deadline.is_expired(now)? {
            if self.active.is_some() {
                self.retire(State::TimedOut);
            }
            self.stopped = true;
            return Err(Error::Expired);
        }
        Ok(())
    }
    fn submit(&mut self, job: &fixtures::Job<R>, now: &MonotonicTime) -> Result<Operation<R>> {
        self.observe(now)?;
        if self.active.is_some() {
            return Err(Error::Busy);
        }
        let binding = job.binding();
        let id = binding.identity;
        let mut expected = self.scope;
        expected.turn_id = id.turn_id;
        expected.job_id = id.job_id;
        if id != expected || binding.role != R::ROLE || binding.target != self.target {
            return Err(Error::Binding);
        }
        if self.ledger.iter().any(|a| {
            a.binding.identity.job_id == id.job_id || a.binding.identity.turn_id == id.turn_id
        }) {
            return Err(Error::Replay);
        }
        if job.deadline().is_expired(now)? {
            return Err(Error::Expired);
        }
        R::validate_input(job.input())?;
        let envelope = serde_json::to_vec(&(binding, job.input())).map_err(|_| Error::Malformed)?;
        checked_total(
            0,
            envelope
                .len()
                .try_into()
                .map_err(|_| ContractError::Overflow)?,
            MAX_PROVIDER_REQUEST_BYTES,
        )?;
        let attempts = checked_total(
            self.attempts,
            1,
            MAX_PROVIDER_REQUESTS_PER_ROLE_PER_INTERACTION,
        )?;
        let audio = checked_duration_total(self.audio, R::audio(job.input()), MAX_CLOUD_AUDIO)?;
        let sequence = self
            .sequence
            .checked_add(1)
            .ok_or(ContractError::Overflow)?;
        let script = self.scripts.front().ok_or(Error::NoScript)?;
        for event in &script.events {
            now.checked_add(event.after)?;
        }
        let role_lifetime = if R::ROLE == ProviderRole::Tts {
            SPEECH_OUTPUT_DEADLINE
        } else {
            MAX_CHILD_CONVERSATION
        };
        let lifetime = job
            .deadline()
            .remaining(now)?
            .min(self.deadline.remaining(now)?)
            .min(role_lifetime);
        let deadline = Deadline::after(now.clone(), lifetime)?;
        let operation = Operation {
            adapter: self.owner.clone(),
            nonce: Arc::new(()),
            binding,
            sequence,
            role: PhantomData,
        };
        let slot = self.ledger.len();
        // Every fallible check precedes this irreversible synthetic attempt.
        self.ledger.push(Attempt {
            binding,
            state: State::AwaitingStart,
        });
        self.attempts = attempts;
        self.audio = audio;
        self.sequence = sequence;
        self.active = Some(Active {
            operation: operation.copy_for_callback(),
            slot,
            started_at: now.clone(),
            deadline,
            script: self.scripts.pop_front().unwrap(),
            cursor: 0,
            received: 0,
            buffer: Vec::new(),
            project: R::project(job.input()).map(str::to_owned),
        });
        Ok(operation)
    }
    fn check_operation(&self, operation: &Operation<R>) -> Result<()> {
        if self.stopped {
            return Err(Error::Stopped);
        }
        let active = self.active.as_ref().ok_or(Error::Stale)?;
        if !Arc::ptr_eq(&self.owner, &operation.adapter)
            || !Arc::ptr_eq(&active.operation.nonce, &operation.nonce)
            || active.operation.binding != operation.binding
            || active.operation.sequence != operation.sequence
        {
            return Err(Error::Binding);
        }
        Ok(())
    }
    /// Test driver can obtain a later ticket to exercise out-of-order delivery.
    /// Nothing is scheduled on an executor and no timer/thread is started.
    fn callback_at(&self, operation: &Operation<R>, index: u64) -> Result<Callback<R>> {
        self.check_operation(operation)?;
        let index_usize = usize::try_from(index).map_err(|_| ContractError::Overflow)?;
        if self
            .active
            .as_ref()
            .unwrap()
            .script
            .events
            .get(index_usize)
            .is_none()
        {
            return Err(Error::Stale);
        }
        Ok(Callback {
            operation: operation.copy_for_callback(),
            index,
        })
    }
    fn deliver(
        &mut self,
        callback: &Callback<R>,
        now: &MonotonicTime,
    ) -> Result<Update<R::Output>> {
        self.check_operation(&callback.operation)?;
        let active = self.active.as_ref().unwrap();
        if callback.index != active.cursor {
            return Err(Error::OutOfOrder);
        }
        let index = usize::try_from(callback.index).map_err(|_| ContractError::Overflow)?;
        let scheduled = active.script.events.get(index).ok_or(Error::Stale)?;
        let due = active.started_at.checked_add(scheduled.after)?;
        now.checked_duration_since(&self.last_time)?;
        // Validate every applicable clock domain before any expiry mutation.
        let expired = active.deadline.is_expired(now)?;
        let due_now = now.checked_cmp(&due)? != std::cmp::Ordering::Less;
        self.observe(now)?;
        if expired {
            self.retire(State::TimedOut);
            return Ok(Update::Ended(State::TimedOut));
        }
        if !due_now {
            return Err(Error::NotDue);
        }
        let active = self.active.as_ref().unwrap();
        let scheduled = &active.script.events[index];
        let state = self.ledger[active.slot].state;
        if !matches!(scheduled.event, Event::Failed(_) | Event::Uncertain)
            && !matches!(
                (&scheduled.event, state),
                (Event::Started, State::AwaitingStart)
                    | (Event::Chunk(_) | Event::Finished, State::Receiving)
            )
        {
            return Err(Error::OutOfOrder);
        }
        let cursor = match active.cursor.checked_add(1) {
            Some(next) => next,
            None => {
                self.retire(State::Malformed);
                return Err(ContractError::Overflow.into());
            }
        };
        let event = scheduled.event.clone();
        self.active.as_mut().unwrap().cursor = cursor;
        match event {
            Event::Started => {
                let slot = self.active.as_ref().unwrap().slot;
                self.ledger[slot].state = State::Receiving;
                Ok(Update::Started)
            }
            Event::Chunk(bytes) => {
                let active = self.active.as_mut().unwrap();
                let total = match checked_total(
                    active.received,
                    bytes
                        .len()
                        .try_into()
                        .map_err(|_| ContractError::Overflow)?,
                    MAX_PROVIDER_RESPONSE_BYTES,
                ) {
                    Ok(total) => total,
                    Err(e) => {
                        self.retire(State::Malformed);
                        return Err(e.into());
                    }
                };
                active.received = total;
                active.buffer.extend(bytes);
                Ok(Update::Progress)
            }
            Event::Finished => {
                let active = self.active.as_ref().unwrap();
                let output = R::decode(&active.buffer, active.project.as_deref());
                let binding = active.operation.binding;
                match output {
                    Ok(output) => {
                        self.retire(State::Succeeded);
                        Ok(Update::Succeeded(Completion { binding, output }))
                    }
                    Err(e) => {
                        self.retire(State::Malformed);
                        Err(e)
                    }
                }
            }
            Event::Failed(_category) => {
                self.retire(State::Failed);
                Ok(Update::Ended(State::Failed))
            }
            Event::Uncertain => {
                self.retire(State::Uncertain);
                Ok(Update::Ended(State::Uncertain))
            }
        }
    }
    fn retire(&mut self, state: State) {
        if let Some(active) = self.active.take() {
            self.ledger[active.slot].state = state;
        }
        // Drop all buffered response data. No refund, retry, fallback or next job.
    }
    fn tick(&mut self, now: &MonotonicTime) -> Result<Option<State>> {
        self.observe(now)?;
        if let Some(active) = &self.active {
            if active.deadline.is_expired(now)? {
                self.retire(State::TimedOut);
                return Ok(Some(State::TimedOut));
            }
        }
        Ok(None)
    }
    fn cancel(&mut self, operation: &Operation<R>) -> Result<State> {
        self.check_operation(operation)?;
        self.retire(State::Canceled);
        Ok(State::Canceled)
    }
    fn stop(&mut self) {
        self.retire(State::Canceled);
        self.stopped = true;
        self.scripts.clear();
    }
}

/// Fixture ingress has no constructor from renderer/provider JSON. Its only
/// issuer is explicitly synthetic, not a native consent/egress bridge.
mod fixtures {
    use super::*;
    pub(super) struct Job<R: Role> {
        binding: Binding,
        deadline: Deadline,
        input: R::Input,
    }
    impl<R: Role> Job<R> {
        pub(super) fn binding(&self) -> Binding {
            self.binding
        }
        pub(super) fn deadline(&self) -> &Deadline {
            &self.deadline
        }
        pub(super) fn input(&self) -> &R::Input {
            &self.input
        }
    }
    #[cfg(test)]
    pub(super) fn synthetic<R: Role>(
        identity: ContractIdentity,
        target: Target,
        deadline: Deadline,
        input: R::Input,
    ) -> Job<R> {
        Job {
            binding: Binding {
                identity,
                role: R::ROLE,
                target,
            },
            deadline,
            input,
        }
    }
    pub(super) fn from_untrusted<R: Role>(_bytes: &[u8]) -> Result<Job<R>> {
        Err(Error::UntrustedJob)
    }
}

#[cfg(test)]
#[path = "adapters_tests.rs"]
mod tests;
