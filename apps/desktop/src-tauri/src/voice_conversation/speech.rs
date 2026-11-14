//! P1-03: deterministic bookkeeping only; no synthesizer, capture or dispatch.
//! Validation is synthetic input from future independent gates, never a grant.
use super::clock::{Deadline, MonotonicTime};
use super::limits::{
    checked_total, SpeechCapacity, SpeechCapacityLease, SpeechSlot, MAX_SPEECH_JOB_AUDIO_BYTES,
    SPEECH_OUTPUT_DEADLINE,
};
use super::model::*;
use super::ContractError;
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Error {
    Contract(ContractError),
    QueueFull,
    StaleCallback,
    InvalidTransition,
    Unauthorized,
    Closed,
}
impl From<ContractError> for Error {
    fn from(error: ContractError) -> Self {
        Self::Contract(error)
    }
}
type Result<T> = std::result::Result<T, Error>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Preparation {
    Local,
    Cloud,
}

/// Test-only facts. No serde, presenter, consent issuance or provider capability.
struct Validation {
    identity: ContractIdentity,
    role: ProviderRole,
    deadline: Deadline,
    source_eligible: bool,
    speaker_allowed: bool,
    cloud_tts_allowed: bool,
}
impl Validation {
    fn allowed(&self, preparation: Preparation) -> bool {
        self.role == ProviderRole::Tts
            && self.source_eligible
            && self.speaker_allowed
            && (preparation == Preparation::Local || self.cloud_tts_allowed)
    }
}

#[derive(Clone)]
struct Callback {
    native_owner: Arc<()>,
    identity: ContractIdentity,
    epoch: Generation,
    operation: u64,
    stage: SpeechState,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Event {
    Prepared,
    AudioReady { bytes: u64 },
    Completed,
    Failed,
}

struct Job {
    callback: Callback,
    preparation: Preparation,
    text: String,
    output_deadline: Deadline,
    grant_deadline: Deadline,
    lease: SpeechCapacityLease,
    slot: SpeechSlot,
}

/// Text is returned as data, never included in diagnostic/error formatting.
struct Finished {
    identity: ContractIdentity,
    outcome: SpeechOutcome,
    text: String,
    /// Candidate only: external session/consent validation must still run.
    follow_up_candidate: bool,
}
type Update = [Option<Finished>; 2];

struct SpeechController {
    run: AppRunId,
    interaction: InteractionId,
    native_owner: Arc<()>,
    epoch: Generation,
    operation: u64,
    last_job_sequence: u64,
    closed: bool,
    last_time: MonotonicTime,
    capacity: SpeechCapacity,
    active: Option<Job>,
    pending: Option<Job>,
}

impl SpeechController {
    fn new(run: AppRunId, interaction: InteractionId, now: MonotonicTime) -> Result<Self> {
        Ok(Self {
            run,
            interaction,
            native_owner: Arc::new(()),
            epoch: Generation::from_native_sequence(1)?,
            operation: 0,
            last_job_sequence: 0,
            closed: false,
            last_time: now,
            capacity: SpeechCapacity::new(),
            active: None,
            pending: None,
        })
    }

    fn state(&self) -> SpeechState {
        self.active
            .as_ref()
            .map_or(SpeechState::Idle, |job| job.callback.stage)
    }

    fn check_time(&self, now: &MonotonicTime) -> Result<()> {
        now.checked_duration_since(&self.last_time)?;
        Ok(())
    }

    /// The native interaction allocator supplies its monotonically increasing
    /// job sequence alongside the typed ID. Gaps are allowed; replay is not.
    fn enqueue(
        &mut self,
        sequence: u64,
        text: String,
        preparation: Preparation,
        validation: &Validation,
        now: &MonotonicTime,
    ) -> Result<()> {
        if self.closed {
            return Err(Error::Closed);
        }
        self.check_time(now)?;
        let identity = validation.identity;
        if identity.app_run_id != self.run
            || identity.interaction_id != self.interaction
            || identity.job_id != JobId::from_native_sequence(sequence)?
            || !validation.allowed(preparation)
            || validation.deadline.is_expired(now)?
            || text.is_empty()
        {
            return Err(Error::Unauthorized);
        }
        if sequence <= self.last_job_sequence {
            return Err(ContractError::DuplicateJob.into());
        }
        if self.active.is_some() && self.pending.is_some() {
            return Err(Error::QueueFull);
        }
        let output_deadline = Deadline::after(now.clone(), SPEECH_OUTPUT_DEADLINE)?;
        let operation = self
            .operation
            .checked_add(1)
            .ok_or(ContractError::Overflow)?;
        // Capacity slots are lease locations, not execution authority. Promotion
        // retains the pending job's exact receipt; the next admission uses the
        // vacated location. No release/reacquire or accounting reset is needed.
        let slot = match self.active.as_ref().map(|job| job.slot) {
            Some(SpeechSlot::Active) => SpeechSlot::Pending,
            Some(SpeechSlot::Pending) | None => SpeechSlot::Active,
        };
        let lease = self.capacity.occupy(slot, identity.job_id, &text, 0)?;
        let job = Job {
            callback: Callback {
                native_owner: self.native_owner.clone(),
                identity,
                epoch: self.epoch,
                operation,
                stage: SpeechState::Preparing,
            },
            preparation,
            text,
            output_deadline,
            grant_deadline: validation.deadline.clone(),
            lease,
            slot,
        };
        if self.active.is_none() {
            self.active = Some(job);
        } else {
            self.pending = Some(job);
        }
        self.operation = operation;
        self.last_job_sequence = sequence;
        self.last_time = now.clone();
        Ok(())
    }

