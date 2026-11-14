//! Synthetic native configuration, presentation and user interactions only.
use super::*;
use crate::voice_conversation::clock::{ManualClock, MonotonicClock};
use crate::voice_conversation::model::*;

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
        Self {
            clock,
            presenter,
            context,
            configuration,
            registry,
        }
    }
    fn prepare(&mut self, domain: Domain, parent: Option<&Grant>) -> Preview {
        self.registry
            .prepare(domain, parent, &self.clock.now())
            .unwrap()
    }
    fn receipt(&self, preview: &Preview) -> presenter::Receipt {
        self.presenter.interact(
            &self.presenter.show(preview),
            presenter::NativeInteraction::Affirm,
            &self.context,
            &self.clock.now(),
        )
    }
    fn issue(&mut self, domain: Domain, parent: Option<&Grant>) -> Grant {
        let preview = self.prepare(domain, parent);
        let receipt = self.receipt(&preview);
        self.registry
            .confirm(
                &preview,
                Confirmation::Native(&receipt),
                &self.context,
                &self.clock.now(),
            )
            .unwrap()
    }
    fn request(&self, domain: Domain) -> UseRequest<'static> {
        let spec = &self.configuration.specifications[domain.index()];
        let (provenance, audio, text_bytes) = match domain {
            Domain::Microphone => (Provenance::PostCue, Duration::from_secs(1), 0),
            Domain::BackgroundWake => (Provenance::Ambient, Duration::from_secs(1), 0),
            Domain::CloudTranscription => (Provenance::PostCue, Duration::from_secs(1), 0),
            Domain::Conversation | Domain::CloudTts => (Provenance::Ordinary, Duration::ZERO, 4),
            Domain::PrivateContext => (
                Provenance::Private,
                Duration::ZERO,
                PRIVATE_FIXTURE.len() as u64,
            ),
        };
        UseRequest {
            context: self.context.clone(),
            domain,
            purpose: spec.purpose,
            target: spec.target,
            data: spec.classes[0],
            provenance,
            background: false,
            project: spec.scope.project,
            reviewed_snapshot: if domain == Domain::PrivateContext {
                Some(PRIVATE_FIXTURE)
            } else {
                None
            },
            requests: u64::from(spec.target.is_some()),
            audio,
            text_bytes,
        }
    }
    fn valid(&mut self, grant: &Grant, domain: Domain) -> Result<()> {
        self.registry
            .validate(grant, &self.request(domain), &self.clock.now())
    }
    fn replace(&mut self, config: TrustedConfiguration, context: Context) {
        self.registry
            .update_configuration(config.clone(), context.clone())
            .unwrap();
        self.configuration = config;
        self.context = context;
    }
}

#[test]
fn six_domains_have_independent_native_preview_confirm_grant_lifecycles() {
    let mut f = Fixture::new();
    for domain in Domain::ALL {
        let preview = f.prepare(domain, None);
        assert_eq!(preview.content().specification.domain, domain);
        assert_eq!(preview.content().context, f.context);
        assert_eq!(preview.content().disclosures, DISCLOSURES);
        assert_eq!(
            preview.content().confirmation_expires_at - preview.content().prepared_at,
            PREVIEW_LIFETIME
        );
        assert_eq!(digest(preview.content()).unwrap(), preview.data.digest);
        let receipt = f.receipt(&preview);
        let grant = f
            .registry
            .confirm(
                &preview,
                Confirmation::Native(&receipt),
                &f.context,
                &f.clock.now(),
            )
            .unwrap();
        assert!(f.registry.pending.is_none());
        f.valid(&grant, domain).unwrap();
        assert_eq!(format!("{grant:?}"), "Grant(<native>)");
    }
    assert_eq!(f.registry.grant_sequence, 6);
}

#[test]
fn every_cross_domain_use_is_rejected_even_with_the_same_provider_ids() {
    let mut f = Fixture::new();
    for issued in Domain::ALL {
        let grant = f.issue(issued, None);
        for attempted in Domain::ALL {
            let result = f.valid(&grant, attempted);
            if issued == attempted {
                result.unwrap();
            } else {
                assert_eq!(result, Err(Error::Binding));
            }
        }
    }
}

#[test]
fn workflow_planning_and_tools_have_no_issuable_domain_or_purpose() {
    assert_eq!(Domain::ALL.len(), 6);
    let f = Fixture::new();
    for domain in Domain::ALL {
        for purpose in [Purpose::Planning, Purpose::Tools, Purpose::Workflow] {
            let mut config = configuration();
            config.specifications[domain.index()].purpose = purpose;
            assert_eq!(
                ConsentRegistry::new(config, context(), &f.presenter, f.clock.now()).err(),
                Some(Error::InvalidPolicy)
            );
        }
    }
    let mut f = Fixture::new();
    let grant = f.issue(Domain::Conversation, None);
    for purpose in [Purpose::Planning, Purpose::Tools, Purpose::Workflow] {
        let mut request = f.request(Domain::Conversation);
        request.purpose = purpose;
        assert_eq!(
            f.registry.validate(&grant, &request, &f.clock.now()),
            Err(Error::Binding)
        );
    }
}

