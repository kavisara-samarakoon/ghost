//! Synthetic-only native consent and payload fixtures. No real GHOST_HOME or I/O.
use super::*;
use crate::voice_conversation::clock::{ManualClock, MonotonicClock};

const PRIVATE_FIXTURE: &str = "synthetic reviewed private context";

fn context() -> Context {
    Context {
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
        os_session: OsSessionId::new(1).unwrap(),
        policy_revision: PolicyRevision::new(1).unwrap(),
    }
}

fn configuration() -> TrustedConfiguration {
    let specifications = Domain::ALL.map(|domain| {
        let (purpose, classes, requests, audio, text_bytes) = match domain {
            Domain::Microphone => (
                Purpose::ConversationCapture,
                vec![DataClass::PostCueAudio],
                0,
                MAX_CAPTURED_TURN,
                0,
            ),
            Domain::BackgroundWake => (
                Purpose::WakeDetection,
                vec![DataClass::AmbientAudio],
                0,
                WAKE_RING_BUFFER,
                0,
            ),
            Domain::CloudTranscription => (
                Purpose::Transcription,
                vec![DataClass::PostCueAudio],
                16,
                MAX_CLOUD_AUDIO,
                0,
            ),
            Domain::Conversation => (
                Purpose::OrdinaryConversation,
                vec![
                    DataClass::OrdinaryText,
                    DataClass::OrdinaryHistory,
                    DataClass::AssistantReply,
                ],
                16,
                Duration::ZERO,
                MAX_CONVERSATION_INPUT_BYTES,
            ),
            Domain::PrivateContext => (
                Purpose::PrivateDisclosure,
                vec![DataClass::PrivateSnapshot],
                1,
                Duration::ZERO,
                MAX_CONVERSATION_INPUT_BYTES,
            ),
            Domain::CloudTts => (
                Purpose::SpeechOutput,
                vec![DataClass::AssistantReply],
                16,
                Duration::ZERO,
                MAX_SPEECH_JOB_TEXT_BYTES,
            ),
        };
        let local = matches!(domain, Domain::Microphone | Domain::BackgroundWake);
        Specification {
            domain,
            purpose,
            target: if local {
                None
            } else {
                Some(Target {
                    provider: ProviderId::new(1).unwrap(),
                    model: ModelId::new(10).unwrap(),
                    voice: if domain == Domain::CloudTts {
                        Some(VoiceId::new(1).unwrap())
                    } else {
                        None
                    },
                })
            },
            classes,
            scope: Scope {
                background: true,
                project: if domain == Domain::PrivateContext {
                    Some(ProjectId::new(1).unwrap())
                } else {
                    None
                },
                snapshot: if domain == Domain::PrivateContext {
                    Some(ReviewedSnapshot {
                        text: PRIVATE_FIXTURE.into(),
                        digest: hash(PRIVATE_FIXTURE.as_bytes()),
                    })
                } else {
                    None
                },
            },
            limits: Limits {
                lifetime: if domain == Domain::BackgroundWake {
                    MAX_WAKE_ARMING
                } else {
                    MAX_CLOUD_INTERACTION
                },
                requests,
                audio,
                text_bytes,
            },
        }
    });
    TrustedConfiguration {
        revision_sequence: NonZeroU64::new(1).unwrap(),
        policy_revision: PolicyRevision::new(1).unwrap(),
        specifications,
    }
}

