use super::*;
use crate::voice_conversation::clock::ManualClock;
use crate::voice_conversation::limits::{
    MAX_QUEUED_SPEECH_TEXT_BYTES, MAX_SPEECH_JOB_TEXT_BYTES, SPEECH_OUTPUT_DEADLINE,
};
use crate::voice_conversation::model::*;
use crate::voice_conversation::native_speech::StopAttempt;
use crate::voice_conversation::ContractError;
use native_speech::{LocalAuthorityFacts, LocalSpeechGrant};
use std::cell::{Cell, RefCell};
use std::cmp::Ordering;
use std::collections::VecDeque;
use std::rc::Rc;

type NativeResult<T> = std::result::Result<T, native_speech::Error>;

#[derive(Clone)]
struct Clock {
    time: Rc<RefCell<MonotonicTime>>,
    reads: Rc<Cell<usize>>,
}

impl Clock {
    fn new() -> Self {
        Self {
            time: Rc::new(RefCell::new(ManualClock::new().now())),
            reads: Rc::new(Cell::new(0)),
        }
    }

    fn advance(&self, amount: Duration) {
        let next = self.time.borrow().checked_add(amount).unwrap();
        self.time.replace(next);
    }
}

impl MonotonicClock for Clock {
    fn now(&self) -> MonotonicTime {
        self.reads.set(self.reads.get() + 1);
        self.time.borrow().clone()
    }
}

struct Request {
    identity: ContractIdentity,
    sequence: u64,
    bytes: usize,
    voice: VoiceId,
    admitted: MonotonicTime,
}

struct Script {
    requests: Vec<Request>,
    stops: Vec<ContractIdentity>,
    polls: usize,
    shutdowns: usize,
    notices: VecDeque<Notice>,
    submission: NativeResult<Evidence>,
    stop: NativeResult<Evidence>,
    during_request: Option<Evidence>,
    on_prepare: Option<Box<dyn FnOnce()>>,
    request_clock: Option<Clock>,
    during_stop: Option<Evidence>,
    at_shutdown: Option<Notice>,
    after_poll: Option<Box<dyn FnOnce()>>,
    #[cfg(target_os = "macos")]
    on_shutdown: Option<Box<dyn FnOnce()>>,
    #[cfg(target_os = "macos")]
    on_drop: Option<Box<dyn FnOnce()>>,
}

impl Default for Script {
    fn default() -> Self {
        Self {
            requests: Vec::new(),
            stops: Vec::new(),
            polls: 0,
            shutdowns: 0,
            notices: VecDeque::new(),
            submission: Ok(Evidence::Requested),
            stop: Ok(Evidence::CancellationUnconfirmed(StopAttempt::Accepted)),
            during_request: None,
            on_prepare: None,
            request_clock: None,
            during_stop: None,
            at_shutdown: None,
            after_poll: None,
            #[cfg(target_os = "macos")]
            on_shutdown: None,
            #[cfg(target_os = "macos")]
            on_drop: None,
        }
    }
}

impl SpeechOutput for Script {
    fn request(
        &mut self,
        binding: native_speech::FlightBinding,
        sequence: u64,
        text: &str,
        voice: &VoiceId,
        admitted: &MonotonicTime,
        check: SubmissionCheck<'_>,
    ) -> NativeResult<Notice> {
        if let Some(hook) = self.on_prepare.take() {
            hook();
        }
        let obligation = RefCell::new(Some(binding.clone()));
        let now = self
            .request_clock
            .as_ref()
            .map_or_else(|| admitted.clone(), MonotonicClock::now);
        check.with_obligation(&obligation).check(&binding, &now)?;
        let identity = binding.identity();
        self.requests.push(Request {
            identity,
            sequence,
            bytes: text.len(),
            voice: voice.clone(),
            admitted: admitted.clone(),
        });
        if let Some(evidence) = self.during_request.take() {
            self.notices.push_back(Notice { identity, evidence });
        }
        self.submission
            .map(|evidence| Notice { identity, evidence })
    }

    fn stop(&mut self, identity: ContractIdentity, _origin: StopOrigin) -> NativeResult<Notice> {
        self.stops.push(identity);
        if let Some(evidence) = self.during_stop.take() {
            self.notices.push_back(Notice { identity, evidence });
        }
        self.stop.map(|evidence| Notice { identity, evidence })
    }

    fn poll(&mut self) -> Option<Notice> {
        self.polls += 1;
        let next = self.notices.pop_front();
        if let Some(action) = self.after_poll.take() {
            action();
        }
        next
    }

    fn shutdown(&mut self) -> Option<Notice> {
        self.shutdowns += 1;
        #[cfg(target_os = "macos")]
        if let Some(action) = self.on_shutdown.take() {
            action();
        }
        self.at_shutdown.take()
    }
}

#[cfg(target_os = "macos")]
impl Drop for Script {
    fn drop(&mut self) {
        if let Some(action) = self.on_drop.take() {
            action();
        }
    }
}

type Gate = Box<dyn Fn(ContractIdentity) -> Option<Validation>>;
type TestCoordinator = Coordinator<Script, Clock, Gate>;

struct Fixture {
    coordinator: TestCoordinator,
    policies: LocalAuthority,
    clock: Clock,
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

fn voice() -> VoiceId {
    VoiceId::new("Selected.é").unwrap()
}

fn notice(sequence: u64, evidence: Evidence) -> Notice {
    Notice {
        identity: identity(sequence),
        evidence,
    }
}

impl Fixture {
    fn new() -> Self {
        let clock = Clock::new();
        let policies = LocalAuthority::default();
        policies.replace(LocalAuthorityFacts {
            scope: Some(identity(1)),
            grants: std::array::from_fn(|index| {
                (index < 4).then(|| LocalSpeechGrant {
                    identity: identity(index as u64 + 1),
                    deadline: Deadline::after(clock.now(), Duration::from_secs(300)).unwrap(),
                    speaker: true,
                    source: true,
                })
            }),
        });
        let source = policies.clone();
        let validate: Gate = Box::new(move |job| {
            let index = (1..=4).position(|sequence| identity(sequence) == job)?;
            let policy = source.borrow().grants[index].clone()?;
            Some(Validation {
                identity: job,
                role: ProviderRole::Tts,
                deadline: policy.deadline,
                source_eligible: policy.source,
                speaker_allowed: policy.speaker,
                cloud_tts_allowed: false,
            })
        });
        let scope = identity(1);
        let mut script = Script::default();
        script.request_clock = Some(clock.clone());
        let mut coordinator = Coordinator::new(
            script,
            clock.clone(),
            validate,
            policies.clone(),
            scope.app_run_id,
            scope.interaction_id,
        )
        .unwrap();
        coordinator.emergency = Some(Rc::new(StopLatch::new(
            coordinator.output_scope.clone(),
            scope,
        )));
        Self {
            coordinator,
            policies,
            clock,
        }
    }

    fn admit(&mut self, sequence: u64) {
        self.coordinator
            .admit(
                identity(sequence),
                sequence,
                "Synthetic text".into(),
                voice(),
            )
            .unwrap();
    }

    fn request(&mut self, sequence: u64) {
        self.admit(sequence);
        assert_eq!(
            self.coordinator.submit(),
            Ok(notice(sequence, Evidence::Requested))
        );
    }

    fn push(&mut self, sequence: u64, evidence: Evidence) {
        self.coordinator
            .port
            .notices
            .push_back(notice(sequence, evidence));
    }

    fn wake(&mut self) -> Result<Notices> {
        let wake = self.coordinator.wake.clone().expect("expected a wake");
        self.coordinator.wake(&wake)
    }

