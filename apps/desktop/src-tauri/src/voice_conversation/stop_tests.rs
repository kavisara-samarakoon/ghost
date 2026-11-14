//! Synthetic cancellation acceptance evidence only, never runtime fanout proof.
use super::*;
use crate::voice_conversation::clock::{ManualClock, MonotonicClock};
use std::sync::Mutex;

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

fn stage(participant: Participant) -> Stage {
    match participant {
        Participant::Session => Stage::Session(SessionState::Active),
        Participant::Wake => Stage::Wake(WakeState::Armed),
        Participant::Stt => Stage::Turn(TurnState::Transcribing),
        Participant::Conversation => Stage::Turn(TurnState::RequestingConversation),
        Participant::Planning => Stage::Planning,
        Participant::Tts | Participant::Speech => Stage::Speech(SpeechState::Speaking),
        Participant::PendingSpeech => Stage::Speech(SpeechState::Preparing),
        Participant::ConsentPreview => Stage::Consent,
        Participant::History => Stage::History,
    }
}

struct Fixture {
    clock: ManualClock,
    supervisor: StopSupervisor,
}
impl Fixture {
    fn new() -> Self {
        let clock = ManualClock::new();
        let supervisor = StopSupervisor::synthetic(identity(1), clock.now());
        Self { clock, supervisor }
    }
    fn enroll(&mut self, participant: Participant) -> Ticket {
        self.supervisor
            .synthetic_work(
                participant,
                identity(participant.index() as u64 + 1),
                stage(participant),
            )
            .unwrap()
    }
    fn input(&mut self, owner: MicrophoneOwner, acquiring: bool) -> InputLease {
        self.supervisor
            .synthetic_input(identity(99), owner, acquiring)
            .unwrap()
    }
    fn all_work(&mut self) -> Vec<Ticket> {
        Participant::ALL.iter().map(|p| self.enroll(*p)).collect()
    }
    fn release(&mut self) {
        let ticket = self.supervisor.release_ticket().unwrap();
        self.supervisor.release_ack(&ticket, true, true).unwrap();
    }
    fn clear_frames(&mut self) {
        let ticket = self.supervisor.frame_cleanup_ticket().unwrap();
        self.supervisor
            .frame_cleanup_ack(&ticket, true, true)
            .unwrap();
    }
    fn finish_local_cleanup(&mut self) {
        if self.supervisor.release_ticket().is_some() {
            self.release();
        }
        if self.supervisor.frame_cleanup_ticket().is_some() {
            self.clear_frames();
        }
        for kind in [Cleanup::Output, Cleanup::Workers] {
            if let Some(ticket) = self.supervisor.cleanup_ticket(kind) {
                self.supervisor.cleanup_ack(&ticket, true).unwrap();
            }
        }
        assert!(self.supervisor.status().local_cleanup_complete);
    }
    fn assert_inert(&mut self, tickets: &[Ticket]) {
        assert!(self.supervisor.status().revoked);
        assert_eq!(self.supervisor.ephemeral_references, 0);
        for ticket in tickets {
            for domain in [
                Eligibility::NewWork,
                Eligibility::Consent,
                Eligibility::Egress,
            ] {
                assert!(!ticket.eligible(domain));
            }
            assert_eq!(self.supervisor.callback(ticket), Err(Error::Closed));
            assert_eq!(
                self.supervisor.work[ticket.participant.index()]
                    .as_ref()
                    .unwrap()
                    .state,
                WorkState::Canceled
            );
        }
    }
}

#[test]
fn stop_from_every_session_lifecycle_phase() {
    for phase in [
        SessionState::Inactive,
        SessionState::Starting,
        SessionState::Active,
        SessionState::Paused,
        SessionState::Stopping,
        SessionState::Stopped,
        SessionState::Expired,
        SessionState::Faulted,
    ] {
        let mut f = Fixture::new();
        let ticket = f
            .supervisor
            .synthetic_work(Participant::Session, identity(1), Stage::Session(phase))
            .unwrap();
        f.supervisor.stop(StopReason::Explicit);
        f.assert_inert(&[ticket]);
        f.finish_local_cleanup();
    }
}