#[test]
fn preview_expiration_is_exact_at_five_minutes_and_consumes_the_attempt() {
    for elapsed in [
        PREVIEW_LIFETIME - Duration::from_nanos(1),
        PREVIEW_LIFETIME,
        PREVIEW_LIFETIME + Duration::from_nanos(1),
    ] {
        let mut f = Fixture::new();
        let preview = f.prepare(Domain::Conversation, None);
        f.clock.advance(elapsed).unwrap();
        let receipt = f.receipt(&preview);
        let result = f.registry.confirm(
            &preview,
            Confirmation::Native(&receipt),
            &f.context,
            &f.clock.now(),
        );
        if elapsed < PREVIEW_LIFETIME {
            assert!(result.is_ok());
        } else {
            assert_eq!(result.err(), Some(Error::Expired));
        }
        assert!(f.registry.pending.is_none());
        assert_eq!(
            f.registry
                .confirm(
                    &preview,
                    Confirmation::Native(&receipt),
                    &f.context,
                    &f.clock.now()
                )
                .err(),
            Some(Error::Stale)
        );
    }
}

#[test]
fn renderer_flags_copied_digests_transcripts_and_ai_output_never_confirm() {
    let mut f = Fixture::new();
    // These data-only forms are deliberately mapped to the sole untrusted case;
    // none can construct the private native Receipt required by Confirmation.
    for input in [
        serde_json::json!({"confirmed": true}),
        serde_json::json!({"digest": [0, 1, 2]}),
        serde_json::json!({"transcript": "yes"}),
        serde_json::json!({"ai_output": "approved"}),
    ] {
        assert!(input.is_object());
        let preview = f.prepare(Domain::Conversation, None);
        assert_eq!(
            f.registry
                .confirm(
                    &preview,
                    Confirmation::Untrusted,
                    &f.context,
                    &f.clock.now()
                )
                .err(),
            Some(Error::UntrustedProof)
        );
        assert!(f.registry.pending.is_none());
        let native = f.receipt(&preview);
        assert_eq!(
            f.registry
                .confirm(
                    &preview,
                    Confirmation::Native(&native),
                    &f.context,
                    &f.clock.now()
                )
                .err(),
            Some(Error::Stale)
        );
    }
    assert_eq!(f.registry.grant_sequence, 0);
}

#[test]
fn declined_native_confirmation_consumes_preview_without_grant() {
    let mut f = Fixture::new();
    let preview = f.prepare(Domain::Microphone, None);
    let receipt = f.presenter.interact(
        &f.presenter.show(&preview),
        presenter::NativeInteraction::Decline,
        &f.context,
        &f.clock.now(),
    );
    assert_eq!(
        f.registry
            .confirm(
                &preview,
                Confirmation::Native(&receipt),
                &f.context,
                &f.clock.now()
            )
            .err(),
        Some(Error::Denied)
    );
    assert!(f.registry.pending.is_none());
    assert_eq!(f.registry.grant_sequence, 0);
}

#[test]
fn foreign_presenter_cannot_sign_even_a_genuine_native_presentation() {
    for show_foreign in [false, true] {
        let mut f = Fixture::new();
        let preview = f.prepare(Domain::Conversation, None);
        let foreign = presenter::TestPresenter::new();
        let shown = if show_foreign {
            foreign.show(&preview)
        } else {
            f.presenter.show(&preview)
        };
        let receipt = if show_foreign {
            f.presenter.interact(
                &shown,
                presenter::NativeInteraction::Affirm,
                &f.context,
                &f.clock.now(),
            )
        } else {
            foreign.interact(
                &shown,
                presenter::NativeInteraction::Affirm,
                &f.context,
                &f.clock.now(),
            )
        };
        assert_eq!(
            f.registry
                .confirm(
                    &preview,
                    Confirmation::Native(&receipt),
                    &f.context,
                    &f.clock.now()
                )
                .err(),
            Some(Error::Binding)
        );
        assert!(f.registry.pending.is_none());
        assert_eq!(f.registry.grant_sequence, 0);
    }
}

fn foreign_contexts() -> Vec<Context> {
    let original = context();
    let mut variants = Vec::new();
    macro_rules! change {
        ($field:ident, $value:expr) => {{
            let mut c = original.clone();
            c.identity.$field = $value;
            variants.push(c);
        }};
    }
    change!(app_run_id, AppRunId::from_native_sequence(2).unwrap());
    change!(
        interaction_id,
        InteractionId::from_native_sequence(2).unwrap()
    );
    change!(session_id, SessionId::from_native_sequence(2).unwrap());
    change!(turn_id, TurnId::from_native_sequence(2).unwrap());
    change!(job_id, JobId::from_native_sequence(2).unwrap());
    change!(generation, Generation::from_native_sequence(2).unwrap());
    change!(
        configuration_revision,
        ConfigurationRevision::from_native_sequence(2).unwrap()
    );
    change!(
        consent_binding_id,
        ConsentBindingId::from_native_sequence(2).unwrap()
    );
    let mut c = original.clone();
    c.os_session = OsSessionId::new(2).unwrap();
    variants.push(c);
    let mut c = original;
    c.policy_revision = PolicyRevision::new(2).unwrap();
    variants.push(c);
    variants
}