struct Fixture {
    clock: ManualClock,
    presenter: presenter::TestPresenter,
    context: Context,
    configuration: TrustedConfiguration,
    registry: ConsentRegistry,
    gate: EgressGate,
}
impl Fixture {
    fn new() -> Self {
        let clock = ManualClock::new();
        let presenter = presenter::TestPresenter::new();
        let context = context();
        let configuration = configuration();
        let registry = ConsentRegistry::new(
            configuration.clone(),
            context.clone(),
            &presenter,
            clock.now(),
        )
        .unwrap();
        let gate = EgressGate::new(&context, clock.now()).unwrap();
        Self {
            clock,
            presenter,
            context,
            configuration,
            registry,
            gate,
        }
    }
    fn issue(&mut self, domain: Domain) -> Grant {
        let preview = self
            .registry
            .prepare(domain, None, &self.clock.now())
            .unwrap();
        let receipt = self.presenter.interact(
            &self.presenter.show(&preview),
            presenter::NativeInteraction::Affirm,
            &self.context,
            &self.clock.now(),
        );
        self.registry
            .confirm(
                &preview,
                Confirmation::Native(&receipt),
                &self.context,
                &self.clock.now(),
            )
            .unwrap()
    }
    // Each exact turn gets its own real registry and genuine fake-native receipt.
    // The egress interaction ledger is retained; no grants are reconstructed.
    fn turn(&mut self, turn: u64, session: u64) {
        self.context.identity.turn_id = TurnId::from_native_sequence(turn).unwrap();
        self.context.identity.session_id = SessionId::from_native_sequence(session).unwrap();
        self.context.identity.job_id = JobId::from_native_sequence(turn).unwrap();
        self.registry = ConsentRegistry::new(
            self.configuration.clone(),
            self.context.clone(),
            &self.presenter,
            self.clock.now(),
        )
        .unwrap();
    }
    fn input(&self, domain: Domain) -> (Intent, synthetic::Payload) {
        let spec = &self.configuration.specifications[domain.index()];
        let (role, data, provenance, text, audio, duration) = match domain {
            Domain::CloudTranscription => (
                ProviderRole::Stt,
                DataClass::PostCueAudio,
                Provenance::PostCue,
                "",
                32,
                Duration::from_secs(1),
            ),
            Domain::Conversation => (
                ProviderRole::Conversation,
                DataClass::OrdinaryText,
                Provenance::Ordinary,
                "test",
                0,
                Duration::ZERO,
            ),
            Domain::CloudTts => (
                ProviderRole::Tts,
                DataClass::AssistantReply,
                Provenance::Ordinary,
                "reply",
                0,
                Duration::ZERO,
            ),
            Domain::PrivateContext => (
                ProviderRole::Conversation,
                DataClass::PrivateSnapshot,
                Provenance::Private,
                PRIVATE_FIXTURE,
                0,
                Duration::ZERO,
            ),
            _ => panic!("not an egress domain"),
        };
        (
            Intent {
                context: self.context.clone(),
                domain,
                purpose: spec.purpose,
                target: spec.target.unwrap(),
                role,
                data,
                provenance,
                background: false,
                project: spec.scope.project,
            },
            synthetic::Payload::fixture(
                self.context.identity,
                data,
                provenance,
                text,
                audio,
                duration,
            ),
        )
    }
    fn reserve(&mut self, grant: Option<&Grant>, domain: Domain) -> EgressResult<SealedJob> {
        let (intent, payload) = self.input(domain);
        self.gate.reserve(
            &mut self.registry,
            grant,
            intent,
            payload,
            &self.clock.now(),
        )
    }
    fn submit(&mut self, domain: Domain) -> (Grant, SealedJob) {
        let grant = self.issue(domain);
        let job = self.reserve(Some(&grant), domain).unwrap();
        (grant, job)
    }
    fn begin(&mut self, job: &SealedJob) -> EgressResult<DispatchTicket> {
        self.gate.begin(&mut self.registry, job, &self.clock.now())
    }
    fn finish(&mut self, ticket: &DispatchTicket, outcome: DispatchOutcome) -> EgressResult<()> {
        self.gate
            .finish(&mut self.registry, ticket, outcome, &self.clock.now())
    }
    fn usage(&self) -> (u64, [u64; 3], Duration, usize, usize, usize) {
        (
            self.gate.sequence,
            self.gate.roles,
            self.gate.cloud_audio,
            self.gate.children.len(),
            self.gate.turns.len(),
            self.gate.grants.len(),
        )
    }
}

fn error<T>(result: EgressResult<T>) -> EgressError {
    match result {
        Ok(_) => panic!("unexpected authority"),
        Err(error) => error,
    }
}

#[test]
fn native_consent_reserves_all_roles_and_seals_exact_payload_before_dispatch() {
    let mut f = Fixture::new();
    for domain in [
        Domain::CloudTranscription,
        Domain::Conversation,
        Domain::CloudTts,
    ] {
        let (grant, job) = f.submit(domain);
        assert!(Arc::ptr_eq(&job.0.grant.data, &grant.data));
        assert_eq!(job.0.intent.context, f.context);
        assert_eq!(
            job.0.intent.target,
            f.configuration.specifications[domain.index()]
                .target
                .unwrap()
        );
        assert_eq!(f.gate.jobs.last().unwrap().state, JobState::Reserved);
        let ticket = f.begin(&job).unwrap();
        assert!(f.finish(&ticket, DispatchOutcome::Succeeded).is_ok());
        assert_eq!(error(f.begin(&job)), EgressError::Stale);
        assert_eq!(
            error(f.finish(&ticket, DispatchOutcome::Succeeded)),
            EgressError::Stale
        );
    }
    assert_eq!(f.gate.roles, [1; 3]);
    assert_eq!(f.gate.cloud_audio, Duration::from_secs(1));
    assert_eq!(f.gate.children[0].turns, 1);
    assert_eq!(f.gate.grants[1].usage.text, 4);
}

#[test]
fn missing_expired_revoked_and_foreign_registry_grants_cannot_reserve() {
    let mut f = Fixture::new();
    assert_eq!(
        error(f.reserve(None, Domain::Conversation)),
        EgressError::MissingGrant
    );
    let grant = f.issue(Domain::Conversation);
    f.registry.revoke(&grant).unwrap();
    assert_eq!(
        error(f.reserve(Some(&grant), Domain::Conversation)),
        EgressError::Consent(Error::Revoked)
    );
    let mut foreign = Fixture::new();
    let grant = foreign.issue(Domain::Conversation);
    assert_eq!(
        error(f.reserve(Some(&grant), Domain::Conversation)),
        EgressError::Consent(Error::Revoked)
    );
    let grant = f.issue(Domain::Conversation);
    f.clock.advance(MAX_CLOUD_INTERACTION).unwrap();
    assert_eq!(
        error(f.reserve(Some(&grant), Domain::Conversation)),
        EgressError::Consent(Error::Expired)
    );
    assert_eq!(f.gate.sequence, 0);
}

