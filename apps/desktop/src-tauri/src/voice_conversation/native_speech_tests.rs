use super::*;
use crate::voice_conversation::clock::ManualClock;
use crate::voice_conversation::limits::{
    MAX_ACTIVE_SPEECH_JOBS, MAX_PENDING_SPEECH_JOBS, MAX_QUEUED_SPEECH_TEXT_BYTES,
};
use crate::voice_conversation::model::{
    ConfigurationRevision, ConsentBindingId, ContractVersion, Generation, InteractionId, SessionId,
    TurnId,
};
use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Call {
    Lookup,
    Prepare,
    Attach,
    BeforeDispatch,
    Speak,
    Stop,
    Detach,
}

struct FakeUtterance(Rc<Cell<usize>>);

impl Drop for FakeUtterance {
    fn drop(&mut self) {
        self.0.set(self.0.get() + 1);
    }
}

struct FakeBackend {
    calls: std::cell::RefCell<Vec<Call>>,
    selected: Option<VoiceId>,
    objects: NativeObjects,
    inbox: Option<Arc<Inbox>>,
    during_attach: Option<Callback>,
    during_speak: Option<Callback>,
    during_stop: Option<Callback>,
    speak_result: Result<()>,
    stop_result: Result<bool>,
    detach_result: Detach,
    attach_result: Result<()>,
    after_attach: Option<Box<dyn FnOnce()>>,
    phase_hook: RefCell<Option<(Call, Box<dyn FnOnce()>)>>,
    captured_inbox: Option<Rc<std::cell::RefCell<Option<Arc<Inbox>>>>>,
    after_stop: std::cell::RefCell<Option<Box<dyn FnOnce()>>>,
    drops: Rc<Cell<usize>>,
    dispatches: Rc<Cell<usize>>,
}

impl FakeBackend {
    fn new() -> Self {
        Self {
            calls: Default::default(),
            selected: Some(voice_id()),
            objects: NativeObjects {
                synthesizer: 10,
                utterance: 11,
            },
            inbox: None,
            during_attach: None,
            during_speak: None,
            during_stop: None,
            speak_result: Ok(()),
            stop_result: Ok(true),
            detach_result: Detach::VerifiedNil,
            attach_result: Ok(()),
            after_attach: None,
            phase_hook: RefCell::new(None),
            captured_inbox: None,
            after_stop: Default::default(),
            drops: Rc::new(Cell::new(0)),
            dispatches: Rc::new(Cell::new(0)),
        }
    }

    fn at(&self, phase: Call) {
        let mut hook = self.phase_hook.borrow_mut();
        if hook
            .as_ref()
            .is_some_and(|(expected, _)| *expected == phase)
        {
            let (_, action) = hook.take().unwrap();
            drop(hook);
            action();
        }
    }

    fn emit(&self, event: Callback) {
        let inbox = self.inbox.as_ref().unwrap();
        inbox.observe(inbox.key, event);
    }
}

impl Backend for FakeBackend {
    type Voice = ();
    type Utterance = FakeUtterance;

    fn find_voice(&self, _id: &VoiceId) -> Result<Option<(VoiceId, Self::Voice)>> {
        self.calls.borrow_mut().push(Call::Lookup);
        self.at(Call::Lookup);
        Ok(self.selected.clone().map(|id| (id, ())))
    }

    fn prepare(
        &self,
        _voice: Self::Voice,
        _text: &str,
    ) -> Result<(Self::Utterance, NativeObjects)> {
        self.calls.borrow_mut().push(Call::Prepare);
        self.at(Call::Prepare);
        Ok((FakeUtterance(self.drops.clone()), self.objects))
    }

    fn attach(&mut self, inbox: Arc<Inbox>) -> Result<()> {
        self.calls.borrow_mut().push(Call::Attach);
        self.at(Call::Attach);
        if let Some(captured) = &self.captured_inbox {
            captured.replace(Some(Arc::clone(&inbox)));
        }
        assert!(self.inbox.replace(inbox).is_none());
        if let Some(event) = self.during_attach {
            self.emit(event);
        }
        if let Some(action) = self.after_attach.take() {
            action();
        }
        self.attach_result
    }

    fn speak(
        &self,
        _utterance: &Self::Utterance,
        before_dispatch: impl FnOnce() -> Result<()>,
    ) -> Result<()> {
        self.at(Call::BeforeDispatch);
        before_dispatch()?;
        self.calls.borrow_mut().push(Call::Speak);
        self.dispatches.set(self.dispatches.get() + 1);
        self.at(Call::Speak);
        if let Some(event) = self.during_speak {
            self.emit(event);
        }
        self.speak_result
    }

    fn stop_immediately(&self) -> Result<bool> {
        self.calls.borrow_mut().push(Call::Stop);
        if let Some(event) = self.during_stop {
            self.emit(event);
        }
        let action = self.after_stop.borrow_mut().take();
        if let Some(action) = action {
            action();
        }
        self.stop_result
    }

    fn detach(&mut self) -> Detach {
        self.calls.borrow_mut().push(Call::Detach);
        self.inbox = None;
        self.detach_result
    }
}

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

fn voice_id() -> VoiceId {
    VoiceId::new("Native.é/Voice 1").unwrap()
}

type FakePort = Port<FakeBackend, ManualClock>;

fn scope() -> OutputScope {
    OutputScope::new(
        identity(1).app_run_id,
        Generation::from_native_sequence(1).unwrap(),
    )
}

fn binding(port: &FakePort, sequence: u64) -> FlightBinding {
    port.scope.bind(identity(sequence), sequence)
}

fn port_with(backend: FakeBackend) -> FakePort {
    Port::new(Ok(backend), ManualClock::new(), scope()).unwrap()
}

