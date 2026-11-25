//! Approved initial design bounds, not measured performance guarantees.
//! Cumulative reservations charge permanently, including when dropped unused.
//! Concurrent speech capacity is separate and released by matching native leases.
//! This conservative accounting is not the production egress gate.
use super::model::{
    ConversationProviderRole, InteractionId, JobId, SessionId, TranscriptionRoute, TurnId,
};
use super::{ContractError, Result};
use std::sync::Arc;
use std::time::Duration;

pub(super) const KIB: u64 = 1024;
pub(super) const MIB: u64 = 1024 * KIB;
pub(super) const MAX_WAKE_ARMING: Duration = Duration::from_secs(8 * 60 * 60);
pub(super) const MAX_CLOUD_INTERACTION: Duration = Duration::from_secs(15 * 60);
pub(super) const MAX_CHILD_CONVERSATION: Duration = Duration::from_secs(5 * 60);
pub(super) const MAX_CHILD_TURNS: u64 = 8;
pub(super) const MAX_CHILD_CONVERSATIONS: u64 = 4;
pub(super) const MAX_AGGREGATE_TURNS: u64 = 16;
pub(super) const IDLE_CHILD_TIMEOUT: Duration = Duration::from_secs(60);
pub(super) const SPEECH_ONSET_WAIT: Duration = Duration::from_secs(5);
pub(super) const FOLLOW_UP_WINDOW: Duration = Duration::from_secs(8);
pub(super) const MAX_CAPTURED_TURN: Duration = Duration::from_secs(30);
pub(super) const MAX_ENCODED_AUDIO_BYTES: u64 = 8 * MIB;
pub(super) const MAX_CLOUD_AUDIO: Duration = Duration::from_secs(120);
pub(super) const FINALIZATION_TIMEOUT: Duration = Duration::from_secs(5);
pub(super) const MAX_USER_TRANSCRIPT_BYTES: u64 = 8192;
pub(super) const MAX_ASSISTANT_REPLY_BYTES: u64 = 4096;
pub(super) const MAX_HISTORY_PAIRS: u64 = 8;
pub(super) const MAX_HISTORY_BYTES: u64 = 64 * KIB;
pub(super) const MAX_CONVERSATION_INPUT_BYTES: u64 = 32 * KIB;
pub(super) const MAX_PROVIDER_REQUEST_BYTES: u64 = 64 * KIB;
pub(super) const MAX_PROVIDER_RESPONSE_BYTES: u64 = 256 * KIB;
pub(super) const MAX_ACTIVE_SPEECH_JOBS: u64 = 1;
pub(super) const MAX_PENDING_SPEECH_JOBS: u64 = 1;
pub(super) const MAX_QUEUED_SPEECH_TEXT_BYTES: u64 = 8 * KIB;
pub(super) const MAX_SPEECH_JOB_TEXT_BYTES: u64 = 4 * KIB;
pub(super) const MAX_SPEECH_JOB_AUDIO_BYTES: u64 = 8 * MIB;
pub(super) const SPEECH_OUTPUT_DEADLINE: Duration = Duration::from_secs(90);
pub(super) const WAKE_RING_BUFFER: Duration = Duration::from_secs(2);
pub(super) const WAKE_COOLDOWN: Duration = Duration::from_secs(3);
pub(super) const MAX_PROVIDER_REQUESTS_PER_ROLE_PER_TURN: u64 = 1;
pub(super) const MAX_PROVIDER_REQUESTS_PER_ROLE_PER_INTERACTION: u64 = 16;

macro_rules! bounds {
    ($name:ident, $ty:ty, { $($variant:ident => $limit:ident),+ $(,)? }) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub(super) enum $name { $($variant),+ }
        impl $name {
            pub(super) const ALL: &'static [Self] = &[$(Self::$variant),+];
            pub(super) const fn maximum(self) -> $ty {
                match self { $(Self::$variant => $limit),+ }
            }
        }
    };
}

bounds!(BudgetLimit, u64, {
    ChildTurns => MAX_CHILD_TURNS,
    ChildConversations => MAX_CHILD_CONVERSATIONS,
    AggregateTurns => MAX_AGGREGATE_TURNS,
    EncodedAudioBytes => MAX_ENCODED_AUDIO_BYTES,
    UserTranscriptBytes => MAX_USER_TRANSCRIPT_BYTES,
    AssistantReplyBytes => MAX_ASSISTANT_REPLY_BYTES,
    HistoryPairs => MAX_HISTORY_PAIRS,
    HistoryBytes => MAX_HISTORY_BYTES,
    ConversationInputBytes => MAX_CONVERSATION_INPUT_BYTES,
    ProviderRequestBytes => MAX_PROVIDER_REQUEST_BYTES,
    ProviderResponseBytes => MAX_PROVIDER_RESPONSE_BYTES,
    SpeechJobTextBytes => MAX_SPEECH_JOB_TEXT_BYTES,
    SpeechJobAudioBytes => MAX_SPEECH_JOB_AUDIO_BYTES,
    ProviderRolePerTurn => MAX_PROVIDER_REQUESTS_PER_ROLE_PER_TURN,
    ProviderRolePerInteraction => MAX_PROVIDER_REQUESTS_PER_ROLE_PER_INTERACTION,
});

