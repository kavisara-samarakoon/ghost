//! P1-07 deterministic, test-only egress. No transport or execution authority.
//! Consent remains the issuer/validator. P1-08 provenance is deliberately absent:
//! only sealed, explicitly synthetic payload fixtures can enter this model.
use super::*;
use crate::voice_conversation::model::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EgressError {
    Consent(Error),
    Contract(ContractError),
    MissingGrant,
    Binding,
    UntrustedPayload,
    Duplicate,
    Stale,
    Stopped,
}
impl From<Error> for EgressError {
    fn from(value: Error) -> Self {
        Self::Consent(value)
    }
}
impl From<ContractError> for EgressError {
    fn from(value: ContractError) -> Self {
        Self::Contract(value)
    }
}
type EgressResult<T> = std::result::Result<T, EgressError>;

// Private fields prevent caller declarations from masquerading as provenance.
// This is NOT a native provenance implementation and has no live-input issuer.
mod synthetic {
    use super::*;

    pub(super) struct Payload {
        identity: ContractIdentity,
        data: DataClass,
        provenance: Provenance,
        text: String,
        audio: Vec<u8>,
        duration: Duration,
    }
    impl Payload {
        pub(super) fn identity(&self) -> ContractIdentity {
            self.identity
        }
        pub(super) fn data(&self) -> DataClass {
            self.data
        }
        pub(super) fn provenance(&self) -> Provenance {
            self.provenance
        }
        pub(super) fn text(&self) -> &str {
            &self.text
        }
        pub(super) fn audio(&self) -> &[u8] {
            &self.audio
        }
        pub(super) fn duration(&self) -> Duration {
            self.duration
        }
        /// Explicit synthetic fixture ONLY; never accepts renderer/provider input.
        #[cfg(test)]
        pub(super) fn fixture(
            identity: ContractIdentity,
            data: DataClass,
            provenance: Provenance,
            text: &str,
            audio_bytes: usize,
            duration: Duration,
        ) -> Self {
            Self {
                identity,
                data,
                provenance,
                text: text.into(),
                audio: vec![0; audio_bytes],
                duration,
            }
        }
        pub(super) fn from_untrusted(_text: &str, _declared: Provenance) -> EgressResult<Self> {
            Err(EgressError::UntrustedPayload)
        }
    }
}

struct Intent {
    context: Context,
    domain: Domain,
    purpose: Purpose,
    target: Target,
    role: ProviderRole,
    data: DataClass,
    provenance: Provenance,
    background: bool,
    project: Option<ProjectId>,
}
impl Intent {
    fn request<'a>(&self, payload: &'a synthetic::Payload) -> EgressResult<UseRequest<'a>> {
        if self.context.identity != payload.identity()
            || self.data != payload.data()
            || self.provenance != payload.provenance()
        {
            return Err(EgressError::Binding);
        }
        Ok(UseRequest {
            context: self.context.clone(),
            domain: self.domain,
            purpose: self.purpose,
            target: Some(self.target),
            data: self.data,
            provenance: self.provenance,
            background: self.background,
            project: self.project,
            reviewed_snapshot: (self.domain == Domain::PrivateContext).then_some(payload.text()),
            requests: 1,
            audio: payload.duration(),
            text_bytes: u64::try_from(payload.text().len()).map_err(|_| ContractError::Overflow)?,
        })
    }
    fn check_role(&self, payload: &synthetic::Payload) -> EgressResult<ConversationProviderRole> {
        let role = ConversationProviderRole::try_from(self.role)?;
        let allowed = match role {
            ConversationProviderRole::Stt => {
                self.domain == Domain::CloudTranscription
                    && self.purpose == Purpose::Transcription
                    && self.data == DataClass::PostCueAudio
                    && payload.text().is_empty()
                    && !payload.audio().is_empty()
                    && payload.duration() > Duration::ZERO
            }
            ConversationProviderRole::Conversation => {
                matches!(
                    (self.domain, self.purpose),
                    (Domain::Conversation, Purpose::OrdinaryConversation)
                        | (Domain::PrivateContext, Purpose::PrivateDisclosure)
                ) && payload.audio().is_empty()
                    && payload.duration() == Duration::ZERO
                    && !payload.text().is_empty()
            }
            ConversationProviderRole::Tts => {
                self.domain == Domain::CloudTts
                    && self.purpose == Purpose::SpeechOutput
                    && self.data == DataClass::AssistantReply
                    && payload.audio().is_empty()
                    && payload.duration() == Duration::ZERO
                    && !payload.text().is_empty()
            }
        };
        if !allowed {
            return Err(EgressError::Binding);
        }
        Ok(role)
    }
}

