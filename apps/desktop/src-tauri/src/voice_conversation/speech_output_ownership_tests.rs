use super::*;
use crate::voice_conversation::clock::{ManualClock, MonotonicClock};
use crate::voice_conversation::model::*;
use std::time::Duration;

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

fn next_session() -> ContractIdentity {
    ContractIdentity {
        session_id: SessionId::from_native_sequence(2).unwrap(),
        generation: Generation::from_native_sequence(2).unwrap(),
        ..identity(2)
    }
}

fn authority() -> OutputAuthority {
    OutputAuthority::new(identity(1).app_run_id, InitialOutput::FreshFixture).unwrap()
}

fn reserve(authority: &mut OutputAuthority) -> FlightTicket {
    let owner = authority.replace_owner(identity(1)).unwrap();
    authority.acquire(&owner, identity(1)).unwrap()
}

fn submit(authority: &mut OutputAuthority) -> FlightTicket {
    let ticket = reserve(authority);
    authority.mark_possibly_submitted(&ticket).unwrap();
    ticket
}

fn observe(
    authority: &mut OutputAuthority,
    ticket: &FlightTicket,
    observation: Observation,
) -> Result<()> {
    authority.observe(ModeledObservation {
        flight: ticket.clone(),
        observation,
    })
}

fn release(authority: &mut OutputAuthority, ticket: &FlightTicket) -> Result<ReleasedOutput> {
    authority.release(ModeledReleaseEvidence {
        flight: ticket.clone(),
    })
}

fn detach_and_retire(authority: &mut OutputAuthority, ticket: &FlightTicket) {
    observe(authority, ticket, Observation::Detach(Detach::Succeeded)).unwrap();
    observe(authority, ticket, Observation::Retired).unwrap();
}

fn observe_finish(authority: &mut OutputAuthority, ticket: &FlightTicket) {
    observe(authority, ticket, Observation::Terminal(Terminal::Finished)).unwrap();
}

#[test]
fn first_owner_reserves_one_exact_flight() {
    let mut output = authority();
    assert_eq!(output.state(), OutputState::Available);
    let owner = output.replace_owner(identity(1)).unwrap();
    let ticket = output.acquire(&owner, identity(2)).unwrap();
    assert!(ticket.owner.matches(&owner));
    assert_eq!(ticket.identity, identity(2));
    assert_eq!(ticket.operation, 1);
    assert_eq!(output.state(), OutputState::Reserved);
    assert_eq!(
        output.acquire(&owner, identity(3)).unwrap_err(),
        Error::Busy
    );
    assert!(output.flight.as_ref().unwrap().ticket.matches(&ticket));
}

#[test]
fn exact_never_submitted_reservation_can_be_cancelled_even_after_revocation() {
    let mut output = authority();
    let ticket = reserve(&mut output);
    output.revoke();
    assert_eq!(output.state(), OutputState::Reserved);
    assert_eq!(output.mark_possibly_submitted(&ticket), Err(Error::Stale));
    assert_eq!(output.request_stop(&ticket), Err(Error::InvalidTransition));
    assert_eq!(
        observe(&mut output, &ticket, Observation::Retired),
        Err(Error::InvalidTransition)
    );
    output.cancel_reservation(&ticket).unwrap();
    assert_eq!(output.state(), OutputState::Available);
    assert_eq!(output.cancel_reservation(&ticket), Err(Error::Stale));
}

#[test]
fn possible_submission_is_recorded_before_the_external_attempt() {
    let mut output = authority();
    let ticket = reserve(&mut output);
    let synthetic_external_submit = |authority: &OutputAuthority| {
        assert_eq!(authority.state(), OutputState::PossiblySubmitted);
    };
    output.mark_possibly_submitted(&ticket).unwrap();
    synthetic_external_submit(&output);
    assert_eq!(
        output.mark_possibly_submitted(&ticket),
        Err(Error::InvalidTransition)
    );
    assert_eq!(
        output.acquire(&ticket.owner, identity(2)).unwrap_err(),
        Error::Busy
    );
    assert_eq!(
        output.cancel_reservation(&ticket),
        Err(Error::ReleaseUnresolved)
    );
}