#[test]
fn every_identity_field_is_revalidated_by_native_registry() {
    for field in 0..11 {
        let mut f = Fixture::new();
        let grant = f.issue(Domain::Conversation);
        let (mut intent, _) = f.input(Domain::Conversation);
        let id = &mut intent.context.identity;
        match field {
            0 => id.app_run_id = AppRunId::from_native_sequence(2).unwrap(),
            1 => id.interaction_id = InteractionId::from_native_sequence(2).unwrap(),
            2 => id.session_id = SessionId::from_native_sequence(2).unwrap(),
            3 => id.turn_id = TurnId::from_native_sequence(2).unwrap(),
            4 => id.job_id = JobId::from_native_sequence(2).unwrap(),
            5 => id.generation = Generation::from_native_sequence(2).unwrap(),
            6 => {
                id.configuration_revision = ConfigurationRevision::from_native_sequence(2).unwrap()
            }
            7 => id.consent_binding_id = ConsentBindingId::from_native_sequence(2).unwrap(),
            8 => intent.context.os_session = OsSessionId::new(2).unwrap(),
            9 => intent.context.policy_revision = PolicyRevision::new(2).unwrap(),
            10 => intent.project = Some(ProjectId::new(2).unwrap()),
            _ => unreachable!(),
        }
        let payload = synthetic::Payload::fixture(
            intent.context.identity,
            intent.data,
            intent.provenance,
            "test",
            0,
            Duration::ZERO,
        );
        assert!(
            f.gate
                .reserve(
                    &mut f.registry,
                    Some(&grant),
                    intent,
                    payload,
                    &f.clock.now()
                )
                .is_err(),
            "field {field}"
        );
        assert_eq!(f.gate.sequence, 0);
    }
}

#[test]
fn wrong_provider_model_voice_role_domain_and_purpose_fail_without_charges() {
    for field in 0..7 {
        let mut f = Fixture::new();
        let grant = f.issue(Domain::Conversation);
        let (mut intent, payload) = f.input(Domain::Conversation);
        match field {
            0 => intent.target.provider = ProviderId::new(2).unwrap(),
            1 => intent.target.model = ModelId::new(2).unwrap(),
            2 => intent.target.voice = Some(VoiceId::new(2).unwrap()),
            3 => intent.role = ProviderRole::Stt,
            4 => intent.role = ProviderRole::Planning,
            5 => intent.domain = Domain::CloudTts,
            6 => intent.purpose = Purpose::PrivateDisclosure,
            _ => unreachable!(),
        }
        let before = f.usage();
        assert!(f
            .gate
            .reserve(
                &mut f.registry,
                Some(&grant),
                intent,
                payload,
                &f.clock.now()
            )
            .is_err());
        assert_eq!(f.usage(), before);
    }
}

#[test]
fn duplicate_reservation_clone_replay_and_fresh_grant_do_not_retry_a_role() {
    let mut f = Fixture::new();
    let (grant, job) = f.submit(Domain::Conversation);
    let copy = grant.clone();
    let before = f.usage();
    assert_eq!(
        error(f.reserve(Some(&copy), Domain::Conversation)),
        EgressError::Duplicate
    );
    let fresh = f.issue(Domain::Conversation);
    assert_eq!(
        error(f.reserve(Some(&fresh), Domain::Conversation)),
        EgressError::Duplicate
    );
    drop(job);
    assert_eq!(
        error(f.reserve(Some(&grant), Domain::Conversation)),
        EgressError::Duplicate
    );
    assert_eq!(f.usage(), before);
}

#[test]
fn provider_switch_is_denied_even_with_new_native_grant_on_new_turn() {
    for model_switch in [false, true] {
        let mut f = Fixture::new();
        f.submit(Domain::Conversation);
        let spec = &mut f.configuration.specifications[Domain::Conversation.index()];
        if model_switch {
            spec.target.as_mut().unwrap().model = ModelId::new(2).unwrap();
        } else {
            spec.target.as_mut().unwrap().provider = ProviderId::new(2).unwrap();
        }
        f.turn(2, 1);
        let grant = f.issue(Domain::Conversation);
        let before = f.usage();
        assert_eq!(
            error(f.reserve(Some(&grant), Domain::Conversation)),
            EgressError::Binding
        );
        assert_eq!(f.usage(), before);
    }
}

#[test]
fn request_and_text_grant_budgets_are_checked_atomically() {
    for counter in 0..2 {
        let mut f = Fixture::new();
        let grant = f.issue(Domain::Conversation);
        let limits = grant.data.preview.content.specification.limits;
        // Fault-injected prior accounting tests the remaining-budget boundary.
        // Authority is still a real registry-issued handle, never reconstructed.
        f.gate.grants.push(GrantUsage {
            grant: grant.clone(),
            usage: Usage {
                requests: if counter == 0 { limits.requests } else { 0 },
                audio: Duration::ZERO,
                text: if counter == 1 {
                    limits.text_bytes - 3
                } else {
                    0
                },
            },
        });
        let before = f.usage();
        assert_eq!(
            error(f.reserve(Some(&grant), Domain::Conversation)),
            EgressError::Contract(ContractError::LimitExceeded)
        );
        assert_eq!(f.usage(), before);
        assert_eq!(
            f.gate.grants[0].usage.requests,
            if counter == 0 { limits.requests } else { 0 }
        );
    }
}

#[test]
fn per_role_request_budget_exhaustion_is_atomic() {
    let mut f = Fixture::new();
    let grant = f.issue(Domain::Conversation);
    f.gate.roles[1] = MAX_PROVIDER_REQUESTS_PER_ROLE_PER_INTERACTION;
    let before = f.usage();
    assert_eq!(
        error(f.reserve(Some(&grant), Domain::Conversation)),
        EgressError::Contract(ContractError::LimitExceeded)
    );
    assert_eq!(f.usage(), before);
}

