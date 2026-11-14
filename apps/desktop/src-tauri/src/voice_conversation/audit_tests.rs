use super::*;
use crate::voice_conversation::clock::{ManualClock, MonotonicClock};
use crate::voice_conversation::model::*;
use serde_json::{json, Value};
use std::time::Duration;

fn identity() -> ContractIdentity {
    ContractIdentity {
        version: ContractVersion::V1,
        app_run_id: AppRunId::from_native_sequence(1).unwrap(),
        interaction_id: InteractionId::from_native_sequence(1).unwrap(),
        session_id: SessionId::from_native_sequence(1).unwrap(),
        turn_id: TurnId::from_native_sequence(1).unwrap(),
        job_id: JobId::from_native_sequence(1).unwrap(),
        generation: Generation::from_native_sequence(1).unwrap(),
        configuration_revision: ConfigurationRevision::from_native_sequence(2).unwrap(),
        consent_binding_id: ConsentBindingId::from_native_sequence(1).unwrap(),
    }
}
fn requested() -> SecurityEvent {
    SecurityEvent::Stop {
        barrier: Barrier::StopListening,
        stage: StopStage::Requested,
    }
}
// These explicit small bounds belong solely to these fixtures. Phase 0 did not
// approve a production audit count, byte budget or retention period.
struct Fixture<const N: usize> {
    clock: ManualClock,
    audit: AuditWindow<N>,
}
impl<const N: usize> Fixture<N> {
    fn new() -> Self {
        let clock = ManualClock::new();
        let expiry = Deadline::after(clock.now(), Duration::from_secs(60)).unwrap();
        let audit = AuditWindow::synthetic(
            identity(),
            FixtureBounds {
                record_bytes: 2048,
                total_bytes: 16384,
            },
            expiry,
            clock.now(),
        )
        .unwrap();
        Self { clock, audit }
    }
    fn prepare(&mut self, event: SecurityEvent) -> Result<SyntheticNativeEvidence> {
        self.audit
            .synthetic_native_event(self.audit.identity, event, &self.clock.now())
    }
    fn add(&mut self, event: SecurityEvent) {
        let evidence = self.prepare(event).unwrap();
        self.audit.append(&evidence, &self.clock.now()).unwrap();
    }
    fn bytes(&mut self) -> Vec<u8> {
        self.audit.serialize(&self.clock.now()).unwrap()
    }
}
fn description() -> Description {
    Description {
        evidence: EvidenceLabel::SyntheticNative,
        version: ContractVersion::V1,
        identity: identity(),
        sequence: NonZeroU64::new(1).unwrap(),
        elapsed_millis: 0,
        event: requested(),
    }
}
fn decode_value(value: Value) -> Result<UntrustedDescription> {
    let bytes = serde_json::to_vec(&value).unwrap();
    decode_description(&bytes, bytes.len())
}

#[test]
fn records_are_versioned_metadata_and_visibly_synthetic() {
    let mut f = Fixture::<2>::new();
    f.add(requested());
    let bytes = f.bytes();
    let v: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(v["version"], 1);
    assert_eq!(v["evidence"], "synthetic_native");
    assert_eq!(v["event"]["stage"], "requested");
    let decoded = decode_description(&bytes, bytes.len()).unwrap();
    assert_eq!(decoded.0.identity, identity());
    assert_eq!(decoded.0.event, requested());
}

#[test]
fn unsupported_audit_versions_and_unknown_schema_fields_are_rejected() {
    for version in [
        json!(0),
        json!(2),
        json!(-1),
        json!(1.2),
        json!("1"),
        json!(true),
    ] {
        let mut v = serde_json::to_value(description()).unwrap();
        v["version"] = version;
        assert!(matches!(decode_value(v), Err(Error::Malformed)));
    }
    let mut v = serde_json::to_value(description()).unwrap();
    v["unknown"] = json!(true);
    assert!(matches!(decode_value(v), Err(Error::Malformed)));
}

#[test]
fn native_authority_labels_and_unsupported_event_kinds_fail_closed() {
    let mut v = serde_json::to_value(description()).unwrap();
    v["evidence"] = json!("authoritative_native");
    assert!(matches!(decode_value(v), Err(Error::Malformed)));
    for event in [
        json!({"event":"workflow_completed"}),
        json!({"event":"tool_output","result":"done"}),
        json!({"event":"egress","role":"conversation","outcome":"verified_success"}),
    ] {
        let mut v = serde_json::to_value(description()).unwrap();
        v["event"] = event;
        assert!(matches!(decode_value(v), Err(Error::Malformed)));
    }
}

