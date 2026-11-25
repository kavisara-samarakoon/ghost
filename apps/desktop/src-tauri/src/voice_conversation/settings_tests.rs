use super::*;
use crate::voice_conversation::clock::{ManualClock, MonotonicClock};
use crate::voice_conversation::model::*;
use serde_json::{json, Value};

fn nz(n: u64) -> NonZeroU64 {
    NonZeroU64::new(n).unwrap()
}
fn context() -> NativeContext {
    NativeContext {
        identity: ContractIdentity {
            version: ContractVersion::V1,
            app_run_id: AppRunId::from_native_sequence(1).unwrap(),
            interaction_id: InteractionId::from_native_sequence(1).unwrap(),
            session_id: SessionId::from_native_sequence(1).unwrap(),
            turn_id: TurnId::from_native_sequence(1).unwrap(),
            job_id: JobId::from_native_sequence(1).unwrap(),
            generation: Generation::from_native_sequence(1).unwrap(),
            configuration_revision: ConfigurationRevision::from_native_sequence(1).unwrap(),
            consent_binding_id: ConsentBindingId::from_native_sequence(1).unwrap(),
        },
        os_session: nz(1),
        permission_revision: nz(1),
        policy_revision: nz(1),
    }
}
struct Catalog {
    profiles: [RegisteredProfile; 8],
    voices: [VoiceId; 2],
    wake: [WakeReference; 2],
    languages: [LanguageId; 2],
}
impl Catalog {
    fn new() -> Self {
        Self {
            profiles: std::array::from_fn(|i| RegisteredProfile {
                role: ProviderRole::ALL[i / 2],
                reference: ProfileReference {
                    profile: ProfileId(nz(i as u64 + 1)),
                    model: ModelId(nz(i as u64 + 11)),
                    credential: Some(CredentialReference {
                        id: CredentialId(nz(i as u64 + 21)),
                        revision: nz(1),
                    }),
                    privacy_revision: nz(1),
                },
            }),
            voices: [VoiceId(nz(1)), VoiceId(nz(2))],
            wake: std::array::from_fn(|i| WakeReference {
                model: WakeModelId(nz(i as u64 + 1)),
                checksum: [i as u8 + 1; 32],
                thresholds: ThresholdId(nz(i as u64 + 1)),
            }),
            languages: [LanguageId(nz(1)), LanguageId(nz(2))],
        }
    }
    fn view(&self) -> FixtureCatalog<'_> {
        FixtureCatalog {
            profiles: &self.profiles,
            voices: &self.voices,
            wake_models: &self.wake,
            languages: &self.languages,
        }
    }
}
struct Fixture {
    native: NativeSettings,
    clock: ManualClock,
    presenter: presenter::TestPresenter,
    catalog: Catalog,
}
impl Fixture {
    fn new() -> Self {
        let clock = ManualClock::new();
        let presenter = presenter::TestPresenter::synthetic();
        let native = NativeSettings::synthetic(context(), &presenter, clock.now());
        Self {
            native,
            clock,
            presenter,
            catalog: Catalog::new(),
        }
    }
    fn candidate(&self) -> SettingsDescription {
        let mut settings = self.native.current.clone();
        settings.revision = settings.revision.checked_next().unwrap();
        settings
    }
    fn prepare(&mut self, settings: SettingsDescription) -> Result<Review> {
        let bytes = serde_json::to_vec(&settings).unwrap();
        self.native.prepare(
            decode(&bytes, bytes.len())?,
            self.native.context,
            &self.catalog.view(),
            &self.clock.now(),
        )
    }
    fn confirm(&mut self, review: &Review) -> Result<ChangeRequirements> {
        let receipt = self.presenter.receipt(review, presenter::Decision::Confirm);
        self.native.confirm(
            &receipt,
            self.native.context,
            &self.catalog.view(),
            &self.clock.now(),
        )
    }
}
fn decode_value(value: Value) -> Result<UntrustedSettings> {
    let bytes = serde_json::to_vec(&value).unwrap();
    decode(&bytes, bytes.len())
}

