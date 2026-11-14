//! Fake native facts and duration metadata only. No audio/model/device resources.
use super::*;
use crate::voice_conversation::clock::{ManualClock, MonotonicClock};
use crate::voice_conversation::model::*;

fn identity() -> ContractIdentity {
    ContractIdentity {
        version: ContractVersion::V1,
        app_run_id: AppRunId::from_native_sequence(1).unwrap(),
        interaction_id: InteractionId::from_native_sequence(1).unwrap(),
        session_id: SessionId::from_native_sequence(1).unwrap(),
        turn_id: TurnId::from_native_sequence(1).unwrap(),
        job_id: JobId::from_native_sequence(1).unwrap(),
        generation: Generation::from_native_sequence(1).unwrap(),
        configuration_revision: ConfigurationRevision::from_native_sequence(1).unwrap(),
        consent_binding_id: ConsentBindingId::from_native_sequence(1).unwrap(),
    }
}

struct Fixture {
    clock: ManualClock,
    model: Arc<()>,
    grant: Deadline,
    wake: WakeController,
}
impl Fixture {
    fn new() -> Self {
        let clock = ManualClock::new();
        let grant =
            Deadline::after(clock.now(), MAX_WAKE_ARMING + Duration::from_secs(600)).unwrap();
        let wake = WakeController::new(identity().app_run_id, clock.now()).unwrap();
        Self {
            clock,
            model: Arc::new(()),
            grant,
            wake,
        }
    }
    fn facts(&self) -> Validation {
        Validation {
            identity: identity(),
            model_instance: self.model.clone(),
            grant_deadline: self.grant.clone(),
            fresh_activation: true,
            consent_valid: true,
            model_integrity_valid: true,
            model_license_eligible: true,
            privacy_clear: true,
            microphone_permission: true,
            microphone_lease_valid: true,
            microphone_owner: MicrophoneOwner::WakeDetection,
            speech_active: false,
            foreground: true,
            background_allowed: true,
        }
    }
    fn arm(&mut self) {
        self.wake.arm(1, &self.facts(), &self.clock.now()).unwrap();
    }
    fn event(&mut self, event: Event) -> Result<Option<AttentionCandidate>> {
        let callback = self.wake.callback.clone().unwrap();
        self.wake
            .handle(&callback, event, &self.facts(), &self.clock.now())
    }
    fn armed(&mut self) {
        self.arm();
        self.event(Event::Ready).unwrap();
    }
    fn advance(&mut self) -> Result<()> {
        self.wake.advance(&self.facts(), &self.clock.now())
    }
    fn at(state: WakeState) -> Self {
        let mut f = Self::new();
        if state == WakeState::Disabled {
            return f;
        }
        f.arm();
        if state == WakeState::Preparing {
            return f;
        }
        f.event(Event::Ready).unwrap();
        match state {
            WakeState::Armed => (),
            WakeState::Cooldown => {
                f.event(Event::Detected {
                    retained: WAKE_RING_BUFFER,
                })
                .unwrap();
            }
            WakeState::Suppressed => {
                let mut facts = f.facts();
                facts.speech_active = true;
                assert_eq!(
                    f.wake.advance(&facts, &f.clock.now()),
                    Err(Error::Suppressed)
                );
            }
            WakeState::Suspended => f.wake.interrupt(Barrier::Lock).unwrap(),
            WakeState::Faulted => {
                assert_eq!(f.event(Event::Failed), Err(Error::DetectorFailed));
            }
            _ => unreachable!(),
        }
        f
    }
    fn snapshot(&self) -> String {
        let callback = self
            .wake
            .callback
            .as_ref()
            .map(|c| (c.identity, c.generation, c.operation, c.stage));
        let deadlines = self.wake.arming.as_ref().map(|a| {
            (
                a.lifetime_deadline.remaining(&self.clock.now()).unwrap(),
                a.grant_deadline.remaining(&self.clock.now()).unwrap(),
            )
        });
        format!(
            "{:?}/{:?}/{}/{}/{}/{:?}/{:?}/{:?}/{:?}",
            self.wake.state,
            self.wake.generation,
            self.wake.operation_sequence,
            self.wake.activation_sequence,
            self.wake.retired,
            self.wake.retained_duration,
            callback,
            deadlines,
            self.clock
                .now()
                .checked_duration_since(&self.wake.last_time)
                .unwrap()
        )
    }
}