#[test]
fn all_required_security_event_categories_have_closed_metadata_shapes() {
    let events = [
        SecurityEvent::Consent {
            domain: ConsentDomain::Microphone,
            decision: ConsentDecision::Confirmed,
        },
        SecurityEvent::Consent {
            domain: ConsentDomain::Conversation,
            decision: ConsentDecision::Revoked,
        },
        SecurityEvent::Egress {
            role: ProviderRole::Stt,
            outcome: EgressOutcome::Authorized,
        },
        SecurityEvent::Egress {
            role: ProviderRole::Conversation,
            outcome: EgressOutcome::Denied,
        },
        SecurityEvent::Egress {
            role: ProviderRole::Tts,
            outcome: EgressOutcome::Uncertain,
        },
        SecurityEvent::Microphone {
            owner: MicrophoneOwner::ConversationTurn,
            stage: InputStage::StoppingUnconfirmed,
        },
        SecurityEvent::Stop {
            barrier: Barrier::Lock,
            stage: StopStage::Requested,
        },
        SecurityEvent::ConfigurationChanged {
            previous: ConfigurationRevision::from_native_sequence(1).unwrap(),
            next: identity().configuration_revision,
        },
        SecurityEvent::Cleanup {
            resource: CleanupResource::FramesAndQueues,
            outcome: CleanupOutcome::Failed,
        },
    ];
    let mut f = Fixture::<9>::new();
    for event in events {
        f.add(event);
    }
    let output = f.bytes();
    assert_eq!(
        output
            .split(|b| *b == b'\n')
            .filter(|s| !s.is_empty())
            .count(),
        9
    );
    for line in output.split(|b| *b == b'\n').filter(|s| !s.is_empty()) {
        assert!(decode_description(line, line.len()).is_ok());
    }
}

#[test]
fn sensitive_content_fields_are_rejected_at_every_schema_boundary() {
    for field in [
        "audio",
        "frames",
        "transcript",
        "private_message",
        "document",
        "api_key",
        "oauth_token",
        "refresh_token",
        "tool_input",
        "tool_output",
        "provider_body",
        "endpoint",
        "path",
        "private_content_hash",
    ] {
        for nested in [false, true] {
            let mut v = serde_json::to_value(description()).unwrap();
            if nested {
                v["event"][field] = json!("secret-fixture-never-echo");
            } else {
                v[field] = json!("secret-fixture-never-echo");
            }
            let error = decode_value(v).err().unwrap();
            assert_eq!(error, Error::Malformed);
            assert!(!format!("{error:?}").contains("secret-fixture"));
        }
    }
}

#[test]
fn raw_sensitive_untrusted_input_cannot_create_any_record_or_echo_in_errors() {
    let mut f = Fixture::<2>::new();
    for bytes in [
        b"synthetic-api-key-123".as_slice(),
        b"private-message-fixture",
        b"audio-bytes-fixture",
        br#"{"source":"native","confirmed":true,"transcript":"private"}"#,
    ] {
        let error = f.audit.ingest_untrusted(bytes).unwrap_err();
        assert_eq!(error, Error::Untrusted);
        assert!(!format!("{error:?}").contains("private"));
    }
    assert_eq!(f.audit.count, 0);
    assert!(f.bytes().is_empty());
}

#[test]
fn deserialized_metadata_and_copied_serialized_records_never_attest_native_outcomes() {
    let mut f = Fixture::<2>::new();
    let bytes = serde_json::to_vec(&description()).unwrap();
    assert!(decode_description(&bytes, bytes.len()).is_ok());
    assert_eq!(f.audit.ingest_untrusted(&bytes), Err(Error::Untrusted));
    f.add(requested());
    let bytes = f.bytes();
    let mut other = Fixture::<2>::new();
    assert_eq!(other.audit.ingest_untrusted(&bytes), Err(Error::Untrusted));
    assert!(other.bytes().is_empty());
}