#[test]
fn stop_from_every_turn_lifecycle_phase() {
    for phase in [
        TurnState::Idle,
        TurnState::Listening,
        TurnState::Finalizing,
        TurnState::Transcribing,
        TurnState::ValidatingTranscript,
        TurnState::RequestingConversation,
        TurnState::ValidatingReply,
        TurnState::ReplyReady,
        TurnState::Speaking,
        TurnState::FollowUpWindow,
        TurnState::AwaitingConsent,
        TurnState::NeedsActionReview,
        TurnState::Canceled,
        TurnState::Failed,
    ] {
        let mut f = Fixture::new();
        let ticket = f
            .supervisor
            .synthetic_work(Participant::Session, identity(1), Stage::Turn(phase))
            .unwrap();
        if matches!(phase, TurnState::Listening | TurnState::Finalizing) {
            f.input(MicrophoneOwner::ConversationTurn, false);
        }
        f.supervisor.stop(StopReason::Explicit);
        f.assert_inert(&[ticket]);
        f.finish_local_cleanup();
    }
}

#[test]
fn stop_from_every_wake_lifecycle_phase() {
    for phase in [
        WakeState::Disabled,
        WakeState::Preparing,
        WakeState::Armed,
        WakeState::Suppressed,
        WakeState::Cooldown,
        WakeState::Suspended,
        WakeState::Faulted,
    ] {
        let mut f = Fixture::new();
        let ticket = f
            .supervisor
            .synthetic_work(Participant::Wake, identity(1), Stage::Wake(phase))
            .unwrap();
        f.supervisor.stop(StopReason::Explicit);
        f.assert_inert(&[ticket]);
        f.finish_local_cleanup();
    }
}

#[test]
fn stop_from_every_speech_lifecycle_phase_clears_both_slots() {
    for phase in [
        SpeechState::Idle,
        SpeechState::Preparing,
        SpeechState::Fetching,
        SpeechState::Speaking,
    ] {
        let mut f = Fixture::new();
        let active = f
            .supervisor
            .synthetic_work(Participant::Speech, identity(1), Stage::Speech(phase))
            .unwrap();
        let pending = f.enroll(Participant::PendingSpeech);
        f.supervisor.stop(StopReason::Explicit);
        f.assert_inert(&[active, pending]);
        assert!(f.supervisor.cleanup_ticket(Cleanup::Output).is_some());
        assert!(!f.supervisor.status().local_cleanup_complete);
        f.finish_local_cleanup();
    }
}

#[test]
fn all_synthetic_roles_stalled_with_external_locks_stop_independently() {
    let mut f = Fixture::new();
    let tickets = f.all_work();
    f.input(MicrophoneOwner::ConversationTurn, false);
    // Four fake transport locks plus renderer, storage and audit. Deliberately
    // held across Stop: the supervisor receives none of these locks or ports.
    let ports: [Mutex<()>; 7] = std::array::from_fn(|_| Mutex::new(()));
    let guards: Vec<_> = ports.iter().map(|p| p.lock().unwrap()).collect();
    assert!(ports.iter().all(|p| p.try_lock().is_err()));
    let status = f.supervisor.stop(StopReason::Explicit);
    assert_eq!(status.release, MicrophoneReleaseStatus::StoppingUnconfirmed);
    f.assert_inert(&tickets);
    assert!(ports.iter().all(|p| p.try_lock().is_err()));
    f.finish_local_cleanup();
    drop(guards);
}

#[test]
fn consent_and_pending_receipt_revoke_before_any_cleanup_ack() {
    let mut f = Fixture::new();
    let tickets = f.all_work();
    for ticket in &tickets {
        assert!(ticket.eligible(Eligibility::Consent));
        assert!(ticket.eligible(Eligibility::Egress));
    }
    f.input(MicrophoneOwner::WakeDetection, false);
    let status = f.supervisor.stop(StopReason::PrivacyRevoked);
    assert!(!status.local_cleanup_complete);
    assert!(f.supervisor.cleanup_result.iter().all(Option::is_none));
    assert!(f.supervisor.release_ticket().is_some());
    f.assert_inert(&tickets);
}

#[test]
fn stop_during_planning_never_restores_planning_authority() {
    let mut f = Fixture::new();
    let planning = f.enroll(Participant::Planning);
    f.supervisor.synthetic_dispatch(&planning, false).unwrap();
    f.supervisor.stop(StopReason::Explicit);
    f.supervisor
        .cancellation_result(&planning, Cancellation::Failed)
        .unwrap();
    f.finish_local_cleanup();
    f.assert_inert(&[planning]);
    assert_eq!(f.supervisor.reservations_consumed, 1);
}

