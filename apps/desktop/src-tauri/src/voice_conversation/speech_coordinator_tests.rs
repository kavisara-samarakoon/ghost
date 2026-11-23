use super::*;
use crate::voice_conversation::clock::ManualClock;
use crate::voice_conversation::limits::{
    MAX_QUEUED_SPEECH_TEXT_BYTES, MAX_SPEECH_JOB_TEXT_BYTES, SPEECH_OUTPUT_DEADLINE,
};
use crate::voice_conversation::model::*;
use crate::voice_conversation::native_speech::StopAttempt;
use crate::voice_conversation::ContractError;
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
    ) -> NativeResult<Notice> {
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

#[derive(Clone)]
struct Policy {
    deadline: Deadline,
    speaker: bool,
    source: bool,
}

type Gate = Box<dyn Fn(ContractIdentity) -> Option<Validation>>;
type TestCoordinator = Coordinator<Script, Clock, Gate>;

struct Fixture {
    coordinator: TestCoordinator,
    policies: Rc<RefCell<[Option<Policy>; 4]>>,
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
        let policies = Rc::new(RefCell::new(std::array::from_fn(|_| {
            Some(Policy {
                deadline: Deadline::after(clock.now(), Duration::from_secs(300)).unwrap(),
                speaker: true,
                source: true,
            })
        })));
        let source = policies.clone();
        let validate: Gate = Box::new(move |job| {
            let index = (1..=4).position(|sequence| identity(sequence) == job)?;
            let policy = source.borrow()[index].clone()?;
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
        let coordinator = Coordinator::new(
            Script::default(),
            clock.clone(),
            validate,
            scope.app_run_id,
            scope.interaction_id,
        )
        .unwrap();
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
        self.policies.borrow_mut()[sequence - 1]
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
        policies.borrow_mut()[0].as_mut().unwrap().speaker = false;
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
    f.policies.borrow_mut()[1].as_mut().unwrap().source = false;
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
    f.policies.borrow_mut()[0].as_mut().unwrap().deadline = foreign;
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
    use super::super::macos::{CleanupOwner, Driver, Scheduler};
    use super::*;

    #[derive(Default)]
    struct Schedule {
        arms: usize,
        fail_next: bool,
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
            Ok(FakeTimer {
                scheduler: self.clone(),
            })
        }
    }

    type TestDriver = Driver<Script, Clock, Gate, FakeScheduler>;
    type Handle = Rc<RefCell<TestDriver>>;

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
            identity(1).interaction_id,
            |_| None,
            |_| panic!("an unresolved flight must block the factory"),
        );
        assert!(matches!(result, Err(Error::Busy)));
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
                identity(1).interaction_id,
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
        drop(driver);
        assert!(scheduler.armed());
        assert_eq!(drops.as_ref().unwrap().get(), 0);
        assert_cleanup_owner_busy(&owner);
        scheduler.fire();
        assert!(scheduler.armed());
        let cancel = callback.unwrap();
        cancel();
        scheduler.fire();
        assert!(!scheduler.armed());
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
                    identity(1).interaction_id,
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
            match failure {
                "borrow" => owner.while_retaining_slot_is_borrowed(|| drop(driver)),
                "scheduler" => {
                    scheduler.0.borrow_mut().fail_next = true;
                    drop(driver);
                }
                "deadline" => {
                    drop(driver);
                    clock.advance(POLL_INTERVAL * MAX_STOP_POLLS as u32);
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
                identity(1).interaction_id,
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
            identity(1).interaction_id,
            |_| None,
            |scope| Ok(native_speech::retirement_fixture(scope, clock, voice(), || {}).0),
        );
        assert!(later.is_ok());
    }
}
