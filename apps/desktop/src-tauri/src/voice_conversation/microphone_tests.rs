//! Deterministic metadata and fake native evidence only; no audio/device access.
use super::*;
use crate::voice_conversation::clock::{ManualClock, MonotonicClock};

const OWNERS: [MicrophoneOwner; 3] = [
    MicrophoneOwner::ManualForeground,
    MicrophoneOwner::WakeDetection,
    MicrophoneOwner::ConversationTurn,
];

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

struct Fixture {
    clock: ManualClock,
    mic: MicrophoneCoordinator,
}
impl Fixture {
    fn new() -> Self {
        let clock = ManualClock::new();
        let mic = MicrophoneCoordinator::new(identity(1).app_run_id, clock.now()).unwrap();
        Self { clock, mic }
    }
    fn facts(&self, sequence: u64) -> Validation {
        Validation {
            identity: identity(sequence),
            deadline: Deadline::after(self.clock.now(), Duration::from_secs(900)).unwrap(),
            activation_allowed: true,
            background_allowed: true,
            explicit_manual_start: true,
            foreground: true,
            privacy_clear: true,
            output_idle: true,
        }
    }
    fn request(&mut self, owner: MicrophoneOwner, sequence: u64) -> RequestTicket {
        self.mic
            .request(
                owner,
                MAX_CAPTURED_TURN,
                &self.facts(sequence),
                &self.clock.now(),
            )
            .unwrap()
    }
    fn begin(&mut self, ticket: &RequestTicket) -> Lease {
        let mut facts = self.facts(1);
        facts.identity = ticket.identity;
        self.mic.begin(ticket, &facts, &self.clock.now()).unwrap()
    }
    fn acquire(&mut self, owner: MicrophoneOwner) -> Lease {
        let ticket = self.request(owner, 1);
        let lease = self.begin(&ticket);
        self.mic
            .acquired(&lease, true, &self.facts(1), &self.clock.now())
            .unwrap();
        lease
    }
    fn release(&mut self) -> ReleaseTicket {
        let ticket = self.mic.release_ticket().unwrap();
        self.mic
            .release_ack(&FakeReleaseAck {
                ticket: ticket.clone(),
                acquisition_settled: true,
                input_stopped: true,
            })
            .unwrap();
        ticket
    }
    fn cleanup(&mut self) -> CleanupTicket {
        let ticket = self.mic.cleanup_ticket().unwrap();
        self.mic
            .cleanup_ack(&FakeCleanupAck {
                ticket: ticket.clone(),
                frames_cleared: true,
                queues_cleared: true,
            })
            .unwrap();
        ticket
    }
    fn snapshot(&self) -> String {
        let current = self.mic.current.as_ref().map(|held| {
            (
                held.lease.id,
                held.lease.request.id,
                held.lease.request.identity,
                held.lease.request.generation,
            )
        });
        let pending = self.mic.pending.as_ref().map(|p| {
            (
                p.ticket.id,
                p.ticket.identity,
                p.ticket.generation,
                p.ticket.owner,
            )
        });
        format!(
            "{:?}/{:?}/{}/{}/{}/{:?}/{:?}/{:?}",
            self.mic.status(),
            self.mic.generation,
            self.mic.request_sequence,
            self.mic.lease_sequence,
            self.mic.closed,
            current,
            pending,
            self.clock
                .now()
                .checked_duration_since(&self.mic.last_time)
                .unwrap()
        )
    }
}

#[test]
fn starts_off_and_request_never_implicitly_acquires() {
    for owner in OWNERS {
        let mut f = Fixture::new();
        assert_eq!(f.mic.status().owner, MicrophoneOwner::Off);
        assert_eq!(f.mic.status().listening, ListeningStatus::NotListening);
        let ticket = f.request(owner, 1);
        assert!(f.mic.current.is_none());
        assert_eq!(f.mic.status().owner, MicrophoneOwner::Off);
        let lease = f.begin(&ticket);
        assert_eq!(f.mic.status().owner, owner);
        assert_eq!(f.mic.status().listening, ListeningStatus::Unconfirmed);
        assert_eq!(f.mic.accepts(&lease, &f.clock.now()), Err(Error::Stale));
        f.mic
            .acquired(&lease, true, &f.facts(1), &f.clock.now())
            .unwrap();
        assert_eq!(f.mic.status().listening, ListeningStatus::Listening);
        assert_eq!(f.mic.accepts(&lease, &f.clock.now()), Ok(true));
    }
}