fn request_at(
    binding: FlightBinding,
    sequence: u64,
    text: &str,
    voice: &VoiceId,
    admitted: &MonotonicTime,
    port: &mut Port<FakeBackend, impl MonotonicClock>,
) -> Result<Notice> {
    let obligation = RefCell::new(Some(binding.clone()));
    let deadline = Deadline::after(admitted.clone(), SPEECH_OUTPUT_DEADLINE)?;
    let authority = authority_for(&binding, &deadline);
    let revoked = Cell::new(false);
    let check = SubmissionCheck::new(&binding, &authority, &revoked, admitted, [&deadline; 2])
        .with_obligation(&obligation);
    port.request(binding.clone(), sequence, text, voice, admitted, check)
}

fn request(port: &mut FakePort, sequence: u64) -> Result<Notice> {
    request_at(
        binding(port, sequence),
        sequence,
        "Synthetic text",
        &voice_id(),
        &port.clock.now(),
        port,
    )
}

fn notice(sequence: u64, evidence: Evidence) -> Notice {
    Notice {
        identity: identity(sequence),
        evidence,
    }
}

#[test]
fn construction_is_inert_and_rejects_an_unavailable_backend() {
    let mut port = port_with(FakeBackend::new());
    assert_eq!(port.poll(), None);
    assert!(port.backend.calls.borrow().is_empty());
    for error in [Error::BackendUnavailable, Error::NativeFailure] {
        assert!(matches!(
            FakePort::new(Err(error), ManualClock::new(), scope()),
            Err(received) if received == error
        ));
    }
}

#[test]
fn synthetic_inventory_preserves_exact_bounded_metadata() {
    let voice = Voice::new(
        voice_id(),
        "é".repeat(MAX_VOICE_NAME_BYTES / 2),
        "en-US".into(),
    )
    .unwrap();
    let voices = validate_inventory(vec![voice]).unwrap();
    assert!(voices[0].id == voice_id());
    assert_eq!(voices[0].name.len(), MAX_VOICE_NAME_BYTES);
    assert_eq!(voices[0].language, "en-US");

    for invalid in [
        "",
        "id\0tail",
        "id\n",
        &"é".repeat(MAX_VOICE_ID_BYTES / 2 + 1),
    ] {
        assert!(matches!(VoiceId::new(invalid), Err(Error::InvalidVoiceId)));
    }
    assert!(VoiceId::new(&"é".repeat(MAX_VOICE_ID_BYTES / 2)).is_ok());
    assert!(matches!(
        Voice::new(
            voice_id(),
            "x".repeat(MAX_VOICE_NAME_BYTES + 1),
            "en-US".into()
        ),
        Err(Error::InvalidVoiceMetadata)
    ));
    assert!(matches!(
        Voice::new(
            voice_id(),
            "Voice".into(),
            "x".repeat(MAX_VOICE_LANGUAGE_BYTES + 1)
        ),
        Err(Error::InvalidVoiceMetadata)
    ));

    let synthetic = || Voice::new(voice_id(), "Test".into(), "en-US".into()).unwrap();
    assert!(matches!(
        validate_inventory(vec![synthetic(), synthetic()]),
        Err(Error::InvalidVoiceMetadata)
    ));
    let inventory: Vec<_> = (0..MAX_VOICES + 1)
        .map(|index| {
            Voice::new(
                VoiceId::new(&format!("voice/{index}")).unwrap(),
                "Test".into(),
                "en-US".into(),
            )
            .unwrap()
        })
        .collect();
    assert!(matches!(
        validate_inventory(inventory),
        Err(Error::InvalidVoiceMetadata)
    ));
}

#[test]
fn missing_or_replaced_voice_never_submits_or_retries() {
    for selected in [None, Some(VoiceId::new("native.é/Voice 1").unwrap())] {
        let mut backend = FakeBackend::new();
        backend.selected = selected;
        let mut port = port_with(backend);
        assert_eq!(request(&mut port, 1), Err(Error::VoiceUnavailable));
        assert_eq!(request(&mut port, 1), Err(Error::StaleJob));
        assert_eq!(port.poll(), None);
        assert_eq!(*port.backend.calls.borrow(), [Call::Lookup]);
    }
}

#[test]
fn validated_request_has_one_native_slot_and_phase_one_byte_bounds() {
    assert_eq!((MAX_ACTIVE_SPEECH_JOBS, MAX_PENDING_SPEECH_JOBS), (1, 1));
    assert_eq!(2 * MAX_SPEECH_JOB_TEXT_BYTES, MAX_QUEUED_SPEECH_TEXT_BYTES);
    let mut port = port_with(FakeBackend::new());
    let admitted = port.clock.now();
    let limit = "é".repeat(MAX_SPEECH_JOB_TEXT_BYTES as usize / 2);
    for invalid in ["", " \n", "x\0y"] {
        assert_eq!(
            request_at(
                binding(&port, 1),
                1,
                invalid,
                &voice_id(),
                &admitted,
                &mut port
            ),
            Err(Error::InvalidText)
        );
        assert!(port
            .take_retirement_report()
            .unwrap()
            .can_abandon_unsubmitted());
    }
    assert_eq!(
        request_at(
            binding(&port, 1),
            1,
            &(limit.clone() + "é"),
            &voice_id(),
            &admitted,
            &mut port
        ),
        Err(Error::Contract(ContractError::LimitExceeded))
    );
    assert!(port
        .take_retirement_report()
        .unwrap()
        .can_abandon_unsubmitted());
    assert!(port.backend.calls.borrow().is_empty());
    assert_eq!(
        request_at(
            binding(&port, 1),
            1,
            &limit,
            &voice_id(),
            &admitted,
            &mut port
        ),
        Ok(notice(1, Evidence::Requested))
    );
    assert_eq!(request(&mut port, 2), Err(Error::Busy));
    assert_eq!(port.poll(), None);
    assert_eq!(port.backend.drops.get(), 0);
    assert_eq!(
        *port.backend.calls.borrow(),
        [Call::Lookup, Call::Prepare, Call::Attach, Call::Speak]
    );
}

