use super::clock::*;
use super::limits::*;
use super::model::*;
use super::ContractError;
use serde::{de::DeserializeOwned, Serialize};
use serde_json::{json, Value};
use std::cmp::Ordering;
use std::time::Duration;

fn identity() -> ContractIdentity {
    ContractIdentity {
        version: ContractVersion::V1,
        app_run_id: AppRunId::from_native_sequence(1).unwrap(),
        interaction_id: InteractionId::from_native_sequence(2).unwrap(),
        session_id: SessionId::from_native_sequence(3).unwrap(),
        turn_id: TurnId::from_native_sequence(4).unwrap(),
        job_id: JobId::from_native_sequence(5).unwrap(),
        generation: Generation::from_native_sequence(6).unwrap(),
        configuration_revision: ConfigurationRevision::from_native_sequence(7).unwrap(),
        consent_binding_id: ConsentBindingId::from_native_sequence(8).unwrap(),
    }
}

fn interaction() -> InteractionBudget {
    InteractionBudget::new(identity().interaction_id)
}

fn session(value: u64) -> SessionId {
    SessionId::from_native_sequence(value).unwrap()
}

fn turn(value: u64) -> TurnId {
    TurnId::from_native_sequence(value).unwrap()
}

fn job(value: u64) -> JobId {
    JobId::from_native_sequence(value).unwrap()
}

fn reserve_role(
    budget: &mut InteractionBudget,
    turn_id: TurnId,
    role: ConversationProviderRole,
) -> super::Result<Reservation> {
    match role {
        ConversationProviderRole::Stt => {
            budget.reserve_transcription(turn_id, TranscriptionRoute::LocalOnly)
        }
        _ => budget.reserve_provider(turn_id, role),
    }
}

#[test]
fn identifiers_are_distinct_nonzero_unsigned_types() {
    use std::any::TypeId;
    let mut types = Vec::new();
    macro_rules! check {
        ($($ty:ty),+ $(,)?) => {$(
            types.push(TypeId::of::<$ty>());
            assert_eq!(<$ty>::from_native_sequence(0), Err(ContractError::InvalidIdentifier));
            for valid in [1, u64::MAX] {
                let id = <$ty>::from_native_sequence(valid).unwrap();
                assert_eq!(serde_json::to_value(id).unwrap(), json!(valid));
                assert_eq!(serde_json::from_value::<$ty>(json!(valid)).unwrap(), id);
            }
            for invalid in ["0", "-1", "1.5", "null", "true", "\"1\"", "{}", "18446744073709551616"] {
                assert!(serde_json::from_str::<$ty>(invalid).is_err(), "{invalid}");
            }
        )+};
    }
    check!(
        AppRunId,
        InteractionId,
        SessionId,
        TurnId,
        JobId,
        Generation,
        ConfigurationRevision,
        ConsentBindingId,
    );
    for (index, ty) in types.iter().enumerate() {
        assert!(!types[index + 1..].contains(ty));
    }
}

#[test]
fn generation_and_configuration_revision_reject_overflow() {
    assert_eq!(
        Generation::from_native_sequence(1).unwrap().checked_next(),
        Generation::from_native_sequence(2)
    );
    assert_eq!(
        ConfigurationRevision::from_native_sequence(1)
            .unwrap()
            .checked_next(),
        ConfigurationRevision::from_native_sequence(2)
    );
    assert_eq!(
        Generation::from_native_sequence(u64::MAX)
            .unwrap()
            .checked_next(),
        Err(ContractError::Overflow)
    );
    assert_eq!(
        ConfigurationRevision::from_native_sequence(u64::MAX)
            .unwrap()
            .checked_next(),
        Err(ContractError::Overflow)
    );
}

#[test]
fn version_and_identity_serialization_are_strict() {
    let valid = serde_json::to_value(identity()).unwrap();
    assert_eq!(valid["version"], 1);
    assert_eq!(
        serde_json::from_value::<ContractIdentity>(valid.clone()).unwrap(),
        identity()
    );
    for version in [
        json!(0),
        json!(2),
        json!(-1),
        json!(256),
        json!(1.0),
        json!("1"),
        Value::Null,
    ] {
        let mut invalid = valid.clone();
        invalid["version"] = version;
        assert!(serde_json::from_value::<ContractIdentity>(invalid).is_err());
    }
    for field in valid.as_object().unwrap().keys() {
        let mut missing = valid.clone();
        missing.as_object_mut().unwrap().remove(field);
        assert!(serde_json::from_value::<ContractIdentity>(missing).is_err());
    }
    let mut unknown = valid.clone();
    unknown["microphone_authorized"] = json!(true);
    assert!(serde_json::from_value::<ContractIdentity>(unknown).is_err());
    let mut invalid_id = valid;
    invalid_id["consent_binding_id"] = json!(0);
    assert!(serde_json::from_value::<ContractIdentity>(invalid_id).is_err());
    assert!(serde_json::from_str::<ContractVersion>("{\"version\":1,\"extra\":true}").is_err());
    assert!(serde_json::from_str::<ContractIdentity>(
        "{\"version\":1,\"version\":1,\"app_run_id\":1,\"interaction_id\":2,\"session_id\":3,\"turn_id\":4,\"job_id\":5,\"generation\":6,\"configuration_revision\":7,\"consent_binding_id\":8}"
    ).is_err());
}