#[test]
fn stop_during_tts_revokes_fetch_and_output_independently() {
    let mut f = Fixture::new();
    let tts = f.enroll(Participant::Tts);
    f.supervisor.synthetic_dispatch(&tts, true).unwrap();
    f.supervisor.stop(StopReason::Explicit);
    assert!(f.supervisor.cleanup_ticket(Cleanup::Output).is_some());
    f.assert_inert(&[tts]);
    f.finish_local_cleanup();
}

#[test]
fn repeated_stop_preserves_generation_and_exact_release_request() {
    let mut f = Fixture::new();
    let tickets = f.all_work();
    f.input(MicrophoneOwner::ManualForeground, false);
    let first = f.supervisor.stop(StopReason::Explicit);
    let release = f.supervisor.release_ticket().unwrap();
    let generation = f.supervisor.generation;
    let sequence = f.supervisor.sequence;
    for _ in 0..100 {
        assert_eq!(f.supervisor.stop(StopReason::Explicit), first);
        assert_eq!(f.supervisor.generation, generation);
        assert_eq!(f.supervisor.sequence, sequence);
        assert!(f.supervisor.release_ticket().unwrap().0.matches(&release.0));
    }
    f.assert_inert(&tickets);
}

#[test]
fn repeated_stop_during_cleanup_and_after_completion_does_not_requeue() {
    let mut f = Fixture::new();
    f.all_work();
    f.input(MicrophoneOwner::ConversationTurn, false);
    f.supervisor.stop(StopReason::Explicit);
    f.release();
    let frames = f.supervisor.frame_cleanup_ticket().unwrap();
    f.supervisor.stop(StopReason::Quit);
    assert!(f.supervisor.release_ticket().is_none());
    assert!(f
        .supervisor
        .frame_cleanup_ticket()
        .unwrap()
        .0
        .matches(&frames.0));
    f.finish_local_cleanup();
    let completed = f.supervisor.status();
    assert_eq!(f.supervisor.stop(StopReason::Lock), completed);
    assert_eq!(f.supervisor.reason, Some(StopReason::Explicit));
    assert!(f.supervisor.cleanup_ticket(Cleanup::Workers).is_none());
    assert!(f.supervisor.cleanup_ticket(Cleanup::Output).is_none());
}

#[test]
fn stop_requests_cancel_and_input_stop_for_all_owners_and_pending_acquisition() {
    for owner in [
        MicrophoneOwner::ManualForeground,
        MicrophoneOwner::WakeDetection,
        MicrophoneOwner::ConversationTurn,
    ] {
        for acquiring in [false, true] {
            let mut f = Fixture::new();
            let lease = f.input(owner, acquiring);
            let status = f.supervisor.stop(StopReason::Explicit);
            assert!(status.cancel_input_requested && status.stop_input_requested);
            assert_eq!(status.owner, owner);
            assert_eq!(status.listening, ListeningStatus::Unconfirmed);
            assert!(!lease.ticket.eligible(Eligibility::NewWork));
            assert!(f.supervisor.frame_cleanup_ticket().is_none());
            f.finish_local_cleanup();
        }
    }
}

#[test]
fn missing_release_ack_never_becomes_release_proof_at_any_timeout() {
    let mut f = Fixture::new();
    f.input(MicrophoneOwner::WakeDetection, false);
    f.supervisor.stop(StopReason::Explicit);
    f.supervisor.tick(&f.clock.now()).unwrap();
    f.clock.advance(Duration::from_millis(1999)).unwrap();
    f.supervisor.tick(&f.clock.now()).unwrap();
    assert!(!f.supervisor.status().missing_ack_warning);
    f.clock.advance(Duration::from_millis(1)).unwrap();
    f.supervisor.tick(&f.clock.now()).unwrap();
    assert!(f.supervisor.status().missing_ack_warning);
    f.clock.advance(Duration::from_secs(86400)).unwrap();
    f.supervisor.tick(&f.clock.now()).unwrap();
    let status = f.supervisor.status();
    assert_eq!(status.release, MicrophoneReleaseStatus::StoppingUnconfirmed);
    assert_eq!(status.listening, ListeningStatus::Unconfirmed);
    assert_eq!(status.owner, MicrophoneOwner::WakeDetection);
    assert!(!status.local_cleanup_complete);
    assert!(f.supervisor.frame_cleanup_ticket().is_none());
}