#[test]
fn utf8_role_and_serialized_body_bounds_reject_without_partial_charge() {
    for (domain, data, text) in [
        (
            Domain::Conversation,
            DataClass::OrdinaryText,
            "é".repeat(4097),
        ),
        (
            Domain::CloudTts,
            DataClass::AssistantReply,
            "é".repeat(2049),
        ),
        (
            Domain::Conversation,
            DataClass::OrdinaryHistory,
            "x".repeat(MAX_CONVERSATION_INPUT_BYTES as usize + 1),
        ),
        (
            Domain::Conversation,
            DataClass::OrdinaryHistory,
            "\0".repeat(20_000),
        ),
    ] {
        let mut f = Fixture::new();
        let grant = f.issue(domain);
        let (mut intent, _) = f.input(domain);
        intent.data = data;
        let payload = synthetic::Payload::fixture(
            f.context.identity,
            data,
            Provenance::Ordinary,
            &text,
            0,
            Duration::ZERO,
        );
        assert!(f
            .gate
            .reserve(
                &mut f.registry,
                Some(&grant),
                intent,
                payload,
                &f.clock.now()
            )
            .is_err());
        assert_eq!(f.gate.sequence, 0);
        assert!(f.gate.children.is_empty());
        assert!(f.gate.grants.is_empty());
    }
}

#[test]
fn exact_text_role_boundaries_are_admitted() {
    for (domain, data, maximum) in [
        (
            Domain::Conversation,
            DataClass::OrdinaryText,
            MAX_USER_TRANSCRIPT_BYTES,
        ),
        (
            Domain::Conversation,
            DataClass::OrdinaryHistory,
            MAX_CONVERSATION_INPUT_BYTES,
        ),
        (
            Domain::CloudTts,
            DataClass::AssistantReply,
            MAX_SPEECH_JOB_TEXT_BYTES,
        ),
    ] {
        let mut f = Fixture::new();
        let grant = f.issue(domain);
        let (mut intent, _) = f.input(domain);
        intent.data = data;
        let payload = synthetic::Payload::fixture(
            f.context.identity,
            data,
            Provenance::Ordinary,
            &"x".repeat(maximum as usize),
            0,
            Duration::ZERO,
        );
        let job = f
            .gate
            .reserve(
                &mut f.registry,
                Some(&grant),
                intent,
                payload,
                &f.clock.now(),
            )
            .unwrap();
        assert_eq!(job.0.payload.text().len() as u64, maximum);
    }
}

#[test]
fn audio_duration_bytes_and_aggregate_cloud_budget_are_enforced() {
    for (bytes, duration) in [
        (MAX_ENCODED_AUDIO_BYTES as usize + 1, Duration::from_secs(1)),
        (32, MAX_CAPTURED_TURN + Duration::from_nanos(1)),
        (0, Duration::from_secs(1)),
        (32, Duration::ZERO),
    ] {
        let mut f = Fixture::new();
        let grant = f.issue(Domain::CloudTranscription);
        let (intent, _) = f.input(Domain::CloudTranscription);
        let payload = synthetic::Payload::fixture(
            f.context.identity,
            DataClass::PostCueAudio,
            Provenance::PostCue,
            "",
            bytes,
            duration,
        );
        assert!(f
            .gate
            .reserve(
                &mut f.registry,
                Some(&grant),
                intent,
                payload,
                &f.clock.now()
            )
            .is_err());
        assert_eq!(f.gate.cloud_audio, Duration::ZERO);
        assert_eq!(f.gate.sequence, 0);
    }
    let mut f = Fixture::new();
    for turn in 1..=4 {
        f.turn(turn, 1);
        let grant = f.issue(Domain::CloudTranscription);
        let (intent, _) = f.input(Domain::CloudTranscription);
        let payload = synthetic::Payload::fixture(
            f.context.identity,
            DataClass::PostCueAudio,
            Provenance::PostCue,
            "",
            32,
            MAX_CAPTURED_TURN,
        );
        f.gate
            .reserve(
                &mut f.registry,
                Some(&grant),
                intent,
                payload,
                &f.clock.now(),
            )
            .unwrap();
    }
    assert_eq!(f.gate.cloud_audio, MAX_CLOUD_AUDIO);
    f.turn(5, 1);
    let grant = f.issue(Domain::CloudTranscription);
    let before = f.usage();
    assert_eq!(
        error(f.reserve(Some(&grant), Domain::CloudTranscription)),
        EgressError::Contract(ContractError::LimitExceeded)
    );
    assert_eq!(f.usage(), before);
}

#[test]
fn aggregate_turn_child_turn_and_child_count_limits_survive_new_registries() {
    let mut f = Fixture::new();
    for turn in 1..=8 {
        f.turn(turn, 1);
        f.submit(Domain::Conversation);
    }
    f.turn(9, 1);
    let grant = f.issue(Domain::Conversation);
    assert_eq!(
        error(f.reserve(Some(&grant), Domain::Conversation)),
        EgressError::Contract(ContractError::LimitExceeded)
    );
    for turn in 9..=16 {
        f.turn(turn, 2);
        f.submit(Domain::Conversation);
    }
    assert_eq!(f.gate.roles[1], 16);
    f.turn(17, 3);
    let grant = f.issue(Domain::Conversation);
    assert!(f.reserve(Some(&grant), Domain::Conversation).is_err());
    assert_eq!(f.gate.roles[1], 16);

    let mut f = Fixture::new();
    for session in 1..=4 {
        f.turn(session, session);
        f.submit(Domain::Conversation);
    }
    f.turn(5, 5);
    let grant = f.issue(Domain::Conversation);
    assert_eq!(
        error(f.reserve(Some(&grant), Domain::Conversation)),
        EgressError::Contract(ContractError::LimitExceeded)
    );
    assert_eq!(f.gate.children.len(), 4);
}