fn closed_enum<T: Serialize + DeserializeOwned + PartialEq + std::fmt::Debug>(
    variants: &[(T, &str)],
) {
    for (variant, name) in variants {
        assert_eq!(serde_json::to_value(variant).unwrap(), json!(name));
        assert_eq!(serde_json::from_value::<T>(json!(name)).unwrap(), *variant);
        assert!(serde_json::from_value::<T>(json!({(*name): {"extra": true}})).is_err());
        assert!(serde_json::from_value::<T>(json!({(*name): true})).is_err());
        assert!(serde_json::from_value::<T>(json!({(*name): []})).is_err());
        assert!(serde_json::from_value::<T>(json!([name])).is_err());
        assert!(serde_json::from_value::<T>(json!(name.to_uppercase())).is_err());
    }
    for invalid in [
        json!("unknown"),
        json!("ACTIVE"),
        json!(""),
        json!(0),
        json!(-1),
        json!(1.5),
        json!(true),
        json!({}),
        json!([]),
        Value::Null,
    ] {
        assert!(serde_json::from_value::<T>(invalid).is_err());
    }
}

#[test]
fn authoritative_session_states_are_closed() {
    closed_enum(&[
        (SessionState::Inactive, "inactive"),
        (SessionState::Starting, "starting"),
        (SessionState::Active, "active"),
        (SessionState::Paused, "paused"),
        (SessionState::Stopping, "stopping"),
        (SessionState::Stopped, "stopped"),
        (SessionState::Expired, "expired"),
        (SessionState::Faulted, "faulted"),
    ]);
}

#[test]
fn authoritative_turn_states_are_closed() {
    closed_enum(&[
        (TurnState::Idle, "idle"),
        (TurnState::Listening, "listening"),
        (TurnState::Finalizing, "finalizing"),
        (TurnState::Transcribing, "transcribing"),
        (TurnState::ValidatingTranscript, "validating_transcript"),
        (TurnState::RequestingConversation, "requesting_conversation"),
        (TurnState::ValidatingReply, "validating_reply"),
        (TurnState::ReplyReady, "reply_ready"),
        (TurnState::Speaking, "speaking"),
        (TurnState::FollowUpWindow, "follow_up_window"),
        (TurnState::AwaitingConsent, "awaiting_consent"),
        (TurnState::NeedsActionReview, "needs_action_review"),
        (TurnState::Canceled, "canceled"),
        (TurnState::Failed, "failed"),
    ]);
}

#[test]
fn authoritative_wake_states_are_closed() {
    closed_enum(&[
        (WakeState::Disabled, "disabled"),
        (WakeState::Preparing, "preparing"),
        (WakeState::Armed, "armed"),
        (WakeState::Suppressed, "suppressed"),
        (WakeState::Cooldown, "cooldown"),
        (WakeState::Suspended, "suspended"),
        (WakeState::Faulted, "faulted"),
    ]);
}

#[test]
fn authoritative_microphone_owners_are_closed() {
    closed_enum(&[
        (MicrophoneOwner::Off, "off"),
        (MicrophoneOwner::ManualForeground, "manual_foreground"),
        (MicrophoneOwner::WakeDetection, "wake_detection"),
        (MicrophoneOwner::ConversationTurn, "conversation_turn"),
    ]);
}

#[test]
fn microphone_release_acknowledgements_are_separate_closed_descriptions() {
    closed_enum(&[
        (MicrophoneReleaseStatus::NotRequested, "not_requested"),
        (
            MicrophoneReleaseStatus::StoppingUnconfirmed,
            "stopping_unconfirmed",
        ),
        (MicrophoneReleaseStatus::Acknowledged, "acknowledged"),
    ]);
}

#[test]
fn visible_listening_status_is_a_separate_closed_description() {
    closed_enum(&[
        (ListeningStatus::NotListening, "not_listening"),
        (ListeningStatus::Listening, "listening"),
        (ListeningStatus::Unconfirmed, "unconfirmed"),
    ]);
}

#[test]
fn authoritative_speech_states_are_closed() {
    closed_enum(&[
        (SpeechState::Idle, "idle"),
        (SpeechState::Preparing, "preparing"),
        (SpeechState::Fetching, "fetching"),
        (SpeechState::Speaking, "speaking"),
    ]);
}

#[test]
fn authoritative_speech_outcomes_are_closed_and_not_speech_states() {
    let variants = [
        (SpeechOutcome::Stop, "stop"),
        (SpeechOutcome::NormalCompletion, "normal_completion"),
        (SpeechOutcome::Failure, "failure"),
        (SpeechOutcome::StaleCallback, "stale_callback"),
    ];
    closed_enum(&variants);
    for (_, name) in variants {
        assert!(serde_json::from_value::<SpeechState>(json!(name)).is_err());
    }
}

#[test]
fn authoritative_provider_roles_include_independent_planning() {
    closed_enum(&[
        (ProviderRole::Stt, "stt"),
        (ProviderRole::Conversation, "conversation"),
        (ProviderRole::Planning, "planning"),
        (ProviderRole::Tts, "tts"),
    ]);
    assert_eq!(ProviderRole::ALL.len(), 4);
}

#[test]
fn conversation_provider_roles_exclude_planning() {
    closed_enum(&[
        (ConversationProviderRole::Stt, "stt"),
        (ConversationProviderRole::Conversation, "conversation"),
        (ConversationProviderRole::Tts, "tts"),
    ]);
    assert_eq!(
        ConversationProviderRole::try_from(ProviderRole::Planning),
        Err(ContractError::PlanningNotConversationRole)
    );
    assert!(serde_json::from_value::<ConversationProviderRole>(json!("planning")).is_err());
    for (general, ordinary) in [
        (ProviderRole::Stt, ConversationProviderRole::Stt),
        (
            ProviderRole::Conversation,
            ConversationProviderRole::Conversation,
        ),
        (ProviderRole::Tts, ConversationProviderRole::Tts),
    ] {
        assert_eq!(
            ConversationProviderRole::try_from(general).unwrap(),
            ordinary
        );
    }
}