#[test]
fn delayed_exact_ack_then_separate_frame_queue_evidence_completes_release() {
    let mut f = Fixture::new();
    f.input(MicrophoneOwner::ConversationTurn, false);
    f.supervisor.stop(StopReason::Explicit);
    f.supervisor.tick(&f.clock.now()).unwrap();
    f.clock.advance(Duration::from_secs(10)).unwrap();
    f.supervisor.tick(&f.clock.now()).unwrap();
    f.release();
    let status = f.supervisor.status();
    assert_eq!(status.release, MicrophoneReleaseStatus::Acknowledged);
    assert_eq!(status.owner, MicrophoneOwner::ConversationTurn);
    assert_eq!(status.listening, ListeningStatus::NotListening);
    assert!(status.frame_cleanup_requested);
    assert!(!status.local_cleanup_complete);
    assert!(!status.missing_ack_warning);
    f.clear_frames();
    assert_eq!(f.supervisor.status().owner, MicrophoneOwner::Off);
    f.finish_local_cleanup();
}

#[test]
fn incomplete_release_evidence_keeps_stopping_unconfirmed() {
    for (settled, stopped) in [(false, false), (false, true), (true, false)] {
        let mut f = Fixture::new();
        f.input(MicrophoneOwner::ConversationTurn, true);
        f.supervisor.stop(StopReason::Explicit);
        let ticket = f.supervisor.release_ticket().unwrap();
        assert_eq!(
            f.supervisor.release_ack(&ticket, settled, stopped),
            Err(Error::Unconfirmed)
        );
        assert_eq!(
            f.supervisor.status().release,
            MicrophoneReleaseStatus::StoppingUnconfirmed
        );
        assert!(f.supervisor.status().cleanup_failed);
        assert!(f.supervisor.frame_cleanup_ticket().is_none());
        f.finish_local_cleanup();
    }
}

#[test]
fn frame_and_queue_cleanup_are_both_required_without_reacquisition() {
    for (frames, queues) in [(false, false), (false, true), (true, false)] {
        let mut f = Fixture::new();
        f.input(MicrophoneOwner::WakeDetection, false);
        f.supervisor.stop(StopReason::Explicit);
        f.release();
        let ticket = f.supervisor.frame_cleanup_ticket().unwrap();
        assert_eq!(
            f.supervisor.frame_cleanup_ack(&ticket, frames, queues),
            Err(Error::Unconfirmed)
        );
        assert_eq!(f.supervisor.status().owner, MicrophoneOwner::WakeDetection);
        assert!(f.supervisor.status().frame_cleanup_requested);
        assert!(!f.supervisor.status().local_cleanup_complete);
        assert!(f.supervisor.status().cleanup_failed);
        assert!(matches!(
            f.supervisor
                .synthetic_input(identity(100), MicrophoneOwner::ConversationTurn, false),
            Err(Error::Closed)
        ));
        f.finish_local_cleanup();
    }
}

#[test]
fn failed_output_or_worker_cleanup_does_not_claim_local_completion() {
    for kind in [Cleanup::Output, Cleanup::Workers] {
        let mut f = Fixture::new();
        f.enroll(Participant::Speech);
        f.supervisor.stop(StopReason::Quit);
        let ticket = f.supervisor.cleanup_ticket(kind).unwrap();
        assert_eq!(
            f.supervisor.cleanup_ack(&ticket, false),
            Err(Error::Unconfirmed)
        );
        assert!(f.supervisor.status().cleanup_failed);
        assert!(!f.supervisor.status().local_cleanup_complete);
        f.finish_local_cleanup();
        assert!(f.supervisor.status().revoked);
    }
}

#[test]
fn audit_failure_and_missing_provider_results_cannot_block_local_cleanup() {
    let mut f = Fixture::new();
    let tickets = f.all_work();
    f.input(MicrophoneOwner::ConversationTurn, false);
    f.supervisor.stop(StopReason::Explicit);
    let audit = f.supervisor.cleanup_ticket(Cleanup::Audit).unwrap();
    assert_eq!(
        f.supervisor.cleanup_ack(&audit, false),
        Err(Error::Unconfirmed)
    );
    assert!(f.supervisor.status().audit_failed);
    f.finish_local_cleanup();
    assert!(f
        .supervisor
        .work
        .iter()
        .flatten()
        .all(|w| w.cancellation.is_none()));
    f.assert_inert(&tickets);
}

#[test]
fn missing_audit_ack_cannot_block_local_cleanup() {
    let mut f = Fixture::new();
    f.supervisor.stop(StopReason::Explicit);
    f.finish_local_cleanup();
    assert!(f.supervisor.cleanup_ticket(Cleanup::Audit).is_some());
    assert!(!f.supervisor.status().audit_failed);
}

