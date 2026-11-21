//! P2-01 native output port. The caller owns policy, scheduling and the Phase 1
//! queue. This test-only bridge owns at most one submitted utterance and reports
//! evidence as data. There is no callback into the caller or a follow-up action.
use super::clock::{Deadline, MonotonicClock, MonotonicTime};
use super::limits::{checked_total, MAX_SPEECH_JOB_TEXT_BYTES, SPEECH_OUTPUT_DEADLINE};
use super::model::{AppRunId, ContractIdentity, JobId, SpeechOutcome};
use super::ContractError;
use std::collections::HashSet;
use std::sync::{Arc, Mutex};

const MAX_VOICES: usize = 512;
const MAX_VOICE_ID_BYTES: usize = 512;
const MAX_VOICE_NAME_BYTES: usize = 256;
const MAX_VOICE_LANGUAGE_BYTES: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Error {
    Contract(ContractError),
    BackendUnavailable,
    NativeFailure,
    InvalidText,
    InvalidVoiceId,
    InvalidVoiceMetadata,
    VoiceUnavailable,
    Busy,
    StaleJob,
    Closed,
    Deadline,
    UnexpectedCallback,
}

impl From<ContractError> for Error {
    fn from(error: ContractError) -> Self {
        Self::Contract(error)
    }
}

type Result<T> = std::result::Result<T, Error>;

fn bounded_field(value: &str, max_bytes: usize) -> bool {
    !value.is_empty() && value.len() <= max_bytes && !value.chars().any(char::is_control)
}

/// An exact native string, with no normalization, parsing or fallback.
#[derive(Clone, PartialEq, Eq, Hash)]
pub(super) struct VoiceId(String);

impl VoiceId {
    pub(super) fn new(value: &str) -> Result<Self> {
        if !bounded_field(value, MAX_VOICE_ID_BYTES) {
            return Err(Error::InvalidVoiceId);
        }
        Ok(Self(value.to_owned()))
    }
}

pub(super) struct Voice {
    pub(super) id: VoiceId,
    pub(super) name: String,
    pub(super) language: String,
}