#[test]
fn nil_native_object_never_reaches_speech() {
    let mut backend = FakeBackend::new();
    backend.objects.utterance = 0;
    let mut port = port_with(backend);
    assert_eq!(request(&mut port, 1), Err(Error::NativeFailure));
    assert_eq!(port.backend.drops.get(), 1);
    assert_eq!(*port.backend.calls.borrow(), [Call::Lookup, Call::Prepare]);
}

#[test]
fn callbacks_require_the_job_generation_synthesizer_and_utterance() {
    let mut port = port_with(FakeBackend::new());
    assert_eq!(request(&mut port, 1), Ok(notice(1, Evidence::Requested)));
    let inbox = port.backend.inbox.as_ref().unwrap().clone();
    let mut bad_job = inbox.key;
    bad_job.identity = identity(2);
    let mut bad_generation = inbox.key;
    bad_generation.identity.generation = Generation::from_native_sequence(2).unwrap();
    let mut bad_synth = inbox.key;
    bad_synth.objects.synthesizer += 1;
    let mut bad_utterance = inbox.key;
    bad_utterance.objects.utterance += 1;
    for key in [bad_job, bad_generation, bad_synth, bad_utterance] {
        for event in [Callback::Start, Callback::Finish, Callback::Cancel] {
            inbox.observe(key, event);
        }
    }
    assert_eq!(
        port.stop(bad_generation.identity, StopOrigin::User),
        Err(Error::StaleJob)
    );
    assert_eq!(port.poll(), None);

    inbox.observe(inbox.key, Callback::Start);
    inbox.observe(inbox.key, Callback::Start);
    inbox.observe(inbox.key, Callback::Finish);
    inbox.observe(inbox.key, Callback::Finish);
    inbox.observe(inbox.key, Callback::Cancel);
    assert_eq!(port.poll(), Some(notice(1, Evidence::Started)));
    assert_eq!(port.poll(), Some(notice(1, Evidence::Finished)));
    assert_eq!(port.poll(), None);
    assert_eq!(port.backend.drops.get(), 1);
    assert_eq!(
        *port.backend.calls.borrow(),
        [
            Call::Lookup,
            Call::Prepare,
            Call::Attach,
            Call::Speak,
            Call::Detach
        ]
    );

    // The fake reuses both native object addresses, like a native allocator can.
    assert!(port.take_retirement_report().is_some());
    assert_eq!(request(&mut port, 2), Ok(notice(2, Evidence::Requested)));
    inbox.observe(inbox.key, Callback::Start);
    inbox.observe(inbox.key, Callback::Cancel);
    assert_eq!(port.poll(), None);
    port.backend.emit(Callback::Start);
    assert_eq!(port.poll(), Some(notice(2, Evidence::Started)));
}

#[test]
fn only_a_callback_after_submission_can_start_playback() {
    let mut backend = FakeBackend::new();
    backend.during_attach = Some(Callback::Start);
    let mut port = port_with(backend);
    assert_eq!(request(&mut port, 1), Ok(notice(1, Evidence::Requested)));
    assert_eq!(port.poll(), None);
    port.backend.emit(Callback::Finish);
    port.backend.emit(Callback::Start);
    assert_eq!(
        port.poll(),
        Some(notice(1, Evidence::Failure(Error::UnexpectedCallback)))
    );
    assert_eq!(port.poll(), None);
    assert_eq!(request(&mut port, 2), Err(Error::Busy));
    assert_eq!(port.backend.calls.borrow().last(), Some(&Call::Stop));

    let mut backend = FakeBackend::new();
    backend.during_speak = Some(Callback::Start);
    let mut port = port_with(backend);
    assert_eq!(request(&mut port, 1), Ok(notice(1, Evidence::Requested)));
    assert_eq!(port.poll(), Some(notice(1, Evidence::Started)));
}

#[test]
fn stop_drops_buffered_success_and_requires_matching_cancellation() {
    let mut port = port_with(FakeBackend::new());
    request(&mut port, 1).unwrap();
    port.backend.emit(Callback::Start);
    port.backend.emit(Callback::Finish);
    let unconfirmed = notice(1, Evidence::CancellationUnconfirmed(StopAttempt::Accepted));
    assert_eq!(port.stop(identity(1), StopOrigin::User), Ok(unconfirmed));
    port.backend.emit(Callback::Start);
    port.backend.emit(Callback::Finish);
    assert_eq!(port.poll(), None);
    assert_eq!(port.stop(identity(1), StopOrigin::User), Ok(unconfirmed));
    assert_eq!(port.backend.drops.get(), 0);
    assert_eq!(request(&mut port, 2), Err(Error::Busy));
    port.backend.emit(Callback::Cancel);
    port.backend.emit(Callback::Cancel);
    assert_eq!(
        port.poll(),
        Some(notice(
            1,
            Evidence::Cancelled {
                stop_requested: true
            }
        ))
    );
    assert_eq!(port.poll(), None);
    assert_eq!(port.backend.drops.get(), 1);
    assert_eq!(
        *port.backend.calls.borrow(),
        [
            Call::Lookup,
            Call::Prepare,
            Call::Attach,
            Call::Speak,
            Call::Stop,
            Call::Detach
        ]
    );
}

#[test]
fn synchronous_cancel_still_arrives_as_separate_evidence() {
    let mut backend = FakeBackend::new();
    backend.during_stop = Some(Callback::Cancel);
    let mut port = port_with(backend);
    request(&mut port, 1).unwrap();
    assert_eq!(
        port.stop(identity(1), StopOrigin::User),
        Ok(notice(
            1,
            Evidence::CancellationUnconfirmed(StopAttempt::Accepted)
        ))
    );
    assert_eq!(
        port.poll(),
        Some(notice(
            1,
            Evidence::Cancelled {
                stop_requested: true
            }
        ))
    );
}

#[test]
fn unsolicited_cancel_cannot_acknowledge_a_later_stop() {
    let mut port = port_with(FakeBackend::new());
    request(&mut port, 1).unwrap();
    port.backend.emit(Callback::Cancel);
    port.stop(identity(1), StopOrigin::User).unwrap();
    assert_eq!(
        port.poll(),
        Some(notice(
            1,
            Evidence::Cancelled {
                stop_requested: false
            }
        ))
    );
    assert_eq!(port.poll(), None);
}