#[test]
fn late_success_for_each_role_cannot_reactivate_or_open_followup() {
    for participant in Participant::ALL {
        let mut f = Fixture::new();
        let ticket = f.enroll(participant);
        f.supervisor.stop(StopReason::Explicit);
        f.finish_local_cleanup();
        let before = f.supervisor.status();
        for _ in 0..3 {
            assert_eq!(f.supervisor.callback(&ticket), Err(Error::Closed));
            assert_eq!(f.supervisor.status(), before);
        }
        assert!(f.supervisor.input.is_none());
        assert_eq!(f.supervisor.ephemeral_references, 0);
    }
}

#[test]
fn wake_event_after_stop_cannot_rearm_or_issue_new_work() {
    let mut f = Fixture::new();
    let wake = f.enroll(Participant::Wake);
    f.supervisor.stop(StopReason::Explicit);
    f.finish_local_cleanup();
    assert_eq!(f.supervisor.callback(&wake), Err(Error::Closed));
    assert!(matches!(
        f.supervisor.synthetic_work(
            Participant::Session,
            identity(2),
            stage(Participant::Session)
        ),
        Err(Error::Closed)
    ));
    assert!(matches!(
        f.supervisor
            .synthetic_input(identity(3), MicrophoneOwner::WakeDetection, false),
        Err(Error::Closed)
    ));
}

#[test]
fn reserved_attempted_and_uncertain_egress_all_become_ineligible() {
    for egress in [
        EgressState::Reserved,
        EgressState::Attempted,
        EgressState::Uncertain,
    ] {
        let mut f = Fixture::new();
        let ticket = f.enroll(Participant::Conversation);
        if egress != EgressState::Reserved {
            f.supervisor
                .synthetic_dispatch(&ticket, egress == EgressState::Uncertain)
                .unwrap();
        }
        assert_eq!(f.supervisor.reservations_consumed, 1);
        f.supervisor.stop(StopReason::Explicit);
        assert_eq!(
            f.supervisor.work_mut(&ticket).unwrap().egress,
            Some(EgressState::Canceled)
        );
        assert_eq!(
            f.supervisor.synthetic_dispatch(&ticket, false),
            Err(Error::Closed)
        );
        assert_eq!(f.supervisor.reservations_consumed, 1);
        f.assert_inert(&[ticket]);
    }
}

#[test]
fn failed_or_uncertain_provider_cancel_never_restores_consent_or_retry() {
    for outcome in [
        Cancellation::Confirmed,
        Cancellation::Failed,
        Cancellation::Uncertain,
    ] {
        for participant in [
            Participant::Stt,
            Participant::Conversation,
            Participant::Planning,
            Participant::Tts,
        ] {
            let mut f = Fixture::new();
            let ticket = f.enroll(participant);
            f.supervisor.synthetic_dispatch(&ticket, true).unwrap();
            f.supervisor.stop(StopReason::Explicit);
            f.supervisor.cancellation_result(&ticket, outcome).unwrap();
            assert_eq!(
                f.supervisor.cancellation_result(&ticket, outcome),
                Err(Error::Stale)
            );
            f.finish_local_cleanup();
            f.assert_inert(std::slice::from_ref(&ticket));
            assert_eq!(f.supervisor.reservations_consumed, 1);
            for fallback in Participant::ALL {
                assert!(matches!(
                    f.supervisor
                        .synthetic_work(fallback, identity(50), stage(fallback)),
                    Err(Error::Closed)
                ));
            }
            assert_eq!(f.supervisor.work.iter().flatten().count(), 1);
        }
    }
}

#[test]
fn all_privacy_barriers_and_quit_revoke_without_automatic_return() {
    for reason in [
        StopReason::Explicit,
        StopReason::Lock,
        StopReason::Sleep,
        StopReason::Logout,
        StopReason::Quit,
        StopReason::PermissionRevoked,
        StopReason::DeviceLost,
        StopReason::PrivacyRevoked,
        StopReason::ConfigurationChanged,
    ] {
        let mut f = Fixture::new();
        let tickets = f.all_work();
        f.input(MicrophoneOwner::WakeDetection, false);
        f.supervisor.stop(reason);
        f.finish_local_cleanup();
        // Passive time/late callbacks represent returning from lock or sleep;
        // neither is a trusted fresh enablement path.
        f.clock.advance(Duration::from_secs(100)).unwrap();
        f.supervisor.tick(&f.clock.now()).unwrap();
        f.assert_inert(&tickets);
        assert!(matches!(
            f.supervisor
                .synthetic_input(identity(50), MicrophoneOwner::WakeDetection, false),
            Err(Error::Closed)
        ));
    }
}