#[test]
fn every_owner_transfer_requires_revocation_release_cleanup_and_explicit_begin() {
    for old_owner in OWNERS {
        for next_owner in OWNERS {
            let mut f = Fixture::new();
            let old = f.acquire(old_owner);
            let generation = f.mic.generation;
            let next = f.request(next_owner, 2);
            assert_ne!(f.mic.generation, generation);
            assert_eq!(f.mic.accepts(&old, &f.clock.now()), Err(Error::Stale));
            assert_eq!(f.mic.status().owner, old_owner);
            assert_eq!(
                f.mic.status().release,
                MicrophoneReleaseStatus::StoppingUnconfirmed
            );
            assert!(f.mic.status().cancel_requested && f.mic.status().stop_input_requested);
            assert_eq!(
                f.mic.begin(&next, &f.facts(2), &f.clock.now()).err(),
                Some(Error::Busy)
            );
            f.release();
            assert_eq!(
                f.mic.status().release,
                MicrophoneReleaseStatus::Acknowledged
            );
            assert!(f.mic.status().cleanup_requested);
            assert_eq!(f.mic.status().owner, old_owner);
            assert_eq!(
                f.mic.begin(&next, &f.facts(2), &f.clock.now()).err(),
                Some(Error::Busy)
            );
            f.cleanup();
            assert!(f.mic.current.is_none());
            assert_eq!(f.mic.status().owner, MicrophoneOwner::Off);
            assert!(f.mic.pending.is_some()); // cleanup cannot start any input
            let new = f.begin(&next);
            f.mic
                .acquired(&new, true, &f.facts(2), &f.clock.now())
                .unwrap();
            assert_eq!(f.mic.accepts(&new, &f.clock.now()), Ok(true));
            assert_eq!(f.mic.accepts(&old, &f.clock.now()), Err(Error::Stale));
            assert_eq!(f.mic.status().owner, next_owner);
        }
    }
}

#[test]
fn extra_pending_request_is_rejected_without_replacing_the_first() {
    let mut f = Fixture::new();
    f.acquire(MicrophoneOwner::WakeDetection);
    let expected = f.request(MicrophoneOwner::ManualForeground, 2);
    let before = f.snapshot();
    assert_eq!(
        f.mic
            .request(
                MicrophoneOwner::ConversationTurn,
                MAX_CAPTURED_TURN,
                &f.facts(3),
                &f.clock.now()
            )
            .err(),
        Some(Error::Busy)
    );
    assert_eq!(f.snapshot(), before);
    assert!(f.mic.pending.as_ref().unwrap().ticket.matches(&expected));
}

#[test]
fn cancel_before_begin_invalidates_cloned_requests_without_acquisition() {
    let mut f = Fixture::new();
    let old = f.request(MicrophoneOwner::ManualForeground, 1);
    let clone = old.clone();
    f.mic.cancel_pending(&old).unwrap();
    assert_eq!(
        f.mic.begin(&clone, &f.facts(1), &f.clock.now()).err(),
        Some(Error::Stale)
    );
    assert_eq!(f.mic.cancel_pending(&clone), Err(Error::Stale));
    let new = f.request(MicrophoneOwner::ConversationTurn, 2);
    let before = f.snapshot();
    assert_eq!(
        f.mic.begin(&old, &f.facts(1), &f.clock.now()).err(),
        Some(Error::Stale)
    );
    assert_eq!(f.mic.cancel_pending(&old), Err(Error::Stale));
    assert_eq!(f.snapshot(), before);
    f.begin(&new);
}

#[test]
fn cancel_queued_transfer_prevents_late_ack_from_starting_next_owner() {
    let mut f = Fixture::new();
    f.acquire(MicrophoneOwner::ManualForeground);
    let canceled = f.request(MicrophoneOwner::WakeDetection, 2);
    f.mic.cancel_pending(&canceled).unwrap();
    f.release();
    f.cleanup();
    f.mic.tick(&f.clock.now()).unwrap();
    assert_eq!(f.mic.status().owner, MicrophoneOwner::Off);
    assert!(f.mic.pending.is_none());
    assert_eq!(
        f.mic.begin(&canceled, &f.facts(2), &f.clock.now()).err(),
        Some(Error::Stale)
    );
}

#[test]
fn cancel_inflight_acquisition_blocks_late_ready_until_native_drain_and_cleanup() {
    let mut f = Fixture::new();
    let request = f.request(MicrophoneOwner::ManualForeground, 1);
    let lease = f.begin(&request);
    f.mic.cancel_pending(&request).unwrap();
    let before = f.snapshot();
    assert_eq!(
        f.mic.acquired(&lease, true, &f.facts(1), &f.clock.now()),
        Err(Error::Stale)
    );
    assert_eq!(f.snapshot(), before);
    assert_eq!(
        f.mic.status().release,
        MicrophoneReleaseStatus::StoppingUnconfirmed
    );
    let ticket = f.mic.release_ticket().unwrap();
    assert_eq!(
        f.mic.release_ack(&FakeReleaseAck {
            ticket,
            acquisition_settled: false,
            input_stopped: true
        }),
        Err(Error::ReleaseUnconfirmed)
    );
    assert!(f.mic.current.is_some());
    f.release();
    f.cleanup();
    assert_eq!(
        f.mic.acquired(&lease, true, &f.facts(1), &f.clock.now()),
        Err(Error::Stale)
    );
    assert_eq!(f.mic.status().owner, MicrophoneOwner::Off);
}

#[test]
fn transfer_while_acquiring_revokes_the_old_start_callback() {
    let mut f = Fixture::new();
    let first = f.request(MicrophoneOwner::ManualForeground, 1);
    let old = f.begin(&first);
    let next = f.request(MicrophoneOwner::ConversationTurn, 2);
    assert_eq!(
        f.mic.acquired(&old, true, &f.facts(1), &f.clock.now()),
        Err(Error::Stale)
    );
    f.release();
    f.cleanup();
    let new = f.begin(&next);
    f.mic
        .acquired(&new, true, &f.facts(2), &f.clock.now())
        .unwrap();
    assert_eq!(
        f.mic.acquired(&old, true, &f.facts(1), &f.clock.now()),
        Err(Error::Stale)
    );
    assert_eq!(f.mic.status().owner, MicrophoneOwner::ConversationTurn);
}