#[test]
fn failed_native_stop_remains_unconfirmed_without_retry() {
    for (result, attempt) in [
        (Ok(false), StopAttempt::Rejected),
        (Err(Error::NativeFailure), StopAttempt::Failed),
    ] {
        let mut backend = FakeBackend::new();
        backend.stop_result = result;
        let mut port = port_with(backend);
        request(&mut port, 1).unwrap();
        let unconfirmed = notice(1, Evidence::CancellationUnconfirmed(attempt));
        assert_eq!(port.stop(identity(1), StopOrigin::User), Ok(unconfirmed));
        assert_eq!(port.poll(), None);
        assert_eq!(port.stop(identity(1), StopOrigin::User), Ok(unconfirmed));
        assert_eq!(request(&mut port, 2), Err(Error::Busy));
        assert_eq!(port.shutdown(), Some(unconfirmed));
        assert_eq!(port.backend.drops.get(), 1);
        assert_eq!(
            *port.backend.calls.borrow(),
            [
                Call::Lookup,
                Call::Prepare,
                Call::Attach,
                Call::Speak,
                Call::Stop,
                Call::Detach
            ]
        );
    }
}

#[test]
fn deadline_is_original_admission_time_and_missing_completion_fails_closed() {
    let mut port = port_with(FakeBackend::new());
    let admitted = port.clock.now();
    port.clock
        .advance(SPEECH_OUTPUT_DEADLINE - Duration::from_nanos(1))
        .unwrap();
    assert_eq!(
        request_at(
            binding(&port, 1),
            1,
            "Test",
            &voice_id(),
            &admitted,
            &mut port
        ),
        Ok(notice(1, Evidence::Requested))
    );
    port.backend.emit(Callback::Start);
    assert_eq!(port.poll(), Some(notice(1, Evidence::Started)));
    port.clock.advance(Duration::from_nanos(1)).unwrap();
    assert_eq!(
        port.poll(),
        Some(notice(1, Evidence::Failure(Error::Deadline)))
    );
    port.backend.emit(Callback::Finish);
    assert_eq!(port.poll(), None);
    assert_eq!(request(&mut port, 2), Err(Error::Busy));
    assert_eq!(port.backend.calls.borrow().last(), Some(&Call::Stop));
}

#[test]
fn expired_or_foreign_admission_never_calls_backend() {
    let mut port = port_with(FakeBackend::new());
    let admitted = port.clock.now();
    port.clock.advance(SPEECH_OUTPUT_DEADLINE).unwrap();
    assert_eq!(
        request_at(
            binding(&port, 1),
            1,
            "Test",
            &voice_id(),
            &admitted,
            &mut port
        ),
        Err(Error::Deadline)
    );
    assert!(port
        .take_retirement_report()
        .unwrap()
        .can_abandon_unsubmitted());
    assert_eq!(
        request_at(
            binding(&port, 1),
            1,
            "Test",
            &voice_id(),
            &ManualClock::new().now(),
            &mut port
        ),
        Err(Error::Contract(ContractError::ClockDomainMismatch))
    );
    assert!(port.backend.calls.borrow().is_empty());
}

#[test]
fn submission_failure_requests_cleanup_but_never_retries_or_reports_success() {
    let mut backend = FakeBackend::new();
    backend.during_speak = Some(Callback::Start);
    backend.speak_result = Err(Error::NativeFailure);
    let mut port = port_with(backend);
    assert_eq!(
        request(&mut port, 1),
        Ok(notice(1, Evidence::Failure(Error::NativeFailure)))
    );
    assert_eq!(port.poll(), None);
    assert_eq!(request(&mut port, 2), Err(Error::Busy));
    assert_eq!(
        *port.backend.calls.borrow(),
        [
            Call::Lookup,
            Call::Prepare,
            Call::Attach,
            Call::Speak,
            Call::Stop
        ]
    );
}

#[test]
fn shutdown_detaches_and_makes_outstanding_callbacks_inert() {
    let mut port = port_with(FakeBackend::new());
    request(&mut port, 1).unwrap();
    let outstanding = port.backend.inbox.as_ref().unwrap().clone();
    assert_eq!(
        port.shutdown(),
        Some(notice(
            1,
            Evidence::CancellationUnconfirmed(StopAttempt::Accepted)
        ))
    );
    assert!(port.backend.inbox.is_none());
    assert_eq!(port.backend.drops.get(), 1);
    for callback in [Callback::Start, Callback::Finish, Callback::Cancel] {
        outstanding.observe(outstanding.key, callback);
    }
    assert_eq!(outstanding.take_evidence(), None);
    assert_eq!(port.poll(), None);
    assert_eq!(port.shutdown(), None);
    assert_eq!(request(&mut port, 2), Err(Error::Closed));
}

#[test]
fn delegate_detach_failure_closes_the_port() {
    let mut backend = FakeBackend::new();
    backend.detach_result = Detach::SetterFailed;
    let mut port = port_with(backend);
    request(&mut port, 1).unwrap();
    port.backend.emit(Callback::Start);
    port.backend.emit(Callback::Finish);
    assert_eq!(port.poll(), Some(notice(1, Evidence::Started)));
    assert_eq!(port.poll(), Some(notice(1, Evidence::Finished)));
    assert_eq!(request(&mut port, 2), Err(Error::Closed));
}