#[test]
fn all_wrong_native_context_bindings_reject_confirmation_and_grant_use() {
    for wrong in foreign_contexts() {
        for wrong_receipt in [false, true] {
            let mut f = Fixture::new();
            let preview = f.prepare(Domain::Conversation, None);
            let receipt_context = if wrong_receipt { &wrong } else { &f.context };
            let receipt = f.presenter.interact(
                &f.presenter.show(&preview),
                presenter::NativeInteraction::Affirm,
                receipt_context,
                &f.clock.now(),
            );
            let current = if wrong_receipt { &f.context } else { &wrong };
            assert_eq!(
                f.registry
                    .confirm(
                        &preview,
                        Confirmation::Native(&receipt),
                        current,
                        &f.clock.now()
                    )
                    .err(),
                Some(Error::Binding)
            );
            assert!(f.registry.pending.is_none());
            assert_eq!(f.registry.grant_sequence, 0);
        }
        let mut f = Fixture::new();
        let grant = f.issue(Domain::Conversation, None);
        let mut request = f.request(Domain::Conversation);
        request.context = wrong;
        assert_eq!(
            f.registry.validate(&grant, &request, &f.clock.now()),
            Err(Error::Binding)
        );
    }
}

#[test]
fn stale_and_duplicate_receipts_cannot_issue_another_grant() {
    let mut f = Fixture::new();
    let first = f.prepare(Domain::Conversation, None);
    let receipt = f.receipt(&first);
    f.registry
        .confirm(
            &first,
            Confirmation::Native(&receipt),
            &f.context,
            &f.clock.now(),
        )
        .unwrap();
    assert_eq!(
        f.registry
            .confirm(
                &first,
                Confirmation::Native(&receipt),
                &f.context,
                &f.clock.now()
            )
            .err(),
        Some(Error::Stale)
    );
    let second = f.prepare(Domain::CloudTts, None);
    assert_eq!(
        f.registry
            .confirm(
                &second,
                Confirmation::Native(&receipt),
                &f.context,
                &f.clock.now()
            )
            .err(),
        Some(Error::Binding)
    );
    assert!(f.registry.pending.is_none());
    assert_eq!(f.registry.grant_sequence, 1);
}

#[test]
fn preview_digest_binds_all_content_and_native_handles_cannot_be_reconstructed() {
    let mut f = Fixture::new();
    let preview = f.prepare(Domain::PrivateContext, None);
    let baseline = preview.content().clone();
    let expected = preview.data.digest;
    for alteration in 0..15 {
        let mut content = baseline.clone();
        match alteration {
            0 => content.context.identity.app_run_id = AppRunId::from_native_sequence(2).unwrap(),
            1 => content.context.os_session = OsSessionId::new(2).unwrap(),
            2 => {
                content.context.identity.interaction_id =
                    InteractionId::from_native_sequence(2).unwrap()
            }
            3 => {
                content.context.identity.configuration_revision =
                    ConfigurationRevision::from_native_sequence(2).unwrap()
            }
            4 => content.context.policy_revision = PolicyRevision::new(2).unwrap(),
            5 => {
                content.specification.target.as_mut().unwrap().provider =
                    ProviderId::new(2).unwrap()
            }
            6 => content.specification.target.as_mut().unwrap().model = ModelId::new(2).unwrap(),
            7 => content.specification.purpose = Purpose::Planning,
            8 => content.specification.classes.push(DataClass::GoogleDerived),
            9 => content.specification.scope.background = !content.specification.scope.background,
            10 => content.specification.limits.requests += 1,
            11 => content.grant_expires_at += Duration::from_nanos(1),
            12 => content.confirmation_expires_at += Duration::from_nanos(1),
            13 => content
                .specification
                .scope
                .snapshot
                .as_mut()
                .unwrap()
                .text
                .push('x'),
            _ => content.generation = content.generation.checked_next().unwrap(),
        }
        assert_ne!(digest(&content).unwrap(), expected);
    }
    let original = &preview.data;
    let copy = Preview {
        data: Arc::new(PreviewData {
            content: baseline,
            digest: expected,
            native_instance: original.native_instance.clone(),
            confirmation_deadline: original.confirmation_deadline.clone(),
            grant_deadline: original.grant_deadline.clone(),
            prepared_at: original.prepared_at.clone(),
            global: original.global.clone(),
            domain: original.domain.clone(),
            parent: original.parent.clone(),
        }),
    };
    let receipt = f.receipt(&copy);
    assert_eq!(
        f.registry
            .confirm(
                &copy,
                Confirmation::Native(&receipt),
                &f.context,
                &f.clock.now()
            )
            .err(),
        Some(Error::Binding)
    );
    assert!(f.registry.pending.is_none());
    assert_eq!(f.registry.grant_sequence, 0);
}

#[test]
fn configuration_owned_by_registry_and_previews_is_immutable() {
    let mut f = Fixture::new();
    let preview = f.prepare(Domain::Conversation, None);
    f.configuration.specifications[Domain::Conversation.index()]
        .target
        .as_mut()
        .unwrap()
        .provider = ProviderId::new(99).unwrap();
    assert_eq!(
        preview.content().specification.target.unwrap().provider,
        ProviderId::new(1).unwrap()
    );
    assert_eq!(
        f.registry.configuration.specifications[Domain::Conversation.index()]
            .target
            .unwrap()
            .provider,
        ProviderId::new(1).unwrap()
    );
    assert_eq!(digest(preview.content()).unwrap(), preview.data.digest);
}