#[test]
fn failed_external_submission_keeps_the_possible_side_effect_reserved() {
    let mut output = authority();
    let ticket = reserve(&mut output);
    let synthetic_external_submit = |authority: &OutputAuthority| {
        assert_eq!(authority.state(), OutputState::PossiblySubmitted);
        Err::<(), _>("synthetic failure after dispatch")
    };
    output.mark_possibly_submitted(&ticket).unwrap();
    assert!(synthetic_external_submit(&output).is_err());
    output.submission_failed(&ticket).unwrap();
    assert_eq!(output.state(), OutputState::ReleaseUnresolved);
    assert_eq!(
        output.acquire(&ticket.owner, identity(2)).unwrap_err(),
        Error::ReleaseUnresolved
    );
    assert_eq!(
        release(&mut output, &ticket).unwrap_err(),
        Error::ReleaseUnresolved
    );
}

#[test]
fn dropping_all_driver_tickets_cannot_release_the_authoritys_output() {
    let mut output = authority();
    {
        let ticket = submit(&mut output);
        output.request_stop(&ticket).unwrap();
        // The modeled driver drops its ticket here. The app still owns the
        // obligation, even when the caller loses all evidence for that flight.
    }
    let owner = output.replace_owner(identity(2)).unwrap();
    assert_eq!(output.state(), OutputState::ReleaseUnresolved);
    assert_eq!(
        output.acquire(&owner, identity(2)).unwrap_err(),
        Error::ReleaseUnresolved
    );
    assert_eq!(output.flight.as_ref().unwrap().ticket.identity, identity(1));
}

#[test]
fn stop_attempt_results_never_substitute_for_a_terminal_callback() {
    for attempt in [
        StopAttempt::Accepted,
        StopAttempt::Rejected,
        StopAttempt::Failed,
    ] {
        let mut output = authority();
        let ticket = submit(&mut output);
        assert_eq!(
            observe(&mut output, &ticket, Observation::StopAttempt(attempt)),
            Err(Error::InvalidTransition)
        );
        output.request_stop(&ticket).unwrap();
        assert_eq!(output.flight.as_ref().unwrap().stop, Some(Stop::Requested));
        observe(&mut output, &ticket, Observation::StopAttempt(attempt)).unwrap();
        output.request_stop(&ticket).unwrap();
        assert_eq!(
            output.flight.as_ref().unwrap().stop,
            Some(Stop::Attempted(attempt))
        );
        detach_and_retire(&mut output, &ticket);
        assert_eq!(
            release(&mut output, &ticket).unwrap_err(),
            Error::ReleaseUnresolved
        );
        assert!(output.flight.as_ref().unwrap().terminal.is_none());
    }
}

#[test]
fn matching_cancellation_still_requires_detachment_retirement_and_release_evidence() {
    let mut output = authority();
    let ticket = submit(&mut output);
    output.request_stop(&ticket).unwrap();
    observe(
        &mut output,
        &ticket,
        Observation::Terminal(Terminal::CancelledAfterStop),
    )
    .unwrap();
    assert_eq!(
        release(&mut output, &ticket).unwrap_err(),
        Error::ReleaseUnresolved
    );
    observe(&mut output, &ticket, Observation::Detach(Detach::Succeeded)).unwrap();
    assert_eq!(
        release(&mut output, &ticket).unwrap_err(),
        Error::ReleaseUnresolved
    );
    observe(&mut output, &ticket, Observation::Retired).unwrap();
    assert_eq!(output.state(), OutputState::ReleaseUnresolved);
    let next_owner = output.replace_owner(identity(2)).unwrap();
    output.request_stop(&ticket).unwrap();
    assert_eq!(
        output.acquire(&next_owner, identity(2)).unwrap_err(),
        Error::ReleaseUnresolved
    );

    let receipt = release(&mut output, &ticket).unwrap();
    assert!(receipt.flight.matches(&ticket));
    assert_eq!(receipt.terminal, Terminal::CancelledAfterStop);
    assert_eq!(output.state(), OutputState::Available);
}

