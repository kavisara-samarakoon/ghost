//! Test-only coordination of the Phase 1 speech queue and a single native port.
//! Authority checks are supplied by the caller. Neither a wake nor a native
//! observation can grant authority or initiate the next job.
use super::{Callback, Event, Job, Preparation, SpeechController, SpeechSlot, Update, Validation};
use crate::voice_conversation::clock::{Deadline, MonotonicClock, MonotonicTime};
use crate::voice_conversation::model::{AppRunId, ContractIdentity, InteractionId, SpeechState};
use crate::voice_conversation::native_speech::{self, Evidence, Notice, SpeechOutput, VoiceId};
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
    clock: C,
    validate: A,
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
        run: AppRunId,
        interaction: InteractionId,
    ) -> Result<Self> {
        Ok(Self {
            logical: SpeechController::new(run, interaction, clock.now())?,
            port,
            clock,
            validate,
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

    fn check_open(&self) -> Result<()> {
        if self.revoked {
            return Err(Error::Closed);
        }
        Ok(())
    }

    fn validation(&self, identity: ContractIdentity) -> Option<Validation> {
        (self.validate)(identity)
            .filter(|value| value.identity == identity && value.allowed(Preparation::Local))
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
        let validation = self.validation(identity).ok_or(Error::Unauthorized)?;
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
        Ok(())
    }

    /// Starts only the current active logical job, on an explicit coordinator
    /// turn. Native preparation is synchronous. The port has no hook to recheck
    /// outside authorization immediately before its final speak call.
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

        self.cleanup = Cleanup::Unresolved;
        let notice = match self.port.request(
            binding.identity,
            admission.sequence,
            &job.text,
            &admission.voice,
            &admission.admitted,
        ) {
            Ok(notice) => notice,
            Err(error) => return Err(self.fail(Error::Native(error))),
        };
        // Even a submission failure reported as a notice can own a native flight.
        self.flight = Some(Flight {
            binding: binding.clone(),
            stop: None,
        });
        if notice.identity != binding.identity {
            return Err(self.fail(Error::Protocol));
        }
        match notice.evidence {
            Evidence::Requested => {
                // Re-sample after synchronous preparation to schedule against
                // time remaining, without changing the original admission.
                let after = self.clock.now();
                self.refresh(&after)?;
                if !self.revoked {
                    self.schedule(&after)?;
                }
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
        self.logical.check_time(now)?;
        let active_identity = self.logical.callback().map(|binding| binding.identity);
        let active = self
            .logical
            .active
            .as_ref()
            .and_then(|job| self.validation(job.callback.identity));
        let pending = self
            .logical
            .pending
            .as_ref()
            .and_then(|job| self.validation(job.callback.identity));
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
        if !self.is_current(wake) {
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
            let Some(notice) = self.port.poll() else {
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
        self.fault.get_or_insert(error);
        let _ = self.stop();
        error
    }

    fn stop(&mut self) -> Result<Option<Notice>> {
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
            let attempt = match self.port.stop(identity) {
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
            return Ok(());
        }
        self.shutdown = true;
        let result = self.stop();
        self.wake = None;
        if let Some(notice) = self.port.shutdown() {
            self.observe_cancellation(notice);
        }
        self.flight = None;
        result.map(|_| ())
    }
}

#[cfg(target_os = "macos")]
mod macos {
    use super::*;
    use crate::voice_conversation::clock::NativeMonotonicClock;
    use core_foundation::base::{kCFAllocatorDefault, TCFType};
    use core_foundation::date::CFAbsoluteTimeGetCurrent;
    use core_foundation::runloop::{
        kCFRunLoopCommonModes, CFRunLoop, CFRunLoopTimer, CFRunLoopTimerCreate,
        CFRunLoopTimerInvalidate, CFRunLoopTimerRef,
    };
    use objc2::MainThreadMarker;
    use std::cell::{Cell, RefCell};
    use std::ffi::c_void;
    use std::marker::PhantomData;
    use std::panic::{catch_unwind, AssertUnwindSafe};
    use std::rc::Rc;

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
        ) -> Result<Rc<RefCell<Self>>> {
            // Acquire before running any constructor that can open the port
            // or reenter on this thread. Failure drops the local reservation.
            let reservation = DriverReservation::acquire()?;
            let coordinator = build()?;
            Ok(Rc::new(RefCell::new(Self {
                coordinator,
                timer: None,
                scheduler,
                _reservation: reservation,
            })))
        }

        pub(super) fn turn<T>(
            owner: &Rc<RefCell<Self>>,
            action: impl FnOnce(&mut Coordinator<P, C, A>) -> Result<T>,
        ) -> Result<T> {
            let mut driver = owner.try_borrow_mut().map_err(|_| Error::Scheduler)?;
            // No timer remains registered across a native call or an owner turn.
            driver.timer = None;
            let result = match catch_unwind(AssertUnwindSafe(|| action(&mut driver.coordinator))) {
                Ok(result) => result,
                Err(payload) => {
                    std::mem::forget(payload);
                    Err(driver.coordinator.fail(Error::Scheduler))
                }
            };
            Self::rearm(owner, &mut driver)?;
            result
        }

        fn rearm(owner: &Rc<RefCell<Self>>, driver: &mut Self) -> Result<()> {
            if let Some(wake) = driver.coordinator.wake.clone() {
                let delay = wake.deadline.as_ref().map_or(Ok(wake.delay), |deadline| {
                    // Reinstalling a timer after a rejected owner operation
                    // must not reset the original scheduled wake.
                    let now = driver.coordinator.clock.now();
                    driver.coordinator.logical.check_time(&now)?;
                    deadline.remaining(&now).map_err(Error::from)
                });
                let weak = Rc::downgrade(owner);
                let timer = delay.and_then(|delay| {
                    driver.scheduler.arm(delay, move || {
                        if let Some(owner) = weak.upgrade() {
                            Self::on_timer(&owner, &wake);
                        }
                    })
                });
                match timer {
                    Ok(timer) => driver.timer = Some(timer),
                    Err(error) => {
                        driver.coordinator.fail(error);
                        driver.coordinator.wake = None;
                        return Err(error);
                    }
                }
            }
            Ok(())
        }

        fn on_timer(owner: &Rc<RefCell<Self>>, wake: &Wake) {
            let current = owner
                .try_borrow()
                .is_ok_and(|driver| driver.coordinator.is_current(wake));
            if current {
                let _ = Self::turn(owner, |coordinator| coordinator.wake(wake));
            }
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
            let _ = self.coordinator.shutdown();
        }
    }

    fn open<A>(
        main: MainThreadMarker,
        run: AppRunId,
        interaction: InteractionId,
        validate: A,
    ) -> Result<Rc<RefCell<Driver<impl SpeechOutput, SharedClock, A, MainRunLoop>>>>
    where
        A: Fn(ContractIdentity) -> Option<Validation> + 'static,
    {
        Driver::new(MainRunLoop(main), || {
            let clock = SharedClock(Rc::new(NativeMonotonicClock::new()));
            let port = native_speech::open(main, clock.clone(), run).map_err(Error::Native)?;
            Coordinator::new(port, clock, validate, run, interaction)
        })
    }

    #[test]
    fn main_driver_links_without_installing_timers_or_opening_the_port() {
        type Link = fn(MainThreadMarker, ContractIdentity, VoiceId) -> Result<()>;
        let entry: Link = |main, identity, voice| {
            let driver = open(main, identity.app_run_id, identity.interaction_id, |_| None)?;
            Driver::turn(&driver, |coordinator| {
                coordinator.admit(identity, 1, String::new(), voice)
            })?;
            Driver::turn(&driver, Coordinator::submit)?;
            Driver::turn(&driver, Coordinator::stop)?;
            Driver::turn(&driver, Coordinator::shutdown)
        };
        let _ = std::hint::black_box(entry);
    }
}

#[cfg(test)]
#[path = "speech_coordinator_tests.rs"]
mod tests;
