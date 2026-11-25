//! Synthetic inputs, scripts, outputs and native callback tickets only.
use super::*;
use crate::voice_conversation::clock::{ManualClock, MonotonicClock};

fn identity(turn: u64) -> ContractIdentity {
    ContractIdentity {
        version: ContractVersion::V1,
        app_run_id: AppRunId::from_native_sequence(1).unwrap(),
        interaction_id: InteractionId::from_native_sequence(1).unwrap(),
        session_id: SessionId::from_native_sequence(1).unwrap(),
        turn_id: TurnId::from_native_sequence(turn).unwrap(),
        job_id: JobId::from_native_sequence(turn).unwrap(),
        generation: Generation::from_native_sequence(1).unwrap(),
        configuration_revision: ConfigurationRevision::from_native_sequence(1).unwrap(),
        consent_binding_id: ConsentBindingId::from_native_sequence(1).unwrap(),
    }
}
fn target<R: Role>() -> Target {
    Target {
        provider: NonZeroU64::new(1).unwrap(),
        model: NonZeroU64::new(10).unwrap(),
        voice: if R::ROLE == ProviderRole::Tts {
            NonZeroU64::new(1)
        } else {
            None
        },
    }
}
fn script(events: Vec<Event>) -> Script {
    Script {
        events: events
            .into_iter()
            .map(|event| Scheduled {
                after: Duration::ZERO,
                event,
            })
            .collect(),
    }
}
fn response(bytes: Vec<u8>) -> Script {
    script(vec![Event::Started, Event::Chunk(bytes), Event::Finished])
}
fn transcript(text: &str) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({"version":1,"text":text})).unwrap()
}
fn reply(kind: &str, text: &str) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({"version":1,"kind":kind,"text":text})).unwrap()
}
fn proposal(steps: usize) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({"kind":"plan","summary":"Synthetic proposal only","steps":(0..steps).map(|_| serde_json::json!({"capability":"generate_next_steps_request"})).collect::<Vec<_>>()})).unwrap()
}
fn audio(payload_bytes: usize) -> Vec<u8> {
    let mut bytes = FAKE_AUDIO_HEADER.to_vec();
    bytes.resize(bytes.len() + payload_bytes, 0);
    bytes
}
trait FixtureRole: Role {
    fn input() -> Self::Input;
    fn output_bytes() -> Vec<u8>;
}
impl FixtureRole for Stt {
    fn input() -> SyntheticAudio {
        SyntheticAudio {
            zeros: vec![0; 32],
            duration: Duration::from_secs(1),
        }
    }
    fn output_bytes() -> Vec<u8> {
        transcript("synthetic transcript")
    }
}
impl FixtureRole for Conversation {
    fn input() -> SyntheticConversation {
        SyntheticConversation {
            text: "synthetic ordinary input".into(),
        }
    }
    fn output_bytes() -> Vec<u8> {
        reply("answer", "synthetic ordinary reply")
    }
}
impl FixtureRole for Planning {
    fn input() -> SyntheticPlanning {
        SyntheticPlanning {
            reviewed_text: "synthetic separately reviewed planning input".into(),
            project_alias: Some("synthetic".into()),
        }
    }
    fn output_bytes() -> Vec<u8> {
        proposal(1)
    }
}
impl FixtureRole for Tts {
    fn input() -> SyntheticSpeechText {
        SyntheticSpeechText {
            text: "synthetic reply to speak".into(),
        }
    }
    fn output_bytes() -> Vec<u8> {
        audio(32)
    }
}
struct Fixture<R: FixtureRole> {
    clock: ManualClock,
    adapter: FakeAdapter<R>,
}
impl<R: FixtureRole> Fixture<R> {
    fn new(scripts: Vec<Script>) -> Self {
        let clock = ManualClock::new();
        let adapter = FakeAdapter::new(identity(1), target::<R>(), scripts, clock.now()).unwrap();
        Self { clock, adapter }
    }
    fn success() -> Self {
        Self::new(vec![response(R::output_bytes())])
    }
    fn job(&self, turn: u64) -> fixtures::Job<R> {
        self.job_with(turn, R::input())
    }
    fn job_with(&self, turn: u64, input: R::Input) -> fixtures::Job<R> {
        fixtures::synthetic(
            identity(turn),
            target::<R>(),
            Deadline::after(self.clock.now(), Duration::from_secs(30)).unwrap(),
            input,
        )
    }
    fn start(&mut self, turn: u64) -> Operation<R> {
        self.adapter
            .submit(&self.job(turn), &self.clock.now())
            .unwrap()
    }
    fn event(&mut self, operation: &Operation<R>, index: u64) -> Result<Update<R::Output>> {
        let callback = self.adapter.callback_at(operation, index)?;
        self.adapter.deliver(&callback, &self.clock.now())
    }
    fn run(&mut self, operation: &Operation<R>) -> Result<Completion<R::Output>> {
        let len = self.adapter.active.as_ref().unwrap().script.events.len();
        for index in 0..len {
            match self.event(operation, index as u64)? {
                Update::Succeeded(done) => return Ok(done),
                Update::Ended(_) => return Err(Error::Malformed),
                _ => (),
            }
        }
        Err(Error::Malformed)
    }
}
fn error<T>(result: Result<T>) -> Error {
    match result {
        Ok(_) => panic!("unexpected acceptance"),
        Err(e) => e,
    }
}