#[test]
fn detach_on_its_own_and_retirement_on_its_own_cannot_release() {
    for observation in [Observation::Detach(Detach::Succeeded), Observation::Retired] {
        let mut output = authority();
        let ticket = submit(&mut output);
        observe(&mut output, &ticket, observation).unwrap();
        assert_eq!(
            release(&mut output, &ticket).unwrap_err(),
            Error::ReleaseUnresolved
        );
        assert_eq!(output.state(), OutputState::ReleaseUnresolved);
    }
}

#[test]
fn failed_and_unknown_detachment_remain_unresolved_with_other_evidence_present() {
    for detach in [Detach::Failed, Detach::Unknown] {
        let mut output = authority();
        let ticket = submit(&mut output);
        observe_finish(&mut output, &ticket);
        observe(&mut output, &ticket, Observation::Retired).unwrap();
        observe(&mut output, &ticket, Observation::Detach(detach)).unwrap();
        assert_eq!(
            release(&mut output, &ticket).unwrap_err(),
            Error::ReleaseUnresolved
        );
        assert_eq!(
            observe(&mut output, &ticket, Observation::Detach(Detach::Succeeded)),
            Err(Error::DuplicateEvidence)
        );
        assert_eq!(output.flight.as_ref().unwrap().detach, Some(detach));
    }
}

#[test]
fn normal_finish_has_its_own_release_outcome() {
    let mut output = authority();
    let ticket = submit(&mut output);
    observe_finish(&mut output, &ticket);
    detach_and_retire(&mut output, &ticket);
    let receipt = release(&mut output, &ticket).unwrap();
    assert_eq!(receipt.terminal, Terminal::Finished);
    assert!(receipt.flight.matches(&ticket));
    assert_eq!(output.state(), OutputState::Available);
}

#[test]
fn finish_cannot_acknowledge_stop_before_or_after_it_arrives() {
    let mut output = authority();
    let ticket = submit(&mut output);
    assert_eq!(
        observe(
            &mut output,
            &ticket,
            Observation::Terminal(Terminal::CancelledAfterStop)
        ),
        Err(Error::InvalidTransition)
    );
    observe_finish(&mut output, &ticket);
    output.request_stop(&ticket).unwrap();
    detach_and_retire(&mut output, &ticket);
    assert_eq!(
        release(&mut output, &ticket).unwrap().terminal,
        Terminal::Finished
    );

    let ticket = submit(&mut output);
    output.request_stop(&ticket).unwrap();
    detach_and_retire(&mut output, &ticket);
    assert_eq!(
        observe(
            &mut output,
            &ticket,
            Observation::Terminal(Terminal::Finished)
        ),
        Err(Error::InvalidTransition)
    );
    assert_eq!(
        release(&mut output, &ticket).unwrap_err(),
        Error::ReleaseUnresolved
    );
    observe(
        &mut output,
        &ticket,
        Observation::Terminal(Terminal::CancelledAfterStop),
    )
    .unwrap();
    assert_eq!(
        release(&mut output, &ticket).unwrap().terminal,
        Terminal::CancelledAfterStop
    );
}

#[test]
fn unsolicited_cancellation_cannot_gain_stop_acknowledgement_by_arriving_late() {
    let mut output = authority();
    let ticket = submit(&mut output);
    output.request_stop(&ticket).unwrap();
    observe(
        &mut output,
        &ticket,
        Observation::Terminal(Terminal::CancelledWithoutStop),
    )
    .unwrap();
    detach_and_retire(&mut output, &ticket);
    assert_eq!(
        release(&mut output, &ticket).unwrap().terminal,
        Terminal::CancelledWithoutStop
    );
}