bounds!(DurationLimit, Duration, {
    WakeArming => MAX_WAKE_ARMING,
    CloudInteraction => MAX_CLOUD_INTERACTION,
    ChildConversation => MAX_CHILD_CONVERSATION,
    IdleChild => IDLE_CHILD_TIMEOUT,
    SpeechOnset => SPEECH_ONSET_WAIT,
    FollowUp => FOLLOW_UP_WINDOW,
    CapturedTurn => MAX_CAPTURED_TURN,
    CloudAudio => MAX_CLOUD_AUDIO,
    Finalization => FINALIZATION_TIMEOUT,
    SpeechOutput => SPEECH_OUTPUT_DEADLINE,
    WakeRing => WAKE_RING_BUFFER,
    WakeCooldown => WAKE_COOLDOWN,
});

pub(super) fn checked_total(used: u64, amount: u64, maximum: u64) -> Result<u64> {
    let next = used.checked_add(amount).ok_or(ContractError::Overflow)?;
    if next > maximum {
        return Err(ContractError::LimitExceeded);
    }
    Ok(next)
}

pub(super) fn checked_duration_total(
    used: Duration,
    amount: Duration,
    maximum: Duration,
) -> Result<Duration> {
    let next = used.checked_add(amount).ok_or(ContractError::Overflow)?;
    if next > maximum {
        return Err(ContractError::LimitExceeded);
    }
    Ok(next)
}

/// Inert bookkeeping, not an authorization token. Neither Clone nor Copy.
/// Consuming marks accounting only; it dispatches nothing and never refunds usage.
#[derive(Debug)]
pub(super) struct Reservation {
    consumed: bool,
}

impl Reservation {
    fn charged() -> Self {
        Self { consumed: false }
    }

    pub(super) fn consume(&mut self) -> Result<()> {
        if self.consumed {
            return Err(ContractError::AlreadyConsumed);
        }
        self.consumed = true;
        Ok(())
    }
}

/// Only approved named maxima; no caller-supplied larger capacity or reset/refund.
pub(super) struct Budget {
    limit: BudgetLimit,
    used: u64,
}

impl Budget {
    pub(super) fn new(limit: BudgetLimit) -> Self {
        Self { limit, used: 0 }
    }

    pub(super) fn used(&self) -> u64 {
        self.used
    }

    pub(super) fn reserve(&mut self, amount: u64) -> Result<Reservation> {
        self.used = checked_total(self.used, amount, self.limit.maximum())?;
        Ok(Reservation::charged())
    }

    /// Rust str lengths count UTF-8 bytes, including multibyte characters.
    pub(super) fn reserve_utf8(&mut self, text: &str) -> Result<Reservation> {
        let bytes = u64::try_from(text.len()).map_err(|_| ContractError::Overflow)?;
        self.reserve(bytes)
    }
}

pub(super) struct DurationBudget {
    limit: DurationLimit,
    used: Duration,
}

impl DurationBudget {
    pub(super) fn new(limit: DurationLimit) -> Self {
        Self {
            limit,
            used: Duration::ZERO,
        }
    }

    pub(super) fn used(&self) -> Duration {
        self.used
    }

    pub(super) fn reserve(&mut self, amount: Duration) -> Result<Reservation> {
        self.used = checked_duration_total(self.used, amount, self.limit.maximum())?;
        Ok(Reservation::charged())
    }
}

/// History stops admitting pairs when either capacity is exhausted.
pub(super) struct HistoryBudget {
    pairs: u64,
    bytes: u64,
}

impl HistoryBudget {
    pub(super) fn new() -> Self {
        Self { pairs: 0, bytes: 0 }
    }

    pub(super) fn usage(&self) -> (u64, u64) {
        (self.pairs, self.bytes)
    }

    pub(super) fn reserve_pair(&mut self, bytes: u64) -> Result<Reservation> {
        let pairs = checked_total(self.pairs, 1, MAX_HISTORY_PAIRS)?;
        let bytes = checked_total(self.bytes, bytes, MAX_HISTORY_BYTES)?;
        self.pairs = pairs;
        self.bytes = bytes;
        Ok(Reservation::charged())
    }
}

#[derive(Clone, Copy)]
struct ChildUsage {
    session_id: SessionId,
    turns: u64,
}