#[test]
fn configuration_and_policy_changes_revoke_grants_and_pending_receipts() {
    for policy_only in [false, true] {
        let mut f = Fixture::new();
        let old_grant = f.issue(Domain::Conversation, None);
        let preview = f.prepare(Domain::CloudTts, None);
        let receipt = f.receipt(&preview);
        let mut config = f.configuration.clone();
        let mut current = f.context.clone();
        if policy_only {
            config.policy_revision = PolicyRevision::new(2).unwrap();
            current.policy_revision = config.policy_revision;
        } else {
            config.revision_sequence = NonZeroU64::new(2).unwrap();
            current.identity.configuration_revision =
                ConfigurationRevision::from_native_sequence(2).unwrap();
        }
        f.replace(config, current);
        assert_eq!(
            f.valid(&old_grant, Domain::Conversation),
            Err(Error::Revoked)
        );
        assert_eq!(
            f.registry
                .confirm(
                    &preview,
                    Confirmation::Native(&receipt),
                    &f.context,
                    &f.clock.now()
                )
                .err(),
            Some(Error::Stale)
        );
        let new_grant = f.issue(Domain::Conversation, None);
        f.valid(&new_grant, Domain::Conversation).unwrap();
    }
}

#[test]
fn provider_model_voice_and_purpose_are_bound_exactly() {
    let mut f = Fixture::new();
    for domain in [
        Domain::CloudTranscription,
        Domain::Conversation,
        Domain::PrivateContext,
        Domain::CloudTts,
    ] {
        let grant = f.issue(domain, None);
        for wrong in 0..4 {
            let mut request = f.request(domain);
            match wrong {
                0 => request.target.as_mut().unwrap().provider = ProviderId::new(99).unwrap(),
                1 => request.target.as_mut().unwrap().model = ModelId::new(99).unwrap(),
                2 => request.target.as_mut().unwrap().voice = Some(VoiceId::new(99).unwrap()),
                _ => request.purpose = Purpose::Workflow,
            }
            assert_eq!(
                f.registry.validate(&grant, &request, &f.clock.now()),
                Err(Error::Binding)
            );
        }
        f.valid(&grant, domain).unwrap();
    }
}

#[test]
fn ordinary_grants_never_authorize_private_google_planning_tools_or_actions() {
    let mut f = Fixture::new();
    let grant = f.issue(Domain::Conversation, None);
    for provenance in [
        Provenance::Private,
        Provenance::DerivedPrivate,
        Provenance::GoogleDerived,
        Provenance::Planning,
        Provenance::Tools,
        Provenance::Workflow,
        Provenance::Ambient,
    ] {
        let mut request = f.request(Domain::Conversation);
        request.provenance = provenance;
        assert_eq!(
            f.registry.validate(&grant, &request, &f.clock.now()),
            Err(Error::Denied)
        );
    }
    for data in [
        DataClass::PrivateSnapshot,
        DataClass::GoogleDerived,
        DataClass::Planning,
        DataClass::Tools,
        DataClass::Workflow,
        DataClass::AmbientAudio,
    ] {
        let mut request = f.request(Domain::Conversation);
        request.data = data;
        assert_eq!(
            f.registry.validate(&grant, &request, &f.clock.now()),
            Err(Error::Binding)
        );
    }
}

#[test]
fn google_derived_and_laundered_private_provenance_are_not_cloud_tts_or_stt() {
    let mut f = Fixture::new();
    for domain in Domain::ALL {
        let grant = f.issue(domain, None);
        let mut request = f.request(domain);
        request.provenance = Provenance::GoogleDerived;
        assert_eq!(
            f.registry.validate(&grant, &request, &f.clock.now()),
            Err(Error::Denied)
        );
    }
    for domain in [Domain::CloudTranscription, Domain::CloudTts] {
        let grant = f.issue(domain, None);
        for provenance in [
            Provenance::Ambient,
            Provenance::Private,
            Provenance::DerivedPrivate,
        ] {
            let mut request = f.request(domain);
            request.provenance = provenance;
            assert_eq!(
                f.registry.validate(&grant, &request, &f.clock.now()),
                Err(Error::Denied)
            );
        }
    }
}

#[test]
fn private_context_requires_exact_reviewed_text_project_and_purpose() {
    let mut f = Fixture::new();
    let grant = f.issue(Domain::PrivateContext, None);
    for wrong in 0..5 {
        let mut request = f.request(Domain::PrivateContext);
        match wrong {
            0 => request.reviewed_snapshot = Some("different private fixture"),
            1 => request.reviewed_snapshot = None,
            2 => request.project = Some(ProjectId::new(2).unwrap()),
            3 => request.purpose = Purpose::OrdinaryConversation,
            _ => request.text_bytes = 0,
        }
        assert_eq!(
            f.registry.validate(&grant, &request, &f.clock.now()),
            Err(Error::Binding)
        );
    }
    let mut request = f.request(Domain::PrivateContext);
    request.provenance = Provenance::DerivedPrivate;
    f.registry
        .validate(&grant, &request, &f.clock.now())
        .unwrap();
    assert_eq!(grant.data.preview.content.specification.limits.requests, 1);
}