#[test]
fn mismatched_authority_owner_route_identity_operation_and_native_flight_cannot_release() {
    let mut output = authority();
    let ticket = submit(&mut output);
    let foreign_ticket = submit(&mut authority());
    // Even an otherwise identical authority/scope cannot trade tickets.
    assert_eq!(
        output
            .acquire(&foreign_ticket.owner, ticket.identity)
            .unwrap_err(),
        Error::Stale
    );
    let mut wrong_authority = ticket.clone();
    wrong_authority.owner.authority = foreign_ticket.owner.authority;
    let mut wrong_owner = ticket.clone();
    wrong_owner.owner.epoch = wrong_owner.owner.epoch.checked_next().unwrap();
    let mut wrong_route = ticket.clone();
    wrong_route.owner.route = RouteRevision(wrong_route.owner.route.0.checked_next().unwrap());
    let mut wrong_generation = ticket.clone();
    wrong_generation.identity.generation = ticket.identity.generation.checked_next().unwrap();
    let mut wrong_scope = ticket.clone();
    wrong_scope.owner.scope = next_session();
    let mut wrong_operation = ticket.clone();
    wrong_operation.operation += 1;
    let mut wrong_flight = ticket.clone();
    wrong_flight.native_flight = Arc::new(());

    observe_finish(&mut output, &ticket);
    detach_and_retire(&mut output, &ticket);
    for wrong in [
        wrong_authority,
        wrong_owner,
        wrong_route,
        wrong_generation,
        wrong_scope,
        wrong_operation,
        wrong_flight,
    ] {
        assert_eq!(output.request_stop(&wrong), Err(Error::Stale));
        assert_eq!(output.cancel_reservation(&wrong), Err(Error::Stale));
        assert_eq!(
            observe(&mut output, &wrong, Observation::Retired),
            Err(Error::Stale)
        );
        assert_eq!(release(&mut output, &wrong).unwrap_err(), Error::Stale);
    }
    assert_eq!(
        release(&mut output, &ticket).unwrap().terminal,
        Terminal::Finished
    );
}

#[test]
fn duplicate_evidence_and_replayed_release_never_change_a_flight() {
    let mut output = authority();
    let ticket = submit(&mut output);
    output.request_stop(&ticket).unwrap();
    for observation in [
        Observation::StopAttempt(StopAttempt::Accepted),
        Observation::Terminal(Terminal::CancelledAfterStop),
        Observation::Detach(Detach::Succeeded),
        Observation::Retired,
    ] {
        observe(&mut output, &ticket, observation).unwrap();
        assert_eq!(
            observe(&mut output, &ticket, observation),
            Err(Error::DuplicateEvidence)
        );
    }
    release(&mut output, &ticket).unwrap();
    assert_eq!(release(&mut output, &ticket).unwrap_err(), Error::Stale);

    let owner = output.replace_owner(identity(2)).unwrap();
    let replacement = output.acquire(&owner, identity(2)).unwrap();
    assert_eq!(release(&mut output, &ticket).unwrap_err(), Error::Stale);
    assert!(output.flight.as_ref().unwrap().ticket.matches(&replacement));
    assert_eq!(output.state(), OutputState::Reserved);
}

#[test]
fn replacing_the_session_keeps_old_obligation_and_old_proof_cannot_touch_new_flight() {
    let mut output = authority();
    let old = submit(&mut output);
    let new_identity = next_session();
    let owner = output.replace_owner(new_identity).unwrap();
    assert_eq!(
        output.acquire(&old.owner, old.identity).unwrap_err(),
        Error::Stale
    );
    assert_eq!(
        output.acquire(&owner, old.identity).unwrap_err(),
        Error::Stale
    );
    assert_eq!(
        output.acquire(&owner, new_identity).unwrap_err(),
        Error::ReleaseUnresolved
    );
    assert_eq!(output.mark_possibly_submitted(&old), Err(Error::Stale));

    output.request_stop(&old).unwrap();
    observe(
        &mut output,
        &old,
        Observation::Terminal(Terminal::CancelledAfterStop),
    )
    .unwrap();
    detach_and_retire(&mut output, &old);
    assert!(release(&mut output, &old).unwrap().flight.matches(&old));

    let new_flight = output.acquire(&owner, new_identity).unwrap();
    assert_eq!(
        observe(&mut output, &old, Observation::Retired),
        Err(Error::Stale)
    );
    assert_eq!(release(&mut output, &old).unwrap_err(), Error::Stale);
    assert!(output.flight.as_ref().unwrap().ticket.matches(&new_flight));
}