#[test]
fn only_terminal_evidence_maps_to_a_phase_one_outcome() {
    for evidence in [
        Evidence::Requested,
        Evidence::Started,
        Evidence::CancellationUnconfirmed(StopAttempt::Accepted),
        Evidence::CancellationUnconfirmed(StopAttempt::Rejected),
        Evidence::CancellationUnconfirmed(StopAttempt::Failed),
    ] {
        assert_eq!(evidence.outcome(), None);
    }
    assert_eq!(
        Evidence::Finished.outcome(),
        Some(SpeechOutcome::NormalCompletion)
    );
    assert_eq!(
        Evidence::Cancelled {
            stop_requested: true
        }
        .outcome(),
        Some(SpeechOutcome::Stop)
    );
    assert_eq!(
        Evidence::Cancelled {
            stop_requested: false
        }
        .outcome(),
        Some(SpeechOutcome::Failure)
    );
    assert_eq!(
        Evidence::Failure(Error::Deadline).outcome(),
        Some(SpeechOutcome::Failure)
    );
}

#[cfg(target_os = "macos")]
#[test]
fn native_entry_points_are_linked_without_calling_avfoundation() {
    use objc2::MainThreadMarker;
    use std::hint::black_box;

    // Materialize the constructor and the speech-port vtable, including all
    // backend methods, without invoking the constructor or discovery.
    type Open = fn(MainThreadMarker, ManualClock, OutputScope) -> Result<Box<dyn RetirementOutput>>;
    let open_port: Open = |marker, clock, scope| {
        Ok(Box::new(open(marker, clock, scope)?) as Box<dyn RetirementOutput>)
    };
    let _ = black_box(open_port);
    let _ = black_box(installed_voices as fn() -> Result<Vec<Voice>>);
}

#[cfg(target_os = "macos")]
pub(in crate::voice_conversation) fn retirement_fixture(
    scope: OutputScope,
    clock: impl MonotonicClock + 'static,
    voice: VoiceId,
    reenter_on_stop: impl FnOnce() + 'static,
) -> (Box<dyn RetirementOutput>, impl Fn(), Rc<Cell<usize>>) {
    let inbox = Rc::new(std::cell::RefCell::new(None::<Arc<Inbox>>));
    let mut backend = FakeBackend::new();
    backend.selected = Some(voice);
    backend.captured_inbox = Some(Rc::clone(&inbox));
    backend.after_stop.replace(Some(Box::new(reenter_on_stop)));
    let drops = backend.drops.clone();
    let port = Port::new(Ok(backend), clock, scope).unwrap();
    let cancel = move || {
        let inbox = inbox.borrow();
        let inbox = inbox.as_ref().unwrap();
        inbox.observe(inbox.key, Callback::Cancel);
    };
    (Box::new(port), cancel, drops)
}

#[cfg(target_os = "macos")]
pub(in crate::voice_conversation) fn preparation_fixture(
    scope: OutputScope,
    clock: impl MonotonicClock + 'static,
    voice: VoiceId,
    after_attach: impl FnOnce() + 'static,
) -> (Box<dyn RetirementOutput>, Rc<Cell<usize>>) {
    let mut backend = FakeBackend::new();
    backend.selected = Some(voice);
    backend.after_attach = Some(Box::new(after_attach));
    let dispatches = backend.dispatches.clone();
    (
        Box::new(Port::new(Ok(backend), clock, scope).unwrap()),
        dispatches,
    )
}

#[test]
fn retirement_preserves_finish_on_either_side_of_stop_without_using_it_as_stop_ack() {
    for finish_before_stop in [true, false] {
        let mut port = port_with(FakeBackend::new());
        let expected = binding(&port, 1);
        request_at(
            expected.clone(),
            1,
            "Synthetic",
            &voice_id(),
            &port.clock.now(),
            &mut port,
        )
        .unwrap();
        port.backend.emit(Callback::Start);
        if finish_before_stop {
            port.backend.emit(Callback::Finish);
        }
        let unconfirmed = port.begin_retirement().unwrap();
        if !finish_before_stop {
            port.backend.emit(Callback::Finish);
        }
        assert_eq!(port.poll(), None);
        assert!(port.take_retirement_report().is_none());
        assert_eq!(port.shutdown(), Some(unconfirmed));
        let report = port.take_retirement_report().unwrap();
        assert!(report.matches(&expected));
        assert_eq!(report.facts().terminal, Terminal::Finished);
        assert_eq!(
            report.facts().first_stop,
            Some((StopOrigin::Shutdown, StopAttempt::Accepted))
        );
        assert_eq!(report.facts().attachment, Attachment::Attached);
        assert_eq!(report.facts().detach, Detach::VerifiedNil);
        assert_eq!(report.facts().submission, Submission::PossiblySubmitted);
        assert_eq!(
            report.facts().uncertainty,
            RetirementUncertainty::OutputReleaseUnverified
        );
        assert!(!report.can_abandon_unsubmitted());
        assert!(port.take_retirement_report().is_none());
    }
}

#[test]
fn delayed_cancel_survives_begin_retirement_but_not_native_disposal() {
    let mut port = port_with(FakeBackend::new());
    request(&mut port, 1).unwrap();
    let old = port.backend.inbox.as_ref().unwrap().clone();
    assert!(port.begin_retirement().is_some());
    assert_eq!(port.poll(), None);
    assert_eq!(port.backend.drops.get(), 0);
    old.observe(old.key, Callback::Finish);
    assert_eq!(port.poll(), None);
    old.observe(old.key, Callback::Cancel);
    old.observe(old.key, Callback::Cancel);
    assert_eq!(
        port.poll(),
        Some(notice(
            1,
            Evidence::Cancelled {
                stop_requested: true
            }
        ))
    );
    assert_eq!(port.backend.drops.get(), 1);
    let report = port.take_retirement_report().unwrap();
    assert_eq!(
        report.facts().terminal,
        Terminal::Cancelled {
            stop_requested: true
        }
    );
    assert!(!report.can_abandon_unsubmitted());
    old.observe(old.key, Callback::Cancel);
    assert_eq!(old.take_evidence(), None);
    assert!(port.take_retirement_report().is_none());
}