#[test]
fn privacy_defaults_have_no_provider_selection_arming_permission_or_output_enablement() {
    let s = SettingsDescription::privacy_default();
    assert_eq!(s.profiles, Profiles::default());
    assert_eq!(s.visibility, FeatureVisibility::default());
    assert!(!s.speaker_output_preference);
    assert!(s.wake_model.is_none() && s.installed_voice.is_none());
    let v = serde_json::to_value(&s).unwrap();
    for prohibited in [
        "grants",
        "permissions",
        "armed",
        "listening",
        "background_enabled",
        "cloud_consent",
        "history",
        "api_key",
    ] {
        assert!(v.get(prohibited).is_none());
    }
    assert_eq!(
        s.consent_presentation,
        ConsentPresentation::CompleteNativeReview
    );
}

#[test]
fn settings_version_one_roundtrips_as_data_only() {
    let s = SettingsDescription::privacy_default();
    let bytes = serde_json::to_vec(&s).unwrap();
    assert_eq!(decode(&bytes, bytes.len()).unwrap().0, s);
    assert_eq!(serde_json::to_value(&s).unwrap()["version"], 1);
}

#[test]
fn unsupported_invalid_and_noninteger_versions_fail_closed() {
    for version in [
        json!(0),
        json!(2),
        json!(255),
        json!(-1),
        json!(1.5),
        json!(true),
        json!("1"),
    ] {
        let mut v = serde_json::to_value(SettingsDescription::privacy_default()).unwrap();
        v["version"] = version;
        assert!(matches!(decode_value(v), Err(Error::Malformed)));
    }
}

#[test]
fn unknown_sensitive_runtime_and_execution_fields_are_rejected() {
    for field in [
        "api_key",
        "oauth_token",
        "refresh_token",
        "transcript",
        "audio",
        "history",
        "grants",
        "permission",
        "armed",
        "listening",
        "background_enabled",
        "endpoint",
        "execute",
        "automatic_retry",
    ] {
        let mut v = serde_json::to_value(SettingsDescription::privacy_default()).unwrap();
        v[field] = json!("sensitive-fixture-do-not-echo");
        let error = decode_value(v).err().unwrap();
        assert_eq!(error, Error::Malformed);
        assert!(!format!("{error:?}").contains("sensitive-fixture"));
    }
}

#[test]
fn nested_unknown_fields_and_duplicate_keys_fail_closed() {
    let mut v = serde_json::to_value(SettingsDescription::privacy_default()).unwrap();
    v["profiles"]["endpoint"] = json!("https://untrusted.invalid");
    assert!(matches!(decode_value(v), Err(Error::Malformed)));
    let mut v = serde_json::to_value(SettingsDescription::privacy_default()).unwrap();
    v["visibility"]["arm_wake"] = json!(true);
    assert!(matches!(decode_value(v), Err(Error::Malformed)));
    let text = serde_json::to_string(&SettingsDescription::privacy_default())
        .unwrap()
        .replacen("\"version\":1", "\"version\":1,\"version\":1", 1);
    assert!(matches!(
        decode(text.as_bytes(), text.len()),
        Err(Error::Malformed)
    ));
}

#[test]
fn fixed_phase0_bounds_cannot_be_overridden_by_unapproved_values() {
    let s = SettingsDescription::privacy_default();
    assert_eq!(s.bounds, ApprovedBounds::Phase0V1);
    for bounds in [
        json!("phase0_v2"),
        json!({"max_turns": 1000}),
        json!("unbounded"),
    ] {
        let mut v = serde_json::to_value(&s).unwrap();
        v["bounds"] = bounds;
        assert!(matches!(decode_value(v), Err(Error::Malformed)));
    }
    let mut v = serde_json::to_value(s).unwrap();
    v["consent_presentation"] = json!("skip_review");
    assert!(matches!(decode_value(v), Err(Error::Malformed)));
}

#[test]
fn exact_fixture_input_byte_limit_is_checked_before_parsing() {
    let bytes = serde_json::to_vec(&SettingsDescription::privacy_default()).unwrap();
    assert!(decode(&bytes, bytes.len()).is_ok());
    assert!(matches!(
        decode(&bytes, bytes.len() - 1),
        Err(Error::TooLarge)
    ));
    assert!(matches!(decode(b"", 0), Err(Error::Malformed)));
    assert!(matches!(decode(b"[]", 2), Err(Error::Malformed)));
}