#[test]
fn invalid_native_policies_fail_closed_without_ordinary_or_google_escalation() {
    let f = Fixture::new();
    for wrong in 0..8 {
        let mut config = configuration();
        let spec = &mut config.specifications[Domain::Conversation.index()];
        match wrong {
            0 => spec.classes.push(DataClass::PrivateSnapshot),
            1 => spec.classes.push(DataClass::GoogleDerived),
            2 => spec.classes.push(DataClass::OrdinaryText),
            3 => spec.target = None,
            4 => {
                spec.scope.snapshot = Some(ReviewedSnapshot {
                    text: PRIVATE_FIXTURE.into(),
                    digest: hash(PRIVATE_FIXTURE.as_bytes()),
                })
            }
            5 => spec.limits.lifetime = MAX_CLOUD_INTERACTION + Duration::from_nanos(1),
            6 => spec.limits.requests = u64::MAX,
            _ => spec.limits.text_bytes = MAX_CONVERSATION_INPUT_BYTES + 1,
        }
        assert_eq!(
            ConsentRegistry::new(config, context(), &f.presenter, f.clock.now()).err(),
            Some(Error::InvalidPolicy)
        );
    }
    let mut config = configuration();
    config.specifications[Domain::PrivateContext.index()]
        .scope
        .snapshot
        .as_mut()
        .unwrap()
        .digest = [0; 32];
    assert_eq!(
        ConsentRegistry::new(config, context(), &f.presenter, f.clock.now()).err(),
        Some(Error::InvalidPolicy)
    );
}

#[test]
fn resource_and_background_scope_limits_are_checked_at_exact_boundaries() {
    let mut f = Fixture::new();
    for domain in [
        Domain::Microphone,
        Domain::BackgroundWake,
        Domain::CloudTranscription,
        Domain::Conversation,
        Domain::CloudTts,
    ] {
        let grant = f.issue(domain, None);
        let spec = grant.data.preview.content.specification.clone();
        let mut request = f.request(domain);
        request.requests = spec.limits.requests;
        request.audio = spec.limits.audio;
        request.text_bytes = spec.limits.text_bytes;
        f.registry
            .validate(&grant, &request, &f.clock.now())
            .unwrap();
        request.requests = spec.limits.requests + 1;
        assert_eq!(
            f.registry.validate(&grant, &request, &f.clock.now()),
            Err(ContractError::LimitExceeded.into())
        );
        request.requests = spec.limits.requests;
        request.audio = spec.limits.audio + Duration::from_nanos(1);
        assert_eq!(
            f.registry.validate(&grant, &request, &f.clock.now()),
            Err(ContractError::LimitExceeded.into())
        );
        request.audio = spec.limits.audio;
        request.text_bytes = spec.limits.text_bytes + 1;
        assert_eq!(
            f.registry.validate(&grant, &request, &f.clock.now()),
            Err(ContractError::LimitExceeded.into())
        );
    }
    let mut config = configuration();
    config.specifications[Domain::Conversation.index()]
        .scope
        .background = false;
    let mut f = Fixture::new();
    let mut c = f.context.clone();
    config.revision_sequence = NonZeroU64::new(2).unwrap();
    c.identity.configuration_revision = ConfigurationRevision::from_native_sequence(2).unwrap();
    f.replace(config, c);
    let grant = f.issue(Domain::Conversation, None);
    let mut request = f.request(Domain::Conversation);
    request.background = true;
    assert_eq!(
        f.registry.validate(&grant, &request, &f.clock.now()),
        Err(Error::Binding)
    );
}

#[test]
fn descendants_require_separate_confirmation_and_inherit_revocation() {
    let mut f = Fixture::new();
    let root = f.issue(Domain::Microphone, None);
    let pending = f.prepare(Domain::Conversation, Some(&root));
    assert_eq!(
        f.registry
            .confirm(
                &pending,
                Confirmation::Untrusted,
                &f.context,
                &f.clock.now()
            )
            .err(),
        Some(Error::UntrustedProof)
    );
    let child = f.issue(Domain::Conversation, Some(&root));
    let grandchild = f.issue(Domain::PrivateContext, Some(&child));
    let independent = f.issue(Domain::CloudTts, None);
    f.valid(&child, Domain::Conversation).unwrap();
    f.valid(&grandchild, Domain::PrivateContext).unwrap();
    f.registry.revoke(&root).unwrap();
    assert_eq!(f.valid(&root, Domain::Microphone), Err(Error::Revoked));
    assert_eq!(f.valid(&child, Domain::Conversation), Err(Error::Revoked));
    assert_eq!(
        f.valid(&grandchild, Domain::PrivateContext),
        Err(Error::Revoked)
    );
    f.valid(&independent, Domain::CloudTts).unwrap();
    f.registry.revoke(&root).unwrap();
}

#[test]
fn domain_revocation_invalidates_descendants_and_stale_receipts_only() {
    let mut f = Fixture::new();
    let root = f.issue(Domain::Microphone, None);
    let child = f.issue(Domain::Conversation, Some(&root));
    let independent = f.issue(Domain::CloudTts, None);
    let preview = f.prepare(Domain::Microphone, None);
    let receipt = f.receipt(&preview);
    f.registry.revoke_domain(Domain::Microphone);
    assert_eq!(f.valid(&child, Domain::Conversation), Err(Error::Revoked));
    f.valid(&independent, Domain::CloudTts).unwrap();
    assert_eq!(
        f.registry
            .confirm(
                &preview,
                Confirmation::Native(&receipt),
                &f.context,
                &f.clock.now()
            )
            .err(),
        Some(Error::Stale)
    );
    let fresh = f.issue(Domain::Microphone, None);
    f.valid(&fresh, Domain::Microphone).unwrap();
    assert_eq!(f.valid(&root, Domain::Microphone), Err(Error::Revoked));
}