#[derive(Clone, Default)]
struct Usage {
    requests: u64,
    audio: Duration,
    text: u64,
}
impl Usage {
    fn charged(&self, request: &UseRequest<'_>, limits: Limits) -> EgressResult<Self> {
        Ok(Self {
            requests: checked_total(self.requests, 1, limits.requests)?,
            audio: checked_duration_total(self.audio, request.audio, limits.audio)?,
            text: checked_total(self.text, request.text_bytes, limits.text_bytes)?,
        })
    }
}

struct Child {
    id: SessionId,
    deadline: Deadline,
    turns: u64,
}
struct Turn {
    context: Context,
    roles: [bool; 3],
    canceled: bool,
}
struct GrantUsage {
    grant: Grant,
    usage: Usage,
}

/// No Clone, serde, getters yielding mutable content, or adapter constructors.
/// Retains the exact authorized bytes, native binding and selected target.
struct SealedJob(Arc<Authorized>);
struct Authorized {
    owner: Arc<()>,
    sequence: u64,
    intent: Intent,
    grant: Grant,
    payload: synthetic::Payload,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum JobState {
    Reserved,
    Attempted,
    Finished,
    Canceled,
}
struct JobRecord {
    job: Arc<Authorized>,
    state: JobState,
}
/// Opaque native callback identity. Provider-returned IDs cannot construct one.
struct DispatchTicket(Arc<Authorized>);
#[derive(Clone, Copy)]
enum DispatchOutcome {
    Succeeded,
    Failed,
    Uncertain,
}

/// One non-resettable native ledger per interaction. All methods are synchronous
/// exclusive mutations; callers may serialize access with a short Mutex. No lock
/// or mutable borrow is held across provider work (there is no provider here).
struct EgressGate {
    owner: Arc<()>,
    run: AppRunId,
    interaction: InteractionId,
    revision: ConfigurationRevision,
    policy: PolicyRevision,
    last_time: MonotonicTime,
    deadline: Deadline,
    stopped: bool,
    sequence: u64,
    children: Vec<Child>,
    turns: Vec<Turn>,
    grants: Vec<GrantUsage>,
    roles: [u64; 3],
    targets: [Option<Target>; 3],
    cloud_audio: Duration,
    jobs: Vec<JobRecord>,
}
impl EgressGate {
    fn new(context: &Context, now: MonotonicTime) -> EgressResult<Self> {
        Ok(Self {
            owner: Arc::new(()),
            run: context.identity.app_run_id,
            interaction: context.identity.interaction_id,
            revision: context.identity.configuration_revision,
            policy: context.policy_revision,
            deadline: Deadline::after(now.clone(), MAX_CLOUD_INTERACTION)?,
            last_time: now,
            stopped: false,
            sequence: 0,
            children: Vec::new(),
            turns: Vec::new(),
            grants: Vec::new(),
            roles: [0; 3],
            targets: [None; 3],
            cloud_audio: Duration::ZERO,
            jobs: Vec::new(),
        })
    }
    fn observe(&mut self, context: &Context, now: &MonotonicTime) -> EgressResult<()> {
        if self.stopped {
            return Err(EgressError::Stopped);
        }
        now.checked_duration_since(&self.last_time)?;
        // Even rejected/expired calls advance the clock high-water mark. An old
        // timestamp must never revive a job after an observed deadline failure.
        self.last_time = now.clone();
        if self.deadline.is_expired(now)? {
            return Err(Error::Expired.into());
        }
        if context.identity.app_run_id != self.run
            || context.identity.interaction_id != self.interaction
            || context.identity.configuration_revision != self.revision
            || context.policy_revision != self.policy
        {
            return Err(EgressError::Binding);
        }
        if let Some(child) = self
            .children
            .iter()
            .find(|c| c.id == context.identity.session_id)
        {
            if child.deadline.is_expired(now)? {
                return Err(Error::Expired.into());
            }
        }
        Ok(())
    }
    fn reserve(
        &mut self,
        registry: &mut ConsentRegistry,
        grant: Option<&Grant>,
        intent: Intent,
        payload: synthetic::Payload,
        now: &MonotonicTime,
    ) -> EgressResult<SealedJob> {
        self.observe(&intent.context, now)?;
        let grant = grant.ok_or(EgressError::MissingGrant)?;
        let request = intent.request(&payload)?;
        // Never infer consent from the intent, payload, cached eligibility or IDs.
        registry.validate(grant, &request, now)?;
        let role = intent.check_role(&payload)?;
        let index = role.index();
        if self.targets[index].is_some_and(|target| target != intent.target) {
            return Err(EgressError::Binding);
        }
        checked_duration_total(Duration::ZERO, request.audio, MAX_CAPTURED_TURN)?;
        checked_total(
            0,
            u64::try_from(payload.audio().len()).map_err(|_| ContractError::Overflow)?,
            MAX_ENCODED_AUDIO_BYTES,
        )?;
        let class_limit = match intent.data {
            DataClass::OrdinaryText => MAX_USER_TRANSCRIPT_BYTES,
            DataClass::AssistantReply => MAX_ASSISTANT_REPLY_BYTES,
            DataClass::OrdinaryHistory => MAX_HISTORY_BYTES,
            _ => MAX_CONVERSATION_INPUT_BYTES,
        };
        checked_total(0, request.text_bytes, class_limit)?;
        // Count the complete synthetic envelope, including audio and JSON
        // escaping. The 64 KiB full-request cap can be stricter than the 8 MiB
        // encoded-audio cap; no future adapter encoding/overhead is assumed.
        let envelope = serde_json::to_vec(&(
            &intent.context,
            intent.domain,
            intent.purpose,
            intent.target,
            intent.data,
            intent.role,
            intent.background,
            intent.project,
            payload.text(),
            payload.audio(),
            payload.duration(),
        ))
        .map_err(|_| EgressError::Consent(Error::Encoding))?;
        checked_total(
            0,
            u64::try_from(envelope.len()).map_err(|_| ContractError::Overflow)?,
            MAX_PROVIDER_REQUEST_BYTES,
        )?;
        let turn = self
            .turns
            .iter()
            .position(|t| t.context.identity.turn_id == intent.context.identity.turn_id);
        if let Some(turn) = turn {
            if self.turns[turn].context != intent.context {
                return Err(EgressError::Binding);
            }
            if self.turns[turn].canceled || self.turns[turn].roles[index] {
                return Err(EgressError::Duplicate);
            }
        } else {
            checked_total(
                self.turns
                    .len()
                    .try_into()
                    .map_err(|_| ContractError::Overflow)?,
                1,
                MAX_AGGREGATE_TURNS,
            )?;
        }
        let child = self
            .children
            .iter()
            .position(|c| c.id == intent.context.identity.session_id);
        let child_deadline = Deadline::after(now.clone(), MAX_CHILD_CONVERSATION)?;
        let child_turns = if let Some(child) = child {
            if self.children[child].deadline.is_expired(now)? {
                return Err(Error::Expired.into());
            }
            checked_total(
                self.children[child].turns,
                u64::from(turn.is_none()),
                MAX_CHILD_TURNS,
            )?
        } else {
            checked_total(
                self.children
                    .len()
                    .try_into()
                    .map_err(|_| ContractError::Overflow)?,
                1,
                MAX_CHILD_CONVERSATIONS,
            )?;
            1
        };
        let requests = checked_total(
            self.roles[index],
            1,
            MAX_PROVIDER_REQUESTS_PER_ROLE_PER_INTERACTION,
        )?;
        let cloud_audio = checked_duration_total(self.cloud_audio, request.audio, MAX_CLOUD_AUDIO)?;
        let grant_slot = self
            .grants
            .iter()
            .position(|u| Arc::ptr_eq(&u.grant.data, &grant.data));
        let usage = grant_slot
            .map(|i| self.grants[i].usage.clone())
            .unwrap_or_default()
            .charged(&request, grant.data.preview.content.specification.limits)?;
        let sequence = self
            .sequence
            .checked_add(1)
            .ok_or(ContractError::Overflow)?;

        // All fallible checks precede this single exclusive commit. No budget can
        // be partially charged on rejection; no job exists before all charges.
        if let Some(child) = child {
            self.children[child].turns = child_turns;
        } else {
            self.children.push(Child {
                id: intent.context.identity.session_id,
                deadline: child_deadline,
                turns: child_turns,
            });
        }
        let turn = turn.unwrap_or_else(|| {
            self.turns.push(Turn {
                context: intent.context.clone(),
                roles: [false; 3],
                canceled: false,
            });
            self.turns.len() - 1
        });
        self.turns[turn].roles[index] = true;
        if let Some(slot) = grant_slot {
            self.grants[slot].usage = usage;
        } else {
            self.grants.push(GrantUsage {
                grant: grant.clone(),
                usage,
            });
        }
        self.roles[index] = requests;
        self.targets[index] = Some(intent.target);
        self.cloud_audio = cloud_audio;
        self.sequence = sequence;
        let job = Arc::new(Authorized {
            owner: self.owner.clone(),
            sequence,
            intent,
            grant: grant.clone(),
            payload,
        });
        self.jobs.push(JobRecord {
            job: job.clone(),
            state: JobState::Reserved,
        });
        Ok(SealedJob(job))
    }
    fn record(&self, job: &Arc<Authorized>) -> EgressResult<usize> {
        if !Arc::ptr_eq(&self.owner, &job.owner) {
            return Err(EgressError::Stale);
        }
        self.jobs
            .iter()
            .position(|r| Arc::ptr_eq(&r.job, job) && r.job.sequence == job.sequence)
            .ok_or(EgressError::Stale)
    }
    /// Simulates the dispatch admission instant only; invokes no adapter. A
    /// second fresh registry check prevents Stop/revocation after reservation.
    fn begin(
        &mut self,
        registry: &mut ConsentRegistry,
        job: &SealedJob,
        now: &MonotonicTime,
    ) -> EgressResult<DispatchTicket> {
        self.observe(&job.0.intent.context, now)?;
        let slot = self.record(&job.0)?;
        if self.jobs[slot].state != JobState::Reserved {
            return Err(EgressError::Stale);
        }
        let request = job.0.intent.request(&job.0.payload)?;
        if let Err(error) = registry.validate(&job.0.grant, &request, now) {
            self.jobs[slot].state = JobState::Canceled;
            return Err(error.into());
        }
        self.jobs[slot].state = JobState::Attempted;
        Ok(DispatchTicket(job.0.clone()))
    }
    /// Outcomes retire bookkeeping only. They cannot refund, authorize, retry,
    /// create another job, issue grants, or return trusted history/reply content.
    fn finish(
        &mut self,
        registry: &mut ConsentRegistry,
        ticket: &DispatchTicket,
        _outcome: DispatchOutcome,
        now: &MonotonicTime,
    ) -> EgressResult<()> {
        self.observe(&ticket.0.intent.context, now)?;
        let slot = self.record(&ticket.0)?;
        if self.jobs[slot].state != JobState::Attempted {
            return Err(EgressError::Stale);
        }
        let request = ticket.0.intent.request(&ticket.0.payload)?;
        self.jobs[slot].state = JobState::Finished;
        registry.validate(&ticket.0.grant, &request, now)?;
        Ok(())
    }
    fn cancel_turn(&mut self, context: &Context) {
        if !self
            .turns
            .iter()
            .any(|t| t.context.identity.turn_id == context.identity.turn_id)
        {
            if self.turns.len() >= MAX_AGGREGATE_TURNS as usize {
                self.stop();
                return;
            }
            self.turns.push(Turn {
                context: context.clone(),
                roles: [false; 3],
                canceled: true,
            });
        }
        for turn in &mut self.turns {
            if turn.context == *context {
                turn.canceled = true;
            }
        }
        for record in &mut self.jobs {
            if record.job.intent.context == *context {
                record.state = JobState::Canceled;
            }
        }
    }
    fn stop(&mut self) {
        // No clock, provider callback or successful generation increment needed.
        self.stopped = true;
        for record in &mut self.jobs {
            record.state = JobState::Canceled;
        }
    }
}

#[cfg(test)]
#[path = "egress_tests.rs"]
mod tests;
