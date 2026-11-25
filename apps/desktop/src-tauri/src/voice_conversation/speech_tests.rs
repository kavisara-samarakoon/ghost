//! Synthetic text, events and clocks only. No hardware or real consent evidence.
use super::*;
use crate::voice_conversation::clock::{ManualClock, MonotonicClock};
use crate::voice_conversation::limits::{KIB, MIB};
use std::time::Duration;

fn identity(sequence: u64) -> ContractIdentity {
    ContractIdentity {
        version: ContractVersion::V1,
        app_run_id: AppRunId::from_native_sequence(1).unwrap(),
        interaction_id: InteractionId::from_native_sequence(1).unwrap(),
        session_id: SessionId::from_native_sequence(1).unwrap(),
        turn_id: TurnId::from_native_sequence(sequence).unwrap(),
        job_id: JobId::from_native_sequence(sequence).unwrap(),
        generation: Generation::from_native_sequence(1).unwrap(),
        configuration_revision: ConfigurationRevision::from_native_sequence(1).unwrap(),
        consent_binding_id: ConsentBindingId::from_native_sequence(1).unwrap(),
    }
}

struct Fixture {
    clock: ManualClock,
    speech: SpeechController,
}
impl Fixture {
    fn new() -> Self {
        let clock = ManualClock::new();
        let id = identity(1);
        let speech = SpeechController::new(id.app_run_id, id.interaction_id, clock.now()).unwrap();
        Self { clock, speech }
    }
    fn validation(&self, sequence: u64) -> Validation {
        Validation {
            identity: identity(sequence),
            role: ProviderRole::Tts,
            deadline: Deadline::after(self.clock.now(), Duration::from_secs(900)).unwrap(),
            source_eligible: true,
            speaker_allowed: true,
            cloud_tts_allowed: true,
        }
    }
    fn enqueue(&mut self, sequence: u64, preparation: Preparation, text: &str) -> Result<()> {
        self.speech.enqueue(
            sequence,
            text.into(),
            preparation,
            &self.validation(sequence),
            &self.clock.now(),
        )
    }
    fn event(&mut self, event: Event) -> Result<Update> {
        let callback = self.speech.callback().unwrap();
        let mut validation = self.validation(1);
        validation.identity = callback.identity;
        self.speech
            .handle(&callback, event, &validation, &self.clock.now())
    }
    fn speaking(&mut self, preparation: Preparation) {
        self.enqueue(1, preparation, "synthetic reply").unwrap();
        self.event(Event::Prepared).unwrap();
        if preparation == Preparation::Cloud {
            self.event(Event::AudioReady { bytes: 1 }).unwrap();
        }
        assert_eq!(self.speech.state(), SpeechState::Speaking);
    }
    fn snapshot(&self) -> String {
        let summarize = |job: &Job| {
            (
                job.callback.identity,
                job.callback.epoch,
                job.callback.operation,
                job.callback.stage,
                job.slot,
                job.text.len(),
            )
        };
        format!(
            "{:?}/{:?}/{}/{}/{}/{:?}/{:?}/{:?}/{:?}",
            self.speech.state(),
            self.speech.epoch,
            self.speech.operation,
            self.speech.last_job_sequence,
            self.speech.closed,
            self.speech.capacity.usage(),
            self.speech.active.as_ref().map(summarize),
            self.speech.pending.as_ref().map(summarize),
            self.clock
                .now()
                .checked_duration_since(&self.speech.last_time)
                .unwrap()
        )
    }
}