#[test]
fn route_change_revokes_submission_without_releasing_the_old_route() {
    for submitted in [false, true] {
        let mut output = authority();
        let old = reserve(&mut output);
        if submitted {
            output.mark_possibly_submitted(&old).unwrap();
        }
        output.change_route().unwrap();
        let new_owner = output.replace_owner(identity(2)).unwrap();
        assert_ne!(old.owner.route, new_owner.route);
        assert_eq!(output.mark_possibly_submitted(&old), Err(Error::Stale));
        assert!(output.acquire(&new_owner, identity(2)).is_err());

        if submitted {
            assert_eq!(output.state(), OutputState::ReleaseUnresolved);
            assert_eq!(
                output.cancel_reservation(&old),
                Err(Error::ReleaseUnresolved)
            );
            output.request_stop(&old).unwrap();
            observe(
                &mut output,
                &old,
                Observation::Terminal(Terminal::CancelledAfterStop),
            )
            .unwrap();
            detach_and_retire(&mut output, &old);
            let mut wrong_route = old.clone();
            wrong_route.owner.route = new_owner.route;
            assert_eq!(
                release(&mut output, &wrong_route).unwrap_err(),
                Error::Stale
            );
            release(&mut output, &old).unwrap();
        } else {
            assert_eq!(output.state(), OutputState::Reserved);
            output.cancel_reservation(&old).unwrap();
        }
        assert!(output.acquire(&new_owner, identity(2)).is_ok());
    }
}

#[test]
fn close_without_background_scope_and_quit_during_preparation_prevent_submission() {
    for quitting in [false, true] {
        let mut output = authority();
        let ticket = reserve(&mut output);
        // Close without valid background scope is a revocation from the host.
        // Quit additionally closes admission for the entire app lifetime.
        let error = if quitting {
            output.shutdown();
            Error::Closed
        } else {
            output.revoke();
            Error::Stale
        };
        assert_eq!(output.state(), OutputState::Reserved);
        assert_eq!(output.mark_possibly_submitted(&ticket), Err(error));
        output.cancel_reservation(&ticket).unwrap();
        if quitting {
            assert_eq!(
                output.replace_owner(identity(2)).unwrap_err(),
                Error::Closed
            );
        }
    }
}

#[test]
fn shutdown_cannot_claim_native_cleanup_and_exact_evidence_cannot_reopen_admission() {
    let mut output = authority();
    let ticket = submit(&mut output);
    output.shutdown();
    output.shutdown();
    assert_eq!(output.state(), OutputState::ReleaseUnresolved);
    assert_eq!(
        output.replace_owner(identity(2)).unwrap_err(),
        Error::Closed
    );
    assert_eq!(
        release(&mut output, &ticket).unwrap_err(),
        Error::ReleaseUnresolved
    );
    output.request_stop(&ticket).unwrap();
    observe(
        &mut output,
        &ticket,
        Observation::Terminal(Terminal::CancelledAfterStop),
    )
    .unwrap();
    detach_and_retire(&mut output, &ticket);
    release(&mut output, &ticket).unwrap();
    assert_eq!(output.state(), OutputState::Available);
    assert_eq!(
        output.replace_owner(identity(2)).unwrap_err(),
        Error::Closed
    );
}