#[test]
fn stt_lifecycle_returns_bounded_untrusted_text_with_exact_binding() {
    let mut f = Fixture::<Stt>::success();
    let operation = f.start(1);
    assert_eq!(f.adapter.ledger[0].state, State::AwaitingStart);
    assert!(matches!(f.event(&operation, 0), Ok(Update::Started)));
    assert_eq!(f.adapter.ledger[0].state, State::Receiving);
    assert!(matches!(f.event(&operation, 1), Ok(Update::Progress)));
    let Ok(Update::Succeeded(done)) = f.event(&operation, 2) else {
        panic!("missing synthetic result")
    };
    assert_eq!(done.output.text, "synthetic transcript");
    assert_eq!(done.binding, operation.binding);
    assert_eq!(f.adapter.ledger[0].state, State::Succeeded);
    assert!(f.adapter.active.is_none());
    assert_eq!(f.adapter.audio, Duration::from_secs(1));
}

#[test]
fn conversation_supports_only_the_three_approved_data_reply_kinds() {
    for (kind, expected) in [
        ("answer", ReplyKind::Answer),
        ("clarify", ReplyKind::Clarify),
        ("needs_action_review", ReplyKind::NeedsActionReview),
    ] {
        let mut f = Fixture::<Conversation>::new(vec![response(reply(kind, "synthetic reply"))]);
        let operation = f.start(1);
        let done = f.run(&operation).unwrap();
        assert_eq!(done.output.kind, expected);
        assert_eq!(done.output.text, "synthetic reply");
        assert_eq!(done.binding.role, ProviderRole::Conversation);
        assert!(f.adapter.active.is_none());
    }
}

#[test]
fn planning_reuses_the_existing_finite_validator_and_returns_a_proposal_only() {
    let mut f = Fixture::<Planning>::success();
    let operation = f.start(1);
    let done = f.run(&operation).unwrap();
    assert_eq!(done.binding, operation.binding);
    assert!(done.output.plan.kind == crate::jarvis::model::Kind::Plan);
    assert_eq!(done.output.plan.steps.len(), 1);
    assert_eq!(
        done.output.plan.steps[0].capability(),
        "generate_next_steps_request"
    );
    assert_eq!(crate::jarvis::model::CAPABILITIES.len(), 13);
    // No review receipts, native requests, execution callbacks or dispatch follow.
    assert!(f.adapter.active.is_none());
}

#[test]
fn tts_returns_synthetic_audio_data_without_speech_or_capture_transitions() {
    let mut f = Fixture::<Tts>::success();
    let operation = f.start(1);
    let done = f.run(&operation).unwrap();
    assert_eq!(done.binding, operation.binding);
    assert_eq!(done.output.zeros, vec![0; 32]);
    assert_eq!(f.adapter.ledger[0].state, State::Succeeded);
    assert!(f.adapter.active.is_none());
    assert_eq!(f.adapter.attempts, 1);
}

#[test]
fn each_port_is_separate_and_preserves_its_typed_input_output_contract() {
    fn stt_port(_: &FakeStt) {}
    fn conversation_port(_: &FakeConversation) {}
    fn planning_port(_: &FakePlanning) {}
    fn tts_port(_: &FakeTts) {}
    stt_port(&Fixture::<Stt>::success().adapter);
    conversation_port(&Fixture::<Conversation>::success().adapter);
    planning_port(&Fixture::<Planning>::success().adapter);
    tts_port(&Fixture::<Tts>::success().adapter);
    trait IfPlanning<A> {
        fn check() {}
    }
    impl<T> IfPlanning<()> for T {}
    impl<T: Role<Output = UntrustedProposal>> IfPlanning<u8> for T {}
    // This becomes ambiguous if Conversation can return a planning proposal.
    let _ = <Conversation as IfPlanning<_>>::check;
    let _ = <Stt as IfPlanning<_>>::check;
    let _ = <Tts as IfPlanning<_>>::check;
}

#[test]
fn every_identity_role_target_and_native_ticket_field_is_revalidated() {
    for field in 0..15 {
        let mut f = Fixture::<Conversation>::success();
        let operation = f.start(1);
        let mut callback = f.adapter.callback_at(&operation, 0).unwrap();
        let binding = &mut callback.operation.binding;
        match field {
            0 => binding.identity.app_run_id = AppRunId::from_native_sequence(2).unwrap(),
            1 => binding.identity.interaction_id = InteractionId::from_native_sequence(2).unwrap(),
            2 => binding.identity.session_id = SessionId::from_native_sequence(2).unwrap(),
            3 => binding.identity.turn_id = TurnId::from_native_sequence(2).unwrap(),
            4 => binding.identity.job_id = JobId::from_native_sequence(2).unwrap(),
            5 => binding.identity.generation = Generation::from_native_sequence(2).unwrap(),
            6 => {
                binding.identity.configuration_revision =
                    ConfigurationRevision::from_native_sequence(2).unwrap()
            }
            7 => {
                binding.identity.consent_binding_id =
                    ConsentBindingId::from_native_sequence(2).unwrap()
            }
            8 => binding.role = ProviderRole::Planning,
            9 => binding.target.provider = NonZeroU64::new(2).unwrap(),
            10 => binding.target.model = NonZeroU64::new(2).unwrap(),
            11 => binding.target.voice = NonZeroU64::new(2),
            12 => callback.operation.adapter = Arc::new(()),
            13 => callback.operation.nonce = Arc::new(()),
            14 => callback.operation.sequence += 1,
            _ => unreachable!(),
        }
        assert_eq!(
            error(f.adapter.deliver(&callback, &f.clock.now())),
            Error::Binding,
            "field {field}"
        );
        assert_eq!(f.adapter.ledger[0].state, State::AwaitingStart);
        assert_eq!(f.adapter.active.as_ref().unwrap().cursor, 0);
        assert_eq!(f.adapter.attempts, 1);
    }
}