#[test]
fn exact_identity_fields_are_required_before_any_callback_or_cancellation() {
    let mut f = Fixture::new();
    let ticket = f.enroll(Participant::Conversation);
    let variants: [fn(&mut ContractIdentity); 8] = [
        |i| i.app_run_id = AppRunId::from_native_sequence(2).unwrap(),
        |i| i.interaction_id = InteractionId::from_native_sequence(2).unwrap(),
        |i| i.session_id = SessionId::from_native_sequence(2).unwrap(),
        |i| i.turn_id = TurnId::from_native_sequence(99).unwrap(),
        |i| i.job_id = JobId::from_native_sequence(99).unwrap(),
        |i| i.generation = Generation::from_native_sequence(2).unwrap(),
        |i| i.configuration_revision = ConfigurationRevision::from_native_sequence(2).unwrap(),
        |i| i.consent_binding_id = ConsentBindingId::from_native_sequence(2).unwrap(),
    ];
    for mutate in variants {
        let mut stale = ticket.clone();
        mutate(&mut stale.identity);
        assert_eq!(f.supervisor.callback(&stale), Err(Error::Stale));
    }
    f.supervisor.stop(StopReason::Explicit);
    for mutate in variants {
        let mut stale = ticket.clone();
        mutate(&mut stale.identity);
        assert_eq!(
            f.supervisor
                .cancellation_result(&stale, Cancellation::Confirmed),
            Err(Error::Stale)
        );
    }
    f.supervisor
        .cancellation_result(&ticket, Cancellation::Confirmed)
        .unwrap();
}

#[test]
fn same_identity_from_foreign_supervisor_and_forged_nonce_are_rejected() {
    let mut f = Fixture::new();
    let ticket = f.enroll(Participant::Conversation);
    let mut foreign = Fixture::new();
    let foreign_ticket = foreign.enroll(Participant::Conversation);
    assert_eq!(f.supervisor.callback(&foreign_ticket), Err(Error::Stale));
    let mut changed = ticket.clone();
    changed.nonce = Arc::new(());
    assert_eq!(f.supervisor.callback(&changed), Err(Error::Stale));
    changed = ticket.clone();
    changed.sequence += 1;
    assert_eq!(f.supervisor.callback(&changed), Err(Error::Stale));
    changed = ticket.clone();
    changed.stage = Stage::Turn(TurnState::ValidatingReply);
    assert_eq!(f.supervisor.callback(&changed), Err(Error::Stale));
    changed = ticket.clone();
    changed.participant = Participant::Planning;
    assert_eq!(f.supervisor.callback(&changed), Err(Error::Stale));
    f.supervisor.callback(&ticket).unwrap();
    assert_eq!(f.supervisor.callback(&ticket), Err(Error::Stale));
}

#[test]
fn foreign_release_cleanup_and_worker_evidence_cannot_release_local_resources() {
    let mut f = Fixture::new();
    let mut foreign = Fixture::new();
    for fixture in [&mut f, &mut foreign] {
        fixture.input(MicrophoneOwner::ConversationTurn, false);
        fixture.supervisor.stop(StopReason::Quit);
    }
    let release = foreign.supervisor.release_ticket().unwrap();
    assert_eq!(
        f.supervisor.release_ack(&release, true, true),
        Err(Error::Stale)
    );
    f.release();
    foreign.release();
    let frames = foreign.supervisor.frame_cleanup_ticket().unwrap();
    assert_eq!(
        f.supervisor.frame_cleanup_ack(&frames, true, true),
        Err(Error::Stale)
    );
    for kind in [Cleanup::Workers, Cleanup::Audit] {
        let ticket = foreign.supervisor.cleanup_ticket(kind).unwrap();
        assert_eq!(f.supervisor.cleanup_ack(&ticket, true), Err(Error::Stale));
    }
    assert!(!f.supervisor.status().local_cleanup_complete);
    f.finish_local_cleanup();
}