#[test]
fn serialized_records_contain_no_transcripts_audio_secrets_or_arbitrary_tool_fields() {
    let mut f = Fixture::<3>::new();
    f.add(SecurityEvent::Consent {
        domain: ConsentDomain::PrivateContext,
        decision: ConsentDecision::Declined,
    });
    f.add(SecurityEvent::Egress {
        role: ProviderRole::Planning,
        outcome: EgressOutcome::Attempted,
    });
    let bytes = f.bytes();
    let output = String::from_utf8(bytes).unwrap();
    for prohibited in [
        "fixture-api-key",
        "fixture-transcript",
        "fixture-audio",
        "oauth_token",
        "tool_input",
        "document_content",
        "http://",
        "https://",
    ] {
        assert!(!output.contains(prohibited));
    }
    for line in output.lines() {
        let v: Value = serde_json::from_str(line).unwrap();
        assert_eq!(v.as_object().unwrap().len(), 6);
        assert!(v.get("identity").is_some() && v.get("sequence").is_some());
    }
}

#[test]
fn requested_attempted_and_uncertain_outcomes_serialize_without_promotion() {
    let mut f = Fixture::<4>::new();
    f.add(SecurityEvent::Egress {
        role: ProviderRole::Conversation,
        outcome: EgressOutcome::Attempted,
    });
    f.add(SecurityEvent::Egress {
        role: ProviderRole::Conversation,
        outcome: EgressOutcome::Uncertain,
    });
    f.add(SecurityEvent::Stop {
        barrier: Barrier::Quit,
        stage: StopStage::TerminationUnconfirmed,
    });
    f.add(SecurityEvent::Cleanup {
        resource: CleanupResource::Input,
        outcome: CleanupOutcome::Requested,
    });
    let output = String::from_utf8(f.bytes()).unwrap();
    assert!(
        output.contains("attempted")
            && output.contains("uncertain")
            && output.contains("termination_unconfirmed")
            && output.contains("requested")
    );
    assert!(
        !output.contains("success")
            && !output.contains("acknowledged")
            && !output.contains("completed")
    );
}

#[test]
fn microphone_off_or_timeout_descriptions_cannot_be_release_acknowledgements() {
    let mut f = Fixture::<2>::new();
    assert!(matches!(
        f.prepare(SecurityEvent::Microphone {
            owner: MicrophoneOwner::Off,
            stage: InputStage::ReleaseAcknowledged
        }),
        Err(Error::InvalidEvent)
    ));
    f.add(SecurityEvent::Microphone {
        owner: MicrophoneOwner::Off,
        stage: InputStage::OwnerOff,
    });
    f.add(SecurityEvent::Microphone {
        owner: MicrophoneOwner::WakeDetection,
        stage: InputStage::StoppingUnconfirmed,
    });
    let output = String::from_utf8(f.bytes()).unwrap();
    assert!(!output.contains("release_acknowledged"));
    let mut v = serde_json::to_value(description()).unwrap();
    v["event"] = json!({"event":"microphone","owner":"wake_detection","stage":"release_acknowledged","reason":"timeout"});
    assert!(matches!(decode_value(v), Err(Error::Malformed)));
}

#[test]
fn configuration_events_require_exact_adjacent_revisions_and_current_native_binding() {
    let mut f = Fixture::<1>::new();
    for (previous, next) in [(1, 3), (2, 2), (2, 3), (u64::MAX, 2)] {
        assert!(f
            .prepare(SecurityEvent::ConfigurationChanged {
                previous: ConfigurationRevision::from_native_sequence(previous).unwrap(),
                next: ConfigurationRevision::from_native_sequence(next).unwrap()
            })
            .is_err());
    }
    assert_eq!(f.audit.count, 0);
    f.add(SecurityEvent::ConfigurationChanged {
        previous: ConfigurationRevision::from_native_sequence(1).unwrap(),
        next: f.audit.identity.configuration_revision,
    });
}