#[test]
fn wrong_native_scope_provider_model_or_voice_cannot_be_submitted() {
    for field in 0..9 {
        let mut f = Fixture::<Conversation>::success();
        let mut id = identity(1);
        let mut target = target::<Conversation>();
        match field {
            0 => id.app_run_id = AppRunId::from_native_sequence(2).unwrap(),
            1 => id.interaction_id = InteractionId::from_native_sequence(2).unwrap(),
            2 => id.session_id = SessionId::from_native_sequence(2).unwrap(),
            3 => id.generation = Generation::from_native_sequence(2).unwrap(),
            4 => {
                id.configuration_revision = ConfigurationRevision::from_native_sequence(2).unwrap()
            }
            5 => id.consent_binding_id = ConsentBindingId::from_native_sequence(2).unwrap(),
            6 => target.provider = NonZeroU64::new(2).unwrap(),
            7 => target.model = NonZeroU64::new(2).unwrap(),
            8 => target.voice = NonZeroU64::new(2),
            _ => unreachable!(),
        }
        let job = fixtures::synthetic::<Conversation>(
            id,
            target,
            f.job(1).deadline().clone(),
            Conversation::input(),
        );
        assert_eq!(
            error(f.adapter.submit(&job, &f.clock.now())),
            Error::Binding
        );
        assert_eq!(f.adapter.attempts, 0);
        assert_eq!(f.adapter.scripts.len(), 1);
    }
}

#[test]
fn native_owners_prevent_foreign_callbacks_with_identical_numeric_bindings() {
    let mut a = Fixture::<Stt>::success();
    let mut b = Fixture::<Stt>::success();
    let oa = a.start(1);
    let ob = b.start(1);
    assert_eq!(oa.binding, ob.binding);
    let foreign = a.adapter.callback_at(&oa, 0).unwrap();
    assert_eq!(
        error(b.adapter.deliver(&foreign, &b.clock.now())),
        Error::Binding
    );
    assert_eq!(b.adapter.ledger[0].state, State::AwaitingStart);
    assert!(b.run(&ob).is_ok());
}

#[test]
fn duplicate_and_out_of_order_callbacks_are_inert() {
    let mut f = Fixture::<Stt>::success();
    let operation = f.start(1);
    let finish = f.adapter.callback_at(&operation, 2).unwrap();
    assert_eq!(
        error(f.adapter.deliver(&finish, &f.clock.now())),
        Error::OutOfOrder
    );
    let start = f.adapter.callback_at(&operation, 0).unwrap();
    assert!(matches!(
        f.adapter.deliver(&start, &f.clock.now()),
        Ok(Update::Started)
    ));
    assert_eq!(
        error(f.adapter.deliver(&start, &f.clock.now())),
        Error::OutOfOrder
    );
    assert_eq!(f.adapter.active.as_ref().unwrap().cursor, 1);
    f.event(&operation, 1).unwrap();
    f.event(&operation, 2).unwrap();
    assert_eq!(
        error(f.adapter.deliver(&finish, &f.clock.now())),
        Error::Stale
    );
    assert_eq!(f.adapter.attempts, 1);
}

#[test]
fn chunks_and_completion_before_start_cannot_advance_the_protocol() {
    for event in [Event::Chunk(transcript("fixture")), Event::Finished] {
        let mut f = Fixture::<Stt>::new(vec![script(vec![event, Event::Started])]);
        let operation = f.start(1);
        assert_eq!(error(f.event(&operation, 0)), Error::OutOfOrder);
        assert_eq!(f.adapter.active.as_ref().unwrap().cursor, 0);
        assert!(f.adapter.active.as_ref().unwrap().buffer.is_empty());
        assert_eq!(f.adapter.ledger[0].state, State::AwaitingStart);
    }
}

fn failure_cases<R: FixtureRole>() {
    for progress in 0..=2 {
        for event in [
            Event::Failed(Failure::Unavailable),
            Event::Failed(Failure::Rejected),
            Event::Uncertain,
        ] {
            let expected = if matches!(event, Event::Uncertain) {
                State::Uncertain
            } else {
                State::Failed
            };
            let mut events = Vec::new();
            if progress >= 1 {
                events.push(Event::Started);
            }
            if progress == 2 {
                events.push(Event::Chunk(R::output_bytes()));
            }
            events.push(event);
            let mut f = Fixture::<R>::new(vec![script(events), response(R::output_bytes())]);
            let job = f.job(1);
            let operation = f.adapter.submit(&job, &f.clock.now()).unwrap();
            for index in 0..progress {
                f.event(&operation, index).unwrap();
            }
            assert!(
                matches!(f.event(&operation, progress), Ok(Update::Ended(state)) if state == expected)
            );
            assert_eq!(f.adapter.ledger[0].state, expected);
            assert_eq!(f.adapter.attempts, 1);
            assert!(f.adapter.active.is_none());
            assert_eq!(f.adapter.scripts.len(), 1);
            assert_eq!(error(f.adapter.submit(&job, &f.clock.now())), Error::Replay);
        }
    }
}
#[test]
fn failure_and_uncertain_outcomes_never_retry_refund_or_start_a_fallback() {
    failure_cases::<Stt>();
    failure_cases::<Conversation>();
    failure_cases::<Planning>();
    failure_cases::<Tts>();
}

#[test]
fn job_turn_replay_and_job_id_reuse_cannot_allocate_another_attempt() {
    for same_job in [false, true] {
        let mut f = Fixture::<Conversation>::new(vec![response(Conversation::output_bytes()); 2]);
        let operation = f.start(1);
        f.run(&operation).unwrap();
        let mut id = identity(2);
        if same_job {
            id.job_id = identity(1).job_id;
        } else {
            id.turn_id = identity(1).turn_id;
        }
        let job = fixtures::synthetic::<Conversation>(
            id,
            target::<Conversation>(),
            f.job(2).deadline().clone(),
            Conversation::input(),
        );
        assert_eq!(error(f.adapter.submit(&job, &f.clock.now())), Error::Replay);
        assert_eq!(f.adapter.attempts, 1);
        assert_eq!(f.adapter.scripts.len(), 1);
    }
}