#[test]
fn normal_local_and_cloud_lifecycles_return_only_completion_candidates() {
    for preparation in [Preparation::Local, Preparation::Cloud] {
        let mut f = Fixture::new();
        assert_eq!(f.speech.state(), SpeechState::Idle);
        f.enqueue(1, preparation, "synthetic reply").unwrap();
        assert_eq!(f.speech.state(), SpeechState::Preparing);
        assert!(f
            .event(Event::Prepared)
            .unwrap()
            .iter()
            .all(Option::is_none));
        if preparation == Preparation::Cloud {
            assert_eq!(f.speech.state(), SpeechState::Fetching);
            f.event(Event::AudioReady { bytes: 1 }).unwrap();
        }
        assert_eq!(f.speech.state(), SpeechState::Speaking);
        let update = f.event(Event::Completed).unwrap();
        let finished = update[0].as_ref().unwrap();
        assert_eq!(finished.outcome, SpeechOutcome::NormalCompletion);
        assert_eq!(finished.identity, identity(1));
        assert_eq!(finished.text, "synthetic reply");
        assert!(finished.follow_up_candidate); // never a capture instruction/grant
        assert_eq!(f.speech.state(), SpeechState::Idle);
        assert_eq!(f.speech.capacity.usage(), (0, 0, 0));
        assert!(f.speech.callback().is_none());
    }
}

#[test]
fn queue_full_duplicate_and_replayed_jobs_are_atomic() {
    let mut f = Fixture::new();
    f.speaking(Preparation::Local);
    f.enqueue(2, Preparation::Cloud, "pending").unwrap();
    let before = f.snapshot();
    assert_eq!(
        f.enqueue(3, Preparation::Local, "third"),
        Err(Error::QueueFull)
    );
    assert_eq!(
        f.enqueue(1, Preparation::Local, "duplicate active"),
        Err(ContractError::DuplicateJob.into())
    );
    assert_eq!(
        f.enqueue(2, Preparation::Cloud, "duplicate pending"),
        Err(ContractError::DuplicateJob.into())
    );
    assert_eq!(f.snapshot(), before);
    let update = f.event(Event::Completed).unwrap();
    assert!(!update[0].as_ref().unwrap().follow_up_candidate);
    assert_eq!(f.speech.state(), SpeechState::Preparing);
    assert_eq!(f.speech.callback().unwrap().identity, identity(2));
    f.enqueue(3, Preparation::Local, "third now fits").unwrap();
    assert_eq!(
        f.enqueue(1, Preparation::Local, "completed replay"),
        Err(ContractError::DuplicateJob.into())
    );
}

#[test]
fn repeated_pending_promotion_preserves_exact_capacity_leases() {
    let mut f = Fixture::new();
    f.speaking(Preparation::Local);
    for sequence in 2..10 {
        f.enqueue(sequence, Preparation::Local, "next").unwrap();
        let old = f.speech.callback().unwrap();
        let pending_operation = f.speech.pending.as_ref().unwrap().callback.operation;
        f.event(Event::Completed).unwrap();
        assert_eq!(f.speech.callback().unwrap().operation, pending_operation);
        assert_eq!(f.speech.capacity.usage().2, 4);
        let before = f.snapshot();
        let mut facts = f.validation(1);
        facts.identity = old.identity;
        assert!(matches!(
            f.speech
                .handle(&old, Event::Completed, &facts, &f.clock.now()),
            Err(Error::StaleCallback)
        ));
        assert_eq!(f.snapshot(), before);
        f.event(Event::Prepared).unwrap();
    }
    f.event(Event::Completed).unwrap();
    assert_eq!(f.speech.capacity.usage(), (0, 0, 0));
}

#[test]
fn pending_callbacks_cannot_change_active_work() {
    let mut f = Fixture::new();
    f.speaking(Preparation::Local);
    f.enqueue(2, Preparation::Cloud, "pending").unwrap();
    let pending = f.speech.pending.as_ref().unwrap().callback.clone();
    let before = f.snapshot();
    assert!(matches!(
        f.speech
            .handle(&pending, Event::Prepared, &f.validation(2), &f.clock.now()),
        Err(Error::StaleCallback)
    ));
    assert_eq!(f.snapshot(), before);
}