const STATES: [WakeState; 7] = [
    WakeState::Disabled,
    WakeState::Preparing,
    WakeState::Armed,
    WakeState::Suppressed,
    WakeState::Cooldown,
    WakeState::Suspended,
    WakeState::Faulted,
];

#[test]
fn default_is_disabled_without_resources_callbacks_or_automatic_arming() {
    let mut f = Fixture::new();
    assert_eq!(f.wake.state, WakeState::Disabled);
    assert!(f.wake.arming.is_none() && f.wake.callback.is_none());
    assert_eq!(f.wake.retained_duration, Duration::ZERO);
    assert_eq!(f.advance(), Err(Error::InvalidTransition));
    assert_eq!(f.wake.state, WakeState::Disabled);
}

#[test]
fn arming_requires_every_independent_native_validation_fact() {
    for missing in 0..9 {
        let mut f = Fixture::new();
        let mut facts = f.facts();
        match missing {
            0 => facts.fresh_activation = false,
            1 => facts.consent_valid = false,
            2 => facts.model_integrity_valid = false,
            3 => facts.model_license_eligible = false,
            4 => facts.privacy_clear = false,
            5 => facts.microphone_permission = false,
            6 => {
                facts.foreground = false;
                facts.background_allowed = false;
            }
            7 => facts.grant_deadline = Deadline::after(f.clock.now(), Duration::ZERO).unwrap(),
            _ => facts.identity.app_run_id = AppRunId::from_native_sequence(2).unwrap(),
        }
        let before = f.snapshot();
        let expected = if matches!(missing, 2 | 3) {
            Error::ModelIneligible
        } else {
            Error::Unauthorized
        };
        assert_eq!(f.wake.arm(1, &facts, &f.clock.now()), Err(expected));
        assert_eq!(f.snapshot(), before);
    }
}

#[test]
fn normal_lifecycle_returns_only_attention_metadata_and_enters_cooldown() {
    let mut f = Fixture::new();
    f.arm();
    assert_eq!(f.wake.state, WakeState::Preparing);
    f.event(Event::Ready).unwrap();
    assert_eq!(f.wake.state, WakeState::Armed);
    f.event(Event::Window {
        retained: WAKE_RING_BUFFER,
    })
    .unwrap();
    assert_eq!(f.wake.retained_duration, WAKE_RING_BUFFER);
    let detector = f.wake.callback.clone().unwrap();
    let candidate = f
        .event(Event::Detected {
            retained: WAKE_RING_BUFFER,
        })
        .unwrap()
        .unwrap();
    assert_eq!(candidate.identity, identity());
    assert_eq!(candidate.generation, detector.generation);
    assert_eq!(candidate.detection, detector.operation);
    assert_eq!(f.wake.state, WakeState::Cooldown);
    assert!(f.wake.callback.is_none());
    assert_eq!(f.wake.retained_duration, Duration::ZERO);
}

#[test]
fn cooldown_requires_three_seconds_and_a_new_preparation_callback() {
    let mut f = Fixture::new();
    f.armed();
    let old = f.wake.callback.clone().unwrap();
    f.event(Event::Detected {
        retained: WAKE_RING_BUFFER,
    })
    .unwrap();
    let before = f.snapshot();
    assert_eq!(
        f.wake.handle(
            &old,
            Event::Detected {
                retained: WAKE_RING_BUFFER
            },
            &f.facts(),
            &f.clock.now()
        ),
        Err(Error::Stale)
    );
    assert_eq!(f.snapshot(), before);
    f.clock
        .advance(WAKE_COOLDOWN - Duration::from_nanos(1))
        .unwrap();
    f.advance().unwrap();
    assert_eq!(f.wake.state, WakeState::Cooldown);
    f.clock.advance(Duration::from_nanos(1)).unwrap();
    f.advance().unwrap();
    assert_eq!(f.wake.state, WakeState::Preparing);
    assert_eq!(
        f.event(Event::Detected {
            retained: WAKE_RING_BUFFER
        }),
        Err(Error::InvalidTransition)
    );
    f.event(Event::Ready).unwrap();
    assert_eq!(
        f.wake.handle(
            &old,
            Event::Detected {
                retained: WAKE_RING_BUFFER
            },
            &f.facts(),
            &f.clock.now()
        ),
        Err(Error::Stale)
    );
    assert!(f
        .event(Event::Detected {
            retained: WAKE_RING_BUFFER
        })
        .unwrap()
        .is_some());
}