#[test]
fn repeated_stop_in_every_phase_preserves_release_and_cleanup_tickets() {
    for phase in 0..6 {
        let mut f = Fixture::new();
        if phase >= 1 {
            let ticket = f.request(MicrophoneOwner::ManualForeground, 1);
            if phase >= 2 {
                let lease = f.begin(&ticket);
                if phase >= 3 {
                    f.mic
                        .acquired(&lease, true, &f.facts(1), &f.clock.now())
                        .unwrap();
                }
                if phase >= 4 {
                    f.mic.stop().unwrap();
                }
                if phase == 5 {
                    f.release();
                }
            }
        }
        f.mic.stop().unwrap();
        let release = f.mic.release_ticket();
        let cleanup = f.mic.cleanup_ticket();
        let before = f.snapshot();
        for _ in 0..3 {
            f.mic.stop().unwrap();
            assert_eq!(f.snapshot(), before);
        }
        assert!(f.mic.pending.is_none());
        if let Some(ticket) = release {
            assert!(f.mic.release_ticket().unwrap().matches(&ticket));
            f.release();
        }
        if let Some(ticket) = cleanup {
            assert!(f.mic.cleanup_ticket().unwrap().matches(&ticket));
        }
        if f.mic.cleanup_ticket().is_some() {
            f.cleanup();
        }
        assert_eq!(f.mic.status().owner, MicrophoneOwner::Off);
    }
}

#[test]
fn missing_ack_warns_at_two_seconds_without_claiming_release() {
    let mut f = Fixture::new();
    f.acquire(MicrophoneOwner::WakeDetection);
    f.mic.stop().unwrap();
    f.mic.tick(&f.clock.now()).unwrap(); // first valid timing sample after Stop
    f.clock
        .advance(RELEASE_WARNING_AFTER - Duration::from_nanos(1))
        .unwrap();
    f.mic.tick(&f.clock.now()).unwrap();
    assert!(!f.mic.status().missing_ack_warning);
    f.clock.advance(Duration::from_nanos(1)).unwrap();
    f.mic.tick(&f.clock.now()).unwrap();
    assert!(f.mic.status().missing_ack_warning);
    for _ in 0..3 {
        f.mic.stop().unwrap();
        f.clock.advance(Duration::from_secs(100)).unwrap();
        f.mic.tick(&f.clock.now()).unwrap();
        assert_eq!(f.mic.status().owner, MicrophoneOwner::WakeDetection);
        assert_eq!(
            f.mic.status().release,
            MicrophoneReleaseStatus::StoppingUnconfirmed
        );
        assert_eq!(f.mic.status().listening, ListeningStatus::Unconfirmed);
        assert!(f.mic.status().missing_ack_warning);
    }
    f.release();
    assert!(!f.mic.status().missing_ack_warning);
    assert_eq!(
        f.mic.status().release,
        MicrophoneReleaseStatus::Acknowledged
    );
    f.cleanup();
}

#[test]
fn lease_expiry_at_equality_never_proves_release_or_allows_transfer() {
    for extra in [Duration::ZERO, Duration::from_nanos(1)] {
        let mut f = Fixture::new();
        let lease = f.acquire(MicrophoneOwner::ConversationTurn);
        f.clock.advance(MAX_CAPTURED_TURN + extra).unwrap();
        assert_eq!(f.mic.accepts(&lease, &f.clock.now()), Ok(false));
        f.mic.tick(&f.clock.now()).unwrap();
        assert_eq!(
            f.mic.status().release,
            MicrophoneReleaseStatus::StoppingUnconfirmed
        );
        assert_eq!(f.mic.status().owner, MicrophoneOwner::ConversationTurn);
        let next = f.request(MicrophoneOwner::WakeDetection, 2);
        assert_eq!(
            f.mic.begin(&next, &f.facts(2), &f.clock.now()).err(),
            Some(Error::Busy)
        );
        f.release();
        f.cleanup();
        assert_eq!(f.mic.status().owner, MicrophoneOwner::Off);
    }
}

#[test]
fn late_acquisition_cannot_extend_original_deadlines() {
    let mut f = Fixture::new();
    let ticket = f.request(MicrophoneOwner::ManualForeground, 1);
    let lease = f.begin(&ticket);
    f.clock.advance(MAX_CAPTURED_TURN).unwrap();
    assert_eq!(
        f.mic.acquired(&lease, true, &f.facts(1), &f.clock.now()),
        Err(Error::Unauthorized)
    );
    assert_eq!(
        f.mic.status().release,
        MicrophoneReleaseStatus::StoppingUnconfirmed
    );
    assert_eq!(f.mic.accepts(&lease, &f.clock.now()), Err(Error::Stale));
}

