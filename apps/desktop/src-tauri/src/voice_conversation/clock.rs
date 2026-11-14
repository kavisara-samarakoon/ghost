use super::{ContractError, Result};
use std::cmp::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// One allocation per clock. Retained identities cannot collide across instances,
/// even at identical elapsed times or after the original clock has been dropped.
#[derive(Clone, Debug)]
struct ClockDomain(Arc<()>);

impl ClockDomain {
    fn new() -> Self {
        Self(Arc::new(()))
    }

    fn check(&self, other: &Self) -> Result<()> {
        if Arc::ptr_eq(&self.0, &other.0) {
            Ok(())
        } else {
            Err(ContractError::ClockDomainMismatch)
        }
    }
}

/// No Eq/Ord/serde implementations: comparison requires checked domain matching.
/// These values are ephemeral elapsed times, never persisted wall-clock timestamps.
#[derive(Clone, Debug)]
pub(super) struct MonotonicTime {
    domain: ClockDomain,
    elapsed: Duration,
}

impl MonotonicTime {
    pub(super) fn checked_add(&self, duration: Duration) -> Result<Self> {
        let elapsed = self
            .elapsed
            .checked_add(duration)
            .ok_or(ContractError::Overflow)?;
        Ok(Self {
            domain: self.domain.clone(),
            elapsed,
        })
    }

    pub(super) fn checked_duration_since(&self, earlier: &Self) -> Result<Duration> {
        self.domain.check(&earlier.domain)?;
        self.elapsed
            .checked_sub(earlier.elapsed)
            .ok_or(ContractError::TimeReversed)
    }

    pub(super) fn checked_cmp(&self, other: &Self) -> Result<Ordering> {
        self.domain.check(&other.domain)?;
        Ok(self.elapsed.cmp(&other.elapsed))
    }
}

pub(super) trait MonotonicClock {
    fn now(&self) -> MonotonicTime;
}

/// Passive readings only; construction starts no timer, thread or worker.
pub(super) struct NativeMonotonicClock {
    domain: ClockDomain,
    origin: Instant,
}

impl NativeMonotonicClock {
    pub(super) fn new() -> Self {
        Self {
            domain: ClockDomain::new(),
            origin: Instant::now(),
        }
    }
}

impl MonotonicClock for NativeMonotonicClock {
    fn now(&self) -> MonotonicTime {
        MonotonicTime {
            domain: self.domain.clone(),
            elapsed: self.origin.elapsed(),
        }
    }
}

#[derive(Clone, Debug)]
pub(super) struct Deadline(MonotonicTime);

impl Deadline {
    pub(super) fn after(now: MonotonicTime, duration: Duration) -> Result<Self> {
        now.checked_add(duration).map(Self)
    }

    /// Exact equality expires the deadline.
    pub(super) fn is_expired(&self, now: &MonotonicTime) -> Result<bool> {
        Ok(now.checked_cmp(&self.0)? != Ordering::Less)
    }

    pub(super) fn remaining(&self, now: &MonotonicTime) -> Result<Duration> {
        self.0.domain.check(&now.domain)?;
        Ok(self.0.elapsed.saturating_sub(now.elapsed))
    }
}

/// Synthetic clock; advancing is checked and cannot move time backwards.
#[cfg(test)]
pub(super) struct ManualClock {
    now: MonotonicTime,
}

#[cfg(test)]
impl ManualClock {
    pub(super) fn new() -> Self {
        Self {
            now: MonotonicTime {
                domain: ClockDomain::new(),
                elapsed: Duration::ZERO,
            },
        }
    }

    pub(super) fn advance(&mut self, duration: Duration) -> Result<()> {
        self.now = self.now.checked_add(duration)?;
        Ok(())
    }
}

#[cfg(test)]
impl MonotonicClock for ManualClock {
    fn now(&self) -> MonotonicTime {
        self.now.clone()
    }
}