#[test]
fn simulated_restart_with_unknown_prior_output_remains_blocked() {
    let mut previous = authority();
    let old = submit(&mut previous);
    previous.shutdown();
    let run = AppRunId::from_native_sequence(2).unwrap();
    let scope = ContractIdentity {
        app_run_id: run,
        ..identity(1)
    };
    let mut restarted = OutputAuthority::new(run, InitialOutput::RestartUnknown).unwrap();
    assert_eq!(restarted.state(), OutputState::ReleaseUnresolved);
    let owner = restarted.replace_owner(scope).unwrap();
    assert_eq!(
        restarted.acquire(&owner, scope).unwrap_err(),
        Error::ReleaseUnresolved
    );
    assert_eq!(release(&mut restarted, &old).unwrap_err(), Error::Stale);
    restarted.change_route().unwrap();
    let fresh_owner = restarted.replace_owner(scope).unwrap();
    assert_eq!(
        restarted.acquire(&fresh_owner, scope).unwrap_err(),
        Error::ReleaseUnresolved
    );
}

#[test]
fn checked_owner_and_route_generation_exhaustion_close_admission_with_old_flight_intact() {
    for route_overflow in [false, true] {
        let mut output = authority();
        let ticket = submit(&mut output);
        let exhausted = Generation::from_native_sequence(u64::MAX).unwrap();
        let result = if route_overflow {
            output.route = RouteRevision(exhausted);
            output.change_route()
        } else {
            output.epoch = exhausted;
            output.replace_owner(identity(2)).map(|_| ())
        };
        assert_eq!(result, Err(Error::Contract(ContractError::Overflow)));
        assert_eq!(output.state(), OutputState::ReleaseUnresolved);
        assert!(output.flight.as_ref().unwrap().ticket.matches(&ticket));
        assert_eq!(
            output.acquire(&ticket.owner, identity(2)).unwrap_err(),
            Error::Closed
        );
        assert_eq!(
            output.replace_owner(identity(2)).unwrap_err(),
            Error::Closed
        );
    }
}

#[test]
fn operation_exhaustion_never_wraps_or_acquires_output() {
    let mut output = authority();
    let owner = output.replace_owner(identity(1)).unwrap();
    output.last_operation = u64::MAX;
    assert_eq!(
        output.acquire(&owner, identity(1)).unwrap_err(),
        Error::Contract(ContractError::Overflow)
    );
    assert_eq!(
        output.replace_owner(identity(2)).unwrap_err(),
        Error::Closed
    );
    assert_eq!(output.state(), OutputState::Available);
}

#[test]
fn timeouts_elapsed_time_and_token_drop_cannot_prove_release() {
    let mut output = authority();
    let ticket = submit(&mut output);
    output.request_stop(&ticket).unwrap();
    observe(
        &mut output,
        &ticket,
        Observation::StopAttempt(StopAttempt::Accepted),
    )
    .unwrap();

    let mut clock = ManualClock::new();
    let start = clock.now();
    clock.advance(Duration::from_secs(86_400)).unwrap();
    assert_eq!(
        clock.now().checked_duration_since(&start).unwrap(),
        Duration::from_secs(86_400)
    );
    drop(ticket.clone());
    drop(ModeledReleaseEvidence {
        flight: ticket.clone(),
    });
    assert_eq!(output.state(), OutputState::ReleaseUnresolved);
    assert_eq!(
        release(&mut output, &ticket).unwrap_err(),
        Error::ReleaseUnresolved
    );
}

#[test]
fn release_retires_ownership_without_reinstating_revoked_submission_authority() {
    let mut output = authority();
    let ticket = submit(&mut output);
    output.request_stop(&ticket).unwrap();
    observe(
        &mut output,
        &ticket,
        Observation::Terminal(Terminal::CancelledAfterStop),
    )
    .unwrap();
    detach_and_retire(&mut output, &ticket);
    let receipt = release(&mut output, &ticket).unwrap();

    assert!(output.owner.is_none());
    assert_eq!(
        output.mark_possibly_submitted(&receipt.flight),
        Err(Error::Stale)
    );
    assert_eq!(
        output.acquire(&ticket.owner, identity(2)).unwrap_err(),
        Error::Stale
    );
    assert_eq!(output.state(), OutputState::Available);
}