#[test]
fn old_callback_cannot_complete_a_new_explicit_job() {
    let mut f = Fixture::<Stt>::new(vec![response(Stt::output_bytes()); 2]);
    let first = f.start(1);
    let old = f.adapter.callback_at(&first, 2).unwrap();
    f.run(&first).unwrap();
    let second = f.start(2);
    assert_eq!(
        error(f.adapter.deliver(&old, &f.clock.now())),
        Error::Binding
    );
    assert_eq!(f.adapter.ledger[1].state, State::AwaitingStart);
    assert!(f.run(&second).is_ok());
    assert_eq!(f.adapter.attempts, 2);
}

#[test]
fn provider_switch_after_failure_is_rejected_with_no_automatic_work() {
    let mut f = Fixture::<Conversation>::new(vec![
        script(vec![Event::Failed(Failure::Unavailable)]),
        response(Conversation::output_bytes()),
    ]);
    let operation = f.start(1);
    f.event(&operation, 0).unwrap();
    let switched = Target {
        provider: NonZeroU64::new(2).unwrap(),
        ..target::<Conversation>()
    };
    let job = fixtures::synthetic::<Conversation>(
        identity(2),
        switched,
        f.job(2).deadline().clone(),
        Conversation::input(),
    );
    assert_eq!(
        error(f.adapter.submit(&job, &f.clock.now())),
        Error::Binding
    );
    assert_eq!(f.adapter.attempts, 1);
    assert!(f.adapter.active.is_none());
    assert_eq!(f.adapter.scripts.len(), 1);
}

fn cancellation_cases<R: FixtureRole>() {
    for progress in 0..=2 {
        let mut f = Fixture::<R>::success();
        let job = f.job(1);
        let operation = f.adapter.submit(&job, &f.clock.now()).unwrap();
        let late = f.adapter.callback_at(&operation, 2).unwrap();
        for index in 0..progress {
            f.event(&operation, index).unwrap();
        }
        assert_eq!(f.adapter.cancel(&operation).unwrap(), State::Canceled);
        assert_eq!(
            error(f.adapter.deliver(&late, &f.clock.now())),
            Error::Stale
        );
        assert_eq!(error(f.adapter.submit(&job, &f.clock.now())), Error::Replay);
        assert_eq!(f.adapter.attempts, 1);
        assert!(f.adapter.active.is_none());
    }
}
#[test]
fn cancellation_at_each_live_phase_rejects_late_results_for_all_roles() {
    cancellation_cases::<Stt>();
    cancellation_cases::<Conversation>();
    cancellation_cases::<Planning>();
    cancellation_cases::<Tts>();
}

#[test]
fn stop_is_clock_independent_and_permanent_even_when_counters_are_exhausted() {
    let mut f = Fixture::<Tts>::new(vec![response(Tts::output_bytes()); 2]);
    let operation = f.start(1);
    let late = f.adapter.callback_at(&operation, 2).unwrap();
    f.event(&operation, 0).unwrap();
    f.adapter.sequence = u64::MAX;
    f.adapter.stop();
    f.adapter.stop();
    assert_eq!(
        error(f.adapter.deliver(&late, &f.clock.now())),
        Error::Stopped
    );
    assert_eq!(
        error(f.adapter.submit(&f.job(2), &f.clock.now())),
        Error::Stopped
    );
    assert_eq!(f.adapter.ledger[0].state, State::Canceled);
    assert_eq!(f.adapter.attempts, 1);
    assert!(f.adapter.active.is_none() && f.adapter.scripts.is_empty());
}

#[test]
fn stalled_jobs_timeout_exactly_and_late_callbacks_do_not_restore_them() {
    let mut f = Fixture::<Conversation>::new(vec![Script {
        events: vec![Scheduled {
            after: Duration::from_secs(30),
            event: Event::Started,
        }],
    }]);
    let operation = f.start(1);
    let callback = f.adapter.callback_at(&operation, 0).unwrap();
    f.clock
        .advance(Duration::from_secs(30) - Duration::from_nanos(1))
        .unwrap();
    assert_eq!(f.adapter.tick(&f.clock.now()).unwrap(), None);
    f.clock.advance(Duration::from_nanos(1)).unwrap();
    assert_eq!(
        f.adapter.tick(&f.clock.now()).unwrap(),
        Some(State::TimedOut)
    );
    assert_eq!(
        error(f.adapter.deliver(&callback, &f.clock.now())),
        Error::Stale
    );
    assert_eq!(f.adapter.attempts, 1);
}

#[test]
fn timeout_wins_over_a_success_callback_at_the_exact_deadline() {
    let mut s = response(Stt::output_bytes());
    s.events[2].after = Duration::from_secs(30);
    let mut f = Fixture::<Stt>::new(vec![s]);
    let operation = f.start(1);
    f.event(&operation, 0).unwrap();
    f.event(&operation, 1).unwrap();
    f.clock.advance(Duration::from_secs(30)).unwrap();
    assert!(matches!(
        f.event(&operation, 2),
        Ok(Update::Ended(State::TimedOut))
    ));
    assert_eq!(f.adapter.ledger[0].state, State::TimedOut);
    assert!(f.adapter.active.is_none());
}

#[test]
fn scripted_delays_are_monotonic_and_early_callbacks_leave_the_job_pending() {
    let mut f = Fixture::<Stt>::new(vec![Script {
        events: vec![Scheduled {
            after: Duration::from_secs(2),
            event: Event::Started,
        }],
    }]);
    let operation = f.start(1);
    assert_eq!(error(f.event(&operation, 0)), Error::NotDue);
    assert_eq!(f.adapter.active.as_ref().unwrap().cursor, 0);
    f.clock.advance(Duration::from_secs(2)).unwrap();
    assert!(matches!(f.event(&operation, 0), Ok(Update::Started)));
    assert_eq!(f.adapter.tick(&f.clock.now()).unwrap(), None);
}