    fn deadline(&self, sequence: usize, delay: Duration) {
        let at = Deadline::after(self.clock.now(), delay).unwrap();
        self.policies.borrow_mut().grants[sequence - 1]
            .as_mut()
            .unwrap()
            .deadline = at;
    }
}

#[test]
fn only_native_start_and_finish_move_the_phase_one_state() {
    let mut f = Fixture::new();
    f.request(1);
    assert_eq!(f.coordinator.logical.state(), SpeechState::Preparing);
    assert_eq!(f.wake(), Ok([None, None]));
    f.push(1, Evidence::Started);
    assert_eq!(f.wake(), Ok([Some(notice(1, Evidence::Started)), None]));
    assert_eq!(f.coordinator.logical.state(), SpeechState::Speaking);
    f.push(1, Evidence::Finished);
    assert_eq!(f.wake(), Ok([Some(notice(1, Evidence::Finished)), None]));
    assert_eq!(f.coordinator.logical.state(), SpeechState::Idle);
    assert_eq!(f.coordinator.logical.capacity.usage(), (0, 0, 0));
    assert_eq!(f.coordinator.port.requests.len(), 1);
    assert!(f.coordinator.port.stops.is_empty());
    assert!(f.coordinator.wake.is_none());
    assert_eq!(f.coordinator.cleanup, Cleanup::Unresolved);
}

#[test]
fn inline_start_is_processed_on_a_later_turn_and_drain_is_bounded() {
    let mut f = Fixture::new();
    f.coordinator.port.during_request = Some(Evidence::Started);
    f.request(1);
    f.push(1, Evidence::Started);
    f.push(1, Evidence::Finished);
    assert_eq!(f.coordinator.logical.state(), SpeechState::Preparing);
    assert_eq!(f.wake(), Ok([Some(notice(1, Evidence::Started)), None]));
    assert_eq!(f.coordinator.port.polls, 2);
    assert_eq!(f.wake(), Ok([Some(notice(1, Evidence::Finished)), None]));
    assert_eq!(f.coordinator.port.polls, 3);
}

#[test]
fn finish_before_start_and_foreign_identity_do_not_advance_authority() {
    let mut f = Fixture::new();
    f.request(1);
    f.admit(2);
    let old = f.coordinator.flight.as_ref().unwrap().binding.clone();
    let mut foreign = notice(1, Evidence::Started);
    foreign.identity.generation = Generation::from_native_sequence(2).unwrap();
    f.coordinator.port.notices.push_back(foreign);
    f.push(1, Evidence::Finished);
    assert_eq!(f.wake(), Ok([None, None]));
    assert!(f.coordinator.logical.matches(&old));
    assert_eq!(f.coordinator.logical.state(), SpeechState::Preparing);
    assert_eq!(f.coordinator.logical.capacity.usage().1, 1);
    f.push(2, Evidence::Started);
    f.push(1, Evidence::Started);
    assert_eq!(f.wake(), Ok([None, Some(notice(1, Evidence::Started))]));
    assert!(!f.coordinator.logical.matches(&old));
}

#[test]
fn old_notice_and_wake_cannot_affect_a_promoted_flight() {
    let mut f = Fixture::new();
    f.request(1);
    f.admit(2);
    let old_wake = f.coordinator.wake.clone().unwrap();
    f.push(1, Evidence::Started);
    f.push(1, Evidence::Finished);
    assert_eq!(
        f.wake(),
        Ok([
            Some(notice(1, Evidence::Started)),
            Some(notice(1, Evidence::Finished))
        ])
    );
    assert_eq!(f.coordinator.logical.state(), SpeechState::Preparing);
    assert_eq!(f.coordinator.port.requests.len(), 1);
    assert_eq!(f.coordinator.submit(), Ok(notice(2, Evidence::Requested)));
    f.push(1, Evidence::Finished);
    f.push(2, Evidence::Started);
    let polls = f.coordinator.port.polls;
    assert_eq!(f.coordinator.wake(&old_wake), Ok([None, None]));
    assert_eq!(f.coordinator.port.polls, polls);
    assert_eq!(f.wake(), Ok([None, Some(notice(2, Evidence::Started))]));
}

#[test]
fn stop_revokes_before_poll_and_discards_buffered_finish_and_foreign_cancel() {
    let mut f = Fixture::new();
    f.request(1);
    f.admit(2);
    f.push(1, Evidence::Started);
    f.wake().unwrap();
    f.push(1, Evidence::Finished);
    f.push(
        2,
        Evidence::Cancelled {
            stop_requested: true,
        },
    );
    let stale = f.coordinator.wake.clone().unwrap();
    let old_epoch = f.coordinator.logical.epoch;
    let reads = f.clock.reads.get();
    let attempt = notice(1, Evidence::CancellationUnconfirmed(StopAttempt::Accepted));

    assert_eq!(f.coordinator.stop(), Ok(Some(attempt)));
    assert_eq!(f.coordinator.stop(), Ok(Some(attempt)));
    assert_eq!(f.clock.reads.get(), reads);
    assert_ne!(f.coordinator.logical.epoch, old_epoch);
    assert_eq!(f.coordinator.logical.capacity.usage(), (0, 0, 0));
    assert!(f.coordinator.admissions.iter().all(Option::is_none));
    assert_eq!(f.coordinator.wake(&stale), Ok([None, None]));
    assert_eq!(f.wake(), Ok([None, None]));
    assert!(f.coordinator.cancellation.is_none());

    let cancel = Evidence::Cancelled {
        stop_requested: true,
    };
    f.push(1, cancel);
    assert_eq!(f.wake(), Ok([Some(notice(1, cancel)), None]));
    assert_eq!(f.coordinator.cancellation, Some(notice(1, cancel)));
    assert_eq!(f.coordinator.cleanup, Cleanup::Unresolved);
    assert_eq!(f.coordinator.submit(), Err(Error::Closed));
    assert_eq!(f.coordinator.port.stops, [identity(1)]);
    assert_eq!(f.coordinator.port.requests.len(), 1);
}

#[test]
fn synchronous_stop_callback_still_requires_polling() {
    let mut f = Fixture::new();
    f.request(1);
    let cancellation = Evidence::Cancelled {
        stop_requested: true,
    };
    f.coordinator.port.during_stop = Some(cancellation);
    assert!(matches!(
        f.coordinator.stop(),
        Ok(Some(Notice {
            evidence: Evidence::CancellationUnconfirmed(StopAttempt::Accepted),
            ..
        }))
    ));
    assert!(f.coordinator.cancellation.is_none());
    assert_eq!(f.wake(), Ok([Some(notice(1, cancellation)), None]));
    assert_eq!(f.coordinator.cleanup, Cleanup::Unresolved);
}

#[test]
fn unsolicited_cancellation_is_terminal_but_cannot_acknowledge_stop() {
    let cancellation = Evidence::Cancelled {
        stop_requested: false,
    };
    for stop_first in [false, true] {
        let mut f = Fixture::new();
        f.request(1);
        f.admit(2);
        f.push(1, cancellation);
        if stop_first {
            f.coordinator.stop().unwrap();
        }
        assert_eq!(f.wake(), Ok([Some(notice(1, cancellation)), None]));
        assert_eq!(f.coordinator.cancellation, Some(notice(1, cancellation)));
        assert_eq!(f.coordinator.cleanup, Cleanup::Unresolved);
        assert!(f.coordinator.wake.is_none());
        assert_eq!(f.coordinator.logical.capacity.usage(), (0, 0, 0));
        assert_eq!(f.coordinator.submit(), Err(Error::Closed));
        assert_eq!(f.coordinator.port.stops.len(), usize::from(stop_first));
    }
}

#[test]
fn rejected_and_failed_stop_stay_blocked_when_observation_budget_runs_out() {
    for attempt in [
        StopAttempt::Accepted,
        StopAttempt::Rejected,
        StopAttempt::Failed,
    ] {
        let mut f = Fixture::new();
        f.request(1);
        f.coordinator.port.stop = Ok(Evidence::CancellationUnconfirmed(attempt));
        assert_eq!(
            f.coordinator.stop(),
            Ok(Some(notice(1, Evidence::CancellationUnconfirmed(attempt))))
        );
        for _ in 0..MAX_STOP_POLLS {
            assert_eq!(f.wake(), Ok([None, None]));
        }
        assert!(f.coordinator.wake.is_none());
        assert!(f.coordinator.cancellation.is_none());
        assert!(f.coordinator.flight.is_some());
        assert_eq!(f.coordinator.cleanup, Cleanup::Unresolved);
        assert_eq!(f.coordinator.port.polls, MAX_STOP_POLLS);
        assert_eq!(f.coordinator.port.stops, [identity(1)]);
        assert_eq!(f.coordinator.submit(), Err(Error::Closed));
        f.coordinator.shutdown().unwrap();
        assert_eq!(f.coordinator.cleanup, Cleanup::Unresolved);
    }
}

#[test]
fn stop_dispatch_error_does_not_clear_the_native_flight() {
    let mut f = Fixture::new();
    f.request(1);
    f.coordinator.port.stop = Err(native_speech::Error::NativeFailure);
    assert_eq!(
        f.coordinator.stop(),
        Err(Error::Native(native_speech::Error::NativeFailure))
    );
    assert_eq!(f.coordinator.stop(), Ok(None));
    assert!(f.coordinator.flight.is_some());
    assert_eq!(f.wake(), Ok([None, None]));
    assert_eq!(f.coordinator.cleanup, Cleanup::Unresolved);
    assert_eq!(f.coordinator.port.stops, [identity(1)]);
}

#[test]
fn stop_return_value_cannot_supply_a_cancellation_receipt() {
    let mut f = Fixture::new();
    f.request(1);
    f.coordinator.port.stop = Ok(Evidence::Cancelled {
        stop_requested: true,
    });
    assert_eq!(f.coordinator.stop(), Err(Error::Protocol));
    assert_eq!(f.wake(), Ok([None, None]));
    assert!(f.coordinator.cancellation.is_none());
    assert!(f.coordinator.flight.is_some());
    assert_eq!(f.coordinator.cleanup, Cleanup::Unresolved);
    assert_eq!(f.coordinator.submit(), Err(Error::Closed));
}

#[test]
fn submission_failure_notice_retains_cleanup_binding_but_never_retries() {
    let mut f = Fixture::new();
    f.admit(1);
    f.admit(2);
    let failure = Evidence::Failure(native_speech::Error::NativeFailure);
    f.coordinator.port.submission = Ok(failure);
    assert_eq!(f.coordinator.submit(), Ok(notice(1, failure)));
    assert_eq!(f.coordinator.logical.capacity.usage(), (0, 0, 0));
    assert_eq!(f.coordinator.port.stops, [identity(1)]);
    assert_eq!(f.coordinator.submit(), Err(Error::Closed));
    assert!(f.coordinator.flight.is_some());
    assert_eq!(f.coordinator.cleanup, Cleanup::Unresolved);
    assert_eq!(f.coordinator.port.requests.len(), 1);
}

#[test]
fn unavailable_voice_and_closed_port_fail_without_fallback_or_cleanup_claim() {
    for error in [
        native_speech::Error::VoiceUnavailable,
        native_speech::Error::Closed,
    ] {
        let mut f = Fixture::new();
        f.request(1);
        f.admit(2);
        f.push(1, Evidence::Started);
        f.push(1, Evidence::Finished);
        f.wake().unwrap();
        // The previous finish does not reveal a failed delegate detach. The
        // real port returns Closed on the next explicit submission.
        f.coordinator.port.submission = Err(error);
        assert_eq!(f.coordinator.submit(), Err(Error::Native(error)));
        assert_eq!(f.coordinator.submit(), Err(Error::Closed));
        assert_eq!(f.coordinator.port.requests.len(), 2);
        assert!(f.coordinator.port.requests[1].voice == voice());
        assert_eq!(f.coordinator.cleanup, Cleanup::Unresolved);
        assert!(f.coordinator.port.stops.is_empty());
    }
}

#[test]
fn shutdown_invalidates_timer_and_does_not_infer_cleanup_from_a_finished_notice() {
    let mut f = Fixture::new();
    assert_eq!(f.coordinator.cleanup, Cleanup::NoNativeWork);
    f.request(1);
    let stale = f.coordinator.wake.clone().unwrap();
    f.coordinator.port.at_shutdown = Some(notice(1, Evidence::Finished));
    f.coordinator.shutdown().unwrap();
    f.coordinator.shutdown().unwrap();
    assert_eq!(f.coordinator.wake(&stale), Ok([None, None]));
    assert_eq!(f.coordinator.port.polls, 0);
    assert_eq!(f.coordinator.port.shutdowns, 1);
    assert_eq!(f.coordinator.port.stops, [identity(1)]);
    assert!(f.coordinator.cancellation.is_none());
    assert_eq!(f.coordinator.cleanup, Cleanup::Unresolved);
    assert_eq!(f.coordinator.submit(), Err(Error::Closed));
}

#[test]
fn admission_samples_once_and_expiry_is_exact_before_submission() {
    let epsilon = Duration::from_nanos(1);
    for elapsed in [
        SPEECH_OUTPUT_DEADLINE - epsilon,
        SPEECH_OUTPUT_DEADLINE,
        SPEECH_OUTPUT_DEADLINE + epsilon,
    ] {
        let mut f = Fixture::new();
        let original = f.clock.now();
        let reads = f.clock.reads.get();
        f.admit(1);
        assert_eq!(f.clock.reads.get(), reads + 1);
        f.clock.advance(elapsed);
        if elapsed < SPEECH_OUTPUT_DEADLINE {
            assert_eq!(f.coordinator.submit(), Ok(notice(1, Evidence::Requested)));
            let request = &f.coordinator.port.requests[0];
            assert_eq!(request.admitted.checked_cmp(&original), Ok(Ordering::Equal));
            assert_eq!(f.coordinator.wake.as_ref().unwrap().delay, epsilon);
        } else {
            assert_eq!(f.coordinator.submit(), Err(Error::Deadline));
            assert!(f.coordinator.port.requests.is_empty());
        }
    }
}

#[test]
fn exact_deadline_wins_over_buffered_finish_and_does_not_restore_authority() {
    let mut f = Fixture::new();
    f.request(1);
    f.push(1, Evidence::Started);
    f.wake().unwrap();
    f.clock.advance(SPEECH_OUTPUT_DEADLINE);
    f.push(1, Evidence::Finished);
    let polls = f.coordinator.port.polls;
    assert_eq!(f.wake(), Err(Error::Deadline));
    assert_eq!(f.coordinator.port.polls, polls);
    assert_eq!(f.wake(), Ok([None, None]));
    assert_eq!(f.coordinator.port.stops, [identity(1)]);
    assert_eq!(f.coordinator.cleanup, Cleanup::Unresolved);
}

#[test]
fn pending_job_keeps_admission_time_at_promotion() {
    let mut f = Fixture::new();
    f.request(1);
    f.clock.advance(Duration::from_secs(10));
    let admitted = f.clock.now();
    f.admit(2);
    f.clock.advance(Duration::from_secs(40));
    f.push(1, Evidence::Started);
    f.push(1, Evidence::Finished);
    f.wake().unwrap();
    assert_eq!(f.coordinator.port.requests.len(), 1);
    f.clock.advance(Duration::from_secs(10));
    assert_eq!(f.coordinator.submit(), Ok(notice(2, Evidence::Requested)));
    let request = &f.coordinator.port.requests[1];
    assert_eq!((request.identity, request.sequence), (identity(2), 2));
    assert_eq!(request.admitted.checked_cmp(&admitted), Ok(Ordering::Equal));
    f.clock.advance(Duration::from_secs(40));
    assert_eq!(f.wake(), Err(Error::Deadline));
    assert_eq!(f.coordinator.port.stops, [identity(2)]);
}

#[test]
fn pending_expiry_releases_its_lease_without_stopping_the_live_job() {
    let mut f = Fixture::new();
    f.request(1);
    f.deadline(2, Duration::from_secs(1));
    f.admit(2);
    f.clock.advance(Duration::from_secs(1));
    f.push(1, Evidence::Started);
    f.push(1, Evidence::Finished);
    assert_eq!(
        f.wake(),
        Ok([
            Some(notice(1, Evidence::Started)),
            Some(notice(1, Evidence::Finished))
        ])
    );
    assert_eq!(f.coordinator.submit(), Err(Error::NoJob));
    assert_eq!(f.coordinator.logical.capacity.usage(), (0, 0, 0));
    assert_eq!(f.coordinator.port.requests.len(), 1);
    assert!(f.coordinator.port.stops.is_empty());
}

#[test]
fn earliest_pending_authorization_caps_the_next_wake() {
    let mut f = Fixture::new();
    f.request(1);
    let shorter = Duration::from_millis(5);
    f.deadline(2, shorter);
    f.admit(2);
    assert_eq!(f.coordinator.wake.as_ref().unwrap().delay, shorter);
    f.clock.advance(shorter);
    f.wake().unwrap();
    assert!(f.coordinator.logical.pending.is_none());
    assert!(f.coordinator.port.stops.is_empty());
    assert_eq!(f.coordinator.wake.as_ref().unwrap().delay, POLL_INTERVAL);
}

#[test]
fn authorization_can_shorten_but_cannot_restore_a_longer_deadline() {
    let mut f = Fixture::new();
    f.request(1);
    f.deadline(1, Duration::from_secs(2));
    f.wake().unwrap();
    f.deadline(1, Duration::from_secs(300));
    f.clock.advance(Duration::from_secs(2));
    assert_eq!(f.wake(), Err(Error::Deadline));
    assert_eq!(f.coordinator.port.stops, [identity(1)]);
}

#[test]
fn positive_evidence_rechecks_authorization_after_polling() {
    let mut f = Fixture::new();
    f.request(1);
    let policies = f.policies.clone();
    f.coordinator.port.after_poll = Some(Box::new(move || {
        policies.borrow_mut().grants[0].as_mut().unwrap().speaker = false;
    }));
    f.push(1, Evidence::Started);
    assert_eq!(f.wake(), Err(Error::Unauthorized));
    assert_eq!(f.coordinator.logical.state(), SpeechState::Idle);
    assert_eq!(f.coordinator.port.stops, [identity(1)]);
    assert_eq!(f.coordinator.submit(), Err(Error::Closed));
}

#[test]
fn promoted_job_must_pass_fresh_source_eligibility_checks() {
    let mut f = Fixture::new();
    f.request(1);
    f.admit(2);
    f.push(1, Evidence::Started);
    f.push(1, Evidence::Finished);
    f.wake().unwrap();
    f.policies.borrow_mut().grants[1].as_mut().unwrap().source = false;
    assert_eq!(f.coordinator.submit(), Err(Error::Unauthorized));
    assert_eq!(f.coordinator.port.requests.len(), 1);
}

#[test]
fn clock_rollback_and_foreign_domains_revoke_even_with_live_output() {
    for foreign in [false, true] {
        let mut f = Fixture::new();
        let old = f.clock.now();
        f.clock.advance(Duration::from_secs(1));
        f.request(1);
        let (replacement, error) = if foreign {
            (ManualClock::new().now(), ContractError::ClockDomainMismatch)
        } else {
            (old, ContractError::TimeReversed)
        };
        f.clock.time.replace(replacement);
        assert_eq!(
            f.wake(),
            Err(Error::Controller(super::super::Error::Contract(error)))
        );
        assert_eq!(f.coordinator.port.polls, 0);
        assert_eq!(f.coordinator.port.stops, [identity(1)]);
        let cancel = Evidence::Cancelled {
            stop_requested: true,
        };
        f.push(1, cancel);
        assert_eq!(f.wake(), Ok([Some(notice(1, cancel)), None]));
        assert_eq!(f.coordinator.cleanup, Cleanup::Unresolved);
    }
}

#[test]
fn foreign_authorization_deadline_cannot_allocate_or_extend_a_lease() {
    let mut f = Fixture::new();
    let foreign = Deadline::after(ManualClock::new().now(), Duration::from_secs(300)).unwrap();
    f.policies.borrow_mut().grants[0].as_mut().unwrap().deadline = foreign;
    assert_eq!(
        f.coordinator.admit(identity(1), 1, "Test".into(), voice()),
        Err(Error::Contract(ContractError::ClockDomainMismatch))
    );
    assert_eq!(f.coordinator.logical.capacity.usage(), (0, 0, 0));
    assert!(f.coordinator.port.requests.is_empty());
}

#[test]
fn phase_one_leases_enforce_exact_utf8_and_aggregate_queue_limits() {
    let mut f = Fixture::new();
    let exact = "é".repeat(MAX_SPEECH_JOB_TEXT_BYTES as usize / 2);
    assert_eq!(
        f.coordinator
            .admit(identity(1), 1, exact.clone() + "é", voice()),
        Err(Error::Controller(super::super::Error::Contract(
            ContractError::LimitExceeded
        )))
    );
    f.coordinator
        .admit(identity(1), 1, exact.clone(), voice())
        .unwrap();
    f.coordinator.admit(identity(2), 2, exact, voice()).unwrap();
    assert_eq!(
        f.coordinator
            .admit(identity(3), 3, "one more".into(), voice()),
        Err(Error::Controller(super::super::Error::QueueFull))
    );
    assert_eq!(
        f.coordinator.logical.capacity.usage(),
        (1, 1, MAX_QUEUED_SPEECH_TEXT_BYTES)
    );
    assert_eq!(f.coordinator.submit(), Ok(notice(1, Evidence::Requested)));
    let request = &f.coordinator.port.requests[0];
    assert_eq!(request.bytes as u64, MAX_SPEECH_JOB_TEXT_BYTES);
    assert!(request.voice == voice());
}

#[test]
fn logical_epoch_exhaustion_cannot_obstruct_immediate_stop() {
    let mut f = Fixture::new();
    f.request(1);
    f.coordinator.logical.epoch = Generation::from_native_sequence(u64::MAX).unwrap();
    assert_eq!(
        f.coordinator.stop(),
        Err(Error::Controller(super::super::Error::Contract(
            ContractError::Overflow
        )))
    );
    assert_eq!(f.coordinator.port.stops, [identity(1)]);
    assert!(f.coordinator.logical.closed);
    assert_eq!(f.coordinator.logical.capacity.usage(), (0, 0, 0));
    assert_eq!(f.coordinator.submit(), Err(Error::Closed));
}

#[cfg(target_os = "macos")]
mod native_driver_ownership {
    use super::super::macos::{CleanupOwner, Driver, DriverHandle, Scheduler};
    use super::*;