#[test]
fn stop_in_idle_preparing_fetching_and_speaking_clears_both_slots() {
    for stage in [
        SpeechState::Idle,
        SpeechState::Preparing,
        SpeechState::Fetching,
        SpeechState::Speaking,
    ] {
        let mut f = Fixture::new();
        if stage != SpeechState::Idle {
            f.enqueue(1, Preparation::Cloud, "active").unwrap();
            if stage != SpeechState::Preparing {
                f.event(Event::Prepared).unwrap();
            }
            if stage == SpeechState::Speaking {
                f.event(Event::AudioReady { bytes: 1 }).unwrap();
            }
            f.enqueue(2, Preparation::Local, "pending").unwrap();
        }
        assert_eq!(f.speech.state(), stage);
        let old = f.speech.callback();
        let epoch = f.speech.epoch;
        assert_eq!(f.speech.stop(), Ok(SpeechOutcome::Stop));
        assert_ne!(f.speech.epoch, epoch);
        assert_eq!(f.speech.state(), SpeechState::Idle);
        assert_eq!(f.speech.capacity.usage(), (0, 0, 0));
        assert!(f.speech.pending.is_none());
        if let Some(old) = old {
            f.enqueue(3, Preparation::Local, "new work").unwrap();
            let before = f.snapshot();
            for event in [
                Event::Prepared,
                Event::AudioReady { bytes: 1 },
                Event::Completed,
                Event::Failed,
            ] {
                assert!(matches!(
                    f.speech
                        .handle(&old, event, &f.validation(1), &f.clock.now()),
                    Err(Error::StaleCallback)
                ));
                assert_eq!(f.snapshot(), before);
            }
        }
    }
}

#[test]
fn stop_is_independent_of_invalid_clocks_and_exhausted_generation() {
    let mut f = Fixture::new();
    f.speaking(Preparation::Cloud);
    f.enqueue(2, Preparation::Local, "pending").unwrap();
    let old = f.speech.callback().unwrap();
    f.speech.epoch = Generation::from_native_sequence(u64::MAX).unwrap();
    assert_eq!(f.speech.stop(), Err(ContractError::Overflow.into()));
    assert_eq!(f.speech.state(), SpeechState::Idle);
    assert_eq!(f.speech.capacity.usage(), (0, 0, 0));
    assert!(f.speech.closed);
    assert_eq!(
        f.enqueue(3, Preparation::Local, "blocked"),
        Err(Error::Closed)
    );
    assert!(matches!(
        f.speech.handle(
            &old,
            Event::Completed,
            &f.validation(1),
            &ManualClock::new().now()
        ),
        Err(Error::StaleCallback)
    ));
    assert_eq!(f.speech.stop(), Err(ContractError::Overflow.into()));
}

#[test]
fn failures_in_every_live_stage_retain_text_and_never_retry_or_promote() {
    for stage in [
        SpeechState::Preparing,
        SpeechState::Fetching,
        SpeechState::Speaking,
    ] {
        let mut f = Fixture::new();
        f.enqueue(1, Preparation::Cloud, "retained active").unwrap();
        if stage != SpeechState::Preparing {
            f.event(Event::Prepared).unwrap();
        }
        if stage == SpeechState::Speaking {
            f.event(Event::AudioReady { bytes: 1 }).unwrap();
        }
        f.enqueue(2, Preparation::Local, "retained pending")
            .unwrap();
        let update = f.event(Event::Failed).unwrap();
        for (index, text) in ["retained active", "retained pending"]
            .into_iter()
            .enumerate()
        {
            let finished = update[index].as_ref().unwrap();
            assert_eq!(finished.outcome, SpeechOutcome::Failure);
            assert_eq!(finished.text, text);
            assert!(!finished.follow_up_candidate);
        }
        assert_eq!(f.speech.state(), SpeechState::Idle);
        assert_eq!(f.speech.capacity.usage(), (0, 0, 0));
        assert!(f
            .speech
            .tick(&f.clock.now())
            .unwrap()
            .iter()
            .all(Option::is_none));
        assert_eq!(
            f.enqueue(1, Preparation::Cloud, "no retry"),
            Err(ContractError::DuplicateJob.into())
        );
    }
}

