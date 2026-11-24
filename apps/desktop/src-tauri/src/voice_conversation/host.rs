//! P1-11 synthetic host policy only. No macOS observers, menu, IPC or services.
//!
//! Lifecycle receipts, authorization facts and audio evidence are test fixtures,
//! not OS delivery or real grants. Effects describe requests only: private P1-02
//! through P1-10 controllers are neither called nor changed. Runtime forwarding
//! would require a separately authorized bridge into those native boundaries.
//!
//! The one pending observation and one pending enablement bound model storage;
//! this is not a proposed OS event queue. Independent native Stop/Quit bypass
//! both slots, clocks, sequences, renderer, provider and network availability.
use super::clock::{Deadline, MonotonicTime};
use super::limits::{MAX_CAPTURED_TURN, MAX_CLOUD_INTERACTION, MAX_WAKE_ARMING};
use super::model::{AppRunId, Generation, MicrophoneOwner, MicrophoneReleaseStatus};
use super::ContractError;
use std::num::NonZeroU64;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Error {
    Contract(ContractError),
    Stale,
    Busy,
    Closed,
    Unauthorized,
    InvalidTransition,
    UnreachableControls,
    CleanupUnconfirmed,
    UrgentCollision,
}
impl From<ContractError> for Error {
    fn from(value: ContractError) -> Self {
        Self::Contract(value)
    }
}
type Result<T> = std::result::Result<T, Error>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct OsSessionId(NonZeroU64);
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct HostId(NonZeroU64);
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Binding {
    run: AppRunId,
    os_session: OsSessionId,
    host: HostId,
    // Observer installation generation, separate from cancelable voice epochs.
    generation: Generation,
}

#[derive(Clone, Debug)]
struct Stamp {
    binding: Binding,
    native: Arc<()>,
    epoch: Generation,
    sequence: u64,
    nonce: Arc<()>,
}
impl Stamp {
    fn matches(&self, other: &Self) -> bool {
        self.binding == other.binding
            && self.epoch == other.epoch
            && self.sequence == other.sequence
            && Arc::ptr_eq(&self.native, &other.native)
            && Arc::ptr_eq(&self.nonce, &other.nonce)
    }
}