    fn callback(&self) -> Option<Callback> {
        self.active.as_ref().map(|job| job.callback.clone())
    }

    fn matches(&self, callback: &Callback) -> bool {
        self.active.as_ref().is_some_and(|job| {
            !self.closed
                && Arc::ptr_eq(&callback.native_owner, &self.native_owner)
                && callback.identity == job.callback.identity
                && callback.epoch == self.epoch
                && callback.operation == job.callback.operation
                && callback.stage == job.callback.stage
        })
    }

    fn expired(job: &Job, now: &MonotonicTime) -> Result<bool> {
        let output = job.output_deadline.is_expired(now)?;
        let grant = job.grant_deadline.is_expired(now)?;
        Ok(output || grant)
    }

    fn finish(&mut self, job: Job, outcome: SpeechOutcome, candidate: bool) -> Result<Finished> {
        self.capacity.release(&job.lease)?;
        Ok(Finished {
            identity: job.callback.identity,
            outcome,
            text: job.text,
            follow_up_candidate: candidate,
        })
    }

    fn fail_all(&mut self) -> Result<Update> {
        let mut result = [None, None];
        if let Some(job) = self.active.take() {
            result[0] = Some(self.finish(job, SpeechOutcome::Failure, false)?);
        }
        if let Some(job) = self.pending.take() {
            result[1] = Some(self.finish(job, SpeechOutcome::Failure, false)?);
        }
        Ok(result)
    }

    fn handle(
        &mut self,
        callback: &Callback,
        event: Event,
        validation: &Validation,
        now: &MonotonicTime,
    ) -> Result<Update> {
        if !self.matches(callback) || validation.identity != callback.identity {
            return Err(Error::StaleCallback);
        }
        let job = self.active.as_ref().unwrap();
        let next = match (job.callback.stage, job.preparation, event) {
            (SpeechState::Preparing, Preparation::Local, Event::Prepared)
            | (SpeechState::Fetching, Preparation::Cloud, Event::AudioReady { .. }) => {
                SpeechState::Speaking
            }
            (SpeechState::Preparing, Preparation::Cloud, Event::Prepared) => SpeechState::Fetching,
            (SpeechState::Speaking, _, Event::Completed) | (_, _, Event::Failed) => {
                job.callback.stage
            }
            _ => return Err(Error::InvalidTransition),
        };
        self.check_time(now)?;
        let expired = Self::expired(job, now)?;
        let fresh_expired = validation.deadline.is_expired(now)?;
        // Check the pending domain before any retirement or promotion mutation.
        let pending_expired = self
            .pending
            .as_ref()
            .map(|job| Self::expired(job, now))
            .transpose()?
            .unwrap_or(false);
        let invalid_audio = match event {
            Event::AudioReady { bytes } => {
                bytes == 0 || checked_total(0, bytes, MAX_SPEECH_JOB_AUDIO_BYTES).is_err()
            }
            _ => false,
        };
        if expired
            || fresh_expired
            || !validation.allowed(job.preparation)
            || event == Event::Failed
            || invalid_audio
        {
            let result = self.fail_all()?;
            self.last_time = now.clone();
            return Ok(result); // no retry, fallback or automatic pending output
        }
        let mut result = [None, None];
        if event == Event::Completed {
            let candidate = self.pending.is_none();
            let job = self.active.take().unwrap();
            result[0] = Some(self.finish(job, SpeechOutcome::NormalCompletion, candidate)?);
            if let Some(job) = self.pending.take() {
                if pending_expired {
                    result[1] = Some(self.finish(job, SpeechOutcome::Failure, false)?);
                } else {
                    // Preparing only. No synthesis/provider work is dispatched.
                    self.active = Some(job);
                }
            }
        } else {
            self.active.as_mut().unwrap().callback.stage = next;
        }
        self.last_time = now.clone();
        Ok(result)
    }

    fn tick(&mut self, now: &MonotonicTime) -> Result<Update> {
        self.check_time(now)?;
        let active_expired = self
            .active
            .as_ref()
            .map(|job| Self::expired(job, now))
            .transpose()?
            .unwrap_or(false);
        let pending_expired = self
            .pending
            .as_ref()
            .map(|job| Self::expired(job, now))
            .transpose()?
            .unwrap_or(false);
        let result = if active_expired {
            self.fail_all()?
        } else if pending_expired {
            let job = self.pending.take().unwrap();
            [None, Some(self.finish(job, SpeechOutcome::Failure, false)?)]
        } else {
            [None, None]
        };
        self.last_time = now.clone();
        Ok(result)
    }

    /// Independent of clocks, validation and provider work. Overflow permanently
    /// closes admission, but cannot prevent queue clearing or exact lease release.
    fn stop(&mut self) -> Result<SpeechOutcome> {
        let next = self.epoch.checked_next();
        if let Ok(epoch) = next {
            self.epoch = epoch;
        } else {
            self.closed = true;
        }
        let active = self
            .active
            .take()
            .map(|job| self.capacity.release(&job.lease));
        let pending = self
            .pending
            .take()
            .map(|job| self.capacity.release(&job.lease));
        if let Some(result) = active {
            result?;
        }
        if let Some(result) = pending {
            result?;
        }
        next?;
        Ok(SpeechOutcome::Stop)
    }
}

#[cfg(test)]
#[path = "speech_tests.rs"]
mod tests;