    #[derive(Default)]
    struct Schedule {
        arms: usize,
        fail_next: bool,
        on_arm: Option<Box<dyn FnOnce()>>,
        action: Option<Box<dyn FnOnce()>>,
    }

    #[derive(Clone, Default)]
    struct FakeScheduler(Rc<RefCell<Schedule>>);

    impl FakeScheduler {
        fn armed(&self) -> bool {
            self.0.borrow().action.is_some()
        }

        fn arms(&self) -> usize {
            self.0.borrow().arms
        }

        fn fire(&self) {
            let action = self.0.borrow_mut().action.take().expect("timer is armed");
            action();
        }
    }

    struct FakeTimer {
        scheduler: FakeScheduler,
    }

    impl Drop for FakeTimer {
        fn drop(&mut self) {
            self.scheduler.0.borrow_mut().action = None;
        }
    }

    impl Scheduler for FakeScheduler {
        type Armed = FakeTimer;

        fn arm(&self, _delay: Duration, action: impl FnOnce() + 'static) -> Result<FakeTimer> {
            let mut schedule = self.0.borrow_mut();
            if std::mem::take(&mut schedule.fail_next) {
                return Err(Error::Scheduler);
            }
            if schedule.action.is_some() {
                return Err(Error::Scheduler);
            }
            schedule.arms += 1;
            schedule.action = Some(Box::new(action));
            let on_arm = schedule.on_arm.take();
            drop(schedule);
            if let Some(on_arm) = on_arm {
                on_arm();
            }
            Ok(FakeTimer {
                scheduler: self.clone(),
            })
        }
    }