#[test]
fn every_state_rejects_illegal_or_fabricated_detector_events() {
    for state in STATES {
        for event in [
            Event::Ready,
            Event::Window {
                retained: WAKE_RING_BUFFER,
            },
            Event::Detected {
                retained: WAKE_RING_BUFFER,
            },
            Event::Failed,
        ] {
            let mut f = Fixture::at(state);
            let valid = matches!(
                (state, event),
                (WakeState::Preparing, Event::Ready | Event::Failed)
                    | (
                        WakeState::Armed,
                        Event::Window { .. } | Event::Detected { .. } | Event::Failed
                    )
            );
            if valid {
                continue;
            }
            let callback = f.wake.callback.clone().unwrap_or_else(|| Callback {
                native_instance: f.wake.native_instance.clone(),
                model_instance: f.model.clone(),
                nonce: Arc::new(()),
                identity: identity(),
                generation: f.wake.generation,
                operation: DetectorOperation(NonZeroU64::new(1).unwrap()),
                stage: state,
            });
            let before = f.snapshot();
            let expected = if f.wake.callback.is_some() {
                Error::InvalidTransition
            } else {
                Error::Stale
            };
            assert_eq!(
                f.wake.handle(&callback, event, &f.facts(), &f.clock.now()),
                Err(expected)
            );
            assert_eq!(f.snapshot(), before);
        }
    }
}

#[test]
fn all_identity_generation_instance_operation_and_stage_mismatches_are_inert() {
    let mut f = Fixture::new();
    f.armed();
    let original = f.wake.callback.clone().unwrap();
    let mut variants = Vec::new();
    macro_rules! foreign {
        ($field:ident, $value:expr) => {{
            let mut c = original.clone();
            c.identity.$field = $value;
            variants.push(c);
        }};
    }
    foreign!(app_run_id, AppRunId::from_native_sequence(2).unwrap());
    foreign!(
        interaction_id,
        InteractionId::from_native_sequence(2).unwrap()
    );
    foreign!(session_id, SessionId::from_native_sequence(2).unwrap());
    foreign!(turn_id, TurnId::from_native_sequence(2).unwrap());
    foreign!(job_id, JobId::from_native_sequence(2).unwrap());
    foreign!(generation, Generation::from_native_sequence(2).unwrap());
    foreign!(
        configuration_revision,
        ConfigurationRevision::from_native_sequence(2).unwrap()
    );
    foreign!(
        consent_binding_id,
        ConsentBindingId::from_native_sequence(2).unwrap()
    );
    let mut c = original.clone();
    c.generation = c.generation.checked_next().unwrap();
    variants.push(c);
    let mut c = original.clone();
    c.operation = DetectorOperation(NonZeroU64::new(99).unwrap());
    variants.push(c);
    let mut c = original.clone();
    c.stage = WakeState::Preparing;
    variants.push(c);
    let mut c = original.clone();
    c.native_instance = Arc::new(());
    variants.push(c);
    let mut c = original.clone();
    c.model_instance = Arc::new(());
    variants.push(c);
    let mut c = original.clone();
    c.nonce = Arc::new(());
    variants.push(c);
    let mut other = Fixture::new();
    other.armed();
    variants.push(other.wake.callback.clone().unwrap());
    let before = f.snapshot();
    for callback in variants {
        assert_eq!(
            f.wake.handle(
                &callback,
                Event::Detected {
                    retained: WAKE_RING_BUFFER
                },
                &f.facts(),
                &f.clock.now()
            ),
            Err(Error::Stale)
        );
        assert_eq!(f.snapshot(), before);
    }
}

#[test]
fn duplicate_ready_window_and_detection_callbacks_cannot_replay() {
    let mut f = Fixture::new();
    f.arm();
    for event in [
        Event::Ready,
        Event::Window {
            retained: WAKE_RING_BUFFER,
        },
        Event::Detected {
            retained: WAKE_RING_BUFFER,
        },
    ] {
        let old = f.wake.callback.clone().unwrap();
        f.event(event).unwrap();
        let before = f.snapshot();
        assert_eq!(
            f.wake.handle(&old, event, &f.facts(), &f.clock.now()),
            Err(Error::Stale)
        );
        assert_eq!(f.snapshot(), before);
    }
}