#[test]
fn first_stop_origin_and_unsolicited_cancellation_are_not_rewritten() {
    let mut backend = FakeBackend::new();
    backend.during_speak = Some(Callback::Cancel);
    backend.speak_result = Err(Error::NativeFailure);
    let mut port = port_with(backend);
    assert_eq!(
        request(&mut port, 1),
        Ok(notice(1, Evidence::Failure(Error::NativeFailure)))
    );
    port.stop(identity(1), StopOrigin::User).unwrap();
    assert_eq!(
        port.poll(),
        Some(notice(
            1,
            Evidence::Cancelled {
                stop_requested: false
            }
        ))
    );
    let report = port.take_retirement_report().unwrap();
    assert_eq!(
        report.facts().terminal,
        Terminal::Cancelled {
            stop_requested: false
        }
    );
    assert_eq!(
        report.facts().first_stop,
        Some((StopOrigin::SubmissionFailure, StopAttempt::Accepted))
    );
    assert_eq!(report.facts().submission, Submission::PossiblySubmitted);
}

#[test]
fn a_missing_terminal_remains_unknown_after_detach_and_disposal() {
    let mut port = port_with(FakeBackend::new());
    request(&mut port, 1).unwrap();
    assert!(port.begin_retirement().is_some());
    port.backend.emit(Callback::Finish);
    assert_eq!(port.poll(), None);
    port.shutdown();
    let report = port.take_retirement_report().unwrap();
    assert_eq!(report.facts().terminal, Terminal::Unresolved);
    assert_eq!(report.facts().detach, Detach::VerifiedNil);
    assert_eq!(port.backend.drops.get(), 1);
    assert!(!report.can_abandon_unsubmitted());
}

#[test]
fn partial_attach_failure_reports_never_submitted_with_independent_detach_disposition() {
    for disposition in [
        Detach::VerifiedNil,
        Detach::SetterFailed,
        Detach::ReadbackUnknown,
    ] {
        let mut backend = FakeBackend::new();
        backend.attach_result = Err(Error::NativeFailure);
        backend.detach_result = disposition;
        let mut port = port_with(backend);
        assert_eq!(request(&mut port, 1), Err(Error::NativeFailure));
        let report = port.take_retirement_report().unwrap();
        assert_eq!(report.facts().attachment, Attachment::MayHaveAttached);
        assert_eq!(report.facts().submission, Submission::NeverSubmitted);
        assert_eq!(report.facts().detach, disposition);
        assert_eq!(
            report.can_abandon_unsubmitted(),
            disposition == Detach::VerifiedNil
        );
        assert_eq!(port.backend.drops.get(), 1);
        assert_eq!(
            *port.backend.calls.borrow(),
            [Call::Lookup, Call::Prepare, Call::Attach, Call::Detach]
        );
    }
}

#[test]
fn preparation_expiry_records_attachment_without_any_submission() {
    #[derive(Clone)]
    struct Clock(Rc<std::cell::RefCell<ManualClock>>);
    impl MonotonicClock for Clock {
        fn now(&self) -> MonotonicTime {
            self.0.borrow().now()
        }
    }
    let clock = Clock(Rc::new(std::cell::RefCell::new(ManualClock::new())));
    let admitted = clock.now();
    let scope = scope();
    let binding = scope.bind(identity(1), 1);
    let mut backend = FakeBackend::new();
    let later = clock.clone();
    backend.after_attach = Some(Box::new(move || {
        later
            .0
            .borrow_mut()
            .advance(SPEECH_OUTPUT_DEADLINE)
            .unwrap();
    }));
    let mut port = Port::new(Ok(backend), clock, scope).unwrap();
    assert_eq!(
        request_at(
            binding.clone(),
            1,
            "Synthetic",
            &voice_id(),
            &admitted,
            &mut port
        ),
        Err(Error::Deadline)
    );
    let report = port.take_retirement_report().unwrap();
    assert!(report.matches(&binding));
    assert_eq!(report.facts().attachment, Attachment::Attached);
    assert_eq!(report.facts().submission, Submission::NeverSubmitted);
    assert_eq!(report.facts().detach, Detach::VerifiedNil);
    assert!(!port.backend.calls.borrow().contains(&Call::Speak));
}

#[test]
fn terminal_report_keeps_failed_and_unknown_detachment_distinct() {
    for disposition in [Detach::SetterFailed, Detach::ReadbackUnknown] {
        let mut backend = FakeBackend::new();
        backend.detach_result = disposition;
        let mut port = port_with(backend);
        request(&mut port, 1).unwrap();
        port.backend.emit(Callback::Start);
        port.backend.emit(Callback::Finish);
        port.poll();
        assert_eq!(port.poll(), Some(notice(1, Evidence::Finished)));
        let report = port.take_retirement_report().unwrap();
        assert_eq!(report.facts().terminal, Terminal::Finished);
        assert_eq!(report.facts().detach, disposition);
        assert!(!report.can_abandon_unsubmitted());
        assert_eq!(request(&mut port, 2), Err(Error::Closed));
    }
}

#[test]
fn retirement_report_requires_exact_authority_epoch_route_identity_operation_and_nonce() {
    let mut port = port_with(FakeBackend::new());
    let original = binding(&port, 1);
    let admitted = port.clock.now();
    let mut binding = original.clone();
    binding.scope.authority = Arc::new(());
    let mut wrong_epoch = original.clone();
    wrong_epoch.scope = port.scope.next_owner().unwrap();
    let mut wrong_route = original.clone();
    wrong_route.scope.route = wrong_route.scope.route.checked_next().unwrap();
    let mut wrong_operation = original.clone();
    wrong_operation.operation += 1;
    let mut wrong_identity = original.clone();
    wrong_identity.identity.generation = wrong_identity.identity.generation.checked_next().unwrap();
    let wrong_nonce = port.scope.bind(identity(1), original.operation);
    for foreign in [&binding, &wrong_epoch, &wrong_route] {
        assert_eq!(
            request_at(
                foreign.clone(),
                1,
                "synthetic",
                &voice_id(),
                &admitted,
                &mut port
            ),
            Err(Error::StaleJob)
        );
    }
    assert!(port.backend.calls.borrow().is_empty());

    request_at(
        original.clone(),
        1,
        "synthetic",
        &voice_id(),
        &admitted,
        &mut port,
    )
    .unwrap();
    port.shutdown();
    let report = port.take_retirement_report().unwrap();
    for foreign in [
        binding,
        wrong_epoch,
        wrong_route,
        wrong_operation,
        wrong_identity,
        wrong_nonce,
    ] {
        assert!(!report.matches(&foreign));
    }
    assert!(report.matches(&original));
    assert!(port.take_retirement_report().is_none());
}