    type TestDriver = Driver<Script, Clock, Gate, FakeScheduler>;
    type Handle = DriverHandle<TestDriver>;

    fn open(scheduler: FakeScheduler) -> Result<Handle> {
        TestDriver::new(scheduler, || Ok(Fixture::new().coordinator))
    }

    fn assert_busy(scheduler: &FakeScheduler) {
        let built = Cell::new(false);
        let attempt = TestDriver::new(scheduler.clone(), || {
            built.set(true);
            Ok(Fixture::new().coordinator)
        });
        assert!(matches!(attempt, Err(Error::Busy)));
        assert!(!built.get(), "rejection must precede port construction");
    }

    fn start(owner: &Handle) {
        TestDriver::turn(owner, |coordinator| {
            coordinator.admit(identity(1), 1, "Synthetic output".into(), voice())?;
            assert_eq!(coordinator.submit()?, notice(1, Evidence::Requested));
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn first_owner_blocks_reentrant_and_repeated_construction_before_the_factory() {
        let scheduler = FakeScheduler::default();
        let owner = TestDriver::new(scheduler.clone(), || {
            assert_busy(&scheduler);
            Ok(Fixture::new().coordinator)
        })
        .unwrap();
        for _ in 0..3 {
            assert_busy(&scheduler);
        }
        assert_eq!(scheduler.arms(), 0);
        TestDriver::turn(&owner, |coordinator| {
            assert!(coordinator.fault.is_none());
            assert_eq!(coordinator.cleanup, Cleanup::NoNativeWork);
            Ok(())
        })
        .unwrap();

        drop(owner);
        assert!(open(scheduler).is_ok());
    }

    #[test]
    fn failed_constructor_rolls_back_its_reservation() {
        let scheduler = FakeScheduler::default();
        let failure = Error::Native(native_speech::Error::BackendUnavailable);
        let rejected = TestDriver::new(scheduler.clone(), || {
            assert_busy(&scheduler);
            Err(failure)
        });
        assert!(matches!(rejected, Err(error) if error == failure));
        assert!(!scheduler.armed());

        let owner = open(scheduler.clone()).unwrap();
        assert_busy(&scheduler);
        drop(owner);
        assert!(open(scheduler).is_ok());
    }

    #[test]
    fn competing_driver_attempts_preserve_the_first_owners_cancellation_timer() {
        let scheduler = FakeScheduler::default();
        let owner = open(scheduler.clone()).unwrap();
        start(&owner);
        assert!(scheduler.armed());

        let cancel = Evidence::Cancelled {
            stop_requested: true,
        };
        TestDriver::turn(&owner, |coordinator| {
            assert_eq!(
                coordinator.stop()?,
                Some(notice(
                    1,
                    Evidence::CancellationUnconfirmed(StopAttempt::Accepted)
                ))
            );
            coordinator.port.notices.extend([
                notice(2, cancel),
                notice(1, Evidence::Finished),
                notice(1, cancel),
            ]);
            assert!(coordinator.cancellation.is_none());
            Ok(())
        })
        .unwrap();

        let arms = scheduler.arms();
        for _ in 0..3 {
            assert_busy(&scheduler);
            assert!(scheduler.armed());
            assert_eq!(scheduler.arms(), arms);
        }
        scheduler.fire();
        assert!(scheduler.armed());
        assert_eq!(scheduler.arms(), arms + 1);
        assert_busy(&scheduler);
        assert!(scheduler.armed());
        assert_eq!(scheduler.arms(), arms + 1);
        scheduler.fire();
        assert!(!scheduler.armed());
        TestDriver::turn(&owner, |coordinator| {
            assert_eq!(coordinator.port.polls, 3);
            assert_eq!(coordinator.port.stops, [identity(1)]);
            assert_eq!(coordinator.cancellation, Some(notice(1, cancel)));
            assert_eq!(coordinator.cleanup, Cleanup::Unresolved);
            assert!(coordinator.fault.is_none());
            assert_eq!(coordinator.submit(), Err(Error::Closed));
            Ok(())
        })
        .unwrap();

        // A stopped driver, even after observing cancellation, still owns the
        // reservation and cannot be displaced by a new port.
        assert_busy(&scheduler);
        drop(owner);
        assert!(open(scheduler).is_ok());
    }

    #[test]
    fn teardown_disarms_before_shutdown_and_holds_ownership_through_port_drop() {
        let scheduler = FakeScheduler::default();
        let seen = Rc::new(RefCell::new(Vec::new()));
        let owner = TestDriver::new(scheduler.clone(), || {
            let mut coordinator = Fixture::new().coordinator;
            let during_shutdown = scheduler.clone();
            let during_drop = scheduler.clone();
            let shutdown_seen = seen.clone();
            let drop_seen = seen.clone();
            coordinator.port.on_shutdown = Some(Box::new(move || {
                assert!(!during_shutdown.armed());
                assert_busy(&during_shutdown);
                shutdown_seen.borrow_mut().push("shutdown");
            }));
            coordinator.port.on_drop = Some(Box::new(move || {
                assert!(!during_drop.armed());
                assert_busy(&during_drop);
                drop_seen.borrow_mut().push("port drop");
            }));
            Ok(coordinator)
        })
        .unwrap();
        start(&owner);
        assert!(scheduler.armed());
        drop(owner);
        assert!(!scheduler.armed());
        assert_eq!(*seen.borrow(), ["shutdown", "port drop"]);
        assert!(open(scheduler).is_ok());
    }

    #[test]
    fn shutdown_and_reservation_release_never_supply_unobserved_native_cleanup() {
        let scheduler = FakeScheduler::default();
        let owner = open(scheduler.clone()).unwrap();
        start(&owner);
        TestDriver::turn(&owner, |coordinator| {
            coordinator.shutdown()?;
            assert!(coordinator.cancellation.is_none());
            assert_eq!(coordinator.cleanup, Cleanup::Unresolved);
            assert_eq!(coordinator.port.shutdowns, 1);
            assert_eq!(coordinator.submit(), Err(Error::Closed));
            Ok(())
        })
        .unwrap();
        assert!(!scheduler.armed());
        assert_busy(&scheduler);
        drop(owner);

        let later = open(scheduler).unwrap();
        TestDriver::turn(&later, |coordinator| {
            assert!(coordinator.cancellation.is_none());
            assert_eq!(coordinator.cleanup, Cleanup::NoNativeWork);
            Ok(())
        })
        .unwrap();
    }

    fn scoped_script(
        scheduler: FakeScheduler,
        configure: impl FnOnce(&mut Coordinator<Script, Clock, Gate>),
    ) -> Handle {
        TestDriver::new(scheduler, || {
            let mut coordinator = Fixture::new().coordinator;
            coordinator.emergency = Some(Rc::new(StopLatch::new(
                coordinator.output_scope.clone(),
                identity(1),
            )));
            configure(&mut coordinator);
            Ok(coordinator)
        })
        .unwrap()
    }

    #[test]
    fn emergency_stop_before_admission_and_during_validation_discards_work() {
        for during_validation in [false, true] {
            let slot = Rc::new(RefCell::new(None::<EmergencyStop>));
            let hook = slot.clone();
            let scheduler = FakeScheduler::default();
            let driver = scoped_script(scheduler.clone(), move |coordinator| {
                let clock = coordinator.clock.clone();
                coordinator.validate = Box::new(move |identity| {
                    if let Some(stop) = hook.borrow_mut().take() {
                        assert_eq!(stop.record(), IntentRecording::Recorded);
                    }
                    Some(Validation {
                        identity,
                        role: ProviderRole::Tts,
                        deadline: Deadline::after(clock.now(), Duration::from_secs(300)).unwrap(),
                        source_eligible: true,
                        speaker_allowed: true,
                        cloud_tts_allowed: false,
                    })
                });
            });
            let stop = driver.emergency_stop().unwrap();
            if during_validation {
                *slot.borrow_mut() = Some(stop.clone());
            } else {
                let guards = [std::sync::Mutex::new(()), std::sync::Mutex::new(())];
                let _held = guards.each_ref().map(|lock| lock.lock().unwrap());
                assert_eq!(stop.record(), IntentRecording::Recorded);
            }
            TestDriver::turn(&driver, |coordinator| {
                assert_eq!(
                    coordinator.admit(identity(1), 1, "Synthetic".into(), voice()),
                    Err(Error::Closed)
                );
                assert!(coordinator.port.requests.is_empty());
                assert_eq!(coordinator.logical.capacity.usage(), (0, 0, 0));
                assert!(coordinator.cancellation.is_none());
                Ok(())
            })
            .unwrap();
            assert_eq!(stop.record(), IntentRecording::AlreadyRecorded);
            assert!(!scheduler.armed());
        }
    }

    #[test]
    fn busy_driver_services_intent_on_success_error_and_unwind() {
        for exit in ["success", "error", "panic"] {
            let scheduler = FakeScheduler::default();
            let driver = scoped_script(scheduler, |_| {});
            start(&driver);
            let stop = driver.emergency_stop().unwrap();
            let result = TestDriver::turn(&driver, |coordinator| {
                assert_eq!(stop.record(), IntentRecording::Recorded);
                assert_eq!(stop.record(), IntentRecording::AlreadyRecorded);
                assert!(coordinator.port.stops.is_empty());
                assert_eq!(TestDriver::turn(&driver, |_| Ok(())), Err(Error::Scheduler));
                match exit {
                    "success" => Ok(()),
                    "error" => Err(Error::NoJob),
                    _ => panic!("synthetic turn panic"),
                }
            });
            assert_eq!(
                result,
                Err(match exit {
                    "success" => Error::Closed,
                    "error" => Error::NoJob,
                    _ => Error::Scheduler,
                })
            );
            TestDriver::turn(&driver, |coordinator| {
                assert_eq!(coordinator.port.stops, [identity(1)]);
                assert_eq!(coordinator.submit(), Err(Error::Closed));
                assert!(coordinator.cancellation.is_none());
                assert_eq!(
                    coordinator.emergency.as_ref().unwrap().intent.get(),
                    Intent::Serviced
                );
                Ok(())
            })
            .unwrap();
        }
    }

    #[test]
    fn busy_timer_preserves_one_stop_wake_and_failed_rearm_tears_down() {
        for fail_rearm in [false, true] {
            let scheduler = FakeScheduler::default();
            let driver = scoped_script(scheduler.clone(), |_| {});
            let stop = driver.emergency_stop().unwrap();
            let on_arm = scheduler.clone();
            scheduler.0.borrow_mut().on_arm = Some(Box::new(move || {
                assert_eq!(stop.record(), IntentRecording::Recorded);
                on_arm.fire(); // The driver still owns its mutable turn.
                on_arm.0.borrow_mut().fail_next = fail_rearm;
            }));
            let result = TestDriver::turn(&driver, |coordinator| {
                coordinator.admit(identity(1), 1, "Synthetic".into(), voice())?;
                coordinator.submit()
            });
            assert_eq!(
                result,
                Err(if fail_rearm {
                    Error::Scheduler
                } else {
                    Error::Closed
                })
            );
            assert_eq!(scheduler.arms(), if fail_rearm { 1 } else { 2 });
            assert_eq!(scheduler.armed(), !fail_rearm);
            if !fail_rearm {
                scheduler.fire();
            }
            TestDriver::turn(&driver, |coordinator| {
                assert_eq!(coordinator.port.stops, [identity(1)]);
                assert!(coordinator.cancellation.is_none());
                assert!(coordinator.revoked);
                assert_eq!(coordinator.shutdown, fail_rearm);
                Ok(())
            })
            .unwrap();
        }
    }

    #[test]
    fn reentrant_stop_suppresses_requested_and_polled_positive_notices() {
        for during_prepare in [false, true] {
            let scheduler = FakeScheduler::default();
            let driver = scoped_script(scheduler.clone(), |_| {});
            let stop = driver.emergency_stop().unwrap();
            let hook = Box::new(move || {
                assert_eq!(stop.record(), IntentRecording::Recorded);
            });
            if during_prepare {
                assert_eq!(
                    TestDriver::turn(&driver, |coordinator| {
                        coordinator.admit(identity(1), 1, "Synthetic".into(), voice())?;
                        coordinator.port.on_prepare = Some(hook);
                        coordinator.port.during_request = Some(Evidence::Started);
                        // The fake reaches its guarded dispatch edge after the Stop hook.
                        coordinator.submit()
                    }),
                    Err(Error::Native(native_speech::Error::Unauthorized))
                );
            } else {
                start(&driver);
                TestDriver::turn(&driver, |coordinator| {
                    coordinator
                        .port
                        .notices
                        .push_back(notice(1, Evidence::Started));
                    coordinator.port.after_poll = Some(hook);
                    let wake = coordinator.wake.clone().unwrap();
                    assert_eq!(coordinator.wake(&wake)?, [None; 2]);
                    Ok(())
                })
                .unwrap();
            }
            TestDriver::turn(&driver, |coordinator| {
                assert_eq!(
                    coordinator.port.requests.len(),
                    usize::from(!during_prepare)
                );
                assert_eq!(coordinator.port.stops.len(), usize::from(!during_prepare));
                assert_eq!(coordinator.cleanup, Cleanup::Unresolved);
                assert_eq!(coordinator.logical.capacity.usage(), (0, 0, 0));
                assert!(coordinator.cancellation.is_none());
                Ok(())
            })
            .unwrap();
        }
    }

    #[test]
    fn submission_stop_check_rejects_foreign_epoch_and_wrong_session_scope() {
        let run = identity(1).app_run_id;
        let route = Generation::from_native_sequence(1).unwrap();
        let owner = OutputScope::new(run, route);
        let latch = Rc::new(StopLatch::new(owner.clone(), identity(1)));
        let stop = EmergencyStop(Rc::downgrade(&latch));
        let exact = owner.bind(identity(1), 1);
        let check = SubmissionStopCheck {
            latch: &latch,
            expected: &exact,
        };
        assert_eq!(check.check_stop(&exact), Ok(()));
        for binding in [
            OutputScope::new(run, route).bind(identity(1), 1),
            owner.next_owner().unwrap().bind(identity(1), 1),
            owner.bind(identity(1), 2),
            owner.bind(identity(1), 1),
        ] {
            assert_eq!(check.check_stop(&binding), Err(RevokedOrMismatched));
        }
        let mutations: [fn(&mut ContractIdentity); 5] = [
            |id| id.interaction_id = InteractionId::from_native_sequence(2).unwrap(),
            |id| id.session_id = SessionId::from_native_sequence(2).unwrap(),
            |id| id.generation = Generation::from_native_sequence(2).unwrap(),
            |id| {
                id.configuration_revision = ConfigurationRevision::from_native_sequence(2).unwrap()
            },
            |id| id.consent_binding_id = ConsentBindingId::from_native_sequence(2).unwrap(),
        ];
        for change in mutations {
            let mut wrong = identity(1);
            change(&mut wrong);
            let binding = owner.bind(wrong, 1);
            let check = SubmissionStopCheck {
                latch: &latch,
                expected: &binding,
            };
            assert_eq!(check.check_stop(&binding), Err(RevokedOrMismatched));
        }
        assert_eq!(stop.record(), IntentRecording::Recorded);
        assert_eq!(check.check_stop(&exact), Err(RevokedOrMismatched));
        assert!(latch.service());
        assert_eq!(check.check_stop(&exact), Err(RevokedOrMismatched));
    }

    #[test]
    fn rejected_or_failed_native_stop_keeps_intent_revoked_without_a_terminal() {
        for native in [
            Ok(Evidence::CancellationUnconfirmed(StopAttempt::Rejected)),
            Ok(Evidence::CancellationUnconfirmed(StopAttempt::Failed)),
            Err(native_speech::Error::NativeFailure),
        ] {
            let scheduler = FakeScheduler::default();
            let driver = scoped_script(scheduler.clone(), |coordinator| {
                coordinator.port.stop = native
            });
            start(&driver);
            let stop = driver.emergency_stop().unwrap();
            assert_eq!(stop.record(), IntentRecording::Recorded);
            for _ in 0..=MAX_STOP_POLLS {
                assert!(scheduler.armed());
                scheduler.fire();
                assert_eq!(stop.record(), IntentRecording::AlreadyRecorded);
            }
            assert!(!scheduler.armed());
            TestDriver::turn(&driver, |coordinator| {
                assert_eq!(coordinator.port.stops, [identity(1)]);
                assert_eq!(coordinator.port.polls, MAX_STOP_POLLS);
                assert!(coordinator.cancellation.is_none());
                assert_eq!(coordinator.cleanup, Cleanup::Unresolved);
                assert_eq!(coordinator.submit(), Err(Error::Closed));
                Ok(())
            })
            .unwrap();
        }
    }

    #[test]
    fn stop_recorded_during_shutdown_is_serviced_without_changing_the_native_attempt() {
        let driver = scoped_script(FakeScheduler::default(), |_| {});
        start(&driver);
        let stop = driver.emergency_stop().unwrap();
        let reentrant = stop.clone();
        TestDriver::turn(&driver, |coordinator| {
            coordinator.port.on_shutdown = Some(Box::new(move || {
                assert_eq!(reentrant.record(), IntentRecording::Recorded);
            }));
            coordinator.shutdown()?;
            assert_eq!(coordinator.port.stops, [identity(1)]);
            assert!(coordinator.cancellation.is_none());
            assert_eq!(
                coordinator.emergency.as_ref().unwrap().intent.get(),
                Intent::Serviced
            );
            Ok(())
        })
        .unwrap();
        drop(driver);
        assert_eq!(stop.record(), IntentRecording::Stale);
    }

    use super::super::macos::{LifecycleIngress, LifecycleReceipt, ManagedOutput};
    use crate::voice_conversation::host::{SpeechLifecycleEvent as Life, SpeechLifecycleHost};
    use crate::voice_conversation::native_speech::RetirementOutput;
    use std::num::NonZeroU64;

    type ModeledOwner = CleanupOwner<Clock, FakeScheduler>;
    type ModeledDriver =
        DriverHandle<Driver<ManagedOutput<Clock, FakeScheduler>, Clock, Gate, FakeScheduler>>;

    fn lifecycle_owner(
        clock: &Clock,
        scheduler: &FakeScheduler,
    ) -> (Rc<ModeledOwner>, Rc<SpeechLifecycleHost>) {
        let id = identity(1);
        let one = NonZeroU64::new(1).unwrap();
        let host = Rc::new(SpeechLifecycleHost::synthetic(
            id.app_run_id,
            one,
            one,
            id.generation,
            clock.now(),
        ));
        assert!(host.apply_speech_event(Life::PermissionRestored, &clock.now()));
        let owner = CleanupOwner::with_host(
            id.app_run_id,
            id.generation,
            clock.clone(),
            scheduler.clone(),
            Some(host.clone()),
        );
        (owner, host)
    }

    fn open_modeled(
        owner: &Rc<ModeledOwner>,
        clock: &Clock,
        session: ContractIdentity,
        authority: LocalAuthority,
        port: impl FnOnce(OutputScope) -> Result<Box<dyn RetirementOutput>>,
    ) -> Result<ModeledDriver> {
        let clock = clock.clone();
        let validate: Gate = Box::new(move |identity| {
            Some(Validation {
                identity,
                role: ProviderRole::Tts,
                deadline: Deadline::after(clock.now(), Duration::from_secs(300)).unwrap(),
                source_eligible: true,
                speaker_allowed: true,
                cloud_tts_allowed: false,
            })
        });
        owner.open_driver(session, validate, authority, port)
    }

    fn record(ingress: &LifecycleIngress, event: Life) {
        ingress.record(&ingress.issue(event).unwrap()).unwrap();
    }

    #[test]
    fn lifecycle_privacy_during_preparation_denies_dispatch_and_survives_busy_host() {
        let clock = Clock::new();
        let scheduler = FakeScheduler::default();
        let (owner, host) = lifecycle_owner(&clock, &scheduler);
        let authority = managed_authority(&clock);
        let ingress = Rc::new(RefCell::new(None::<(LifecycleIngress, LifecycleReceipt)>));
        let during = ingress.clone();
        let mut dispatches = None;
        let driver = open_modeled(&owner, &clock, identity(1), authority.clone(), |scope| {
            let (port, count) =
                native_speech::preparation_fixture(scope, clock.clone(), voice(), move || {
                    let (ingress, receipt) = during.borrow_mut().take().unwrap();
                    ingress.record(&receipt).unwrap();
                });
            dispatches = Some(count);
            Ok(port)
        })
        .unwrap();
        let control = driver.lifecycle().unwrap();
        *ingress.borrow_mut() = Some((control.clone(), control.issue(Life::Sleep).unwrap()));
        host.while_busy(|| {
            assert_eq!(
                Driver::turn(&driver, |coordinator| {
                    coordinator.admit(identity(1), 1, "Synthetic".into(), voice())?;
                    coordinator.submit()
                }),
                Err(Error::Native(native_speech::Error::Unauthorized))
            );
            assert_eq!(dispatches.unwrap().get(), 0);
            record(&control, Life::Wake);
            control.service_host(&clock.now());
        });
        control.service_host(&clock.now());
        assert!(
            !host.observed_sleeping(),
            "Wake must follow the retained Sleep"
        );
        assert!(authority.borrow().scope.is_none());
        assert!(!host.admission_available());
        let report = owner.take_report().unwrap();
        assert_eq!(report.facts().first_stop, None);
        assert!(report.can_abandon_unsubmitted());
        assert!(!scheduler.armed());
    }

    #[test]
    fn lifecycle_receipts_reject_foreign_replay_and_stale_replacement_scopes() {
        let clock = Clock::new();
        let scheduler = FakeScheduler::default();
        let (owner, _) = lifecycle_owner(&clock, &scheduler);
        let (foreign_owner, _) = lifecycle_owner(&clock, &scheduler);
        let open = |owner: &Rc<ModeledOwner>, session: ContractIdentity, authority| {
            open_modeled(owner, &clock, session, authority, |scope| {
                Ok(native_speech::retirement_fixture(scope, clock.clone(), voice(), || {}).0)
            })
            .unwrap()
        };
        let other_driver = open(&foreign_owner, identity(1), managed_authority(&clock));
        let foreign = other_driver.lifecycle().unwrap().issue(Life::Stop).unwrap();
        drop(other_driver);
        let old_authority = managed_authority(&clock);
        let driver = open(&owner, identity(1), old_authority.clone());
        let old = driver.lifecycle().unwrap();
        assert_eq!(old.record(&foreign), Err(Error::Unauthorized));
        let pending = old.issue(Life::Reopen).unwrap();
        let late = old.issue(Life::ConsentLost).unwrap();
        assert_eq!(old.record(&late), Err(Error::Unauthorized));
        old.record(&pending).unwrap();
        assert_eq!(old.record(&pending), Err(Error::Unauthorized));
        drop(driver);
        let reused = open_modeled(
            &foreign_owner,
            &clock,
            identity(1),
            old_authority.clone(),
            |_| panic!("an already bound grant store must be rejected before the factory"),
        );
        assert!(matches!(reused, Err(Error::Unauthorized)));

        let mut next = identity(1);
        next.session_id = SessionId::from_native_sequence(2).unwrap();
        next.generation = next.generation.checked_next().unwrap();
        next.configuration_revision = next.configuration_revision.checked_next().unwrap();
        let fresh = managed_authority(&clock);
        fresh.borrow_mut().scope = Some(next);
        fresh.borrow_mut().grants[0].as_mut().unwrap().identity = next;
        let successor = open(&owner, next, fresh.clone());
        assert!(old_authority.borrow().scope.is_none());
        assert_eq!(old.record(&late), Err(Error::Unauthorized));
        assert_eq!(
            successor.lifecycle().unwrap().record(&late),
            Err(Error::Unauthorized)
        );
        assert_eq!(fresh.borrow().scope, Some(next));
        Driver::turn(&successor, |coordinator| {
            assert!(!coordinator.revoked);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn lifecycle_revocations_fail_closed_even_when_authority_borrow_is_busy() {
        for event in [
            Life::Stop,
            Life::ConsentLost,
            Life::ScopeReplaced,
            Life::Close,
            Life::Lock,
            Life::Logout,
            Life::Interrupted,
            Life::PermissionLost,
        ] {
            let clock = Clock::new();
            let (owner, _) = lifecycle_owner(&clock, &FakeScheduler::default());
            let authority = managed_authority(&clock);
            let driver = open_modeled(&owner, &clock, identity(1), authority.clone(), |scope| {
                Ok(native_speech::retirement_fixture(scope, clock.clone(), voice(), || {}).0)
            })
            .unwrap();
            let control = driver.lifecycle().unwrap();
            let held = authority.borrow_mut();
            record(&control, event);
            assert!(held.scope.is_some());
            Driver::turn(&driver, |coordinator| {
                assert_eq!(coordinator.submit(), Err(Error::Closed));
                Ok(())
            })
            .unwrap();
            drop(held);
            control.service_host(&clock.now());
            assert!(authority.borrow().scope.is_none());
        }
    }

    #[test]
    fn route_fault_and_quit_never_clear_submitted_output_through_cleanup_failures() {
        for (event, failure) in [
            (Life::RouteChanged, "scheduler"),
            (Life::RouteUnknown, "handoff"),
            (Life::ObserverLost, "budget"),
            (Life::Quit, "budget"),
        ] {
            let clock = Clock::new();
            let scheduler = FakeScheduler::default();
            let (owner, _) = lifecycle_owner(&clock, &scheduler);
            let driver = open_modeled(
                &owner,
                &clock,
                identity(1),
                managed_authority(&clock),
                |scope| {
                    Ok(native_speech::retirement_fixture(scope, clock.clone(), voice(), || {}).0)
                },
            )
            .unwrap();
            Driver::turn(&driver, |coordinator| {
                coordinator.admit(identity(1), 1, "Synthetic".into(), voice())?;
                coordinator.submit()?;
                Ok(())
            })
            .unwrap();
            let control = driver.lifecycle().unwrap();
            record(&control, event);
            match failure {
                "scheduler" => {
                    scheduler.0.borrow_mut().fail_next = true;
                    drop(driver);
                }
                "handoff" => owner.while_retaining_slot_is_borrowed(|| drop(driver)),
                _ => {
                    drop(driver);
                    clock.advance(POLL_INTERVAL * MAX_STOP_POLLS as u32);
                    scheduler.fire();
                }
            }
            let report = owner.take_report().unwrap();
            assert_eq!(
                report.facts().submission,
                native_speech::Submission::PossiblySubmitted
            );
            assert_eq!(
                report.facts().first_stop,
                Some((StopOrigin::User, StopAttempt::Accepted))
            );
            assert!(!report.can_abandon_unsubmitted());
            assert!(!scheduler.armed());
            assert_cleanup_owner_busy(&owner);
        }
    }

    #[test]
    fn quit_during_scheduler_registration_suppresses_the_positive_result() {
        let clock = Clock::new();
        let scheduler = FakeScheduler::default();
        let (owner, _) = lifecycle_owner(&clock, &scheduler);
        let driver = open_modeled(
            &owner,
            &clock,
            identity(1),
            managed_authority(&clock),
            |scope| Ok(native_speech::retirement_fixture(scope, clock.clone(), voice(), || {}).0),
        )
        .unwrap();
        let ingress = driver.lifecycle().unwrap();
        scheduler.0.borrow_mut().on_arm = Some(Box::new(move || record(&ingress, Life::Quit)));
        assert_eq!(
            Driver::turn(&driver, |coordinator| {
                coordinator.admit(identity(1), 1, "Synthetic".into(), voice())?;
                coordinator.submit()
            }),
            Err(Error::Closed)
        );
        assert!(
            scheduler.armed(),
            "retirement now owns the one scheduler slot"
        );
        clock.advance(POLL_INTERVAL * MAX_STOP_POLLS as u32);
        scheduler.fire();
        let report = owner.take_report().unwrap();
        assert_eq!(
            report.facts().submission,
            native_speech::Submission::PossiblySubmitted
        );
        assert!(!report.can_abandon_unsubmitted());
        assert_cleanup_owner_busy(&owner);
    }

    #[test]
    fn unresolved_modeled_route_closes_even_idle_admission_before_factory() {
        let clock = Clock::new();
        let (owner, host) = lifecycle_owner(&clock, &FakeScheduler::default());
        let driver = open_modeled(
            &owner,
            &clock,
            identity(1),
            managed_authority(&clock),
            |scope| Ok(native_speech::retirement_fixture(scope, clock.clone(), voice(), || {}).0),
        )
        .unwrap();
        let ingress = driver.lifecycle().unwrap();
        record(&ingress, Life::RouteUnknown);
        record(&ingress, Life::PermissionRestored);
        record(&ingress, Life::Reopen);
        drop(driver);
        assert!(
            owner.take_report().is_none(),
            "an idle route loss had no native flight"
        );
        assert!(!host.admission_available());
        let reopening = open_modeled(
            &owner,
            &clock,
            identity(1),
            managed_authority(&clock),
            |_| panic!("unknown route must close admission before the port factory"),
        );
        assert!(matches!(reopening, Err(Error::Closed)));
    }

    #[test]
    fn full_lifecycle_queue_preserves_order_and_leaves_a_fail_closed_fault() {
        let clock = Clock::new();
        let (owner, host) = lifecycle_owner(&clock, &FakeScheduler::default());
        let driver = open_modeled(
            &owner,
            &clock,
            identity(1),
            managed_authority(&clock),
            |scope| Ok(native_speech::retirement_fixture(scope, clock.clone(), voice(), || {}).0),
        )
        .unwrap();
        let ingress = driver.lifecycle().unwrap();
        for _ in 0..8 {
            record(&ingress, Life::Reopen);
        }
        assert_eq!(
            ingress.record(&ingress.issue(Life::Quit).unwrap()),
            Err(Error::Busy)
        );
        ingress.service_host(&clock.now());
        assert!(!host.admission_available());
        Driver::turn(&driver, |coordinator| {
            assert!(coordinator.shutdown);
            Ok(())
        })
        .unwrap();
    }

    fn managed_authority(clock: &Clock) -> LocalAuthority {
        let authority = LocalAuthority::default();
        authority.borrow_mut().scope = Some(identity(1));
        authority.borrow_mut().grants[0] = Some(LocalSpeechGrant {
            identity: identity(1),
            deadline: Deadline::after(clock.now(), Duration::from_secs(300)).unwrap(),
            speaker: true,
            source: true,
        });
        authority
    }

    fn cleanup_owner(
        clock: Clock,
        scheduler: FakeScheduler,
    ) -> Rc<CleanupOwner<Clock, FakeScheduler>> {
        CleanupOwner::new(
            identity(1).app_run_id,
            Generation::from_native_sequence(1).unwrap(),
            clock,
            scheduler,
        )
    }

    fn assert_cleanup_owner_busy(owner: &Rc<CleanupOwner<Clock, FakeScheduler>>) {
        let result = owner.open_driver(
            identity(1),
            |_| None,
            Default::default(),
            |_| panic!("an unresolved flight must block the factory"),
        );
        assert!(matches!(result, Err(Error::Busy)));
    }

    #[test]
    fn managed_bridge_denies_preparation_stop_before_dispatch_and_retires_exact_obligation() {
        let clock = Clock::new();
        let owner = cleanup_owner(clock.clone(), FakeScheduler::default());
        let validated_clock = clock.clone();
        let control = Rc::new(RefCell::new(None::<EmergencyStop>));
        let reentrant = control.clone();
        let mut dispatches = None;
        let driver = owner
            .open_driver(
                identity(1),
                move |id| {
                    Some(Validation {
                        identity: id,
                        role: ProviderRole::Tts,
                        deadline: Deadline::after(validated_clock.now(), Duration::from_secs(300))
                            .unwrap(),
                        source_eligible: true,
                        speaker_allowed: true,
                        cloud_tts_allowed: false,
                    })
                },
                managed_authority(&clock),
                |scope| {
                    let (port, count) = native_speech::preparation_fixture(
                        scope,
                        clock.clone(),
                        voice(),
                        move || {
                            assert_eq!(
                                reentrant.borrow().as_ref().unwrap().record(),
                                IntentRecording::Recorded
                            )
                        },
                    );
                    dispatches = Some(count);
                    Ok(port)
                },
            )
            .unwrap();
        *control.borrow_mut() = driver.emergency_stop();
        assert_eq!(
            Driver::turn(&driver, |coordinator| {
                coordinator.admit(identity(1), 1, "Synthetic".into(), voice())?;
                coordinator.submit()
            }),
            Err(Error::Native(native_speech::Error::Unauthorized))
        );
        assert_eq!(dispatches.unwrap().get(), 0);
        let report = owner.take_report().unwrap();
        assert_eq!(
            report.facts().submission,
            native_speech::Submission::NeverSubmitted
        );
        assert_eq!(report.facts().first_stop, None);
        assert_eq!(report.facts().detach, native_speech::Detach::VerifiedNil);
        assert!(report.can_abandon_unsubmitted());
        Driver::turn(&driver, |coordinator| {
            assert!(coordinator.revoked);
            assert!(coordinator.cancellation.is_none());
            assert_eq!(coordinator.logical.capacity.usage(), (0, 0, 0));
            Ok(())
        })
        .unwrap();
        drop(driver);
        let later = owner.open_driver(
            identity(1),
            |_| None,
            Default::default(),
            |scope| Ok(native_speech::retirement_fixture(scope, clock.clone(), voice(), || {}).0),
        );
        assert!(later.is_ok());
    }

    #[test]
    fn retained_cleanup_collects_late_cancel_across_drop_without_admitting_a_replacement() {
        let scheduler = FakeScheduler::default();
        let clock = Clock::new();
        let owner = cleanup_owner(clock.clone(), scheduler.clone());
        let during_stop = owner.clone();
        let on_stop = scheduler.clone();
        let validated_clock = clock.clone();
        let mut callback = None;
        let mut drops = None;
        let driver = owner
            .open_driver(
                identity(1),
                move |job| {
                    Some(Validation {
                        identity: job,
                        role: ProviderRole::Tts,
                        deadline: Deadline::after(validated_clock.now(), Duration::from_secs(300))
                            .unwrap(),
                        source_eligible: true,
                        speaker_allowed: true,
                        cloud_tts_allowed: false,
                    })
                },
                managed_authority(&clock),
                |scope| {
                    let (port, cancel, count) = native_speech::retirement_fixture(
                        scope,
                        clock.clone(),
                        voice(),
                        move || {
                            assert!(
                                !on_stop.armed(),
                                "driver timer must be invalidated before Stop"
                            );
                            assert_cleanup_owner_busy(&during_stop);
                        },
                    );
                    callback = Some(cancel);
                    drops = Some(count);
                    Ok(port)
                },
            )
            .unwrap();
        Driver::turn(&driver, |coordinator| {
            coordinator.admit(identity(1), 1, "Synthetic text".into(), voice())?;
            coordinator.submit()?;
            Ok(())
        })
        .unwrap();
        let stop = driver.emergency_stop().unwrap();
        drop(driver);
        assert_eq!(stop.record(), IntentRecording::Recorded);
        assert!(scheduler.armed());
        assert_eq!(drops.as_ref().unwrap().get(), 0);
        assert_cleanup_owner_busy(&owner);
        scheduler.fire();
        assert_eq!(stop.0.upgrade().unwrap().intent.get(), Intent::Serviced);
        assert_eq!(stop.record(), IntentRecording::AlreadyRecorded);
        assert!(scheduler.armed());
        let cancel = callback.unwrap();
        cancel();
        scheduler.fire();
        assert!(!scheduler.armed());
        assert_eq!(stop.record(), IntentRecording::Stale);
        assert_eq!(drops.unwrap().get(), 1);
        let report = owner.take_report().unwrap();
        assert!(!report.can_abandon_unsubmitted());
        let facts = report.facts();
        assert_eq!(
            facts.terminal,
            native_speech::Terminal::Cancelled {
                stop_requested: true
            }
        );
        assert_eq!(facts.detach, native_speech::Detach::VerifiedNil);
        assert_eq!(
            facts.resources,
            native_speech::ResourceRetirement::OwnedFlightRelinquished
        );
        assert_eq!(
            facts.first_stop,
            Some((StopOrigin::Shutdown, StopAttempt::Accepted))
        );
        cancel();
        assert!(owner.take_report().is_none());
        assert_cleanup_owner_busy(&owner);
    }

    #[test]
    fn retention_budget_and_failed_handoff_dispose_resources_without_clearing_the_barrier() {
        for failure in ["poll budget", "deadline", "scheduler", "borrow"] {
            let scheduler = FakeScheduler::default();
            let clock = Clock::new();
            let owner = cleanup_owner(clock.clone(), scheduler.clone());
            let validated_clock = clock.clone();
            let mut drops = None;
            let driver = owner
                .open_driver(
                    identity(1),
                    move |job| {
                        Some(Validation {
                            identity: job,
                            role: ProviderRole::Tts,
                            deadline: Deadline::after(
                                validated_clock.now(),
                                Duration::from_secs(300),
                            )
                            .unwrap(),
                            source_eligible: true,
                            speaker_allowed: true,
                            cloud_tts_allowed: false,
                        })
                    },
                    managed_authority(&clock),
                    |scope| {
                        let (port, _, count) =
                            native_speech::retirement_fixture(scope, clock.clone(), voice(), || {});
                        drops = Some(count);
                        Ok(port)
                    },
                )
                .unwrap();
            Driver::turn(&driver, |coordinator| {
                coordinator.admit(identity(1), 1, "Synthetic text".into(), voice())?;
                coordinator.submit()?;
                Ok(())
            })
            .unwrap();
            let emergency = driver.emergency_stop().unwrap();
            assert_eq!(emergency.record(), IntentRecording::Recorded);
            match failure {
                "borrow" => owner.while_retaining_slot_is_borrowed(|| drop(driver)),
                "scheduler" => {
                    scheduler.0.borrow_mut().fail_next = true;
                    drop(driver);
                }
                "deadline" => {
                    drop(driver);
                    clock.advance(POLL_INTERVAL * (MAX_STOP_POLLS as u32 - 1));
                    assert_eq!(emergency.record(), IntentRecording::AlreadyRecorded);
                    clock.advance(POLL_INTERVAL);
                    scheduler.fire();
                }
                "poll budget" => {
                    Driver::turn(&driver, |coordinator| coordinator.stop()).unwrap();
                    // One native cleanup poll before handoff consumes the shared
                    // budget. Driver replacement cannot restart the allowance.
                    scheduler.fire();
                    drop(driver);
                    for _ in 1..MAX_STOP_POLLS {
                        assert!(scheduler.armed());
                        scheduler.fire();
                    }
                }
                _ => unreachable!(),
            }
            assert!(!scheduler.armed(), "{failure}");
            assert_eq!(drops.unwrap().get(), 1, "{failure}");
            assert!(
                !owner.take_report().unwrap().can_abandon_unsubmitted(),
                "{failure}"
            );
            assert!(owner.take_report().is_none());
            assert_cleanup_owner_busy(&owner);
        }
    }

    #[test]
    fn exact_never_submitted_report_can_abandon_preparation_without_opening_a_flight() {
        let scheduler = FakeScheduler::default();
        let clock = Clock::new();
        let owner = cleanup_owner(clock.clone(), scheduler.clone());
        let validated_clock = clock.clone();
        let driver = owner
            .open_driver(
                identity(1),
                move |job| {
                    Some(Validation {
                        identity: job,
                        role: ProviderRole::Tts,
                        deadline: Deadline::after(validated_clock.now(), Duration::from_secs(300))
                            .unwrap(),
                        source_eligible: true,
                        speaker_allowed: true,
                        cloud_tts_allowed: false,
                    })
                },
                managed_authority(&clock),
                |scope| {
                    let (port, _, _) =
                        native_speech::retirement_fixture(scope, clock.clone(), voice(), || {});
                    Ok(port)
                },
            )
            .unwrap();
        let result = Driver::turn(&driver, |coordinator| {
            let missing = VoiceId::new("missing").unwrap();
            coordinator.admit(identity(1), 1, "Synthetic text".into(), missing)?;
            coordinator.submit()
        });
        assert_eq!(
            result,
            Err(Error::Native(native_speech::Error::VoiceUnavailable))
        );
        let old = driver.emergency_stop().unwrap();
        drop(driver);
        assert!(!scheduler.armed());
        assert_cleanup_owner_busy(&owner);
        let report = owner.take_report().unwrap();
        assert!(report.can_abandon_unsubmitted());
        assert_eq!(
            report.facts().submission,
            native_speech::Submission::NeverSubmitted
        );
        let later = owner.open_driver(
            ContractIdentity {
                session_id: SessionId::from_native_sequence(2).unwrap(),
                ..identity(1)
            },
            |_| None,
            Default::default(),
            |scope| Ok(native_speech::retirement_fixture(scope, clock, voice(), || {}).0),
        );
        let later = later.unwrap();
        assert_eq!(old.record(), IntentRecording::Stale);
        Driver::turn(&later, |coordinator| {
            assert!(!coordinator.emergency.as_ref().unwrap().revoked.get());
            Ok(())
        })
        .unwrap();
    }
}