#[test]
fn speech_and_other_microphone_owners_suppress_all_live_detector_work() {
    for state in [WakeState::Preparing, WakeState::Armed, WakeState::Cooldown] {
        for busy in 0..5 {
            let mut f = Fixture::at(state);
            let mut facts = f.facts();
            match busy {
                0 => facts.speech_active = true,
                1 => facts.microphone_owner = MicrophoneOwner::ManualForeground,
                2 => facts.microphone_owner = MicrophoneOwner::ConversationTurn,
                3 => facts.microphone_owner = MicrophoneOwner::Off,
                _ => facts.microphone_lease_valid = false,
            }
            let old = f.wake.callback.clone();
            assert_eq!(
                f.wake.advance(&facts, &f.clock.now()),
                Err(Error::Suppressed)
            );
            assert_eq!(f.wake.state, WakeState::Suppressed);
            assert!(f.wake.callback.is_none());
            assert_eq!(f.wake.retained_duration, Duration::ZERO);
            if let Some(old) = old {
                assert_eq!(
                    f.wake
                        .handle(&old, Event::Ready, &f.facts(), &f.clock.now()),
                    Err(Error::Stale)
                );
            }
        }
    }
}

#[test]
fn initially_missing_ownership_is_suppressed_without_creating_a_lease() {
    for owner in [
        MicrophoneOwner::Off,
        MicrophoneOwner::ManualForeground,
        MicrophoneOwner::ConversationTurn,
    ] {
        let mut f = Fixture::new();
        let mut facts = f.facts();
        facts.microphone_owner = owner;
        f.wake.arm(1, &facts, &f.clock.now()).unwrap();
        assert_eq!(f.wake.state, WakeState::Suppressed);
        assert!(f.wake.callback.is_none());
        assert_eq!(facts.microphone_owner, owner); // wake cannot transfer ownership
        assert_eq!(
            f.wake.advance(&facts, &f.clock.now()),
            Err(Error::Suppressed)
        );
    }
}

#[test]
fn clearing_suppression_prepares_without_renewing_arming_or_cooldown() {
    let mut f = Fixture::new();
    f.armed();
    f.event(Event::Detected {
        retained: WAKE_RING_BUFFER,
    })
    .unwrap();
    f.clock.advance(Duration::from_secs(1)).unwrap();
    let mut busy = f.facts();
    busy.speech_active = true;
    assert_eq!(
        f.wake.advance(&busy, &f.clock.now()),
        Err(Error::Suppressed)
    );
    let remaining = f
        .wake
        .arming
        .as_ref()
        .unwrap()
        .lifetime_deadline
        .remaining(&f.clock.now())
        .unwrap();
    f.advance().unwrap();
    assert_eq!(f.wake.state, WakeState::Cooldown);
    assert_eq!(
        f.wake
            .arming
            .as_ref()
            .unwrap()
            .lifetime_deadline
            .remaining(&f.clock.now())
            .unwrap(),
        remaining
    );
    f.clock.advance(Duration::from_secs(2)).unwrap();
    f.advance().unwrap();
    assert_eq!(f.wake.state, WakeState::Preparing);
    f.event(Event::Ready).unwrap();
    assert_eq!(
        f.wake
            .arming
            .as_ref()
            .unwrap()
            .lifetime_deadline
            .remaining(&f.clock.now())
            .unwrap(),
        MAX_WAKE_ARMING - WAKE_COOLDOWN
    );
}

#[test]
fn active_arming_cannot_be_reset_by_another_arm_request() {
    for state in [
        WakeState::Preparing,
        WakeState::Armed,
        WakeState::Suppressed,
        WakeState::Cooldown,
    ] {
        let mut f = Fixture::at(state);
        let before = f.snapshot();
        assert_eq!(
            f.wake.arm(2, &f.facts(), &f.clock.now()),
            Err(Error::InvalidTransition)
        );
        assert_eq!(f.snapshot(), before);
    }
}