#[test]
fn duplicate_stage_callbacks_are_inert() {
    let mut f = Fixture::new();
    f.enqueue(1, Preparation::Cloud, "text").unwrap();
    for event in [
        Event::Prepared,
        Event::AudioReady { bytes: 1 },
        Event::Completed,
    ] {
        let old = f.speech.callback().unwrap();
        f.event(event).unwrap();
        let before = f.snapshot();
        assert!(matches!(
            f.speech
                .handle(&old, event, &f.validation(1), &f.clock.now()),
            Err(Error::StaleCallback)
        ));
        assert_eq!(f.snapshot(), before);
    }
}

#[test]
fn all_foreign_identity_fields_generations_stages_and_native_owners_reject() {
    let mut f = Fixture::new();
    f.speaking(Preparation::Local);
    let original = f.speech.callback().unwrap();
    let mut variants = Vec::new();
    macro_rules! foreign {
        ($field:ident, $value:expr) => {{
            let mut callback = original.clone();
            callback.identity.$field = $value;
            variants.push(callback);
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
    let mut callback = original.clone();
    callback.epoch = Generation::from_native_sequence(2).unwrap();
    variants.push(callback);
    let mut callback = original.clone();
    callback.operation += 1;
    variants.push(callback);
    let mut callback = original.clone();
    callback.stage = SpeechState::Preparing;
    variants.push(callback);
    let mut callback = original.clone();
    callback.native_owner = Arc::new(());
    variants.push(callback);
    let before = f.snapshot();
    for callback in variants {
        let mut facts = f.validation(1);
        facts.identity = callback.identity;
        assert!(matches!(
            f.speech
                .handle(&callback, Event::Completed, &facts, &f.clock.now()),
            Err(Error::StaleCallback)
        ));
        assert_eq!(f.snapshot(), before);
    }
    let mut other = Fixture::new();
    other.speaking(Preparation::Local);
    assert!(matches!(
        f.speech.handle(
            &other.speech.callback().unwrap(),
            Event::Completed,
            &f.validation(1),
            &f.clock.now()
        ),
        Err(Error::StaleCallback)
    ));
    assert_eq!(f.snapshot(), before);
}

#[test]
fn invalid_event_matrix_never_advances_state() {
    for preparation in [Preparation::Local, Preparation::Cloud] {
        for stage in [
            SpeechState::Preparing,
            SpeechState::Fetching,
            SpeechState::Speaking,
        ] {
            if preparation == Preparation::Local && stage == SpeechState::Fetching {
                continue;
            }
            let mut f = Fixture::new();
            f.enqueue(1, preparation, "text").unwrap();
            if stage != SpeechState::Preparing {
                f.event(Event::Prepared).unwrap();
            }
            if stage == SpeechState::Speaking && preparation == Preparation::Cloud {
                f.event(Event::AudioReady { bytes: 1 }).unwrap();
            }
            for event in [
                Event::Prepared,
                Event::AudioReady { bytes: 1 },
                Event::Completed,
            ] {
                let valid = matches!(
                    (stage, preparation, event),
                    (SpeechState::Preparing, _, Event::Prepared)
                        | (
                            SpeechState::Fetching,
                            Preparation::Cloud,
                            Event::AudioReady { .. }
                        )
                        | (SpeechState::Speaking, _, Event::Completed)
                );
                if !valid {
                    let before = f.snapshot();
                    assert!(matches!(f.event(event), Err(Error::InvalidTransition)));
                    assert_eq!(f.snapshot(), before);
                }
            }
        }
    }
}

#[test]
fn utf8_text_and_total_queue_limits_are_exact() {
    let mut f = Fixture::new();
    let text = "é".repeat((2 * KIB) as usize);
    f.enqueue(1, Preparation::Local, &text).unwrap();
    f.enqueue(2, Preparation::Cloud, &text).unwrap();
    assert_eq!(f.speech.capacity.usage(), (1, 1, 8 * KIB));
    for text in [
        "x".repeat((4 * KIB + 1) as usize),
        "é".repeat((2 * KIB + 1) as usize),
    ] {
        let mut f = Fixture::new();
        let before = f.snapshot();
        assert_eq!(
            f.enqueue(1, Preparation::Local, &text),
            Err(ContractError::LimitExceeded.into())
        );
        assert_eq!(f.snapshot(), before);
    }
    let mut f = Fixture::new();
    assert_eq!(
        f.enqueue(1, Preparation::Local, ""),
        Err(Error::Unauthorized)
    );
}

#[test]
fn audio_bounds_reject_empty_oversized_and_overflowing_results() {
    for bytes in [0, 1, 8 * MIB, 8 * MIB + 1, u64::MAX] {
        let mut f = Fixture::new();
        f.enqueue(1, Preparation::Cloud, "retained").unwrap();
        f.event(Event::Prepared).unwrap();
        let update = f.event(Event::AudioReady { bytes }).unwrap();
        if (1..=8 * MIB).contains(&bytes) {
            assert!(update.iter().all(Option::is_none));
            assert_eq!(f.speech.state(), SpeechState::Speaking);
        } else {
            assert_eq!(f.speech.state(), SpeechState::Idle);
            let failed = update[0].as_ref().unwrap();
            assert_eq!(failed.text, "retained");
            assert_eq!(failed.outcome, SpeechOutcome::Failure);
            assert!(!failed.follow_up_candidate);
        }
    }
}

#[test]
fn ninety_second_deadline_is_absolute_and_equality_expires() {
    for elapsed in [
        SPEECH_OUTPUT_DEADLINE - Duration::from_nanos(1),
        SPEECH_OUTPUT_DEADLINE,
        SPEECH_OUTPUT_DEADLINE + Duration::from_nanos(1),
    ] {
        for use_callback in [false, true] {
            let mut f = Fixture::new();
            f.speaking(Preparation::Cloud);
            f.clock.advance(elapsed).unwrap();
            let update = if use_callback {
                f.event(Event::Completed).unwrap()
            } else {
                f.speech.tick(&f.clock.now()).unwrap()
            };
            if elapsed < SPEECH_OUTPUT_DEADLINE {
                assert_eq!(
                    f.speech.state(),
                    if use_callback {
                        SpeechState::Idle
                    } else {
                        SpeechState::Speaking
                    }
                );
                if use_callback {
                    assert!(update[0].as_ref().unwrap().follow_up_candidate);
                }
            } else {
                assert_eq!(f.speech.state(), SpeechState::Idle);
                assert_eq!(update[0].as_ref().unwrap().outcome, SpeechOutcome::Failure);
                assert!(!update[0].as_ref().unwrap().follow_up_candidate);
            }
        }
    }
}

#[test]
fn queue_wait_and_stage_changes_never_renew_output_deadlines() {
    let mut f = Fixture::new();
    f.speaking(Preparation::Local);
    f.clock.advance(Duration::from_secs(10)).unwrap();
    f.enqueue(2, Preparation::Cloud, "pending").unwrap();
    f.clock.advance(Duration::from_secs(79)).unwrap();
    f.event(Event::Completed).unwrap();
    assert_eq!(f.speech.state(), SpeechState::Preparing);
    f.event(Event::Prepared).unwrap();
    f.clock.advance(Duration::from_secs(10)).unwrap();
    f.event(Event::AudioReady { bytes: 1 }).unwrap();
    f.clock.advance(Duration::from_secs(1)).unwrap();
    let update = f.event(Event::Completed).unwrap();
    assert_eq!(update[0].as_ref().unwrap().outcome, SpeechOutcome::Failure);
    assert!(!update[0].as_ref().unwrap().follow_up_candidate);
}

#[test]
fn expired_pending_job_is_not_promoted_and_can_expire_independently() {
    for complete in [false, true] {
        let mut f = Fixture::new();
        f.speaking(Preparation::Local);
        let mut facts = f.validation(2);
        facts.deadline = Deadline::after(f.clock.now(), Duration::from_secs(1)).unwrap();
        f.speech
            .enqueue(
                2,
                "pending text".into(),
                Preparation::Cloud,
                &facts,
                &f.clock.now(),
            )
            .unwrap();
        f.clock.advance(Duration::from_secs(1)).unwrap();
        let update = if complete {
            f.event(Event::Completed).unwrap()
        } else {
            f.speech.tick(&f.clock.now()).unwrap()
        };
        assert_eq!(update[1].as_ref().unwrap().outcome, SpeechOutcome::Failure);
        assert_eq!(update[1].as_ref().unwrap().text, "pending text");
        assert!(!update[1].as_ref().unwrap().follow_up_candidate);
        assert!(f.speech.pending.is_none());
        if complete {
            assert!(!update[0].as_ref().unwrap().follow_up_candidate);
        }
        assert_eq!(
            f.speech.state(),
            if complete {
                SpeechState::Idle
            } else {
                SpeechState::Speaking
            }
        );
    }
}

#[test]
fn denied_or_expired_preparation_never_allocates_capacity() {
    for case in 0..8 {
        let mut f = Fixture::new();
        let mut facts = f.validation(1);
        match case {
            0 => facts.speaker_allowed = false,
            1 => facts.source_eligible = false,
            2 => facts.cloud_tts_allowed = false,
            3 => facts.role = ProviderRole::Conversation,
            4 => facts.role = ProviderRole::Planning,
            5 => facts.role = ProviderRole::Stt,
            6 => facts.deadline = Deadline::after(f.clock.now(), Duration::ZERO).unwrap(),
            _ => facts.identity.job_id = JobId::from_native_sequence(99).unwrap(),
        }
        let before = f.snapshot();
        assert_eq!(
            f.speech
                .enqueue(1, "text".into(), Preparation::Cloud, &facts, &f.clock.now()),
            Err(Error::Unauthorized)
        );
        assert_eq!(f.snapshot(), before);
    }
    let mut f = Fixture::new();
    let mut facts = f.validation(1);
    facts.cloud_tts_allowed = false;
    f.speech
        .enqueue(
            1,
            "local only".into(),
            Preparation::Local,
            &facts,
            &f.clock.now(),
        )
        .unwrap();
}

#[test]
fn revoked_scope_and_original_grant_expiry_cannot_produce_followup() {
    for case in 0..6 {
        let mut f = Fixture::new();
        let mut facts = f.validation(1);
        facts.deadline = Deadline::after(f.clock.now(), Duration::from_secs(1)).unwrap();
        f.speech
            .enqueue(
                1,
                "reply".into(),
                Preparation::Cloud,
                &facts,
                &f.clock.now(),
            )
            .unwrap();
        f.event(Event::Prepared).unwrap();
        f.event(Event::AudioReady { bytes: 1 }).unwrap();
        let mut fresh = f.validation(1);
        match case {
            0 => fresh.speaker_allowed = false,
            1 => fresh.source_eligible = false,
            2 => fresh.cloud_tts_allowed = false,
            3 => fresh.role = ProviderRole::Planning,
            4 => fresh.deadline = Deadline::after(f.clock.now(), Duration::ZERO).unwrap(),
            _ => f.clock.advance(Duration::from_secs(1)).unwrap(),
        }
        let update = f
            .speech
            .handle(
                &f.speech.callback().unwrap(),
                Event::Completed,
                &fresh,
                &f.clock.now(),
            )
            .unwrap();
        assert_eq!(update[0].as_ref().unwrap().outcome, SpeechOutcome::Failure);
        assert!(!update[0].as_ref().unwrap().follow_up_candidate);
        assert_eq!(f.speech.state(), SpeechState::Idle);
    }
}

#[test]
fn foreign_and_reversed_clocks_are_rejected_atomically_but_stop_works() {
    let mut f = Fixture::new();
    f.speaking(Preparation::Local);
    let earlier = f.clock.now();
    f.clock.advance(Duration::from_secs(1)).unwrap();
    f.speech.tick(&f.clock.now()).unwrap();
    let callback = f.speech.callback().unwrap();
    let before = f.snapshot();
    for (now, error) in [
        (earlier, ContractError::TimeReversed),
        (ManualClock::new().now(), ContractError::ClockDomainMismatch),
    ] {
        assert!(
            matches!(f.speech.handle(&callback, Event::Completed, &f.validation(1), &now), Err(Error::Contract(e)) if e == error)
        );
        assert!(matches!(f.speech.tick(&now), Err(Error::Contract(e)) if e == error));
        assert_eq!(
            f.speech
                .enqueue(2, "new".into(), Preparation::Local, &f.validation(2), &now),
            Err(error.into())
        );
        assert_eq!(f.snapshot(), before);
    }
    assert_eq!(f.speech.stop(), Ok(SpeechOutcome::Stop));
}

#[test]
fn foreign_grant_domains_are_rejected_without_state_or_capacity_change() {
    let mut f = Fixture::new();
    let mut facts = f.validation(1);
    facts.deadline = Deadline::after(ManualClock::new().now(), Duration::from_secs(1)).unwrap();
    let before = f.snapshot();
    assert_eq!(
        f.speech
            .enqueue(1, "text".into(), Preparation::Local, &facts, &f.clock.now()),
        Err(ContractError::ClockDomainMismatch.into())
    );
    assert_eq!(f.snapshot(), before);
    f.speaking(Preparation::Local);
    let before = f.snapshot();
    assert!(matches!(
        f.speech.handle(
            &f.speech.callback().unwrap(),
            Event::Completed,
            &facts,
            &f.clock.now()
        ),
        Err(Error::Contract(ContractError::ClockDomainMismatch))
    ));
    assert_eq!(f.snapshot(), before);
}

#[test]
fn operation_and_deadline_overflow_do_not_consume_ids_or_capacity() {
    let mut f = Fixture::new();
    f.speech.operation = u64::MAX;
    let before = f.snapshot();
    assert_eq!(
        f.enqueue(1, Preparation::Local, "text"),
        Err(ContractError::Overflow.into())
    );
    assert_eq!(f.snapshot(), before);
    let mut f = Fixture::new();
    let mut facts = f.validation(1);
    f.clock
        .advance(Duration::MAX - Duration::from_secs(89))
        .unwrap();
    facts.deadline = Deadline::after(f.clock.now(), Duration::from_secs(1)).unwrap();
    let before = f.snapshot();
    assert_eq!(
        f.speech
            .enqueue(1, "text".into(), Preparation::Local, &facts, &f.clock.now()),
        Err(ContractError::Overflow.into())
    );
    assert_eq!(f.snapshot(), before);
}

#[test]
fn native_sequence_gaps_are_valid_but_zero_and_reuse_are_rejected() {
    let mut f = Fixture::new();
    assert_eq!(
        f.speech.enqueue(
            0,
            "text".into(),
            Preparation::Local,
            &f.validation(1),
            &f.clock.now()
        ),
        Err(ContractError::InvalidIdentifier.into())
    );
    f.enqueue(99, Preparation::Local, "gap").unwrap();
    f.speech.stop().unwrap();
    assert_eq!(
        f.enqueue(99, Preparation::Local, "replay"),
        Err(ContractError::DuplicateJob.into())
    );
    f.enqueue(u64::MAX, Preparation::Local, "last native id")
        .unwrap();
    assert_eq!(f.speech.last_job_sequence, u64::MAX);
    f.speech.stop().unwrap();
    assert_eq!(
        f.enqueue(u64::MAX, Preparation::Local, "replay max"),
        Err(ContractError::DuplicateJob.into())
    );
}