#[derive(Clone, Copy)]
struct TurnUsage {
    turn_id: TurnId,
    captured_audio: Duration,
    provider_roles: [bool; 3],
}

/// One interaction owns all child/turn/provider/audio accounting. Fixed-size storage.
/// This is a pure admission ledger, not a conversation state machine or egress gate.
/// It cannot be cloned/deserialized/reset. Pauses have no accounting operation.
pub(super) struct InteractionBudget {
    interaction_id: InteractionId,
    children: [Option<ChildUsage>; MAX_CHILD_CONVERSATIONS as usize],
    turns: [Option<TurnUsage>; MAX_AGGREGATE_TURNS as usize],
    provider_requests: [u64; 3],
    cloud_audio: Duration,
}

impl InteractionBudget {
    pub(super) fn new(interaction_id: InteractionId) -> Self {
        Self {
            interaction_id,
            children: [None; MAX_CHILD_CONVERSATIONS as usize],
            turns: [None; MAX_AGGREGATE_TURNS as usize],
            provider_requests: [0; 3],
            cloud_audio: Duration::ZERO,
        }
    }

    pub(super) fn interaction_id(&self) -> InteractionId {
        self.interaction_id
    }

    pub(super) fn cloud_audio(&self) -> Duration {
        self.cloud_audio
    }

    pub(super) fn provider_requests(&self, role: ConversationProviderRole) -> u64 {
        self.provider_requests[role.index()]
    }

    pub(super) fn child_turns(&self, session_id: SessionId) -> Result<u64> {
        self.children
            .iter()
            .flatten()
            .find(|child| child.session_id == session_id)
            .map(|child| child.turns)
            .ok_or(ContractError::UnknownSession)
    }

    pub(super) fn reserve_child(&mut self, session_id: SessionId) -> Result<Reservation> {
        if self.child_turns(session_id).is_ok() {
            return Err(ContractError::DuplicateSession);
        }
        let slot = self
            .children
            .iter_mut()
            .find(|slot| slot.is_none())
            .ok_or(ContractError::LimitExceeded)?;
        *slot = Some(ChildUsage {
            session_id,
            turns: 0,
        });
        Ok(Reservation::charged())
    }

    /// Admit a turn independently of transcription route. No cloud audio is charged.
    pub(super) fn reserve_turn(
        &mut self,
        session_id: SessionId,
        turn_id: TurnId,
        audio: Duration,
    ) -> Result<Reservation> {
        let child = self
            .children
            .iter_mut()
            .flatten()
            .find(|child| child.session_id == session_id)
            .ok_or(ContractError::UnknownSession)?;
        let child_turns = checked_total(child.turns, 1, MAX_CHILD_TURNS)?;
        if self
            .turns
            .iter()
            .flatten()
            .any(|turn| turn.turn_id == turn_id)
        {
            return Err(ContractError::DuplicateTurn);
        }
        let slot = self
            .turns
            .iter_mut()
            .find(|slot| slot.is_none())
            .ok_or(ContractError::LimitExceeded)?;
        checked_duration_total(Duration::ZERO, audio, MAX_CAPTURED_TURN)?;
        // Commit only after every bound is checked; failures change nothing.
        child.turns = child_turns;
        *slot = Some(TurnUsage {
            turn_id,
            captured_audio: audio,
            provider_roles: [false; 3],
        });
        Ok(Reservation::charged())
    }

    /// Pure accounting for one STT attempt, not dispatch. The full admitted audio
    /// duration is charged only for cloud routing. Charge and role admission are
    /// atomic and irreversible, including when a future dispatch is ambiguous.
    pub(super) fn reserve_transcription(
        &mut self,
        turn_id: TurnId,
        route: TranscriptionRoute,
    ) -> Result<Reservation> {
        let turn = self
            .turns
            .iter_mut()
            .flatten()
            .find(|turn| turn.turn_id == turn_id)
            .ok_or(ContractError::UnknownTurn)?;
        let index = ConversationProviderRole::Stt.index();
        if turn.provider_roles[index] {
            return Err(ContractError::DuplicateProviderRole);
        }
        let requests = checked_total(
            self.provider_requests[index],
            1,
            MAX_PROVIDER_REQUESTS_PER_ROLE_PER_INTERACTION,
        )?;
        let cloud_audio = match route {
            TranscriptionRoute::LocalOnly => self.cloud_audio,
            TranscriptionRoute::Cloud => {
                checked_duration_total(self.cloud_audio, turn.captured_audio, MAX_CLOUD_AUDIO)?
            }
        };
        turn.provider_roles[index] = true;
        self.provider_requests[index] = requests;
        self.cloud_audio = cloud_audio;
        Ok(Reservation::charged())
    }