impl Voice {
    fn new(id: VoiceId, name: String, language: String) -> Result<Self> {
        if !bounded_field(&name, MAX_VOICE_NAME_BYTES)
            || !bounded_field(&language, MAX_VOICE_LANGUAGE_BYTES)
        {
            return Err(Error::InvalidVoiceMetadata);
        }
        Ok(Self { id, name, language })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum StopAttempt {
    Accepted,
    Rejected,
    Failed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Evidence {
    Requested,
    Started,
    Finished,
    Cancelled { stop_requested: bool },
    CancellationUnconfirmed(StopAttempt),
    Failure(Error),
}

impl Evidence {
    /// A terminal candidate for the caller's independent identity and policy
    /// checks. This port does not advance the Phase 1 controller.
    pub(super) fn outcome(self) -> Option<SpeechOutcome> {
        match self {
            Self::Finished => Some(SpeechOutcome::NormalCompletion),
            Self::Cancelled {
                stop_requested: true,
            } => Some(SpeechOutcome::Stop),
            Self::Cancelled {
                stop_requested: false,
            }
            | Self::Failure(_) => Some(SpeechOutcome::Failure),
            Self::Requested | Self::Started | Self::CancellationUnconfirmed(_) => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Notice {
    pub(super) identity: ContractIdentity,
    pub(super) evidence: Evidence,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct NativeObjects {
    synthesizer: usize,
    utterance: usize,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct CallbackKey {
    identity: ContractIdentity,
    objects: NativeObjects,
}

#[derive(Clone, Copy)]
enum Callback {
    Start,
    Finish,
    Cancel,
}

#[derive(Clone, Copy)]
enum Observation {
    Unsubmitted,
    Waiting,
    Started,
    Finished,
    Cancelled { stop_requested: bool },
    Invalid,
    Closed,
}

struct CallbackState {
    observation: Observation,
    start_pending: bool,
    stopping: bool,
}

/// One mailbox per utterance. No native references are transported across
/// threads. Native object addresses are compared as opaque values, never dereferenced.
struct Inbox {
    key: CallbackKey,
    state: Mutex<CallbackState>,
}

impl Inbox {
    fn new(key: CallbackKey) -> Self {
        Self {
            key,
            state: Mutex::new(CallbackState {
                observation: Observation::Unsubmitted,
                start_pending: false,
                stopping: false,
            }),
        }
    }

    fn arm(&self) -> Result<()> {
        let mut state = self.state.lock().map_err(|_| Error::NativeFailure)?;
        state.observation = Observation::Waiting;
        Ok(())
    }

    fn observe(&self, key: CallbackKey, callback: Callback) {
        if key != self.key {
            return;
        }
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        state.observation = match (state.observation, callback, state.stopping) {
            (Observation::Waiting, Callback::Start, false) => {
                state.start_pending = true;
                Observation::Started
            }
            (Observation::Started, Callback::Finish, false) => Observation::Finished,
            (Observation::Waiting, Callback::Finish, false) => Observation::Invalid,
            (Observation::Waiting | Observation::Started, Callback::Cancel, stopping) => {
                Observation::Cancelled {
                    stop_requested: stopping,
                }
            }
            (current, _, _) => current,
        };
    }

    fn begin_stop(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.stopping = true;
            state.start_pending = false;
            if !matches!(state.observation, Observation::Cancelled { .. }) {
                state.observation = Observation::Waiting;
            }
        }
    }

    fn take_evidence(&self) -> Option<Evidence> {
        let Ok(mut state) = self.state.lock() else {
            return Some(Evidence::Failure(Error::NativeFailure));
        };
        if std::mem::take(&mut state.start_pending) {
            return Some(Evidence::Started);
        }
        let evidence = match state.observation {
            Observation::Finished => Evidence::Finished,
            Observation::Cancelled { stop_requested } => Evidence::Cancelled { stop_requested },
            Observation::Invalid => return Some(Evidence::Failure(Error::UnexpectedCallback)),
            _ => return None,
        };
        state.observation = Observation::Closed;
        Some(evidence)
    }

    fn close(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.start_pending = false;
            state.observation = Observation::Closed;
        }
    }
}

trait Backend: Sized {
    type Voice;
    type Utterance;

    fn find_voice(&self, id: &VoiceId) -> Result<Option<(VoiceId, Self::Voice)>>;
    fn prepare(&self, voice: Self::Voice, text: &str) -> Result<(Self::Utterance, NativeObjects)>;
    fn attach(&mut self, inbox: Arc<Inbox>) -> Result<()>;
    fn speak(&self, utterance: &Self::Utterance) -> Result<()>;
    fn stop_immediately(&self) -> Result<bool>;
    fn detach(&mut self) -> Result<()>;
}

fn validate_inventory(voices: Vec<Voice>) -> Result<Vec<Voice>> {
    if voices.len() > MAX_VOICES {
        return Err(Error::InvalidVoiceMetadata);
    }
    let mut ids = HashSet::with_capacity(voices.len());
    for voice in &voices {
        if !ids.insert(&voice.id) {
            return Err(Error::InvalidVoiceMetadata);
        }
    }
    Ok(voices)
}

pub(super) trait SpeechOutput {
    fn request(
        &mut self,
        identity: ContractIdentity,
        sequence: u64,
        text: &str,
        voice: &VoiceId,
        admitted: &MonotonicTime,
    ) -> Result<Notice>;
    fn stop(&mut self, identity: ContractIdentity) -> Result<Notice>;
    fn poll(&mut self) -> Option<Notice>;
    fn shutdown(&mut self) -> Option<Notice>;
}

struct Flight<U> {
    utterance: U,
    inbox: Arc<Inbox>,
    deadline: Deadline,
    stop_attempt: Option<StopAttempt>,
}

impl<U> Flight<U> {
    fn notice(&self, evidence: Evidence) -> Notice {
        Notice {
            identity: self.inbox.key.identity,
            evidence,
        }
    }
}

/// One slot for actual output. The caller owns any pending job and must supply
/// its original Phase 1 admission time so preparation does not reset its deadline.
struct Port<B: Backend, C: MonotonicClock> {
    backend: B,
    clock: C,
    run: AppRunId,
    last_sequence: u64,
    active: Option<Flight<B::Utterance>>,
    closed: bool,
}

impl<B: Backend, C: MonotonicClock> Port<B, C> {
    fn new(backend: Result<B>, clock: C, run: AppRunId) -> Result<Self> {
        Ok(Self {
            backend: backend?,
            clock,
            run,
            last_sequence: 0,
            active: None,
            closed: false,
        })
    }

    fn request_stop(backend: &B, flight: &mut Flight<B::Utterance>) -> Notice {
        let attempt = *flight.stop_attempt.get_or_insert_with(|| {
            flight.inbox.begin_stop();
            match backend.stop_immediately() {
                Ok(true) => StopAttempt::Accepted,
                Ok(false) => StopAttempt::Rejected,
                Err(_) => StopAttempt::Failed,
            }
        });
        flight.notice(Evidence::CancellationUnconfirmed(attempt))
    }

    fn clear_delegate(&mut self, inbox: &Inbox) {
        inbox.close();
        if self.backend.detach().is_err() {
            // A failing detach cannot grant a fresh slot.
            self.closed = true;
        }
    }

    fn retire(&mut self) {
        if let Some(flight) = self.active.take() {
            self.clear_delegate(&flight.inbox);
        }
    }
}

impl<B: Backend, C: MonotonicClock> SpeechOutput for Port<B, C> {
    fn request(
        &mut self,
        identity: ContractIdentity,
        sequence: u64,
        text: &str,
        voice: &VoiceId,
        admitted: &MonotonicTime,
    ) -> Result<Notice> {
        if self.closed {
            return Err(Error::Closed);
        }
        if self.active.is_some() {
            return Err(Error::Busy);
        }
        if identity.app_run_id != self.run
            || sequence <= self.last_sequence
            || JobId::from_native_sequence(sequence)? != identity.job_id
        {
            return Err(Error::StaleJob);
        }
        let bytes = u64::try_from(text.len()).map_err(|_| ContractError::Overflow)?;
        checked_total(0, bytes, MAX_SPEECH_JOB_TEXT_BYTES)?;
        if text.trim().is_empty() || text.contains('\0') {
            return Err(Error::InvalidText);
        }
        let now = self.clock.now();
        now.checked_duration_since(admitted)?;
        let deadline = Deadline::after(admitted.clone(), SPEECH_OUTPUT_DEADLINE)?;
        if deadline.is_expired(&now)? {
            return Err(Error::Deadline);
        }
        self.last_sequence = sequence;
        let (resolved_id, resolved_voice) = self
            .backend
            .find_voice(voice)?
            .ok_or(Error::VoiceUnavailable)?;
        if &resolved_id != voice {
            return Err(Error::VoiceUnavailable);
        }
        let (utterance, objects) = self.backend.prepare(resolved_voice, text)?;
        if objects.synthesizer == 0 || objects.utterance == 0 {
            return Err(Error::NativeFailure);
        }
        let inbox = Arc::new(Inbox::new(CallbackKey { identity, objects }));
        if let Err(error) = self.backend.attach(Arc::clone(&inbox)) {
            self.clear_delegate(&inbox);
            return Err(error);
        }
        if deadline.is_expired(&self.clock.now()).unwrap_or(true) {
            self.clear_delegate(&inbox);
            return Err(Error::Deadline);
        }
        if let Err(error) = inbox.arm() {
            self.clear_delegate(&inbox);
            return Err(error);
        }
        self.active = Some(Flight {
            utterance,
            inbox,
            deadline,
            stop_attempt: None,
        });
        // Publish the slot before invoking AVFoundation, which may call back
        // inline. No mailbox lock is held across a backend method.
        let flight = self.active.as_mut().expect("just installed the flight");
        if let Err(error) = self.backend.speak(&flight.utterance) {
            // Native submission can fail after crossing the boundary. Keep
            // ownership until cancellation is observed or the port shuts down.
            Self::request_stop(&self.backend, flight);
            return Ok(flight.notice(Evidence::Failure(error)));
        }
        Ok(flight.notice(Evidence::Requested))
    }

    fn stop(&mut self, identity: ContractIdentity) -> Result<Notice> {
        let flight = self.active.as_mut().ok_or(Error::StaleJob)?;
        if flight.inbox.key.identity != identity {
            return Err(Error::StaleJob);
        }
        Ok(Self::request_stop(&self.backend, flight))
    }

    /// The owner polls with its run-loop scheduling. Expiry suppresses buffered
    /// success and attempts immediate stop. Missing evidence keeps the slot held.
    fn poll(&mut self) -> Option<Notice> {
        let flight = self.active.as_mut()?;
        if flight.stop_attempt.is_none()
            && flight
                .deadline
                .is_expired(&self.clock.now())
                .unwrap_or(true)
        {
            Self::request_stop(&self.backend, flight);
            return Some(flight.notice(Evidence::Failure(Error::Deadline)));
        }
        let evidence = flight.inbox.take_evidence()?;
        let notice = flight.notice(evidence);
        match evidence {
            Evidence::Finished | Evidence::Cancelled { .. } => self.retire(),
            Evidence::Failure(_) if flight.stop_attempt.is_none() => {
                Self::request_stop(&self.backend, flight);
            }
            Evidence::Failure(_) => return None, // e.g. a poisoned mailbox after stop
            _ => {}
        }
        Some(notice)
    }

    fn shutdown(&mut self) -> Option<Notice> {
        self.closed = true;
        let flight = self.active.as_mut()?;
        let unconfirmed = Self::request_stop(&self.backend, flight);
        let notice = match flight.inbox.take_evidence() {
            Some(evidence @ Evidence::Cancelled { .. }) => flight.notice(evidence),
            _ => unconfirmed,
        };
        self.retire();
        Some(notice)
    }
}

impl<B: Backend, C: MonotonicClock> Drop for Port<B, C> {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}

#[cfg(target_os = "macos")]
#[allow(unused_imports)] // The isolated port has no runtime caller in P2-01.
pub(super) use macos::{installed_voices, open};

#[cfg(target_os = "macos")]
#[allow(dead_code)] // Compiled in tests. No automated test instantiates AVFoundation.
mod macos {
    use super::*;
    use objc2::rc::{autoreleasepool, Retained};
    use objc2::runtime::{AnyClass, AnyObject, AnyProtocol, Bool, NSObject, NSObjectProtocol};
    use objc2::{
        define_class, extern_protocol, msg_send, sel, AnyThread, DefinedClass, MainThreadMarker,
    };
    use objc2_foundation::{NSArray, NSString, NSUTF8StringEncoding};
    use std::panic::{catch_unwind, AssertUnwindSafe};
    use std::ptr::NonNull;

    #[link(name = "AVFoundation", kind = "framework")]
    extern "C" {}

    // SAFETY: The runtime protocol has NSObject as its only superprotocol.
    // Its optional delegate selectors are implemented with their SDK signatures below.
    extern_protocol!(
        unsafe trait AVSpeechSynthesizerDelegate: NSObjectProtocol {}
    );

    define_class!(
        // SAFETY: NSObject permits subclassing. Immutable ivars hold only an
        // Arc to a synchronized Rust mailbox and are initialized before init.
        #[unsafe(super(NSObject))]
        #[name = "GhostM45BIsolatedSpeechDelegate"]
        #[ivars = Arc<Inbox>]
        struct Delegate;

        // SAFETY: Delegate inherits NSObject's protocol implementations.
        unsafe impl NSObjectProtocol for Delegate {}

        // SAFETY: Delegate implements the three optional methods below; each
        // takes two Objective-C object pointers and returns void.
        unsafe impl AVSpeechSynthesizerDelegate for Delegate {}

        impl Delegate {
            #[unsafe(method(speechSynthesizer:didStartSpeechUtterance:))]
            fn started(&self, synthesizer: *mut AnyObject, utterance: *mut AnyObject) {
                self.record(synthesizer, utterance, Callback::Start);
            }

            #[unsafe(method(speechSynthesizer:didFinishSpeechUtterance:))]
            fn finished(&self, synthesizer: *mut AnyObject, utterance: *mut AnyObject) {
                self.record(synthesizer, utterance, Callback::Finish);
            }

            #[unsafe(method(speechSynthesizer:didCancelSpeechUtterance:))]
            fn cancelled(&self, synthesizer: *mut AnyObject, utterance: *mut AnyObject) {
                self.record(synthesizer, utterance, Callback::Cancel);
            }
        }
    );

    impl Delegate {
        fn record(&self, synthesizer: *mut AnyObject, utterance: *mut AnyObject, event: Callback) {
            if let Err(payload) = catch_unwind(AssertUnwindSafe(|| {
                let inbox = self.ivars();
                let key = CallbackKey {
                    identity: inbox.key.identity,
                    objects: NativeObjects {
                        synthesizer: synthesizer as usize,
                        utterance: utterance as usize,
                    },
                };
                inbox.observe(key, event);
            })) {
                // Even a panic payload with a panicking destructor must stay
                // on this side of the Objective-C callback boundary.
                std::mem::forget(payload);
            }
        }
    }

    struct MacBackend {
        _main_thread: MainThreadMarker,
        synthesizer: Retained<AnyObject>,
        utterance_class: &'static AnyClass,
        voice_class: &'static AnyClass,
        // The framework's delegate property is weak.
        delegate: Option<Retained<Delegate>>,
    }

    struct MacUtterance {
        object: Retained<AnyObject>,
        _voice: Retained<AnyObject>,
    }

    impl MacBackend {
        // Owner entry points and destruction stay on the macOS main thread
        // with its normal run loop. There is no wait for callback delivery.
        fn new(main_thread: MainThreadMarker) -> Result<Self> {
            native_call(|| {
                let synth_class =
                    AnyClass::get(c"AVSpeechSynthesizer").ok_or(Error::BackendUnavailable)?;
                let utterance_class =
                    AnyClass::get(c"AVSpeechUtterance").ok_or(Error::BackendUnavailable)?;
                let voice_class =
                    AnyClass::get(c"AVSpeechSynthesisVoice").ok_or(Error::BackendUnavailable)?;
                AnyProtocol::get(c"AVSpeechSynthesizerDelegate")
                    .ok_or(Error::BackendUnavailable)?;
                // SAFETY: The verified AVSpeechSynthesizer class inherits
                // NSObject's +new. The new family transfers +1 ownership.
                let synthesizer = unsafe { msg_send![synth_class, new] };
                Ok(Self {
                    _main_thread: main_thread,
                    synthesizer: checked_object(synthesizer, synth_class)?,
                    utterance_class,
                    voice_class,
                    delegate: None,
                })
            })
        }
    }

    pub(in crate::voice_conversation) fn open(
        main_thread: MainThreadMarker,
        clock: impl MonotonicClock,
        run: AppRunId,
    ) -> Result<impl SpeechOutput> {
        Port::new(MacBackend::new(main_thread), clock, run)
    }

    // Wry already enables objc2's exception API in the locked macOS build.
    // Keep foreign exceptions in that existing handler and discard their data.
    fn native_call<T>(call: impl FnOnce() -> Result<T>) -> Result<T> {
        autoreleasepool(|_| {
            objc2::exception::catch(AssertUnwindSafe(call)).map_err(|_| Error::NativeFailure)?
        })
    }

    fn checked_object(
        object: Option<Retained<AnyObject>>,
        expected: &AnyClass,
    ) -> Result<Retained<AnyObject>> {
        let object = object.ok_or(Error::NativeFailure)?;
        let mut class = Some(object.class());
        while let Some(current) = class {
            if std::ptr::eq(current, expected) {
                return Ok(object);
            }
            class = current.superclass();
        }
        Err(Error::NativeFailure)
    }

    fn native_string(value: &str) -> Result<Retained<NSString>> {
        // SAFETY: initWithBytes:length:encoding: copies exactly the live UTF-8
        // slice, takes NSUInteger/NSStringEncoding and returns an owned NSString
        // or nil. msg_send transfers the NSString allocation to init.
        let string: Option<Retained<NSString>> = unsafe {
            msg_send![NSString::alloc(), initWithBytes: value.as_ptr().cast::<std::ffi::c_void>(),
                length: value.len(), encoding: NSUTF8StringEncoding]
        };
        string.ok_or(Error::NativeFailure)
    }

    fn read_string(object: Option<Retained<AnyObject>>, max: usize) -> Result<String> {
        let string = object
            .and_then(|object| object.downcast::<NSString>().ok())
            .ok_or(Error::InvalidVoiceMetadata)?;
        let bytes = string.len();
        if bytes == 0 || bytes > max {
            return Err(Error::InvalidVoiceMetadata);
        }
        let mut buffer = [0_u8; MAX_VOICE_ID_BYTES + 1];
        let pointer = NonNull::from(&mut buffer[0]).cast();
        // SAFETY: All callers' maxima fit this stack allocation. Foundation
        // writes at most bytes + 1 bytes to the buffer during this call.
        if !unsafe {
            string.getCString_maxLength_encoding(pointer, bytes + 1, NSUTF8StringEncoding)
        } {
            return Err(Error::InvalidVoiceMetadata);
        }
        let value =
            std::str::from_utf8(&buffer[..bytes]).map_err(|_| Error::InvalidVoiceMetadata)?;
        if !bounded_field(value, max) {
            return Err(Error::InvalidVoiceMetadata);
        }
        Ok(value.to_owned())
    }

    fn voice_id(voice: &AnyObject) -> Result<VoiceId> {
        // SAFETY: The caller has verified the AVSpeechSynthesisVoice class.
        // The identifier getter returns an autoreleased/borrowed NSString (+0).
        let id = unsafe { msg_send![voice, identifier] };
        read_string(id, MAX_VOICE_ID_BYTES).map(VoiceId)
    }

    /// This can run off the speech owner thread. It only uses the SDK's sendable
    /// voice objects within the calling thread and returns bounded Rust data.
    pub(in crate::voice_conversation) fn installed_voices() -> Result<Vec<Voice>> {
        native_call(|| {
            let voice_class =
                AnyClass::get(c"AVSpeechSynthesisVoice").ok_or(Error::BackendUnavailable)?;
            // SAFETY: +speechVoices on the verified class returns an NSArray
            // at +0. Check both the array and element types before use.
            let array: Option<Retained<AnyObject>> =
                unsafe { msg_send![voice_class, speechVoices] };
            let array = array
                .and_then(|object| object.downcast::<NSArray>().ok())
                .ok_or(Error::NativeFailure)?;
            let count = array.count();
            if count > MAX_VOICES {
                return Err(Error::InvalidVoiceMetadata);
            }
            let mut voices = Vec::with_capacity(count);
            for index in 0..count {
                let voice = checked_object(Some(array.objectAtIndex(index)), voice_class)?;
                // SAFETY: Both checked voice getters return NSString pointers
                // at +0. Rust retains them until bounded conversion completes.
                let (name, language) =
                    unsafe { (msg_send![&*voice, name], msg_send![&*voice, language]) };
                voices.push(Voice::new(
                    voice_id(&voice)?,
                    read_string(name, MAX_VOICE_NAME_BYTES)?,
                    read_string(language, MAX_VOICE_LANGUAGE_BYTES)?,
                )?);
            }
            validate_inventory(voices)
        })
    }

    impl Backend for MacBackend {
        type Voice = Retained<AnyObject>;
        type Utterance = MacUtterance;

        fn find_voice(&self, id: &VoiceId) -> Result<Option<(VoiceId, Self::Voice)>> {
            native_call(|| {
                let identifier = native_string(&id.0)?;
                // SAFETY: The verified voice class's factory takes a live NSString
                // and returns a nullable voice at +0; nil means unavailable.
                let voice: Option<Retained<AnyObject>> =
                    unsafe { msg_send![self.voice_class, voiceWithIdentifier: &*identifier] };
                voice
                    .map(|voice| {
                        let voice = checked_object(Some(voice), self.voice_class)?;
                        Ok((voice_id(&voice)?, voice))
                    })
                    .transpose()
            })
        }

        fn prepare(
            &self,
            voice: Self::Voice,
            text: &str,
        ) -> Result<(Self::Utterance, NativeObjects)> {
            native_call(|| {
                let text = native_string(text)?;
                // SAFETY: The verified utterance class's plain-text factory
                // takes a live NSString and returns an utterance at +0.
                let utterance =
                    unsafe { msg_send![self.utterance_class, speechUtteranceWithString: &*text] };
                let object = checked_object(utterance, self.utterance_class)?;
                if object
                    .class()
                    .instance_method(sel!(setPrefersAssistiveTechnologySettings:))
                    .is_some()
                {
                    // SAFETY: The SDK's macOS 11+ setter was found on the verified
                    // utterance. A false BOOL preserves explicit voice selection.
                    unsafe {
                        let _: () =
                            msg_send![&*object, setPrefersAssistiveTechnologySettings: Bool::NO];
                    }
                }

                // SAFETY: Both objects have verified classes. setVoice: takes a
                // voice object, retains it and returns void. voice returns +0.
                let selected: Option<Retained<AnyObject>> = unsafe {
                    let _: () = msg_send![&*object, setVoice: &*voice];
                    msg_send![&*object, voice]
                };
                let selected = checked_object(selected, self.voice_class)?;
                if voice_id(&selected)? != voice_id(&voice)? {
                    return Err(Error::VoiceUnavailable);
                }
                let objects = NativeObjects {
                    synthesizer: Retained::as_ptr(&self.synthesizer) as usize,
                    utterance: Retained::as_ptr(&object) as usize,
                };
                Ok((
                    MacUtterance {
                        object,
                        _voice: voice,
                    },
                    objects,
                ))
            })
        }

        fn attach(&mut self, inbox: Arc<Inbox>) -> Result<()> {
            if self.delegate.is_some() {
                return Err(Error::NativeFailure);
            }
            native_call(|| {
                let allocated = Delegate::alloc().set_ivars(inbox);
                // SAFETY: Ivars are initialized; NSObject's inherited init
                // consumes the allocation and returns the retained delegate.
                let delegate: Option<Retained<Delegate>> =
                    unsafe { msg_send![super(allocated), init] };
                self.delegate = Some(delegate.ok_or(Error::NativeFailure)?);
                let delegate = self.delegate.as_deref().ok_or(Error::NativeFailure)?;
                // SAFETY: The retained delegate conforms to the SDK protocol.
                // setDelegate: accepts the Objective-C object and returns void.
                unsafe {
                    let _: () = msg_send![&*self.synthesizer, setDelegate: delegate];
                }
                Ok(())
            })
        }

        fn speak(&self, utterance: &Self::Utterance) -> Result<()> {
            native_call(|| {
                // SAFETY: A fresh, validated utterance is submitted once to the
                // owned synthesizer, on its main thread. Both live through the
                // void call. No other utterance is queued in the bridge.
                unsafe {
                    let _: () = msg_send![&*self.synthesizer, speakUtterance: &*utterance.object];
                }
                Ok(())
            })
        }

        fn stop_immediately(&self) -> Result<bool> {
            native_call(|| {
                // SAFETY: The SDK defines AVSpeechBoundary as NSInteger and
                // Immediate as zero. The return is Objective-C BOOL, not proof
                // that the delegate observed cancellation.
                let accepted: Bool =
                    unsafe { msg_send![&*self.synthesizer, stopSpeakingAtBoundary: 0_isize] };
                Ok(accepted.as_bool())
            })
        }

        fn detach(&mut self) -> Result<()> {
            let result = native_call(|| {
                // SAFETY: The SDK declares a weak nullable delegate. Passing
                // nil clears the association and setDelegate: returns void.
                unsafe {
                    let _: () =
                        msg_send![&*self.synthesizer, setDelegate: std::ptr::null::<AnyObject>()];
                }
                Ok(())
            });
            self.delegate = None;
            result
        }
    }
}

#[path = "native_speech_tests.rs"]
mod tests;