#[test]
fn tts_deadline_is_capped_at_ninety_seconds_without_renewal_on_progress() {
    let mut f = Fixture::<Tts>::success();
    let job = fixtures::synthetic::<Tts>(
        identity(1),
        target::<Tts>(),
        Deadline::after(f.clock.now(), MAX_CHILD_CONVERSATION).unwrap(),
        Tts::input(),
    );
    let operation = f.adapter.submit(&job, &f.clock.now()).unwrap();
    f.clock.advance(Duration::from_secs(60)).unwrap();
    f.event(&operation, 0).unwrap();
    assert_eq!(
        f.adapter
            .active
            .as_ref()
            .unwrap()
            .deadline
            .remaining(&f.clock.now())
            .unwrap(),
        Duration::from_secs(30)
    );
    f.clock.advance(Duration::from_secs(30)).unwrap();
    assert_eq!(
        f.adapter.tick(&f.clock.now()).unwrap(),
        Some(State::TimedOut)
    );
}

#[test]
fn sixteen_attempt_limit_and_missing_scripts_never_allocate_hidden_work() {
    let mut f = Fixture::<Stt>::new(vec![response(Stt::output_bytes()); 16]);
    for turn in 1..=16 {
        let operation = f.start(turn);
        f.run(&operation).unwrap();
    }
    assert_eq!(f.adapter.attempts, 16);
    assert_eq!(
        error(f.adapter.submit(&f.job(17), &f.clock.now())),
        Error::Contract(ContractError::LimitExceeded)
    );
    let mut empty = Fixture::<Stt>::new(Vec::new());
    assert_eq!(
        error(empty.adapter.submit(&empty.job(1), &empty.clock.now())),
        Error::NoScript
    );
    assert_eq!(empty.adapter.attempts, 0);
}

#[test]
fn failed_cloud_audio_attempts_keep_their_aggregate_audio_charge() {
    let mut f = Fixture::<Stt>::new(vec![script(vec![Event::Uncertain]); 5]);
    for turn in 1..=4 {
        let job = f.job_with(
            turn,
            SyntheticAudio {
                zeros: vec![0; 32],
                duration: MAX_CAPTURED_TURN,
            },
        );
        let operation = f.adapter.submit(&job, &f.clock.now()).unwrap();
        assert!(matches!(
            f.event(&operation, 0),
            Ok(Update::Ended(State::Uncertain))
        ));
    }
    assert_eq!(f.adapter.audio, MAX_CLOUD_AUDIO);
    assert_eq!(
        error(f.adapter.submit(&f.job(5), &f.clock.now())),
        Error::Contract(ContractError::LimitExceeded)
    );
    assert_eq!(f.adapter.attempts, 4);
    assert_eq!(f.adapter.scripts.len(), 1);
}

#[test]
fn stt_audio_request_bounds_fail_before_creating_an_attempt() {
    for (bytes, duration) in [
        (0, Duration::from_secs(1)),
        (32, Duration::ZERO),
        (32, MAX_CAPTURED_TURN + Duration::from_nanos(1)),
        (MAX_ENCODED_AUDIO_BYTES as usize + 1, Duration::from_secs(1)),
        (MAX_PROVIDER_REQUEST_BYTES as usize, Duration::from_secs(1)),
    ] {
        let mut f = Fixture::<Stt>::success();
        let job = f.job_with(
            1,
            SyntheticAudio {
                zeros: vec![0; bytes],
                duration,
            },
        );
        assert!(f.adapter.submit(&job, &f.clock.now()).is_err());
        assert_eq!(f.adapter.attempts, 0);
        assert!(f.adapter.active.is_none());
    }
}

#[test]
fn text_and_full_serialized_request_bounds_include_utf8_and_escaping() {
    let mut f = Fixture::<Conversation>::success();
    let job = f.job_with(
        1,
        SyntheticConversation {
            text: "é".repeat(MAX_CONVERSATION_INPUT_BYTES as usize / 2 + 1),
        },
    );
    assert_eq!(
        error(f.adapter.submit(&job, &f.clock.now())),
        Error::Contract(ContractError::LimitExceeded)
    );
    let job = f.job_with(
        1,
        SyntheticConversation {
            text: "\\".repeat(MAX_CONVERSATION_INPUT_BYTES as usize),
        },
    );
    assert_eq!(
        error(f.adapter.submit(&job, &f.clock.now())),
        Error::Contract(ContractError::LimitExceeded)
    );
    assert_eq!(f.adapter.attempts, 0);
    let job = f.job_with(
        1,
        SyntheticConversation {
            text: "é".repeat(MAX_CONVERSATION_INPUT_BYTES as usize / 2),
        },
    );
    assert!(f.adapter.submit(&job, &f.clock.now()).is_ok());
    let mut f = Fixture::<Tts>::success();
    let job = f.job_with(
        1,
        SyntheticSpeechText {
            text: "é".repeat(2049),
        },
    );
    assert_eq!(
        error(f.adapter.submit(&job, &f.clock.now())),
        Error::Contract(ContractError::LimitExceeded)
    );
}

#[test]
fn exact_transcript_and_reply_utf8_output_bounds_are_preserved() {
    let mut stt = Fixture::<Stt>::new(vec![response(transcript(&"é".repeat(4096)))]);
    let operation = stt.start(1);
    assert_eq!(
        stt.run(&operation).unwrap().output.text.len() as u64,
        MAX_USER_TRANSCRIPT_BYTES
    );
    let mut conversation =
        Fixture::<Conversation>::new(vec![response(reply("answer", &"é".repeat(2048)))]);
    let operation = conversation.start(1);
    assert_eq!(
        conversation.run(&operation).unwrap().output.text.len() as u64,
        MAX_ASSISTANT_REPLY_BYTES
    );
}