#[test]
fn transcription_routes_remain_closed() {
    closed_enum(&[
        (TranscriptionRoute::LocalOnly, "local_only"),
        (TranscriptionRoute::Cloud, "cloud"),
    ]);
}

fn state_description() -> StateDescription {
    StateDescription {
        identity: identity(),
        session: SessionState::Inactive,
        turn: TurnState::Idle,
        wake: WakeState::Disabled,
        microphone_owner: MicrophoneOwner::Off,
        microphone_release: MicrophoneReleaseStatus::NotRequested,
        listening_status: ListeningStatus::NotListening,
        speech: SpeechState::Idle,
        provider_role: ProviderRole::Stt,
    }
}

#[test]
fn nested_descriptions_reject_unknown_fields_and_states() {
    let description = state_description();
    let value = serde_json::to_value(description).unwrap();
    assert_eq!(
        serde_json::from_value::<StateDescription>(value.clone()).unwrap(),
        description
    );
    for field in [
        "session",
        "turn",
        "wake",
        "microphone_owner",
        "microphone_release",
        "listening_status",
        "speech",
        "provider_role",
    ] {
        let mut invalid = value.clone();
        invalid[field] = json!("unknown");
        assert!(serde_json::from_value::<StateDescription>(invalid).is_err());
    }
    for field in value.as_object().unwrap().keys() {
        let mut missing = value.clone();
        missing.as_object_mut().unwrap().remove(field);
        assert!(serde_json::from_value::<StateDescription>(missing).is_err());
    }
    let mut unknown = value.clone();
    unknown["grant"] = json!(true);
    assert!(serde_json::from_value::<StateDescription>(unknown).is_err());
    let mut nested = value;
    nested["identity"]["grant"] = json!(true);
    assert!(serde_json::from_value::<StateDescription>(nested).is_err());
}

#[test]
fn obsolete_lifecycle_descriptions_are_rejected_without_aliases() {
    for name in ["idle", "closed", "failed"] {
        assert!(serde_json::from_value::<SessionState>(json!(name)).is_err());
    }
    for name in [
        "waiting_for_speech",
        "capturing",
        "awaiting_provider",
        "awaiting_speech",
        "completed",
        "cancelled",
    ] {
        assert!(serde_json::from_value::<TurnState>(json!(name)).is_err());
    }
    for name in ["disarmed", "expired"] {
        assert!(serde_json::from_value::<WakeState>(json!(name)).is_err());
    }
    for name in [
        "inactive",
        "awaiting_consent",
        "capturing",
        "finalizing",
        "failed",
        "stopping_unconfirmed",
    ] {
        assert!(serde_json::from_value::<MicrophoneOwner>(json!(name)).is_err());
    }
    for name in ["pending", "active", "completed", "cancelled", "failed"] {
        assert!(serde_json::from_value::<SpeechState>(json!(name)).is_err());
    }
    assert!(serde_json::from_value::<ProviderRole>(json!("assistant")).is_err());
    for field in ["interaction", "microphone"] {
        let mut old = serde_json::to_value(state_description()).unwrap();
        old[field] = json!("idle");
        assert!(serde_json::from_value::<StateDescription>(old).is_err());
    }
}

#[test]
fn owner_off_does_not_acknowledge_release_or_claim_not_listening() {
    let mut description = state_description();
    description.microphone_release = MicrophoneReleaseStatus::StoppingUnconfirmed;
    description.listening_status = ListeningStatus::Unconfirmed;
    let value = serde_json::to_value(description).unwrap();
    assert_eq!(value["microphone_owner"], "off");
    assert_eq!(value["microphone_release"], "stopping_unconfirmed");
    assert_eq!(value["listening_status"], "unconfirmed");
    // Ownership does not supply or rewrite acknowledgement/visible descriptions.
    assert_eq!(
        serde_json::from_value::<StateDescription>(value).unwrap(),
        description
    );
    assert!(serde_json::from_value::<MicrophoneReleaseStatus>(json!("off")).is_err());
    assert!(serde_json::from_value::<ListeningStatus>(json!("off")).is_err());
    assert!(serde_json::from_value::<MicrophoneOwner>(json!("acknowledged")).is_err());
    assert!(serde_json::from_value::<ListeningStatus>(json!("acknowledged")).is_err());
    assert!(serde_json::from_value::<MicrophoneReleaseStatus>(json!("not_listening")).is_err());
}

#[test]
fn planning_description_cannot_enter_ordinary_conversation_accounting() {
    let mut budget = interaction();
    budget.reserve_child(session(1)).unwrap();
    budget
        .reserve_turn(session(1), turn(1), MAX_CAPTURED_TURN)
        .unwrap();
    let role: ProviderRole = serde_json::from_value(json!("planning")).unwrap();
    let outcome = ConversationProviderRole::try_from(role)
        .and_then(|ordinary| budget.reserve_provider(turn(1), ordinary));
    assert!(matches!(
        outcome,
        Err(ContractError::PlanningNotConversationRole)
    ));
    for ordinary in ConversationProviderRole::ALL {
        assert_eq!(budget.provider_requests(ordinary), 0);
    }
    assert_eq!(budget.cloud_audio(), Duration::ZERO);
}