#[test]
fn eight_hour_arming_expiry_is_exact_in_every_live_state() {
    for state in [
        WakeState::Preparing,
        WakeState::Armed,
        WakeState::Suppressed,
        WakeState::Cooldown,
    ] {
        for extra in [Duration::ZERO, Duration::from_nanos(1)] {
            let mut f = Fixture::at(state);
            f.clock.advance(MAX_WAKE_ARMING + extra).unwrap();
            assert_eq!(f.advance(), Err(Error::Expired));
            assert_eq!(f.wake.state, WakeState::Disabled);
            assert!(f.wake.arming.is_none() && f.wake.callback.is_none());
            assert_eq!(f.advance(), Err(Error::InvalidTransition));
            assert_eq!(f.wake.arm(1, &f.facts(), &f.clock.now()), Err(Error::Stale));
        }
    }
    let mut f = Fixture::new();
    f.armed();
    f.clock
        .advance(MAX_WAKE_ARMING - Duration::from_nanos(1))
        .unwrap();
    assert!(f
        .event(Event::Detected {
            retained: WAKE_RING_BUFFER
        })
        .unwrap()
        .is_some());
    f.clock.advance(Duration::from_nanos(1)).unwrap();
    assert_eq!(f.advance(), Err(Error::Expired)); // cooldown cannot extend lifetime
}

#[test]
fn original_grant_is_not_extended_by_callbacks_suppression_or_fresh_facts() {
    let mut f = Fixture::new();
    f.grant = Deadline::after(f.clock.now(), Duration::from_secs(1)).unwrap();
    f.arm();
    f.grant = Deadline::after(f.clock.now(), MAX_WAKE_ARMING).unwrap();
    f.event(Event::Ready).unwrap();
    let mut busy = f.facts();
    busy.speech_active = true;
    assert_eq!(
        f.wake.advance(&busy, &f.clock.now()),
        Err(Error::Suppressed)
    );
    f.advance().unwrap();
    f.event(Event::Ready).unwrap();
    f.clock.advance(Duration::from_secs(1)).unwrap();
    assert_eq!(
        f.event(Event::Detected {
            retained: WAKE_RING_BUFFER
        }),
        Err(Error::Expired)
    );
    assert_eq!(f.wake.state, WakeState::Disabled);
}

#[test]
fn newly_shortened_grant_deadline_stays_binding() {
    let mut f = Fixture::new();
    f.armed();
    let mut short = f.facts();
    short.grant_deadline = Deadline::after(f.clock.now(), Duration::from_secs(1)).unwrap();
    f.wake.advance(&short, &f.clock.now()).unwrap();
    f.advance().unwrap();
    f.clock.advance(Duration::from_secs(1)).unwrap();
    assert_eq!(
        f.event(Event::Detected {
            retained: WAKE_RING_BUFFER
        }),
        Err(Error::Expired)
    );
}

#[test]
fn ring_limit_is_duration_metadata_only_and_exact_at_two_seconds() {
    for retained in [
        Duration::ZERO,
        Duration::from_nanos(1),
        WAKE_RING_BUFFER,
        WAKE_RING_BUFFER + Duration::from_nanos(1),
        Duration::MAX,
    ] {
        for detected in [false, true] {
            let mut f = Fixture::new();
            f.armed();
            let event = if detected {
                Event::Detected { retained }
            } else {
                Event::Window { retained }
            };
            let result = f.event(event);
            if retained > WAKE_RING_BUFFER || (detected && retained.is_zero()) {
                assert_eq!(result, Err(ContractError::LimitExceeded.into()));
                assert_eq!(f.wake.state, WakeState::Faulted);
                assert!(f.wake.arming.is_none());
            } else {
                assert!(result.is_ok());
                assert_eq!(
                    f.wake.retained_duration,
                    if detected { Duration::ZERO } else { retained }
                );
            }
        }
    }
}