#[test]
fn zero_negative_or_string_references_and_revisions_are_not_native_ids() {
    for revision in [json!(0), json!(-1), json!("secret-value")] {
        let mut v = serde_json::to_value(SettingsDescription::privacy_default()).unwrap();
        v["revision"] = revision;
        assert!(matches!(decode_value(v), Err(Error::Malformed)));
    }
    let mut v = serde_json::to_value(SettingsDescription::privacy_default()).unwrap();
    v["installed_voice"] = json!("api-key-fixture");
    assert!(matches!(decode_value(v), Err(Error::Malformed)));
}

#[test]
fn deserialized_settings_do_not_update_native_state_or_issue_confirmation() {
    let mut f = Fixture::new();
    let mut candidate = f.candidate();
    candidate.visibility = FeatureVisibility {
        conversation: true,
        wake: true,
        speech: true,
    };
    candidate.speaker_output_preference = true;
    candidate.profiles.conversation = Some(f.catalog.profiles[2].reference);
    let before = f.native.current.clone();
    let bytes = serde_json::to_vec(&candidate).unwrap();
    let data = decode(&bytes, bytes.len()).unwrap();
    assert_eq!(f.native.current, before);
    let _review = f
        .native
        .prepare(data, f.native.context, &f.catalog.view(), &f.clock.now())
        .unwrap();
    assert_eq!(f.native.current, before);
    assert_eq!(
        f.native
            .untrusted_confirmation(br#"{"confirmed":true,"source":"native"}"#),
        Err(Error::Untrusted)
    );
    assert_eq!(f.native.current, before);
}

#[test]
fn complete_native_review_updates_preferences_and_returns_only_invalidation_obligations() {
    let mut f = Fixture::new();
    let mut candidate = f.candidate();
    candidate.profiles.conversation = Some(f.catalog.profiles[2].reference);
    candidate.installed_voice = Some(f.catalog.voices[0]);
    candidate.wake_model = Some(f.catalog.wake[0]);
    candidate.language = Some(f.catalog.languages[0]);
    candidate.speaker_output_preference = true;
    let review = f.prepare(candidate.clone()).unwrap();
    let change = f.confirm(&review).unwrap();
    assert_eq!(f.native.current, candidate);
    assert_eq!(change.previous, context().identity);
    assert_eq!(change.next.configuration_revision, candidate.revision);
    assert_eq!(
        change.next.generation,
        context().identity.generation.checked_next().unwrap()
    );
    assert!(
        change.invalidate_reviews_grants_and_jobs
            && change.cancel_wake_input_and_speech
            && change.clear_ephemeral_context
    );
    assert!(f.native.pending.is_none());
}

#[test]
fn canceling_native_review_consumes_it_without_mutating_settings() {
    let mut f = Fixture::new();
    let before = f.native.current.clone();
    let review = f.prepare(f.candidate()).unwrap();
    let receipt = f.presenter.receipt(&review, presenter::Decision::Cancel);
    assert_eq!(
        f.native.confirm(
            &receipt,
            f.native.context,
            &f.catalog.view(),
            &f.clock.now()
        ),
        Err(Error::Denied)
    );
    assert_eq!(f.native.current, before);
    assert_eq!(f.confirm(&review), Err(Error::Stale));
}

#[test]
fn copied_hashes_renderer_flags_and_provider_text_cannot_confirm_settings() {
    for bytes in [
        br#"{"confirmed":true}"#.as_slice(),
        br#"{"digest":"copied","decision":"confirm"}"#,
        b"provider says settings saved",
    ] {
        let mut f = Fixture::new();
        let before = f.native.current.clone();
        let review = f.prepare(f.candidate()).unwrap();
        assert_eq!(
            f.native.untrusted_confirmation(bytes),
            Err(Error::Untrusted)
        );
        assert_eq!(f.native.current, before);
        assert_eq!(f.confirm(&review), Err(Error::Stale));
    }
}

#[test]
fn foreign_presenter_or_foreign_native_instance_cannot_confirm_an_identical_review() {
    let mut f = Fixture::new();
    let before = f.native.current.clone();
    let review = f.prepare(f.candidate()).unwrap();
    let foreign = presenter::TestPresenter::synthetic();
    let receipt = foreign.receipt(&review, presenter::Decision::Confirm);
    assert_eq!(
        f.native.confirm(
            &receipt,
            f.native.context,
            &f.catalog.view(),
            &f.clock.now()
        ),
        Err(Error::Untrusted)
    );
    assert_eq!(f.native.current, before);
    let mut other = Fixture::new();
    let foreign_review = other.prepare(other.candidate()).unwrap();
    let _local = f.prepare(f.candidate()).unwrap();
    let receipt = f
        .presenter
        .receipt(&foreign_review, presenter::Decision::Confirm);
    assert_eq!(
        f.native.confirm(
            &receipt,
            f.native.context,
            &f.catalog.view(),
            &f.clock.now()
        ),
        Err(Error::Untrusted)
    );
}

#[test]
fn successful_review_and_old_configuration_cannot_replay() {
    let mut f = Fixture::new();
    let old_context = f.native.context;
    let review = f.prepare(f.candidate()).unwrap();
    f.confirm(&review).unwrap();
    assert_eq!(f.confirm(&review), Err(Error::Stale));
    assert!(matches!(
        f.native.prepare(
            UntrustedSettings(f.native.current.clone()),
            old_context,
            &f.catalog.view(),
            &f.clock.now()
        ),
        Err(Error::Stale)
    ));
    assert!(matches!(
        f.prepare(f.native.current.clone()),
        Err(Error::Stale)
    ));
}

#[test]
fn skipped_stale_or_reused_configuration_revisions_are_rejected() {
    let mut f = Fixture::new();
    for revision in [1, 3, u64::MAX] {
        let mut candidate = f.candidate();
        candidate.revision = ConfigurationRevision::from_native_sequence(revision).unwrap();
        assert!(matches!(f.prepare(candidate), Err(Error::Stale)));
    }
}

#[test]
fn only_one_settings_review_can_be_pending() {
    let mut f = Fixture::new();
    let review = f.prepare(f.candidate()).unwrap();
    assert!(matches!(f.prepare(f.candidate()), Err(Error::Busy)));
    f.confirm(&review).unwrap();
}

#[test]
fn expired_review_is_consumed_at_exact_five_minutes() {
    let mut f = Fixture::new();
    let before = f.native.current.clone();
    let review = f.prepare(f.candidate()).unwrap();
    f.clock.advance(REVIEW_LIFETIME).unwrap();
    assert_eq!(f.confirm(&review), Err(Error::Expired));
    assert_eq!(f.confirm(&review), Err(Error::Stale));
    assert_eq!(f.native.current, before);
}

#[test]
fn expired_unused_review_can_be_replaced_by_a_fresh_explicit_review() {
    let mut f = Fixture::new();
    let old = f.prepare(f.candidate()).unwrap();
    f.clock.advance(REVIEW_LIFETIME).unwrap();
    let fresh = f.prepare(f.candidate()).unwrap();
    assert!(!Arc::ptr_eq(&old.0, &fresh.0));
    f.confirm(&fresh).unwrap();
}

#[test]
fn permission_or_policy_revision_changes_invalidate_pending_review_without_granting_permission() {
    for (permission, policy) in [(2, 1), (1, 2), (2, 2)] {
        let mut f = Fixture::new();
        let before = f.native.current.clone();
        let review = f.prepare(f.candidate()).unwrap();
        f.native
            .native_context_changed(nz(permission), nz(policy))
            .unwrap();
        assert_eq!(f.confirm(&review), Err(Error::Stale));
        assert_eq!(f.native.current, before);
        assert!(matches!(
            f.native.prepare(
                UntrustedSettings(f.candidate()),
                context(),
                &f.catalog.view(),
                &f.clock.now()
            ),
            Err(Error::Stale)
        ));
    }
}

#[test]
fn every_native_identity_and_permission_binding_is_rechecked_on_confirmation() {
    let mutations: [fn(&mut NativeContext); 11] = [
        |c| c.identity.app_run_id = AppRunId::from_native_sequence(2).unwrap(),
        |c| c.identity.interaction_id = InteractionId::from_native_sequence(2).unwrap(),
        |c| c.identity.session_id = SessionId::from_native_sequence(2).unwrap(),
        |c| c.identity.turn_id = TurnId::from_native_sequence(2).unwrap(),
        |c| c.identity.job_id = JobId::from_native_sequence(2).unwrap(),
        |c| c.identity.generation = Generation::from_native_sequence(2).unwrap(),
        |c| {
            c.identity.configuration_revision =
                ConfigurationRevision::from_native_sequence(2).unwrap()
        },
        |c| c.identity.consent_binding_id = ConsentBindingId::from_native_sequence(2).unwrap(),
        |c| c.os_session = nz(2),
        |c| c.permission_revision = nz(2),
        |c| c.policy_revision = nz(2),
    ];
    for mutate in mutations {
        let mut f = Fixture::new();
        let before = f.native.current.clone();
        let review = f.prepare(f.candidate()).unwrap();
        let receipt = f.presenter.receipt(&review, presenter::Decision::Confirm);
        let mut changed = f.native.context;
        mutate(&mut changed);
        assert_eq!(
            f.native
                .confirm(&receipt, changed, &f.catalog.view(), &f.clock.now()),
            Err(Error::Stale)
        );
        assert_eq!(f.native.current, before);
    }
}

#[test]
fn registered_profiles_are_bound_to_role_model_credential_and_privacy_revision() {
    for change in 0..5 {
        let mut f = Fixture::new();
        let mut candidate = f.candidate();
        let mut reference = f.catalog.profiles[2].reference;
        match change {
            0 => reference.profile = ProfileId(nz(99)),
            1 => reference.model = ModelId(nz(99)),
            2 => reference.credential.as_mut().unwrap().id = CredentialId(nz(99)),
            3 => reference.credential.as_mut().unwrap().revision = nz(2),
            _ => reference.privacy_revision = nz(2),
        }
        candidate.profiles.conversation = Some(reference);
        assert!(matches!(f.prepare(candidate), Err(Error::InvalidReference)));
    }
    let mut f = Fixture::new();
    let mut candidate = f.candidate();
    candidate.profiles.stt = Some(f.catalog.profiles[2].reference);
    assert!(matches!(f.prepare(candidate), Err(Error::InvalidReference)));
}

#[test]
fn wake_model_checksum_and_calibration_references_must_match_the_native_fixture() {
    for field in 0..3 {
        let mut f = Fixture::new();
        let mut candidate = f.candidate();
        let mut wake = f.catalog.wake[0];
        match field {
            0 => wake.model = WakeModelId(nz(99)),
            1 => wake.checksum[0] ^= 1,
            _ => wake.thresholds = ThresholdId(nz(99)),
        }
        candidate.wake_model = Some(wake);
        assert!(matches!(f.prepare(candidate), Err(Error::InvalidReference)));
    }
}

#[test]
fn unknown_installed_voice_or_language_is_not_silently_selected() {
    let mut f = Fixture::new();
    let mut candidate = f.candidate();
    candidate.installed_voice = Some(VoiceId(nz(99)));
    assert!(matches!(f.prepare(candidate), Err(Error::InvalidReference)));
    let mut candidate = f.candidate();
    candidate.language = Some(LanguageId(nz(99)));
    assert!(matches!(f.prepare(candidate), Err(Error::InvalidReference)));
}

#[test]
fn registry_changes_between_preview_and_confirmation_cannot_silently_switch_models() {
    let mut f = Fixture::new();
    let before = f.native.current.clone();
    let mut candidate = f.candidate();
    candidate.profiles.conversation = Some(f.catalog.profiles[2].reference);
    let review = f.prepare(candidate).unwrap();
    f.catalog.profiles[2].reference.model = ModelId(nz(99));
    assert_eq!(f.confirm(&review), Err(Error::InvalidReference));
    assert_eq!(f.native.current, before);
}

#[test]
fn provider_model_speaker_and_wake_changes_all_require_fresh_independent_authority() {
    let mut f = Fixture::new();
    for change in 0..6 {
        let mut candidate = f.candidate();
        match change {
            0 => candidate.profiles.conversation = Some(f.catalog.profiles[2].reference),
            1 => candidate.profiles.conversation = Some(f.catalog.profiles[3].reference),
            2 => candidate.installed_voice = Some(f.catalog.voices[0]),
            3 => candidate.wake_model = Some(f.catalog.wake[0]),
            4 => candidate.language = Some(f.catalog.languages[1]),
            _ => candidate.speaker_output_preference = true,
        }
        let review = f.prepare(candidate).unwrap();
        let requirements = f.confirm(&review).unwrap();
        assert!(
            requirements.invalidate_reviews_grants_and_jobs
                && requirements.cancel_wake_input_and_speech
                && requirements.clear_ephemeral_context
        );
    }
}

#[test]
fn manual_m45_has_no_capture_policy_override_in_settings() {
    let mut v = serde_json::to_value(SettingsDescription::privacy_default()).unwrap();
    v["manual_background_capture"] = json!(true);
    assert!(matches!(decode_value(v), Err(Error::Malformed)));
    let mut v = serde_json::to_value(SettingsDescription::privacy_default()).unwrap();
    v["manual_auto_send"] = json!(true);
    assert!(matches!(decode_value(v), Err(Error::Malformed)));
}

#[test]
fn clock_domain_errors_and_rollback_close_review_admission_without_changes() {
    for foreign in [false, true] {
        let mut f = Fixture::new();
        let before = f.native.current.clone();
        let old = f.clock.now();
        let review = f.prepare(f.candidate()).unwrap();
        f.clock.advance(Duration::from_secs(1)).unwrap();
        f.native.observe(&f.clock.now()).unwrap();
        let bad = if foreign {
            ManualClock::new().now()
        } else {
            old
        };
        let receipt = f.presenter.receipt(&review, presenter::Decision::Confirm);
        assert!(matches!(
            f.native
                .confirm(&receipt, f.native.context, &f.catalog.view(), &bad),
            Err(Error::Contract(_))
        ));
        assert!(f.native.closed && f.native.pending.is_none());
        assert_eq!(f.native.current, before);
    }
}

#[test]
fn exhausted_review_sequence_never_wraps_or_creates_a_pending_review() {
    let mut f = Fixture::new();
    f.native.sequence = u64::MAX;
    assert!(matches!(
        f.prepare(f.candidate()),
        Err(Error::Contract(ContractError::Overflow))
    ));
    assert!(f.native.closed && f.native.pending.is_none());
    assert_eq!(f.native.sequence, u64::MAX);
}

#[test]
fn exhausted_configuration_revision_never_resets_or_mutates_settings() {
    let mut f = Fixture::new();
    f.native.current.revision = ConfigurationRevision::from_native_sequence(u64::MAX).unwrap();
    f.native.context.identity.configuration_revision = f.native.current.revision;
    let candidate = f.native.current.clone();
    let before = candidate.clone();
    assert!(matches!(
        f.prepare(candidate),
        Err(Error::Contract(ContractError::Overflow))
    ));
    assert!(f.native.closed);
    assert_eq!(f.native.current, before);
}

#[test]
fn exhausted_generation_prevents_commit_after_native_confirmation() {
    let mut f = Fixture::new();
    f.native.context.identity.generation = Generation::from_native_sequence(u64::MAX).unwrap();
    let before = f.native.current.clone();
    let review = f.prepare(f.candidate()).unwrap();
    assert_eq!(
        f.confirm(&review),
        Err(Error::Contract(ContractError::Overflow))
    );
    assert!(f.native.closed);
    assert_eq!(f.native.current, before);
}

#[test]
fn deadline_overflow_does_not_create_a_review_or_change_preferences() {
    let mut f = Fixture::new();
    let before = f.native.current.clone();
    f.clock.advance(Duration::MAX).unwrap();
    assert!(matches!(
        f.prepare(f.candidate()),
        Err(Error::Contract(ContractError::Overflow))
    ));
    assert!(f.native.pending.is_none());
    assert_eq!(f.native.current, before);
}