#[test]
fn failed_and_uncertain_dispatches_permanently_consume_the_same_reservation() {
    for outcome in [DispatchOutcome::Failed, DispatchOutcome::Uncertain] {
        let mut f = Fixture::new();
        let (grant, job) = f.submit(Domain::Conversation);
        let ticket = f.begin(&job).unwrap();
        let before = f.usage();
        f.finish(&ticket, outcome).unwrap();
        assert_eq!(f.usage(), before);
        assert_eq!(
            error(f.reserve(Some(&grant), Domain::Conversation)),
            EgressError::Duplicate
        );
        assert_eq!(error(f.begin(&job)), EgressError::Stale);
        assert_eq!(
            error(f.finish(&ticket, DispatchOutcome::Succeeded)),
            EgressError::Stale
        );
    }
}

#[test]
fn cancellation_invalidates_reserved_jobs_and_late_callbacks_without_refunds() {
    let mut f = Fixture::new();
    let (grant, job) = f.submit(Domain::Conversation);
    let ticket = f.begin(&job).unwrap();
    let before = f.usage();
    f.gate.cancel_turn(&f.context);
    assert_eq!(
        error(f.finish(&ticket, DispatchOutcome::Succeeded)),
        EgressError::Stale
    );
    assert_eq!(error(f.begin(&job)), EgressError::Stale);
    assert_eq!(
        error(f.reserve(Some(&grant), Domain::Conversation)),
        EgressError::Duplicate
    );
    let tts = f.issue(Domain::CloudTts);
    assert_eq!(
        error(f.reserve(Some(&tts), Domain::CloudTts)),
        EgressError::Duplicate
    );
    assert_eq!(f.usage(), before);
}

#[test]
fn cancellation_before_first_reservation_is_also_permanent() {
    let mut f = Fixture::new();
    f.gate.cancel_turn(&f.context);
    let grant = f.issue(Domain::Conversation);
    assert_eq!(
        error(f.reserve(Some(&grant), Domain::Conversation)),
        EgressError::Duplicate
    );
    assert_eq!(f.gate.sequence, 0);
}

#[test]
fn stop_is_immediate_with_in_flight_work_and_needs_no_clock_or_callback() {
    let mut f = Fixture::new();
    let (grant, job) = f.submit(Domain::Conversation);
    let ticket = f.begin(&job).unwrap();
    let (_, pending) = f.submit(Domain::CloudTts);
    let before = f.usage();
    f.gate.sequence = u64::MAX;
    f.gate.stop();
    f.gate.stop();
    assert_eq!(error(f.begin(&pending)), EgressError::Stopped);
    assert_eq!(
        error(f.finish(&ticket, DispatchOutcome::Succeeded)),
        EgressError::Stopped
    );
    assert_eq!(
        error(f.reserve(Some(&grant), Domain::Conversation)),
        EgressError::Stopped
    );
    assert_eq!(f.gate.roles, before.1);
    assert!(f.gate.jobs.iter().all(|r| r.state == JobState::Canceled));
}

#[test]
fn revocation_after_reservation_and_after_attempt_is_rechecked() {
    for attempted in [false, true] {
        for revoke in 0..3 {
            let mut f = Fixture::new();
            let (grant, job) = f.submit(Domain::Conversation);
            let ticket = if attempted {
                Some(f.begin(&job).unwrap())
            } else {
                None
            };
            match revoke {
                0 => f.registry.revoke(&grant).unwrap(),
                1 => f.registry.revoke_domain(Domain::Conversation),
                2 => f.registry.revoke_all().unwrap(),
                _ => unreachable!(),
            }
            let before = f.usage();
            let result = if let Some(ticket) = ticket {
                f.finish(&ticket, DispatchOutcome::Succeeded)
            } else {
                f.begin(&job).map(|_| ())
            };
            assert_eq!(error(result), EgressError::Consent(Error::Revoked));
            assert_eq!(f.usage(), before);
            assert!(f.begin(&job).is_err());
        }
    }
}

#[test]
fn expiry_after_reservation_and_child_deadline_block_dispatch_and_new_jobs() {
    let mut f = Fixture::new();
    f.configuration.specifications[Domain::Conversation.index()]
        .limits
        .lifetime = Duration::from_secs(1);
    f.turn(1, 1);
    let (_, job) = f.submit(Domain::Conversation);
    f.clock.advance(Duration::from_secs(1)).unwrap();
    assert_eq!(error(f.begin(&job)), EgressError::Consent(Error::Expired));
    assert_eq!(f.gate.roles[1], 1);

    let mut f = Fixture::new();
    f.submit(Domain::Conversation);
    f.clock.advance(MAX_CHILD_CONVERSATION).unwrap();
    f.turn(2, 1);
    let grant = f.issue(Domain::Conversation);
    assert_eq!(
        error(f.reserve(Some(&grant), Domain::Conversation)),
        EgressError::Consent(Error::Expired)
    );
}

#[test]
fn foreign_job_and_callback_cannot_match_a_gate_by_copied_metadata() {
    let mut a = Fixture::new();
    let mut b = Fixture::new();
    let (_, job) = a.submit(Domain::Conversation);
    let ticket = a.begin(&job).unwrap();
    let (_, other) = b.submit(Domain::Conversation);
    assert_eq!(job.0.intent.context, other.0.intent.context);
    assert_eq!(error(b.begin(&job)), EgressError::Stale);
    assert_eq!(
        error(b.finish(&ticket, DispatchOutcome::Succeeded)),
        EgressError::Stale
    );
}