#[test]
fn pending_grant_expiry_cannot_be_renewed_by_fresh_facts_or_cleanup() {
    for duration in [Duration::from_secs(1), MAX_CAPTURED_TURN] {
        let mut f = Fixture::new();
        f.acquire(MicrophoneOwner::WakeDetection);
        let mut facts = f.facts(2);
        if duration == Duration::from_secs(1) {
            facts.deadline = Deadline::after(f.clock.now(), duration).unwrap();
        }
        let next = f
            .mic
            .request(
                MicrophoneOwner::ConversationTurn,
                MAX_CAPTURED_TURN,
                &facts,
                &f.clock.now(),
            )
            .unwrap();
        f.release();
        f.cleanup();
        f.clock.advance(duration).unwrap();
        assert_eq!(
            f.mic.begin(&next, &f.facts(2), &f.clock.now()).err(),
            Some(Error::Expired)
        );
        assert_eq!(f.mic.status().owner, MicrophoneOwner::Off);
        f.mic.tick(&f.clock.now()).unwrap();
        assert!(f.mic.pending.is_none());
        assert_eq!(
            f.mic.begin(&next, &f.facts(2), &f.clock.now()).err(),
            Some(Error::Stale)
        );
    }
}

#[test]
fn forged_foreign_wrong_owner_and_duplicate_release_acks_are_inert() {
    let mut f = Fixture::new();
    f.acquire(MicrophoneOwner::ManualForeground);
    f.mic.stop().unwrap();
    let original = f.mic.release_ticket().unwrap();
    let mut variants = Vec::new();
    macro_rules! forged {
        ($change:expr) => {{
            let mut ticket = original.clone();
            ($change)(&mut ticket);
            variants.push(ticket);
        }};
    }
    forged!(|t: &mut ReleaseTicket| t.nonce = Arc::new(()));
    forged!(|t: &mut ReleaseTicket| t.lease.nonce = Arc::new(()));
    forged!(|t: &mut ReleaseTicket| t.lease.id = LeaseId(NonZeroU64::new(99).unwrap()));
    forged!(|t: &mut ReleaseTicket| t.lease.request.id = RequestId(NonZeroU64::new(99).unwrap()));
    forged!(|t: &mut ReleaseTicket| t.lease.request.nonce = Arc::new(()));
    forged!(|t: &mut ReleaseTicket| t.lease.request.native_instance = Arc::new(()));
    forged!(|t: &mut ReleaseTicket| t.lease.request.owner = MicrophoneOwner::WakeDetection);
    forged!(|t: &mut ReleaseTicket| t.lease.request.generation =
        Generation::from_native_sequence(99).unwrap());
    forged!(
        |t: &mut ReleaseTicket| t.lease.request.identity.app_run_id =
            AppRunId::from_native_sequence(99).unwrap()
    );
    forged!(
        |t: &mut ReleaseTicket| t.lease.request.identity.interaction_id =
            InteractionId::from_native_sequence(99).unwrap()
    );
    forged!(
        |t: &mut ReleaseTicket| t.lease.request.identity.session_id =
            SessionId::from_native_sequence(99).unwrap()
    );
    forged!(|t: &mut ReleaseTicket| t.lease.request.identity.turn_id =
        TurnId::from_native_sequence(99).unwrap());
    forged!(|t: &mut ReleaseTicket| t.lease.request.identity.job_id =
        JobId::from_native_sequence(99).unwrap());
    forged!(
        |t: &mut ReleaseTicket| t.lease.request.identity.generation =
            Generation::from_native_sequence(99).unwrap()
    );
    forged!(
        |t: &mut ReleaseTicket| t.lease.request.identity.configuration_revision =
            ConfigurationRevision::from_native_sequence(99).unwrap()
    );
    forged!(
        |t: &mut ReleaseTicket| t.lease.request.identity.consent_binding_id =
            ConsentBindingId::from_native_sequence(99).unwrap()
    );
    let mut other = Fixture::new();
    other.acquire(MicrophoneOwner::ManualForeground);
    other.mic.stop().unwrap();
    variants.push(other.mic.release_ticket().unwrap());
    let before = f.snapshot();
    for ticket in variants {
        assert_eq!(
            f.mic.release_ack(&FakeReleaseAck {
                ticket,
                acquisition_settled: true,
                input_stopped: true
            }),
            Err(Error::Stale)
        );
        assert_eq!(f.snapshot(), before);
    }
    f.release();
    let before = f.snapshot();
    assert_eq!(
        f.mic.release_ack(&FakeReleaseAck {
            ticket: original,
            acquisition_settled: true,
            input_stopped: true
        }),
        Err(Error::Stale)
    );
    assert_eq!(f.snapshot(), before);
}