#[test]
fn revoking_parent_before_child_confirmation_consumes_and_rejects_child() {
    let mut f = Fixture::new();
    let root = f.issue(Domain::Microphone, None);
    let preview = f.prepare(Domain::Conversation, Some(&root));
    let receipt = f.receipt(&preview);
    f.registry.revoke(&root).unwrap();
    assert_eq!(
        f.registry
            .confirm(
                &preview,
                Confirmation::Native(&receipt),
                &f.context,
                &f.clock.now()
            )
            .err(),
        Some(Error::Revoked)
    );
    assert!(f.registry.pending.is_none());
    assert_eq!(
        f.registry
            .prepare(Domain::Conversation, Some(&root), &f.clock.now())
            .err(),
        Some(Error::Revoked)
    );
}

#[test]
fn opaque_grant_clones_and_other_registries_do_not_recreate_authority() {
    let mut f = Fixture::new();
    let grant = f.issue(Domain::Conversation, None);
    let clone = grant.clone();
    let mut other = Fixture::new();
    assert_eq!(
        other.valid(&grant, Domain::Conversation),
        Err(Error::Revoked)
    );
    assert_eq!(other.registry.revoke(&grant), Err(Error::Binding));
    assert_eq!(
        other
            .registry
            .prepare(Domain::PrivateContext, Some(&grant), &other.clock.now())
            .err(),
        Some(Error::Revoked)
    );
    f.registry.revoke(&grant).unwrap();
    assert_eq!(f.valid(&clone, Domain::Conversation), Err(Error::Revoked));
}

#[test]
fn child_deadlines_are_clamped_to_parent_and_never_renew_on_confirmation() {
    let mut f = Fixture::new();
    let mut cfg = configuration();
    cfg.revision_sequence = NonZeroU64::new(2).unwrap();
    cfg.specifications[Domain::Microphone.index()]
        .limits
        .lifetime = Duration::from_secs(10);
    let mut c = f.context.clone();
    c.identity.configuration_revision = ConfigurationRevision::from_native_sequence(2).unwrap();
    f.replace(cfg, c);
    let root = f.issue(Domain::Microphone, None);
    f.clock.advance(Duration::from_secs(8)).unwrap();
    let child = f.issue(Domain::Conversation, Some(&root));
    assert_eq!(
        child
            .data
            .preview
            .grant_deadline
            .remaining(&f.clock.now())
            .unwrap(),
        Duration::from_secs(2)
    );
    f.clock.advance(Duration::from_secs(2)).unwrap();
    assert_eq!(f.valid(&child, Domain::Conversation), Err(Error::Expired));
}

#[test]
fn session_restart_and_configuration_revision_do_not_reset_original_interaction() {
    let mut f = Fixture::new();
    let old = f.issue(Domain::Conversation, None);
    f.clock
        .advance(MAX_CLOUD_INTERACTION - Duration::from_secs(1))
        .unwrap();
    let mut cfg = f.configuration.clone();
    cfg.revision_sequence = NonZeroU64::new(2).unwrap();
    let mut c = f.context.clone();
    c.identity.configuration_revision = ConfigurationRevision::from_native_sequence(2).unwrap();
    c.identity.session_id = SessionId::from_native_sequence(2).unwrap();
    c.identity.generation = Generation::from_native_sequence(2).unwrap();
    f.replace(cfg, c);
    assert_eq!(f.valid(&old, Domain::Conversation), Err(Error::Revoked));
    let fresh = f.issue(Domain::Conversation, None);
    assert_eq!(
        fresh
            .data
            .preview
            .grant_deadline
            .remaining(&f.clock.now())
            .unwrap(),
        Duration::from_secs(1)
    );
    f.clock.advance(Duration::from_secs(1)).unwrap();
    assert_eq!(f.valid(&fresh, Domain::Conversation), Err(Error::Expired));
    assert_eq!(
        f.registry
            .prepare(Domain::Conversation, None, &f.clock.now())
            .err(),
        Some(Error::Expired)
    );
}