#[test]
fn private_google_ambient_planning_tools_and_workflow_never_gain_ordinary_authority() {
    for domain in [Domain::Conversation, Domain::CloudTts] {
        for provenance in [
            Provenance::Private,
            Provenance::DerivedPrivate,
            Provenance::GoogleDerived,
            Provenance::Ambient,
            Provenance::PostCue,
            Provenance::Planning,
            Provenance::Tools,
            Provenance::Workflow,
        ] {
            for data in [
                DataClass::OrdinaryText,
                DataClass::OrdinaryHistory,
                DataClass::AssistantReply,
                DataClass::PrivateSnapshot,
                DataClass::GoogleDerived,
                DataClass::AmbientAudio,
                DataClass::Planning,
                DataClass::Tools,
                DataClass::Workflow,
            ] {
                let mut f = Fixture::new();
                let grant = f.issue(domain);
                let (mut intent, _) = f.input(domain);
                intent.data = data;
                intent.provenance = provenance;
                let payload = synthetic::Payload::fixture(
                    f.context.identity,
                    data,
                    provenance,
                    "test",
                    0,
                    Duration::ZERO,
                );
                assert!(f
                    .gate
                    .reserve(
                        &mut f.registry,
                        Some(&grant),
                        intent,
                        payload,
                        &f.clock.now()
                    )
                    .is_err());
                assert_eq!(f.gate.sequence, 0);
            }
        }
    }
}

#[test]
fn ambient_audio_is_not_cloud_transcription_input() {
    let mut f = Fixture::new();
    let grant = f.issue(Domain::CloudTranscription);
    let (mut intent, _) = f.input(Domain::CloudTranscription);
    intent.provenance = Provenance::Ambient;
    let payload = synthetic::Payload::fixture(
        f.context.identity,
        DataClass::PostCueAudio,
        Provenance::Ambient,
        "",
        32,
        Duration::from_secs(1),
    );
    assert_eq!(
        error(f.gate.reserve(
            &mut f.registry,
            Some(&grant),
            intent,
            payload,
            &f.clock.now()
        )),
        EgressError::Consent(Error::Denied)
    );
}

#[test]
fn untrusted_provenance_declarations_and_relabeling_cannot_create_evidence() {
    assert_eq!(
        error(synthetic::Payload::from_untrusted(
            "renderer text",
            Provenance::Ordinary
        )),
        EgressError::UntrustedPayload
    );
    assert_eq!(
        error(synthetic::Payload::from_untrusted(
            "AI reply",
            Provenance::Ordinary
        )),
        EgressError::UntrustedPayload
    );
    let mut f = Fixture::new();
    let grant = f.issue(Domain::Conversation);
    let (intent, _) = f.input(Domain::Conversation);
    let payload = synthetic::Payload::fixture(
        f.context.identity,
        DataClass::OrdinaryText,
        Provenance::DerivedPrivate,
        "paraphrase",
        0,
        Duration::ZERO,
    );
    assert_eq!(
        error(f.gate.reserve(
            &mut f.registry,
            Some(&grant),
            intent,
            payload,
            &f.clock.now()
        )),
        EgressError::Binding
    );
}

#[test]
fn exact_reviewed_private_snapshot_has_separate_one_shot_authority() {
    let mut f = Fixture::new();
    let ordinary = f.issue(Domain::Conversation);
    assert!(f.reserve(Some(&ordinary), Domain::PrivateContext).is_err());
    let (grant, job) = f.submit(Domain::PrivateContext);
    assert_eq!(job.0.payload.text(), PRIVATE_FIXTURE);
    assert_eq!(f.gate.grants[0].usage.requests, 1);
    let ticket = f.begin(&job).unwrap();
    f.finish(&ticket, DispatchOutcome::Uncertain).unwrap();
    assert_eq!(
        error(f.reserve(Some(&grant), Domain::PrivateContext)),
        EgressError::Duplicate
    );
    assert!(f.reserve(Some(&grant), Domain::Conversation).is_err());
    assert_eq!(f.gate.roles[1], 1);
}

#[test]
fn changed_snapshot_google_derivation_and_wrong_target_cannot_use_private_review() {
    for variant in 0..4 {
        let mut f = Fixture::new();
        let grant = f.issue(Domain::PrivateContext);
        let (mut intent, _) = f.input(Domain::PrivateContext);
        let text = if variant == 0 {
            "synthetic changed private context!"
        } else {
            PRIVATE_FIXTURE
        };
        if variant == 1 {
            intent.provenance = Provenance::GoogleDerived;
        }
        if variant == 2 {
            intent.target.provider = ProviderId::new(2).unwrap();
        }
        if variant == 3 {
            intent.purpose = Purpose::OrdinaryConversation;
        }
        let payload = synthetic::Payload::fixture(
            f.context.identity,
            intent.data,
            intent.provenance,
            text,
            0,
            Duration::ZERO,
        );
        assert!(f
            .gate
            .reserve(
                &mut f.registry,
                Some(&grant),
                intent,
                payload,
                &f.clock.now()
            )
            .is_err());
        assert_eq!(f.gate.sequence, 0);
        // A rejected reservation does not spend the exact separately reviewed grant.
        assert!(f.reserve(Some(&grant), Domain::PrivateContext).is_ok());
    }
}

#[test]
fn planning_tools_and_workflow_purposes_cannot_be_issued_as_egress_authority() {
    for purpose in [Purpose::Planning, Purpose::Tools, Purpose::Workflow] {
        let mut f = Fixture::new();
        let grant = f.issue(Domain::Conversation);
        let (mut intent, payload) = f.input(Domain::Conversation);
        intent.purpose = purpose;
        assert!(f
            .gate
            .reserve(
                &mut f.registry,
                Some(&grant),
                intent,
                payload,
                &f.clock.now()
            )
            .is_err());
        assert_eq!(f.gate.sequence, 0);
    }
}

