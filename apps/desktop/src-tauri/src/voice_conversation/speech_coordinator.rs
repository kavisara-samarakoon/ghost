//! Test-only coordination of the Phase 1 speech queue and a single native port.
//! Authority checks are supplied by the caller. Neither a wake nor a native
//! observation can grant authority or initiate the next job.
use super::{Callback, Event, Job, Preparation, SpeechController, SpeechSlot, Update, Validation};
use crate::voice_conversation::clock::{Deadline, MonotonicClock, MonotonicTime};
use crate::voice_conversation::model::{
    AppRunId, ContractIdentity, Generation, InteractionId, SpeechState,
};
use crate::voice_conversation::native_speech::{
    self, Evidence, FlightBinding, LocalAuthorityState, Notice, OutputScope, SpeechOutput,
    StopOrigin, SubmissionCheck, VoiceId,
};
use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};
use std::sync::Arc;
use std::time::Duration;

const POLL_INTERVAL: Duration = Duration::from_millis(50);
const MAX_STOP_POLLS: usize = 40;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Error {
    Contract(crate::voice_conversation::ContractError),
    Controller(super::Error),
    Native(native_speech::Error),
    Unauthorized,
    Deadline,
    Busy,
    Closed,
    NoJob,
    Protocol,
    Scheduler,
}

impl From<super::Error> for Error {
    fn from(error: super::Error) -> Self {
        Self::Controller(error)
    }
}

impl From<crate::voice_conversation::ContractError> for Error {
    fn from(error: crate::voice_conversation::ContractError) -> Self {
        Self::Contract(error)
    }
}

type Result<T> = std::result::Result<T, Error>;
type Notices = [Option<Notice>; 2];

#[derive(Clone)]
struct Wake {
    token: Arc<()>,
    delay: Duration,
    deadline: Option<Deadline>,
}