#[test]
fn exact_identity_and_generation_are_required_for_every_event() {
    let mutations: [fn(&mut ContractIdentity); 8] = [
        |i| i.app_run_id = AppRunId::from_native_sequence(2).unwrap(),
        |i| i.interaction_id = InteractionId::from_native_sequence(2).unwrap(),
        |i| i.session_id = SessionId::from_native_sequence(2).unwrap(),
        |i| i.turn_id = TurnId::from_native_sequence(2).unwrap(),
        |i| i.job_id = JobId::from_native_sequence(2).unwrap(),
        |i| i.generation = Generation::from_native_sequence(2).unwrap(),
        |i| i.configuration_revision = ConfigurationRevision::from_native_sequence(3).unwrap(),
        |i| i.consent_binding_id = ConsentBindingId::from_native_sequence(2).unwrap(),
    ];
    let mut f = Fixture::<2>::new();
    for mutate in mutations {
        let mut id = identity();
        mutate(&mut id);
        assert!(matches!(
            f.audit
                .synthetic_native_event(id, requested(), &f.clock.now()),
            Err(Error::Stale)
        ));
    }
    assert!(f.audit.pending.is_none());
    assert_eq!(f.audit.count, 0);
}

#[test]
fn foreign_native_evidence_with_identical_numeric_identity_is_rejected() {
    let mut f = Fixture::<2>::new();
    let mut other = Fixture::<2>::new();
    let local = f.prepare(requested()).unwrap();
    let foreign = other.prepare(requested()).unwrap();
    assert_eq!(f.audit.append(&foreign, &f.clock.now()), Err(Error::Stale));
    f.audit.append(&local, &f.clock.now()).unwrap();
    assert_eq!(f.audit.count, 1);
}

#[test]
fn copied_metadata_and_changed_event_cannot_replace_the_exact_native_receipt() {
    let mut f = Fixture::<2>::new();
    let evidence = f.prepare(requested()).unwrap();
    let mut description = evidence.0.description;
    description.event = SecurityEvent::Cleanup {
        resource: CleanupResource::Workers,
        outcome: CleanupOutcome::Acknowledged,
    };
    let forged = SyntheticNativeEvidence(Arc::new(Draft {
        owner: evidence.0.owner.clone(),
        observed_at: f.clock.now(),
        description,
    }));
    assert_eq!(f.audit.append(&forged, &f.clock.now()), Err(Error::Stale));
    f.audit.append(&evidence, &f.clock.now()).unwrap();
    assert_eq!(
        f.audit.records[0].as_ref().unwrap().description.event,
        requested()
    );
}

#[test]
fn duplicate_old_or_out_of_order_evidence_never_replays_or_discards_current_event() {
    let mut f = Fixture::<3>::new();
    let first = f.prepare(requested()).unwrap();
    assert!(matches!(f.prepare(requested()), Err(Error::Busy)));
    f.audit.append(&first, &f.clock.now()).unwrap();
    assert_eq!(f.audit.append(&first, &f.clock.now()), Err(Error::Stale));
    let second = f
        .prepare(SecurityEvent::Stop {
            barrier: Barrier::Sleep,
            stage: StopStage::Requested,
        })
        .unwrap();
    assert_eq!(f.audit.append(&first, &f.clock.now()), Err(Error::Stale));
    f.audit.append(&second, &f.clock.now()).unwrap();
    assert_eq!(
        f.audit.records[0]
            .as_ref()
            .unwrap()
            .description
            .sequence
            .get(),
        1
    );
    assert_eq!(
        f.audit.records[1]
            .as_ref()
            .unwrap()
            .description
            .sequence
            .get(),
        2
    );
}

#[test]
fn fixed_event_capacity_never_evicts_or_silently_accepts_an_extra_event() {
    let mut f = Fixture::<2>::new();
    f.add(requested());
    f.add(requested());
    let before = f.bytes();
    assert!(matches!(f.prepare(requested()), Err(Error::Full)));
    assert_eq!(f.audit.count, 2);
    assert_eq!(f.bytes(), before);
}

#[test]
fn zero_capacity_or_missing_fixture_budgets_are_rejected() {
    let clock = ManualClock::new();
    assert!(matches!(
        AuditWindow::<0>::synthetic(
            identity(),
            FixtureBounds {
                record_bytes: 1000,
                total_bytes: 2000
            },
            Deadline::after(clock.now(), Duration::from_secs(1)).unwrap(),
            clock.now()
        ),
        Err(Error::Full)
    ));
    for (record_bytes, total_bytes) in [(0, 1), (1, 0), (0, 0)] {
        assert!(matches!(
            AuditWindow::<1>::synthetic(
                identity(),
                FixtureBounds {
                    record_bytes,
                    total_bytes
                },
                Deadline::after(clock.now(), Duration::from_secs(1)).unwrap(),
                clock.now()
            ),
            Err(Error::Full)
        ));
    }
}

