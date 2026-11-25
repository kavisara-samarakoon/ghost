//! P2-03A synthetic application-level output ownership. This authority is
//! independent of a logical driver and owns no native objects or speech text.
//! Tickets represent occupancy only. They cannot grant permission to speak.
use super::model::{AppRunId, ContractIdentity, Generation};
use super::ContractError;
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Error {
    Contract(ContractError),
    Closed,
    Busy,
    Stale,
    InvalidTransition,
    DuplicateEvidence,
    ReleaseUnresolved,
}

type Result<T> = std::result::Result<T, Error>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct RouteRevision(Generation);

#[derive(Clone, Debug)]
struct OwnerTicket {
    authority: Arc<()>,
    scope: ContractIdentity,
    epoch: Generation,
    route: RouteRevision,
}

impl OwnerTicket {
    fn matches(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.authority, &other.authority)
            && self.scope == other.scope
            && self.epoch == other.epoch
            && self.route == other.route
    }

    fn covers(&self, identity: ContractIdentity) -> bool {
        // Match the Phase 1 Stop scope: the turn and job can vary within the
        // bound interaction/session, generation, configuration and consent.
        self.scope
            == ContractIdentity {
                turn_id: self.scope.turn_id,
                job_id: self.scope.job_id,
                ..identity
            }
    }
}

#[derive(Clone, Debug)]
struct FlightTicket {
    owner: OwnerTicket,
    identity: ContractIdentity,
    operation: u64,
    native_flight: Arc<()>,
}