#[test]
fn counter_and_duration_overflow_fail_closed_without_partial_charges() {
    for counter in 0..6 {
        let mut f = Fixture::new();
        let domain = if counter == 4 || counter == 5 {
            Domain::CloudTranscription
        } else {
            Domain::Conversation
        };
        let grant = f.issue(domain);
        match counter {
            0 => f.gate.sequence = u64::MAX,
            1 => f.gate.roles[1] = u64::MAX,
            2 | 3 | 5 => f.gate.grants.push(GrantUsage {
                grant: grant.clone(),
                usage: Usage {
                    requests: if counter == 2 { u64::MAX } else { 0 },
                    text: if counter == 3 { u64::MAX } else { 0 },
                    audio: if counter == 5 {
                        Duration::MAX
                    } else {
                        Duration::ZERO
                    },
                },
            }),
            4 => f.gate.cloud_audio = Duration::MAX,
            _ => unreachable!(),
        }
        let before = f.usage();
        assert_eq!(
            error(f.reserve(Some(&grant), domain)),
            EgressError::Contract(ContractError::Overflow)
        );
        assert_eq!(f.usage(), before);
    }
}

#[test]
fn clock_domain_reversal_and_deadline_overflow_are_fail_closed() {
    let mut f = Fixture::new();
    let grant = f.issue(Domain::Conversation);
    let (intent, payload) = f.input(Domain::Conversation);
    let foreign = ManualClock::new();
    assert_eq!(
        error(f.gate.reserve(
            &mut f.registry,
            Some(&grant),
            intent,
            payload,
            &foreign.now()
        )),
        EgressError::Contract(ContractError::ClockDomainMismatch)
    );
    let earlier = f.clock.now();
    f.clock.advance(Duration::from_secs(1)).unwrap();
    let (_, job) = f.submit(Domain::Conversation);
    assert_eq!(
        error(f.gate.begin(&mut f.registry, &job, &earlier)),
        EgressError::Contract(ContractError::TimeReversed)
    );
    let mut clock = ManualClock::new();
    clock.advance(Duration::MAX).unwrap();
    assert_eq!(
        error(EgressGate::new(&f.context, clock.now())),
        EgressError::Contract(ContractError::Overflow)
    );
    assert_eq!(f.gate.roles[1], 1);
}

#[test]
fn concurrent_reservations_are_serialized_and_only_one_can_commit() {
    use std::sync::{Barrier, Mutex};
    let mut f = Fixture::new();
    let grant = f.issue(Domain::Conversation);
    let first = f.input(Domain::Conversation);
    let second = f.input(Domain::Conversation);
    let state = Arc::new(Mutex::new((f.gate, f.registry)));
    let barrier = Arc::new(Barrier::new(3));
    let now = f.clock.now();
    let handles: Vec<_> = [first, second]
        .into_iter()
        .map(|(intent, payload)| {
            let state = state.clone();
            let barrier = barrier.clone();
            let grant = grant.clone();
            let now = now.clone();
            std::thread::spawn(move || {
                barrier.wait();
                let mut state = state.lock().unwrap();
                let (gate, registry) = &mut *state;
                gate.reserve(registry, Some(&grant), intent, payload, &now)
                    .map(|_| ())
            })
        })
        .collect();
    barrier.wait();
    let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
    assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
    assert_eq!(
        results
            .iter()
            .filter(|r| **r == Err(EgressError::Duplicate))
            .count(),
        1
    );
    let state = state.lock().unwrap();
    assert_eq!(state.0.sequence, 1);
    assert_eq!(state.0.roles[1], 1);
    assert_eq!(state.0.grants[0].usage.text, 4);
}

#[test]
fn sealed_jobs_and_payload_evidence_are_neither_cloneable_nor_serde_inputs() {
    // Ambiguous inference makes this fail to compile if a forbidden impl appears.
    trait AmbiguousIfClone<A> {
        fn check() {}
    }
    impl<T: ?Sized> AmbiguousIfClone<()> for T {}
    impl<T: ?Sized + Clone> AmbiguousIfClone<u8> for T {}
    let _ = <SealedJob as AmbiguousIfClone<_>>::check;
    let _ = <synthetic::Payload as AmbiguousIfClone<_>>::check;
    trait AmbiguousIfDeserialize<A> {
        fn check() {}
    }
    impl<T: ?Sized> AmbiguousIfDeserialize<()> for T {}
    impl<T: serde::de::DeserializeOwned> AmbiguousIfDeserialize<u8> for T {}
    let _ = <SealedJob as AmbiguousIfDeserialize<_>>::check;
    let _ = <synthetic::Payload as AmbiguousIfDeserialize<_>>::check;
    trait AmbiguousIfSerialize<A> {
        fn check() {}
    }
    impl<T: ?Sized> AmbiguousIfSerialize<()> for T {}
    impl<T: ?Sized + serde::Serialize> AmbiguousIfSerialize<u8> for T {}
    let _ = <SealedJob as AmbiguousIfSerialize<_>>::check;
}