#[test]
fn privacy_barriers_revoke_all_live_states_and_require_fresh_activation() {
    for state in [
        WakeState::Preparing,
        WakeState::Armed,
        WakeState::Suppressed,
        WakeState::Cooldown,
    ] {
        for barrier in [
            Barrier::Lock,
            Barrier::Sleep,
            Barrier::Logout,
            Barrier::PermissionRevoked,
            Barrier::Pause,
            Barrier::DeviceLost,
            Barrier::StopListening,
            Barrier::Quit,
        ] {
            let mut f = Fixture::at(state);
            let old = f.wake.callback.clone();
            f.wake.interrupt(barrier).unwrap();
            let target = if matches!(barrier, Barrier::StopListening | Barrier::Quit) {
                WakeState::Disabled
            } else {
                WakeState::Suspended
            };
            assert_eq!(f.wake.state, target);
            assert!(f.wake.arming.is_none() && f.wake.callback.is_none());
            assert_eq!(f.wake.retained_duration, Duration::ZERO);
            assert_eq!(f.advance(), Err(Error::InvalidTransition));
            if let Some(callback) = old {
                assert_eq!(
                    f.wake
                        .handle(&callback, Event::Ready, &f.facts(), &f.clock.now()),
                    Err(Error::Stale)
                );
            }
            if barrier == Barrier::Quit {
                assert_eq!(
                    f.wake.arm(2, &f.facts(), &f.clock.now()),
                    Err(Error::Closed)
                );
            } else {
                assert_eq!(f.wake.arm(1, &f.facts(), &f.clock.now()), Err(Error::Stale));
                let mut not_fresh = f.facts();
                not_fresh.fresh_activation = false;
                assert_eq!(
                    f.wake.arm(2, &not_fresh, &f.clock.now()),
                    Err(Error::Unauthorized)
                );
                f.wake.arm(2, &f.facts(), &f.clock.now()).unwrap();
                assert_eq!(f.wake.state, WakeState::Preparing);
            }
        }
    }
}

#[test]
fn runtime_permission_privacy_consent_and_model_loss_fail_closed() {
    for loss in 0..6 {
        let mut f = Fixture::new();
        f.armed();
        let callback = f.wake.callback.clone().unwrap();
        let mut facts = f.facts();
        match loss {
            0 => facts.consent_valid = false,
            1 => facts.privacy_clear = false,
            2 => facts.microphone_permission = false,
            3 => {
                facts.foreground = false;
                facts.background_allowed = false;
            }
            4 => facts.model_integrity_valid = false,
            _ => facts.model_license_eligible = false,
        }
        let expected = if loss >= 4 {
            Error::ModelIneligible
        } else {
            Error::Unauthorized
        };
        assert_eq!(
            f.wake.handle(
                &callback,
                Event::Detected {
                    retained: WAKE_RING_BUFFER
                },
                &facts,
                &f.clock.now()
            ),
            Err(expected)
        );
        assert_eq!(
            f.wake.state,
            if loss >= 4 {
                WakeState::Faulted
            } else {
                WakeState::Suspended
            }
        );
        assert_eq!(f.advance(), Err(Error::InvalidTransition));
        assert!(f.wake.arming.is_none() && f.wake.callback.is_none());
    }
}

#[test]
fn detector_failure_is_terminal_until_new_explicit_activation() {
    for state in [WakeState::Preparing, WakeState::Armed] {
        let mut f = Fixture::at(state);
        let old = f.wake.callback.clone().unwrap();
        assert_eq!(f.event(Event::Failed), Err(Error::DetectorFailed));
        assert_eq!(f.wake.state, WakeState::Faulted);
        assert_eq!(
            f.wake
                .handle(&old, Event::Ready, &f.facts(), &f.clock.now()),
            Err(Error::Stale)
        );
        assert_eq!(f.advance(), Err(Error::InvalidTransition));
        f.wake.arm(2, &f.facts(), &f.clock.now()).unwrap();
        assert_eq!(f.wake.state, WakeState::Preparing);
    }
}

#[test]
fn clock_domain_and_rollback_failures_cannot_reactivate_a_detector() {
    for foreign in [false, true] {
        let mut f = Fixture::new();
        f.armed();
        let earlier = f.clock.now();
        f.clock.advance(Duration::from_secs(1)).unwrap();
        f.advance().unwrap();
        let callback = f.wake.callback.clone().unwrap();
        let now = if foreign {
            ManualClock::new().now()
        } else {
            earlier
        };
        let error = if foreign {
            ContractError::ClockDomainMismatch
        } else {
            ContractError::TimeReversed
        };
        assert_eq!(
            f.wake.handle(
                &callback,
                Event::Detected {
                    retained: WAKE_RING_BUFFER
                },
                &f.facts(),
                &now
            ),
            Err(error.into())
        );
        assert_eq!(f.wake.state, WakeState::Faulted);
        assert!(f.wake.arming.is_none() && f.wake.callback.is_none());
        f.wake.interrupt(Barrier::StopListening).unwrap();
        assert_eq!(f.wake.state, WakeState::Disabled);
    }
    let mut f = Fixture::new();
    let before = f.snapshot();
    assert_eq!(
        f.wake.arm(1, &f.facts(), &ManualClock::new().now()),
        Err(ContractError::ClockDomainMismatch.into())
    );
    assert_eq!(f.snapshot(), before);
}