    pub(super) fn reserve_provider(
        &mut self,
        turn_id: TurnId,
        role: ConversationProviderRole,
    ) -> Result<Reservation> {
        // STT must choose a route so generic admission cannot bypass audio charging.
        if role == ConversationProviderRole::Stt {
            return Err(ContractError::TranscriptionRouteRequired);
        }
        let turn = self
            .turns
            .iter_mut()
            .flatten()
            .find(|turn| turn.turn_id == turn_id)
            .ok_or(ContractError::UnknownTurn)?;
        let index = role.index();
        if turn.provider_roles[index] {
            return Err(ContractError::DuplicateProviderRole);
        }
        let requests = checked_total(
            self.provider_requests[index],
            1,
            MAX_PROVIDER_REQUESTS_PER_ROLE_PER_INTERACTION,
        )?;
        turn.provider_roles[index] = true;
        self.provider_requests[index] = requests;
        Ok(Reservation::charged())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SpeechSlot {
    Active,
    Pending,
}

impl SpeechSlot {
    fn index(self) -> usize {
        match self {
            Self::Active => 0,
            Self::Pending => 1,
        }
    }
}

/// Native-only capacity receipt, not a speech/provider permission. No serde or Clone.
/// References may be replayed, but only the currently occupying receipt can release.
#[derive(Debug)]
pub(super) struct SpeechCapacityLease {
    owner: Arc<()>,
    slot: SpeechSlot,
    sequence: u64,
    job_id: JobId,
}

struct SpeechOccupant {
    sequence: u64,
    job_id: JobId,
    text_bytes: u64,
}

/// Occupancy bookkeeping only, not a queue implementation or speech state machine.
/// Capacity is reusable; cumulative interaction budgets are never refunded here.
pub(super) struct SpeechCapacity {
    owner: Arc<()>,
    slots: [Option<SpeechOccupant>; (MAX_ACTIVE_SPEECH_JOBS + MAX_PENDING_SPEECH_JOBS) as usize],
    last_sequence: u64,
    text_bytes: u64,
}

impl SpeechCapacity {
    pub(super) fn new() -> Self {
        Self {
            owner: Arc::new(()),
            slots: [None, None],
            last_sequence: 0,
            text_bytes: 0,
        }
    }

    pub(super) fn usage(&self) -> (u64, u64, u64) {
        (
            u64::from(self.slots[SpeechSlot::Active.index()].is_some()),
            u64::from(self.slots[SpeechSlot::Pending.index()].is_some()),
            self.text_bytes,
        )
    }

    pub(super) fn occupy(
        &mut self,
        slot: SpeechSlot,
        job_id: JobId,
        text: &str,
        audio_bytes: u64,
    ) -> Result<SpeechCapacityLease> {
        if self.slots[slot.index()].is_some() {
            return Err(ContractError::LimitExceeded);
        }
        if self.slots.iter().flatten().any(|job| job.job_id == job_id) {
            return Err(ContractError::DuplicateJob);
        }
        let text_bytes = u64::try_from(text.len()).map_err(|_| ContractError::Overflow)?;
        checked_total(0, text_bytes, MAX_SPEECH_JOB_TEXT_BYTES)?;
        checked_total(0, audio_bytes, MAX_SPEECH_JOB_AUDIO_BYTES)?;
        let total_text = checked_total(self.text_bytes, text_bytes, MAX_QUEUED_SPEECH_TEXT_BYTES)?;
        let sequence = self
            .last_sequence
            .checked_add(1)
            .ok_or(ContractError::Overflow)?;
        self.slots[slot.index()] = Some(SpeechOccupant {
            sequence,
            job_id,
            text_bytes,
        });
        self.last_sequence = sequence;
        self.text_bytes = total_text;
        Ok(SpeechCapacityLease {
            owner: Arc::clone(&self.owner),
            slot,
            sequence,
            job_id,
        })
    }

    /// Explicit release of this exact occupant. Drop alone never releases capacity.
    pub(super) fn release(&mut self, lease: &SpeechCapacityLease) -> Result<()> {
        if !Arc::ptr_eq(&self.owner, &lease.owner) {
            return Err(ContractError::StaleSpeechLease);
        }
        let occupant = self.slots[lease.slot.index()]
            .as_ref()
            .ok_or(ContractError::StaleSpeechLease)?;
        if occupant.sequence != lease.sequence || occupant.job_id != lease.job_id {
            return Err(ContractError::StaleSpeechLease);
        }
        let text_bytes = self
            .text_bytes
            .checked_sub(occupant.text_bytes)
            .ok_or(ContractError::Overflow)?;
        self.slots[lease.slot.index()] = None;
        self.text_bytes = text_bytes;
        Ok(())
    }
}