#[test]
fn oversized_role_outputs_retire_the_attempt_without_refund_or_retry() {
    let mut f = Fixture::<Stt>::new(vec![response(transcript(&"é".repeat(4097)))]);
    let operation = f.start(1);
    assert!(f.run(&operation).is_err());
    assert_eq!(f.adapter.ledger[0].state, State::Malformed);
    assert_eq!(f.adapter.attempts, 1);
    let mut f = Fixture::<Conversation>::new(vec![response(reply("answer", &"é".repeat(2049)))]);
    let operation = f.start(1);
    assert!(f.run(&operation).is_err());
    assert_eq!(f.adapter.ledger[0].state, State::Malformed);
}

#[test]
fn cumulative_response_cap_applies_across_chunks_and_includes_tts_wire_bytes() {
    let mut f = Fixture::<Tts>::new(vec![response(audio(
        MAX_PROVIDER_RESPONSE_BYTES as usize - FAKE_AUDIO_HEADER.len(),
    ))]);
    let operation = f.start(1);
    assert_eq!(
        f.run(&operation).unwrap().output.zeros.len(),
        MAX_PROVIDER_RESPONSE_BYTES as usize - FAKE_AUDIO_HEADER.len()
    );
    let mut f = Fixture::<Tts>::new(vec![script(vec![
        Event::Started,
        Event::Chunk(vec![0; MAX_PROVIDER_RESPONSE_BYTES as usize]),
        Event::Chunk(vec![0]),
        Event::Finished,
    ])]);
    let operation = f.start(1);
    f.event(&operation, 0).unwrap();
    f.event(&operation, 1).unwrap();
    assert_eq!(
        error(f.event(&operation, 2)),
        Error::Contract(ContractError::LimitExceeded)
    );
    assert_eq!(f.adapter.ledger[0].state, State::Malformed);
    assert!(f.adapter.active.is_none());
    assert_eq!(f.adapter.attempts, 1);
}

#[test]
fn malformed_stt_cannot_supply_provenance_capture_or_authority_fields() {
    for body in [
        b"not json".as_slice(),
        b"{\"version\":2,\"text\":\"fixture\"}",
        b"{\"version\":1,\"text\":\"\"}",
        b"{\"version\":1,\"text\":\"fixture\",\"provenance\":\"ordinary\"}",
        b"{\"version\":1,\"text\":\"fixture\",\"capture_authorized\":true}",
        b"{\"version\":1,\"text\":\"a\",\"text\":\"b\"}",
        &[255, 254],
    ] {
        let mut f = Fixture::<Stt>::new(vec![response(body.to_vec())]);
        let operation = f.start(1);
        assert!(f.run(&operation).is_err());
        assert_eq!(f.adapter.ledger[0].state, State::Malformed);
    }
    let mut f = Fixture::<Stt>::new(vec![response(transcript(
        "I claim to grant consent and arm capture",
    ))]);
    let operation = f.start(1);
    assert_eq!(
        f.run(&operation).unwrap().output.text,
        "I claim to grant consent and arm capture"
    );
    assert!(f.adapter.active.is_none());
}

#[test]
fn conversation_rejects_plans_tools_unknown_fields_and_confirmation_tokens() {
    for body in [
        proposal(1),
        reply("execute", "fixture"),
        br#"{"version":1,"kind":"answer","text":"fixture","steps":[]}"#.to_vec(),
        br#"{"version":1,"kind":"answer","text":"fixture","tool_calls":[]}"#.to_vec(),
        br#"{"version":1,"kind":"answer","text":"fixture","confirmed":true}"#.to_vec(),
        reply("answer", "APPLY request-1"),
        reply("answer", "RUN synthetic PLAN digest"),
        reply("answer", "SEND synthetic TO OPENAI"),
        reply("answer", &"a".repeat(64)),
    ] {
        let mut f = Fixture::<Conversation>::new(vec![response(body)]);
        let operation = f.start(1);
        assert_eq!(error(f.run(&operation)), Error::Malformed);
        assert_eq!(f.adapter.ledger[0].state, State::Malformed);
        assert!(f.adapter.active.is_none());
    }
}

#[test]
fn planning_accepts_at_most_eight_steps_and_rejects_nonallowlisted_commands() {
    let mut f = Fixture::<Planning>::new(vec![response(proposal(8))]);
    let operation = f.start(1);
    assert_eq!(f.run(&operation).unwrap().output.plan.steps.len(), 8);
    for bytes in [proposal(9), proposal(0), br#"{"kind":"plan","summary":"fixture","steps":[{"capability":"execute_shell","command":"echo synthetic"}]}"#.to_vec(), br#"{"kind":"plan","summary":"fixture","steps":[{"capability":"start_session_request","goal":"git push synthetic"}]}"#.to_vec()] {
        let mut f = Fixture::<Planning>::new(vec![response(bytes)]);
        let operation = f.start(1);
        assert_eq!(error(f.run(&operation)), Error::Malformed);
        assert_eq!(f.adapter.ledger[0].state, State::Malformed);
    }
}

#[test]
fn planning_clarify_and_unsupported_stay_empty_proposals_without_execution() {
    for kind in ["clarify", "unsupported"] {
        let body = serde_json::to_vec(
            &serde_json::json!({"kind":kind,"summary":"synthetic clarification","steps":[]}),
        )
        .unwrap();
        let mut f = Fixture::<Planning>::new(vec![response(body)]);
        let operation = f.start(1);
        assert!(f.run(&operation).unwrap().output.plan.steps.is_empty());
    }
}