#[test]
fn altered_release_identity_owner_or_nonce_cannot_count_as_ack() {
    let mut f = Fixture::new();
    f.input(MicrophoneOwner::WakeDetection, false);
    f.supervisor.stop(StopReason::Explicit);
    let release = f.supervisor.release_ticket().unwrap();
    let mutations: [fn(&mut InputLease); 5] = [
        |l| l.ticket.identity.generation = Generation::from_native_sequence(2).unwrap(),
        |l| l.ticket.identity.job_id = JobId::from_native_sequence(1).unwrap(),
        |l| l.owner = MicrophoneOwner::ConversationTurn,
        |l| l.release_nonce = Arc::new(()),
        |l| l.cleanup_nonce = Arc::new(()),
    ];
    for mutate in mutations {
        let mut altered = release.clone();
        mutate(&mut altered.0);
        assert_eq!(
            f.supervisor.release_ack(&altered, true, true),
            Err(Error::Stale)
        );
    }
    f.release();
    let frames = f.supervisor.frame_cleanup_ticket().unwrap();
    for mutate in mutations {
        let mut altered = frames.clone();
        mutate(&mut altered.0);
        assert_eq!(
            f.supervisor.frame_cleanup_ack(&altered, true, true),
            Err(Error::Stale)
        );
    }
    f.finish_local_cleanup();
}

#[test]
fn cleanup_ack_requires_separate_requested_stage_and_cannot_replay() {
    let mut f = Fixture::new();
    let lease = f.input(MicrophoneOwner::ConversationTurn, true);
    let premature = FrameCleanupTicket(lease.clone());
    assert_eq!(
        f.supervisor.frame_cleanup_ack(&premature, true, true),
        Err(Error::Stale)
    );
    assert_eq!(
        f.supervisor.release_ack(&ReleaseTicket(lease), true, true),
        Err(Error::Stale)
    );
    f.supervisor.stop(StopReason::Explicit);
    assert_eq!(
        f.supervisor.frame_cleanup_ack(&premature, true, true),
        Err(Error::Stale)
    );
    let release = f.supervisor.release_ticket().unwrap();
    f.release();
    assert_eq!(
        f.supervisor.release_ack(&release, true, true),
        Err(Error::Stale)
    );
    let frames = f.supervisor.frame_cleanup_ticket().unwrap();
    f.clear_frames();
    assert_eq!(
        f.supervisor.frame_cleanup_ack(&frames, true, true),
        Err(Error::Stale)
    );
    let worker = f.supervisor.cleanup_ticket(Cleanup::Workers).unwrap();
    f.supervisor.cleanup_ack(&worker, true).unwrap();
    assert_eq!(f.supervisor.cleanup_ack(&worker, true), Err(Error::Stale));
}

#[test]
fn clock_domain_error_revokes_before_cleanup_and_cannot_disable_stop() {
    let mut f = Fixture::new();
    let tickets = f.all_work();
    f.input(MicrophoneOwner::ConversationTurn, false);
    let foreign_clock = ManualClock::new();
    assert_eq!(
        f.supervisor.tick(&foreign_clock.now()),
        Err(Error::Contract(ContractError::ClockDomainMismatch))
    );
    assert_eq!(f.supervisor.reason, Some(StopReason::ClockFailure));
    f.supervisor.stop(StopReason::Explicit);
    f.assert_inert(&tickets);
    f.finish_local_cleanup();
}

#[test]
fn backwards_clock_revokes_and_ack_progress_needs_no_clock() {
    let mut f = Fixture::new();
    f.input(MicrophoneOwner::ManualForeground, false);
    let old = f.clock.now();
    f.clock.advance(Duration::from_secs(1)).unwrap();
    f.supervisor.tick(&f.clock.now()).unwrap();
    assert_eq!(
        f.supervisor.tick(&old),
        Err(Error::Contract(ContractError::TimeReversed))
    );
    assert_eq!(
        f.supervisor.status().release,
        MicrophoneReleaseStatus::StoppingUnconfirmed
    );
    f.finish_local_cleanup();
    assert!(f.supervisor.status().revoked);
}

#[test]
fn clock_error_after_stop_never_discards_exact_release_ticket() {
    let mut f = Fixture::new();
    f.input(MicrophoneOwner::WakeDetection, false);
    f.supervisor.stop(StopReason::Explicit);
    let release = f.supervisor.release_ticket().unwrap();
    assert!(f.supervisor.tick(&ManualClock::new().now()).is_err());
    assert!(f.supervisor.release_ticket().unwrap().0.matches(&release.0));
    f.finish_local_cleanup();
}