fn authority_for(binding: &FlightBinding, deadline: &Deadline) -> LocalAuthorityState {
    let authority = LocalAuthorityState::default();
    authority.borrow_mut().scope = Some(binding.identity());
    authority.borrow_mut().grants[0] = Some(LocalSpeechGrant {
        identity: binding.identity(),
        deadline: deadline.clone(),
        speaker: true,
        source: true,
    });
    authority
}

#[test]
fn stop_at_every_preparation_stage_denies_native_dispatch_and_records_detach() {
    for phase in [
        Call::Lookup,
        Call::Prepare,
        Call::Attach,
        Call::BeforeDispatch,
    ] {
        for detach in [
            Detach::VerifiedNil,
            Detach::ReadbackUnknown,
            Detach::SetterFailed,
        ] {
            let mut backend = FakeBackend::new();
            backend.detach_result = detach;
            let revoked = Rc::new(Cell::new(false));
            let during = revoked.clone();
            backend
                .phase_hook
                .replace(Some((phase, Box::new(move || during.set(true)))));
            let mut port = port_with(backend);
            let binding = binding(&port, 1);
            let now = port.clock.now();
            let deadline = Deadline::after(now.clone(), SPEECH_OUTPUT_DEADLINE).unwrap();
            let authority = authority_for(&binding, &deadline);
            let obligation = RefCell::new(Some(binding.clone()));
            let check = SubmissionCheck::new(&binding, &authority, &revoked, &now, [&deadline; 2])
                .with_obligation(&obligation);
            assert_eq!(
                port.request(binding.clone(), 1, "Synthetic", &voice_id(), &now, check),
                Err(Error::Unauthorized)
            );
            assert_eq!(
                *port.backend.calls.borrow(),
                [Call::Lookup, Call::Prepare, Call::Attach, Call::Detach]
            );
            let report = port.take_retirement_report().unwrap();
            assert!(report.matches(&binding));
            assert_eq!(report.facts().submission, Submission::NeverSubmitted);
            assert_eq!(report.facts().first_stop, None);
            assert_eq!(report.facts().detach, detach);
            assert_eq!(
                report.can_abandon_unsubmitted(),
                detach == Detach::VerifiedNil
            );
        }
    }
}

#[test]
fn final_check_rejects_missing_busy_foreign_or_replayed_authority() {
    let mut port = port_with(FakeBackend::new());
    let expected = binding(&port, 1);
    let now = port.clock.now();
    let deadline = Deadline::after(now.clone(), SPEECH_OUTPUT_DEADLINE).unwrap();
    let authority = authority_for(&expected, &deadline);
    let revoked = Cell::new(false);
    let obligation = RefCell::new(Some(expected.clone()));
    let make = || SubmissionCheck::new(&expected, &authority, &revoked, &now, [&deadline; 2]);

    assert_eq!(
        port.request(expected.clone(), 1, "Synthetic", &voice_id(), &now, make()),
        Err(Error::Unauthorized)
    );
    assert_eq!(
        port.take_retirement_report().unwrap().facts().submission,
        Submission::NeverSubmitted
    );
    let missing = RefCell::new(None);
    assert_eq!(
        make().with_obligation(&missing).check(&expected, &now),
        Err(Error::Unauthorized)
    );

    let foreign = OutputScope::new(
        expected.identity().app_run_id,
        Generation::from_native_sequence(1).unwrap(),
    )
    .bind(expected.identity(), 1);
    assert_eq!(
        make().with_obligation(&obligation).check(&foreign, &now),
        Err(Error::Unauthorized)
    );
    let wrong_obligation = RefCell::new(Some(foreign));
    assert_eq!(
        make()
            .with_obligation(&wrong_obligation)
            .check(&expected, &now),
        Err(Error::Unauthorized)
    );
    {
        let _held = authority.borrow_mut();
        assert_eq!(
            make().with_obligation(&obligation).check(&expected, &now),
            Err(Error::Busy)
        );
    }
    {
        let _held = obligation.borrow_mut();
        assert_eq!(
            make().with_obligation(&obligation).check(&expected, &now),
            Err(Error::Busy)
        );
    }
    let mut once = make().with_obligation(&obligation);
    assert_eq!(once.check(&expected, &now), Ok(()));
    assert_eq!(once.check(&expected, &now), Err(Error::Unauthorized));
}

#[test]
fn actual_dispatch_consumes_one_check_and_stop_inside_a_throw_keeps_the_first_attempt() {
    let mut backend = FakeBackend::new();
    let revoked = Rc::new(Cell::new(false));
    let during = revoked.clone();
    backend
        .phase_hook
        .replace(Some((Call::Speak, Box::new(move || during.set(true)))));
    backend.during_speak = Some(Callback::Start);
    backend.during_stop = Some(Callback::Cancel);
    backend.speak_result = Err(Error::NativeFailure);
    let mut port = port_with(backend);
    let binding = binding(&port, 1);
    let now = port.clock.now();
    let deadline = Deadline::after(now.clone(), SPEECH_OUTPUT_DEADLINE).unwrap();
    let authority = authority_for(&binding, &deadline);
    let obligation = RefCell::new(Some(binding.clone()));
    let check = SubmissionCheck::new(&binding, &authority, &revoked, &now, [&deadline; 2])
        .with_obligation(&obligation);
    assert_eq!(
        port.request(binding.clone(), 1, "Synthetic", &voice_id(), &now, check),
        Ok(notice(1, Evidence::Failure(Error::NativeFailure)))
    );
    assert!(revoked.get());
    assert_eq!(
        port.stop(binding.identity(), StopOrigin::User),
        Ok(notice(
            1,
            Evidence::CancellationUnconfirmed(StopAttempt::Accepted)
        ))
    );
    assert_eq!(
        port.poll(),
        Some(notice(
            1,
            Evidence::Cancelled {
                stop_requested: true
            }
        ))
    );
    let report = port.take_retirement_report().unwrap();
    assert_eq!(report.facts().submission, Submission::PossiblySubmitted);
    assert_eq!(
        report.facts().first_stop,
        Some((StopOrigin::SubmissionFailure, StopAttempt::Accepted))
    );
    assert!(!report.can_abandon_unsubmitted());
    assert_eq!(
        *port.backend.calls.borrow(),
        [
            Call::Lookup,
            Call::Prepare,
            Call::Attach,
            Call::Speak,
            Call::Stop,
            Call::Detach
        ]
    );
}