#[test]
fn approved_integer_limits_use_binary_bytes() {
    assert_eq!(KIB, 1024);
    assert_eq!(MIB, 1_048_576);
    let expected = [
        8, 4, 16, 8_388_608, 8192, 4096, 8, 65_536, 32_768, 65_536, 262_144, 4096, 8_388_608, 1, 16,
    ];
    let actual: Vec<_> = BudgetLimit::ALL
        .iter()
        .map(|limit| limit.maximum())
        .collect();
    assert_eq!(actual, expected);
}

#[test]
fn approved_duration_limits_are_exact() {
    let expected = [28_800, 900, 300, 60, 5, 8, 30, 120, 5, 90, 2, 3];
    let actual: Vec<_> = DurationLimit::ALL
        .iter()
        .map(|limit| limit.maximum())
        .collect();
    assert_eq!(actual, expected.map(Duration::from_secs));
}

#[test]
fn every_integer_budget_accepts_exact_limit_and_rejects_one_more_atomically() {
    for &limit in BudgetLimit::ALL {
        let mut budget = Budget::new(limit);
        budget
            .reserve(limit.maximum() - 1)
            .unwrap()
            .consume()
            .unwrap();
        budget.reserve(1).unwrap().consume().unwrap();
        assert_eq!(budget.used(), limit.maximum());
        assert!(matches!(
            budget.reserve(1),
            Err(ContractError::LimitExceeded)
        ));
        assert_eq!(budget.used(), limit.maximum());
        let mut empty = Budget::new(limit);
        assert!(matches!(
            empty.reserve(limit.maximum() + 1),
            Err(ContractError::LimitExceeded)
        ));
        assert_eq!(empty.used(), 0);
    }
}

#[test]
fn every_duration_budget_accepts_exact_limit_and_rejects_one_nanosecond_more() {
    for &limit in DurationLimit::ALL {
        let mut budget = DurationBudget::new(limit);
        budget
            .reserve(limit.maximum() - Duration::from_nanos(1))
            .unwrap();
        budget
            .reserve(Duration::from_nanos(1))
            .unwrap()
            .consume()
            .unwrap();
        assert_eq!(budget.used(), limit.maximum());
        assert!(matches!(
            budget.reserve(Duration::from_nanos(1)),
            Err(ContractError::LimitExceeded)
        ));
        assert_eq!(budget.used(), limit.maximum());
        let mut empty = DurationBudget::new(limit);
        assert!(matches!(
            empty.reserve(limit.maximum() + Duration::from_nanos(1)),
            Err(ContractError::LimitExceeded)
        ));
        assert_eq!(empty.used(), Duration::ZERO);
    }
}

#[test]
fn checked_arithmetic_rejects_overflow_and_preserves_budgets() {
    assert_eq!(
        checked_total(u64::MAX, 1, u64::MAX),
        Err(ContractError::Overflow)
    );
    assert_eq!(checked_total(u64::MAX - 1, 1, u64::MAX), Ok(u64::MAX));
    assert_eq!(
        checked_duration_total(Duration::MAX, Duration::from_nanos(1), Duration::MAX),
        Err(ContractError::Overflow)
    );
    assert_eq!(
        checked_duration_total(Duration::MAX, Duration::ZERO, Duration::MAX),
        Ok(Duration::MAX)
    );
    let mut bytes = Budget::new(BudgetLimit::UserTranscriptBytes);
    bytes.reserve(1).unwrap();
    assert!(matches!(
        bytes.reserve(u64::MAX),
        Err(ContractError::Overflow)
    ));
    assert_eq!(bytes.used(), 1);
    let mut duration = DurationBudget::new(DurationLimit::CapturedTurn);
    duration.reserve(Duration::from_nanos(1)).unwrap();
    assert!(matches!(
        duration.reserve(Duration::MAX),
        Err(ContractError::Overflow)
    ));
    assert_eq!(duration.used(), Duration::from_nanos(1));
}

#[test]
fn consumed_and_dropped_reservations_never_refund_and_cannot_be_reconsumed() {
    let mut budget = Budget::new(BudgetLimit::ChildTurns);
    let mut consumed = budget.reserve(4).unwrap();
    consumed.consume().unwrap();
    assert_eq!(consumed.consume(), Err(ContractError::AlreadyConsumed));
    drop(consumed);
    drop(budget.reserve(4).unwrap());
    assert_eq!(budget.used(), 8);
    assert!(matches!(
        budget.reserve(1),
        Err(ContractError::LimitExceeded)
    ));
    let mut duration = DurationBudget::new(DurationLimit::CloudAudio);
    drop(duration.reserve(MAX_CLOUD_AUDIO).unwrap());
    assert!(matches!(
        duration.reserve(Duration::from_nanos(1)),
        Err(ContractError::LimitExceeded)
    ));
}

#[test]
fn utf8_bounds_measure_bytes_rather_than_characters() {
    for limit in [
        BudgetLimit::UserTranscriptBytes,
        BudgetLimit::AssistantReplyBytes,
        BudgetLimit::SpeechJobTextBytes,
    ] {
        let text = "é".repeat((limit.maximum() / 2) as usize);
        assert_eq!(text.len() as u64, limit.maximum());
        let mut budget = Budget::new(limit);
        budget.reserve_utf8(&text).unwrap();
        assert!(matches!(
            budget.reserve_utf8("a"),
            Err(ContractError::LimitExceeded)
        ));
        let mut empty = Budget::new(limit);
        assert!(matches!(
            empty.reserve_utf8(&(text + "a")),
            Err(ContractError::LimitExceeded)
        ));
        assert_eq!(empty.used(), 0);
    }
}