#[test]
fn clock_errors_and_future_or_foreign_receipts_consume_confirmation_attempts() {
    for wrong in 0..3 {
        let mut f = Fixture::new();
        let preview = f.prepare(Domain::Conversation, None);
        let shown = f.presenter.show(&preview);
        let receipt_time = match wrong {
            0 => f.clock.now().checked_add(Duration::from_secs(1)).unwrap(),
            1 => ManualClock::new().now(),
            _ => f.clock.now(),
        };
        let receipt = f.presenter.interact(
            &shown,
            presenter::NativeInteraction::Affirm,
            &f.context,
            &receipt_time,
        );
        let confirm_time = if wrong == 2 {
            ManualClock::new().now()
        } else {
            f.clock.now()
        };
        let expected = if wrong == 0 {
            ContractError::TimeReversed
        } else {
            ContractError::ClockDomainMismatch
        };
        assert_eq!(
            f.registry
                .confirm(
                    &preview,
                    Confirmation::Native(&receipt),
                    &f.context,
                    &confirm_time
                )
                .err(),
            Some(expected.into())
        );
        assert!(f.registry.pending.is_none());
        assert_eq!(f.registry.grant_sequence, 0);
    }
    let mut f = Fixture::new();
    let grant = f.issue(Domain::Conversation, None);
    let earlier = f.clock.now();
    f.clock.advance(Duration::from_secs(1)).unwrap();
    f.valid(&grant, Domain::Conversation).unwrap();
    assert_eq!(
        f.registry
            .validate(&grant, &f.request(Domain::Conversation), &earlier),
        Err(ContractError::TimeReversed.into())
    );
    assert_eq!(
        f.registry.validate(
            &grant,
            &f.request(Domain::Conversation),
            &ManualClock::new().now()
        ),
        Err(ContractError::ClockDomainMismatch.into())
    );
    f.registry.revoke_all().unwrap();
    assert_eq!(f.valid(&grant, Domain::Conversation), Err(Error::Revoked));
}

#[test]
fn deadline_overflow_and_expired_grants_cannot_issue_authority() {
    let mut f = Fixture::new();
    f.clock.advance(Duration::MAX).unwrap();
    assert_eq!(
        ConsentRegistry::new(configuration(), context(), &f.presenter, f.clock.now()).err(),
        Some(ContractError::Overflow.into())
    );
    let mut f = Fixture::new();
    f.clock.advance(Duration::MAX - MAX_WAKE_ARMING).unwrap();
    f.registry =
        ConsentRegistry::new(configuration(), context(), &f.presenter, f.clock.now()).unwrap();
    f.clock
        .advance(MAX_WAKE_ARMING - Duration::from_secs(1))
        .unwrap();
    assert_eq!(
        f.registry
            .prepare(Domain::BackgroundWake, None, &f.clock.now())
            .err(),
        Some(ContractError::Overflow.into())
    );
    assert!(f.registry.pending.is_none());
    assert_eq!(f.registry.preview_sequence, 0);
    let mut f = Fixture::new();
    let mut cfg = configuration();
    cfg.revision_sequence = NonZeroU64::new(2).unwrap();
    cfg.specifications[Domain::Conversation.index()]
        .limits
        .lifetime = Duration::from_secs(1);
    let mut c = f.context.clone();
    c.identity.configuration_revision = ConfigurationRevision::from_native_sequence(2).unwrap();
    f.replace(cfg, c);
    let preview = f.prepare(Domain::Conversation, None);
    let receipt = f.receipt(&preview);
    f.clock.advance(Duration::from_secs(1)).unwrap();
    assert_eq!(
        f.registry
            .confirm(
                &preview,
                Confirmation::Native(&receipt),
                &f.context,
                &f.clock.now()
            )
            .err(),
        Some(Error::Expired)
    );
    assert!(f.registry.pending.is_none());
}

#[test]
fn preview_and_grant_counter_overflow_fail_without_wrap_or_replay() {
    let mut f = Fixture::new();
    f.registry.preview_sequence = u64::MAX;
    assert_eq!(
        f.registry
            .prepare(Domain::Conversation, None, &f.clock.now())
            .err(),
        Some(ContractError::Overflow.into())
    );
    assert!(f.registry.pending.is_none());
    assert_eq!(f.registry.grant_sequence, 0);
    let mut f = Fixture::new();
    let preview = f.prepare(Domain::Conversation, None);
    let receipt = f.receipt(&preview);
    f.registry.grant_sequence = u64::MAX;
    assert_eq!(
        f.registry
            .confirm(
                &preview,
                Confirmation::Native(&receipt),
                &f.context,
                &f.clock.now()
            )
            .err(),
        Some(ContractError::Overflow.into())
    );
    assert!(f.registry.pending.is_none());
    assert_eq!(f.registry.grant_sequence, u64::MAX);
}

#[test]
fn generation_overflow_cannot_prevent_global_revocation_or_restore_authority() {
    let mut f = Fixture::new();
    let grant = f.issue(Domain::Conversation, None);
    f.registry.generation = Generation::from_native_sequence(u64::MAX).unwrap();
    assert_eq!(f.registry.revoke_all(), Err(ContractError::Overflow.into()));
    assert!(!grant.data.preview.global.load(Ordering::Acquire));
    assert!(f.registry.closed);
    assert_eq!(f.valid(&grant, Domain::Conversation), Err(Error::Revoked));
    assert_eq!(
        f.registry
            .prepare(Domain::Conversation, None, &f.clock.now())
            .err(),
        Some(Error::Closed)
    );
    f.registry.revoke_all().unwrap();
    assert!(!f.registry.global.load(Ordering::Acquire));
}