#[test]
fn cleanup_cannot_be_early_forged_duplicated_or_release_a_new_lease() {
    let mut f = Fixture::new();
    f.acquire(MicrophoneOwner::ManualForeground);
    f.mic.stop().unwrap();
    let release = f.mic.release_ticket().unwrap();
    let early = CleanupTicket {
        release: release.clone(),
        nonce: Arc::new(()),
    };
    let before = f.snapshot();
    assert_eq!(
        f.mic.cleanup_ack(&FakeCleanupAck {
            ticket: early,
            frames_cleared: true,
            queues_cleared: true
        }),
        Err(Error::Stale)
    );
    assert_eq!(f.snapshot(), before);
    f.release();
    let original = f.mic.cleanup_ticket().unwrap();
    for corrupt_release in [false, true] {
        let mut forged = original.clone();
        if corrupt_release {
            forged.release.nonce = Arc::new(());
        } else {
            forged.nonce = Arc::new(());
        }
        let before = f.snapshot();
        assert_eq!(
            f.mic.cleanup_ack(&FakeCleanupAck {
                ticket: forged,
                frames_cleared: true,
                queues_cleared: true
            }),
            Err(Error::Stale)
        );
        assert_eq!(f.snapshot(), before);
    }
    f.cleanup();
    f.acquire(MicrophoneOwner::ConversationTurn);
    let before = f.snapshot();
    assert_eq!(
        f.mic.cleanup_ack(&FakeCleanupAck {
            ticket: original,
            frames_cleared: true,
            queues_cleared: true
        }),
        Err(Error::Stale)
    );
    assert_eq!(
        f.mic.release_ack(&FakeReleaseAck {
            ticket: release,
            acquisition_settled: true,
            input_stopped: true
        }),
        Err(Error::Stale)
    );
    assert_eq!(f.snapshot(), before);
}

#[test]
fn release_failures_keep_uncertain_ownership_and_cancel_queued_transfer() {
    for (settled, stopped) in [(false, true), (true, false), (false, false)] {
        let mut f = Fixture::new();
        f.acquire(MicrophoneOwner::WakeDetection);
        let next = f.request(MicrophoneOwner::ConversationTurn, 2);
        let ticket = f.mic.release_ticket().unwrap();
        assert_eq!(
            f.mic.release_ack(&FakeReleaseAck {
                ticket,
                acquisition_settled: settled,
                input_stopped: stopped
            }),
            Err(Error::ReleaseUnconfirmed)
        );
        assert!(f.mic.status().faulted);
        assert_eq!(f.mic.status().owner, MicrophoneOwner::WakeDetection);
        assert_eq!(
            f.mic.status().release,
            MicrophoneReleaseStatus::StoppingUnconfirmed
        );
        assert!(f.mic.pending.is_none());
        f.mic.stop().unwrap();
        f.release();
        f.cleanup();
        assert_eq!(
            f.mic.begin(&next, &f.facts(2), &f.clock.now()).err(),
            Some(Error::Stale)
        );
        assert_eq!(f.mic.status().owner, MicrophoneOwner::Off);
    }
}

#[test]
fn frame_or_queue_cleanup_failure_blocks_transfer_until_matching_success() {
    for (frames, queues) in [(false, true), (true, false), (false, false)] {
        let mut f = Fixture::new();
        f.acquire(MicrophoneOwner::ManualForeground);
        let next = f.request(MicrophoneOwner::WakeDetection, 2);
        f.release();
        let ticket = f.mic.cleanup_ticket().unwrap();
        assert_eq!(
            f.mic.cleanup_ack(&FakeCleanupAck {
                ticket,
                frames_cleared: frames,
                queues_cleared: queues
            }),
            Err(Error::CleanupUnconfirmed)
        );
        assert!(f.mic.current.is_some());
        assert!(f.mic.status().faulted && f.mic.status().cleanup_requested);
        assert_eq!(
            f.mic.status().release,
            MicrophoneReleaseStatus::Acknowledged
        );
        assert!(f.mic.pending.is_none());
        f.mic.stop().unwrap();
        f.cleanup();
        assert_eq!(
            f.mic.begin(&next, &f.facts(2), &f.clock.now()).err(),
            Some(Error::Stale)
        );
        assert_eq!(f.mic.status().owner, MicrophoneOwner::Off);
    }
}

#[test]
fn generation_exhaustion_revokes_even_when_the_counter_cannot_advance() {
    for owned in [false, true] {
        let mut f = Fixture::new();
        f.mic.generation = Generation::from_native_sequence(u64::MAX).unwrap();
        let request = f.request(MicrophoneOwner::ConversationTurn, 1);
        let lease = f.begin(&request);
        if owned {
            f.mic
                .acquired(&lease, true, &f.facts(1), &f.clock.now())
                .unwrap();
        }
        assert_eq!(f.mic.stop(), Err(ContractError::Overflow.into()));
        assert!(f.mic.closed);
        assert_eq!(
            f.mic.status().release,
            MicrophoneReleaseStatus::StoppingUnconfirmed
        );
        assert_eq!(
            f.mic.acquired(&lease, true, &f.facts(1), &f.clock.now()),
            Err(Error::Stale)
        );
        assert_eq!(f.mic.accepts(&lease, &f.clock.now()), Err(Error::Stale));
        f.mic.stop().unwrap(); // retrying Stop never replaces the original ticket
        f.release();
        f.cleanup();
        assert_eq!(f.mic.status().owner, MicrophoneOwner::Off);
        assert_eq!(
            f.mic
                .request(
                    MicrophoneOwner::ManualForeground,
                    MAX_CAPTURED_TURN,
                    &f.facts(2),
                    &f.clock.now()
                )
                .err(),
            Some(Error::Closed)
        );
    }
}

