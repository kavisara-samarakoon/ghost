//! P1-06: ephemeral native consent contracts and a test-only trusted presenter.
//! No IPC, persistence, real sheet, credentials, devices or provider dispatch.
use super::clock::{Deadline, MonotonicTime};
use super::limits::*;
use super::model::{ConfigurationRevision, ContractIdentity, Generation};
use super::ContractError;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::num::NonZeroU64;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::Duration;

const PREVIEW_LIFETIME: Duration = Duration::from_secs(5 * 60);
const DISCLOSURES: &str = "Grants and reviewed content are ephemeral for this application run. Spoken words may contain personal information or bystander speech; a wake word is not authentication. Ambient audio is never sent to providers. Cloud disclosure is limited to the displayed provider, model, purpose, data classes and bounds. Private content needs exact review. Stop Listening revokes voice authority; prior transmissions cannot be recalled. Recording Send, planning and workflow approvals remain independent.";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Error {
    Contract(ContractError),
    InvalidPolicy,
    Binding,
    Stale,
    Expired,
    Denied,
    UntrustedProof,
    Revoked,
    Busy,
    Closed,
    Encoding,
}
impl From<ContractError> for Error {
    fn from(error: ContractError) -> Self {
        Self::Contract(error)
    }
}
type Result<T> = std::result::Result<T, Error>;

macro_rules! native_id {
    ($($name:ident),+ $(,)?) => {$(
        #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
        #[serde(transparent)]
        struct $name(NonZeroU64);
        impl $name {
            fn new(sequence: u64) -> Result<Self> {
                NonZeroU64::new(sequence).map(Self).ok_or(ContractError::InvalidIdentifier.into())
            }
        }
    )+};
}
native_id!(
    OsSessionId,
    PolicyRevision,
    ProviderId,
    ModelId,
    VoiceId,
    ProjectId,
    PreviewId,
    GrantId
);