#[test]
fn history_obeys_both_pair_and_byte_caps_without_partial_charge() {
    let mut history = HistoryBudget::new();
    for _ in 0..8 {
        history.reserve_pair(8192).unwrap().consume().unwrap();
    }
    assert_eq!(history.usage(), (8, 65_536));
    assert!(matches!(
        history.reserve_pair(0),
        Err(ContractError::LimitExceeded)
    ));
    assert_eq!(history.usage(), (8, 65_536));
    let mut bytes = HistoryBudget::new();
    assert!(matches!(
        bytes.reserve_pair(65_537),
        Err(ContractError::LimitExceeded)
    ));
    assert_eq!(bytes.usage(), (0, 0));
    bytes.reserve_pair(65_536).unwrap();
    assert!(matches!(
        bytes.reserve_pair(1),
        Err(ContractError::LimitExceeded)
    ));
    assert_eq!(bytes.usage(), (1, 65_536));
    let mut pairs = HistoryBudget::new();
    for _ in 0..8 {
        pairs.reserve_pair(1).unwrap();
    }
    assert!(matches!(
        pairs.reserve_pair(1),
        Err(ContractError::LimitExceeded)
    ));
    assert_eq!(pairs.usage(), (8, 8));
}

#[test]
fn child_admission_is_bounded_and_repeated_sessions_cannot_reset_usage() {
    let mut budget = interaction();
    for id in 1..=4 {
        budget.reserve_child(session(id)).unwrap();
    }
    assert!(matches!(
        budget.reserve_child(session(5)),
        Err(ContractError::LimitExceeded)
    ));
    for id in 1..=8 {
        budget
            .reserve_turn(session(1), turn(id), Duration::ZERO)
            .unwrap();
    }
    assert!(matches!(
        budget.reserve_child(session(1)),
        Err(ContractError::DuplicateSession)
    ));
    assert!(matches!(
        budget.reserve_turn(session(1), turn(9), Duration::ZERO),
        Err(ContractError::LimitExceeded)
    ));
    assert_eq!(budget.child_turns(session(1)).unwrap(), 8);
    assert_eq!(budget.child_turns(session(2)).unwrap(), 0);
    assert_eq!(
        budget.child_turns(session(5)),
        Err(ContractError::UnknownSession)
    );
}

#[test]
fn aggregate_turn_limit_survives_new_child_sessions_and_inert_pause_metadata() {
    let mut budget = interaction();
    budget.reserve_child(session(1)).unwrap();
    for id in 1..=8 {
        budget
            .reserve_turn(session(1), turn(id), Duration::ZERO)
            .unwrap();
        budget
            .reserve_provider(turn(id), ConversationProviderRole::Conversation)
            .unwrap();
    }
    // Creating/deserializing paused state descriptions has no link to accounting.
    assert_eq!(
        serde_json::from_value::<SessionState>(json!("paused")).unwrap(),
        SessionState::Paused
    );
    budget.reserve_child(session(2)).unwrap();
    for id in 9..=16 {
        budget
            .reserve_turn(session(2), turn(id), Duration::ZERO)
            .unwrap();
        budget
            .reserve_provider(turn(id), ConversationProviderRole::Conversation)
            .unwrap();
    }
    budget.reserve_child(session(3)).unwrap();
    assert!(matches!(
        budget.reserve_turn(session(3), turn(17), Duration::ZERO),
        Err(ContractError::LimitExceeded)
    ));
    assert_eq!(budget.child_turns(session(3)).unwrap(), 0);
    assert_eq!(
        budget.provider_requests(ConversationProviderRole::Conversation),
        16
    );
    assert_eq!(budget.interaction_id(), identity().interaction_id);
}

#[test]
fn turn_admission_checks_capture_bound_without_charging_cloud_audio() {
    let mut budget = interaction();
    budget.reserve_child(session(1)).unwrap();
    assert!(matches!(
        budget.reserve_turn(session(1), turn(1), Duration::from_secs(31)),
        Err(ContractError::LimitExceeded)
    ));
    assert_eq!(budget.child_turns(session(1)).unwrap(), 0);
    assert_eq!(budget.cloud_audio(), Duration::ZERO);
    for id in 1..=3 {
        budget
            .reserve_turn(session(1), turn(id), MAX_CAPTURED_TURN)
            .unwrap()
            .consume()
            .unwrap();
    }
    assert_eq!(budget.cloud_audio(), Duration::ZERO);
    budget.reserve_child(session(2)).unwrap();
    drop(
        budget
            .reserve_turn(session(2), turn(4), MAX_CAPTURED_TURN)
            .unwrap(),
    );
    assert_eq!(budget.child_turns(session(2)).unwrap(), 1);
    assert_eq!(budget.cloud_audio(), Duration::ZERO);
}

#[test]
fn local_only_interaction_can_transcribe_more_than_cloud_audio_limit() {
    let mut budget = interaction();
    for child in 1..=2 {
        budget.reserve_child(session(child)).unwrap();
        for offset in 1..=8 {
            let id = turn((child - 1) * 8 + offset);
            budget
                .reserve_turn(session(child), id, MAX_CAPTURED_TURN)
                .unwrap();
            budget
                .reserve_transcription(id, TranscriptionRoute::LocalOnly)
                .unwrap()
                .consume()
                .unwrap();
        }
    }
    assert_eq!(budget.provider_requests(ConversationProviderRole::Stt), 16);
    assert_eq!(budget.cloud_audio(), Duration::ZERO);
}

