//! P1-12 metadata schema and bounded synthetic audit sink only.
//! No log files, runtime logger, provider, secret store or secure-deletion claim.
//! Phase 0 supplies no numeric audit capacity/retention policy. Each fixture must
//! explicitly supply a fixed slot count, byte budgets and a native expiry. These
//! are test bounds, not new product settings or approved production defaults.
//! Existing private controllers are not imported or used as evidence issuers.
use super::clock::{Deadline, MonotonicTime};
use super::model::{
    ConfigurationRevision, ContractIdentity, ContractVersion, MicrophoneOwner, ProviderRole,
};
use super::ContractError;
use serde::{Deserialize, Serialize};
use std::num::NonZeroU64;
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Error {
    Contract(ContractError),
    Malformed,
    TooLarge,
    InvalidEvent,
    Stale,
    Busy,
    Closed,
    Expired,
    Untrusted,
    Full,
    Encoding,
}
impl From<ContractError> for Error {
    fn from(value: ContractError) -> Self {
        Self::Contract(value)
    }
}
type Result<T> = std::result::Result<T, Error>;

macro_rules! closed {
    ($name:ident { $($variant:ident),+ $(,)? }) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
        #[serde(rename_all = "snake_case")]
        enum $name { $($variant),+ }
    };
}
closed!(ConsentDomain {
    Microphone,
    BackgroundWake,
    CloudStt,
    Conversation,
    PrivateContext,
    CloudTts
});
closed!(ConsentDecision {
    ReviewRequested,
    Confirmed,
    Declined,
    Revoked,
    Expired
});
closed!(EgressOutcome {
    Authorized,
    Denied,
    Attempted,
    Uncertain,
    ResponseValidated,
    Canceled
});
closed!(InputStage {
    OwnerOff,
    AcquisitionRequested,
    CaptureActive,
    CancellationRequested,
    StoppingUnconfirmed,
    ReleaseAcknowledged,
    FramesAndQueuesCleared
});
closed!(Barrier {
    StopListening,
    Lock,
    Sleep,
    Logout,
    Quit,
    PermissionLost,
    DeviceLost,
    HostTerminated
});
closed!(StopStage {
    Requested,
    StoppingUnconfirmed,
    CleanupPending,
    LocalCleanupAcknowledged,
    TerminationUnconfirmed
});
closed!(CleanupResource {
    Input,
    FramesAndQueues,
    Output,
    Workers,
    Content,
    Audit
});
closed!(CleanupOutcome {
    Requested,
    Failed,
    Unconfirmed,
    Acknowledged
});

/// No free-form text/bytes, URLs, paths, hashes of private content, credentials,
/// tool arguments or provider-returned IDs can enter the event schema.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case", deny_unknown_fields)]
enum SecurityEvent {
    Consent {
        domain: ConsentDomain,
        decision: ConsentDecision,
    },
    Egress {
        role: ProviderRole,
        outcome: EgressOutcome,
    },
    Microphone {
        owner: MicrophoneOwner,
        stage: InputStage,
    },
    Stop {
        barrier: Barrier,
        stage: StopStage,
    },
    ConfigurationChanged {
        previous: ConfigurationRevision,
        next: ConfigurationRevision,
    },
    Cleanup {
        resource: CleanupResource,
        outcome: CleanupOutcome,
    },
}
impl SecurityEvent {
    fn validate(self, identity: &ContractIdentity) -> Result<()> {
        match self {
            Self::Microphone { owner, stage } => {
                if (owner == MicrophoneOwner::Off) != (stage == InputStage::OwnerOff) {
                    return Err(Error::InvalidEvent);
                }
            }
            Self::ConfigurationChanged { previous, next } => {
                if previous.checked_next()? != next || next != identity.configuration_revision {
                    return Err(Error::InvalidEvent);
                }
            }
            _ => (),
        }
        Ok(())
    }
}

/// A decoded description remains untrusted. It cannot be converted into native
/// evidence or restored to the append path, regardless of its source label.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Description {
    evidence: EvidenceLabel,
    version: ContractVersion,
    identity: ContractIdentity,
    sequence: NonZeroU64,
    elapsed_millis: u64,
    event: SecurityEvent,
}
struct UntrustedDescription(Description);
fn decode_description(bytes: &[u8], fixture_input_limit: usize) -> Result<UntrustedDescription> {
    if bytes.len() > fixture_input_limit {
        return Err(Error::TooLarge);
    }
    let description: Description = serde_json::from_slice(bytes).map_err(|_| Error::Malformed)?;
    description.event.validate(&description.identity)?;
    Ok(UntrustedDescription(description))
}