#[test]
fn foreign_grant_domains_are_validated_even_when_original_arming_expired() {
    for expired in [false, true] {
        let mut f = Fixture::new();
        f.armed();
        if expired {
            f.clock.advance(MAX_WAKE_ARMING).unwrap();
        }
        let mut facts = f.facts();
        facts.grant_deadline = Deadline::after(ManualClock::new().now(), MAX_WAKE_ARMING).unwrap();
        assert_eq!(
            f.wake.advance(&facts, &f.clock.now()),
            Err(ContractError::ClockDomainMismatch.into())
        );
        assert_eq!(f.wake.state, WakeState::Faulted);
    }
    let mut f = Fixture::new();
    let mut facts = f.facts();
    facts.grant_deadline = Deadline::after(ManualClock::new().now(), MAX_WAKE_ARMING).unwrap();
    assert_eq!(
        f.wake.arm(1, &facts, &f.clock.now()),
        Err(ContractError::ClockDomainMismatch.into())
    );
    assert_eq!(f.wake.state, WakeState::Disabled);
}

#[test]
fn generation_overflow_revokes_callbacks_and_never_emits_attention() {
    for detection in [false, true] {
        let mut f = Fixture::new();
        f.wake.generation = Generation::from_native_sequence(u64::MAX - 1).unwrap();
        f.arm();
        if detection {
            f.event(Event::Ready).unwrap();
        }
        let result = if detection {
            f.event(Event::Detected {
                retained: WAKE_RING_BUFFER,
            })
            .map(|_| ())
        } else {
            f.wake.interrupt(Barrier::StopListening)
        };
        assert_eq!(result, Err(ContractError::Overflow.into()));
        assert!(f.wake.retired && f.wake.arming.is_none() && f.wake.callback.is_none());
        assert_eq!(f.wake.state, WakeState::Faulted);
        f.wake.interrupt(Barrier::StopListening).unwrap();
        assert_eq!(
            f.wake.arm(2, &f.facts(), &f.clock.now()),
            Err(Error::Closed)
        );
    }
    let mut f = Fixture::new();
    f.wake.generation = Generation::from_native_sequence(u64::MAX).unwrap();
    assert_eq!(
        f.wake.arm(1, &f.facts(), &f.clock.now()),
        Err(ContractError::Overflow.into())
    );
    assert!(f.wake.retired && f.wake.arming.is_none());
}

#[test]
fn detector_operation_overflow_fails_closed_during_all_install_paths() {
    for stage in 0..4 {
        let mut f = Fixture::new();
        if stage > 0 {
            f.arm();
        }
        if stage > 1 {
            f.event(Event::Ready).unwrap();
        }
        if stage == 3 {
            let mut busy = f.facts();
            busy.speech_active = true;
            assert_eq!(
                f.wake.advance(&busy, &f.clock.now()),
                Err(Error::Suppressed)
            );
        }
        f.wake.operation_sequence = u64::MAX;
        let result = match stage {
            0 => f.wake.arm(1, &f.facts(), &f.clock.now()),
            1 => f.event(Event::Ready).map(|_| ()),
            2 => f
                .event(Event::Window {
                    retained: WAKE_RING_BUFFER,
                })
                .map(|_| ()),
            _ => f.advance(),
        };
        assert_eq!(result, Err(ContractError::Overflow.into()));
        assert_eq!(f.wake.state, WakeState::Faulted);
        assert!(f.wake.retired && f.wake.arming.is_none() && f.wake.callback.is_none());
    }
}