#[test]
fn full_request_bound_includes_audio_not_just_text_and_metadata() {
    let mut f = Fixture::new();
    let grant = f.issue(Domain::CloudTranscription);
    let (intent, _) = f.input(Domain::CloudTranscription);
    let payload = synthetic::Payload::fixture(
        f.context.identity,
        DataClass::PostCueAudio,
        Provenance::PostCue,
        "",
        MAX_PROVIDER_REQUEST_BYTES as usize,
        Duration::from_secs(1),
    );
    assert_eq!(
        error(f.gate.reserve(
            &mut f.registry,
            Some(&grant),
            intent,
            payload,
            &f.clock.now()
        )),
        EgressError::Contract(ContractError::LimitExceeded)
    );
    assert_eq!(f.gate.cloud_audio, Duration::ZERO);
    assert_eq!(f.gate.sequence, 0);
}

#[test]
fn payload_identity_and_metadata_cannot_be_substituted_after_native_validation() {
    for field in 0..3 {
        let mut f = Fixture::new();
        let grant = f.issue(Domain::Conversation);
        let (intent, _) = f.input(Domain::Conversation);
        let mut identity = f.context.identity;
        if field == 0 {
            identity.turn_id = TurnId::from_native_sequence(2).unwrap();
        }
        if field == 1 {
            identity.generation = Generation::from_native_sequence(2).unwrap();
        }
        let data = if field == 2 {
            DataClass::OrdinaryHistory
        } else {
            DataClass::OrdinaryText
        };
        let payload = synthetic::Payload::fixture(
            identity,
            data,
            Provenance::Ordinary,
            "test",
            0,
            Duration::ZERO,
        );
        assert_eq!(
            error(f.gate.reserve(
                &mut f.registry,
                Some(&grant),
                intent,
                payload,
                &f.clock.now()
            )),
            EgressError::Binding
        );
        assert_eq!(f.gate.sequence, 0);
    }
}

#[test]
fn expired_grant_is_rejected_before_reservation_even_while_interaction_is_live() {
    let mut f = Fixture::new();
    f.configuration.specifications[Domain::Conversation.index()]
        .limits
        .lifetime = Duration::from_secs(1);
    f.turn(1, 1);
    let grant = f.issue(Domain::Conversation);
    f.clock.advance(Duration::from_secs(1)).unwrap();
    assert_eq!(
        error(f.reserve(Some(&grant), Domain::Conversation)),
        EgressError::Consent(Error::Expired)
    );
    assert_eq!(f.gate.sequence, 0);
}

#[test]
fn child_expiry_blocks_previously_sealed_jobs_and_callbacks() {
    for attempted in [false, true] {
        let mut f = Fixture::new();
        let (_, job) = f.submit(Domain::Conversation);
        let ticket = if attempted {
            Some(f.begin(&job).unwrap())
        } else {
            None
        };
        f.clock.advance(MAX_CHILD_CONVERSATION).unwrap();
        let result = if let Some(ticket) = ticket {
            f.finish(&ticket, DispatchOutcome::Succeeded)
        } else {
            f.begin(&job).map(|_| ())
        };
        assert_eq!(error(result), EgressError::Consent(Error::Expired));
        assert_eq!(f.gate.roles[1], 1);
    }
}

#[test]
fn revoked_parent_invalidates_a_separately_confirmed_child_egress_grant() {
    let mut f = Fixture::new();
    let parent = f.issue(Domain::Microphone);
    let preview = f
        .registry
        .prepare(Domain::Conversation, Some(&parent), &f.clock.now())
        .unwrap();
    let receipt = f.presenter.interact(
        &f.presenter.show(&preview),
        presenter::NativeInteraction::Affirm,
        &f.context,
        &f.clock.now(),
    );
    let child = f
        .registry
        .confirm(
            &preview,
            Confirmation::Native(&receipt),
            &f.context,
            &f.clock.now(),
        )
        .unwrap();
    let job = f.reserve(Some(&child), Domain::Conversation).unwrap();
    f.registry.revoke(&parent).unwrap();
    assert_eq!(error(f.begin(&job)), EgressError::Consent(Error::Revoked));
    assert_eq!(f.gate.roles[1], 1);
}

#[test]
fn separately_reviewed_derived_private_snapshot_stays_exact_and_one_shot() {
    let mut f = Fixture::new();
    let grant = f.issue(Domain::PrivateContext);
    let (mut intent, _) = f.input(Domain::PrivateContext);
    intent.provenance = Provenance::DerivedPrivate;
    let payload = synthetic::Payload::fixture(
        f.context.identity,
        DataClass::PrivateSnapshot,
        Provenance::DerivedPrivate,
        PRIVATE_FIXTURE,
        0,
        Duration::ZERO,
    );
    let job = f
        .gate
        .reserve(
            &mut f.registry,
            Some(&grant),
            intent,
            payload,
            &f.clock.now(),
        )
        .unwrap();
    assert_eq!(job.0.payload.text(), PRIVATE_FIXTURE);
    assert!(f.reserve(Some(&grant), Domain::PrivateContext).is_err());
    assert_eq!(f.gate.grants[0].usage.requests, 1);
}

#[test]
fn expired_dispatch_cannot_be_revived_by_replaying_an_older_clock_reading() {
    let mut f = Fixture::new();
    let (_, job) = f.submit(Domain::Conversation);
    let earlier = f.clock.now();
    f.clock.advance(MAX_CHILD_CONVERSATION).unwrap();
    assert_eq!(error(f.begin(&job)), EgressError::Consent(Error::Expired));
    assert_eq!(
        error(f.gate.begin(&mut f.registry, &job, &earlier)),
        EgressError::Contract(ContractError::TimeReversed)
    );
    assert_eq!(f.gate.roles[1], 1);
}
