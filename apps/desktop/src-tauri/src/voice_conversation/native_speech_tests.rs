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
    detach_result: Result<()>,
    drops: Rc<Cell<usize>>,
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
            detach_result: Ok(()),
            drops: Rc::new(Cell::new(0)),
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
        Ok(self.selected.clone().map(|id| (id, ())))
    }

    fn prepare(
        &self,
        _voice: Self::Voice,
        _text: &str,
    ) -> Result<(Self::Utterance, NativeObjects)> {
        self.calls.borrow_mut().push(Call::Prepare);
        Ok((FakeUtterance(self.drops.clone()), self.objects))
    }

    fn attach(&mut self, inbox: Arc<Inbox>) -> Result<()> {
        self.calls.borrow_mut().push(Call::Attach);
        assert!(self.inbox.replace(inbox).is_none());
        if let Some(event) = self.during_attach {
            self.emit(event);
        }
        Ok(())
    }

    fn speak(&self, _utterance: &Self::Utterance) -> Result<()> {
        self.calls.borrow_mut().push(Call::Speak);
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
        self.stop_result
    }

    fn detach(&mut self) -> Result<()> {
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

fn port_with(backend: FakeBackend) -> FakePort {
    Port::new(Ok(backend), ManualClock::new(), identity(1).app_run_id).unwrap()
}

fn request(port: &mut FakePort, sequence: u64) -> Result<Notice> {
    port.request(
        identity(sequence),
        sequence,
        "Synthetic text",
        &voice_id(),
        &port.clock.now(),
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
            FakePort::new(Err(error), ManualClock::new(), identity(1).app_run_id),
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
            port.request(identity(1), 1, invalid, &voice_id(), &admitted),
            Err(Error::InvalidText)
        );
    }
    assert_eq!(
        port.request(
            identity(1),
            1,
            &(limit.clone() + "é"),
            &voice_id(),
            &admitted
        ),
        Err(Error::Contract(ContractError::LimitExceeded))
    );
    assert!(port.backend.calls.borrow().is_empty());
    assert_eq!(
        port.request(identity(1), 1, &limit, &voice_id(), &admitted),
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
    assert_eq!(port.stop(bad_generation.identity), Err(Error::StaleJob));
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
    assert_eq!(port.stop(identity(1)), Ok(unconfirmed));
    port.backend.emit(Callback::Start);
    port.backend.emit(Callback::Finish);
    assert_eq!(port.poll(), None);
    assert_eq!(port.stop(identity(1)), Ok(unconfirmed));
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
        port.stop(identity(1)),
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
    port.stop(identity(1)).unwrap();
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
        assert_eq!(port.stop(identity(1)), Ok(unconfirmed));
        assert_eq!(port.poll(), None);
        assert_eq!(port.stop(identity(1)), Ok(unconfirmed));
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
        port.request(identity(1), 1, "Test", &voice_id(), &admitted),
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
        port.request(identity(1), 1, "Test", &voice_id(), &admitted),
        Err(Error::Deadline)
    );
    assert_eq!(
        port.request(
            identity(1),
            1,
            "Test",
            &voice_id(),
            &ManualClock::new().now()
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
    backend.detach_result = Err(Error::NativeFailure);
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
    type Open = fn(MainThreadMarker, ManualClock, AppRunId) -> Result<Box<dyn SpeechOutput>>;
    let open_port: Open =
        |marker, clock, run| Ok(Box::new(open(marker, clock, run)?) as Box<dyn SpeechOutput>);
    let _ = black_box(open_port);
    let _ = black_box(installed_voices as fn() -> Result<Vec<Voice>>);
}