#[test]
fn overflowing_transfer_does_not_allocate_next_request_but_stops_old_owner() {
    let mut f = Fixture::new();
    f.acquire(MicrophoneOwner::ManualForeground);
    f.mic.generation = Generation::from_native_sequence(u64::MAX).unwrap();
    let sequence = f.mic.request_sequence;
    assert_eq!(
        f.mic
            .request(
                MicrophoneOwner::WakeDetection,
                MAX_CAPTURED_TURN,
                &f.facts(2),
                &f.clock.now()
            )
            .err(),
        Some(ContractError::Overflow.into())
    );
    assert!(f.mic.closed && f.mic.pending.is_none());
    assert_eq!(f.mic.request_sequence, sequence);
    assert_eq!(
        f.mic.status().release,
        MicrophoneReleaseStatus::StoppingUnconfirmed
    );
    f.release();
    f.cleanup();
}

#[test]
fn request_and_lease_counter_overflow_are_atomic_and_stop_remains_available() {
    let mut f = Fixture::new();
    f.acquire(MicrophoneOwner::WakeDetection);
    f.mic.request_sequence = u64::MAX;
    let before = f.snapshot();
    assert_eq!(
        f.mic
            .request(
                MicrophoneOwner::ManualForeground,
                MAX_CAPTURED_TURN,
                &f.facts(2),
                &f.clock.now()
            )
            .err(),
        Some(ContractError::Overflow.into())
    );
    assert_eq!(f.snapshot(), before);
    f.mic.stop().unwrap();
    f.release();
    f.cleanup();
    let mut f = Fixture::new();
    let ticket = f.request(MicrophoneOwner::ManualForeground, 1);
    f.mic.lease_sequence = u64::MAX;
    let before = f.snapshot();
    assert_eq!(
        f.mic.begin(&ticket, &f.facts(1), &f.clock.now()).err(),
        Some(ContractError::Overflow.into())
    );
    assert_eq!(f.snapshot(), before);
    f.mic.stop().unwrap();
    assert!(f.mic.pending.is_none() && f.mic.current.is_none());
}

#[test]
fn clock_errors_reject_requests_and_callbacks_without_acquiring_or_releasing() {
    for foreign in [false, true] {
        let mut f = Fixture::new();
        let old_time = f.clock.now();
        f.clock.advance(Duration::from_secs(1)).unwrap();
        f.mic.tick(&f.clock.now()).unwrap();
        let bad_time = if foreign {
            ManualClock::new().now()
        } else {
            old_time
        };
        let error = if foreign {
            ContractError::ClockDomainMismatch
        } else {
            ContractError::TimeReversed
        };
        let before = f.snapshot();
        assert_eq!(
            f.mic
                .request(
                    MicrophoneOwner::ManualForeground,
                    MAX_CAPTURED_TURN,
                    &f.facts(1),
                    &bad_time
                )
                .err(),
            Some(error.into())
        );
        assert_eq!(f.snapshot(), before);
        let ticket = f.request(MicrophoneOwner::ManualForeground, 1);
        let before = f.snapshot();
        assert_eq!(
            f.mic.begin(&ticket, &f.facts(1), &bad_time).err(),
            Some(error.into())
        );
        assert_eq!(f.snapshot(), before);
        let lease = f.begin(&ticket);
        let before = f.snapshot();
        assert_eq!(
            f.mic.acquired(&lease, true, &f.facts(1), &bad_time),
            Err(error.into())
        );
        assert_eq!(f.snapshot(), before);
        f.mic
            .acquired(&lease, true, &f.facts(1), &f.clock.now())
            .unwrap();
        assert_eq!(f.mic.accepts(&lease, &bad_time), Err(error.into()));
        assert_eq!(f.mic.tick(&bad_time), Err(error.into()));
        assert_eq!(
            f.mic.status().release,
            MicrophoneReleaseStatus::StoppingUnconfirmed
        );
        // ACK and cleanup are clock-independent; they cannot begin another owner.
        f.mic.stop().unwrap();
        f.release();
        f.cleanup();
        assert_eq!(f.mic.status().owner, MicrophoneOwner::Off);
    }
}

#[test]
fn foreign_deadline_and_deadline_overflow_do_not_consume_native_sequences() {
    let mut f = Fixture::new();
    let mut facts = f.facts(1);
    facts.deadline = Deadline::after(ManualClock::new().now(), Duration::from_secs(1)).unwrap();
    let before = f.snapshot();
    assert_eq!(
        f.mic
            .request(
                MicrophoneOwner::ManualForeground,
                MAX_CAPTURED_TURN,
                &facts,
                &f.clock.now()
            )
            .err(),
        Some(ContractError::ClockDomainMismatch.into())
    );
    assert_eq!(f.snapshot(), before);
    let mut f = Fixture::new();
    let mut facts = f.facts(1);
    f.clock
        .advance(Duration::MAX - Duration::from_secs(10))
        .unwrap();
    facts.deadline = Deadline::after(f.clock.now(), Duration::from_secs(1)).unwrap();
    let before = f.snapshot();
    assert_eq!(
        f.mic
            .request(
                MicrophoneOwner::ManualForeground,
                MAX_CAPTURED_TURN,
                &facts,
                &f.clock.now()
            )
            .err(),
        Some(ContractError::Overflow.into())
    );
    assert_eq!(f.snapshot(), before);
}