#[test]
fn mixed_local_and_cloud_accounting_is_atomic_across_children() {
    let mut budget = interaction();
    budget.reserve_child(session(1)).unwrap();
    for id in 1..=8 {
        budget
            .reserve_turn(session(1), turn(id), MAX_CAPTURED_TURN)
            .unwrap();
        let route = if id % 2 == 0 {
            TranscriptionRoute::Cloud
        } else {
            TranscriptionRoute::LocalOnly
        };
        budget
            .reserve_transcription(turn(id), route)
            .unwrap()
            .consume()
            .unwrap();
    }
    assert_eq!(budget.cloud_audio(), MAX_CLOUD_AUDIO);
    assert_eq!(budget.provider_requests(ConversationProviderRole::Stt), 8);
    budget.reserve_child(session(2)).unwrap();
    budget
        .reserve_turn(session(2), turn(9), Duration::from_nanos(1))
        .unwrap();
    // Cloud failure changes neither the audio total nor the turn's STT admission.
    assert!(matches!(
        budget.reserve_transcription(turn(9), TranscriptionRoute::Cloud),
        Err(ContractError::LimitExceeded)
    ));
    assert_eq!(budget.cloud_audio(), MAX_CLOUD_AUDIO);
    assert_eq!(budget.provider_requests(ConversationProviderRole::Stt), 8);
    budget
        .reserve_transcription(turn(9), TranscriptionRoute::LocalOnly)
        .unwrap();
    assert_eq!(budget.provider_requests(ConversationProviderRole::Stt), 9);
    assert!(matches!(
        budget.reserve_transcription(turn(9), TranscriptionRoute::Cloud),
        Err(ContractError::DuplicateProviderRole)
    ));
    assert_eq!(budget.child_turns(session(2)).unwrap(), 1);
    assert_eq!(budget.cloud_audio(), MAX_CLOUD_AUDIO);
}

#[test]
fn cloud_reservations_remain_charged_after_consumption_drop_or_ambiguous_dispatch() {
    let mut budget = interaction();
    budget.reserve_child(session(1)).unwrap();
    for id in 1..=4 {
        budget
            .reserve_turn(session(1), turn(id), MAX_CAPTURED_TURN)
            .unwrap();
        let mut reservation = budget
            .reserve_transcription(turn(id), TranscriptionRoute::Cloud)
            .unwrap();
        if id != 4 {
            reservation.consume().unwrap();
            assert_eq!(reservation.consume(), Err(ContractError::AlreadyConsumed));
        }
        // Dropping the receipt after an unknown future dispatch outcome never refunds.
        drop(reservation);
        assert!(matches!(
            budget.reserve_transcription(turn(id), TranscriptionRoute::Cloud),
            Err(ContractError::DuplicateProviderRole)
        ));
    }
    assert_eq!(budget.cloud_audio(), MAX_CLOUD_AUDIO);
    budget
        .reserve_turn(session(1), turn(5), Duration::from_secs(1))
        .unwrap();
    assert!(matches!(
        budget.reserve_transcription(turn(5), TranscriptionRoute::Cloud),
        Err(ContractError::LimitExceeded)
    ));
    assert_eq!(budget.provider_requests(ConversationProviderRole::Stt), 4);
    assert_eq!(budget.cloud_audio(), MAX_CLOUD_AUDIO);
}

#[test]
fn generic_provider_admission_cannot_bypass_transcription_routing() {
    let mut budget = interaction();
    budget.reserve_child(session(1)).unwrap();
    budget
        .reserve_turn(session(1), turn(1), MAX_CAPTURED_TURN)
        .unwrap();
    assert!(matches!(
        budget.reserve_provider(turn(1), ConversationProviderRole::Stt),
        Err(ContractError::TranscriptionRouteRequired)
    ));
    assert_eq!(budget.provider_requests(ConversationProviderRole::Stt), 0);
    assert_eq!(budget.cloud_audio(), Duration::ZERO);
    budget
        .reserve_transcription(turn(1), TranscriptionRoute::Cloud)
        .unwrap();
    assert_eq!(budget.cloud_audio(), MAX_CAPTURED_TURN);
}

#[test]
fn unknown_and_duplicate_turns_do_not_charge_or_allow_replay() {
    let mut budget = interaction();
    assert!(matches!(
        budget.reserve_turn(session(1), turn(1), Duration::ZERO),
        Err(ContractError::UnknownSession)
    ));
    assert!(matches!(
        budget.reserve_transcription(turn(1), TranscriptionRoute::Cloud),
        Err(ContractError::UnknownTurn)
    ));
    budget.reserve_child(session(1)).unwrap();
    budget.reserve_child(session(2)).unwrap();
    budget
        .reserve_turn(session(1), turn(1), Duration::from_secs(1))
        .unwrap();
    assert!(matches!(
        budget.reserve_turn(session(2), turn(1), Duration::from_secs(1)),
        Err(ContractError::DuplicateTurn)
    ));
    assert_eq!(budget.child_turns(session(2)).unwrap(), 0);
    assert_eq!(budget.cloud_audio(), Duration::ZERO);
    assert_eq!(budget.provider_requests(ConversationProviderRole::Stt), 0);
}