/// Serialize-only, fixed-size metadata, visibly synthetic. No Deserialize impl.
#[derive(Serialize)]
#[serde(transparent)]
struct Record {
    description: Description,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum EvidenceLabel {
    SyntheticNative,
}

struct Draft {
    owner: Arc<()>,
    description: Description,
    observed_at: MonotonicTime,
}
/// Opaque evidence from the explicit fake native port, never an IPC input.
#[derive(Clone)]
struct SyntheticNativeEvidence(Arc<Draft>);

/// Caller-supplied test fixture policy only. Retention is an absolute native
/// deadline and cannot be reset by events, clear, rejected input or serialization.
struct FixtureBounds {
    record_bytes: u64,
    total_bytes: u64,
}
struct AuditWindow<const CAPACITY: usize> {
    owner: Arc<()>,
    identity: ContractIdentity,
    origin: MonotonicTime,
    last_time: MonotonicTime,
    expiry: Deadline,
    bounds: FixtureBounds,
    records: [Option<Record>; CAPACITY],
    count: usize,
    bytes: u64,
    sequence: u64,
    pending: Option<SyntheticNativeEvidence>,
    closed: bool,
}
impl<const CAPACITY: usize> AuditWindow<CAPACITY> {
    fn synthetic(
        identity: ContractIdentity,
        bounds: FixtureBounds,
        expiry: Deadline,
        now: MonotonicTime,
    ) -> Result<Self> {
        if CAPACITY == 0 || bounds.record_bytes == 0 || bounds.total_bytes == 0 {
            return Err(Error::Full);
        }
        // Check the fixed arena's size arithmetic before creating any slots.
        let slots = u64::try_from(CAPACITY).map_err(|_| ContractError::Overflow)?;
        slots
            .checked_mul(std::mem::size_of::<Option<Record>>() as u64)
            .ok_or(ContractError::Overflow)?;
        if expiry.is_expired(&now)? {
            return Err(Error::Expired);
        }
        Ok(Self {
            owner: Arc::new(()),
            identity,
            origin: now.clone(),
            last_time: now,
            expiry,
            bounds,
            records: std::array::from_fn(|_| None),
            count: 0,
            bytes: 0,
            sequence: 0,
            pending: None,
            closed: false,
        })
    }
    fn close(&mut self) {
        self.closed = true;
        self.pending = None;
        for record in &mut self.records {
            *record = None;
        }
        self.count = 0;
        self.bytes = 0;
        // Reference removal only. No secure erasure, durable log or sequence reset.
    }
    fn observe(&mut self, now: &MonotonicTime) -> Result<()> {
        if self.closed {
            return Err(Error::Closed);
        }
        if let Err(error) = now.checked_duration_since(&self.last_time) {
            self.close();
            return Err(error.into());
        }
        self.last_time = now.clone();
        if self.expiry.is_expired(now)? {
            self.close();
            return Err(Error::Expired);
        }
        Ok(())
    }
    /// Explicit fake native port. It accepts finite metadata only, holds at most
    /// one immutable event, and cannot manufacture evidence for other windows.
    fn synthetic_native_event(
        &mut self,
        identity: ContractIdentity,
        event: SecurityEvent,
        now: &MonotonicTime,
    ) -> Result<SyntheticNativeEvidence> {
        self.observe(now)?;
        if identity != self.identity {
            return Err(Error::Stale);
        }
        if self.pending.is_some() {
            return Err(Error::Busy);
        }
        if self.count == CAPACITY {
            return Err(Error::Full);
        }
        event.validate(&identity)?;
        let sequence = self.sequence.checked_add(1).ok_or_else(|| {
            self.close();
            Error::Contract(ContractError::Overflow)
        })?;
        let elapsed_millis = u64::try_from(now.checked_duration_since(&self.origin)?.as_millis())
            .map_err(|_| {
            self.close();
            Error::Contract(ContractError::Overflow)
        })?;
        let evidence = SyntheticNativeEvidence(Arc::new(Draft {
            owner: self.owner.clone(),
            observed_at: now.clone(),
            description: Description {
                evidence: EvidenceLabel::SyntheticNative,
                version: ContractVersion::V1,
                identity,
                sequence: NonZeroU64::new(sequence).ok_or(ContractError::InvalidIdentifier)?,
                elapsed_millis,
                event,
            },
        }));
        self.pending = Some(evidence.clone());
        Ok(evidence)
    }
    fn append(&mut self, evidence: &SyntheticNativeEvidence, now: &MonotonicTime) -> Result<()> {
        if !self
            .pending
            .as_ref()
            .is_some_and(|p| Arc::ptr_eq(&p.0, &evidence.0))
            || !Arc::ptr_eq(&self.owner, &evidence.0.owner)
            || evidence.0.description.identity != self.identity
        {
            return Err(Error::Stale);
        }
        self.pending = None; // one append attempt, even when the sink refuses it
        self.observe(now)?;
        now.checked_duration_since(&evidence.0.observed_at)?;
        let expected = self
            .sequence
            .checked_add(1)
            .ok_or(ContractError::Overflow)?;
        if evidence.0.description.sequence.get() != expected {
            return Err(Error::Stale);
        }
        let record = Record {
            description: evidence.0.description,
        };
        let size = u64::try_from(
            serde_json::to_vec(&record)
                .map_err(|_| Error::Encoding)?
                .len(),
        )
        .map_err(|_| ContractError::Overflow)?;
        if size > self.bounds.record_bytes {
            return Err(Error::TooLarge);
        }
        let line_size = size.checked_add(1).ok_or_else(|| {
            self.close();
            Error::Contract(ContractError::Overflow)
        })?;
        let total = self.bytes.checked_add(line_size).ok_or_else(|| {
            self.close();
            Error::Contract(ContractError::Overflow)
        })?;
        if total > self.bounds.total_bytes {
            return Err(Error::TooLarge);
        }
        let slot = self.records.get_mut(self.count).ok_or(Error::Full)?;
        *slot = Some(record);
        self.count += 1;
        self.bytes = total;
        self.sequence = expected;
        Ok(())
    }
    fn serialize(&mut self, now: &MonotonicTime) -> Result<Vec<u8>> {
        self.observe(now)?;
        let capacity = usize::try_from(self.bytes).map_err(|_| ContractError::Overflow)?;
        let mut output = Vec::with_capacity(capacity);
        for record in self.records.iter().flatten() {
            serde_json::to_writer(&mut output, record).map_err(|_| Error::Encoding)?;
            output.push(b'\n');
        }
        Ok(output)
    }
    fn ingest_untrusted(&mut self, _bytes: &[u8]) -> Result<()> {
        Err(Error::Untrusted) // no JSON/source flag can attest a native outcome
    }
}

#[cfg(test)]
#[path = "audit_tests.rs"]
mod tests;