/// Long-lived native control identity, not voice authority. It survives voice
/// epoch exhaustion so revocation and Quit cannot be disabled by arithmetic.
#[derive(Clone)]
struct NativeControl {
    binding: Binding,
    native: Arc<()>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Window {
    Visible,
    Hidden,
    Minimized,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Menu {
    Unavailable,
    Preparing,
    Ready,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Application {
    Running,
    ShuttingDown,
    TerminationObserved,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Event {
    MenuPreparing,
    MenuReady { keyboard: bool, accessible: bool },
    MenuLost,
    CloseWindow,
    HideWindow,
    Minimize,
    ReopenWindow,
    Focus,
    Blur,
    NavigateAway,
    RendererLost,
    Lock,
    Unlock,
    Sleep,
    Wake,
    Logout,
    PermissionLost,
    PermissionRestored,
    DeviceLost,
    Pause,
    EndConversation,
    HostTerminated,
}
impl Event {
    fn urgent(self) -> bool {
        matches!(
            self,
            Self::MenuLost
                | Self::RendererLost
                | Self::Lock
                | Self::Sleep
                | Self::Logout
                | Self::PermissionLost
                | Self::DeviceLost
                | Self::HostTerminated
        )
    }
    fn revoking(self) -> bool {
        matches!(
            self,
            Self::MenuLost
                | Self::RendererLost
                | Self::Lock
                | Self::Sleep
                | Self::Logout
                | Self::PermissionLost
                | Self::DeviceLost
                | Self::Pause
                | Self::EndConversation
                | Self::HostTerminated
        )
    }
}

#[derive(Clone, Debug)]
struct Observation {
    stamp: Stamp,
    event: Event,
}
impl Observation {
    fn matches(&self, other: &Self) -> bool {
        self.stamp.matches(&other.stamp) && self.event == other.event
    }
}

/// Independently validated SYNTHETIC consent facts. No network/tool/planning
/// permission exists in this host type. Background scope never covers M45.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Scope {
    microphone: bool,
    manual: bool,
    wake: bool,
    conversation: bool,
    background_wake: bool,
    background_conversation: bool,
}
impl Scope {
    fn valid(self) -> bool {
        self.microphone
            && (self.manual || self.wake || self.conversation)
            && (!self.background_wake || self.wake)
            && (!self.background_conversation || self.conversation)
    }
    fn maximum(self) -> Duration {
        if self.conversation {
            MAX_CLOUD_INTERACTION
        } else if self.wake {
            MAX_WAKE_ARMING
        } else {
            MAX_CAPTURED_TURN
        }
    }
}
struct EnableData {
    stamp: Stamp,
    scope: Scope,
    deadline: Deadline,
    confirmation_deadline: Deadline,
}
#[derive(Clone)]
struct EnablePreview(Arc<EnableData>);
struct Authorization {
    preview: EnablePreview,
    alive: Arc<AtomicBool>,
}

#[derive(Clone, Debug)]
struct CaptureTicket {
    stamp: Stamp,
    owner: MicrophoneOwner,
}
impl CaptureTicket {
    fn matches(&self, other: &Self) -> bool {
        self.stamp.matches(&other.stamp) && self.owner == other.owner
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CapturePhase {
    Acquiring,
    Capturing,
    StoppingUnconfirmed,
    Cleaning,
}
struct Capture {
    ticket: CaptureTicket,
    phase: CapturePhase,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Resource {
    Output,
    Workers,
    Content,
}
#[derive(Clone, Debug)]
struct CleanupTicket(Stamp);
#[derive(Clone, Debug)]
struct FrameTicket {
    cleanup: CleanupTicket,
    capture: CaptureTicket,
}
struct Cleanup {
    ticket: CleanupTicket,
    required: [bool; 3],
    acknowledged: [bool; 3],
}
impl Cleanup {
    fn complete(&self) -> bool {
        (0..3).all(|i| !self.required[i] || self.acknowledged[i])
    }
}

/// Inert, bounded request flags; never proof that a participant received them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Effects {
    cancel_manual: bool,
    end_child: bool,
    revoke_wake: bool,
    revoke_voice: bool,
    revoke_egress: bool,
    stop_output: bool,
    terminate_workers: bool,
    clear_content: bool,
    terminate_host: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum VoiceLabel {
    Off,
    ListeningLocally,
    ManualCapture,
    ConversationCapture,
    Paused,
    StartingUnconfirmed,
    StoppingUnconfirmed,
    Cleaning,
}
impl VoiceLabel {
    fn text(self) -> &'static str {
        match self {
            Self::Off => "Off",
            Self::ListeningLocally => "Listening locally",
            Self::ManualCapture => "Manual recording",
            Self::ConversationCapture => "Conversation capture",
            Self::Paused => "Paused",
            Self::StartingUnconfirmed => "Starting/unconfirmed",
            Self::StoppingUnconfirmed => "Stopping/unconfirmed",
            Self::Cleaning => "Input released; cleanup pending",
        }
    }
}
struct MenuStatus {
    stop_label: &'static str,
    stop_reachable: bool,
    keyboard_reachable: bool,
    voice_label: VoiceLabel,
}

struct HostModel {
    binding: Binding,
    native: Arc<()>,
    epoch: Generation,
    sequence: u64,
    pending: Option<Observation>,
    pending_enable: Option<EnablePreview>,
    authorization: Option<Authorization>,
    application: Application,
    window: Window,
    foreground: bool,
    menu: Menu,
    renderer_available: bool,
    locked: bool,
    sleeping: bool,
    logged_out: bool,
    permission: bool,
    device_available: bool,
    paused: bool,
    blocked_modes: [bool; 3],
    stop_latched: bool,
    faulted: bool,
    error_visible: bool,
    last_time: MonotonicTime,
    capture: Option<Capture>,
    cleanup_seed: CleanupTicket,
    cleanup: Option<Cleanup>,
    cleanup_failed: bool,
    effects: Effects,
}

impl HostModel {
    fn synthetic(binding: Binding, now: MonotonicTime) -> Self {
        let native = Arc::new(());
        let cleanup_seed = CleanupTicket(Stamp {
            binding,
            native: native.clone(),
            epoch: binding.generation,
            sequence: 0,
            nonce: Arc::new(()),
        });
        Self {
            binding,
            native,
            epoch: binding.generation,
            sequence: 0,
            pending: None,
            pending_enable: None,
            authorization: None,
            application: Application::Running,
            window: Window::Visible,
            foreground: true,
            menu: Menu::Unavailable,
            renderer_available: true,
            locked: false,
            sleeping: false,
            logged_out: false,
            permission: false,
            device_available: true,
            paused: false,
            blocked_modes: [false; 3],
            stop_latched: false,
            faulted: false,
            error_visible: false,
            last_time: now,
            capture: None,
            cleanup_seed,
            cleanup: None,
            cleanup_failed: false,
            effects: Effects::default(),
        }
    }

    fn native_control(&self) -> NativeControl {
        NativeControl {
            binding: self.binding,
            native: self.native.clone(),
        }
    }
    fn check_control(&self, control: &NativeControl) -> Result<()> {
        if control.binding != self.binding || !Arc::ptr_eq(&control.native, &self.native) {
            return Err(Error::Stale);
        }
        Ok(())
    }
    fn stamp(&mut self) -> Result<Stamp> {
        let Some(sequence) = self.sequence.checked_add(1) else {
            self.faulted = true;
            self.revoke();
            return Err(ContractError::Overflow.into());
        };
        self.sequence = sequence;
        Ok(Stamp {
            binding: self.binding,
            native: self.native.clone(),
            epoch: self.epoch,
            sequence,
            nonce: Arc::new(()),
        })
    }
    fn invalidate_epoch(&mut self) -> bool {
        self.pending = None;
        self.pending_enable = None;
        match self.epoch.checked_next() {
            Ok(epoch) => {
                self.epoch = epoch;
                true
            }
            Err(_) => {
                self.faulted = true;
                false
            }
        }
    }
    fn observe(&mut self, now: &MonotonicTime) -> Result<()> {
        if let Err(error) = now.checked_duration_since(&self.last_time) {
            self.faulted = true;
            self.revoke();
            return Err(error.into());
        }
        self.last_time = now.clone();
        if let Some(auth) = &self.authorization {
            match auth.preview.0.deadline.is_expired(now) {
                Ok(false) => (),
                Ok(true) => self.revoke(),
                Err(error) => {
                    self.faulted = true;
                    self.revoke();
                    return Err(error.into());
                }
            }
        }
        Ok(())
    }
    fn privacy_clear(&self) -> bool {
        !self.locked
            && !self.sleeping
            && !self.logged_out
            && self.permission
            && self.device_available
            && !self.faulted
            && self.application == Application::Running
    }
    fn cleanup_complete(&self) -> bool {
        self.capture.is_none() && self.cleanup.as_ref().is_none_or(Cleanup::complete)
    }

    fn start_cleanup(&mut self, required: [bool; 3], stop_input: bool) {
        let cleanup = self.cleanup.get_or_insert_with(|| Cleanup {
            ticket: self.cleanup_seed.clone(),
            required: [false; 3],
            acknowledged: [false; 3],
        });
        for (i, required) in required.into_iter().enumerate() {
            cleanup.required[i] |= required;
        }
        if stop_input {
            if let Some(capture) = &mut self.capture {
                if matches!(
                    capture.phase,
                    CapturePhase::Acquiring | CapturePhase::Capturing
                ) {
                    capture.phase = CapturePhase::StoppingUnconfirmed;
                }
            }
        }
    }
    /// Revocation precedes cleanup requests. This never calls private Stop or
    /// waits on a provider, renderer, clock, queue, audit or external mutex.
    fn revoke(&mut self) {
        if let Some(auth) = self.authorization.take() {
            auth.alive.store(false, Ordering::Release);
        }
        if !self.stop_latched {
            let _ = self.invalidate_epoch();
        }
        self.stop_latched = true;
        self.pending = None;
        self.pending_enable = None;
        self.paused = false;
        self.blocked_modes = [true; 3];
        self.effects.revoke_voice = true;
        self.effects.revoke_wake = true;
        self.effects.revoke_egress = true;
        self.effects.stop_output = true;
        self.effects.terminate_workers = true;
        self.effects.clear_content = true;
        self.start_cleanup([true; 3], true);
    }
    fn stop_listening(&mut self, control: &NativeControl) -> Result<()> {
        self.check_control(control)?;
        self.revoke();
        Ok(())
    }
    fn menu_stop(&mut self, control: &NativeControl) -> Result<()> {
        self.check_control(control)?;
        if self.menu != Menu::Ready {
            return Err(Error::UnreachableControls);
        }
        self.stop_listening(control)
    }
    fn quit(&mut self, control: &NativeControl) -> Result<()> {
        self.check_control(control)?;
        self.revoke();
        self.effects.terminate_host = true;
        if self.application != Application::TerminationObserved {
            self.application = Application::ShuttingDown;
        }
        Ok(())
    }

    /// A single issued observation is immutable by contract and matched in full.
    /// There is intentionally no JSON/renderer/provider observation constructor.
    fn synthetic_observation(&mut self, event: Event) -> Result<Observation> {
        // Only this trusted synthetic issuer may supersede an ordinary receipt.
        // A caller cannot make an existing receipt urgent by changing its kind:
        // apply still matches the complete newly issued receipt.
        if let Some(pending) = &self.pending {
            if !event.urgent() {
                return Err(Error::Busy);
            }
            if pending.event.urgent() {
                // One slot cannot preserve both notifications. Fault/revoke
                // immediately, without allocating a receipt or reading a clock;
                // retaining either event must never retain listening authority.
                let termination_unconfirmed =
                    pending.event == Event::HostTerminated || event == Event::HostTerminated;
                self.faulted = true;
                self.error_visible = true;
                self.revoke(); // invalidates all pending receipts, keeps ACK identities
                if termination_unconfirmed {
                    self.effects.terminate_host = true;
                    // This is a shutdown request, not accepted termination proof.
                    if self.application != Application::TerminationObserved {
                        self.application = Application::ShuttingDown;
                    }
                }
                return Err(Error::UrgentCollision);
            }
        }
        if self.application == Application::TerminationObserved
            || (self.application == Application::ShuttingDown && event != Event::HostTerminated)
        {
            return Err(Error::Closed);
        }
        self.pending = None; // old ordinary callbacks are inert before issuance
        let observation = Observation {
            stamp: self.stamp()?,
            event,
        };
        self.pending = Some(observation.clone());
        Ok(observation)
    }

    fn apply(&mut self, observation: &Observation, now: &MonotonicTime) -> Result<()> {
        if !self
            .pending
            .as_ref()
            .is_some_and(|p| p.matches(observation))
            || observation.stamp.binding != self.binding
            || observation.stamp.epoch != self.epoch
        {
            return Err(Error::Stale);
        }
        self.pending = None; // consume even an invalid native transition
                             // Authenticated revoking notifications must work even with a bad clock.
        if !observation.event.revoking() {
            self.observe(now)?;
        }
        match observation.event {
            Event::MenuPreparing if self.menu == Menu::Unavailable => self.menu = Menu::Preparing,
            Event::MenuReady {
                keyboard,
                accessible,
            } if self.menu == Menu::Preparing => {
                if keyboard && accessible {
                    self.menu = Menu::Ready;
                } else {
                    self.menu_lost();
                    return Err(Error::UnreachableControls);
                }
            }
            Event::MenuLost => self.menu_lost(),
            Event::CloseWindow | Event::HideWindow | Event::Minimize => {
                if self.menu != Menu::Ready {
                    self.menu_lost();
                    return Err(Error::UnreachableControls);
                }
                self.leave_foreground();
                self.window = if observation.event == Event::Minimize {
                    Window::Minimized
                } else {
                    Window::Hidden
                };
            }
            Event::ReopenWindow => self.window = Window::Visible,
            Event::Focus if self.window == Window::Visible => self.foreground = true,
            Event::Blur => self.leave_foreground(),
            Event::NavigateAway => self.cancel_mode(MicrophoneOwner::ManualForeground),
            Event::RendererLost => {
                self.renderer_available = false;
                if self
                    .pending_enable
                    .as_ref()
                    .is_some_and(|preview| preview.0.scope.manual)
                {
                    self.pending_enable = None;
                }
                self.cancel_mode(MicrophoneOwner::ManualForeground);
                if self.menu != Menu::Ready {
                    self.menu_lost();
                }
            }
            Event::Lock if !self.locked => {
                self.locked = true;
                self.revoke();
            }
            Event::Unlock if self.locked => self.locked = false,
            Event::Sleep if !self.sleeping => {
                self.sleeping = true;
                self.revoke();
            }
            Event::Wake if self.sleeping => self.sleeping = false,
            Event::Logout if !self.logged_out => {
                self.logged_out = true;
                self.revoke();
            }
            Event::PermissionLost => {
                self.permission = false;
                self.revoke();
            }
            Event::PermissionRestored => self.permission = true,
            Event::DeviceLost => {
                self.device_available = false;
                self.revoke();
            }
            Event::Pause if self.authorization.is_some() && !self.paused => {
                self.paused = true;
                if !self.invalidate_epoch() {
                    self.revoke();
                }
                self.effects.revoke_egress = true;
                self.effects.stop_output = true;
                self.effects.terminate_workers = true;
                self.effects.clear_content = true;
                self.start_cleanup([true; 3], true);
            }
            Event::EndConversation => self.cancel_mode(MicrophoneOwner::ConversationTurn),
            Event::HostTerminated => {
                self.revoke();
                self.effects.terminate_host = true;
                self.application = Application::TerminationObserved;
            }
            _ => return Err(Error::InvalidTransition),
        }
        Ok(())
    }
    fn menu_lost(&mut self) {
        self.menu = Menu::Unavailable;
        self.revoke();
        self.window = Window::Visible;
        self.error_visible = true;
    }
    fn cancel_mode(&mut self, owner: MicrophoneOwner) {
        let index = match owner {
            MicrophoneOwner::ManualForeground => {
                self.effects.cancel_manual = true;
                0
            }
            MicrophoneOwner::WakeDetection => {
                self.effects.revoke_wake = true;
                1
            }
            MicrophoneOwner::ConversationTurn => {
                self.effects.end_child = true;
                self.effects.revoke_egress = true;
                self.effects.stop_output = true;
                self.effects.clear_content = true;
                2
            }
            MicrophoneOwner::Off => return,
        };
        self.blocked_modes[index] = true;
        let held = self
            .capture
            .as_ref()
            .is_some_and(|c| c.ticket.owner == owner);
        if held && !self.invalidate_epoch() {
            self.revoke();
        }
        if held || owner == MicrophoneOwner::ConversationTurn {
            self.start_cleanup(
                [
                    owner == MicrophoneOwner::ConversationTurn,
                    false,
                    owner == MicrophoneOwner::ConversationTurn,
                ],
                held,
            );
        }
    }
    fn leave_foreground(&mut self) {
        self.foreground = false;
        self.pending_enable = None;
        if self.authorization.is_some() && self.menu != Menu::Ready {
            self.menu_lost();
            return;
        }
        self.cancel_mode(MicrophoneOwner::ManualForeground);
        if let Some(auth) = &self.authorization {
            let scope = auth.preview.0.scope;
            if scope.wake && !scope.background_wake {
                self.cancel_mode(MicrophoneOwner::WakeDetection);
            }
            if scope.conversation && !scope.background_conversation {
                self.cancel_mode(MicrophoneOwner::ConversationTurn);
            }
            if !scope.background_wake && !scope.background_conversation {
                self.revoke();
            }
        }
    }

    fn prepare_enable(
        &mut self,
        scope: Scope,
        lifetime: Duration,
        now: &MonotonicTime,
    ) -> Result<EnablePreview> {
        self.observe(now)?;
        if !self.privacy_clear() || !self.cleanup_complete() {
            return Err(Error::Closed);
        }
        if self.authorization.is_some() || self.pending_enable.is_some() {
            return Err(Error::Busy);
        }
        if !scope.valid()
            || (scope.manual && !self.renderer_available)
            || lifetime.is_zero()
            || lifetime > scope.maximum()
            || ((!self.foreground || self.window != Window::Visible) && self.menu != Menu::Ready)
        {
            return Err(Error::Unauthorized);
        }
        let deadline = Deadline::after(now.clone(), lifetime)?;
        let confirmation_deadline = Deadline::after(now.clone(), Duration::from_secs(5 * 60))?;
        let preview = EnablePreview(Arc::new(EnableData {
            stamp: self.stamp()?,
            scope,
            deadline,
            confirmation_deadline,
        }));
        self.pending_enable = Some(preview.clone());
        Ok(preview)
    }
    /// Explicit test-only native affirmative action, not the P1-06 issuer.
    /// An old preview cannot survive Stop/privacy/foreground cleanup.
    fn enable(
        &mut self,
        control: &NativeControl,
        preview: &EnablePreview,
        now: &MonotonicTime,
    ) -> Result<()> {
        self.check_control(control)?;
        let pending = self.pending_enable.take().ok_or(Error::Stale)?;
        if !Arc::ptr_eq(&pending.0, &preview.0) || preview.0.stamp.epoch != self.epoch {
            return Err(Error::Stale);
        }
        self.observe(now)?;
        if !self.privacy_clear()
            || !self.cleanup_complete()
            || (preview.0.scope.manual && !self.renderer_available)
            || preview.0.deadline.is_expired(now)?
            || preview.0.confirmation_deadline.is_expired(now)?
        {
            return Err(Error::Unauthorized);
        }
        self.cleanup = None;
        self.cleanup_seed = CleanupTicket(self.stamp()?);
        self.authorization = Some(Authorization {
            preview: preview.clone(),
            alive: Arc::new(AtomicBool::new(true)),
        });
        self.blocked_modes = [false; 3];
        self.paused = false;
        self.stop_latched = false;
        self.effects = Effects::default();
        Ok(())
    }
    fn eligible(&mut self, owner: MicrophoneOwner, now: &MonotonicTime) -> Result<bool> {
        self.observe(now)?;
        let cleaning = self.cleanup.as_ref().is_some_and(|c| !c.complete())
            || self.capture.as_ref().is_some_and(|c| {
                matches!(
                    c.phase,
                    CapturePhase::StoppingUnconfirmed | CapturePhase::Cleaning
                )
            });
        if !self.privacy_clear() || self.paused || cleaning {
            return Ok(false);
        }
        let Some(auth) = &self.authorization else {
            return Ok(false);
        };
        if !auth.alive.load(Ordering::Acquire) {
            return Ok(false);
        }
        let scope = auth.preview.0.scope;
        let foreground = self.foreground && self.window == Window::Visible;
        let background = self.menu == Menu::Ready;
        Ok(match owner {
            MicrophoneOwner::Off => false,
            MicrophoneOwner::ManualForeground => {
                scope.manual && self.renderer_available && foreground && !self.blocked_modes[0]
            }
            MicrophoneOwner::WakeDetection => {
                scope.wake
                    && !self.blocked_modes[1]
                    && (foreground || (background && scope.background_wake))
            }
            MicrophoneOwner::ConversationTurn => {
                scope.conversation
                    && !self.blocked_modes[2]
                    && (foreground || (background && scope.background_conversation))
            }
        })
    }
    fn resume(&mut self, control: &NativeControl, now: &MonotonicTime) -> Result<()> {
        self.check_control(control)?;
        self.observe(now)?;
        if !self.paused
            || self.authorization.is_none()
            || !self.privacy_clear()
            || !self.cleanup_complete()
        {
            return Err(Error::Unauthorized);
        }
        self.cleanup = None;
        self.cleanup_seed = CleanupTicket(self.stamp()?);
        self.paused = false;
        // The original authorization, deadline and blocked modes are retained.
        Ok(())
    }

    /// Synthetic audio ownership evidence only; starts no capture or hardware.
    fn synthetic_capture(
        &mut self,
        owner: MicrophoneOwner,
        acquiring: bool,
        now: &MonotonicTime,
    ) -> Result<CaptureTicket> {
        if self.capture.is_some() {
            return Err(Error::Busy);
        }
        if !self.eligible(owner, now)? {
            return Err(Error::Unauthorized);
        }
        let ticket = CaptureTicket {
            stamp: self.stamp()?,
            owner,
        };
        self.cleanup = None;
        self.cleanup_seed = CleanupTicket(self.stamp()?);
        self.capture = Some(Capture {
            ticket: ticket.clone(),
            phase: if acquiring {
                CapturePhase::Acquiring
            } else {
                CapturePhase::Capturing
            },
        });
        Ok(ticket)
    }
    fn capture_ready(&mut self, ticket: &CaptureTicket, now: &MonotonicTime) -> Result<()> {
        let held = self.capture.as_ref().ok_or(Error::Stale)?;
        if !held.ticket.matches(ticket)
            || ticket.stamp.epoch != self.epoch
            || held.phase != CapturePhase::Acquiring
        {
            return Err(Error::Stale);
        }
        // Existing occupancy is not a cleanup barrier during acquisition.
        self.observe(now)?;
        if !self.privacy_clear() || self.paused || self.authorization.is_none() {
            return Err(Error::Unauthorized);
        }
        self.capture.as_mut().ok_or(Error::Stale)?.phase = CapturePhase::Capturing;
        Ok(())
    }

    fn release_ack(
        &mut self,
        ticket: &CaptureTicket,
        acquisition_settled: bool,
        input_stopped: bool,
    ) -> Result<()> {
        let held = self.capture.as_mut().ok_or(Error::Stale)?;
        if !held.ticket.matches(ticket) || held.phase != CapturePhase::StoppingUnconfirmed {
            return Err(Error::Stale);
        }
        if !acquisition_settled || !input_stopped {
            self.cleanup_failed = true;
            return Err(Error::CleanupUnconfirmed);
        }
        held.phase = CapturePhase::Cleaning;
        Ok(())
    }
    fn frame_ticket(&self) -> Option<FrameTicket> {
        let capture = self.capture.as_ref()?;
        (capture.phase == CapturePhase::Cleaning).then(|| FrameTicket {
            cleanup: self.cleanup.as_ref().unwrap().ticket.clone(),
            capture: capture.ticket.clone(),
        })
    }
    fn frames_ack(
        &mut self,
        ticket: &FrameTicket,
        frames_cleared: bool,
        queues_cleared: bool,
    ) -> Result<()> {
        let expected = self.frame_ticket().ok_or(Error::Stale)?;
        if !expected.cleanup.0.matches(&ticket.cleanup.0)
            || !expected.capture.matches(&ticket.capture)
        {
            return Err(Error::Stale);
        }
        if !frames_cleared || !queues_cleared {
            self.cleanup_failed = true;
            return Err(Error::CleanupUnconfirmed);
        }
        self.capture = None;
        Ok(())
    }
    fn cleanup_ticket(&self) -> Option<CleanupTicket> {
        self.cleanup.as_ref().map(|c| c.ticket.clone())
    }
    fn resource_ack(
        &mut self,
        ticket: &CleanupTicket,
        resource: Resource,
        success: bool,
    ) -> Result<()> {
        let cleanup = self.cleanup.as_mut().ok_or(Error::Stale)?;
        let index = resource as usize;
        if !cleanup.ticket.0.matches(&ticket.0)
            || !cleanup.required[index]
            || cleanup.acknowledged[index]
        {
            return Err(Error::Stale);
        }
        if !success {
            self.cleanup_failed = true;
            return Err(Error::CleanupUnconfirmed);
        }
        cleanup.acknowledged[index] = true;
        Ok(())
    }
    fn shutdown_complete(&self) -> bool {
        self.application == Application::TerminationObserved && self.cleanup_complete()
    }
    fn microphone_status(&self) -> (MicrophoneOwner, MicrophoneReleaseStatus) {
        match &self.capture {
            None => (MicrophoneOwner::Off, MicrophoneReleaseStatus::NotRequested),
            Some(c) => (
                c.ticket.owner,
                match c.phase {
                    CapturePhase::Acquiring | CapturePhase::Capturing => {
                        MicrophoneReleaseStatus::NotRequested
                    }
                    CapturePhase::StoppingUnconfirmed => {
                        MicrophoneReleaseStatus::StoppingUnconfirmed
                    }
                    CapturePhase::Cleaning => MicrophoneReleaseStatus::Acknowledged,
                },
            ),
        }
    }
    fn menu_status(&self) -> MenuStatus {
        let voice_label = match &self.capture {
            Some(c) => match c.phase {
                CapturePhase::StoppingUnconfirmed => VoiceLabel::StoppingUnconfirmed,
                CapturePhase::Acquiring => VoiceLabel::StartingUnconfirmed,
                CapturePhase::Cleaning => VoiceLabel::Cleaning,
                CapturePhase::Capturing => match c.ticket.owner {
                    MicrophoneOwner::WakeDetection => VoiceLabel::ListeningLocally,
                    MicrophoneOwner::ManualForeground => VoiceLabel::ManualCapture,
                    MicrophoneOwner::ConversationTurn => VoiceLabel::ConversationCapture,
                    MicrophoneOwner::Off => VoiceLabel::Off,
                },
            },
            None if self.paused => VoiceLabel::Paused,
            None => VoiceLabel::Off,
        };
        MenuStatus {
            stop_label: "Stop Listening",
            stop_reachable: self.menu == Menu::Ready && self.application == Application::Running,
            keyboard_reachable: self.menu == Menu::Ready
                && self.application == Application::Running,
            voice_label,
        }
    }
}

/// Synthetic lifecycle input. Only the scoped coordinator adapter issues
/// receipts for these events. Availability events never issue speech grants.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SpeechLifecycleEvent {
    Stop,
    ConsentLost,
    ScopeReplaced,
    RouteChanged,
    RouteUnknown,
    ObserverLost,
    Close,
    Reopen,
    Quit,
    Terminated,
    Sleep,
    Wake,
    Lock,
    Unlock,
    Logout,
    Interrupted,
    PermissionLost,
    PermissionRestored,
}

impl SpeechLifecycleEvent {
    pub(super) fn revokes(self) -> bool {
        // This slice has no independent background speech issuer.
        !matches!(
            self,
            Self::Reopen | Self::Wake | Self::Unlock | Self::PermissionRestored
        )
    }

    pub(super) fn loses_route(self) -> bool {
        matches!(
            self,
            Self::RouteChanged | Self::RouteUnknown | Self::ObserverLost
        )
    }

    pub(super) fn quits(self) -> bool {
        matches!(self, Self::Quit | Self::Terminated)
    }
}

/// One modeled host instance. NativeControl and HostModel remain private.
/// The adapter can request transitions; it cannot acknowledge resources.
pub(super) struct SpeechLifecycleHost {
    model: std::cell::RefCell<HostModel>,
    control: NativeControl,
}

impl SpeechLifecycleHost {
    pub(super) fn synthetic(
        run: AppRunId,
        host: NonZeroU64,
        os_session: NonZeroU64,
        observer: Generation,
        now: MonotonicTime,
    ) -> Self {
        let model = HostModel::synthetic(
            Binding {
                run,
                host: HostId(host),
                os_session: OsSessionId(os_session),
                generation: observer,
            },
            now,
        );
        let control = model.native_control();
        Self {
            model: std::cell::RefCell::new(model),
            control,
        }
    }

    pub(super) fn run(&self) -> AppRunId {
        self.control.binding.run
    }

    pub(super) fn admission_available(&self) -> bool {
        self.model.try_borrow().is_ok_and(|host| {
            host.privacy_clear()
                && host.cleanup_complete()
                && host.window == Window::Visible
                && host.foreground
        })
    }

    /// False means the transition is still pending. Other host errors are
    /// consumed as a fail-closed modeled host fault, preserving cleanup tickets.
    pub(super) fn apply_speech_event(
        &self,
        event: SpeechLifecycleEvent,
        now: &MonotonicTime,
    ) -> bool {
        let Ok(mut host) = self.model.try_borrow_mut() else {
            return false;
        };
        use SpeechLifecycleEvent as E;
        let observation = match event {
            E::Stop | E::Interrupted => {
                let _ = host.stop_listening(&self.control);
                return true;
            }
            E::Quit => {
                let _ = host.quit(&self.control);
                return true;
            }
            E::ConsentLost | E::ScopeReplaced => Event::EndConversation,
            E::RouteChanged | E::RouteUnknown => Event::DeviceLost,
            E::ObserverLost => Event::MenuLost,
            E::Close => Event::CloseWindow,
            E::Reopen => Event::ReopenWindow,
            E::Terminated => Event::HostTerminated,
            E::Sleep => Event::Sleep,
            E::Wake => Event::Wake,
            E::Lock => Event::Lock,
            E::Unlock => Event::Unlock,
            E::Logout => Event::Logout,
            E::PermissionLost => Event::PermissionLost,
            E::PermissionRestored => Event::PermissionRestored,
        };
        let result = host
            .synthetic_observation(observation)
            .and_then(|receipt| host.apply(&receipt, now));
        if result.is_err() {
            host.faulted = true;
            host.error_visible = true;
            host.revoke();
        }
        true
    }

    #[cfg(test)]
    pub(super) fn observed_sleeping(&self) -> bool {
        self.model.borrow().sleeping
    }

    #[cfg(test)]
    pub(super) fn while_busy(&self, action: impl FnOnce()) {
        let _borrow = self.model.borrow_mut();
        action();
    }
}

#[cfg(test)]
#[path = "host_tests.rs"]
mod tests;