#[test]
fn each_provider_role_is_once_per_turn_and_at_most_sixteen_per_interaction() {
    let mut budget = interaction();
    for child in 1..=2 {
        budget.reserve_child(session(child)).unwrap();
        for offset in 1..=8 {
            let id = turn((child - 1) * 8 + offset);
            budget
                .reserve_turn(session(child), id, Duration::ZERO)
                .unwrap();
            for role in ConversationProviderRole::ALL {
                let mut reservation = reserve_role(&mut budget, id, role).unwrap();
                reservation.consume().unwrap();
                assert!(matches!(
                    reserve_role(&mut budget, id, role),
                    Err(ContractError::DuplicateProviderRole)
                ));
                assert_eq!(budget.provider_requests(role), (child - 1) * 8 + offset);
            }
        }
    }
    for role in ConversationProviderRole::ALL {
        assert_eq!(budget.provider_requests(role), 16);
    }
    // A seventeenth turn cannot enter the interaction to acquire another request.
    budget.reserve_child(session(3)).unwrap();
    assert!(matches!(
        budget.reserve_turn(session(3), turn(17), Duration::ZERO),
        Err(ContractError::LimitExceeded)
    ));
    assert!(matches!(
        budget.reserve_provider(turn(17), ConversationProviderRole::Tts),
        Err(ContractError::UnknownTurn)
    ));
}

#[test]
fn speech_capacity_is_one_active_one_pending_with_bounded_total_text() {
    assert_eq!((MAX_ACTIVE_SPEECH_JOBS, MAX_PENDING_SPEECH_JOBS), (1, 1));
    assert_eq!(MAX_QUEUED_SPEECH_TEXT_BYTES, 8192);
    let mut capacity = SpeechCapacity::new();
    let text = "é".repeat(2048);
    let active = capacity
        .occupy(
            SpeechSlot::Active,
            job(1),
            &text,
            MAX_SPEECH_JOB_AUDIO_BYTES,
        )
        .unwrap();
    let pending = capacity
        .occupy(
            SpeechSlot::Pending,
            job(2),
            &text,
            MAX_SPEECH_JOB_AUDIO_BYTES,
        )
        .unwrap();
    assert_eq!(capacity.usage(), (1, 1, 8192));
    assert!(matches!(
        capacity.occupy(SpeechSlot::Active, job(3), "a", 1),
        Err(ContractError::LimitExceeded)
    ));
    assert!(matches!(
        capacity.occupy(SpeechSlot::Pending, job(4), "a", 1),
        Err(ContractError::LimitExceeded)
    ));
    assert_eq!(capacity.usage(), (1, 1, 8192));
    capacity.release(&active).unwrap();
    assert_eq!(capacity.usage(), (0, 1, 4096));
    let replacement = capacity
        .occupy(SpeechSlot::Active, job(3), &text, 1)
        .unwrap();
    capacity.release(&pending).unwrap();
    let next_pending = capacity
        .occupy(SpeechSlot::Pending, job(4), &text, 1)
        .unwrap();
    assert_eq!(capacity.usage(), (1, 1, 8192));
    capacity.release(&replacement).unwrap();
    capacity.release(&next_pending).unwrap();
    assert_eq!(capacity.usage(), (0, 0, 0));
}

#[test]
fn speech_release_rejects_duplicate_stale_and_foreign_receipts() {
    let mut capacity = SpeechCapacity::new();
    let old = capacity
        .occupy(SpeechSlot::Active, job(1), "old", 1)
        .unwrap();
    capacity.release(&old).unwrap();
    assert_eq!(capacity.release(&old), Err(ContractError::StaleSpeechLease));
    // Same slot and job ID still require the new occupancy sequence.
    let current = capacity
        .occupy(SpeechSlot::Active, job(1), "current", 1)
        .unwrap();
    assert_eq!(capacity.release(&old), Err(ContractError::StaleSpeechLease));
    assert_eq!(capacity.usage(), (1, 0, 7));
    let mut other = SpeechCapacity::new();
    let foreign = other
        .occupy(SpeechSlot::Active, job(1), "foreign", 1)
        .unwrap();
    assert_eq!(
        capacity.release(&foreign),
        Err(ContractError::StaleSpeechLease)
    );
    assert_eq!(
        other.release(&current),
        Err(ContractError::StaleSpeechLease)
    );
    assert_eq!(capacity.usage(), (1, 0, 7));
    assert_eq!(other.usage(), (1, 0, 7));
    capacity.release(&current).unwrap();
    other.release(&foreign).unwrap();
}

#[test]
fn speech_job_bounds_and_identity_fail_atomically_and_drop_does_not_release() {
    let mut capacity = SpeechCapacity::new();
    assert!(matches!(
        capacity.occupy(SpeechSlot::Active, job(1), &"é".repeat(2049), 1),
        Err(ContractError::LimitExceeded)
    ));
    assert!(matches!(
        capacity.occupy(
            SpeechSlot::Active,
            job(1),
            "ok",
            MAX_SPEECH_JOB_AUDIO_BYTES + 1
        ),
        Err(ContractError::LimitExceeded)
    ));
    assert_eq!(capacity.usage(), (0, 0, 0));
    let active = capacity
        .occupy(SpeechSlot::Active, job(1), "ok", 1)
        .unwrap();
    assert!(matches!(
        capacity.occupy(SpeechSlot::Pending, job(1), "ok", 1),
        Err(ContractError::DuplicateJob)
    ));
    assert_eq!(capacity.usage(), (1, 0, 2));
    capacity.release(&active).unwrap();
    let pending = capacity
        .occupy(SpeechSlot::Pending, job(1), "ok", 1)
        .unwrap();
    assert_eq!(
        capacity.release(&active),
        Err(ContractError::StaleSpeechLease)
    );
    drop(pending);
    assert_eq!(capacity.usage(), (0, 1, 2));
    assert!(matches!(
        capacity.occupy(SpeechSlot::Pending, job(2), "ok", 1),
        Err(ContractError::LimitExceeded)
    ));
}

