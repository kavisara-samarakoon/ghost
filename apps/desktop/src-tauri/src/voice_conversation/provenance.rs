//! P1-08: bounded, ephemeral native history/provenance contracts, test-only.
//! Synthetic sources only; no storage, IPC, adapters, capture or consent issuer.
//! Eligibility and review candidates are data, never permission to send or act.
use super::clock::{Deadline, MonotonicTime};
use super::limits::*;
use super::model::*;
use super::ContractError;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::num::NonZeroU64;
use std::sync::Arc;
use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Error {
    Contract(ContractError),
    Binding,
    Stale,
    Closed,
    Expired,
    Busy,
    Denied,
    UntrustedSource,
    PrivateReviewRequired,
    SharingPaused,
    Encoding,
}
impl From<ContractError> for Error {
    fn from(value: ContractError) -> Self {
        Self::Contract(value)
    }
}
type Result<T> = std::result::Result<T, Error>;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
enum Origin {
    TypedUserText,
    PostCueTranscript,
    AssistantReply,
    PersonalMemory,
    ProjectMemory,
    Google,
    PlanningResult,
    ToolResult,
    WorkflowResult,
    HostStatus,
    AmbientAudio,
    PostCueAudio,
    Summary,
    Quotation,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
enum Role {
    User,
    Assistant,
    Context,
    Audio,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
enum RecordOutcome {
    NativeSourceData,
    UntrustedAdvisory,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
enum ContentClass {
    OrdinaryUserText,
    OrdinaryHistory,
    AssistantReply,
    PrivateLocal,
    PrivateDerived,
    GoogleDerived,
    ActionDerived,
    AmbientAudio,
    PostCueAudio,
    DataOnly,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
enum Transform {
    Transcript,
    AssistantReply,
    Summary,
    Quotation,
}
#[derive(Clone, Copy)]
enum SessionEnd {
    Stop,
    ConsentRevoked,
    SessionTerminated,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
enum Purpose {
    Conversation,
    CloudTts,
    PrivateDisclosure,
    Planning,
    Tools,
    Workflow,
    CloudStt,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
struct Recipient {
    provider: NonZeroU64,
    model: NonZeroU64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Restrictions {
    private: bool,
    google: bool,
    action: bool,
    ambient: bool,
}
impl Restrictions {
    fn for_origin(origin: Origin) -> Self {
        Self {
            private: matches!(origin, Origin::PersonalMemory | Origin::ProjectMemory),
            google: origin == Origin::Google,
            action: matches!(
                origin,
                Origin::PlanningResult | Origin::ToolResult | Origin::WorkflowResult
            ),
            ambient: origin == Origin::AmbientAudio,
        }
    }
    fn union(self, other: Self) -> Self {
        Self {
            private: self.private || other.private,
            google: self.google || other.google,
            action: self.action || other.action,
            ambient: self.ambient || other.ambient,
        }
    }
    fn forbidden(self) -> bool {
        self.google || self.action || self.ambient
    }
}

// Handles carry no content. Copies cannot retain text after native clear/expiry,
// forge another source, change identities or become deserializable capabilities.
#[derive(Clone)]
struct Turn(Arc<TurnData>);
struct TurnData {
    owner: Arc<()>,
    epoch: Arc<()>,
    identity: ContractIdentity,
}
#[derive(Clone)]
pub(super) struct Record(Arc<()>);
struct Entry {
    handle: Record,
    sequence: u64,
    identity: ContractIdentity,
    origin: Origin,
    role: Role,
    outcome: RecordOutcome,
    restrictions: Restrictions,
    sources: Vec<u64>,
    deadline: Deadline,
    text: Option<String>,
    valid: bool,
}
struct Pair {
    user: u64,
    reply: u64,
}
struct Derivation(Arc<()>);
struct Pending {
    handle: Arc<()>,
    turn: Turn,
    revision: u64,
    kind: Transform,
    sources: Vec<u64>,
    restrictions: Restrictions,
    deadline: Deadline,
}

/// Describes the independent gate still required. There is deliberately no
/// "authorized" variant and no grant, one-shot budget, job or executor here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ConsentRequirement {
    OrdinaryConversationGrant,
    CloudTtsGrant,
    ExactPrivateReview,
}
pub(super) struct Snapshot {
    owner: Arc<()>,
    epoch: Arc<()>,
    turn: Turn,
    revision: u64,
    recipient: Recipient,
    purpose: Purpose,
    requirement: ConsentRequirement,
    digest: [u8; 32],
    selected_reply: Option<u64>,
}
pub(super) struct ValidatedInput<'a> {
    // A view borrows native state. Clear/revocation cannot race a live view.
    _history: &'a History,
    body: String,
    requirement: ConsentRequirement,
    digest: [u8; 32],
}
impl ValidatedInput<'_> {
    pub(super) fn body(&self) -> &str {
        &self.body
    }
    pub(super) fn requirement(&self) -> ConsentRequirement {
        self.requirement
    }
    pub(super) fn digest(&self) -> [u8; 32] {
        self.digest
    }
}

/// One native session. Caller supplies its existing interaction/grant retention
/// deadline; this module cannot renew that authority or allocate child sessions.
/// The history arena includes pending/context text in its 64 KiB allowance.
/// Sixteen text slots derive from the approved eight-pair capacity. Audio
/// metadata has one slot per origin/turn and never contains raw frames.
pub(super) struct History {
    owner: Arc<()>,
    epoch: Arc<()>,
    identity: ContractIdentity,
    recipient: Recipient,
    project: Option<NonZeroU64>,
    deadline: Deadline,
    idle: Deadline,
    last_time: MonotonicTime,
    closed: bool,
    sharing_paused: bool,
    sequence: u64,
    revision: u64,
    turn_count: u64,
    seen_turns: Vec<TurnId>,
    active: Option<Turn>,
    entries: Vec<Entry>,
    pairs: Vec<Pair>,
    bytes: u64,
    // Once restricted data enters this epoch, dropping/revoking a source cannot
    // make subsequent automatic replies ordinary. Only explicit Clear resets it.
    inherited: Restrictions,
    pending: Option<Pending>,
}
impl History {
    fn new(
        identity: ContractIdentity,
        recipient: Recipient,
        project: Option<NonZeroU64>,
        retention: Deadline,
        now: MonotonicTime,
    ) -> Result<Self> {
        let remaining = retention.remaining(&now)?;
        if remaining.is_zero() {
            return Err(Error::Expired);
        }
        let lifetime = remaining
            .min(MAX_CLOUD_INTERACTION)
            .min(MAX_CHILD_CONVERSATION);
        Ok(Self {
            owner: Arc::new(()),
            epoch: Arc::new(()),
            identity,
            recipient,
            project,
            deadline: Deadline::after(now.clone(), lifetime)?,
            idle: Deadline::after(now.clone(), IDLE_CHILD_TIMEOUT)?,
            last_time: now,
            closed: false,
            sharing_paused: false,
            sequence: 0,
            revision: 0,
            turn_count: 0,
            seen_turns: Vec::new(),
            active: None,
            entries: Vec::new(),
            pairs: Vec::new(),
            bytes: 0,
            inherited: Restrictions::default(),
            pending: None,
        })
    }
    fn scope_matches(&self, identity: &ContractIdentity) -> bool {
        let mut expected = self.identity;
        expected.turn_id = identity.turn_id;
        expected.job_id = identity.job_id;
        expected == *identity
    }
    fn observe(&mut self, now: &MonotonicTime) -> Result<()> {
        if self.closed {
            return Err(Error::Closed);
        }
        now.checked_duration_since(&self.last_time)?;
        self.last_time = now.clone();
        let expired = self.deadline.is_expired(now)?;
        let idle = self.idle.is_expired(now)? && self.active.is_none();
        if expired || idle {
            self.stop();
            return Err(Error::Expired);
        }
        // Expiry removes text and transitively invalidates every influenced item.
        let expired: Vec<_> = self
            .entries
            .iter()
            .filter_map(|entry| {
                if entry.valid {
                    match entry.deadline.is_expired(now) {
                        Ok(true) => Some(Ok(entry.sequence)),
                        Ok(false) => None,
                        Err(e) => Some(Err(e)),
                    }
                } else {
                    None
                }
            })
            .collect::<std::result::Result<_, _>>()?;
        for sequence in expired {
            self.invalidate(sequence);
        }
        Ok(())
    }
    fn check_turn(&self, turn: &Turn) -> Result<()> {
        if !Arc::ptr_eq(&turn.0.owner, &self.owner)
            || !Arc::ptr_eq(&turn.0.epoch, &self.epoch)
            || !self.scope_matches(&turn.0.identity)
            || !self
                .active
                .as_ref()
                .is_some_and(|active| Arc::ptr_eq(&active.0, &turn.0))
        {
            return Err(Error::Binding);
        }
        Ok(())
    }
    fn begin_turn(&mut self, identity: ContractIdentity, now: &MonotonicTime) -> Result<Turn> {
        self.observe(now)?;
        if !self.scope_matches(&identity) {
            return Err(Error::Binding);
        }
        if self.active.is_some() {
            return Err(Error::Busy);
        }
        if self.seen_turns.contains(&identity.turn_id) {
            return Err(Error::Stale);
        }
        let count = checked_total(self.turn_count, 1, MAX_CHILD_TURNS)?;
        let turn = Turn(Arc::new(TurnData {
            owner: self.owner.clone(),
            epoch: self.epoch.clone(),
            identity,
        }));
        self.turn_count = count;
        self.seen_turns.push(identity.turn_id);
        self.active = Some(turn.clone());
        Ok(turn)
    }
    fn slot(&self, record: &Record) -> Result<usize> {
        self.entries
            .iter()
            .position(|e| Arc::ptr_eq(&e.handle.0, &record.0) && e.valid)
            .ok_or(Error::Stale)
    }
    fn next_revision(&self) -> Result<u64> {
        self.revision
            .checked_add(1)
            .ok_or(ContractError::Overflow.into())
    }
    fn insert(
        &mut self,
        turn: &Turn,
        origin: Origin,
        role: Role,
        text: Option<&str>,
        restrictions: Restrictions,
        sources: Vec<u64>,
        deadline: Deadline,
    ) -> Result<Record> {
        self.check_turn(turn)?;
        let amount =
            u64::try_from(text.map_or(0, str::len)).map_err(|_| ContractError::Overflow)?;
        let limit = match role {
            Role::User => MAX_USER_TRANSCRIPT_BYTES,
            Role::Assistant => MAX_ASSISTANT_REPLY_BYTES,
            _ => MAX_CONVERSATION_INPUT_BYTES,
        };
        checked_total(0, amount, limit)?;
        if text.is_some_and(str::is_empty) {
            return Err(Error::Denied);
        }
        let bytes = match checked_total(self.bytes, amount, MAX_HISTORY_BYTES) {
            Ok(bytes) => bytes,
            Err(e) => {
                self.sharing_paused = true;
                return Err(e.into());
            }
        };
        if role == Role::Audio
            && self.entries.iter().any(|e| {
                e.role == Role::Audio && e.identity == turn.0.identity && e.origin == origin
            })
        {
            return Err(Error::Stale);
        }
        if role != Role::Audio
            && self
                .entries
                .iter()
                .filter(|e| e.role != Role::Audio)
                .count()
                >= MAX_HISTORY_PAIRS as usize * 2
        {
            self.sharing_paused = true;
            return Err(ContractError::LimitExceeded.into());
        }
        let sequence = self
            .sequence
            .checked_add(1)
            .ok_or(ContractError::Overflow)?;
        let revision = self.next_revision()?;
        let handle = Record(Arc::new(()));
        self.entries.push(Entry {
            handle: handle.clone(),
            sequence,
            identity: turn.0.identity,
            origin,
            role,
            outcome: if sources.is_empty() {
                RecordOutcome::NativeSourceData
            } else {
                RecordOutcome::UntrustedAdvisory
            },
            restrictions,
            sources,
            deadline,
            text: text.map(str::to_owned),
            valid: true,
        });
        self.bytes = bytes;
        self.sequence = sequence;
        self.revision = revision;
        self.inherited = self.inherited.union(restrictions);
        Ok(handle)
    }
    fn accept_source(
        &mut self,
        turn: &Turn,
        source: fixtures::Source,
        now: &MonotonicTime,
    ) -> Result<Record> {
        self.observe(now)?;
        self.check_turn(turn)?;
        if !source.matches(turn, self.project) {
            return Err(Error::Binding);
        }
        if source.deadline().is_expired(now)? {
            return Err(Error::Expired);
        }
        let lifetime = source
            .deadline()
            .remaining(now)?
            .min(self.deadline.remaining(now)?);
        let deadline = Deadline::after(now.clone(), lifetime)?;
        self.insert(
            turn,
            source.origin(),
            source.role(),
            source.text(),
            Restrictions::for_origin(source.origin()),
            Vec::new(),
            deadline,
        )
    }
    fn record_text(
        &mut self,
        record: &Record,
        expected: ContractIdentity,
        now: &MonotonicTime,
    ) -> Result<&str> {
        self.observe(now)?;
        let slot = self.slot(record)?;
        if self.entries[slot].identity != expected {
            return Err(Error::Binding);
        }
        self.entries[slot].text.as_deref().ok_or(Error::Denied)
    }
    /// Native code snapshots ALL retained textual influences. A provider cannot
    /// choose a smaller source list, claim an origin or provide a provenance flag.
    fn prepare_transform(
        &mut self,
        turn: &Turn,
        kind: Transform,
        now: &MonotonicTime,
    ) -> Result<Derivation> {
        self.observe(now)?;
        self.check_turn(turn)?;
        if self.pending.is_some() {
            return Err(Error::Busy);
        }
        let mut sources = Vec::new();
        let mut restrictions = self.inherited;
        let mut lifetime = self.deadline.remaining(now)?;
        for entry in self.entries.iter().filter(|e| e.valid) {
            let audio = kind == Transform::Transcript
                && entry.origin == Origin::PostCueAudio
                && entry.identity == turn.0.identity;
            if entry.text.is_some() || audio {
                sources.push(entry.sequence);
                restrictions = restrictions.union(entry.restrictions);
                lifetime = lifetime.min(entry.deadline.remaining(now)?);
            }
        }
        if sources.is_empty()
            || (kind == Transform::Transcript
                && !self.entries.iter().any(|e| {
                    e.valid && e.identity == turn.0.identity && e.origin == Origin::PostCueAudio
                }))
            || (kind == Transform::Transcript && restrictions.ambient)
        {
            return Err(Error::Denied);
        }
        let handle = Arc::new(());
        self.pending = Some(Pending {
            handle: handle.clone(),
            turn: turn.clone(),
            revision: self.revision,
            kind,
            sources,
            restrictions,
            deadline: Deadline::after(now.clone(), lifetime)?,
        });
        Ok(Derivation(handle))
    }
    fn complete_transform(
        &mut self,
        derivation: &Derivation,
        output: &str,
        now: &MonotonicTime,
    ) -> Result<Record> {
        self.observe(now)?;
        let pending = self.pending.as_ref().ok_or(Error::Stale)?;
        if !Arc::ptr_eq(&pending.handle, &derivation.0) {
            return Err(Error::Stale);
        }
        // One-shot native callback; even rejected output cannot replay the ticket.
        let pending = self.pending.take().unwrap();
        self.check_turn(&pending.turn)?;
        if pending.revision != self.revision
            || pending
                .sources
                .iter()
                .any(|id| !self.entries.iter().any(|e| e.valid && e.sequence == *id))
        {
            return Err(Error::Stale);
        }
        if pending.deadline.is_expired(now)? {
            return Err(Error::Expired);
        }
        let (origin, role) = match pending.kind {
            Transform::Transcript => (Origin::PostCueTranscript, Role::User),
            Transform::AssistantReply => (Origin::AssistantReply, Role::Assistant),
            Transform::Summary => (Origin::Summary, Role::Context),
            Transform::Quotation => (Origin::Quotation, Role::Context),
        };
        self.insert(
            &pending.turn,
            origin,
            role,
            Some(output),
            pending.restrictions.union(self.inherited),
            pending.sources,
            pending.deadline,
        )
    }
    fn commit_pair(
        &mut self,
        turn: &Turn,
        user: &Record,
        reply: &Record,
        now: &MonotonicTime,
    ) -> Result<()> {
        self.observe(now)?;
        self.check_turn(turn)?;
        let user = &self.entries[self.slot(user)?];
        let reply = &self.entries[self.slot(reply)?];
        if user.role != Role::User
            || reply.role != Role::Assistant
            || user.identity != turn.0.identity
            || reply.identity != turn.0.identity
            || !reply.sources.contains(&user.sequence)
            || self
                .pairs
                .iter()
                .any(|p| p.user == user.sequence || p.reply == reply.sequence)
        {
            return Err(Error::Binding);
        }
        checked_total(
            self.pairs
                .len()
                .try_into()
                .map_err(|_| ContractError::Overflow)?,
            1,
            MAX_HISTORY_PAIRS,
        )?;
        let revision = self.next_revision()?;
        let idle = Deadline::after(now.clone(), IDLE_CHILD_TIMEOUT)?;
        self.pairs.push(Pair {
            user: user.sequence,
            reply: reply.sequence,
        });
        self.revision = revision;
        self.active = None;
        self.pending = None;
        self.idle = idle;
        Ok(())
    }
    fn requirement(&self, purpose: Purpose) -> Result<ConsentRequirement> {
        if self.inherited.forbidden() {
            return Err(Error::Denied);
        }
        if self.inherited.private {
            return if purpose == Purpose::PrivateDisclosure {
                Ok(ConsentRequirement::ExactPrivateReview)
            } else {
                Err(Error::PrivateReviewRequired)
            };
        }
        match purpose {
            Purpose::Conversation => Ok(ConsentRequirement::OrdinaryConversationGrant),
            Purpose::CloudTts => Ok(ConsentRequirement::CloudTtsGrant),
            _ => Err(Error::Denied),
        }
    }
    fn content_class(&self, entry: &Entry) -> ContentClass {
        if entry.restrictions.google {
            ContentClass::GoogleDerived
        } else if entry.restrictions.ambient {
            ContentClass::AmbientAudio
        } else if entry.restrictions.action {
            ContentClass::ActionDerived
        } else if entry.restrictions.private {
            if matches!(entry.origin, Origin::PersonalMemory | Origin::ProjectMemory) {
                ContentClass::PrivateLocal
            } else {
                ContentClass::PrivateDerived
            }
        } else if entry.origin == Origin::PostCueAudio {
            ContentClass::PostCueAudio
        } else if entry.role == Role::Assistant {
            ContentClass::AssistantReply
        } else if self.pairs.iter().any(|p| p.user == entry.sequence)
            || matches!(entry.origin, Origin::Summary | Origin::Quotation)
        {
            ContentClass::OrdinaryHistory
        } else if entry.role == Role::User {
            ContentClass::OrdinaryUserText
        } else {
            ContentClass::DataOnly
        }
    }
    fn input_body(&self, purpose: Purpose, selected_reply: Option<u64>) -> Result<String> {
        // A broken pair is an explicit failure, never silent history truncation.
        if self.pairs.iter().any(|pair| {
            [pair.user, pair.reply]
                .iter()
                .any(|id| !self.entries.iter().any(|e| e.valid && e.sequence == *id))
        }) {
            return Err(Error::Stale);
        }
        let records: Vec<_> = self
            .entries
            .iter()
            .filter(|e| {
                e.valid && e.text.is_some() && selected_reply.is_none_or(|id| id == e.sequence)
            })
            .collect();
        if records.is_empty() {
            return Err(Error::Denied);
        }
        if purpose == Purpose::CloudTts
            && (selected_reply.is_none()
                || records.len() != 1
                || records[0].role != Role::Assistant)
        {
            return Err(Error::Denied);
        }
        let text: Vec<_> = records
            .iter()
            .map(|e| {
                (
                    e.sequence,
                    e.identity,
                    e.origin,
                    self.content_class(e),
                    e.role,
                    "data_only",
                    e.text.as_deref().unwrap(),
                    e.outcome,
                )
            })
            .collect();
        serde_json::to_string(&(ContractVersion::V1, self.recipient, purpose, text))
            .map_err(|_| Error::Encoding)
    }
    fn snapshot(
        &mut self,
        turn: &Turn,
        recipient: Recipient,
        purpose: Purpose,
        now: &MonotonicTime,
    ) -> Result<Snapshot> {
        self.prepare_snapshot(turn, recipient, purpose, None, now)
    }
    fn reply_snapshot(
        &mut self,
        turn: &Turn,
        reply: &Record,
        recipient: Recipient,
        now: &MonotonicTime,
    ) -> Result<Snapshot> {
        self.observe(now)?;
        self.check_turn(turn)?;
        let entry = &self.entries[self.slot(reply)?];
        if entry.identity != turn.0.identity || entry.role != Role::Assistant {
            return Err(Error::Binding);
        }
        self.prepare_snapshot(
            turn,
            recipient,
            Purpose::CloudTts,
            Some(entry.sequence),
            now,
        )
    }
    fn prepare_snapshot(
        &mut self,
        turn: &Turn,
        recipient: Recipient,
        purpose: Purpose,
        selected_reply: Option<u64>,
        now: &MonotonicTime,
    ) -> Result<Snapshot> {
        self.observe(now)?;
        self.check_turn(turn)?;
        if self.sharing_paused {
            return Err(Error::SharingPaused);
        }
        if recipient != self.recipient {
            return Err(Error::Binding);
        }
        let requirement = self.requirement(purpose)?;
        let body = self.input_body(purpose, selected_reply)?;
        if let Err(e) = checked_total(
            0,
            body.len().try_into().map_err(|_| ContractError::Overflow)?,
            MAX_CONVERSATION_INPUT_BYTES,
        ) {
            self.sharing_paused = true;
            return Err(e.into());
        }
        Ok(Snapshot {
            owner: self.owner.clone(),
            epoch: self.epoch.clone(),
            turn: turn.clone(),
            revision: self.revision,
            recipient,
            purpose,
            requirement,
            digest: Sha256::digest(body.as_bytes()).into(),
            selected_reply,
        })
    }
    /// Future egress must call this with the opaque native handle, then separately
    /// validate consent and reserve budgets. Copied metadata/digests are no proof.
    pub(super) fn validate_snapshot<'a>(
        &'a mut self,
        snapshot: &Snapshot,
        now: &MonotonicTime,
    ) -> Result<ValidatedInput<'a>> {
        self.observe(now)?;
        self.check_turn(&snapshot.turn)?;
        if self.sharing_paused {
            return Err(Error::SharingPaused);
        }
        if !Arc::ptr_eq(&snapshot.owner, &self.owner)
            || !Arc::ptr_eq(&snapshot.epoch, &self.epoch)
            || snapshot.revision != self.revision
            || snapshot.recipient != self.recipient
        {
            return Err(Error::Stale);
        }
        let requirement = self.requirement(snapshot.purpose)?;
        let body = self.input_body(snapshot.purpose, snapshot.selected_reply)?;
        let digest: [u8; 32] = Sha256::digest(body.as_bytes()).into();
        if requirement != snapshot.requirement || digest != snapshot.digest {
            return Err(Error::Stale);
        }
        checked_total(
            0,
            body.len().try_into().map_err(|_| ContractError::Overflow)?,
            MAX_CONVERSATION_INPUT_BYTES,
        )?;
        Ok(ValidatedInput {
            _history: self,
            body,
            requirement,
            digest,
        })
    }
    fn invalidate(&mut self, sequence: u64) {
        let mut invalid = vec![sequence];
        // Native insertion is a forward-only DAG: sixteen text nodes and at most
        // two audio metadata markers per each of the eight native turns.
        for entry in &mut self.entries {
            if entry.sequence == sequence || entry.sources.iter().any(|id| invalid.contains(id)) {
                entry.valid = false;
                entry.text = None;
                invalid.push(entry.sequence);
            }
        }
        self.pending = None;
        // Do not refund retained-history accounting or erase inherited restrictions.
    }
    fn revoke(&mut self, record: &Record) -> Result<()> {
        let slot = self.slot(record)?;
        self.invalidate(self.entries[slot].sequence);
        match self.next_revision() {
            Ok(next) => {
                self.revision = next;
                Ok(())
            }
            Err(e) => {
                self.stop();
                Err(e)
            }
        }
    }
    fn erase(&mut self) {
        self.entries.clear();
        self.pairs.clear();
        self.pending = None;
        self.active = None;
        self.bytes = 0;
        self.epoch = Arc::new(());
    }
    fn clear(&mut self, now: &MonotonicTime) -> Result<()> {
        self.observe(now)?;
        let next = self.next_revision();
        self.erase();
        self.inherited = Restrictions::default();
        self.sharing_paused = false;
        match next {
            Ok(next) => {
                self.revision = next;
                Ok(())
            }
            Err(e) => {
                self.closed = true;
                Err(e)
            }
        }
        // Turn tombstones, turn count and root/idle deadlines never reset.
    }
    fn cancel_turn(&mut self, turn: &Turn) -> Result<()> {
        self.check_turn(turn)?;
        let next = self.next_revision();
        // Conservative cancellation clears this epoch's data and pending work;
        // unlike explicit Clear it does not remove the inherited restrictions.
        self.erase();
        match next {
            Ok(next) => {
                self.revision = next;
                Ok(())
            }
            Err(e) => {
                self.closed = true;
                Err(e)
            }
        }
    }
    fn stop(&mut self) {
        self.closed = true;
        self.erase();
    }
    /// An explicit native lifecycle event, not a supervisor or automatic observer.
    /// Future integration must forward revocation before using another snapshot.
    fn end(&mut self, _reason: SessionEnd) {
        self.stop();
    }
}

/// Only synthetic native events issue source evidence. There is no live source
/// reader and no renderer/provider-to-provenance conversion in this phase.
mod fixtures {
    use super::*;
    pub(super) struct Source {
        turn: Turn,
        origin: Origin,
        role: Role,
        text: Option<String>,
        deadline: Deadline,
        project: Option<NonZeroU64>,
    }
    impl Source {
        pub(super) fn matches(&self, turn: &Turn, project: Option<NonZeroU64>) -> bool {
            Arc::ptr_eq(&self.turn.0, &turn.0) && self.project == project
        }
        pub(super) fn origin(&self) -> Origin {
            self.origin
        }
        pub(super) fn role(&self) -> Role {
            self.role
        }
        pub(super) fn text(&self) -> Option<&str> {
            self.text.as_deref()
        }
        pub(super) fn deadline(&self) -> &Deadline {
            &self.deadline
        }
    }
    // The caller is a synthetic trusted event, never an arbitrary origin claim.
    #[cfg(test)]
    pub(super) fn native_text(
        turn: &Turn,
        origin: Origin,
        text: &str,
        project: Option<NonZeroU64>,
        deadline: Deadline,
    ) -> Result<Source> {
        let role = match origin {
            Origin::TypedUserText => Role::User,
            Origin::PersonalMemory
            | Origin::ProjectMemory
            | Origin::Google
            | Origin::PlanningResult
            | Origin::ToolResult
            | Origin::WorkflowResult
            | Origin::HostStatus => Role::Context,
            _ => return Err(Error::UntrustedSource),
        };
        Ok(Source {
            turn: turn.clone(),
            origin,
            role,
            text: Some(text.into()),
            project,
            deadline,
        })
    }
    #[cfg(test)]
    pub(super) fn audio_marker(
        turn: &Turn,
        origin: Origin,
        duration: Duration,
        project: Option<NonZeroU64>,
        deadline: Deadline,
    ) -> Result<Source> {
        let maximum = match origin {
            Origin::AmbientAudio => WAKE_RING_BUFFER,
            Origin::PostCueAudio => MAX_CAPTURED_TURN,
            _ => return Err(Error::Denied),
        };
        if duration.is_zero() {
            return Err(Error::Denied);
        }
        checked_duration_total(Duration::ZERO, duration, maximum)?;
        Ok(Source {
            turn: turn.clone(),
            origin,
            role: Role::Audio,
            text: None,
            project,
            deadline,
        })
    }
    pub(super) fn from_untrusted(_text: &str, _claimed_origin: Origin) -> Result<Source> {
        Err(Error::UntrustedSource)
    }
}

#[cfg(test)]
#[path = "provenance_tests.rs"]
mod tests;