#[test]
fn exact_record_and_aggregate_byte_bounds_include_jsonl_framing() {
    let mut probe = Fixture::<1>::new();
    probe.add(requested());
    let size = serde_json::to_vec(probe.audit.records[0].as_ref().unwrap())
        .unwrap()
        .len() as u64;
    let mut f = Fixture::<1>::new();
    f.audit.bounds = FixtureBounds {
        record_bytes: size,
        total_bytes: size + 1,
    };
    f.add(requested());
    assert_eq!(f.bytes().len() as u64, size + 1);
    for bounds in [
        FixtureBounds {
            record_bytes: size - 1,
            total_bytes: size + 1,
        },
        FixtureBounds {
            record_bytes: size,
            total_bytes: size,
        },
    ] {
        let mut f = Fixture::<1>::new();
        f.audit.bounds = bounds;
        let evidence = f.prepare(requested()).unwrap();
        assert_eq!(
            f.audit.append(&evidence, &f.clock.now()),
            Err(Error::TooLarge)
        );
        assert_eq!(f.audit.count, 0);
        assert_eq!(f.audit.sequence, 0);
        assert!(f.audit.pending.is_none());
        assert_eq!(f.audit.append(&evidence, &f.clock.now()), Err(Error::Stale));
    }
}

#[test]
fn expiry_at_exact_native_deadline_clears_references_without_renewal() {
    let mut f = Fixture::<2>::new();
    let old = f.clock.now();
    f.add(requested());
    f.clock.advance(Duration::from_secs(59)).unwrap();
    assert!(!f.bytes().is_empty());
    f.clock.advance(Duration::from_secs(1)).unwrap();
    assert_eq!(f.audit.serialize(&f.clock.now()), Err(Error::Expired));
    assert!(f.audit.closed && f.audit.records.iter().all(Option::is_none));
    assert_eq!(f.audit.serialize(&old), Err(Error::Closed));
    assert_eq!(f.audit.sequence, 1);
}

#[test]
fn pending_event_cannot_extend_retention_or_append_after_expiry() {
    let mut f = Fixture::<2>::new();
    let evidence = f.prepare(requested()).unwrap();
    f.clock.advance(Duration::from_secs(60)).unwrap();
    assert_eq!(
        f.audit.append(&evidence, &f.clock.now()),
        Err(Error::Expired)
    );
    assert!(f.audit.pending.is_none());
    assert_eq!(f.audit.count, 0);
}

#[test]
fn expired_or_foreign_retention_deadlines_do_not_create_a_window() {
    let clock = ManualClock::new();
    let other = ManualClock::new();
    assert!(matches!(
        AuditWindow::<1>::synthetic(
            identity(),
            FixtureBounds {
                record_bytes: 1000,
                total_bytes: 2000
            },
            Deadline::after(clock.now(), Duration::ZERO).unwrap(),
            clock.now()
        ),
        Err(Error::Expired)
    ));
    assert!(matches!(
        AuditWindow::<1>::synthetic(
            identity(),
            FixtureBounds {
                record_bytes: 1000,
                total_bytes: 2000
            },
            Deadline::after(other.now(), Duration::from_secs(1)).unwrap(),
            clock.now()
        ),
        Err(Error::Contract(ContractError::ClockDomainMismatch))
    ));
}

#[test]
fn clock_domain_errors_and_rollback_close_audit_admission_and_exports() {
    for foreign in [false, true] {
        let mut f = Fixture::<2>::new();
        let old = f.clock.now();
        f.add(requested());
        f.clock.advance(Duration::from_secs(1)).unwrap();
        f.audit.observe(&f.clock.now()).unwrap();
        let bad = if foreign {
            ManualClock::new().now()
        } else {
            old
        };
        assert!(matches!(f.audit.serialize(&bad), Err(Error::Contract(_))));
        assert!(f.audit.closed);
        assert_eq!(f.audit.count, 0);
        assert!(matches!(f.prepare(requested()), Err(Error::Closed)));
    }
}

#[test]
fn final_sequence_is_accepted_once_and_exhaustion_never_wraps() {
    let mut f = Fixture::<2>::new();
    f.audit.sequence = u64::MAX - 1;
    f.add(requested());
    assert_eq!(f.audit.sequence, u64::MAX);
    assert!(!f.bytes().is_empty());
    assert!(matches!(
        f.prepare(requested()),
        Err(Error::Contract(ContractError::Overflow))
    ));
    assert!(f.audit.closed);
    assert_eq!(f.audit.sequence, u64::MAX);
}