#[test]
fn deterministic_deadline_expires_at_equality_and_afterwards() {
    let mut clock = ManualClock::new();
    let start = clock.now();
    let deadline = Deadline::after(start.clone(), FOLLOW_UP_WINDOW).unwrap();
    assert!(!deadline.is_expired(&start).unwrap());
    assert_eq!(deadline.remaining(&start).unwrap(), Duration::from_secs(8));
    clock
        .advance(Duration::from_secs(8) - Duration::from_nanos(1))
        .unwrap();
    assert!(!deadline.is_expired(&clock.now()).unwrap());
    assert_eq!(
        deadline.remaining(&clock.now()).unwrap(),
        Duration::from_nanos(1)
    );
    clock.advance(Duration::from_nanos(1)).unwrap();
    assert!(deadline.is_expired(&clock.now()).unwrap());
    assert_eq!(deadline.remaining(&clock.now()).unwrap(), Duration::ZERO);
    clock.advance(Duration::from_secs(1)).unwrap();
    assert!(deadline.is_expired(&clock.now()).unwrap());
    assert_eq!(deadline.remaining(&clock.now()).unwrap(), Duration::ZERO);
    assert_eq!(
        clock.now().checked_duration_since(&start).unwrap(),
        Duration::from_secs(9)
    );
    assert_eq!(
        start.checked_duration_since(&clock.now()),
        Err(ContractError::TimeReversed)
    );
}

#[test]
fn zero_duration_deadline_and_clock_overflow_are_safe() {
    let mut clock = ManualClock::new();
    assert!(Deadline::after(clock.now(), Duration::ZERO)
        .unwrap()
        .is_expired(&clock.now())
        .unwrap());
    clock.advance(Duration::MAX).unwrap();
    let at_max = clock.now();
    assert_eq!(
        clock.advance(Duration::from_nanos(1)),
        Err(ContractError::Overflow)
    );
    assert_eq!(clock.now().checked_cmp(&at_max).unwrap(), Ordering::Equal);
    assert_eq!(
        Deadline::after(at_max.clone(), Duration::from_nanos(1)).unwrap_err(),
        ContractError::Overflow
    );
    clock.advance(Duration::ZERO).unwrap();
    assert_eq!(clock.now().checked_cmp(&at_max).unwrap(), Ordering::Equal);
}

#[test]
fn child_creation_does_not_extend_an_interaction_deadline() {
    let mut clock = ManualClock::new();
    let deadline = Deadline::after(clock.now(), MAX_CLOUD_INTERACTION).unwrap();
    let mut budget = interaction();
    budget.reserve_child(session(1)).unwrap();
    clock.advance(Duration::from_secs(899)).unwrap();
    budget.reserve_child(session(2)).unwrap();
    assert_eq!(
        deadline.remaining(&clock.now()).unwrap(),
        Duration::from_secs(1)
    );
    clock.advance(Duration::from_secs(1)).unwrap();
    assert!(deadline.is_expired(&clock.now()).unwrap());
}

#[test]
fn native_clock_reads_are_monotonic_without_starting_workers() {
    let clock = NativeMonotonicClock::new();
    let earlier = clock.now();
    let later = clock.now();
    assert_ne!(later.checked_cmp(&earlier).unwrap(), Ordering::Less);
    assert!(later.checked_duration_since(&earlier).is_ok());
}

#[test]
fn foreign_clock_domains_reject_timestamp_and_deadline_comparisons() {
    let first = ManualClock::new();
    let mut second = ManualClock::new();
    let first_time = first.now();
    let second_time = second.now();
    for (a, b) in [(&first_time, &second_time), (&second_time, &first_time)] {
        assert_eq!(a.checked_cmp(b), Err(ContractError::ClockDomainMismatch));
        assert_eq!(
            a.checked_duration_since(b),
            Err(ContractError::ClockDomainMismatch)
        );
    }
    let deadline = Deadline::after(first_time.clone(), Duration::from_secs(5)).unwrap();
    assert_eq!(
        deadline.is_expired(&second_time),
        Err(ContractError::ClockDomainMismatch)
    );
    assert_eq!(
        deadline.remaining(&second_time),
        Err(ContractError::ClockDomainMismatch)
    );
    second.advance(Duration::from_secs(10)).unwrap();
    assert_eq!(
        deadline.is_expired(&second.now()),
        Err(ContractError::ClockDomainMismatch)
    );
    let same_domain = first_time.checked_add(Duration::from_secs(5)).unwrap();
    assert!(deadline.is_expired(&same_domain).unwrap());
    assert_eq!(
        same_domain.checked_duration_since(&first_time).unwrap(),
        Duration::from_secs(5)
    );
}

#[test]
fn native_manual_and_recreated_clocks_never_share_a_domain() {
    let native = NativeMonotonicClock::new();
    let another_native = NativeMonotonicClock::new();
    let manual = ManualClock::new();
    assert_eq!(
        native.now().checked_cmp(&another_native.now()),
        Err(ContractError::ClockDomainMismatch)
    );
    assert_eq!(
        native.now().checked_cmp(&manual.now()),
        Err(ContractError::ClockDomainMismatch)
    );
    let old = manual.now();
    drop(manual);
    let replacement = ManualClock::new();
    assert_eq!(
        old.checked_cmp(&replacement.now()),
        Err(ContractError::ClockDomainMismatch)
    );
}