#[test]
fn invalid_configuration_updates_revoke_first_and_never_fall_back() {
    for invalid in 0..4 {
        let mut f = Fixture::new();
        let grant = f.issue(Domain::Conversation, None);
        let mut cfg = f.configuration.clone();
        let mut c = f.context.clone();
        if invalid != 0 {
            cfg.revision_sequence = NonZeroU64::new(2).unwrap();
            c.identity.configuration_revision =
                ConfigurationRevision::from_native_sequence(2).unwrap();
        }
        match invalid {
            0 => (),
            1 => c.os_session = OsSessionId::new(2).unwrap(),
            2 => c.identity.app_run_id = AppRunId::from_native_sequence(2).unwrap(),
            _ => cfg.specifications[Domain::Conversation.index()].purpose = Purpose::Workflow,
        }
        assert_eq!(f.registry.update_configuration(cfg, c), Err(Error::Binding));
        assert_eq!(f.valid(&grant, Domain::Conversation), Err(Error::Revoked));
        assert!(f.registry.closed);
    }
}

#[test]
fn manual_m45_scope_stays_foreground_and_never_implies_transcription_send() {
    let mut f = Fixture::new();
    let mut cfg = configuration();
    cfg.revision_sequence = NonZeroU64::new(2).unwrap();
    let mic = &mut cfg.specifications[Domain::Microphone.index()];
    mic.purpose = Purpose::ManualRecording;
    mic.scope.background = false;
    let mut c = f.context.clone();
    c.identity.configuration_revision = ConfigurationRevision::from_native_sequence(2).unwrap();
    f.replace(cfg, c);
    let grant = f.issue(Domain::Microphone, None);
    let mut request = f.request(Domain::Microphone);
    f.registry
        .validate(&grant, &request, &f.clock.now())
        .unwrap();
    request.background = true;
    assert_eq!(
        f.registry.validate(&grant, &request, &f.clock.now()),
        Err(Error::Binding)
    );
    assert_eq!(
        f.valid(&grant, Domain::CloudTranscription),
        Err(Error::Binding)
    );
    assert_eq!(f.valid(&grant, Domain::PrivateContext), Err(Error::Binding));
}

#[test]
fn unused_preview_is_not_replaced_and_no_confirmation_extends_root_deadlines() {
    let mut f = Fixture::new();
    let preview = f.prepare(Domain::BackgroundWake, None);
    assert_eq!(
        f.registry
            .prepare(Domain::Conversation, None, &f.clock.now())
            .err(),
        Some(Error::Busy)
    );
    assert!(Arc::ptr_eq(
        f.registry.pending.as_ref().unwrap(),
        &preview.data
    ));
    f.clock
        .advance(PREVIEW_LIFETIME - Duration::from_secs(1))
        .unwrap();
    let receipt = f.receipt(&preview);
    let grant = f
        .registry
        .confirm(
            &preview,
            Confirmation::Native(&receipt),
            &f.context,
            &f.clock.now(),
        )
        .unwrap();
    assert_eq!(
        grant
            .data
            .preview
            .grant_deadline
            .remaining(&f.clock.now())
            .unwrap(),
        MAX_WAKE_ARMING - PREVIEW_LIFETIME + Duration::from_secs(1)
    );
    f.clock
        .advance(MAX_WAKE_ARMING - PREVIEW_LIFETIME + Duration::from_secs(1))
        .unwrap();
    assert_eq!(f.valid(&grant, Domain::BackgroundWake), Err(Error::Expired));
    assert_eq!(
        f.registry
            .prepare(Domain::BackgroundWake, None, &f.clock.now())
            .err(),
        Some(Error::Expired)
    );
}

#[test]
fn expired_unused_preview_is_retired_on_the_next_native_prepare() {
    let mut f = Fixture::new();
    let expired = f.prepare(Domain::Conversation, None);
    let old_receipt = f.receipt(&expired);
    f.clock.advance(PREVIEW_LIFETIME).unwrap();
    let fresh = f.prepare(Domain::CloudTts, None);
    assert_ne!(expired.content().id, fresh.content().id);
    assert_eq!(
        f.registry
            .confirm(
                &expired,
                Confirmation::Native(&old_receipt),
                &f.context,
                &f.clock.now()
            )
            .err(),
        Some(Error::Stale)
    );
    assert!(Arc::ptr_eq(
        f.registry.pending.as_ref().unwrap(),
        &fresh.data
    ));
    let native = f.receipt(&fresh);
    f.registry
        .confirm(
            &fresh,
            Confirmation::Native(&native),
            &f.context,
            &f.clock.now(),
        )
        .unwrap();
}

#[test]
fn old_and_foreign_preview_attempts_do_not_consume_an_unrelated_new_preview() {
    let mut f = Fixture::new();
    let old = f.prepare(Domain::Conversation, None);
    assert_eq!(
        f.registry
            .confirm(&old, Confirmation::Untrusted, &f.context, &f.clock.now())
            .err(),
        Some(Error::UntrustedProof)
    );
    let fresh = f.prepare(Domain::CloudTts, None);
    let mut other = Fixture::new();
    let foreign = other.prepare(Domain::CloudTts, None);
    for stale in [&old, &foreign] {
        assert_eq!(
            f.registry
                .confirm(stale, Confirmation::Untrusted, &f.context, &f.clock.now())
                .err(),
            Some(Error::Stale)
        );
        assert!(Arc::ptr_eq(
            f.registry.pending.as_ref().unwrap(),
            &fresh.data
        ));
    }
    let native = f.receipt(&fresh);
    f.registry
        .confirm(
            &fresh,
            Confirmation::Native(&native),
            &f.context,
            &f.clock.now(),
        )
        .unwrap();
}