// Workflow approvals are deliberately absent from this domain type.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum Domain {
    Microphone,
    BackgroundWake,
    CloudTranscription,
    Conversation,
    PrivateContext,
    CloudTts,
}
impl Domain {
    const ALL: [Self; 6] = [
        Self::Microphone,
        Self::BackgroundWake,
        Self::CloudTranscription,
        Self::Conversation,
        Self::PrivateContext,
        Self::CloudTts,
    ];
    fn index(self) -> usize {
        Self::ALL.iter().position(|domain| *domain == self).unwrap()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum Purpose {
    ManualRecording,
    ConversationCapture,
    WakeDetection,
    Transcription,
    OrdinaryConversation,
    PrivateDisclosure,
    SpeechOutput,
    Planning,
    Tools,
    Workflow,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum DataClass {
    AmbientAudio,
    PostCueAudio,
    OrdinaryText,
    OrdinaryHistory,
    AssistantReply,
    PrivateSnapshot,
    GoogleDerived,
    Planning,
    Tools,
    Workflow,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Provenance {
    Ambient,
    PostCue,
    Ordinary,
    Private,
    DerivedPrivate,
    GoogleDerived,
    Planning,
    Tools,
    Workflow,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
struct Target {
    provider: ProviderId,
    model: ModelId,
    voice: Option<VoiceId>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
struct Context {
    identity: ContractIdentity,
    os_session: OsSessionId,
    policy_revision: PolicyRevision,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
struct Limits {
    lifetime: Duration,
    requests: u64,
    audio: Duration,
    text_bytes: u64,
}

#[derive(Clone, PartialEq, Eq, Serialize)]
struct ReviewedSnapshot {
    text: String,
    digest: [u8; 32],
}

#[derive(Clone, PartialEq, Eq, Serialize)]
struct Scope {
    background: bool,
    project: Option<ProjectId>,
    snapshot: Option<ReviewedSnapshot>,
}

#[derive(Clone, PartialEq, Eq, Serialize)]
struct Specification {
    domain: Domain,
    purpose: Purpose,
    target: Option<Target>,
    classes: Vec<DataClass>,
    scope: Scope,
    limits: Limits,
}
impl Specification {
    fn local_wake(&self) -> bool {
        self.domain == Domain::BackgroundWake
            || (self.domain == Domain::Microphone && self.purpose == Purpose::WakeDetection)
    }
    fn policy_valid(&self) -> bool {
        use DataClass as D;
        use Purpose as P;
        let (purpose_valid, allowed, requests, audio, text): (bool, &[D], u64, Duration, u64) =
            match self.domain {
                Domain::Microphone if self.purpose == P::WakeDetection => {
                    (true, &[D::AmbientAudio], 0, MAX_CAPTURED_TURN, 0)
                }
                Domain::Microphone => (
                    matches!(self.purpose, P::ManualRecording | P::ConversationCapture),
                    &[D::PostCueAudio],
                    0,
                    MAX_CAPTURED_TURN,
                    0,
                ),
                Domain::BackgroundWake => (
                    self.purpose == P::WakeDetection,
                    &[D::AmbientAudio],
                    0,
                    WAKE_RING_BUFFER,
                    0,
                ),
                Domain::CloudTranscription => (
                    self.purpose == P::Transcription,
                    &[D::PostCueAudio],
                    MAX_PROVIDER_REQUESTS_PER_ROLE_PER_INTERACTION,
                    MAX_CLOUD_AUDIO,
                    0,
                ),
                Domain::Conversation => (
                    self.purpose == P::OrdinaryConversation,
                    &[D::OrdinaryText, D::OrdinaryHistory, D::AssistantReply],
                    MAX_PROVIDER_REQUESTS_PER_ROLE_PER_INTERACTION,
                    Duration::ZERO,
                    MAX_CONVERSATION_INPUT_BYTES,
                ),
                Domain::PrivateContext => (
                    self.purpose == P::PrivateDisclosure,
                    &[D::PrivateSnapshot],
                    1,
                    Duration::ZERO,
                    MAX_CONVERSATION_INPUT_BYTES,
                ),
                Domain::CloudTts => (
                    self.purpose == P::SpeechOutput,
                    &[D::AssistantReply],
                    MAX_PROVIDER_REQUESTS_PER_ROLE_PER_INTERACTION,
                    Duration::ZERO,
                    MAX_SPEECH_JOB_TEXT_BYTES,
                ),
            };
        let local = matches!(self.domain, Domain::Microphone | Domain::BackgroundWake);
        let snapshot_valid = match (&self.scope.snapshot, self.domain) {
            (Some(snapshot), Domain::PrivateContext) => {
                !snapshot.text.is_empty()
                    && snapshot.text.len() as u64 <= self.limits.text_bytes
                    && hash(snapshot.text.as_bytes()) == snapshot.digest
            }
            (None, Domain::PrivateContext) | (Some(_), _) => false,
            (None, _) => true,
        };
        purpose_valid
            && snapshot_valid
            && !self.classes.is_empty()
            && self.classes.len() <= allowed.len()
            && self.classes.iter().all(|class| allowed.contains(class))
            && self
                .classes
                .iter()
                .enumerate()
                .all(|(i, class)| !self.classes[..i].contains(class))
            && (local == self.target.is_none())
            && self
                .target
                .is_none_or(|target| (self.domain == Domain::CloudTts) == target.voice.is_some())
            && !(self.purpose == P::ManualRecording && self.scope.background)
            && self.limits.lifetime > Duration::ZERO
            && self.limits.lifetime
                <= if self.local_wake() {
                    MAX_WAKE_ARMING
                } else {
                    MAX_CLOUD_INTERACTION
                }
            && self.limits.requests <= requests
            && (local || self.limits.requests > 0)
            && self.limits.audio <= audio
            && self.limits.text_bytes <= text
    }
}

/// Private native configuration, not a deserializable renderer input. The
/// registry owns an immutable copy; fixtures are the only configuration source.
#[derive(Clone)]
struct TrustedConfiguration {
    revision_sequence: NonZeroU64,
    policy_revision: PolicyRevision,
    specifications: [Specification; 6],
}
impl TrustedConfiguration {
    fn valid_for(&self, context: &Context) -> bool {
        ConfigurationRevision::from_native_sequence(self.revision_sequence.get()).ok()
            == Some(context.identity.configuration_revision)
            && self.policy_revision == context.policy_revision
            && self
                .specifications
                .iter()
                .enumerate()
                .all(|(i, spec)| spec.domain.index() == i && spec.policy_valid())
    }
}

#[derive(Clone, PartialEq, Eq, Serialize)]
struct ParentBinding {
    id: GrantId,
    preview_digest: [u8; 32],
}

#[derive(Clone, PartialEq, Eq, Serialize)]
struct PreviewContent {
    version: u8,
    id: PreviewId,
    context: Context,
    generation: Generation,
    specification: Specification,
    parent: Option<ParentBinding>,
    prepared_at: Duration,
    confirmation_expires_at: Duration,
    grant_expires_at: Duration,
    disclosures: &'static str,
}

fn hash(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}
fn digest(content: &PreviewContent) -> Result<[u8; 32]> {
    serde_json::to_vec(content)
        .map(|bytes| hash(&bytes))
        .map_err(|_| Error::Encoding)
}

struct PreviewData {
    content: PreviewContent,
    digest: [u8; 32],
    native_instance: Arc<()>,
    confirmation_deadline: Deadline,
    grant_deadline: Deadline,
    prepared_at: MonotonicTime,
    global: Arc<AtomicBool>,
    domain: Arc<AtomicBool>,
    parent: Option<Grant>,
}

#[derive(Clone)]
struct Preview {
    data: Arc<PreviewData>,
}
impl Preview {
    fn content(&self) -> &PreviewContent {
        &self.data.content
    }
}

struct GrantData {
    id: GrantId,
    preview: Arc<PreviewData>,
    revoked: AtomicBool,
    parent: Option<Grant>,
}

/// Immutable, opaque, non-deserializable native handle. Clone cannot renew it.
#[derive(Clone)]
struct Grant {
    data: Arc<GrantData>,
}
impl std::fmt::Debug for Grant {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Grant(<native>)")
    }
}

/// This entire module is registered only under cfg(test). Receipt fields and
/// construction stay inside the trusted-presenter submodule, beyond the registry.
mod presenter {
    use super::*;
    pub(super) struct TestPresenter {
        identity: Arc<()>,
    }
    pub(super) struct Presentation {
        preview: Arc<PreviewData>,
        shown_digest: [u8; 32],
        presenter: Arc<()>,
    }
    pub(super) enum NativeInteraction {
        Affirm,
        Decline,
    }
    pub(super) struct Receipt {
        presentation: Arc<Presentation>,
        context: Context,
        interaction: NativeInteraction,
        observed_at: MonotonicTime,
        signer: Arc<()>,
    }
    impl TestPresenter {
        pub(super) fn new() -> Self {
            Self {
                identity: Arc::new(()),
            }
        }
        pub(super) fn key(&self) -> Arc<()> {
            self.identity.clone()
        }
        pub(super) fn show(&self, preview: &Preview) -> Arc<Presentation> {
            Arc::new(Presentation {
                preview: preview.data.clone(),
                shown_digest: preview.data.digest,
                presenter: self.identity.clone(),
            })
        }
        pub(super) fn interact(
            &self,
            presentation: &Arc<Presentation>,
            interaction: NativeInteraction,
            context: &Context,
            now: &MonotonicTime,
        ) -> Receipt {
            // The fake native event carries the active native OS/run context.
            // There is no renderer boolean, transcript or digest-based constructor.
            Receipt {
                presentation: presentation.clone(),
                context: context.clone(),
                interaction,
                observed_at: now.clone(),
                signer: self.identity.clone(),
            }
        }
    }
    impl Receipt {
        pub(super) fn verify(
            &self,
            key: &Arc<()>,
            preview: &Arc<PreviewData>,
            context: &Context,
            now: &MonotonicTime,
        ) -> Result<()> {
            if !Arc::ptr_eq(&self.signer, key)
                || !Arc::ptr_eq(&self.presentation.presenter, key)
                || !Arc::ptr_eq(&self.presentation.preview, preview)
                || self.presentation.shown_digest != preview.digest
                || &self.context != context
            {
                return Err(Error::Binding);
            }
            self.observed_at
                .checked_duration_since(&preview.prepared_at)?;
            now.checked_duration_since(&self.observed_at)?;
            if !matches!(self.interaction, NativeInteraction::Affirm) {
                return Err(Error::Denied);
            }
            Ok(())
        }
    }
}

enum Confirmation<'a> {
    Native(&'a presenter::Receipt),
    Untrusted,
}

struct UseRequest<'a> {
    context: Context,
    domain: Domain,
    purpose: Purpose,
    target: Option<Target>,
    data: DataClass,
    provenance: Provenance,
    background: bool,
    project: Option<ProjectId>,
    reviewed_snapshot: Option<&'a str>,
    requests: u64,
    audio: Duration,
    text_bytes: u64,
}

struct ConsentRegistry {
    native_instance: Arc<()>,
    presenter: Arc<()>,
    context: Context,
    configuration: Arc<TrustedConfiguration>,
    origin: MonotonicTime,
    last_time: MonotonicTime,
    interaction_deadline: Deadline,
    wake_deadline: Deadline,
    generation: Generation,
    global: Arc<AtomicBool>,
    domains: [Arc<AtomicBool>; 6],
    preview_sequence: u64,
    grant_sequence: u64,
    pending: Option<Arc<PreviewData>>,
    closed: bool,
}
impl ConsentRegistry {
    fn new(
        configuration: TrustedConfiguration,
        context: Context,
        presenter: &presenter::TestPresenter,
        now: MonotonicTime,
    ) -> Result<Self> {
        if !configuration.valid_for(&context) {
            return Err(Error::InvalidPolicy);
        }
        Ok(Self {
            native_instance: Arc::new(()),
            presenter: presenter.key(),
            context,
            configuration: Arc::new(configuration),
            origin: now.clone(),
            last_time: now.clone(),
            interaction_deadline: Deadline::after(now.clone(), MAX_CLOUD_INTERACTION)?,
            wake_deadline: Deadline::after(now, MAX_WAKE_ARMING)?,
            generation: Generation::from_native_sequence(1)?,
            global: Arc::new(AtomicBool::new(true)),
            domains: std::array::from_fn(|_| Arc::new(AtomicBool::new(true))),
            preview_sequence: 0,
            grant_sequence: 0,
            pending: None,
            closed: false,
        })
    }

    fn observe(&mut self, now: &MonotonicTime) -> Result<()> {
        now.checked_duration_since(&self.last_time)?;
        self.last_time = now.clone();
        Ok(())
    }

    fn lineage_valid(&self, grant: &Grant, now: &MonotonicTime) -> Result<()> {
        let mut node = Some(grant);
        while let Some(grant) = node {
            let data = &grant.data;
            let preview = &data.preview;
            if self.closed
                || !Arc::ptr_eq(&preview.native_instance, &self.native_instance)
                || data.revoked.load(Ordering::Acquire)
                || !preview.global.load(Ordering::Acquire)
                || !preview.domain.load(Ordering::Acquire)
                || preview.content.generation != self.generation
            {
                return Err(Error::Revoked);
            }
            if preview.content.context != self.context {
                return Err(Error::Binding);
            }
            if preview.grant_deadline.is_expired(now)? {
                return Err(Error::Expired);
            }
            // Parent nodes are immutable, issued earlier and never caller-writable;
            // there is no cyclic or automatic authority derivation path.
            node = data.parent.as_ref();
        }
        Ok(())
    }

    fn prepare(
        &mut self,
        domain: Domain,
        parent: Option<&Grant>,
        now: &MonotonicTime,
    ) -> Result<Preview> {
        if self.closed {
            return Err(Error::Closed);
        }
        self.observe(now)?;
        // Expiry is evaluated on explicit calls; no timer or background task.
        if let Some(pending) = &self.pending {
            let preview_expired = pending.confirmation_deadline.is_expired(now)?;
            let grant_expired = pending.grant_deadline.is_expired(now)?;
            if preview_expired || grant_expired {
                self.pending = None;
            } else {
                return Err(Error::Busy);
            }
        }
        if let Some(parent) = parent {
            self.lineage_valid(parent, now)?;
        }
        let spec = self.configuration.specifications[domain.index()].clone();
        let root = if spec.local_wake() {
            &self.wake_deadline
        } else {
            &self.interaction_deadline
        };
        if root.is_expired(now)? {
            return Err(Error::Expired);
        }
        let mut lifetime = root.remaining(now)?.min(spec.limits.lifetime);
        if let Some(parent) = parent {
            lifetime = lifetime.min(parent.data.preview.grant_deadline.remaining(now)?);
        }
        let grant_deadline = Deadline::after(now.clone(), lifetime)?;
        let confirmation_deadline = Deadline::after(now.clone(), PREVIEW_LIFETIME)?;
        let sequence = self
            .preview_sequence
            .checked_add(1)
            .ok_or(ContractError::Overflow)?;
        let id = PreviewId::new(sequence)?;
        let prepared_at = now.checked_duration_since(&self.origin)?;
        let content = PreviewContent {
            version: 1,
            id,
            context: self.context.clone(),
            generation: self.generation,
            specification: spec,
            parent: parent.map(|g| ParentBinding {
                id: g.data.id,
                preview_digest: g.data.preview.digest,
            }),
            prepared_at,
            confirmation_expires_at: prepared_at
                .checked_add(PREVIEW_LIFETIME)
                .ok_or(ContractError::Overflow)?,
            grant_expires_at: prepared_at
                .checked_add(lifetime)
                .ok_or(ContractError::Overflow)?,
            disclosures: DISCLOSURES,
        };
        let data = Arc::new(PreviewData {
            digest: digest(&content)?,
            content,
            native_instance: self.native_instance.clone(),
            confirmation_deadline,
            grant_deadline,
            prepared_at: now.clone(),
            global: self.global.clone(),
            domain: self.domains[domain.index()].clone(),
            parent: parent.cloned(),
        });
        self.preview_sequence = sequence;
        self.pending = Some(data.clone());
        Ok(Preview { data })
    }

    fn confirm(
        &mut self,
        preview: &Preview,
        proof: Confirmation<'_>,
        current: &Context,
        now: &MonotonicTime,
    ) -> Result<Grant> {
        // Select the referenced local preview first. A late attempt for a different,
        // already consumed preview must not consume a newer native presentation.
        let pending = self.pending.as_ref().ok_or(Error::Stale)?;
        if preview.data.content.id != pending.content.id
            || !Arc::ptr_eq(&preview.data.native_instance, &self.native_instance)
        {
            return Err(Error::Stale);
        }
        // Consume before checking proof, context or expiry, including rejected
        // attempts and reconstructed handles claiming this local preview ID.
        let pending = self.pending.take().unwrap();
        if self.closed {
            return Err(Error::Closed);
        }
        self.observe(now)?;
        if !Arc::ptr_eq(&preview.data, &pending)
            || current != &self.context
            || pending.content.context != self.context
            || pending.content.generation != self.generation
            || digest(&pending.content)? != pending.digest
            || pending.content.specification
                != self.configuration.specifications[pending.content.specification.domain.index()]
        {
            return Err(Error::Binding);
        }
        let preview_expired = pending.confirmation_deadline.is_expired(now)?;
        let grant_expired = pending.grant_deadline.is_expired(now)?;
        if preview_expired || grant_expired {
            return Err(Error::Expired);
        }
        if !pending.global.load(Ordering::Acquire) || !pending.domain.load(Ordering::Acquire) {
            return Err(Error::Revoked);
        }
        if let Some(parent) = &pending.parent {
            self.lineage_valid(parent, now)?;
        }
        match proof {
            Confirmation::Native(receipt) => {
                receipt.verify(&self.presenter, &pending, current, now)?
            }
            Confirmation::Untrusted => return Err(Error::UntrustedProof),
        }
        let sequence = self
            .grant_sequence
            .checked_add(1)
            .ok_or(ContractError::Overflow)?;
        let grant = Grant {
            data: Arc::new(GrantData {
                id: GrantId::new(sequence)?,
                parent: pending.parent.clone(),
                preview: pending,
                revoked: AtomicBool::new(false),
            }),
        };
        self.grant_sequence = sequence;
        Ok(grant)
    }

    /// Eligibility only. This does not reserve cumulative budgets, read private
    /// context, create an egress job or dispatch anything; those gates stay external.
    fn validate(
        &mut self,
        grant: &Grant,
        request: &UseRequest<'_>,
        now: &MonotonicTime,
    ) -> Result<()> {
        self.observe(now)?;
        self.lineage_valid(grant, now)?;
        let spec = &grant.data.preview.content.specification;
        if request.context != self.context
            || request.domain != spec.domain
            || request.purpose != spec.purpose
            || request.target != spec.target
            || (request.background && !spec.scope.background)
            || request.project != spec.scope.project
            || !spec.classes.contains(&request.data)
        {
            return Err(Error::Binding);
        }
        let provenance_valid = match (spec.domain, request.data, request.provenance) {
            (Domain::Microphone, DataClass::AmbientAudio, Provenance::Ambient) => {
                spec.purpose == Purpose::WakeDetection
            }
            (
                Domain::Microphone | Domain::CloudTranscription,
                DataClass::PostCueAudio,
                Provenance::PostCue,
            ) => spec.purpose != Purpose::WakeDetection,
            (Domain::BackgroundWake, DataClass::AmbientAudio, Provenance::Ambient) => true,
            (
                Domain::Conversation,
                DataClass::OrdinaryText | DataClass::OrdinaryHistory | DataClass::AssistantReply,
                Provenance::Ordinary,
            ) => true,
            (Domain::CloudTts, DataClass::AssistantReply, Provenance::Ordinary) => true,
            (
                Domain::PrivateContext,
                DataClass::PrivateSnapshot,
                Provenance::Private | Provenance::DerivedPrivate,
            ) => true,
            _ => false,
        };
        if !provenance_valid {
            return Err(Error::Denied);
        }
        match (&spec.scope.snapshot, request.reviewed_snapshot) {
            (Some(expected), Some(actual))
                if expected.text == actual
                    && expected.digest == hash(actual.as_bytes())
                    && request.text_bytes == actual.len() as u64 =>
            {
                ()
            }
            (None, None) => (),
            _ => return Err(Error::Binding),
        }
        if request.requests > spec.limits.requests
            || request.audio > spec.limits.audio
            || request.text_bytes > spec.limits.text_bytes
        {
            return Err(ContractError::LimitExceeded.into());
        }
        Ok(())
    }

    fn revoke(&mut self, grant: &Grant) -> Result<()> {
        if !Arc::ptr_eq(&grant.data.preview.native_instance, &self.native_instance) {
            return Err(Error::Binding);
        }
        grant.data.revoked.store(true, Ordering::Release);
        Ok(())
    }

    fn revoke_domain(&mut self, domain: Domain) {
        self.domains[domain.index()].store(false, Ordering::Release);
        self.domains[domain.index()] = Arc::new(AtomicBool::new(true));
        if self
            .pending
            .as_ref()
            .is_some_and(|p| p.content.specification.domain == domain)
        {
            self.pending = None;
        }
    }

    fn revoke_all(&mut self) -> Result<()> {
        self.global.store(false, Ordering::Release);
        self.pending = None;
        if self.closed {
            return Ok(());
        }
        match self.generation.checked_next() {
            Ok(generation) => {
                self.generation = generation;
                self.global = Arc::new(AtomicBool::new(true));
                Ok(())
            }
            Err(error) => {
                self.closed = true;
                Err(error.into())
            }
        }
    }

    fn update_configuration(
        &mut self,
        configuration: TrustedConfiguration,
        context: Context,
    ) -> Result<()> {
        if self.closed {
            return Err(Error::Closed);
        }
        self.revoke_all()?; // all old authority is gone before replacing configuration
        let old = &self.context;
        let revisions_valid = configuration.revision_sequence
            >= self.configuration.revision_sequence
            && configuration.policy_revision >= self.configuration.policy_revision
            && (configuration.revision_sequence > self.configuration.revision_sequence
                || configuration.policy_revision > self.configuration.policy_revision);
        if !revisions_valid
            || !configuration.valid_for(&context)
            || context.identity.app_run_id != old.identity.app_run_id
            || context.os_session != old.os_session
            || context.identity.interaction_id != old.identity.interaction_id
        {
            self.closed = true;
            self.global.store(false, Ordering::Release);
            return Err(Error::Binding);
        }
        self.configuration = Arc::new(configuration);
        self.context = context;
        // Root deadlines remain unchanged across revisions and child restarts.
        Ok(())
    }
}

#[cfg(test)]
#[path = "consent_tests.rs"]
mod tests;

// P1-07's minimal test-only integration bridge: a child can call the private
// registry without exporting grant constructors or changing consent semantics.
#[cfg(test)]
#[path = "egress.rs"]
mod egress;