#[test]
fn validation_never_follows_from_ownership_or_request_metadata() {
    for owner in OWNERS {
        for condition in 0..7 {
            let mut f = Fixture::new();
            let mut facts = f.facts(1);
            match condition {
                0 => facts.activation_allowed = false,
                1 => facts.privacy_clear = false,
                2 => facts.output_idle = false,
                3 => {
                    facts.foreground = false;
                    facts.background_allowed = false;
                }
                4 => facts.identity.app_run_id = AppRunId::from_native_sequence(2).unwrap(),
                5 => facts.deadline = Deadline::after(f.clock.now(), Duration::ZERO).unwrap(),
                _ => {
                    facts.explicit_manual_start = false;
                    if owner != MicrophoneOwner::ManualForeground {
                        facts.activation_allowed = false;
                    }
                }
            }
            let before = f.snapshot();
            assert_eq!(
                f.mic
                    .request(owner, MAX_CAPTURED_TURN, &facts, &f.clock.now())
                    .err(),
                Some(Error::Unauthorized)
            );
            assert_eq!(f.snapshot(), before);
        }
    }
    let mut f = Fixture::new();
    assert_eq!(
        f.mic
            .request(
                MicrophoneOwner::Off,
                MAX_CAPTURED_TURN,
                &f.facts(1),
                &f.clock.now()
            )
            .err(),
        Some(Error::InvalidOwner)
    );
    let ticket = f.request(MicrophoneOwner::ManualForeground, 1);
    let mut denied = f.facts(1);
    denied.activation_allowed = false;
    assert_eq!(
        f.mic.begin(&ticket, &denied, &f.clock.now()).err(),
        Some(Error::Unauthorized)
    );
    let lease = f.begin(&ticket);
    assert_eq!(
        f.mic.acquired(&lease, true, &denied, &f.clock.now()),
        Err(Error::Unauthorized)
    );
    assert_eq!(
        f.mic.status().release,
        MicrophoneReleaseStatus::StoppingUnconfirmed
    );
}

#[test]
fn manual_foreground_loss_cancels_pending_and_inflight_work_without_resume() {
    for phase in 0..3 {
        let mut f = Fixture::new();
        let ticket = f.request(MicrophoneOwner::ManualForeground, 1);
        let lease = if phase > 0 {
            Some(f.begin(&ticket))
        } else {
            None
        };
        if phase == 2 {
            f.mic
                .acquired(lease.as_ref().unwrap(), true, &f.facts(1), &f.clock.now())
                .unwrap();
        }
        f.mic.foreground_lost().unwrap();
        assert!(f.mic.pending.is_none());
        if let Some(lease) = lease {
            assert_eq!(
                f.mic.acquired(&lease, true, &f.facts(1), &f.clock.now()),
                Err(Error::Stale)
            );
            assert_eq!(
                f.mic.status().release,
                MicrophoneReleaseStatus::StoppingUnconfirmed
            );
            f.release();
            f.cleanup();
        }
        f.mic.tick(&f.clock.now()).unwrap();
        assert_eq!(f.mic.status().owner, MicrophoneOwner::Off);
        assert_eq!(
            f.mic.begin(&ticket, &f.facts(1), &f.clock.now()).err(),
            Some(Error::Stale)
        );
    }
    let mut f = Fixture::new();
    f.acquire(MicrophoneOwner::WakeDetection);
    let manual = f.request(MicrophoneOwner::ManualForeground, 2);
    f.mic.foreground_lost().unwrap();
    f.release();
    f.cleanup();
    assert_eq!(
        f.mic.begin(&manual, &f.facts(2), &f.clock.now()).err(),
        Some(Error::Stale)
    );
}

#[test]
fn manual_start_and_owner_duration_bounds_are_not_weakened() {
    for owner in OWNERS {
        let maximum = if owner == MicrophoneOwner::WakeDetection {
            MAX_WAKE_ARMING
        } else {
            MAX_CAPTURED_TURN
        };
        let mut f = Fixture::new();
        f.mic
            .request(owner, maximum, &f.facts(1), &f.clock.now())
            .unwrap();
        for duration in [Duration::ZERO, maximum + Duration::from_nanos(1)] {
            let mut f = Fixture::new();
            let before = f.snapshot();
            assert_eq!(
                f.mic
                    .request(owner, duration, &f.facts(1), &f.clock.now())
                    .err(),
                Some(ContractError::LimitExceeded.into())
            );
            assert_eq!(f.snapshot(), before);
        }
    }
    let mut f = Fixture::new();
    let mut facts = f.facts(1);
    facts.foreground = false;
    facts.background_allowed = true;
    assert_eq!(
        f.mic
            .request(
                MicrophoneOwner::ManualForeground,
                MAX_CAPTURED_TURN,
                &facts,
                &f.clock.now()
            )
            .err(),
        Some(Error::Unauthorized)
    );
    f.mic
        .request(
            MicrophoneOwner::WakeDetection,
            MAX_WAKE_ARMING,
            &facts,
            &f.clock.now(),
        )
        .unwrap();
}