#[test]
fn malformed_or_real_encoded_audio_never_becomes_synthetic_tts_output() {
    for bytes in [
        vec![],
        FAKE_AUDIO_HEADER.to_vec(),
        b"RIFF synthetic WAVE".to_vec(),
        b"https://synthetic.invalid/audio".to_vec(),
        [FAKE_AUDIO_HEADER, &[1]].concat(),
    ] {
        let mut f = Fixture::<Tts>::new(vec![response(bytes)]);
        let operation = f.start(1);
        assert_eq!(error(f.run(&operation)), Error::Malformed);
        assert_eq!(f.adapter.ledger[0].state, State::Malformed);
    }
}

#[test]
fn clocks_from_other_domains_rollback_and_expired_fixture_jobs_fail_closed() {
    let mut f = Fixture::<Stt>::success();
    let foreign = ManualClock::new();
    let job = f.job(1);
    assert_eq!(
        error(f.adapter.submit(&job, &foreign.now())),
        Error::Contract(ContractError::ClockDomainMismatch)
    );
    let foreign_job = fixtures::synthetic::<Stt>(
        identity(1),
        target::<Stt>(),
        Deadline::after(foreign.now(), Duration::from_secs(30)).unwrap(),
        Stt::input(),
    );
    assert_eq!(
        error(f.adapter.submit(&foreign_job, &f.clock.now())),
        Error::Contract(ContractError::ClockDomainMismatch)
    );
    let expired = fixtures::synthetic::<Stt>(
        identity(1),
        target::<Stt>(),
        Deadline::after(f.clock.now(), Duration::ZERO).unwrap(),
        Stt::input(),
    );
    assert_eq!(
        error(f.adapter.submit(&expired, &f.clock.now())),
        Error::Expired
    );
    let operation = f.start(1);
    let old = f.clock.now();
    f.clock.advance(Duration::from_secs(1)).unwrap();
    f.adapter.tick(&f.clock.now()).unwrap();
    let callback = f.adapter.callback_at(&operation, 0).unwrap();
    assert_eq!(
        error(f.adapter.deliver(&callback, &old)),
        Error::Contract(ContractError::TimeReversed)
    );
    assert_eq!(
        error(f.adapter.deliver(&callback, &foreign.now())),
        Error::Contract(ContractError::ClockDomainMismatch)
    );
    assert_eq!(f.adapter.ledger[0].state, State::AwaitingStart);
}

#[test]
fn counter_and_duration_overflow_fail_without_partial_admission() {
    for counter in 0..3 {
        let mut f = Fixture::<Stt>::success();
        match counter {
            0 => f.adapter.attempts = u64::MAX,
            1 => f.adapter.sequence = u64::MAX,
            2 => f.adapter.audio = Duration::MAX,
            _ => unreachable!(),
        }
        assert_eq!(
            error(f.adapter.submit(&f.job(1), &f.clock.now())),
            Error::Contract(ContractError::Overflow)
        );
        assert!(f.adapter.ledger.is_empty() && f.adapter.active.is_none());
        assert_eq!(f.adapter.scripts.len(), 1);
    }
    let mut f = Fixture::<Stt>::success();
    let operation = f.start(1);
    f.event(&operation, 0).unwrap();
    f.adapter.active.as_mut().unwrap().received = u64::MAX;
    assert_eq!(
        error(f.event(&operation, 1)),
        Error::Contract(ContractError::Overflow)
    );
    assert_eq!(f.adapter.ledger[0].state, State::Malformed);
    assert_eq!(f.adapter.attempts, 1);
}

#[test]
fn clock_deadline_and_schedule_overflow_cannot_allocate_an_attempt() {
    let mut clock = ManualClock::new();
    clock.advance(Duration::MAX).unwrap();
    assert_eq!(
        error(FakeStt::new(
            identity(1),
            target::<Stt>(),
            Vec::new(),
            clock.now()
        )),
        Error::Contract(ContractError::Overflow)
    );
    let mut f = Fixture::<Stt>::new(vec![Script {
        events: vec![Scheduled {
            after: Duration::MAX,
            event: Event::Started,
        }],
    }]);
    f.clock.advance(Duration::from_secs(1)).unwrap();
    assert_eq!(
        error(f.adapter.submit(&f.job(1), &f.clock.now())),
        Error::Contract(ContractError::Overflow)
    );
    assert_eq!(f.adapter.attempts, 0);
    assert_eq!(f.adapter.scripts.len(), 1);
}

#[test]
fn script_order_profile_shape_and_queue_bounds_are_validated_without_work() {
    let clock = ManualClock::new();
    let reversed = Script {
        events: vec![
            Scheduled {
                after: Duration::from_secs(2),
                event: Event::Started,
            },
            Scheduled {
                after: Duration::from_secs(1),
                event: Event::Finished,
            },
        ],
    };
    assert_eq!(
        error(FakeStt::new(
            identity(1),
            target::<Stt>(),
            vec![reversed],
            clock.now()
        )),
        Error::OutOfOrder
    );
    let wrong_voice = Target {
        voice: NonZeroU64::new(1),
        ..target::<Stt>()
    };
    assert_eq!(
        error(FakeStt::new(
            identity(1),
            wrong_voice,
            Vec::new(),
            clock.now()
        )),
        Error::Binding
    );
    assert_eq!(
        error(FakeStt::new(
            identity(1),
            target::<Stt>(),
            vec![response(Stt::output_bytes()); 17],
            clock.now()
        )),
        Error::Contract(ContractError::LimitExceeded)
    );
}