#[test]
fn final_authority_is_read_after_attachment_and_deadlines_never_extend() {
    for scenario in [
        "speaker",
        "source",
        "consent",
        "configuration",
        "removed",
        "shorter",
        "original grant",
        "duplicate",
    ] {
        let mut backend = FakeBackend::new();
        let scope = scope();
        let binding = scope.bind(identity(1), 1);
        let clock = ManualClock::new();
        let now = clock.now();
        let output = Deadline::after(now.clone(), SPEECH_OUTPUT_DEADLINE).unwrap();
        let expired = Deadline::after(now.clone(), Duration::ZERO).unwrap();
        let authority = Rc::new(authority_for(&binding, &output));
        let during = authority.clone();
        let edge = expired.clone();
        backend.after_attach = Some(Box::new(move || {
            let mut facts = during.borrow_mut();
            let mut grant = facts.grants[0].clone().unwrap();
            match scenario {
                "speaker" => grant.speaker = false,
                "source" => grant.source = false,
                "consent" => {
                    facts.scope.as_mut().unwrap().consent_binding_id =
                        ConsentBindingId::from_native_sequence(2).unwrap()
                }
                "configuration" => {
                    facts.scope.as_mut().unwrap().configuration_revision =
                        ConfigurationRevision::from_native_sequence(2).unwrap()
                }
                "removed" => {
                    facts.grants[0] = None;
                    return;
                }
                "shorter" => grant.deadline = edge,
                "duplicate" => facts.grants[1] = Some(grant.clone()),
                _ => (),
            }
            facts.grants[0] = Some(grant);
        }));
        let granted = if scenario == "original grant" {
            &expired
        } else {
            &output
        };
        let mut port = Port::new(Ok(backend), clock, scope).unwrap();
        let obligation = RefCell::new(Some(binding.clone()));
        let stop = Cell::new(false);
        let check = SubmissionCheck::new(&binding, &authority, &stop, &now, [&output, granted])
            .with_obligation(&obligation);
        let expected = if matches!(scenario, "shorter" | "original grant") {
            Error::Deadline
        } else {
            Error::Unauthorized
        };
        assert_eq!(
            port.request(binding.clone(), 1, "Synthetic", &voice_id(), &now, check),
            Err(expected),
            "{scenario}"
        );
        assert!(!port.backend.calls.borrow().contains(&Call::Speak));
        assert_eq!(
            port.take_retirement_report().unwrap().facts().submission,
            Submission::NeverSubmitted
        );
    }
}

#[test]
fn final_clock_read_denies_stop_rollback_foreign_domain_and_exact_expiry() {
    struct EdgeClock {
        before: MonotonicTime,
        edge: MonotonicTime,
        at_edge: Rc<Cell<bool>>,
        stop_on_edge: Option<Rc<Cell<bool>>>,
    }
    impl MonotonicClock for EdgeClock {
        fn now(&self) -> MonotonicTime {
            if self.at_edge.get() {
                if let Some(stop) = &self.stop_on_edge {
                    stop.set(true);
                }
                self.edge.clone()
            } else {
                self.before.clone()
            }
        }
    }
    for scenario in ["stop", "rollback", "foreign", "expiry"] {
        let initial = ManualClock::new().now();
        let now = initial.checked_add(Duration::from_secs(1)).unwrap();
        let edge = match scenario {
            "rollback" => initial,
            "foreign" => ManualClock::new().now(),
            "expiry" => now.checked_add(SPEECH_OUTPUT_DEADLINE).unwrap(),
            _ => now.clone(),
        };
        let revoked = Rc::new(Cell::new(false));
        let at_edge = Rc::new(Cell::new(false));
        let hook = at_edge.clone();
        let mut backend = FakeBackend::new();
        backend.after_attach = Some(Box::new(move || hook.set(true)));
        let clock = EdgeClock {
            before: now.clone(),
            edge,
            at_edge,
            stop_on_edge: (scenario == "stop").then(|| revoked.clone()),
        };
        let mut port = Port::new(Ok(backend), clock, scope()).unwrap();
        let binding = port.scope.bind(identity(1), 1);
        let deadline = Deadline::after(now.clone(), SPEECH_OUTPUT_DEADLINE).unwrap();
        let authority = authority_for(&binding, &deadline);
        let obligation = RefCell::new(Some(binding.clone()));
        let check = SubmissionCheck::new(&binding, &authority, &revoked, &now, [&deadline; 2])
            .with_obligation(&obligation);
        let error = match scenario {
            "stop" => Error::Unauthorized,
            "rollback" => Error::Contract(ContractError::TimeReversed),
            "foreign" => Error::Contract(ContractError::ClockDomainMismatch),
            _ => Error::Deadline,
        };
        assert_eq!(
            port.request(binding.clone(), 1, "Synthetic", &voice_id(), &now, check),
            Err(error)
        );
        assert!(!port.backend.calls.borrow().contains(&Call::Speak));
        assert_eq!(
            port.take_retirement_report().unwrap().facts().submission,
            Submission::NeverSubmitted
        );
    }
}