impl FlightTicket {
    fn matches(&self, other: &Self) -> bool {
        self.owner.matches(&other.owner)
            && self.identity == other.identity
            && self.operation == other.operation
            && Arc::ptr_eq(&self.native_flight, &other.native_flight)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StopAttempt {
    Accepted,
    Rejected,
    Failed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stop {
    Requested,
    Attempted(StopAttempt),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Terminal {
    Finished,
    CancelledAfterStop,
    CancelledWithoutStop,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Detach {
    Succeeded,
    Failed,
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Observation {
    StopAttempt(StopAttempt),
    Terminal(Terminal),
    Detach(Detach),
    Retired,
}

// The child test module supplies these fixtures. There is no live issuer,
// public constructor or deserialization from an untrusted boundary.
struct ModeledObservation {
    flight: FlightTicket,
    observation: Observation,
}

/// Independent release evidence for the modeled flight on its original route.
/// This fixture asserts the release criterion in tests. AVFoundation terminal
/// events, detachment and retirement alone cannot construct it.
struct ModeledReleaseEvidence {
    flight: FlightTicket,
}

/// An exact accounting result for the released flight. It has no submission
/// authority and is never accepted back as evidence by this contract.
#[derive(Debug)]
struct ReleasedOutput {
    flight: FlightTicket,
    terminal: Terminal,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Reserved,
    PossiblySubmitted,
    ReleaseUnresolved,
}

struct Flight {
    ticket: FlightTicket,
    phase: Phase,
    stop: Option<Stop>,
    terminal: Option<Terminal>,
    detach: Option<Detach>,
    retired: bool,
}

impl Flight {
    fn observe(&mut self, observation: Observation) -> Result<()> {
        match observation {
            Observation::StopAttempt(attempt) => match self.stop {
                Some(Stop::Requested) => self.stop = Some(Stop::Attempted(attempt)),
                Some(Stop::Attempted(_)) => return Err(Error::DuplicateEvidence),
                None => return Err(Error::InvalidTransition),
            },
            Observation::Terminal(terminal) => {
                if self.terminal.is_some() {
                    return Err(Error::DuplicateEvidence);
                }
                match terminal {
                    // A finish first seen after Stop cannot settle the stopped
                    // flight. An earlier recorded Finish keeps its own meaning.
                    Terminal::Finished if self.stop.is_some() => {
                        return Err(Error::InvalidTransition);
                    }
                    Terminal::CancelledAfterStop if self.stop.is_none() => {
                        return Err(Error::InvalidTransition);
                    }
                    Terminal::Finished
                    | Terminal::CancelledAfterStop
                    | Terminal::CancelledWithoutStop => self.terminal = Some(terminal),
                }
            }
            Observation::Detach(detach) if self.detach.is_none() => self.detach = Some(detach),
            Observation::Retired if !self.retired => self.retired = true,
            Observation::Detach(_) | Observation::Retired => return Err(Error::DuplicateEvidence),
        }
        self.phase = Phase::ReleaseUnresolved;
        Ok(())
    }

    /// The caller must also present separate, exactly matched release evidence.
    /// A stop attempt's return value is deliberately absent from this policy.
    fn release_terminal(&self) -> Result<Terminal> {
        if self.detach != Some(Detach::Succeeded) || !self.retired {
            return Err(Error::ReleaseUnresolved);
        }
        self.terminal.ok_or(Error::ReleaseUnresolved)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum OutputState {
    Available,
    Reserved,
    PossiblySubmitted,
    ReleaseUnresolved,
}

enum InitialOutput {
    /// A synthetic fresh authority with no inherited output obligation.
    FreshFixture,
    /// Restart without verified prior output release. There is no recovery
    /// path for this state in P2-03A.
    RestartUnknown,
}

struct OutputAuthority {
    run: AppRunId,
    instance: Arc<()>,
    epoch: Generation,
    route: RouteRevision,
    last_operation: u64,
    owner: Option<OwnerTicket>,
    flight: Option<Flight>,
    unknown_prior_output: bool,
    closed: bool,
}

impl OutputAuthority {
    // Kept private: creating an authority for a live app and reconciling its
    // startup state are future responsibilities of the trusted app lifecycle.
    fn new(run: AppRunId, initial: InitialOutput) -> Result<Self> {
        let first = Generation::from_native_sequence(1).map_err(Error::Contract)?;
        Ok(Self {
            run,
            instance: Arc::new(()),
            epoch: first,
            route: RouteRevision(first),
            last_operation: 0,
            owner: None,
            flight: None,
            unknown_prior_output: matches!(initial, InitialOutput::RestartUnknown),
            closed: false,
        })
    }

    fn state(&self) -> OutputState {
        match self.flight.as_ref().map(|flight| flight.phase) {
            Some(Phase::Reserved) => OutputState::Reserved,
            Some(Phase::PossiblySubmitted) => OutputState::PossiblySubmitted,
            Some(Phase::ReleaseUnresolved) => OutputState::ReleaseUnresolved,
            None if self.unknown_prior_output => OutputState::ReleaseUnresolved,
            None => OutputState::Available,
        }
    }

    fn check_open(&self) -> Result<()> {
        if self.closed {
            return Err(Error::Closed);
        }
        Ok(())
    }

    fn check_owner(&self, owner: &OwnerTicket) -> Result<()> {
        self.check_open()?;
        if !self
            .owner
            .as_ref()
            .is_some_and(|current| current.matches(owner))
        {
            return Err(Error::Stale);
        }
        Ok(())
    }

    fn replace_owner(&mut self, scope: ContractIdentity) -> Result<OwnerTicket> {
        self.check_open()?;
        if scope.app_run_id != self.run {
            return Err(Error::Stale);
        }
        self.revoke();
        self.epoch = self
            .epoch
            .checked_next()
            .map_err(|error| self.close_on_error(error))?;
        let ticket = OwnerTicket {
            authority: Arc::clone(&self.instance),
            scope,
            epoch: self.epoch,
            route: self.route,
        };
        self.owner = Some(ticket.clone());
        Ok(ticket)
    }

    fn change_route(&mut self) -> Result<()> {
        self.check_open()?;
        self.revoke();
        let revision = self
            .route
            .0
            .checked_next()
            .map_err(|error| self.close_on_error(error))?;
        self.route = RouteRevision(revision);
        Ok(())
    }

    fn acquire(&mut self, owner: &OwnerTicket, identity: ContractIdentity) -> Result<FlightTicket> {
        self.check_owner(owner)?;
        if !owner.covers(identity) {
            return Err(Error::Stale);
        }
        if self.unknown_prior_output {
            return Err(Error::ReleaseUnresolved);
        }
        if let Some(flight) = &self.flight {
            return Err(match flight.phase {
                Phase::ReleaseUnresolved => Error::ReleaseUnresolved,
                Phase::Reserved | Phase::PossiblySubmitted => Error::Busy,
            });
        }
        let operation = self
            .last_operation
            .checked_add(1)
            .ok_or_else(|| self.close_on_error(ContractError::Overflow))?;
        self.last_operation = operation;
        let ticket = FlightTicket {
            owner: owner.clone(),
            identity,
            operation,
            native_flight: Arc::new(()),
        };
        self.flight = Some(Flight {
            ticket: ticket.clone(),
            phase: Phase::Reserved,
            stop: None,
            terminal: None,
            detach: None,
            retired: false,
        });
        Ok(ticket)
    }

    fn flight_mut(&mut self, ticket: &FlightTicket) -> Result<&mut Flight> {
        // Cleanup uses the recorded flight binding. It may outlive its owner,
        // route or session and remains meaningful after authority is revoked.
        self.flight
            .as_mut()
            .filter(|flight| flight.ticket.matches(ticket))
            .ok_or(Error::Stale)
    }

    fn submitted_mut(&mut self, ticket: &FlightTicket) -> Result<&mut Flight> {
        let flight = self.flight_mut(ticket)?;
        if flight.phase == Phase::Reserved {
            return Err(Error::InvalidTransition);
        }
        Ok(flight)
    }

    /// The trusted submit path must complete this transition before any
    /// external submission call. Abandoning the attempt afterward is ambiguous.
    fn mark_possibly_submitted(&mut self, ticket: &FlightTicket) -> Result<()> {
        self.check_owner(&ticket.owner)?;
        let flight = self.flight_mut(ticket)?;
        if flight.phase != Phase::Reserved {
            return Err(Error::InvalidTransition);
        }
        flight.phase = Phase::PossiblySubmitted;
        Ok(())
    }

    fn submission_failed(&mut self, ticket: &FlightTicket) -> Result<()> {
        self.submitted_mut(ticket)?.phase = Phase::ReleaseUnresolved;
        Ok(())
    }

    /// Only the exact reservation that never crossed the submission transition
    /// can be abandoned without native cleanup evidence.
    fn cancel_reservation(&mut self, ticket: &FlightTicket) -> Result<()> {
        if self.flight_mut(ticket)?.phase != Phase::Reserved {
            return Err(Error::ReleaseUnresolved);
        }
        self.flight = None;
        Ok(())
    }

    fn request_stop(&mut self, ticket: &FlightTicket) -> Result<()> {
        let flight = self.submitted_mut(ticket)?;
        flight.stop.get_or_insert(Stop::Requested);
        flight.phase = Phase::ReleaseUnresolved;
        // Stop revokes the stopped owner. Cleanup for an old flight must
        // leave a newer owner bound, while that flight still blocks admission.
        if self
            .owner
            .as_ref()
            .is_some_and(|owner| owner.matches(&ticket.owner))
        {
            self.revoke();
        }
        Ok(())
    }

    fn observe(&mut self, evidence: ModeledObservation) -> Result<()> {
        self.submitted_mut(&evidence.flight)?
            .observe(evidence.observation)
    }

    fn release(&mut self, evidence: ModeledReleaseEvidence) -> Result<ReleasedOutput> {
        let flight = self.submitted_mut(&evidence.flight)?;
        let terminal = flight.release_terminal()?;
        let flight = flight.ticket.clone();
        self.flight = None;
        Ok(ReleasedOutput { flight, terminal })
    }

    fn revoke(&mut self) {
        self.owner = None;
        if let Some(flight) = &mut self.flight {
            if flight.phase == Phase::PossiblySubmitted {
                flight.phase = Phase::ReleaseUnresolved;
            }
        }
    }

    fn shutdown(&mut self) {
        self.closed = true;
        self.revoke();
    }

    fn close_on_error(&mut self, error: ContractError) -> Error {
        self.shutdown();
        Error::Contract(error)
    }
}

#[cfg(test)]
#[path = "speech_output_ownership_tests.rs"]
mod tests;
