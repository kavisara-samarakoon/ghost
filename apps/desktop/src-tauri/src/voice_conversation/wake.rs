//! P1-05: synthetic detector lifecycle only. No audio bytes, ring allocation,
//! engine, native microphone operations, renderer messages or provider authority.
use super::clock::{Deadline, MonotonicTime};
use super::limits::{MAX_WAKE_ARMING, WAKE_COOLDOWN, WAKE_RING_BUFFER};
use super::model::{AppRunId, ContractIdentity, Generation, MicrophoneOwner, WakeState};
use super::ContractError;
use std::num::NonZeroU64;
use std::sync::Arc;
use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Error {
    Contract(ContractError),
    InvalidTransition,
    Stale,
    Unauthorized,
    ModelIneligible,
    Expired,
    Suppressed,
    DetectorFailed,
    Closed,
}
impl From<ContractError> for Error {
    fn from(error: ContractError) -> Self {
        Self::Contract(error)
    }
}
type Result<T> = std::result::Result<T, Error>;

/// Test-only native validation facts. These are not a consent issuer or a lease.
struct Validation {
    identity: ContractIdentity,
    model_instance: Arc<()>,
    grant_deadline: Deadline,
    fresh_activation: bool,
    consent_valid: bool,
    model_integrity_valid: bool,
    model_license_eligible: bool,
    privacy_clear: bool,
    microphone_permission: bool,
    microphone_lease_valid: bool,
    microphone_owner: MicrophoneOwner,
    speech_active: bool,
    foreground: bool,
    background_allowed: bool,
}
impl Validation {
    fn permitted(&self) -> bool {
        self.consent_valid
            && self.privacy_clear
            && self.microphone_permission
            && (self.foreground || self.background_allowed)
    }
    fn model_eligible(&self) -> bool {
        self.model_integrity_valid && self.model_license_eligible
    }
    fn blocked(&self) -> bool {
        self.speech_active
            || self.microphone_owner != MicrophoneOwner::WakeDetection
            || !self.microphone_lease_valid
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct DetectorOperation(NonZeroU64);

#[derive(Clone, Debug)]
struct Callback {
    native_instance: Arc<()>,
    model_instance: Arc<()>,
    nonce: Arc<()>,
    identity: ContractIdentity,
    generation: Generation,
    operation: DetectorOperation,
    stage: WakeState,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Event {
    Ready,
    /// Occupancy metadata, not PCM or an actual ring buffer.
    Window {
        retained: Duration,
    },
    Detected {
        retained: Duration,
    },
    Failed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Barrier {
    Lock,
    Sleep,
    Logout,
    PermissionRevoked,
    Pause,
    DeviceLost,
    StopListening,
    Quit,
}

/// Attention metadata only. No serde, grants, provider jobs or microphone lease.
#[derive(Debug, PartialEq, Eq)]
struct AttentionCandidate {
    identity: ContractIdentity,
    generation: Generation,
    detection: DetectorOperation,
}

struct Arming {
    identity: ContractIdentity,
    model_instance: Arc<()>,
    lifetime_deadline: Deadline,
    grant_deadline: Deadline,
}

struct WakeController {
    run: AppRunId,
    native_instance: Arc<()>,
    state: WakeState,
    generation: Generation,
    operation_sequence: u64,
    activation_sequence: u64,
    retired: bool,
    last_time: MonotonicTime,
    arming: Option<Arming>,
    callback: Option<Callback>,
    cooldown_deadline: Option<Deadline>,
    retained_duration: Duration,
}

impl WakeController {
    fn new(run: AppRunId, now: MonotonicTime) -> Result<Self> {
        Ok(Self {
            run,
            native_instance: Arc::new(()),
            state: WakeState::Disabled,
            generation: Generation::from_native_sequence(1)?,
            operation_sequence: 0,
            activation_sequence: 0,
            retired: false,
            last_time: now,
            arming: None,
            callback: None,
            cooldown_deadline: None,
            retained_duration: Duration::ZERO,
        })
    }

    fn check_time(&self, now: &MonotonicTime) -> Result<()> {
        now.checked_duration_since(&self.last_time)?;
        Ok(())
    }

    /// Invalidate first, even if generation cannot advance. No hardware release
    /// is claimed: exclusive ownership and release ACKs remain external to wake.
    fn invalidate(&mut self, state: WakeState, retain_arming: bool) -> Result<()> {
        let live = self.arming.is_some() || self.callback.is_some();
        self.callback = None;
        self.retained_duration = Duration::ZERO;
        if !retain_arming {
            self.arming = None;
            self.cooldown_deadline = None;
        }
        if live {
            match self.generation.checked_next() {
                Ok(generation) => self.generation = generation,
                Err(error) => {
                    self.retired = true;
                    self.state = WakeState::Faulted;
                    self.arming = None;
                    self.cooldown_deadline = None;
                    return Err(error.into());
                }
            }
        }
        self.state = state;
        Ok(())
    }

    fn arithmetic_fault(&mut self, error: ContractError) -> Error {
        self.retired = true;
        let _ = self.invalidate(WakeState::Faulted, false);
        Error::Contract(error)
    }

    fn install_callback(&mut self, stage: WakeState) -> Result<()> {
        let sequence = self
            .operation_sequence
            .checked_add(1)
            .ok_or_else(|| self.arithmetic_fault(ContractError::Overflow))?;
        let operation =
            DetectorOperation(NonZeroU64::new(sequence).ok_or(ContractError::InvalidIdentifier)?);
        let arming = self.arming.as_ref().ok_or(Error::InvalidTransition)?;
        self.callback = Some(Callback {
            native_instance: self.native_instance.clone(),
            model_instance: arming.model_instance.clone(),
            nonce: Arc::new(()),
            identity: arming.identity,
            generation: self.generation,
            operation,
            stage,
        });
        self.operation_sequence = sequence;
        self.state = stage;
        Ok(())
    }

    /// Native activation sequencing is supplied by the synthetic caller. A new
    /// explicit activation is required after revocation; old sequences cannot replay.
    fn arm(&mut self, activation: u64, facts: &Validation, now: &MonotonicTime) -> Result<()> {
        if self.retired {
            return Err(Error::Closed);
        }
        if !matches!(
            self.state,
            WakeState::Disabled | WakeState::Suspended | WakeState::Faulted
        ) {
            return Err(Error::InvalidTransition);
        }
        self.check_time(now)?;
        let expired = facts.grant_deadline.is_expired(now)?;
        if !facts.fresh_activation
            || !facts.permitted()
            || expired
            || facts.identity.app_run_id != self.run
        {
            return Err(Error::Unauthorized);
        }
        if !facts.model_eligible() {
            return Err(Error::ModelIneligible);
        }
        if activation == 0 || activation <= self.activation_sequence {
            return Err(Error::Stale);
        }
        let lifetime_deadline = Deadline::after(now.clone(), MAX_WAKE_ARMING)
            .map_err(|error| self.arithmetic_fault(error))?;
        let generation = self
            .generation
            .checked_next()
            .map_err(|error| self.arithmetic_fault(error))?;
        self.generation = generation;
        self.arming = Some(Arming {
            identity: facts.identity,
            model_instance: facts.model_instance.clone(),
            lifetime_deadline,
            grant_deadline: facts.grant_deadline.clone(),
        });
        self.activation_sequence = activation;
        self.last_time = now.clone();
        if facts.blocked() {
            self.state = WakeState::Suppressed;
            Ok(())
        } else {
            self.install_callback(WakeState::Preparing)
        }
    }

    fn matches(&self, callback: &Callback) -> bool {
        self.callback.as_ref().is_some_and(|expected| {
            !self.retired
                && callback.identity == expected.identity
                && callback.generation == self.generation
                && callback.operation == expected.operation
                && callback.stage == self.state
                && callback.stage == expected.stage
                && Arc::ptr_eq(&callback.native_instance, &self.native_instance)
                && Arc::ptr_eq(&callback.model_instance, &expected.model_instance)
                && Arc::ptr_eq(&callback.nonce, &expected.nonce)
        })
    }

    /// Revalidate, retaining the earliest deadline seen. Facts never extend an arm.
    fn validate(&mut self, facts: &Validation, now: &MonotonicTime) -> Result<()> {
        let arming = self.arming.as_ref().ok_or(Error::InvalidTransition)?;
        if facts.identity != arming.identity
            || !Arc::ptr_eq(&facts.model_instance, &arming.model_instance)
        {
            return Err(Error::Stale);
        }
        let time_result = (|| {
            self.check_time(now)?;
            let lifetime_expired = arming.lifetime_deadline.is_expired(now)?;
            let grant_expired = arming.grant_deadline.is_expired(now)?;
            let fresh_expired = facts.grant_deadline.is_expired(now)?;
            let narrowed =
                if arming.grant_deadline.remaining(now)? <= facts.grant_deadline.remaining(now)? {
                    arming.grant_deadline.clone()
                } else {
                    facts.grant_deadline.clone()
                };
            Ok::<_, Error>((lifetime_expired || grant_expired || fresh_expired, narrowed))
        })();
        let (expired, narrowed) = match time_result {
            Ok(value) => value,
            Err(error) => {
                let _ = self.invalidate(WakeState::Faulted, false);
                return Err(error);
            }
        };
        self.last_time = now.clone();
        if expired {
            self.invalidate(WakeState::Disabled, false)?;
            return Err(Error::Expired);
        }
        if !facts.permitted() {
            self.invalidate(WakeState::Suspended, false)?;
            return Err(Error::Unauthorized);
        }
        if !facts.model_eligible() {
            self.invalidate(WakeState::Faulted, false)?;
            return Err(Error::ModelIneligible);
        }
        self.arming.as_mut().unwrap().grant_deadline = narrowed;
        if facts.blocked() {
            if self.state != WakeState::Suppressed {
                self.invalidate(WakeState::Suppressed, true)?;
            }
            return Err(Error::Suppressed);
        }
        Ok(())
    }

    /// Explicit synthetic scheduler input, never a background task. Clearing a
    /// temporary suppression may prepare within the same unexpired arming only.
    fn advance(&mut self, facts: &Validation, now: &MonotonicTime) -> Result<()> {
        self.validate(facts, now)?;
        if matches!(self.state, WakeState::Cooldown | WakeState::Suppressed) {
            if self
                .cooldown_deadline
                .as_ref()
                .map(|d| d.is_expired(now))
                .transpose()?
                .is_some_and(|expired| !expired)
            {
                self.state = WakeState::Cooldown;
            } else {
                self.cooldown_deadline = None;
                self.install_callback(WakeState::Preparing)?;
            }
        }
        Ok(())
    }

    fn handle(
        &mut self,
        callback: &Callback,
        event: Event,
        facts: &Validation,
        now: &MonotonicTime,
    ) -> Result<Option<AttentionCandidate>> {
        if !self.matches(callback) {
            return Err(Error::Stale);
        }
        if !matches!(
            (self.state, event),
            (WakeState::Preparing, Event::Ready | Event::Failed)
                | (
                    WakeState::Armed,
                    Event::Window { .. } | Event::Detected { .. } | Event::Failed
                )
        ) {
            return Err(Error::InvalidTransition);
        }
        self.validate(facts, now)?;
        match event {
            Event::Failed => {
                self.invalidate(WakeState::Faulted, false)?;
                Err(Error::DetectorFailed)
            }
            Event::Ready => {
                self.install_callback(WakeState::Armed)?;
                Ok(None)
            }
            Event::Window { retained } | Event::Detected { retained } => {
                if retained > WAKE_RING_BUFFER
                    || (matches!(event, Event::Detected { .. }) && retained.is_zero())
                {
                    self.invalidate(WakeState::Faulted, false)?;
                    return Err(ContractError::LimitExceeded.into());
                }
                if matches!(event, Event::Detected { .. }) {
                    let deadline = Deadline::after(now.clone(), WAKE_COOLDOWN)
                        .map_err(|error| self.arithmetic_fault(error))?;
                    self.invalidate(WakeState::Cooldown, true)?;
                    self.cooldown_deadline = Some(deadline);
                    Ok(Some(AttentionCandidate {
                        identity: callback.identity,
                        generation: callback.generation,
                        detection: callback.operation,
                    }))
                } else {
                    self.install_callback(WakeState::Armed)?;
                    self.retained_duration = retained;
                    Ok(None)
                }
            }
        }
    }

    /// Clock-independent revocation. These transitions never claim microphone
    /// release or mutate the separately reviewed microphone/session controllers.
    fn interrupt(&mut self, barrier: Barrier) -> Result<()> {
        let target = match barrier {
            Barrier::StopListening | Barrier::Quit => WakeState::Disabled,
            _ => WakeState::Suspended,
        };
        if self.retired {
            return Ok(());
        }
        if barrier == Barrier::Quit {
            self.retired = true;
        }
        self.invalidate(target, false)
    }
}

#[cfg(test)]
#[path = "wake_tests.rs"]
mod tests;