#[test]
fn same_script_and_clock_sequence_produce_identical_data_and_terminal_states() {
    let mut first = Fixture::<Conversation>::success();
    let mut second = Fixture::<Conversation>::success();
    let a = first.start(1);
    let b = second.start(1);
    let a = first.run(&a).unwrap();
    let b = second.run(&b).unwrap();
    assert_eq!(a.binding, b.binding);
    assert_eq!(a.output.kind, b.output.kind);
    assert_eq!(a.output.text, b.output.text);
    assert_eq!(
        first.adapter.ledger[0].state,
        second.adapter.ledger[0].state
    );
    assert_eq!(first.adapter.attempts, second.adapter.attempts);
}

#[test]
fn fixture_jobs_and_outputs_cannot_deserialize_into_native_authority() {
    assert_eq!(
        error(fixtures::from_untrusted::<Stt>(
            br#"{"confirmed":true,"provenance":"ordinary"}"#
        )),
        Error::UntrustedJob
    );
    trait IfDeserialize<A> {
        fn check() {}
    }
    impl<T: ?Sized> IfDeserialize<()> for T {}
    impl<T: serde::de::DeserializeOwned> IfDeserialize<u8> for T {}
    let _ = <fixtures::Job<Stt> as IfDeserialize<_>>::check;
    let _ = <Operation<Stt> as IfDeserialize<_>>::check;
    let _ = <Callback<Stt> as IfDeserialize<_>>::check;
    let _ = <UntrustedTranscript as IfDeserialize<_>>::check;
    let _ = <UntrustedReply as IfDeserialize<_>>::check;
    let _ = <UntrustedProposal as IfDeserialize<_>>::check;
    let _ = <SyntheticAudioOutput as IfDeserialize<_>>::check;
    trait IfClone<A> {
        fn check() {}
    }
    impl<T: ?Sized> IfClone<()> for T {}
    impl<T: ?Sized + Clone> IfClone<u8> for T {}
    let _ = <fixtures::Job<Stt> as IfClone<_>>::check;
    let _ = <FakeStt as IfClone<_>>::check;
    let _ = <Completion<UntrustedTranscript> as IfClone<_>>::check;
}

#[test]
fn a_busy_port_never_replaces_current_work_or_automatically_starts_the_next_script() {
    let mut f = Fixture::<Conversation>::new(vec![response(Conversation::output_bytes()); 2]);
    let first = f.start(1);
    let second_job = f.job(2);
    assert_eq!(
        error(f.adapter.submit(&second_job, &f.clock.now())),
        Error::Busy
    );
    assert_eq!(f.adapter.attempts, 1);
    assert_eq!(f.adapter.scripts.len(), 1);
    f.run(&first).unwrap();
    assert!(f.adapter.active.is_none());
    assert_eq!(f.adapter.scripts.len(), 1);
    let second = f.adapter.submit(&second_job, &f.clock.now()).unwrap();
    assert_eq!(f.adapter.attempts, 2);
    assert!(f.run(&second).is_ok());
}

#[test]
fn split_utf8_and_json_chunks_return_no_text_before_validated_completion() {
    let bytes = transcript("synthetic é speech");
    let split = bytes.iter().position(|b| *b == 0xc3).unwrap() + 1;
    let mut f = Fixture::<Stt>::new(vec![script(vec![
        Event::Started,
        Event::Chunk(bytes[..split].to_vec()),
        Event::Chunk(bytes[split..].to_vec()),
        Event::Finished,
    ])]);
    let operation = f.start(1);
    assert!(matches!(f.event(&operation, 0), Ok(Update::Started)));
    assert!(matches!(f.event(&operation, 1), Ok(Update::Progress)));
    assert!(matches!(f.event(&operation, 2), Ok(Update::Progress)));
    let Ok(Update::Succeeded(done)) = f.event(&operation, 3) else {
        panic!("missing bounded synthetic text")
    };
    assert_eq!(done.output.text, "synthetic é speech");
}

#[test]
fn planning_project_context_is_required_by_the_existing_validator() {
    let mut f = Fixture::<Planning>::success();
    let job = f.job_with(
        1,
        SyntheticPlanning {
            reviewed_text: "synthetic reviewed input".into(),
            project_alias: None,
        },
    );
    let operation = f.adapter.submit(&job, &f.clock.now()).unwrap();
    assert_eq!(error(f.run(&operation)), Error::Malformed);
    assert_eq!(f.adapter.ledger[0].state, State::Malformed);
    assert_eq!(f.adapter.attempts, 1);
    let mut f = Fixture::<Planning>::success();
    let job = f.job_with(
        1,
        SyntheticPlanning {
            reviewed_text: "synthetic input".into(),
            project_alias: Some("../synthetic".into()),
        },
    );
    assert_eq!(
        error(f.adapter.submit(&job, &f.clock.now())),
        Error::Malformed
    );
    assert_eq!(f.adapter.attempts, 0);
}

#[test]
fn new_explicit_jobs_never_renew_the_original_interaction_deadline() {
    let mut f = Fixture::<Stt>::new(vec![response(Stt::output_bytes()); 2]);
    let first = f.start(1);
    f.run(&first).unwrap();
    f.clock
        .advance(MAX_CLOUD_INTERACTION - Duration::from_secs(10))
        .unwrap();
    let second = f.start(2);
    assert_eq!(
        f.adapter
            .active
            .as_ref()
            .unwrap()
            .deadline
            .remaining(&f.clock.now())
            .unwrap(),
        Duration::from_secs(10)
    );
    let late = f.adapter.callback_at(&second, 0).unwrap();
    let old = f.clock.now();
    f.clock.advance(Duration::from_secs(10)).unwrap();
    assert_eq!(error(f.adapter.tick(&f.clock.now())), Error::Expired);
    assert_eq!(f.adapter.ledger[1].state, State::TimedOut);
    assert_eq!(error(f.adapter.deliver(&late, &old)), Error::Stopped);
    assert_eq!(f.adapter.attempts, 2);
}