#[test]
fn exhausted_generation_cannot_obstruct_revocation_or_cleanup() {
    let mut f = Fixture::new();
    f.supervisor.generation = Generation::from_native_sequence(u64::MAX).unwrap();
    f.supervisor.identity.generation = f.supervisor.generation;
    let mut scoped = identity(1);
    scoped.generation = f.supervisor.generation;
    let ticket = f
        .supervisor
        .synthetic_work(
            Participant::Conversation,
            scoped,
            stage(Participant::Conversation),
        )
        .unwrap();
    f.supervisor
        .synthetic_input(scoped, MicrophoneOwner::ConversationTurn, true)
        .unwrap();
    let status = f.supervisor.stop(StopReason::Explicit);
    assert!(status.generation_exhausted);
    assert_eq!(status.release, MicrophoneReleaseStatus::StoppingUnconfirmed);
    assert_eq!(f.supervisor.stop(StopReason::Quit), status);
    f.assert_inert(&[ticket]);
    f.finish_local_cleanup();
}

#[test]
fn exhausted_sequence_on_new_work_fails_closed_and_stop_allocates_no_sequence() {
    let mut f = Fixture::new();
    let ticket = f.enroll(Participant::Wake);
    f.input(MicrophoneOwner::WakeDetection, false);
    f.supervisor.sequence = u64::MAX;
    assert!(matches!(
        f.supervisor.synthetic_work(
            Participant::Conversation,
            identity(5),
            stage(Participant::Conversation)
        ),
        Err(Error::Contract(ContractError::Overflow))
    ));
    assert_eq!(f.supervisor.reason, Some(StopReason::CounterExhaustion));
    f.supervisor.stop(StopReason::Explicit);
    assert_eq!(f.supervisor.sequence, u64::MAX);
    f.assert_inert(&[ticket]);
    f.finish_local_cleanup();
}

#[test]
fn exhausted_input_sequence_does_not_allocate_a_lease_or_block_stop() {
    let mut f = Fixture::new();
    f.supervisor.sequence = u64::MAX;
    assert!(matches!(
        f.supervisor
            .synthetic_input(identity(1), MicrophoneOwner::WakeDetection, true),
        Err(Error::Contract(ContractError::Overflow))
    ));
    assert!(f.supervisor.status().revoked);
    assert!(f.supervisor.input.is_none());
    f.supervisor.stop(StopReason::Explicit);
    f.finish_local_cleanup();
}

#[test]
fn simultaneous_clock_and_counter_faults_leave_stop_and_ack_available() {
    let mut f = Fixture::new();
    f.input(MicrophoneOwner::ConversationTurn, false);
    f.supervisor.sequence = u64::MAX;
    f.supervisor.generation = Generation::from_native_sequence(u64::MAX).unwrap();
    assert!(f.supervisor.tick(&ManualClock::new().now()).is_err());
    let status = f.supervisor.stop(StopReason::Explicit);
    assert!(status.generation_exhausted);
    assert_eq!(status.release, MicrophoneReleaseStatus::StoppingUnconfirmed);
    f.finish_local_cleanup();
}

#[test]
fn fixed_slots_and_consumed_reservations_never_reset_on_stop() {
    let mut f = Fixture::new();
    let tickets = f.all_work();
    assert_eq!(f.supervisor.work.iter().flatten().count(), 10);
    assert_eq!(f.supervisor.ephemeral_references, 10);
    assert_eq!(f.supervisor.reservations_consumed, 4);
    for participant in Participant::ALL {
        assert!(matches!(
            f.supervisor
                .synthetic_work(participant, identity(50), stage(participant)),
            Err(Error::Busy)
        ));
    }
    f.supervisor.stop(StopReason::Explicit);
    f.finish_local_cleanup();
    f.assert_inert(&tickets);
    assert_eq!(f.supervisor.reservations_consumed, 4);
    assert_eq!(f.supervisor.work.iter().flatten().count(), 10);
}

#[test]
fn completed_work_is_invalidated_without_provider_recancellation_or_reuse() {
    let mut f = Fixture::new();
    let ticket = f.enroll(Participant::Conversation);
    f.supervisor.synthetic_dispatch(&ticket, false).unwrap();
    f.supervisor.callback(&ticket).unwrap();
    assert!(matches!(
        f.supervisor.synthetic_work(
            Participant::Conversation,
            identity(50),
            stage(Participant::Conversation)
        ),
        Err(Error::Busy)
    ));
    f.supervisor.stop(StopReason::Explicit);
    assert!(!f.supervisor.work_mut(&ticket).unwrap().cancel_requested);
    assert_eq!(
        f.supervisor
            .cancellation_result(&ticket, Cancellation::Confirmed),
        Err(Error::Stale)
    );
    f.assert_inert(&[ticket]);
    assert_eq!(f.supervisor.reservations_consumed, 1);
}