#[test]
fn failed_or_duplicate_acquisition_never_claims_live_input() {
    let mut f = Fixture::new();
    let ticket = f.request(MicrophoneOwner::ManualForeground, 1);
    let lease = f.begin(&ticket);
    assert_eq!(
        f.mic.acquired(&lease, false, &f.facts(1), &f.clock.now()),
        Err(Error::Unauthorized)
    );
    assert_eq!(f.mic.status().listening, ListeningStatus::Unconfirmed);
    f.release();
    f.cleanup();
    let lease = f.acquire(MicrophoneOwner::ManualForeground);
    let before = f.snapshot();
    assert_eq!(
        f.mic.acquired(&lease, true, &f.facts(1), &f.clock.now()),
        Err(Error::Stale)
    );
    assert_eq!(f.snapshot(), before);
}

#[test]
fn shortened_validation_deadlines_survive_begin_and_acquisition_callbacks() {
    for shorten_at_begin in [false, true] {
        let mut f = Fixture::new();
        let ticket = f.request(MicrophoneOwner::ManualForeground, 1);
        let mut short = f.facts(1);
        short.deadline = Deadline::after(f.clock.now(), Duration::from_secs(1)).unwrap();
        let lease = if shorten_at_begin {
            f.mic.begin(&ticket, &short, &f.clock.now()).unwrap()
        } else {
            f.begin(&ticket)
        };
        let facts = if shorten_at_begin { f.facts(1) } else { short };
        f.mic
            .acquired(&lease, true, &facts, &f.clock.now())
            .unwrap();
        f.clock.advance(Duration::from_secs(1)).unwrap();
        assert_eq!(f.mic.accepts(&lease, &f.clock.now()), Ok(false));
        f.mic.tick(&f.clock.now()).unwrap();
        assert_eq!(
            f.mic.status().release,
            MicrophoneReleaseStatus::StoppingUnconfirmed
        );
        assert_eq!(f.mic.status().owner, MicrophoneOwner::ManualForeground);
    }
}

#[test]
fn stop_timing_never_uses_a_stale_pre_stop_clock_sample() {
    let mut f = Fixture::new();
    f.acquire(MicrophoneOwner::WakeDetection);
    f.clock.advance(Duration::from_secs(10)).unwrap();
    f.mic.stop().unwrap(); // no clock dependency and no guessed Stop timestamp
    f.mic.tick(&f.clock.now()).unwrap();
    assert!(!f.mic.status().missing_ack_warning);
    f.clock.advance(RELEASE_WARNING_AFTER).unwrap();
    f.mic.tick(&f.clock.now()).unwrap();
    assert!(f.mic.status().missing_ack_warning);
    assert_eq!(
        f.mic.status().release,
        MicrophoneReleaseStatus::StoppingUnconfirmed
    );
}

#[test]
fn forged_request_and_acquisition_tickets_never_take_ownership() {
    let mut f = Fixture::new();
    let ticket = f.request(MicrophoneOwner::ManualForeground, 1);
    for change in 0..3 {
        let mut forged = ticket.clone();
        match change {
            0 => forged.nonce = Arc::new(()),
            1 => forged.native_instance = Arc::new(()),
            _ => forged.generation = Generation::from_native_sequence(99).unwrap(),
        }
        let before = f.snapshot();
        assert_eq!(
            f.mic.begin(&forged, &f.facts(1), &f.clock.now()).err(),
            Some(Error::Stale)
        );
        assert_eq!(f.mic.cancel_pending(&forged), Err(Error::Stale));
        assert_eq!(f.snapshot(), before);
    }
    let lease = f.begin(&ticket);
    for change in 0..4 {
        let mut forged = lease.clone();
        match change {
            0 => forged.nonce = Arc::new(()),
            1 => forged.request.native_instance = Arc::new(()),
            2 => forged.request.identity.job_id = JobId::from_native_sequence(99).unwrap(),
            _ => forged.request.owner = MicrophoneOwner::WakeDetection,
        }
        let before = f.snapshot();
        assert_eq!(
            f.mic.acquired(&forged, true, &f.facts(1), &f.clock.now()),
            Err(Error::Stale)
        );
        assert_eq!(f.snapshot(), before);
    }
}

#[test]
fn overflowing_transfer_preserves_monotonic_time_of_the_stop_request() {
    let mut f = Fixture::new();
    f.acquire(MicrophoneOwner::ManualForeground);
    let earlier = f.clock.now();
    f.clock.advance(Duration::from_secs(1)).unwrap();
    f.mic.generation = Generation::from_native_sequence(u64::MAX).unwrap();
    assert_eq!(
        f.mic
            .request(
                MicrophoneOwner::WakeDetection,
                MAX_CAPTURED_TURN,
                &f.facts(2),
                &f.clock.now()
            )
            .err(),
        Some(ContractError::Overflow.into())
    );
    assert_eq!(
        f.clock
            .now()
            .checked_duration_since(&f.mic.last_time)
            .unwrap(),
        Duration::ZERO
    );
    let ticket = f.mic.release_ticket().unwrap();
    assert_eq!(
        f.mic.tick(&earlier),
        Err(ContractError::TimeReversed.into())
    );
    assert!(f.mic.release_ticket().unwrap().matches(&ticket));
    assert_eq!(
        f.mic.status().release,
        MicrophoneReleaseStatus::StoppingUnconfirmed
    );
    f.release();
    f.cleanup();
}
