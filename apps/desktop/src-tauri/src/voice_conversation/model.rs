use super::{ContractError, Result};
use serde::{Deserialize, Serialize};
use std::num::NonZeroU64;

// Native sequence values, scoped to an app run; these are metadata, never grants.
// No random generation, IPC registration, consent issuance or resource acquisition.
macro_rules! identifier {
    ($($name:ident),+ $(,)?) => {$(
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub(super) struct $name(NonZeroU64);

        impl $name {
            pub(super) fn from_native_sequence(value: u64) -> Result<Self> {
                NonZeroU64::new(value).map(Self).ok_or(ContractError::InvalidIdentifier)
            }
        }
    )+};
}

identifier!(
    AppRunId,
    InteractionId,
    SessionId,
    TurnId,
    JobId,
    Generation,
    ConfigurationRevision,
    ConsentBindingId,
);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "u8", into = "u8")]
pub(super) enum ContractVersion {
    V1,
}

impl TryFrom<u8> for ContractVersion {
    type Error = &'static str;

    fn try_from(value: u8) -> std::result::Result<Self, Self::Error> {
        match value {
            1 => Ok(Self::V1),
            _ => Err("unsupported conversation contract version"),
        }
    }
}

impl From<ContractVersion> for u8 {
    fn from(_: ContractVersion) -> Self {
        1
    }
}

impl Generation {
    pub(super) fn checked_next(self) -> Result<Self> {
        let next = self.0.get().checked_add(1).ok_or(ContractError::Overflow)?;
        Self::from_native_sequence(next)
    }
}

impl ConfigurationRevision {
    pub(super) fn checked_next(self) -> Result<Self> {
        let next = self.0.get().checked_add(1).ok_or(ContractError::Overflow)?;
        Self::from_native_sequence(next)
    }
}

macro_rules! states {
    ($name:ident { $($variant:ident),+ $(,)? }) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
        #[serde(rename_all = "snake_case")]
        pub(super) enum $name { $($variant),+ }
    };
}

// Authoritative Phase 0 descriptions only. No transition methods or defaults
// that activate resources. InteractionId binds budgets, not another lifecycle.
states!(SessionState {
    Inactive,
    Starting,
    Active,
    Paused,
    Stopping,
    Stopped,
    Expired,
    Faulted
});
states!(TurnState {
    Idle,
    Listening,
    Finalizing,
    Transcribing,
    ValidatingTranscript,
    RequestingConversation,
    ValidatingReply,
    ReplyReady,
    Speaking,
    FollowUpWindow,
    AwaitingConsent,
    NeedsActionReview,
    Canceled,
    Failed
});
states!(WakeState {
    Disabled,
    Preparing,
    Armed,
    Suppressed,
    Cooldown,
    Suspended,
    Faulted
});
states!(MicrophoneOwner {
    Off,
    ManualForeground,
    WakeDetection,
    ConversationTurn
});

// Auxiliary acknowledgement descriptions, separate from effective ownership:
// NotRequested records no request; StoppingUnconfirmed records incomplete release;
// Acknowledged records an acknowledgement. None is a native release proof/grant.
states!(MicrophoneReleaseStatus {
    NotRequested,
    StoppingUnconfirmed,
    Acknowledged
});

// Auxiliary user-visible descriptions, not additional microphone lifecycle states.
// Unconfirmed is necessary to describe uncertainty without claiming capture stopped.
// No automatic mapping from owner/release status to visible status is defined here.
states!(ListeningStatus {
    NotListening,
    Listening,
    Unconfirmed
});

states!(SpeechState {
    Idle,
    Preparing,
    Fetching,
    Speaking
});
// Speech events/outcomes are deliberately not speech lifecycle states.
states!(SpeechOutcome {
    Stop,
    NormalCompletion,
    Failure,
    StaleCallback
});

// Planning describes the existing independently reviewed Jarvis role. Merely
// describing it grants no planner access; all existing review gates stay separate.
states!(ProviderRole {
    Stt,
    Conversation,
    Planning,
    Tts
});

// A narrower role contract structurally excludes planning from ordinary budgets.
// This introduces no additional provider roles, dispatch or action authority.
states!(ConversationProviderRole {
    Stt,
    Conversation,
    Tts
});

// Route descriptions do not grant provider access or consent.
states!(TranscriptionRoute { LocalOnly, Cloud });

impl ProviderRole {
    pub(super) const ALL: [Self; 4] = [Self::Stt, Self::Conversation, Self::Planning, Self::Tts];
}

impl TryFrom<ProviderRole> for ConversationProviderRole {
    type Error = ContractError;

    fn try_from(role: ProviderRole) -> Result<Self> {
        match role {
            ProviderRole::Stt => Ok(Self::Stt),
            ProviderRole::Conversation => Ok(Self::Conversation),
            ProviderRole::Tts => Ok(Self::Tts),
            ProviderRole::Planning => Err(ContractError::PlanningNotConversationRole),
        }
    }
}

impl ConversationProviderRole {
    pub(super) const ALL: [Self; 3] = [Self::Stt, Self::Conversation, Self::Tts];

    pub(super) const fn index(self) -> usize {
        match self {
            Self::Stt => 0,
            Self::Conversation => 1,
            Self::Tts => 2,
        }
    }
}

/// Serialized identity descriptions are untrusted data, including consent_binding_id.
/// Deserialization never validates consent or authorizes microphone/provider/actions.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ContractIdentity {
    pub(super) version: ContractVersion,
    pub(super) app_run_id: AppRunId,
    pub(super) interaction_id: InteractionId,
    pub(super) session_id: SessionId,
    pub(super) turn_id: TurnId,
    pub(super) job_id: JobId,
    pub(super) generation: Generation,
    pub(super) configuration_revision: ConfigurationRevision,
    pub(super) consent_binding_id: ConsentBindingId,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct StateDescription {
    pub(super) identity: ContractIdentity,
    pub(super) session: SessionState,
    pub(super) turn: TurnState,
    pub(super) wake: WakeState,
    pub(super) microphone_owner: MicrophoneOwner,
    pub(super) microphone_release: MicrophoneReleaseStatus,
    pub(super) listening_status: ListeningStatus,
    pub(super) speech: SpeechState,
    pub(super) provider_role: ProviderRole,
}