#[test]
fn arming_and_cooldown_deadline_arithmetic_overflow_fail_closed() {
    let mut f = Fixture::new();
    f.clock
        .advance(Duration::MAX - MAX_WAKE_ARMING + Duration::from_secs(1))
        .unwrap();
    f.grant = Deadline::after(f.clock.now(), Duration::from_secs(1)).unwrap();
    assert_eq!(
        f.wake.arm(1, &f.facts(), &f.clock.now()),
        Err(ContractError::Overflow.into())
    );
    assert_eq!(f.wake.state, WakeState::Faulted);
    assert!(f.wake.arming.is_none());
    let mut f = Fixture::new();
    f.clock
        .advance(Duration::MAX - MAX_WAKE_ARMING - Duration::from_secs(1))
        .unwrap();
    f.grant = Deadline::after(f.clock.now(), MAX_WAKE_ARMING + Duration::from_secs(1)).unwrap();
    f.armed();
    f.clock
        .advance(MAX_WAKE_ARMING - Duration::from_secs(1))
        .unwrap();
    assert_eq!(
        f.event(Event::Detected {
            retained: WAKE_RING_BUFFER
        }),
        Err(ContractError::Overflow.into())
    );
    assert_eq!(f.wake.state, WakeState::Faulted);
    assert!(f.wake.callback.is_none() && f.wake.arming.is_none());
}

#[test]
fn repeated_stop_is_clock_independent_in_every_state() {
    for state in STATES {
        let mut f = Fixture::at(state);
        f.wake.interrupt(Barrier::StopListening).unwrap();
        let before = f.snapshot();
        for _ in 0..3 {
            f.wake.interrupt(Barrier::StopListening).unwrap();
            assert_eq!(f.snapshot(), before);
        }
        assert_eq!(f.wake.state, WakeState::Disabled);
    }
    let mut f = Fixture::new();
    f.wake.interrupt(Barrier::Quit).unwrap();
    assert_eq!(
        f.wake.arm(1, &f.facts(), &f.clock.now()),
        Err(Error::Closed)
    );
}

#[test]
fn foreground_manual_scope_never_becomes_background_wake_authority() {
    let mut f = Fixture::new();
    let mut facts = f.facts();
    facts.microphone_owner = MicrophoneOwner::ManualForeground;
    facts.foreground = false;
    facts.background_allowed = false;
    assert_eq!(
        f.wake.arm(1, &facts, &f.clock.now()),
        Err(Error::Unauthorized)
    );
    facts.background_allowed = true;
    f.wake.arm(1, &facts, &f.clock.now()).unwrap();
    assert_eq!(f.wake.state, WakeState::Suppressed);
    assert_eq!(facts.microphone_owner, MicrophoneOwner::ManualForeground);
    assert!(f.wake.callback.is_none());
}

#[test]
fn attention_metadata_cannot_replace_independent_native_authorization() {
    let mut f = Fixture::new();
    f.armed();
    let callback = f.wake.callback.clone().unwrap();
    let candidate = f
        .event(Event::Detected {
            retained: WAKE_RING_BUFFER,
        })
        .unwrap()
        .unwrap();
    // Candidate carries descriptive IDs only, not native callback nonce/instance,
    // audio, a provider job, a consent receipt or a microphone lease.
    let forged = Callback {
        identity: candidate.identity,
        generation: candidate.generation,
        operation: candidate.detection,
        native_instance: Arc::new(()),
        model_instance: f.model.clone(),
        nonce: Arc::new(()),
        stage: WakeState::Armed,
    };
    assert_eq!(
        f.wake.handle(
            &forged,
            Event::Detected {
                retained: WAKE_RING_BUFFER
            },
            &f.facts(),
            &f.clock.now()
        ),
        Err(Error::Stale)
    );
    f.wake.interrupt(Barrier::StopListening).unwrap();
    let mut facts = f.facts();
    facts.identity = candidate.identity;
    facts.fresh_activation = false;
    assert_eq!(
        f.wake.arm(2, &facts, &f.clock.now()),
        Err(Error::Unauthorized)
    );
    facts.fresh_activation = true;
    facts.consent_valid = false;
    assert_eq!(
        f.wake.arm(2, &facts, &f.clock.now()),
        Err(Error::Unauthorized)
    );
    facts.consent_valid = true;
    facts.microphone_permission = false;
    assert_eq!(
        f.wake.arm(2, &facts, &f.clock.now()),
        Err(Error::Unauthorized)
    );
    assert_eq!(
        f.wake
            .handle(&callback, Event::Ready, &f.facts(), &f.clock.now()),
        Err(Error::Stale)
    );
    assert_eq!(f.wake.state, WakeState::Disabled);
}