#[test]
fn aggregate_byte_arithmetic_overflow_is_fail_closed() {
    let mut f = Fixture::<2>::new();
    let evidence = f.prepare(requested()).unwrap();
    f.audit.bytes = u64::MAX;
    assert_eq!(
        f.audit.append(&evidence, &f.clock.now()),
        Err(Error::Contract(ContractError::Overflow))
    );
    assert!(f.audit.closed);
    assert_eq!(f.audit.count, 0);
    assert_eq!(f.audit.serialize(&f.clock.now()), Err(Error::Closed));
}

#[test]
fn elapsed_time_encoding_overflow_closes_the_window_without_truncation() {
    let clock = ManualClock::new();
    let expiry = Deadline::after(clock.now(), Duration::MAX).unwrap();
    let audit = AuditWindow::<1>::synthetic(
        identity(),
        FixtureBounds {
            record_bytes: 2048,
            total_bytes: 4096,
        },
        expiry,
        clock.now(),
    )
    .unwrap();
    let mut f = Fixture { clock, audit };
    f.clock.advance(Duration::from_secs(u64::MAX)).unwrap();
    assert!(matches!(
        f.prepare(requested()),
        Err(Error::Contract(ContractError::Overflow))
    ));
    assert!(f.audit.closed && f.audit.pending.is_none());
}

#[test]
fn explicit_close_removes_window_references_without_claiming_secure_deletion_or_runtime_stop() {
    let mut f = Fixture::<2>::new();
    f.add(requested());
    let pending = f.prepare(requested()).unwrap();
    f.audit.close();
    assert_eq!(f.audit.count, 0);
    assert_eq!(f.audit.bytes, 0);
    assert_eq!(f.audit.append(&pending, &f.clock.now()), Err(Error::Stale));
    assert_eq!(f.audit.serialize(&f.clock.now()), Err(Error::Closed));
    assert_eq!(f.audit.sequence, 1);
}

#[test]
fn malformed_duplicate_nested_or_zero_identity_inputs_do_not_echo_content() {
    let bytes = serde_json::to_string(&description()).unwrap().replacen(
        "\"version\":1",
        "\"version\":1,\"version\":1",
        1,
    );
    assert!(matches!(
        decode_description(bytes.as_bytes(), bytes.len()),
        Err(Error::Malformed)
    ));
    let mut v = serde_json::to_value(description()).unwrap();
    v["identity"]["job_id"] = json!(0);
    assert!(matches!(decode_value(v), Err(Error::Malformed)));
    let mut v = serde_json::to_value(description()).unwrap();
    v["identity"]["secret"] = json!("fixture-secret");
    let error = decode_value(v).err().unwrap();
    assert_eq!(error, Error::Malformed);
    assert!(!format!("{error:?}").contains("fixture-secret"));
}

#[test]
fn exact_untrusted_input_size_limit_and_sequence_shape_are_validated() {
    let bytes = serde_json::to_vec(&description()).unwrap();
    assert!(decode_description(&bytes, bytes.len()).is_ok());
    assert!(matches!(
        decode_description(&bytes, bytes.len() - 1),
        Err(Error::TooLarge)
    ));
    let mut v = serde_json::to_value(description()).unwrap();
    v["sequence"] = json!(0);
    assert!(matches!(decode_value(v), Err(Error::Malformed)));
}

#[test]
fn all_privacy_barriers_retain_requested_or_unconfirmed_status() {
    for barrier in [
        Barrier::StopListening,
        Barrier::Lock,
        Barrier::Sleep,
        Barrier::Logout,
        Barrier::Quit,
        Barrier::PermissionLost,
        Barrier::DeviceLost,
        Barrier::HostTerminated,
    ] {
        let mut f = Fixture::<1>::new();
        let event = SecurityEvent::Stop {
            barrier,
            stage: StopStage::StoppingUnconfirmed,
        };
        f.add(event);
        let bytes = f.bytes();
        assert_eq!(
            decode_description(&bytes, bytes.len()).unwrap().0.event,
            event
        );
        assert!(!String::from_utf8(bytes).unwrap().contains("acknowledged"));
    }
}