impl Wake {
    fn matches(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.token, &other.token)
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Cleanup {
    NoNativeWork,
    // P2-01 reports delegate observations without reporting detach success.
    // This status carries no host cleanup acknowledgement.
    Unresolved,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Intent {
    Open,
    Pending,
    Serviced,
}

struct StopLatch {
    owner: OutputScope,
    session: ContractIdentity,
    revoked: Cell<bool>,
    intent: Cell<Intent>,
}

impl StopLatch {
    fn new(owner: OutputScope, session: ContractIdentity) -> Self {
        Self {
            owner,
            session,
            revoked: Cell::new(false),
            intent: Cell::new(Intent::Open),
        }
    }

    fn covers(&self, identity: ContractIdentity) -> bool {
        native_speech::same_session(self.session, identity)
    }

    fn service(&self) -> bool {
        if self.intent.get() != Intent::Pending {
            return false;
        }
        self.intent.set(Intent::Serviced);
        true
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum IntentRecording {
    Recorded,
    AlreadyRecorded,
    Stale,
}

/// Created only by the native driver constructor. A handle conveys only the
/// ability to revoke its own scope and never keeps the driver or port alive.
#[derive(Clone)]
struct EmergencyStop(Weak<StopLatch>);

impl EmergencyStop {
    fn record(&self) -> IntentRecording {
        let Some(latch) = self.0.upgrade() else {
            return IntentRecording::Stale;
        };
        latch.revoked.set(true);
        if latch.intent.get() == Intent::Open {
            latch.intent.set(Intent::Pending);
            IntentRecording::Recorded
        } else {
            IntentRecording::AlreadyRecorded
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
struct RevokedOrMismatched;

/// P2-03D may carry this read-only view through synchronous preparation. It
/// currently checks coordinator entry only. It does not gate the native call.
struct SubmissionStopCheck<'a> {
    latch: &'a StopLatch,
    expected: &'a FlightBinding,
}

impl SubmissionStopCheck<'_> {
    fn check_stop(&self, flight: &FlightBinding) -> std::result::Result<(), RevokedOrMismatched> {
        if self.latch.revoked.get()
            || !self.latch.owner.accepts(flight)
            || !self.latch.covers(flight.identity())
            || !self.expected.matches(flight)
        {
            return Err(RevokedOrMismatched);
        }
        Ok(())
    }
}

#[derive(Clone, Default)]
struct LocalAuthority(Rc<AuthorityState>);

#[derive(Default)]
struct AuthorityState {
    facts: LocalAuthorityState,
    lifecycle_bound: Cell<bool>,
}

impl std::ops::Deref for LocalAuthority {
    type Target = LocalAuthorityState;

    fn deref(&self) -> &Self::Target {
        &self.0.facts
    }
}

struct Admission {
    sequence: u64,
    admitted: MonotonicTime,
    voice: VoiceId,
}

struct Stop {
    attempt: Option<Notice>,
    polls_left: usize,
}

struct Flight {
    // The opaque controller binding changes stage only after an accepted Start.
    // On revoke, keep the old binding exclusively for cleanup evidence.
    binding: Callback,
    stop: Option<Stop>,
}

struct Coordinator<P: SpeechOutput, C, A> {
    logical: SpeechController,
    port: P,
    output_scope: OutputScope,
    emergency: Option<Rc<StopLatch>>,
    clock: C,
    validate: A,
    authority: LocalAuthority,
    host_deadline: Option<Deadline>,
    admissions: [Option<Admission>; 2],
    flight: Option<Flight>,
    wake: Option<Wake>,
    cleanup: Cleanup,
    revoked: bool,
    shutdown: bool,
    fault: Option<Error>,
    cancellation: Option<Notice>,
}

fn slot_index(slot: SpeechSlot) -> usize {
    match slot {
        SpeechSlot::Active => 0,
        SpeechSlot::Pending => 1,
    }
}

impl<P, C, A> Coordinator<P, C, A>
where
    P: SpeechOutput,
    C: MonotonicClock,
    A: Fn(ContractIdentity) -> Option<Validation>,
{
    fn new(
        port: P,
        clock: C,
        validate: A,
        authority: LocalAuthority,
        run: AppRunId,
        interaction: InteractionId,
    ) -> Result<Self> {
        Ok(Self {
            logical: SpeechController::new(run, interaction, clock.now())?,
            port,
            output_scope: OutputScope::new(run, Generation::from_native_sequence(1)?),
            emergency: None,
            clock,
            validate,
            authority,
            host_deadline: None,
            admissions: [None, None],
            flight: None,
            wake: None,
            cleanup: Cleanup::NoNativeWork,
            revoked: false,
            shutdown: false,
            fault: None,
            cancellation: None,
        })
    }

    fn service_emergency(&mut self) -> bool {
        if self.emergency.as_ref().is_some_and(|latch| latch.service()) {
            let _ = self.stop_for(StopOrigin::User);
            true
        } else {
            false
        }
    }

    fn check_open(&mut self) -> Result<()> {
        self.service_emergency();
        if self.revoked {
            return Err(Error::Closed);
        }
        Ok(())
    }

    fn validation(&self, identity: ContractIdentity, now: &MonotonicTime) -> Option<Validation> {
        let mut validation = (self.validate)(identity)
            .filter(|value| value.identity == identity && value.allowed(Preparation::Local))?;
        if let Some(host) = &self.host_deadline {
            if host.remaining(now).ok()? < validation.deadline.remaining(now).ok()? {
                validation.deadline = host.clone();
            }
        }
        Some(validation)
    }

    fn admit(
        &mut self,
        identity: ContractIdentity,
        sequence: u64,
        text: String,
        voice: VoiceId,
    ) -> Result<()> {
        self.check_open()?;
        let now = self.clock.now();
        self.refresh(&now)?;
        self.check_open()?;
        if self
            .emergency
            .as_ref()
            .is_some_and(|latch| !latch.covers(identity))
        {
            return Err(Error::Unauthorized);
        }
        let validation = self.validation(identity, &now);
        self.check_open()?;
        let validation = validation.ok_or(Error::Unauthorized)?;
        // Validate the domain before the controller creates a capacity lease.
        if let Err(error) = validation.deadline.is_expired(&now) {
            return Err(self.fail(error.into()));
        }
        self.logical
            .enqueue(sequence, text, Preparation::Local, &validation, &now)?;
        let job = self
            .logical
            .pending
            .as_ref()
            .or(self.logical.active.as_ref())
            .expect("admission added a job");
        self.admissions[slot_index(job.slot)] = Some(Admission {
            sequence,
            admitted: now.clone(),
            voice,
        });
        self.schedule(&now)?;
        self.check_open()
    }

    /// Starts only the current active logical job, on an explicit coordinator
    /// turn. The borrowed final check follows the exact flight to dispatch.
    fn submit(&mut self) -> Result<Notice> {
        self.check_open()?;
        let now = self.clock.now();
        self.refresh(&now)?;
        self.check_open()?;
        if self.flight.is_some() {
            return Err(Error::Busy);
        }
        let job = self.logical.active.as_ref().ok_or(Error::NoJob)?;
        let admission = self.admissions[slot_index(job.slot)]
            .as_ref()
            .ok_or(Error::Protocol)?;
        let binding = job.callback.clone();
        let native_flight = self.output_scope.bind(binding.identity, binding.operation);
        let latch = self.emergency.as_deref().ok_or(Error::Unauthorized)?;
        let stop = SubmissionStopCheck {
            latch,
            expected: &native_flight,
        };
        stop.check_stop(&native_flight)
            .map_err(|_| Error::Unauthorized)?;
        let check = SubmissionCheck::new(
            &native_flight,
            &self.authority,
            &latch.revoked,
            &now,
            [&job.output_deadline, &job.grant_deadline],
        );

        self.cleanup = Cleanup::Unresolved;
        let notice = match self.port.request(
            native_flight.clone(),
            admission.sequence,
            &job.text,
            &admission.voice,
            &admission.admitted,
            check,
        ) {
            Ok(notice) => notice,
            Err(error) => return Err(self.fail(Error::Native(error))),
        };
        // Even a submission failure reported as a notice can own a native flight.
        self.flight = Some(Flight {
            binding: binding.clone(),
            stop: None,
        });
        // A request that reentered Stop can still have submitted. Keep the
        // flight before service and suppress the positive submission notice.
        self.check_open()?;
        if notice.identity != binding.identity {
            return Err(self.fail(Error::Protocol));
        }
        match notice.evidence {
            Evidence::Requested => {
                // Re-sample after synchronous preparation to schedule against
                // time remaining, without changing the original admission.
                let after = self.clock.now();
                self.refresh(&after)?;
                self.check_open()?;
                self.schedule(&after)?;
                self.check_open()?;
            }
            Evidence::Failure(error) => {
                self.fail(Error::Native(error));
            }
            _ => return Err(self.fail(Error::Protocol)),
        }
        Ok(notice)
    }

    fn tighten(job: &mut Job, validation: Option<&Validation>, now: &MonotonicTime) -> Result<()> {
        let deadline = match validation {
            Some(value) => value.deadline.clone(),
            None => Deadline::after(now.clone(), Duration::ZERO)?,
        };
        // A refresh can tighten the existing Phase 1 grant deadline. Keep the
        // earlier deadline and let the controller expire its own leases.
        if deadline.remaining(now)? < job.grant_deadline.remaining(now)? {
            job.grant_deadline = deadline;
        }
        Ok(())
    }

    fn refresh(&mut self, now: &MonotonicTime) -> Result<Option<Validation>> {
        let result = self.refresh_inner(now);
        result.map_err(|error| self.fail(error))
    }

    fn refresh_inner(&mut self, now: &MonotonicTime) -> Result<Option<Validation>> {
        self.check_open()?;
        self.logical.check_time(now)?;
        let active_identity = self.logical.callback().map(|binding| binding.identity);
        let active = self
            .logical
            .active
            .as_ref()
            .and_then(|job| self.validation(job.callback.identity, now));
        self.check_open()?;
        let pending = self
            .logical
            .pending
            .as_ref()
            .and_then(|job| self.validation(job.callback.identity, now));
        self.check_open()?;
        if let Some(job) = &mut self.logical.active {
            Self::tighten(job, active.as_ref(), now)?;
        }
        if let Some(job) = &mut self.logical.pending {
            Self::tighten(job, pending.as_ref(), now)?;
        }
        let update = self.logical.tick(now)?;
        let active_ended = active_identity.is_some_and(|identity| {
            update
                .iter()
                .flatten()
                .any(|result| result.identity == identity)
        });
        self.drop_retired(update);
        if active_ended {
            return Err(if active.is_some() {
                Error::Deadline
            } else {
                Error::Unauthorized
            });
        }
        Ok(active)
    }

    fn drop_retired(&mut self, update: Update) {
        // The Phase 1 update owns any retired text. Drop it here, along with
        // follow-up candidate data, without logging or dispatching anything.
        drop(update);
        let mut occupied = [false; 2];
        for job in self.logical.active.iter().chain(&self.logical.pending) {
            occupied[slot_index(job.slot)] = true;
        }
        for (admission, occupied) in self.admissions.iter_mut().zip(occupied) {
            if !occupied {
                *admission = None;
            }
        }
    }

    fn schedule(&mut self, now: &MonotonicTime) -> Result<()> {
        self.schedule_inner(now).map_err(|error| self.fail(error))
    }

    fn schedule_inner(&mut self, now: &MonotonicTime) -> Result<()> {
        let mut delay = self.flight.as_ref().map(|_| POLL_INTERVAL);
        for job in self.logical.active.iter().chain(&self.logical.pending) {
            let remaining = job
                .output_deadline
                .remaining(now)?
                .min(job.grant_deadline.remaining(now)?);
            delay = Some(delay.map_or(remaining, |current| remaining.min(current)));
        }
        self.wake = delay
            .map(|delay| {
                Deadline::after(now.clone(), delay).map(|deadline| Wake {
                    token: Arc::new(()),
                    delay,
                    deadline: Some(deadline),
                })
            })
            .transpose()?;
        Ok(())
    }

    fn arm_cleanup(&mut self) {
        self.wake = Some(Wake {
            token: Arc::new(()),
            delay: POLL_INTERVAL,
            deadline: None,
        });
    }

    fn is_current(&self, wake: &Wake) -> bool {
        self.wake
            .as_ref()
            .is_some_and(|current| current.matches(wake))
    }

    fn wake(&mut self, wake: &Wake) -> Result<Notices> {
        let mut notices = [None; 2];
        if self.service_emergency() || !self.is_current(wake) {
            return Ok(notices);
        }
        self.wake = None;
        if self.revoked {
            self.poll_cleanup(&mut notices);
            return Ok(notices);
        }

        let now = self.clock.now();
        self.refresh(&now)?;
        let mut now = now;
        for output in &mut notices {
            if self.flight.is_none() {
                break;
            }
            let notice = self.port.poll();
            if self.service_emergency() {
                return Ok([None; 2]);
            }
            let Some(notice) = notice else {
                break;
            };
            now = self.clock.now();
            let fresh = self.refresh(&now)?;
            if self.accept(notice, fresh.as_ref(), &now)? {
                *output = Some(notice);
            }
            if self.revoked {
                break;
            }
        }
        if !self.revoked {
            self.schedule(&now)?;
        }
        if self.service_emergency() {
            return Ok([None; 2]);
        }
        Ok(notices)
    }

    fn accept(
        &mut self,
        notice: Notice,
        validation: Option<&Validation>,
        now: &MonotonicTime,
    ) -> Result<bool> {
        let Some(flight) = self.flight.as_ref() else {
            return Ok(false);
        };
        if notice.identity != flight.binding.identity || !self.logical.matches(&flight.binding) {
            return Ok(false);
        }
        let event = match (notice.evidence, flight.binding.stage) {
            (Evidence::Started, SpeechState::Preparing) => Event::Prepared,
            (Evidence::Finished, SpeechState::Speaking) => Event::Completed,
            (
                Evidence::Cancelled {
                    stop_requested: false,
                },
                _,
            ) => Event::Failed,
            (Evidence::Failure(error), _) => {
                self.fail(Error::Native(error));
                return Ok(true);
            }
            _ => return Ok(false),
        };
        let validation = validation.ok_or_else(|| self.fail(Error::Unauthorized))?;
        let flight = self.flight.as_ref().expect("notice matched a flight");
        let update = self
            .logical
            .handle(&flight.binding, event, validation, now)
            .map_err(|error| self.fail(error.into()))?;
        self.drop_retired(update);
        match event {
            Event::Prepared => {
                let binding = self
                    .logical
                    .callback()
                    .ok_or_else(|| self.fail(Error::Protocol))?;
                self.flight
                    .as_mut()
                    .expect("start keeps its flight")
                    .binding = binding;
            }
            Event::Completed => self.flight = None,
            Event::Failed => {
                // An unsolicited cancellation is a terminal native observation.
                // It cannot acknowledge a user Stop or attest to delegate detach.
                self.cancellation = Some(notice);
                self.flight = None;
                self.stop()?;
            }
            Event::AudioReady { .. } => unreachable!("local speech has no cloud audio"),
        }
        Ok(true)
    }

    fn fail(&mut self, error: Error) -> Error {
        self.service_emergency();
        self.fault.get_or_insert(error);
        let origin = if error == Error::Deadline {
            StopOrigin::Deadline
        } else {
            StopOrigin::CoordinatorFailure
        };
        let _ = self.stop_for(origin);
        error
    }

    fn stop(&mut self) -> Result<Option<Notice>> {
        self.stop_for(StopOrigin::User)
    }

    fn stop_for(&mut self, origin: StopOrigin) -> Result<Option<Notice>> {
        if let Some(latch) = &self.emergency {
            latch.revoked.set(true);
        }
        if self.revoked {
            return Ok(self
                .flight
                .as_ref()
                .and_then(|flight| flight.stop.as_ref()?.attempt));
        }
        self.revoked = true;
        self.wake = None;
        let logical = self.logical.stop();
        self.admissions = [None, None];
        // Drive native Stop even if logical generation or lease retirement failed.
        let native = if let Some(flight) = &mut self.flight {
            let identity = flight.binding.identity;
            let attempt = match self.port.stop(identity, origin) {
                Ok(notice)
                    if notice.identity == identity
                        && matches!(notice.evidence, Evidence::CancellationUnconfirmed(_)) =>
                {
                    Ok(notice)
                }
                Ok(_) => Err(Error::Protocol),
                Err(error) => Err(Error::Native(error)),
            };
            flight.stop = Some(Stop {
                attempt: attempt.ok(),
                polls_left: MAX_STOP_POLLS,
            });
            self.arm_cleanup();
            attempt.map(Some)
        } else {
            Ok(None)
        };
        logical?;
        native
    }

    fn poll_cleanup(&mut self, notices: &mut Notices) {
        let Some(flight) = self.flight.as_mut() else {
            return;
        };
        let Some(stop) = flight.stop.as_mut() else {
            return;
        };
        if stop.polls_left == 0 {
            return;
        }
        stop.polls_left -= 1;
        let remaining = stop.polls_left;
        for output in notices {
            let Some(notice) = self.port.poll() else {
                break;
            };
            if self.observe_cancellation(notice) {
                *output = Some(notice);
                return;
            }
        }
        // The observation budget only bounds timer work. Missing evidence
        // leaves the scope revoked and the exact flight retained for shutdown.
        if remaining > 0 {
            self.arm_cleanup();
        }
    }

    fn observe_cancellation(&mut self, notice: Notice) -> bool {
        let exact = self.flight.as_ref().is_some_and(|flight| {
            flight.stop.is_some() && flight.binding.identity == notice.identity
        });
        if !exact || !matches!(notice.evidence, Evidence::Cancelled { .. }) {
            return false;
        }
        // stop_requested=false is recorded as unsolicited and never counts as
        // confirmed cancellation of our Stop. Neither variant proves cleanup.
        self.cancellation = Some(notice);
        self.flight = None;
        self.wake = None;
        true
    }

    fn shutdown(&mut self) -> Result<()> {
        if self.shutdown {
            self.service_emergency();
            return Ok(());
        }
        self.shutdown = true;
        self.service_emergency();
        let result = self.stop_for(StopOrigin::Shutdown);
        self.wake = None;
        if let Some(notice) = self.port.shutdown() {
            self.observe_cancellation(notice);
        }
        self.flight = None;
        self.service_emergency();
        result.map(|_| ())
    }
}

#[cfg(target_os = "macos")]
mod macos {
    use super::*;
    use crate::voice_conversation::clock::NativeMonotonicClock;
    use crate::voice_conversation::host::{
        self, CleanupTicket, OutputAckAuthority, SpeechEnablePreview, SpeechLifecycleEvent,
        SpeechLifecycleHost,
    };
    use crate::voice_conversation::native_speech::{
        Detach, ResourceRetirement, RetirementFacts, RetirementOutput, RetirementReport,
        RetirementUncertainty, Submission, Terminal,
    };
    use core_foundation::base::{kCFAllocatorDefault, TCFType};
    use core_foundation::date::CFAbsoluteTimeGetCurrent;
    use core_foundation::runloop::{
        kCFRunLoopCommonModes, CFRunLoop, CFRunLoopTimer, CFRunLoopTimerCreate,
        CFRunLoopTimerInvalidate, CFRunLoopTimerRef,
    };
    use objc2::MainThreadMarker;
    use std::ffi::c_void;
    use std::marker::PhantomData;
    use std::panic::{catch_unwind, AssertUnwindSafe};

    struct Handler {
        timer: CFRunLoopTimerRef,
        action: Box<dyn FnOnce()>,
    }

    // The CF timer carries no Rust context pointer. The main-thread registry
    // owns one handler, matched by the address of its live CF timer.
    thread_local! {
        static HANDLER: RefCell<Option<Handler>> = const { RefCell::new(None) };
        static DRIVER_LIVE: Cell<bool> = const { Cell::new(false) };
    }

    // The reservation and its release stay on the thread that owns the driver.
    struct DriverReservation(PhantomData<Rc<()>>);

    impl DriverReservation {
        fn acquire() -> Result<Self> {
            DRIVER_LIVE
                .try_with(|live| {
                    if live.get() {
                        return Err(Error::Busy);
                    }
                    live.set(true);
                    Ok(Self(PhantomData))
                })
                .map_err(|_| Error::Scheduler)?
        }
    }

    impl Drop for DriverReservation {
        fn drop(&mut self) {
            let _ = DRIVER_LIVE.try_with(|live| live.set(false));
        }
    }

    fn take_handler(timer: CFRunLoopTimerRef) -> Option<Handler> {
        HANDLER
            .try_with(|slot| {
                let mut slot = slot.try_borrow_mut().ok()?;
                if slot.as_ref()?.timer != timer {
                    return None;
                }
                slot.take()
            })
            .ok()
            .flatten()
    }

    extern "C" fn fired(timer: CFRunLoopTimerRef, _info: *mut c_void) {
        if let Err(payload) = catch_unwind(AssertUnwindSafe(|| {
            if let Some(handler) = take_handler(timer) {
                (handler.action)();
            }
        })) {
            // Never unwind through Core Foundation, even during payload drop.
            std::mem::forget(payload);
        }
    }

    struct Timer {
        native: CFRunLoopTimer,
        _main: MainThreadMarker,
    }

    impl Timer {
        fn arm(
            main: MainThreadMarker,
            delay: Duration,
            action: impl FnOnce() + 'static,
        ) -> Result<Self> {
            let timer = HANDLER
                .try_with(|slot| {
                    let mut slot = slot.try_borrow_mut().map_err(|_| Error::Scheduler)?;
                    if slot.is_some() {
                        return Err(Error::Scheduler);
                    }
                    // SAFETY: Timer creation uses the verified CF ABI, a null
                    // context, and a non-unwinding callback. Interval zero is one-shot.
                    let raw = unsafe {
                        CFRunLoopTimerCreate(
                            kCFAllocatorDefault,
                            CFAbsoluteTimeGetCurrent() + delay.as_secs_f64(),
                            0.0,
                            0,
                            0,
                            fired,
                            std::ptr::null_mut(),
                        )
                    };
                    if raw.is_null() {
                        return Err(Error::Scheduler);
                    }
                    // SAFETY: A non-null Create result transfers one owned reference.
                    let native = unsafe { CFRunLoopTimer::wrap_under_create_rule(raw) };
                    *slot = Some(Handler {
                        timer: raw,
                        action: Box::new(action),
                    });
                    Ok(Self {
                        native,
                        _main: main,
                    })
                })
                .map_err(|_| Error::Scheduler)??;
            // SAFETY: The constant names Core Foundation's common modes. The
            // main-thread marker binds timer creation, delivery and invalidation.
            CFRunLoop::get_main().add_timer(&timer.native, unsafe { kCFRunLoopCommonModes });
            Ok(timer)
        }
    }

    impl Drop for Timer {
        fn drop(&mut self) {
            let raw = self.native.as_concrete_TypeRef();
            drop(take_handler(raw));
            // SAFETY: The retained timer is valid through this call on its owner
            // thread. Invalidation is safe even after the one-shot fired.
            unsafe { CFRunLoopTimerInvalidate(raw) };
        }
    }

    pub(super) trait Scheduler {
        type Armed;

        fn arm(&self, delay: Duration, action: impl FnOnce() + 'static) -> Result<Self::Armed>;
    }

    #[derive(Clone)]
    struct MainRunLoop(MainThreadMarker);

    impl Scheduler for MainRunLoop {
        type Armed = Timer;

        fn arm(&self, delay: Duration, action: impl FnOnce() + 'static) -> Result<Timer> {
            Timer::arm(self.0, delay, action)
        }
    }

    #[derive(Clone)]
    struct SharedClock(Rc<NativeMonotonicClock>);

    impl MonotonicClock for SharedClock {
        fn now(&self) -> MonotonicTime {
            self.0.now()
        }
    }

    const LIFECYCLE_QUEUE: usize = 8;

    fn host_error(error: host::Error) -> Error {
        match error {
            host::Error::Busy => Error::Busy,
            host::Error::Closed => Error::Closed,
            _ => Error::Unauthorized,
        }
    }

    #[derive(Clone, Copy, PartialEq, Eq)]
    enum RouteAvailability {
        Known,
        Lost,
        Restored,
        Closed,
    }

    #[derive(Clone, Copy)]
    struct QueuedLifecycle {
        event: SpeechLifecycleEvent,
        sequence: u64,
    }

    #[derive(Clone)]
    pub(super) struct ModeledEnablePreview {
        host: SpeechEnablePreview,
        owner: Weak<LifecycleOwner>,
        sequence: u64,
        session: ContractIdentity,
        scope: Option<Weak<LifecycleScope>>,
    }

    struct ConfirmedModeledEnable {
        receipt: ModeledEnablePreview,
        confirmed_at: MonotonicTime,
    }

    struct LifecycleOwner {
        host: Rc<SpeechLifecycleHost>,
        route: Cell<RouteAvailability>,
        current: RefCell<Option<Rc<LifecycleScope>>>,
        confirmed: RefCell<Option<ConfirmedModeledEnable>>,
    }

    impl LifecycleOwner {
        fn bind(
            self: &Rc<Self>,
            session: ContractIdentity,
            authority: &LocalAuthority,
            latch: &Rc<StopLatch>,
            now: &MonotonicTime,
        ) -> Result<(LifecycleIngress, Deadline)> {
            self.ready()?;
            let mut confirmed = self.confirmed.try_borrow_mut().map_err(|_| Error::Busy)?;
            let confirmation = confirmed.as_ref().ok_or(Error::Unauthorized)?;
            self.check_confirmed(confirmation, session, now)?;
            self.check_local_scope(authority, session)?;
            let mut current = self.current.try_borrow_mut().map_err(|_| Error::Busy)?;
            if authority.0.lifecycle_bound.get() {
                return Err(Error::Unauthorized);
            }
            if let Some(old) = current.as_ref() {
                if old.pending() {
                    return Err(Error::Busy);
                }
                old.stop.record();
                old.invalidate()?;
            }
            let deadline = self
                .host
                .activate_speech_enable(
                    &confirmation.receipt.host,
                    now,
                    current.as_ref().map(|old| &old.live),
                )
                .map_err(host_error)?;
            *confirmed = None;
            // A local grant store belongs to exactly one lifecycle binding,
            // even across different synthetic host owners.
            authority.0.lifecycle_bound.set(true);
            let scope = Rc::new(LifecycleScope {
                owner: Rc::downgrade(self),
                stop: EmergencyStop(Rc::downgrade(latch)),
                authority: authority.clone(),
                session,
                live: Cell::new(true),
                issued: Cell::new(0),
                applied_sequence: Cell::new(0),
                queue: Cell::new([None; LIFECYCLE_QUEUE]),
                last_route_loss: Cell::new(0),
                fault: Cell::new(false),
                fault_applied: Cell::new(false),
                quit: Cell::new(false),
                revoked: Cell::new(false),
                cleanup_ticket: std::cell::OnceCell::new(),
            });
            *current = Some(scope.clone());
            Ok((LifecycleIngress(scope), deadline))
        }

        fn ready(&self) -> Result<()> {
            if matches!(
                self.route.get(),
                RouteAvailability::Lost | RouteAvailability::Closed
            ) {
                return Err(Error::Closed);
            }
            let current = self.current.try_borrow().map_err(|_| Error::Busy)?;
            if current.as_ref().is_some_and(|scope| scope.pending()) {
                return Err(Error::Busy);
            }
            Ok(())
        }

        fn lineage(&self) -> Result<(Option<Weak<LifecycleScope>>, u64)> {
            let current = self.current.try_borrow().map_err(|_| Error::Busy)?;
            Ok(current.as_ref().map_or((None, 0), |scope| {
                (Some(Rc::downgrade(scope)), scope.applied_sequence.get())
            }))
        }

        fn check_confirmation(
            &self,
            receipt: &ModeledEnablePreview,
            session: ContractIdentity,
            now: &MonotonicTime,
        ) -> Result<()> {
            if receipt.session != session {
                return Err(Error::Unauthorized);
            }
            let (scope, sequence) = self.lineage()?;
            let exact_scope = match (&scope, &receipt.scope) {
                (None, None) => true,
                (Some(scope), Some(expected)) => Weak::ptr_eq(scope, expected),
                _ => false,
            };
            if !exact_scope || sequence != receipt.sequence {
                return Err(Error::Unauthorized);
            }
            self.host
                .validate_speech_enable(&receipt.host, session, now)
                .map_err(host_error)
        }

        fn check_confirmed(
            &self,
            confirmation: &ConfirmedModeledEnable,
            session: ContractIdentity,
            now: &MonotonicTime,
        ) -> Result<()> {
            now.checked_duration_since(&confirmation.confirmed_at)?;
            self.check_confirmation(&confirmation.receipt, session, now)
        }

        fn check_local_scope(
            &self,
            authority: &LocalAuthority,
            session: ContractIdentity,
        ) -> Result<()> {
            if session.app_run_id != self.host.run()
                || !authority
                    .try_borrow()
                    .map_err(|_| Error::Busy)?
                    .scope
                    .is_some_and(|scope| native_speech::same_session(scope, session))
            {
                return Err(Error::Unauthorized);
            }
            Ok(())
        }

        fn check_admission(
            &self,
            session: ContractIdentity,
            authority: &LocalAuthority,
            now: &MonotonicTime,
        ) -> Result<()> {
            self.ready()?;
            self.host.check_admission_available().map_err(host_error)?;
            self.check_local_scope(authority, session)?;
            let confirmed = self.confirmed.try_borrow().map_err(|_| Error::Busy)?;
            let confirmation = confirmed.as_ref().ok_or(Error::Unauthorized)?;
            self.check_confirmed(confirmation, session, now)
        }

        fn service(&self, now: &MonotonicTime) {
            // No owner borrow spans a modeled transition.
            let current = self.current.try_borrow().ok().and_then(|slot| slot.clone());
            if let Some(current) = current {
                current.service(self, now);
            }
        }
    }

    struct LifecycleScope {
        owner: Weak<LifecycleOwner>,
        stop: EmergencyStop,
        authority: LocalAuthority,
        session: ContractIdentity,
        live: Cell<bool>,
        issued: Cell<u64>,
        applied_sequence: Cell<u64>,
        queue: Cell<[Option<QueuedLifecycle>; LIFECYCLE_QUEUE]>,
        last_route_loss: Cell<u64>,
        fault: Cell<bool>,
        fault_applied: Cell<bool>,
        quit: Cell<bool>,
        revoked: Cell<bool>,
        cleanup_ticket: std::cell::OnceCell<CleanupTicket>,
    }

    impl LifecycleScope {
        fn invalidate(&self) -> Result<()> {
            let mut state = self.authority.try_borrow_mut().map_err(|_| Error::Busy)?;
            if state
                .scope
                .is_some_and(|scope| native_speech::same_session(scope, self.session))
            {
                state.scope = None;
                state.grants.fill(None);
            }
            Ok(())
        }

        fn revoke(&self) {
            self.stop.record();
            self.revoked.set(true);
        }

        fn fail_ingress(&self, owner: &LifecycleOwner, event: SpeechLifecycleEvent) {
            self.revoke();
            self.fault.set(true);
            owner.route.set(RouteAvailability::Closed);
            if event.quits() {
                self.quit.set(true);
            }
        }

        fn pending(&self) -> bool {
            self.queue.get()[0].is_some() || (self.fault.get() && !self.fault_applied.get())
        }

        fn bind_cleanup_ticket(&self, owner: &LifecycleOwner) {
            if let Ok(Some(ticket)) = owner.host.output_cleanup_ticket() {
                let _ = self.cleanup_ticket.set(ticket);
            }
        }

        fn service(&self, owner: &LifecycleOwner, now: &MonotonicTime) {
            if !self.live.get() {
                return;
            }
            let mut queued = self.queue.get();
            while let Some(QueuedLifecycle { event, sequence }) = queued[0] {
                if event == SpeechLifecycleEvent::RouteRestored {
                    // Availability can drain while an old flight still needs
                    // resolution. A newer accepted loss supersedes this receipt.
                    let loss = self.last_route_loss.get();
                    if owner.route.get() == RouteAvailability::Lost && loss != 0 && loss < sequence
                    {
                        match owner.host.restore_modeled_route(now) {
                            Ok(()) => owner.route.set(RouteAvailability::Restored),
                            Err(host::Error::Busy) => return,
                            Err(_) => owner.route.set(RouteAvailability::Closed),
                        }
                    }
                } else {
                    if (event.revokes() && self.invalidate().is_err())
                        || !owner.host.apply_speech_event(event, now)
                    {
                        return;
                    }
                    if event.revokes() {
                        self.bind_cleanup_ticket(owner);
                    }
                }
                queued.rotate_left(1);
                queued[LIFECYCLE_QUEUE - 1] = None;
                self.queue.set(queued);
            }
            if self.fault.get() && !self.fault_applied.get() && self.invalidate().is_ok() {
                let event = if self.quit.get() {
                    SpeechLifecycleEvent::Quit
                } else {
                    SpeechLifecycleEvent::ObserverLost
                };
                if owner.host.apply_speech_event(event, now) {
                    self.bind_cleanup_ticket(owner);
                    self.fault_applied.set(true);
                }
            }
        }
    }

    /// The preissued control holds the exact driver's latch, synthetic host,
    /// and a private per-binding receipt identity. None is serializable.
    #[derive(Clone)]
    pub(super) struct LifecycleIngress(Rc<LifecycleScope>);

    pub(super) struct LifecycleReceipt {
        scope: Weak<LifecycleScope>,
        sequence: u64,
        event: SpeechLifecycleEvent,
    }

    impl LifecycleIngress {
        pub(super) fn issue(&self, event: SpeechLifecycleEvent) -> Result<LifecycleReceipt> {
            if !self.0.live.get() || self.0.owner.upgrade().is_none() || self.0.fault.get() {
                return Err(Error::Closed);
            }
            let Some(sequence) = self.0.issued.get().checked_add(1) else {
                if let Some(owner) = self.0.owner.upgrade() {
                    self.0.fail_ingress(&owner, event);
                }
                return Err(Error::Closed);
            };
            self.0.issued.set(sequence);
            Ok(LifecycleReceipt {
                scope: Rc::downgrade(&self.0),
                sequence,
                event,
            })
        }

        pub(super) fn record(&self, receipt: &LifecycleReceipt) -> Result<()> {
            let scope = &self.0;
            let owner = scope.owner.upgrade().ok_or(Error::Closed)?;
            if !scope.live.get()
                || !Weak::ptr_eq(&receipt.scope, &Rc::downgrade(scope))
                || scope.applied_sequence.get().checked_add(1) != Some(receipt.sequence)
            {
                return Err(Error::Unauthorized);
            }
            if scope.fault.get() {
                return Err(Error::Closed);
            }
            if receipt.event.revokes() {
                scope.revoke();
                let _ = scope.invalidate(); // retained event retries a busy authority
            }
            if receipt.event.loses_route() && owner.route.get() != RouteAvailability::Closed {
                scope.last_route_loss.set(receipt.sequence);
                owner.route.set(RouteAvailability::Lost);
            }
            if receipt.event == SpeechLifecycleEvent::ObserverLost || receipt.event.quits() {
                owner.route.set(RouteAvailability::Closed);
            }
            if receipt.event.quits() {
                scope.quit.set(true);
            }
            let mut queued = scope.queue.get();
            let Some(slot) = queued.iter_mut().find(|slot| slot.is_none()) else {
                // Existing accepted transitions retain their FIFO order. The
                // rejected event closes the scope, with sticky Quit if needed.
                scope.fail_ingress(&owner, receipt.event);
                return Err(Error::Busy);
            };
            *slot = Some(QueuedLifecycle {
                event: receipt.event,
                sequence: receipt.sequence,
            });
            scope.applied_sequence.set(receipt.sequence);
            scope.queue.set(queued);
            Ok(())
        }

        pub(super) fn service_host(&self, now: &MonotonicTime) {
            if let Some(owner) = self.0.owner.upgrade() {
                self.0.service(&owner, now);
            }
        }
    }

    // All borrows of the inner driver are controlled here. A busy timer
    // callback therefore runs inside a turn with a guaranteed Stop checkpoint.
    pub(super) struct DriverHandle<D> {
        driver: Rc<RefCell<D>>,
        emergency: Option<EmergencyStop>,
        lifecycle: Option<LifecycleIngress>,
    }

    impl<D> DriverHandle<D> {
        pub(super) fn emergency_stop(&self) -> Option<EmergencyStop> {
            self.emergency.clone()
        }

        pub(super) fn lifecycle(&self) -> Option<LifecycleIngress> {
            self.lifecycle.clone()
        }
    }

    pub(super) struct Driver<
        P: SpeechOutput,
        C: MonotonicClock,
        A: Fn(ContractIdentity) -> Option<Validation>,
        S: Scheduler,
    > {
        coordinator: Coordinator<P, C, A>,
        timer: Option<S::Armed>,
        scheduler: S,
        // Field destruction follows declaration order. Keep the reservation
        // through timer invalidation, shutdown and actual port destruction.
        _reservation: DriverReservation,
        lifecycle: Option<LifecycleIngress>,
    }

    impl<P, C, A, S> Driver<P, C, A, S>
    where
        P: SpeechOutput + 'static,
        C: MonotonicClock + 'static,
        A: Fn(ContractIdentity) -> Option<Validation> + 'static,
        S: Scheduler + 'static,
    {
        pub(super) fn new(
            scheduler: S,
            build: impl FnOnce() -> Result<Coordinator<P, C, A>>,
        ) -> Result<DriverHandle<Self>> {
            Self::new_bound(|| Ok((build()?, None)), scheduler)
        }

        fn new_bound(
            build: impl FnOnce() -> Result<(Coordinator<P, C, A>, Option<LifecycleIngress>)>,
            scheduler: S,
        ) -> Result<DriverHandle<Self>> {
            // Acquire before running any constructor that can open the port
            // or reenter on this thread. Failure drops the local reservation.
            let reservation = DriverReservation::acquire()?;
            let (coordinator, lifecycle) = build()?;
            let emergency = coordinator
                .emergency
                .as_ref()
                .map(|latch| EmergencyStop(Rc::downgrade(latch)));
            Ok(DriverHandle {
                emergency,
                lifecycle: lifecycle.clone(),
                driver: Rc::new(RefCell::new(Self {
                    coordinator,
                    timer: None,
                    scheduler,
                    _reservation: reservation,
                    lifecycle,
                })),
            })
        }

        pub(super) fn turn<T>(
            handle: &DriverHandle<Self>,
            action: impl FnOnce(&mut Coordinator<P, C, A>) -> Result<T>,
        ) -> Result<T> {
            Self::run_turn(&handle.driver, action)
        }

        fn run_turn<T>(
            owner: &Rc<RefCell<Self>>,
            action: impl FnOnce(&mut Coordinator<P, C, A>) -> Result<T>,
        ) -> Result<T> {
            let mut driver = owner.try_borrow_mut().map_err(|_| Error::Scheduler)?;
            // No timer remains registered across a native call or an owner turn.
            driver.timer = None;
            driver.service_lifecycle();
            driver.coordinator.service_emergency();
            let result = match catch_unwind(AssertUnwindSafe(|| action(&mut driver.coordinator))) {
                Ok(result) => result,
                Err(payload) => {
                    std::mem::forget(payload);
                    let error = driver.coordinator.fail(Error::Scheduler);
                    // An unwind can interrupt the return from port.request.
                    // Shutdown must still reach the port's own exact flight.
                    let _ = driver.coordinator.shutdown();
                    Err(error)
                }
            };
            let drained = driver.service_lifecycle() | driver.coordinator.service_emergency();
            let consumed = Self::rearm(owner, &mut driver)?;
            // Clock/scheduler hooks can reenter while the driver is borrowed.
            // A consumed one-shot from that turn is replaced by the Stop wake.
            let mut during_arm =
                driver.service_lifecycle() | driver.coordinator.service_emergency();
            if during_arm || consumed {
                driver.timer = None;
                if Self::rearm(owner, &mut driver)? {
                    return Err(Self::scheduler_failed(&mut driver, Error::Scheduler));
                }
                during_arm |= driver.service_lifecycle() | driver.coordinator.service_emergency();
            }
            if drained || during_arm {
                return result.and(Err(Error::Closed));
            }
            result
        }

        fn service_lifecycle(&mut self) -> bool {
            let Some(lifecycle) = &self.lifecycle else {
                return false;
            };
            let was_revoked = self.coordinator.revoked;
            if lifecycle.0.pending() {
                lifecycle.service_host(&self.coordinator.clock.now());
            }
            if lifecycle.0.quit.get() {
                // This can run after successful ordinary timer registration.
                // Remove that timer before shutdown hands the slot to retirement.
                self.timer = None;
                let _ = self.coordinator.shutdown();
            }
            !was_revoked && self.coordinator.revoked
        }

        fn rearm(owner: &Rc<RefCell<Self>>, driver: &mut Self) -> Result<bool> {
            if let Some(wake) = driver.coordinator.wake.clone() {
                let delay = wake.deadline.as_ref().map_or(Ok(wake.delay), |deadline| {
                    // Reinstalling a timer after a rejected owner operation
                    // must not reset the original scheduled wake.
                    let now = driver.coordinator.clock.now();
                    driver.coordinator.logical.check_time(&now)?;
                    deadline.remaining(&now).map_err(Error::from)
                });
                let weak = Rc::downgrade(owner);
                let busy = Rc::new(Cell::new(false));
                let delivered = Rc::clone(&busy);
                let timer = delay.and_then(|delay| {
                    match catch_unwind(AssertUnwindSafe(|| {
                        driver.scheduler.arm(delay, move || {
                            if let Some(owner) = weak.upgrade() {
                                delivered.set(Self::on_timer(&owner, &wake));
                            }
                        })
                    })) {
                        Ok(result) => result,
                        Err(payload) => {
                            std::mem::forget(payload);
                            Err(Error::Scheduler)
                        }
                    }
                });
                match timer {
                    Ok(timer) => driver.timer = Some(timer),
                    Err(error) => return Err(Self::scheduler_failed(driver, error)),
                }
                return Ok(busy.get());
            }
            Ok(false)
        }

        fn scheduler_failed(driver: &mut Self, error: Error) -> Error {
            driver.timer = None;
            driver.service_lifecycle();
            driver.coordinator.fail(error);
            driver.coordinator.wake = None;
            let _ = driver.coordinator.shutdown();
            driver.service_lifecycle();
            error
        }

        fn on_timer(owner: &Rc<RefCell<Self>>, wake: &Wake) -> bool {
            let current = match owner.try_borrow() {
                Ok(driver) => driver.coordinator.is_current(wake),
                Err(_) => return true,
            };
            if current {
                let _ = Self::run_turn(owner, |coordinator| coordinator.wake(wake));
            }
            false
        }
    }

    impl<
            P: SpeechOutput,
            C: MonotonicClock,
            A: Fn(ContractIdentity) -> Option<Validation>,
            S: Scheduler,
        > Drop for Driver<P, C, A, S>
    {
        fn drop(&mut self) {
            self.timer = None;
            if let Some(lifecycle) = &self.lifecycle {
                lifecycle.service_host(&self.coordinator.clock.now());
            }
            let _ = self.coordinator.shutdown();
            if let Some(lifecycle) = &self.lifecycle {
                lifecycle.service_host(&self.coordinator.clock.now());
            }
        }
    }

    struct CleanupBudget {
        started: MonotonicTime,
        last: MonotonicTime,
        polls_left: usize,
    }

    impl CleanupBudget {
        fn new(now: MonotonicTime) -> Self {
            Self {
                started: now.clone(),
                last: now,
                polls_left: MAX_STOP_POLLS,
            }
        }

        fn next_delay(&self, now: &MonotonicTime) -> Option<Duration> {
            now.checked_duration_since(&self.last).ok()?;
            let elapsed = now.checked_duration_since(&self.started).ok()?;
            let remaining = (POLL_INTERVAL * MAX_STOP_POLLS as u32).saturating_sub(elapsed);
            (self.polls_left > 0 && !remaining.is_zero()).then_some(remaining.min(POLL_INTERVAL))
        }

        fn poll(&mut self, now: MonotonicTime) -> bool {
            if self.next_delay(&now).is_none() {
                return false;
            }
            self.last = now;
            self.polls_left -= 1;
            true
        }
    }

    /// Lives from before the factory through destruction of the actual port.
    /// Temporarily removing a port from a slot cannot make it look absent.
    struct PortLease(Rc<Cell<bool>>);

    impl Drop for PortLease {
        fn drop(&mut self) {
            self.0.set(false);
        }
    }

    struct OwnedPort {
        port: Box<dyn RetirementOutput>,
        _lease: PortLease,
    }

    impl std::ops::Deref for OwnedPort {
        type Target = dyn RetirementOutput;

        fn deref(&self) -> &Self::Target {
            &*self.port
        }
    }

    impl std::ops::DerefMut for OwnedPort {
        fn deref_mut(&mut self) -> &mut Self::Target {
            &mut *self.port
        }
    }

    /// Independently provisioned by the modeled fixture at owner setup.
    /// Resolution never accepts a caller-supplied issuer or raw observation.
    #[derive(Clone, Default)]
    pub(super) struct ModeledReleaseIssuer(Rc<()>);

    #[derive(Clone, Copy)]
    pub(super) enum ModeledReleaseAssertion {
        Released,
        Unknown,
    }

    #[derive(Clone)]
    pub(super) struct ReleaseChallenge {
        pub(super) flight: FlightBinding,
        pub(super) ticket: CleanupTicket,
    }

    pub(super) struct ModeledReleaseEvidence {
        issuer: Rc<()>,
        challenge: ReleaseChallenge,
        consumed: Cell<bool>,
    }

    #[cfg(test)]
    impl ModeledReleaseEvidence {
        pub(super) fn was_consumed(&self) -> bool {
            self.consumed.get()
        }
    }

    impl ModeledReleaseIssuer {
        pub(super) fn issue(
            &self,
            assertion: ModeledReleaseAssertion,
            challenge: ReleaseChallenge,
        ) -> Option<ModeledReleaseEvidence> {
            match assertion {
                ModeledReleaseAssertion::Released => Some(ModeledReleaseEvidence {
                    issuer: self.0.clone(),
                    challenge,
                    consumed: Cell::new(false),
                }),
                ModeledReleaseAssertion::Unknown => None,
            }
        }
    }

    struct ReleaseAuthority {
        issuer: ModeledReleaseIssuer,
        host: OutputAckAuthority,
    }

    struct StoredRetirement {
        flight: FlightBinding,
        report: RetirementReport,
    }

    pub(super) struct ReportAccounting(RetirementFacts);

    impl ReportAccounting {
        pub(super) fn facts(&self) -> RetirementFacts {
            self.0
        }

        pub(super) fn can_abandon_unsubmitted(&self) -> bool {
            self.0.uncertainty == RetirementUncertainty::NoSubmissionOrDelegate
        }
    }

    struct Retiring {
        port: OwnedPort,
        budget: CleanupBudget,
        emergency: Rc<StopLatch>,
    }

    /// One owner survives each driver and its scheduler registration. The
    /// obligation is published before calling the port, then only an exact
    /// never-submitted report can undo it. A submitted flight stays blocked
    /// even after all bridge resources and the driver have been dropped.
    pub(super) struct CleanupOwner<C: MonotonicClock, S: Scheduler> {
        run: AppRunId,
        scope: RefCell<OutputScope>,
        clock: C,
        scheduler: S,
        timer: RefCell<Option<S::Armed>>,
        wake: RefCell<Option<Rc<()>>>,
        retiring: RefCell<Option<Retiring>>,
        obligation: RefCell<Option<FlightBinding>>,
        report: RefCell<Option<StoredRetirement>>,
        accounting: Cell<Option<RetirementFacts>>,
        port_live: Rc<Cell<bool>>,
        attempted: Cell<bool>,
        generation_fault: Cell<bool>,
        lifecycle: Option<Rc<LifecycleOwner>>,
        release_authority: Option<ReleaseAuthority>,
    }

    impl<C: MonotonicClock + Clone + 'static, S: Scheduler + Clone + 'static> CleanupOwner<C, S> {
        pub(super) fn new(run: AppRunId, route: Generation, clock: C, scheduler: S) -> Rc<Self> {
            Self::with_host(run, route, clock, scheduler, None)
        }

        pub(super) fn with_host(
            run: AppRunId,
            route: Generation,
            clock: C,
            scheduler: S,
            host: Option<Rc<SpeechLifecycleHost>>,
        ) -> Rc<Self> {
            Self::build(run, route, clock, scheduler, host, None)
        }

        pub(super) fn with_release_issuer(
            run: AppRunId,
            route: Generation,
            clock: C,
            scheduler: S,
            host: (Rc<SpeechLifecycleHost>, OutputAckAuthority),
            issuer: ModeledReleaseIssuer,
        ) -> Rc<Self> {
            Self::build(
                run,
                route,
                clock,
                scheduler,
                Some(host.0),
                Some(ReleaseAuthority {
                    issuer,
                    host: host.1,
                }),
            )
        }

        fn build(
            run: AppRunId,
            route: Generation,
            clock: C,
            scheduler: S,
            host: Option<Rc<SpeechLifecycleHost>>,
            release_authority: Option<ReleaseAuthority>,
        ) -> Rc<Self> {
            Rc::new(Self {
                lifecycle: host.map(|host| {
                    Rc::new(LifecycleOwner {
                        host,
                        route: Cell::new(RouteAvailability::Known),
                        current: RefCell::new(None),
                        confirmed: RefCell::new(None),
                    })
                }),
                run,
                scope: RefCell::new(OutputScope::new(run, route)),
                clock,
                scheduler,
                timer: RefCell::new(None),
                wake: RefCell::new(None),
                retiring: RefCell::new(None),
                obligation: RefCell::new(None),
                report: RefCell::new(None),
                accounting: Cell::new(None),
                port_live: Rc::new(Cell::new(false)),
                attempted: Cell::new(false),
                generation_fault: Cell::new(false),
                release_authority,
            })
        }

        fn check_quiescent(&self) -> Result<()> {
            if self.generation_fault.get() {
                return Err(Error::Closed);
            }
            if self.port_live.get()
                || self.accounting.get().is_some()
                || self
                    .obligation
                    .try_borrow()
                    .map_err(|_| Error::Busy)?
                    .is_some()
                || self
                    .retiring
                    .try_borrow()
                    .map_err(|_| Error::Busy)?
                    .is_some()
                || self.report.try_borrow().map_err(|_| Error::Busy)?.is_some()
            {
                return Err(Error::Busy);
            }
            Ok(())
        }

        pub(super) fn prepare_modeled_enable(
            &self,
            session: ContractIdentity,
            lifetime: Duration,
        ) -> Result<ModeledEnablePreview> {
            let now = self.clock.now();
            let lifecycle = self.lifecycle.as_ref().ok_or(Error::Unauthorized)?;
            lifecycle.service(&now);
            self.check_quiescent()?;
            lifecycle.ready()?;
            let mut confirmed = lifecycle
                .confirmed
                .try_borrow_mut()
                .map_err(|_| Error::Busy)?;
            if let Some(previous) = confirmed.as_ref() {
                match lifecycle.check_confirmed(previous, previous.receipt.session, &now) {
                    Ok(()) | Err(Error::Busy) => return Err(Error::Busy),
                    Err(_) => {
                        lifecycle
                            .host
                            .discard_speech_enable(&previous.receipt.host)
                            .map_err(host_error)?;
                        *confirmed = None;
                    }
                }
            }
            let (scope, sequence) = lifecycle.lineage()?;
            let host = lifecycle
                .host
                .prepare_speech_enable(session, lifetime, &now)
                .map_err(host_error)?;
            Ok(ModeledEnablePreview {
                host,
                owner: Rc::downgrade(lifecycle),
                sequence,
                session,
                scope,
            })
        }

        pub(super) fn confirm_modeled_enable(&self, receipt: &ModeledEnablePreview) -> Result<()> {
            let now = self.clock.now();
            let lifecycle = self.lifecycle.as_ref().ok_or(Error::Unauthorized)?;
            lifecycle.service(&now);
            self.check_quiescent()?;
            lifecycle.ready()?;
            if !Weak::ptr_eq(&receipt.owner, &Rc::downgrade(lifecycle)) {
                return Err(Error::Unauthorized);
            }
            let mut confirmed = lifecycle
                .confirmed
                .try_borrow_mut()
                .map_err(|_| Error::Busy)?;
            if confirmed.is_some() {
                return Err(Error::Unauthorized);
            }
            match lifecycle.check_confirmation(receipt, receipt.session, &now) {
                Ok(()) => {
                    *confirmed = Some(ConfirmedModeledEnable {
                        receipt: receipt.clone(),
                        confirmed_at: now,
                    });
                    Ok(())
                }
                Err(Error::Busy) => Err(Error::Busy),
                Err(error) => {
                    lifecycle
                        .host
                        .discard_speech_enable(&receipt.host)
                        .map_err(host_error)?;
                    Err(error)
                }
            }
        }

        pub(super) fn open_driver<A>(
            self: &Rc<Self>,
            session: ContractIdentity,
            validate: A,
            authority: LocalAuthority,
            open_port: impl FnOnce(OutputScope) -> Result<Box<dyn RetirementOutput>>,
        ) -> Result<DriverHandle<Driver<ManagedOutput<C, S>, C, A, S>>>
        where
            A: Fn(ContractIdentity) -> Option<Validation> + 'static,
        {
            if session.app_run_id != self.run || authority.0.lifecycle_bound.get() {
                return Err(Error::Unauthorized);
            }
            self.check_quiescent()?;
            Driver::new_bound(
                || {
                    // Hold the lifetime witness through the factory and its
                    // destructor, including reentrant admission/resolution.
                    self.port_live.set(true);
                    let lease = PortLease(self.port_live.clone());
                    let now = self.clock.now();
                    let route = if let Some(lifecycle) = &self.lifecycle {
                        lifecycle.service(&now);
                        lifecycle.check_admission(session, &authority, &now)?;
                        lifecycle.route.get()
                    } else {
                        RouteAvailability::Known
                    };
                    let on_overflow = |error| {
                        self.generation_fault.set(true);
                        Error::Native(error)
                    };
                    let candidate = {
                        let mut base = self.scope.try_borrow_mut().map_err(|_| Error::Busy)?;
                        let next = base.next_owner().map_err(on_overflow)?;
                        let candidate = if route == RouteAvailability::Restored {
                            next.next_modeled_route().map_err(on_overflow)?
                        } else {
                            next.clone()
                        };
                        // Burn a factory-visible epoch even if the factory
                        // fails. Keep the route revision pending until commit.
                        *base = next;
                        candidate
                    };
                    let emergency = Rc::new(StopLatch::new(candidate.clone(), session));
                    let port = open_port(candidate.clone())?;
                    let managed = ManagedOutput {
                        owner: Rc::clone(self),
                        scope: candidate.clone(),
                        port: Some(OwnedPort {
                            port,
                            _lease: lease,
                        }),
                        budget: None,
                        emergency: Rc::clone(&emergency),
                    };
                    let mut coordinator = Coordinator::new(
                        managed,
                        self.clock.clone(),
                        validate,
                        authority,
                        self.run,
                        session.interaction_id,
                    )?;
                    let after = self.clock.now();
                    after.checked_duration_since(&now)?;
                    coordinator.logical.check_time(&after)?;
                    let mut base = self.scope.try_borrow_mut().map_err(|_| Error::Busy)?;
                    let lifecycle = if let Some(lifecycle) = &self.lifecycle {
                        lifecycle.service(&after);
                        if lifecycle.route.get() != route {
                            return Err(Error::Closed);
                        }
                        let (ingress, deadline) =
                            lifecycle.bind(session, &coordinator.authority, &emergency, &after)?;
                        coordinator.host_deadline = Some(deadline);
                        lifecycle.route.set(RouteAvailability::Known);
                        Some(ingress)
                    } else {
                        None
                    };
                    *base = candidate.clone();
                    self.attempted.set(false);
                    coordinator.emergency = Some(emergency);
                    coordinator.output_scope = candidate;
                    Ok((coordinator, lifecycle))
                },
                self.scheduler.clone(),
            )
        }

        #[cfg(test)]
        pub(super) fn exhaust_modeled_epoch(&self) {
            self.scope.borrow_mut().exhaust_epoch();
        }

        fn collect(&self, port: &mut dyn RetirementOutput) -> bool {
            let Ok(mut obligation) = self.obligation.try_borrow_mut() else {
                return false;
            };
            let Ok(mut slot) = self.report.try_borrow_mut() else {
                return false;
            };
            if slot.is_some() || self.accounting.get().is_some() {
                return false;
            }
            let Some(flight) = obligation.as_ref() else {
                return false;
            };
            let Some(report) = port.take_retirement_report() else {
                return false;
            };
            if !report.matches(flight) {
                return false;
            }
            let flight = flight.clone();
            if report.can_abandon_unsubmitted() {
                *obligation = None;
            }
            self.accounting.set(Some(report.facts()));
            *slot = Some(StoredRetirement { flight, report });
            true
        }

        /// Diagnostic accounting remains take-once. A registered resolver
        /// retains the sole authoritative report until coordinated resolution.
        pub(super) fn take_report(&self) -> Option<ReportAccounting> {
            let mut report = self.report.try_borrow_mut().ok()?;
            let facts = self.accounting.take()?;
            if self.release_authority.is_none() {
                report.take();
            }
            Some(ReportAccounting(facts))
        }

        pub(super) fn release_challenge(&self) -> Result<ReleaseChallenge> {
            let lifecycle = self.lifecycle.as_ref().ok_or(Error::Unauthorized)?;
            let current = lifecycle.current.try_borrow().map_err(|_| Error::Busy)?;
            let ticket = current
                .as_ref()
                .and_then(|scope| scope.cleanup_ticket.get())
                .ok_or(Error::Unauthorized)?
                .clone();
            let slot = self.obligation.try_borrow().map_err(|_| Error::Busy)?;
            let flight = slot.as_ref().ok_or(Error::Unauthorized)?.clone();
            Ok(ReleaseChallenge { flight, ticket })
        }

        pub(super) fn resolve_output(
            &self,
            ticket: &CleanupTicket,
            evidence: Option<&ModeledReleaseEvidence>,
        ) -> Result<()> {
            let authority = self.release_authority.as_ref().ok_or(Error::Unauthorized)?;
            let lifecycle = self.lifecycle.as_ref().ok_or(Error::Unauthorized)?;
            let current = lifecycle.current.try_borrow().map_err(|_| Error::Busy)?;
            let scope = current
                .as_ref()
                .filter(|scope| scope.live.get() && scope.revoked.get())
                .ok_or(Error::Unauthorized)?;

            // The lifetime witness covers construction, handoff, polling,
            // and resource destruction even if all storage slots look empty.
            let retiring = self.retiring.try_borrow_mut().map_err(|_| Error::Busy)?;
            if self.port_live.get() || retiring.is_some() || scope.pending() {
                return Err(Error::Busy);
            }
            if !scope
                .cleanup_ticket
                .get()
                .is_some_and(|bound| bound.matches(ticket))
            {
                return Err(Error::Unauthorized);
            }
            let output = self.scope.try_borrow().map_err(|_| Error::Busy)?;
            let mut obligation = self.obligation.try_borrow_mut().map_err(|_| Error::Busy)?;
            let mut report = self.report.try_borrow_mut().map_err(|_| Error::Busy)?;
            let host = lifecycle
                .host
                .prepare_output_ack(&authority.host, ticket)
                .map_err(host_error)?;

            match (obligation.as_ref(), report.as_ref(), evidence) {
                (None, None, None) if !self.attempted.get() => (),
                (None, Some(stored), None)
                    if stored.report.can_abandon_unsubmitted()
                        && output.accepts(&stored.flight)
                        && native_speech::same_session(scope.session, stored.flight.identity()) =>
                {
                    ()
                }
                (Some(flight), Some(stored), Some(evidence))
                    if output.accepts(flight)
                        && native_speech::same_session(scope.session, flight.identity())
                        && stored.report.matches(flight)
                        && !evidence.consumed.get()
                        && Rc::ptr_eq(&authority.issuer.0, &evidence.issuer)
                        && evidence.challenge.flight.matches(flight)
                        && evidence.challenge.ticket.matches(ticket)
                        && Self::confirmed_retirement(stored.report.facts()) =>
                {
                    ()
                }
                _ => return Err(Error::Unauthorized),
            }

            // All guards and validation precede this commit. These operations
            // perform no callbacks and cannot fail.
            if let Some(evidence) = evidence {
                evidence.consumed.set(true);
            }
            *obligation = None;
            *report = None;
            host.commit();
            Ok(())
        }

        pub(super) fn confirmed_retirement(facts: RetirementFacts) -> bool {
            facts.submission == Submission::PossiblySubmitted
                && facts.uncertainty == RetirementUncertainty::OutputReleaseUnverified
                && facts.detach == Detach::VerifiedNil
                && facts.resources == ResourceRetirement::OwnedFlightRelinquished
                && match facts.terminal {
                    // Native facts cannot establish whether a Finish buffered
                    // around Stop preceded it. Fail closed in that case.
                    Terminal::Finished => facts.first_stop.is_none(),
                    Terminal::Cancelled {
                        stop_requested: true,
                    } => facts.first_stop.is_some(),
                    Terminal::Cancelled {
                        stop_requested: false,
                    } => true,
                    Terminal::Unresolved => false,
                }
        }

        #[cfg(test)]
        pub(super) fn while_release_slot_busy(&self, report: bool, action: impl FnOnce()) {
            if report {
                let _held = self.report.borrow_mut();
                action();
            } else {
                let _held = self.obligation.borrow_mut();
                action();
            }
        }

        #[cfg(test)]
        pub(super) fn modeled_scope(&self) -> OutputScope {
            self.scope.borrow().clone()
        }

        #[cfg(test)]
        pub(super) fn while_retaining_slot_is_borrowed(&self, action: impl FnOnce()) {
            let _slot = self.retiring.borrow_mut();
            action();
        }

        fn finish(&self, mut port: OwnedPort) {
            port.shutdown();
            self.collect(&mut *port);
        }

        fn handoff(
            self: &Rc<Self>,
            mut port: OwnedPort,
            budget: CleanupBudget,
            emergency: Rc<StopLatch>,
        ) -> Option<Notice> {
            let attempt = port.begin_retirement();
            // The bridge already attempted Stop for this port (or had no
            // active flight). A reentrant intent needs no second native attempt.
            emergency.service();
            if attempt.is_none() || budget.next_delay(&self.clock.now()).is_none() {
                self.finish(port);
                emergency.service();
                return attempt;
            }
            // No native calls run while borrowing the retained-port slot. On a
            // failed handoff, finish the incoming port without touching the
            // existing port or wake. The prepublished obligation still blocks.
            let installed = self
                .retiring
                .try_borrow_mut()
                .ok()
                .filter(|slot| slot.is_none());
            if let Some(mut slot) = installed {
                *slot = Some(Retiring {
                    port,
                    budget,
                    emergency,
                });
                drop(slot);
                self.schedule_retirement();
            } else {
                self.finish(port);
                emergency.service();
            }
            attempt
        }

        fn schedule_retirement(self: &Rc<Self>) {
            let delay = self
                .retiring
                .borrow()
                .as_ref()
                .and_then(|retiring| retiring.budget.next_delay(&self.clock.now()));
            if let Some(delay) = delay {
                let weak = Rc::downgrade(self);
                let wake = Rc::new(());
                *self.wake.borrow_mut() = Some(Rc::clone(&wake));
                let registration = Rc::new(Cell::new(false));
                let delivery = Rc::clone(&registration);
                let timer = catch_unwind(AssertUnwindSafe(|| {
                    self.scheduler.arm(delay, move || {
                        // Unexpected inline delivery consumes the wake. Let
                        // the registering owner dispose instead of recursing.
                        if delivery.replace(true) {
                            if let Some(owner) = weak.upgrade() {
                                owner.poll_retirement(&wake);
                            }
                        }
                    })
                }));
                let inline = registration.replace(true);
                match timer {
                    Ok(Ok(timer)) if !inline => {
                        *self.timer.borrow_mut() = Some(timer);
                        return;
                    }
                    Ok(_) => (),
                    Err(payload) => std::mem::forget(payload),
                }
            }
            self.wake.borrow_mut().take();
            let retiring = self.retiring.borrow_mut().take();
            if let Some(retiring) = retiring {
                self.finish(retiring.port);
                retiring.emergency.service();
            }
            if let Some(lifecycle) = &self.lifecycle {
                lifecycle.service(&self.clock.now());
            }
        }

        fn poll_retirement(self: &Rc<Self>, wake: &Rc<()>) {
            {
                let mut current = self.wake.borrow_mut();
                if !current
                    .as_ref()
                    .is_some_and(|current| Rc::ptr_eq(current, wake))
                {
                    return;
                }
                current.take();
            }
            self.timer.borrow_mut().take();
            let retiring = self.retiring.borrow_mut().take();
            let Some(mut retiring) = retiring else { return };
            if let Some(lifecycle) = &self.lifecycle {
                lifecycle.service(&self.clock.now());
            }
            retiring.emergency.service();
            if retiring.budget.poll(self.clock.now()) {
                retiring.port.poll();
                retiring.emergency.service();
                if self.collect(&mut *retiring.port) {
                    return;
                }
                *self.retiring.borrow_mut() = Some(retiring);
                self.schedule_retirement();
            } else {
                self.finish(retiring.port);
                retiring.emergency.service();
            }
        }
    }

    pub(super) struct ManagedOutput<
        C: MonotonicClock + Clone + 'static,
        S: Scheduler + Clone + 'static,
    > {
        owner: Rc<CleanupOwner<C, S>>,
        scope: OutputScope,
        port: Option<OwnedPort>,
        budget: Option<CleanupBudget>,
        emergency: Rc<StopLatch>,
    }

    impl<C: MonotonicClock + Clone + 'static, S: Scheduler + Clone + 'static> ManagedOutput<C, S> {
        fn start_budget(&mut self) {
            self.budget
                .get_or_insert_with(|| CleanupBudget::new(self.owner.clock.now()));
        }
    }

    impl<C: MonotonicClock + Clone + 'static, S: Scheduler + Clone + 'static> SpeechOutput
        for ManagedOutput<C, S>
    {
        fn request(
            &mut self,
            binding: FlightBinding,
            sequence: u64,
            text: &str,
            voice: &VoiceId,
            admitted: &MonotonicTime,
            check: SubmissionCheck<'_>,
        ) -> native_speech::Result<Notice> {
            let port = self.port.as_mut().ok_or(native_speech::Error::Closed)?;
            if !self.scope.accepts(&binding) {
                return Err(native_speech::Error::StaleJob);
            }
            // Sample before publishing the obligation or entering native code.
            let before = self.owner.clock.now();
            if self.emergency.revoked.get() {
                return Err(native_speech::Error::Closed);
            }
            {
                let mut obligation = self
                    .owner
                    .obligation
                    .try_borrow_mut()
                    .map_err(|_| native_speech::Error::Busy)?;
                if obligation.is_some()
                    || self.owner.accounting.get().is_some()
                    || self
                        .owner
                        .report
                        .try_borrow()
                        .map_err(|_| native_speech::Error::Busy)?
                        .is_some()
                {
                    return Err(native_speech::Error::Busy);
                }
                self.owner.attempted.set(true);
                *obligation = Some(binding.clone());
            }
            let check = check.with_obligation(&self.owner.obligation);
            let result = port.request(binding, sequence, text, voice, admitted, check);
            if matches!(
                result,
                Ok(Notice {
                    evidence: Evidence::Failure(_),
                    ..
                })
            ) {
                self.budget = Some(CleanupBudget::new(before));
            }
            self.owner.collect(&mut **port);
            result
        }

        fn stop(
            &mut self,
            identity: ContractIdentity,
            origin: StopOrigin,
        ) -> native_speech::Result<Notice> {
            if !self
                .owner
                .obligation
                .try_borrow()
                .map_err(|_| native_speech::Error::Busy)?
                .as_ref()
                .is_some_and(|binding| binding.identity() == identity)
            {
                return Err(native_speech::Error::StaleJob);
            }
            self.start_budget();
            self.port
                .as_mut()
                .ok_or(native_speech::Error::Closed)?
                .stop(identity, origin)
        }

        fn poll(&mut self) -> Option<Notice> {
            if let Some(budget) = &mut self.budget {
                if !budget.poll(self.owner.clock.now()) {
                    if let Some(port) = self.port.take() {
                        self.owner.finish(port);
                    }
                    return None;
                }
            }
            let before = self.owner.clock.now();
            let port = self.port.as_mut()?;
            let notice = port.poll();
            if matches!(
                notice,
                Some(Notice {
                    evidence: Evidence::Failure(_),
                    ..
                })
            ) && self.budget.is_none()
            {
                self.budget = Some(CleanupBudget::new(before));
            }
            self.owner.collect(&mut **port);
            // A final permitted poll must retire here. The coordinator may
            // exhaust its own scheduling budget without scheduling again.
            if self
                .budget
                .as_ref()
                .is_some_and(|budget| budget.next_delay(&self.owner.clock.now()).is_none())
            {
                if let Some(port) = self.port.take() {
                    self.owner.finish(port);
                }
            }
            notice
        }

        fn shutdown(&mut self) -> Option<Notice> {
            let port = self.port.take()?;
            let budget = self
                .budget
                .take()
                .unwrap_or_else(|| CleanupBudget::new(self.owner.clock.now()));
            self.owner.handoff(port, budget, Rc::clone(&self.emergency))
        }
    }

    impl<C: MonotonicClock + Clone + 'static, S: Scheduler + Clone + 'static> Drop
        for ManagedOutput<C, S>
    {
        fn drop(&mut self) {
            // Fallback for a failed construction or unwinding driver teardown.
            // The obligation was recorded before native submission.
            if let Some(port) = self.port.take() {
                self.owner.finish(port);
                self.emergency.service();
            }
        }
    }

    thread_local! {
        // One root per native main thread. A replacement driver cannot create
        // another root or discard the old obligation. Host wiring is deferred.
        static NATIVE_OWNER: RefCell<Option<Rc<CleanupOwner<SharedClock, MainRunLoop>>>> =
            const { RefCell::new(None) };
    }

    fn open<A>(
        main: MainThreadMarker,
        session: ContractIdentity,
        validate: A,
        authority: LocalAuthority,
    ) -> Result<
        DriverHandle<Driver<ManagedOutput<SharedClock, MainRunLoop>, SharedClock, A, MainRunLoop>>,
    >
    where
        A: Fn(ContractIdentity) -> Option<Validation> + 'static,
    {
        let run = session.app_run_id;
        let owner = NATIVE_OWNER
            .try_with(|slot| {
                let mut slot = slot.try_borrow_mut().map_err(|_| Error::Busy)?;
                let owner = slot.get_or_insert_with(|| {
                    CleanupOwner::new(
                        run,
                        Generation::from_native_sequence(1).expect("nonzero modeled route"),
                        SharedClock(Rc::new(NativeMonotonicClock::new())),
                        MainRunLoop(main),
                    )
                });
                if owner.run != run {
                    return Err(Error::Closed);
                }
                Ok(Rc::clone(owner))
            })
            .map_err(|_| Error::Scheduler)??;
        let clock = owner.clock.clone();
        owner.open_driver(session, validate, authority, |scope| {
            let port = native_speech::open(main, clock, scope).map_err(Error::Native)?;
            Ok(Box::new(port))
        })
    }

    #[test]
    fn main_driver_links_without_installing_timers_or_opening_the_port() {
        type Link = fn(MainThreadMarker, ContractIdentity, VoiceId) -> Result<()>;
        let entry: Link = |main, identity, voice| {
            let driver = open(main, identity, |_| None, Default::default())?;
            let emergency = driver.emergency_stop().ok_or(Error::Protocol)?;
            Driver::turn(&driver, |coordinator| {
                coordinator.admit(identity, 1, String::new(), voice)
            })?;
            Driver::turn(&driver, Coordinator::submit)?;
            let _ = emergency.record();
            Driver::turn(&driver, Coordinator::stop)?;
            Driver::turn(&driver, Coordinator::shutdown)
        };
        let _ = std::hint::black_box(entry);
    }
}

#[cfg(test)]
#[path = "speech_coordinator_tests.rs"]
mod tests;
