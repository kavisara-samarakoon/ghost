//! Synthetic inputs only. FakeReleaseEvidence is not hardware release validation.
use super::*;
use crate::voice_conversation::clock::{ManualClock, MonotonicClock};

fn fu01_allocate(f: &mut Fixture, grant: Deadline, now: MonotonicTime) -> Result<Session> {
    f.interaction.session(
        ConfigurationRevision::from_native_sequence(1).unwrap(),
        f.facts.binding,
        grant,
        now,
    )
}

fn fu01_allocation_snapshot(f: &Fixture) -> String {
    format!(
        "{}/{}/{}/{}/{:?}",
        f.snapshot(),
        f.interaction.sessions,
        f.interaction.turns,
        f.interaction.jobs,
        (1..=MAX_CHILD_CONVERSATIONS + 1)
            .map(|id| f
                .interaction
                .budget
                .child_turns(SessionId::from_native_sequence(id).unwrap()))
            .collect::<Vec<_>>()
    )
}

#[test]
fn fu01_allocation_before_deadlines_preserves_valid_allocation() {
    let mut f = Fixture::active();
    f.transcript();
    let sequence = f.interaction.sessions;
    let grant = Deadline::after(f.clock.now(), Duration::from_secs(1)).unwrap();
    f.clock.advance(Duration::from_nanos(999_999_999)).unwrap();
    let before = f.snapshot();
    let now = f.clock.now();
    let session = fu01_allocate(&mut f, grant, now).unwrap();
    assert_eq!(session.state, SessionState::Inactive);
    assert_eq!(f.interaction.sessions, sequence + 1);
    assert_eq!(
        session.owner.key.session,
        SessionId::from_native_sequence(sequence + 1).unwrap()
    );
    // Allocation is not admission and does not touch the existing session/ledger.
    assert_eq!(f.snapshot(), before);
    assert_eq!(
        f.interaction.budget.child_turns(session.owner.key.session),
        Err(ContractError::UnknownSession)
    );
    assert_eq!(
        f.interaction
            .budget
            .child_turns(f.session.owner.key.session),
        Ok(1)
    );
    assert_eq!(f.interaction.admitted_turns, 1);
}

#[test]
fn fu01_allocation_rejects_exact_and_past_interaction_expiry_atomically() {
    for extra in [Duration::ZERO, Duration::from_nanos(1)] {
        let mut f = Fixture::active();
        f.transcript();
        let grant = Deadline::after(
            f.clock.now(),
            MAX_CLOUD_INTERACTION + Duration::from_secs(1),
        )
        .unwrap();
        f.clock.advance(MAX_CLOUD_INTERACTION + extra).unwrap();
        let before = fu01_allocation_snapshot(&f);
        let now = f.clock.now();
        assert_eq!(
            fu01_allocate(&mut f, grant, now).err(),
            Some(Error::Unauthorized)
        );
        assert_eq!(fu01_allocation_snapshot(&f), before);
    }
}

#[test]
fn fu01_allocation_rejects_exact_and_past_grant_expiry_atomically() {
    for extra in [Duration::ZERO, Duration::from_nanos(1)] {
        let mut f = Fixture::active();
        f.transcript();
        let grant = Deadline::after(f.clock.now(), Duration::from_secs(1)).unwrap();
        f.clock.advance(Duration::from_secs(1) + extra).unwrap();
        let before = fu01_allocation_snapshot(&f);
        let now = f.clock.now();
        assert_eq!(
            fu01_allocate(&mut f, grant, now).err(),
            Some(Error::Unauthorized)
        );
        assert_eq!(fu01_allocation_snapshot(&f), before);
        // A rejected grant does not consume an ID; a later valid grant still works.
        let sequence = f.interaction.sessions;
        let grant = f.facts.deadline.clone();
        let now = f.clock.now();
        let session = fu01_allocate(&mut f, grant, now).unwrap();
        assert_eq!(
            session.owner.key.session,
            SessionId::from_native_sequence(sequence + 1).unwrap()
        );
    }
}

#[test]
fn fu01_allocation_preserves_both_clock_domain_checks() {
    for case in 0..3 {
        let mut f = Fixture::inactive();
        let foreign = ManualClock::new();
        if case == 2 {
            f.clock.advance(MAX_CLOUD_INTERACTION).unwrap();
        }
        let grant = if case == 0 {
            f.facts.deadline.clone()
        } else {
            Deadline::after(foreign.now(), MAX_CLOUD_INTERACTION).unwrap()
        };
        let now = if case == 0 {
            foreign.now()
        } else {
            f.clock.now()
        };
        let before = fu01_allocation_snapshot(&f);
        assert_eq!(
            fu01_allocate(&mut f, grant, now).err(),
            Some(Error::Contract(ContractError::ClockDomainMismatch))
        );
        assert_eq!(fu01_allocation_snapshot(&f), before);
    }
}

#[test]
fn fu01_startup_timeout_returns_invalidation_outcome_and_stays_revoked() {
    for epoch in [1, u64::MAX] {
        let mut f = Fixture::inactive();
        f.session.owner.key.epoch = Generation::from_native_sequence(epoch).unwrap();
        f.facts.deadline = Deadline::after(f.clock.now(), Duration::from_secs(2)).unwrap();
        f.control(SessionEvent::Start).unwrap();
        let op = f.session.startup.clone().unwrap();
        f.clock.advance(Duration::from_secs(1)).unwrap();
        assert_eq!(f.control(SessionEvent::Tick), Ok(Outcome::Applied));
        assert_eq!(f.session.state, SessionState::Starting);
        f.clock.advance(Duration::from_secs(1)).unwrap();
        let expected = if epoch == u64::MAX {
            Outcome::GenerationExhausted
        } else {
            Outcome::Applied
        };
        assert_eq!(f.control(SessionEvent::Tick), Ok(expected));
        assert_eq!(f.session.state, SessionState::Faulted);
        assert!(f.session.revoked);
        assert!(f.session.startup.is_none());
        assert_eq!(
            f.clock
                .now()
                .checked_duration_since(&f.session.last_time)
                .unwrap(),
            Duration::ZERO
        );
        assert_eq!(
            f.session.owner.key.epoch,
            Generation::from_native_sequence(epoch.saturating_add(1)).unwrap()
        );
        assert!(f.session.ready(&op, &f.facts, &f.clock.now()).is_err());
        assert_eq!(
            f.control(SessionEvent::Resume),
            Err(Error::InvalidTransition)
        );
        assert_eq!(
            f.control(SessionEvent::Start),
            Err(Error::InvalidTransition)
        );
        assert!(f.session.revoked);
    }
}

#[test]
fn fu01_startup_timeout_late_release_ack_changes_cleanup_only() {
    for epoch in [1, u64::MAX] {
        let mut f = Fixture::inactive();
        f.session.owner.key.epoch = Generation::from_native_sequence(epoch).unwrap();
        f.facts.deadline = Deadline::after(f.clock.now(), Duration::from_secs(1)).unwrap();
        f.control(SessionEvent::Start).unwrap();
        // Synthetic outstanding leases exercise timeout cleanup, not hardware.
        let owner = TurnOwner {
            session: f.session.owner.clone(),
            turn: TurnId::from_native_sequence(1).unwrap(),
            generation: Generation::from_native_sequence(1).unwrap(),
            binding: f.facts.binding,
        };
        for resource in [Resource::Input, Resource::Output] {
            f.session.cleanup.hold(
                owner.clone(),
                JobId::from_native_sequence(1).unwrap(),
                resource,
            );
        }
        let input = FakeReleaseEvidence(f.ticket(Resource::Input));
        let output = FakeReleaseEvidence(f.ticket(Resource::Output));
        assert_eq!(
            f.session.cleanup.acknowledge(&input),
            Err(Error::InvalidAcknowledgement)
        );
        f.clock.advance(Duration::from_secs(1)).unwrap();
        let expected = if epoch == u64::MAX {
            Outcome::GenerationExhausted
        } else {
            Outcome::Applied
        };
        assert_eq!(f.control(SessionEvent::Tick), Ok(expected));
        assert!(!f.session.cleanup.clear());
        let key = f.session.owner.key;
        f.session.cleanup.acknowledge(&input).unwrap();
        assert!(!f.session.cleanup.clear());
        f.session.cleanup.acknowledge(&output).unwrap();
        assert!(f.session.cleanup.clear());
        assert_eq!(
            f.session.cleanup.acknowledge(&input),
            Err(Error::InvalidAcknowledgement)
        );
        assert_eq!(f.session.state, SessionState::Faulted);
        assert_eq!(f.session.owner.key, key);
        assert!(f.session.revoked);
        assert!(f.session.startup.is_none());
        assert_eq!(
            f.control(SessionEvent::CleanupComplete),
            Err(Error::InvalidTransition)
        );
    }
}

#[test]
fn fu01_canceled_startup_cannot_reactivate_on_late_timeout() {
    let mut f = Fixture::inactive();
    f.facts.deadline = Deadline::after(f.clock.now(), Duration::from_secs(1)).unwrap();
    f.control(SessionEvent::Start).unwrap();
    let op = f.session.startup.clone().unwrap();
    f.control(SessionEvent::Stop).unwrap();
    f.clock.advance(Duration::from_secs(1)).unwrap();
    assert_eq!(f.control(SessionEvent::Tick), Ok(Outcome::Applied));
    assert_eq!(f.session.state, SessionState::Stopping);
    assert!(f.session.revoked);
    assert!(f.session.startup.is_none());
    assert!(f.session.ready(&op, &f.facts, &f.clock.now()).is_err());
}

#[test]
fn fu01_startup_timeout_preserves_monotonic_time_validation() {
    let mut f = Fixture::inactive();
    f.facts.deadline = Deadline::after(f.clock.now(), Duration::from_secs(2)).unwrap();
    f.control(SessionEvent::Start).unwrap();
    let earlier = f.clock.now();
    f.clock.advance(Duration::from_secs(1)).unwrap();
    f.control(SessionEvent::Tick).unwrap();
    let before = f.snapshot();
    assert_eq!(
        f.session.tick(&earlier),
        Err(Error::Contract(ContractError::TimeReversed))
    );
    assert_eq!(
        f.session.tick(&ManualClock::new().now()),
        Err(Error::Contract(ContractError::ClockDomainMismatch))
    );
    assert_eq!(f.snapshot(), before);
    f.clock.advance(Duration::from_secs(1)).unwrap();
    assert_eq!(f.control(SessionEvent::Tick), Ok(Outcome::Applied));
    assert_eq!(f.session.state, SessionState::Faulted);
}

struct Fixture {
    clock: ManualClock,
    interaction: Interaction,
    session: Session,
    facts: GuardFacts,
}
impl Fixture {
    fn inactive() -> Self {
        let clock = ManualClock::new();
        let binding = ConsentBindingId::from_native_sequence(1).unwrap();
        let deadline = Deadline::after(clock.now(), MAX_CLOUD_INTERACTION).unwrap();
        let mut interaction = Interaction::new(
            AppRunId::from_native_sequence(1).unwrap(),
            InteractionId::from_native_sequence(1).unwrap(),
            clock.now(),
        )
        .unwrap();
        let session = interaction
            .session(
                ConfigurationRevision::from_native_sequence(1).unwrap(),
                binding,
                deadline.clone(),
                clock.now(),
            )
            .unwrap();
        Self {
            clock,
            interaction,
            session,
            facts: GuardFacts {
                gate: Gate::Allowed,
                binding,
                deadline,
                privacy_clear: true,
                provenance_allowed: true,
                history_allowed: true,
                budget_allowed: true,
            },
        }
    }
    fn active() -> Self {
        let mut f = Self::inactive();
        f.control(SessionEvent::Start).unwrap();
        let op = f.session.startup.clone().unwrap();
        f.session.ready(&op, &f.facts, &f.clock.now()).unwrap();
        f
    }
    fn control(&mut self, event: SessionEvent) -> Result<Outcome> {
        let owner = self.session.owner.clone();
        self.session.control(
            &mut self.interaction,
            &owner,
            event,
            &self.facts,
            &self.clock.now(),
        )
    }
    fn prepare(&mut self) -> Result<Outcome> {
        let owner = self.session.owner.clone();
        self.session
            .prepare_turn(&mut self.interaction, &owner, &self.clock.now())
    }
    fn command(&mut self, event: TurnEvent) -> Result<Outcome> {
        self.command_route(event, TranscriptionRoute::LocalOnly)
    }
    fn command_route(&mut self, event: TurnEvent, route: TranscriptionRoute) -> Result<Outcome> {
        let owner = self.session.turn.as_ref().unwrap().owner.clone();
        self.session.command(
            &mut self.interaction,
            &owner,
            event,
            &self.facts,
            &self.clock.now(),
            MAX_CAPTURED_TURN,
            route,
        )
    }
    fn callback(&mut self, event: TurnEvent, bytes: u64, follow_up: bool) -> Result<Outcome> {
        let op = self
            .session
            .turn
            .as_ref()
            .unwrap()
            .operation
            .clone()
            .unwrap();
        self.session
            .callback(&op, event, &self.clock.now(), bytes, follow_up)
    }
    fn ticket(&self, resource: Resource) -> ReleaseTicket {
        self.session.cleanup.resources[resource.index()]
            .as_ref()
            .unwrap()
            .ticket
            .clone()
    }
    fn ack(&mut self, resource: Resource) {
        let evidence = FakeReleaseEvidence(self.ticket(resource));
        self.session.cleanup.acknowledge(&evidence).unwrap();
    }
    fn begin(&mut self) {
        self.prepare().unwrap();
        self.command(TurnEvent::BeginCapture).unwrap();
        if !self.session.turn.as_ref().unwrap().speech_onset {
            self.callback(TurnEvent::SpeechOnset, 0, false).unwrap();
        }
    }
    fn audio(&mut self) {
        self.begin();
        self.callback(TurnEvent::FinishCapture, 0, false).unwrap();
        self.ack(Resource::Input);
        self.callback(TurnEvent::AudioReady, 1, false).unwrap();
    }
    fn transcript(&mut self) {
        self.audio();
        self.command(TurnEvent::StartStt).unwrap();
        self.callback(TurnEvent::SttResult, 10, false).unwrap();
    }
    fn reply(&mut self) {
        self.transcript();
        self.command(TurnEvent::RequestConversation).unwrap();
        self.callback(TurnEvent::ConversationResult, 10, false)
            .unwrap();
        self.command(TurnEvent::ReplyAccepted).unwrap();
    }
    fn speaking(&mut self) {
        self.reply();
        self.command(TurnEvent::RequestSpeech).unwrap();
        self.callback(TurnEvent::SpeechStarted, 0, false).unwrap();
    }
    fn state(&self) -> TurnState {
        self.session.turn.as_ref().unwrap().state
    }
    fn snapshot(&self) -> String {
        let t = self.session.turn.as_ref();
        format!(
            "{:?}/{:?}/{}/{:?}/{:?}/{:?}/{:?}/{}/{}/{}/{}/{:?}",
            self.session.state,
            self.session.owner.key,
            self.session.revoked,
            t.map(|t| (
                &t.owner,
                t.state,
                t.retired,
                t.audio_retained,
                t.transcript_bytes,
                t.reply_bytes
            )),
            t.and_then(|t| t.operation.as_ref())
                .map(|op| (op.role, op.job, op.stage)),
            self.session
                .idle_deadline
                .remaining(&self.clock.now())
                .unwrap(),
            self.session
                .cleanup
                .resources
                .each_ref()
                .map(|r| r.as_ref().map(|r| (r.ticket.lease, r.release_requested))),
            self.interaction.admitted_turns,
            self.interaction.jobs,
            self.interaction
                .budget
                .provider_requests(ConversationProviderRole::Stt),
            self.interaction
                .budget
                .provider_requests(ConversationProviderRole::Conversation),
            self.interaction.budget.cloud_audio()
        )
    }
}

const SESSIONS: [SessionState; 8] = [
    SessionState::Inactive,
    SessionState::Starting,
    SessionState::Active,
    SessionState::Paused,
    SessionState::Stopping,
    SessionState::Stopped,
    SessionState::Expired,
    SessionState::Faulted,
];
const SESSION_EVENTS: [SessionEvent; 9] = [
    SessionEvent::Start,
    SessionEvent::Ready,
    SessionEvent::Pause,
    SessionEvent::Resume,
    SessionEvent::End,
    SessionEvent::Stop,
    SessionEvent::Fault,
    SessionEvent::Tick,
    SessionEvent::CleanupComplete,
];
const TURNS: [TurnState; 14] = [
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
];
const TURN_EVENTS: [TurnEvent; 20] = [
    TurnEvent::BeginCapture,
    TurnEvent::SpeechOnset,
    TurnEvent::FinishCapture,
    TurnEvent::AudioReady,
    TurnEvent::StartStt,
    TurnEvent::SttResult,
    TurnEvent::RequestConversation,
    TurnEvent::ConversationResult,
    TurnEvent::ReplyAccepted,
    TurnEvent::ActionReview,
    TurnEvent::RequestSpeech,
    TurnEvent::SpeechStarted,
    TurnEvent::SpeechCompleted,
    TurnEvent::TextComplete,
    TurnEvent::Cancel,
    TurnEvent::Fail,
    TurnEvent::RejectValidation,
    TurnEvent::Recover,
    TurnEvent::ReviewDone,
    TurnEvent::Tick,
];

#[test]
fn complete_session_matrix_including_all_invalid_pairs() {
    use SessionEvent as E;
    use SessionState as S;
    let mut legal = vec![
        (S::Inactive, E::Start, S::Starting),
        (S::Starting, E::Ready, S::Active),
        (S::Active, E::Pause, S::Paused),
        (S::Paused, E::Resume, S::Active),
        (S::Starting, E::Pause, S::Stopping),
        (S::Starting, E::End, S::Stopping),
        (S::Active, E::End, S::Stopping),
        (S::Paused, E::End, S::Stopping),
        (S::Stopping, E::CleanupComplete, S::Stopped),
    ];
    for state in [S::Starting, S::Active, S::Paused] {
        legal.push((state, E::Stop, S::Stopping));
    }
    for state in [S::Starting, S::Active, S::Paused, S::Stopping] {
        legal.push((state, E::Fault, S::Faulted));
        legal.push((state, E::Tick, state));
    }
    for state in [S::Inactive, S::Stopping, S::Stopped, S::Expired, S::Faulted] {
        legal.push((state, E::Stop, state));
    }
    for state in SESSIONS {
        for event in SESSION_EVENTS {
            let expected = legal
                .iter()
                .find(|(s, e, _)| *s == state && *e == event)
                .map(|(_, _, s)| *s)
                .ok_or(Error::InvalidTransition);
            assert_eq!(session_next(state, event), expected, "{state:?}/{event:?}");
        }
    }
}

#[test]
fn complete_turn_matrix_including_all_invalid_pairs() {
    use TurnEvent as E;
    use TurnState as S;
    let mut legal = vec![
        (S::Idle, E::BeginCapture, S::Listening),
        (S::Listening, E::SpeechOnset, S::Listening),
        (S::Listening, E::FinishCapture, S::Finalizing),
        (S::Finalizing, E::AudioReady, S::Finalizing),
        (S::Finalizing, E::StartStt, S::Transcribing),
        (S::Transcribing, E::SttResult, S::ValidatingTranscript),
        (
            S::ValidatingTranscript,
            E::RequestConversation,
            S::RequestingConversation,
        ),
        (
            S::RequestingConversation,
            E::ConversationResult,
            S::ValidatingReply,
        ),
        (S::ValidatingReply, E::ReplyAccepted, S::ReplyReady),
        (
            S::ValidatingTranscript,
            E::ActionReview,
            S::NeedsActionReview,
        ),
        (S::ValidatingReply, E::ActionReview, S::NeedsActionReview),
        (S::ReplyReady, E::RequestSpeech, S::ReplyReady),
        (S::ReplyReady, E::SpeechStarted, S::Speaking),
        (S::Speaking, E::SpeechCompleted, S::FollowUpWindow),
        (S::ReplyReady, E::TextComplete, S::Idle),
        (S::NeedsActionReview, E::ReviewDone, S::Idle),
        (S::AwaitingConsent, E::Recover, S::Idle),
    ];
    for state in TURNS.into_iter().filter(|s| !turn_terminal(*s)) {
        legal.push((state, E::Cancel, S::Canceled));
        legal.push((state, E::Tick, state));
    }
    for state in [
        S::Listening,
        S::Finalizing,
        S::Transcribing,
        S::RequestingConversation,
        S::ReplyReady,
        S::Speaking,
    ] {
        legal.push((state, E::Fail, S::Failed));
    }
    for state in [S::ValidatingTranscript, S::ValidatingReply] {
        legal.push((state, E::RejectValidation, S::Failed));
    }
    for state in TURNS {
        for event in TURN_EVENTS {
            let expected = legal
                .iter()
                .find(|(s, e, _)| *s == state && *e == event)
                .map(|(_, _, s)| *s)
                .ok_or(Error::InvalidTransition);
            assert_eq!(turn_next(state, event), expected, "{state:?}/{event:?}");
        }
    }
}

#[test]
fn startup_requires_authorized_admission_and_is_charged_once() {
    for gate in [Gate::Denied, Gate::MissingConsent] {
        let mut f = Fixture::inactive();
        f.facts.gate = gate;
        let before = f.snapshot();
        assert_eq!(f.control(SessionEvent::Start), Err(Error::Unauthorized));
        assert_eq!(f.snapshot(), before);
        assert_eq!(
            f.interaction
                .budget
                .child_turns(f.session.owner.key.session),
            Err(ContractError::UnknownSession)
        );
    }
    let mut f = Fixture::inactive();
    f.control(SessionEvent::Start).unwrap();
    assert_eq!(f.session.state, SessionState::Starting);
    let before = f.snapshot();
    assert_eq!(
        f.control(SessionEvent::Start),
        Err(Error::InvalidTransition)
    );
    assert_eq!(f.snapshot(), before);
}

#[test]
fn startup_cancellation_races_cannot_resume_or_accept_late_ready() {
    for event in [SessionEvent::Pause, SessionEvent::End, SessionEvent::Stop] {
        let mut f = Fixture::inactive();
        f.control(SessionEvent::Start).unwrap();
        let op = f.session.startup.clone().unwrap();
        f.control(event).unwrap();
        assert_eq!(f.session.state, SessionState::Stopping);
        let before = f.snapshot();
        assert!(f.session.ready(&op, &f.facts, &f.clock.now()).is_err());
        assert_eq!(f.snapshot(), before);
        assert_eq!(
            f.control(SessionEvent::Resume),
            Err(Error::InvalidTransition)
        );
        f.control(SessionEvent::CleanupComplete).unwrap();
        assert_eq!(f.session.state, SessionState::Stopped);
        assert!(f.control(SessionEvent::Start).is_err());
    }
}

#[test]
fn inactive_rejects_specific_failure_and_expiry_but_global_stop_works() {
    let mut f = Fixture::inactive();
    for event in [
        SessionEvent::Fault,
        SessionEvent::Tick,
        SessionEvent::End,
        SessionEvent::Resume,
    ] {
        let before = f.snapshot();
        assert!(f.control(event).is_err());
        assert_eq!(f.snapshot(), before);
    }
    assert_eq!(f.session.stop(), Outcome::Applied);
    assert_eq!(f.session.state, SessionState::Inactive);
}

#[test]
fn pause_resume_preserves_deadlines_and_canceled_turn_ownership() {
    let mut f = Fixture::active();
    f.begin();
    let owner = f.session.turn.as_ref().unwrap().owner.clone();
    let op = f.session.turn.as_ref().unwrap().operation.clone().unwrap();
    let child = f.session.child_deadline.clone().unwrap();
    let idle = f.session.idle_deadline.clone();
    f.control(SessionEvent::Pause).unwrap();
    assert_eq!(f.state(), TurnState::Canceled);
    assert_eq!(f.control(SessionEvent::Resume), Err(Error::CleanupPending));
    f.ack(Resource::Input);
    f.clock.advance(Duration::from_secs(10)).unwrap();
    f.control(SessionEvent::Resume).unwrap();
    assert_eq!(f.session.state, SessionState::Active);
    assert!(f.session.turn.is_none());
    assert_eq!(
        child.remaining(&f.clock.now()).unwrap(),
        f.session
            .child_deadline
            .as_ref()
            .unwrap()
            .remaining(&f.clock.now())
            .unwrap()
    );
    assert_eq!(
        idle.remaining(&f.clock.now()).unwrap(),
        f.session.idle_deadline.remaining(&f.clock.now()).unwrap()
    );
    assert!(f.session.check_turn(&owner).is_err());
    assert!(f
        .session
        .callback(&op, TurnEvent::FinishCapture, &f.clock.now(), 0, false)
        .is_err());
    f.begin();
    assert_ne!(f.session.turn.as_ref().unwrap().owner.turn, owner.turn);
    assert_eq!(f.interaction.admitted_turns, 2);
}

#[test]
fn resume_requires_explicit_valid_scope_and_does_not_capture() {
    let mut f = Fixture::active();
    f.control(SessionEvent::Pause).unwrap();
    f.facts.gate = Gate::MissingConsent;
    let before = f.snapshot();
    assert_eq!(f.control(SessionEvent::Resume), Err(Error::Unauthorized));
    assert_eq!(f.snapshot(), before);
    f.facts.gate = Gate::Allowed;
    f.control(SessionEvent::Resume).unwrap();
    assert!(f.session.turn.is_none());
    assert!(f.session.cleanup.clear());
    assert_eq!(f.interaction.admitted_turns, 0);
}

#[test]
fn paused_idle_expiry_is_not_extended_by_resume() {
    let mut f = Fixture::active();
    f.control(SessionEvent::Pause).unwrap();
    f.clock.advance(IDLE_CHILD_TIMEOUT).unwrap();
    assert_eq!(f.control(SessionEvent::Resume), Ok(Outcome::Expired));
    assert_eq!(f.session.state, SessionState::Expired);
}

fn fixture_at(state: TurnState) -> Fixture {
    let mut f = Fixture::active();
    match state {
        TurnState::Idle => {
            f.prepare().unwrap();
        }
        TurnState::Listening => {
            f.begin();
        }
        TurnState::Finalizing => {
            f.audio();
        }
        TurnState::Transcribing => {
            f.audio();
            f.command(TurnEvent::StartStt).unwrap();
        }
        TurnState::ValidatingTranscript => {
            f.transcript();
        }
        TurnState::RequestingConversation => {
            f.transcript();
            f.command(TurnEvent::RequestConversation).unwrap();
        }
        TurnState::ValidatingReply => {
            f.transcript();
            f.command(TurnEvent::RequestConversation).unwrap();
            f.callback(TurnEvent::ConversationResult, 10, false)
                .unwrap();
        }
        TurnState::ReplyReady => {
            f.reply();
        }
        TurnState::Speaking => {
            f.speaking();
        }
        TurnState::FollowUpWindow => {
            f.speaking();
            f.callback(TurnEvent::SpeechCompleted, 0, true).unwrap();
        }
        TurnState::AwaitingConsent => {
            f.transcript();
            f.facts.gate = Gate::MissingConsent;
            f.command(TurnEvent::RequestConversation).unwrap();
        }
        TurnState::NeedsActionReview => {
            f.transcript();
            f.command(TurnEvent::ActionReview).unwrap();
        }
        TurnState::Canceled => {
            f.begin();
            f.command(TurnEvent::Cancel).unwrap();
        }
        TurnState::Failed => {
            f.begin();
            f.callback(TurnEvent::Fail, 0, false).unwrap();
        }
    }
    assert_eq!(f.state(), state);
    f
}

#[test]
fn independent_stop_during_every_turn_state_clears_content_and_jobs() {
    for state in TURNS {
        let mut f = fixture_at(state);
        let admitted = f.interaction.admitted_turns;
        f.facts.gate = Gate::Denied;
        assert_eq!(f.session.stop(), Outcome::Applied);
        assert_eq!(f.session.state, SessionState::Stopping);
        assert!(f.session.revoked);
        let turn = f.session.turn.as_ref().unwrap();
        assert!(turn.retired);
        assert!(turn.operation.is_none());
        assert!(!turn.audio_retained);
        assert_eq!((turn.transcript_bytes, turn.reply_bytes), (0, 0));
        assert_eq!(f.interaction.admitted_turns, admitted);
        assert_eq!(f.session.stop(), Outcome::Applied);
    }
}

#[test]
fn cancellation_during_every_live_turn_state_is_independent_and_not_refunded() {
    for state in TURNS.into_iter().filter(|s| !turn_terminal(*s)) {
        let mut f = fixture_at(state);
        let admitted = f.interaction.admitted_turns;
        let owner = f.session.turn.as_ref().unwrap().owner.clone();
        f.facts.gate = Gate::Denied;
        let foreign = ManualClock::new();
        assert_eq!(
            f.session.command(
                &mut f.interaction,
                &owner,
                TurnEvent::Cancel,
                &f.facts,
                &foreign.now(),
                MAX_CAPTURED_TURN,
                TranscriptionRoute::Cloud
            ),
            Ok(Outcome::Applied)
        );
        assert_eq!(f.state(), TurnState::Canceled);
        assert_eq!(f.interaction.admitted_turns, admitted);
        assert!(f.session.check_turn(&owner).is_err());
    }
}

#[test]
fn terminal_turns_cannot_replay_but_explicit_new_ownership_can_be_admitted() {
    for state in [TurnState::Canceled, TurnState::Failed] {
        let mut f = fixture_at(state);
        let old = f.session.turn.as_ref().unwrap().owner.clone();
        for event in TURN_EVENTS {
            let before = f.snapshot();
            assert!(f.command(event).is_err());
            assert_eq!(f.snapshot(), before);
        }
        f.ack(Resource::Input);
        f.begin();
        assert_ne!(old.turn, f.session.turn.as_ref().unwrap().owner.turn);
        assert_eq!(f.interaction.admitted_turns, 2);
    }
}

#[test]
fn terminal_sessions_cannot_reactivate_after_cleanup() {
    for state in [
        SessionState::Stopped,
        SessionState::Expired,
        SessionState::Faulted,
    ] {
        let mut f = Fixture::active();
        f.begin();
        match state {
            SessionState::Stopped => {
                f.session.stop();
                f.ack(Resource::Input);
                f.control(SessionEvent::CleanupComplete).unwrap();
            }
            SessionState::Expired => {
                f.clock.advance(MAX_CHILD_CONVERSATION).unwrap();
                f.control(SessionEvent::Tick).unwrap();
                f.ack(Resource::Input);
            }
            _ => {
                f.control(SessionEvent::Fault).unwrap();
                f.ack(Resource::Input);
            }
        }
        assert_eq!(f.session.state, state);
        for event in SESSION_EVENTS
            .into_iter()
            .filter(|e| *e != SessionEvent::Stop)
        {
            let before = f.snapshot();
            assert!(f.control(event).is_err());
            assert_eq!(f.snapshot(), before);
        }
        f.session.stop();
        assert_eq!(f.session.state, state);
    }
}

#[test]
fn invalid_commands_are_atomic_at_every_live_stage() {
    for state in TURNS {
        let mut f = fixture_at(state);
        for event in TURN_EVENTS {
            if turn_next(state, event).is_err() {
                let before = f.snapshot();
                assert!(f.command(event).is_err(), "{state:?}/{event:?}");
                assert_eq!(f.snapshot(), before);
            }
        }
    }
}

#[test]
fn wrong_session_identity_fields_and_foreign_native_owner_are_rejected() {
    let mut f = Fixture::active();
    let owner = f.session.owner.clone();
    let mut variants = Vec::new();
    let mut o = owner.clone();
    o.key.run = AppRunId::from_native_sequence(2).unwrap();
    variants.push(o);
    let mut o = owner.clone();
    o.key.interaction = InteractionId::from_native_sequence(2).unwrap();
    variants.push(o);
    let mut o = owner.clone();
    o.key.session = SessionId::from_native_sequence(2).unwrap();
    variants.push(o);
    let mut o = owner.clone();
    o.key.epoch = o.key.epoch.checked_next().unwrap();
    variants.push(o);
    let mut o = owner.clone();
    o.key.configuration = o.key.configuration.checked_next().unwrap();
    variants.push(o);
    let mut o = owner.clone();
    o.key.binding = ConsentBindingId::from_native_sequence(2).unwrap();
    variants.push(o);
    let mut o = owner.clone();
    o.instance = Arc::new(());
    variants.push(o);
    for owner in variants {
        let before = f.snapshot();
        assert_eq!(
            f.session.control(
                &mut f.interaction,
                &owner,
                SessionEvent::Pause,
                &f.facts,
                &f.clock.now()
            ),
            Err(Error::OwnershipMismatch)
        );
        assert_eq!(f.snapshot(), before);
    }
    let mut other = Fixture::active();
    let before = f.snapshot();
    assert_eq!(
        f.session.prepare_turn(
            &mut other.interaction,
            &f.session.owner.clone(),
            &f.clock.now()
        ),
        Err(Error::OwnershipMismatch)
    );
    assert_eq!(f.snapshot(), before);
}

#[test]
fn wrong_turn_generation_binding_id_role_job_and_stage_reject_callbacks() {
    let mut f = Fixture::active();
    f.audio();
    f.command(TurnEvent::StartStt).unwrap();
    let op = f.session.turn.as_ref().unwrap().operation.clone().unwrap();
    let mut variants = Vec::new();
    let mut o = op.clone();
    o.turn.as_mut().unwrap().generation = Generation::from_native_sequence(2).unwrap();
    variants.push(o);
    let mut o = op.clone();
    o.turn.as_mut().unwrap().binding = ConsentBindingId::from_native_sequence(2).unwrap();
    variants.push(o);
    let mut o = op.clone();
    o.turn.as_mut().unwrap().turn = TurnId::from_native_sequence(2).unwrap();
    variants.push(o);
    let mut o = op.clone();
    o.role = OperationRole::Conversation;
    variants.push(o);
    let mut o = op.clone();
    o.job = JobId::from_native_sequence(999).unwrap();
    variants.push(o);
    let mut o = op.clone();
    o.stage = TurnState::RequestingConversation;
    variants.push(o);
    let mut o = op.clone();
    o.owner.instance = Arc::new(());
    variants.push(o);
    for op in variants {
        let before = f.snapshot();
        assert!(f
            .session
            .callback(&op, TurnEvent::SttResult, &f.clock.now(), 10, false)
            .is_err());
        assert_eq!(f.snapshot(), before);
    }
}

#[test]
fn duplicate_and_out_of_order_callbacks_cannot_change_state_or_accounting() {
    let mut f = Fixture::active();
    f.audio();
    f.command(TurnEvent::StartStt).unwrap();
    let op = f.session.turn.as_ref().unwrap().operation.clone().unwrap();
    let before = f.snapshot();
    assert!(f
        .session
        .callback(
            &op,
            TurnEvent::ConversationResult,
            &f.clock.now(),
            10,
            false
        )
        .is_err());
    assert_eq!(f.snapshot(), before);
    f.callback(TurnEvent::SttResult, 10, false).unwrap();
    let before = f.snapshot();
    assert!(f
        .session
        .callback(&op, TurnEvent::SttResult, &f.clock.now(), 10, false)
        .is_err());
    assert_eq!(f.snapshot(), before);
}

#[test]
fn callbacks_cannot_extend_the_stored_authorization_deadline() {
    let mut f = Fixture::active();
    f.audio();
    f.facts.deadline = Deadline::after(f.clock.now(), Duration::from_secs(1)).unwrap();
    f.command(TurnEvent::StartStt).unwrap();
    let mut op = f.session.turn.as_ref().unwrap().operation.clone().unwrap();
    op.deadline = Deadline::after(f.clock.now(), MAX_CLOUD_INTERACTION).unwrap();
    f.clock.advance(Duration::from_secs(1)).unwrap();
    f.session
        .callback(&op, TurnEvent::SttResult, &f.clock.now(), 10, false)
        .unwrap();
    assert_eq!(f.state(), TurnState::Canceled);
}

#[test]
fn clock_domain_mismatch_and_time_reversal_are_atomic_but_stop_still_works() {
    let mut f = Fixture::active();
    f.transcript();
    let owner = f.session.turn.as_ref().unwrap().owner.clone();
    let foreign = ManualClock::new();
    let before = f.snapshot();
    assert_eq!(
        f.session.command(
            &mut f.interaction,
            &owner,
            TurnEvent::RequestConversation,
            &f.facts,
            &foreign.now(),
            MAX_CAPTURED_TURN,
            TranscriptionRoute::LocalOnly
        ),
        Err(Error::Contract(ContractError::ClockDomainMismatch))
    );
    assert_eq!(f.snapshot(), before);
    let old = f.clock.now();
    f.clock.advance(Duration::from_secs(1)).unwrap();
    f.control(SessionEvent::Tick).unwrap();
    let before = f.snapshot();
    assert_eq!(
        f.session.tick(&old),
        Err(Error::Contract(ContractError::TimeReversed))
    );
    assert_eq!(f.snapshot(), before);
    assert_eq!(f.session.stop(), Outcome::Applied);
}

#[test]
fn foreign_guard_deadlines_reject_without_charging_operations() {
    let mut f = Fixture::active();
    f.transcript();
    let other = ManualClock::new();
    f.facts.deadline = Deadline::after(other.now(), MAX_CLOUD_INTERACTION).unwrap();
    let before = f.snapshot();
    assert_eq!(
        f.command(TurnEvent::RequestConversation),
        Err(Error::Contract(ContractError::ClockDomainMismatch))
    );
    assert_eq!(f.snapshot(), before);
}

#[test]
fn session_generation_overflow_revokes_even_when_stop_cannot_increment() {
    let mut f = Fixture::active();
    f.begin();
    f.session.owner.key.epoch = Generation::from_native_sequence(u64::MAX).unwrap();
    assert_eq!(f.session.stop(), Outcome::GenerationExhausted);
    assert!(f.session.revoked);
    assert_eq!(f.session.state, SessionState::Faulted);
    assert_eq!(f.state(), TurnState::Canceled);
    f.ack(Resource::Input);
    assert_eq!(f.session.state, SessionState::Faulted);
}

#[test]
fn turn_generation_overflow_blocks_recovery_and_cancellation_still_revokes() {
    let mut f = fixture_at(TurnState::AwaitingConsent);
    f.session.turn.as_mut().unwrap().owner.generation =
        Generation::from_native_sequence(u64::MAX).unwrap();
    f.facts.gate = Gate::Allowed;
    let before = f.snapshot();
    assert_eq!(
        f.command(TurnEvent::Recover),
        Err(Error::Contract(ContractError::Overflow))
    );
    assert_eq!(f.snapshot(), before);
    assert_eq!(
        f.command(TurnEvent::Cancel),
        Ok(Outcome::GenerationExhausted)
    );
    assert!(f.session.revoked);
}

#[test]
fn identifier_allocation_overflow_does_not_charge_admission() {
    let mut f = Fixture::active();
    f.prepare().unwrap();
    f.interaction.jobs = u64::MAX;
    let before = f.snapshot();
    assert_eq!(
        f.command(TurnEvent::BeginCapture),
        Err(Error::Contract(ContractError::Overflow))
    );
    assert_eq!(f.snapshot(), before);
    f.interaction.turns = u64::MAX;
    f.command(TurnEvent::Cancel).unwrap();
    assert_eq!(f.prepare(), Err(Error::Contract(ContractError::Overflow)));
}

#[test]
fn fake_release_ack_rejects_forgery_foreign_lease_early_and_duplicate_ack() {
    let mut f = Fixture::active();
    f.begin();
    let ticket = f.ticket(Resource::Input);
    assert_eq!(
        f.session
            .cleanup
            .acknowledge(&FakeReleaseEvidence(ticket.clone())),
        Err(Error::InvalidAcknowledgement)
    );
    f.session.stop();
    let mut forged = ticket.clone();
    forged.nonce = Arc::new(());
    assert_eq!(
        f.session.cleanup.acknowledge(&FakeReleaseEvidence(forged)),
        Err(Error::InvalidAcknowledgement)
    );
    let mut forged = ticket.clone();
    forged.lease = JobId::from_native_sequence(999).unwrap();
    assert_eq!(
        f.session.cleanup.acknowledge(&FakeReleaseEvidence(forged)),
        Err(Error::InvalidAcknowledgement)
    );
    let mut other = Fixture::active();
    other.begin();
    other.session.stop();
    assert_eq!(
        f.session
            .cleanup
            .acknowledge(&FakeReleaseEvidence(other.ticket(Resource::Input))),
        Err(Error::InvalidAcknowledgement)
    );
    assert_eq!(
        f.control(SessionEvent::CleanupComplete),
        Err(Error::CleanupPending)
    );
    f.session
        .cleanup
        .acknowledge(&FakeReleaseEvidence(ticket.clone()))
        .unwrap();
    assert_eq!(
        f.session.cleanup.acknowledge(&FakeReleaseEvidence(ticket)),
        Err(Error::InvalidAcknowledgement)
    );
    assert_eq!(f.session.state, SessionState::Stopping);
    f.control(SessionEvent::CleanupComplete).unwrap();
    assert_eq!(f.session.state, SessionState::Stopped);
}

#[test]
fn late_fake_ack_after_epoch_invalidation_changes_cleanup_only() {
    for terminal in [SessionState::Expired, SessionState::Faulted] {
        let mut f = Fixture::active();
        f.begin();
        let ticket = f.ticket(Resource::Input);
        let epoch = f.session.owner.key.epoch;
        f.session.invalidate(terminal);
        assert_ne!(epoch, f.session.owner.key.epoch);
        f.session
            .cleanup
            .acknowledge(&FakeReleaseEvidence(ticket))
            .unwrap();
        assert!(f.session.cleanup.clear());
        assert_eq!(f.session.state, terminal);
        assert!(f.session.revoked);
        assert!(f.session.turn.as_ref().unwrap().operation.is_none());
    }
}

#[test]
fn off_and_serialized_ack_metadata_cannot_complete_native_cleanup() {
    let mut f = Fixture::active();
    f.begin();
    f.session.stop();
    let off = MicrophoneOwner::Off;
    let ack: MicrophoneReleaseStatus = serde_json::from_str("\"acknowledged\"").unwrap();
    assert_eq!(off, MicrophoneOwner::Off);
    assert_eq!(ack, MicrophoneReleaseStatus::Acknowledged);
    assert!(!f.session.cleanup.clear());
    assert_eq!(
        f.control(SessionEvent::CleanupComplete),
        Err(Error::CleanupPending)
    );
}

#[test]
fn finalization_requires_fake_input_release_ack_and_no_duplicate_audio() {
    let mut f = Fixture::active();
    f.begin();
    f.callback(TurnEvent::FinishCapture, 0, false).unwrap();
    let before = f.snapshot();
    assert_eq!(
        f.callback(TurnEvent::AudioReady, 1, false),
        Err(Error::CleanupPending)
    );
    assert_eq!(f.snapshot(), before);
    f.ack(Resource::Input);
    let op = f.session.turn.as_ref().unwrap().operation.clone().unwrap();
    f.callback(TurnEvent::AudioReady, 1, false).unwrap();
    let before = f.snapshot();
    assert!(f
        .session
        .callback(&op, TurnEvent::AudioReady, &f.clock.now(), 1, false)
        .is_err());
    assert_eq!(f.snapshot(), before);
}

#[test]
fn capture_consent_wait_does_not_admit_and_recovery_does_not_capture() {
    let mut f = Fixture::active();
    f.prepare().unwrap();
    f.facts.gate = Gate::MissingConsent;
    let idle = f.session.idle_deadline.clone();
    f.command(TurnEvent::BeginCapture).unwrap();
    assert_eq!(f.state(), TurnState::AwaitingConsent);
    assert_eq!(f.interaction.admitted_turns, 0);
    f.facts.gate = Gate::Allowed;
    f.facts.binding = ConsentBindingId::from_native_sequence(2).unwrap();
    f.command(TurnEvent::Recover).unwrap();
    assert_eq!(f.state(), TurnState::Idle);
    assert!(f.session.turn.as_ref().unwrap().operation.is_none());
    assert!(f.session.cleanup.clear());
    assert_eq!(f.interaction.admitted_turns, 0);
    assert_eq!(
        idle.remaining(&f.clock.now()).unwrap(),
        f.session.idle_deadline.remaining(&f.clock.now()).unwrap()
    );
    f.command(TurnEvent::BeginCapture).unwrap();
    assert_eq!(f.interaction.admitted_turns, 1);
}

#[test]
fn missing_or_expired_cloud_stt_consent_discards_audio_and_retires_turn() {
    for expired in [false, true] {
        let mut f = Fixture::active();
        f.audio();
        let id = f.session.turn.as_ref().unwrap().owner.turn;
        if expired {
            f.facts.deadline = Deadline::after(f.clock.now(), Duration::ZERO).unwrap();
        } else {
            f.facts.gate = Gate::MissingConsent;
        }
        f.command_route(TurnEvent::StartStt, TranscriptionRoute::Cloud)
            .unwrap();
        assert_eq!(f.state(), TurnState::Canceled);
        assert!(!f.session.turn.as_ref().unwrap().audio_retained);
        assert_eq!(f.interaction.admitted_turns, 1);
        assert_eq!(
            f.interaction
                .budget
                .provider_requests(ConversationProviderRole::Stt),
            0
        );
        f.facts.gate = Gate::Allowed;
        f.facts.deadline = f.session.grant_deadline.clone();
        f.begin();
        assert_ne!(id, f.session.turn.as_ref().unwrap().owner.turn);
    }
}

#[test]
fn text_consent_recovery_revalidates_without_dispatch_or_speech() {
    for speech in [false, true] {
        let mut f = Fixture::active();
        if speech {
            f.reply();
        } else {
            f.transcript();
        }
        let event = if speech {
            TurnEvent::RequestSpeech
        } else {
            TurnEvent::RequestConversation
        };
        let checkpoint = f.state();
        f.facts.gate = Gate::MissingConsent;
        f.command(event).unwrap();
        let jobs = f.interaction.jobs;
        let before = f.snapshot();
        f.facts.provenance_allowed = false;
        f.facts.gate = Gate::Allowed;
        assert_eq!(f.command(TurnEvent::Recover), Err(Error::Inadmissible));
        assert_eq!(f.snapshot(), before);
        f.facts.provenance_allowed = true;
        f.facts.binding = ConsentBindingId::from_native_sequence(2).unwrap();
        f.command(TurnEvent::Recover).unwrap();
        assert_eq!(f.state(), checkpoint);
        assert!(f.session.turn.as_ref().unwrap().operation.is_none());
        assert_eq!(f.interaction.jobs, jobs);
        f.command(event).unwrap();
        assert_eq!(f.interaction.jobs, jobs + 1);
    }
}

#[test]
fn consent_and_action_review_expire_at_earliest_original_deadline() {
    for state in [TurnState::AwaitingConsent, TurnState::NeedsActionReview] {
        for limit in [Duration::from_secs(10), IDLE_CHILD_TIMEOUT] {
            let mut f = fixture_at(state);
            if limit < IDLE_CHILD_TIMEOUT {
                f.session.grant_deadline = Deadline::after(f.clock.now(), limit).unwrap();
            }
            f.clock.advance(limit - Duration::from_nanos(1)).unwrap();
            f.control(SessionEvent::Tick).unwrap();
            assert_eq!(f.session.state, SessionState::Active);
            f.clock.advance(Duration::from_nanos(1)).unwrap();
            assert_eq!(f.control(SessionEvent::Tick), Ok(Outcome::Expired));
            assert_eq!(f.state(), TurnState::Canceled);
            assert_eq!(f.session.turn.as_ref().unwrap().transcript_bytes, 0);
        }
    }
}

#[test]
fn action_review_from_transcript_or_reply_is_inert_and_dismissal_retires() {
    for reply in [false, true] {
        let mut f = Fixture::active();
        f.transcript();
        if reply {
            f.command(TurnEvent::RequestConversation).unwrap();
            f.callback(TurnEvent::ConversationResult, 10, false)
                .unwrap();
        }
        f.command(TurnEvent::ActionReview).unwrap();
        let jobs = f.interaction.jobs;
        for event in [
            TurnEvent::RequestConversation,
            TurnEvent::RequestSpeech,
            TurnEvent::BeginCapture,
        ] {
            assert!(f.command(event).is_err());
        }
        f.command(TurnEvent::ReviewDone).unwrap();
        assert_eq!(f.state(), TurnState::Idle);
        assert!(f.session.turn.as_ref().unwrap().retired);
        assert_eq!(f.interaction.jobs, jobs);
    }
}

#[test]
fn action_review_handoff_is_cancellation_without_any_other_operation() {
    let mut f = fixture_at(TurnState::NeedsActionReview);
    let jobs = f.interaction.jobs;
    f.command(TurnEvent::Cancel).unwrap();
    assert_eq!(f.state(), TurnState::Canceled);
    assert_eq!(f.interaction.jobs, jobs);
}

#[test]
fn reply_ready_during_preparation_duplicate_output_is_rejected() {
    let mut f = Fixture::active();
    f.reply();
    f.command(TurnEvent::RequestSpeech).unwrap();
    assert_eq!(f.state(), TurnState::ReplyReady);
    let before = f.snapshot();
    assert_eq!(
        f.command(TurnEvent::RequestSpeech),
        Err(Error::InvalidTransition)
    );
    assert_eq!(f.snapshot(), before);
    f.facts.gate = Gate::MissingConsent;
    assert_eq!(
        f.command(TurnEvent::RequestSpeech),
        Err(Error::InvalidTransition)
    );
    assert_eq!(f.snapshot(), before);
    f.callback(TurnEvent::SpeechStarted, 0, false).unwrap();
    assert_eq!(f.state(), TurnState::Speaking);
}

#[test]
fn speech_stop_or_failure_cannot_open_followup() {
    for started in [false, true] {
        for failure in [false, true] {
            let mut f = Fixture::active();
            f.reply();
            f.command(TurnEvent::RequestSpeech).unwrap();
            if started {
                f.callback(TurnEvent::SpeechStarted, 0, false).unwrap();
            }
            let op = f.session.turn.as_ref().unwrap().operation.clone().unwrap();
            if failure {
                f.callback(TurnEvent::Fail, 0, false).unwrap();
                assert_eq!(f.state(), TurnState::Failed);
                assert_eq!(f.session.turn.as_ref().unwrap().reply_bytes, 10);
            } else {
                f.command(TurnEvent::Cancel).unwrap();
            }
            let before = f.snapshot();
            assert!(f
                .session
                .callback(&op, TurnEvent::SpeechCompleted, &f.clock.now(), 0, true)
                .is_err());
            assert_eq!(f.snapshot(), before);
        }
    }
}

#[test]
fn normal_speech_completion_followup_uses_fresh_turn_and_release_ack() {
    let mut f = Fixture::active();
    f.speaking();
    let old = f.session.turn.as_ref().unwrap().owner.turn;
    f.callback(TurnEvent::SpeechCompleted, 0, true).unwrap();
    assert_eq!(f.state(), TurnState::FollowUpWindow);
    assert_eq!(f.prepare(), Err(Error::CleanupPending));
    f.ack(Resource::Output);
    f.clock.advance(Duration::from_secs(7)).unwrap();
    f.prepare().unwrap();
    let fresh = f.session.turn.as_ref().unwrap().owner.turn;
    assert_ne!(fresh, old);
    f.command(TurnEvent::BeginCapture).unwrap();
    assert_eq!(f.state(), TurnState::Listening);
    assert_eq!(f.interaction.admitted_turns, 2);
}

#[test]
fn followup_timeout_returns_idle_and_does_not_renew_on_prepare() {
    let mut f = Fixture::active();
    f.speaking();
    f.callback(TurnEvent::SpeechCompleted, 0, true).unwrap();
    f.ack(Resource::Output);
    f.clock.advance(FOLLOW_UP_WINDOW).unwrap();
    f.control(SessionEvent::Tick).unwrap();
    assert_eq!(f.state(), TurnState::Idle);
    assert!(f.session.turn.as_ref().unwrap().retired);
    let mut f = Fixture::active();
    f.speaking();
    f.callback(TurnEvent::SpeechCompleted, 0, true).unwrap();
    f.ack(Resource::Output);
    f.clock.advance(Duration::from_secs(7)).unwrap();
    f.prepare().unwrap();
    f.clock.advance(Duration::from_secs(1)).unwrap();
    f.command(TurnEvent::BeginCapture).unwrap();
    assert_ne!(f.state(), TurnState::Listening);
    assert_eq!(f.interaction.admitted_turns, 1);
}

#[test]
fn speech_completion_without_followup_and_text_only_completion_retire() {
    let mut f = Fixture::active();
    f.speaking();
    f.callback(TurnEvent::SpeechCompleted, 0, false).unwrap();
    assert_eq!(f.state(), TurnState::Idle);
    assert!(f.session.turn.as_ref().unwrap().retired);
    f.ack(Resource::Output);
    let mut f = Fixture::active();
    f.reply();
    f.command(TurnEvent::TextComplete).unwrap();
    assert_eq!(f.state(), TurnState::Idle);
    assert!(f.session.turn.as_ref().unwrap().retired);
    assert!(f.session.cleanup.clear());
}

#[test]
fn eight_and_sixteen_admissions_survive_child_recovery_and_final_turn_finishes() {
    let mut f = Fixture::active();
    for child in 0..2 {
        for n in 0..8 {
            f.reply();
            assert_eq!(f.session.state, SessionState::Active);
            assert_eq!(f.interaction.admitted_turns, child * 8 + n + 1);
            let outcome = f.command(TurnEvent::TextComplete).unwrap();
            if n == 7 {
                assert_eq!(outcome, Outcome::Expired);
                assert_eq!(f.session.state, SessionState::Expired);
            } else {
                assert_eq!(outcome, Outcome::Applied);
            }
        }
        assert_eq!(
            f.interaction
                .budget
                .child_turns(f.session.owner.key.session),
            Ok(8)
        );
        if child == 0 {
            f.session = f
                .interaction
                .session(
                    ConfigurationRevision::from_native_sequence(1).unwrap(),
                    f.facts.binding,
                    f.facts.deadline.clone(),
                    f.clock.now(),
                )
                .unwrap();
            f.control(SessionEvent::Start).unwrap();
            let op = f.session.startup.clone().unwrap();
            f.session.ready(&op, &f.facts, &f.clock.now()).unwrap();
        }
    }
    let total = f.interaction.admitted_turns;
    assert_eq!(total, 16);
    f.session = f
        .interaction
        .session(
            ConfigurationRevision::from_native_sequence(1).unwrap(),
            f.facts.binding,
            f.facts.deadline.clone(),
            f.clock.now(),
        )
        .unwrap();
    assert_eq!(
        f.control(SessionEvent::Start),
        Err(Error::Contract(ContractError::LimitExceeded))
    );
    assert_eq!(f.interaction.admitted_turns, total);
}

#[test]
fn hard_child_expiry_still_cancels_the_final_admitted_turn() {
    let mut f = Fixture::active();
    for _ in 0..6 {
        f.reply();
        f.command(TurnEvent::TextComplete).unwrap();
    }
    f.reply();
    // Time passes while an admitted turn is in progress; completion then starts
    // a real idle period. No test or transition rewrites the absolute deadline.
    f.clock.advance(Duration::from_secs(239)).unwrap();
    f.command(TurnEvent::TextComplete).unwrap();
    f.clock.advance(Duration::from_secs(59)).unwrap();
    f.reply();
    assert_eq!(f.interaction.admitted_turns, 8);
    assert_eq!(f.session.state, SessionState::Active);
    f.clock.advance(Duration::from_secs(2)).unwrap();
    assert_eq!(f.command(TurnEvent::TextComplete), Ok(Outcome::Expired));
    assert_eq!(f.session.state, SessionState::Expired);
}

#[test]
fn cloud_audio_exhaustion_does_not_deny_local_stt_or_refund() {
    let mut f = Fixture::active();
    for _ in 0..4 {
        f.audio();
        f.command_route(TurnEvent::StartStt, TranscriptionRoute::Cloud)
            .unwrap();
        f.callback(TurnEvent::Fail, 0, false).unwrap();
    }
    assert_eq!(f.interaction.budget.cloud_audio(), MAX_CLOUD_AUDIO);
    f.audio();
    let before = f.snapshot();
    assert_eq!(
        f.command_route(TurnEvent::StartStt, TranscriptionRoute::Cloud),
        Err(Error::Contract(ContractError::LimitExceeded))
    );
    assert_eq!(f.snapshot(), before);
    f.command_route(TurnEvent::StartStt, TranscriptionRoute::LocalOnly)
        .unwrap();
    assert_eq!(f.interaction.budget.cloud_audio(), MAX_CLOUD_AUDIO);
    assert_eq!(f.interaction.admitted_turns, 5);
}

#[test]
fn failed_and_canceled_turns_are_charged_exactly_once() {
    let mut f = Fixture::active();
    for n in 1..=8 {
        f.begin();
        if n % 2 == 0 {
            f.callback(TurnEvent::Fail, 0, false).unwrap();
        } else {
            f.command(TurnEvent::Cancel).unwrap();
        }
        f.ack(Resource::Input);
        assert_eq!(f.interaction.admitted_turns, n);
    }
    assert_eq!(f.session.state, SessionState::Expired);
    assert_eq!(
        f.interaction
            .budget
            .child_turns(f.session.owner.key.session),
        Ok(8)
    );
}

#[test]
fn history_overflow_blocks_sharing_and_clear_does_not_refund_accounting() {
    let mut f = Fixture::active();
    f.transcript();
    f.facts.history_allowed = false;
    let before = f.snapshot();
    assert_eq!(
        f.command(TurnEvent::RequestConversation),
        Err(Error::Inadmissible)
    );
    assert_eq!(f.snapshot(), before);
    f.facts.history_allowed = true;
    f.command(TurnEvent::RequestConversation).unwrap();
    f.callback(TurnEvent::ConversationResult, 10, false)
        .unwrap();
    let used = f
        .interaction
        .budget
        .provider_requests(ConversationProviderRole::Conversation);
    // Model explicit resolution through admissibility only; no history store.
    f.facts.history_allowed = false;
    assert_eq!(
        f.command(TurnEvent::ReplyAccepted),
        Err(Error::Inadmissible)
    );
    f.facts.history_allowed = true;
    f.command(TurnEvent::ReplyAccepted).unwrap();
    assert_eq!(
        f.interaction
            .budget
            .provider_requests(ConversationProviderRole::Conversation),
        used
    );
    assert_eq!(f.interaction.admitted_turns, 1);
}

#[test]
fn provenance_privacy_and_independent_budget_denials_are_atomic() {
    for denial in 0..3 {
        let mut f = Fixture::active();
        f.transcript();
        match denial {
            0 => f.facts.provenance_allowed = false,
            1 => f.facts.privacy_clear = false,
            _ => f.facts.budget_allowed = false,
        }
        let before = f.snapshot();
        assert!(f.command(TurnEvent::RequestConversation).is_err());
        assert_eq!(f.snapshot(), before);
    }
}

#[test]
fn oversized_or_empty_audio_transcript_reply_results_fail_without_retry() {
    for (stage, event, max) in [
        (
            TurnState::Finalizing,
            TurnEvent::AudioReady,
            MAX_ENCODED_AUDIO_BYTES,
        ),
        (
            TurnState::Transcribing,
            TurnEvent::SttResult,
            MAX_USER_TRANSCRIPT_BYTES,
        ),
        (
            TurnState::RequestingConversation,
            TurnEvent::ConversationResult,
            MAX_ASSISTANT_REPLY_BYTES,
        ),
    ] {
        for bytes in [0, max + 1] {
            let mut f = if stage == TurnState::Finalizing {
                let mut f = Fixture::active();
                f.begin();
                f.callback(TurnEvent::FinishCapture, 0, false).unwrap();
                f.ack(Resource::Input);
                f
            } else {
                fixture_at(stage)
            };
            f.callback(event, bytes, false).unwrap();
            assert_eq!(f.state(), TurnState::Failed);
            assert!(f.session.turn.as_ref().unwrap().operation.is_none());
        }
    }
}

#[test]
fn validation_rejection_is_terminal_for_transcript_and_reply() {
    for state in [TurnState::ValidatingTranscript, TurnState::ValidatingReply] {
        let mut f = fixture_at(state);
        f.command(TurnEvent::RejectValidation).unwrap();
        assert_eq!(f.state(), TurnState::Failed);
    }
}

#[test]
fn onset_finalization_and_speech_deadlines_use_exact_boundaries() {
    let mut f = Fixture::active();
    f.prepare().unwrap();
    f.command(TurnEvent::BeginCapture).unwrap();
    f.clock.advance(SPEECH_ONSET_WAIT).unwrap();
    f.control(SessionEvent::Tick).unwrap();
    assert_eq!(f.state(), TurnState::Canceled);
    let mut f = Fixture::active();
    f.begin();
    f.clock.advance(MAX_CAPTURED_TURN).unwrap();
    f.control(SessionEvent::Tick).unwrap();
    assert_eq!(f.state(), TurnState::Finalizing);
    f.ack(Resource::Input);
    f.clock.advance(FINALIZATION_TIMEOUT).unwrap();
    f.control(SessionEvent::Tick).unwrap();
    assert_eq!(f.state(), TurnState::Failed);
    let mut f = Fixture::active();
    f.speaking();
    f.clock.advance(SPEECH_OUTPUT_DEADLINE).unwrap();
    f.control(SessionEvent::Tick).unwrap();
    assert_eq!(f.state(), TurnState::Failed);
}

#[test]
fn captured_duration_is_not_extended_by_speech_onset_and_delayed_overbound_fails() {
    let mut f = Fixture::active();
    f.prepare().unwrap();
    f.command(TurnEvent::BeginCapture).unwrap();
    f.clock.advance(Duration::from_secs(4)).unwrap();
    f.callback(TurnEvent::SpeechOnset, 0, false).unwrap();
    f.clock.advance(Duration::from_secs(26)).unwrap();
    f.callback(TurnEvent::FinishCapture, 0, false).unwrap();
    assert_eq!(f.state(), TurnState::Finalizing);
    let mut f = Fixture::active();
    f.begin();
    f.clock
        .advance(MAX_CAPTURED_TURN + Duration::from_nanos(1))
        .unwrap();
    f.control(SessionEvent::Tick).unwrap();
    assert_eq!(f.state(), TurnState::Failed);
}

#[test]
fn interaction_deadline_is_inherited_by_new_children() {
    let mut f = Fixture::inactive();
    f.clock.advance(Duration::from_secs(899)).unwrap();
    f.session = f
        .interaction
        .session(
            ConfigurationRevision::from_native_sequence(1).unwrap(),
            f.facts.binding,
            f.facts.deadline.clone(),
            f.clock.now(),
        )
        .unwrap();
    f.control(SessionEvent::Start).unwrap();
    let op = f.session.startup.clone().unwrap();
    f.session.ready(&op, &f.facts, &f.clock.now()).unwrap();
    f.clock.advance(Duration::from_secs(1)).unwrap();
    assert_eq!(f.control(SessionEvent::Tick), Ok(Outcome::Expired));
}

#[test]
fn followup_timeout_does_not_extend_the_sixty_second_idle_ceiling() {
    let mut f = Fixture::active();
    f.speaking();
    f.callback(TurnEvent::SpeechCompleted, 0, true).unwrap();
    f.ack(Resource::Output);
    let idle = f.session.idle_deadline.clone();
    f.clock.advance(FOLLOW_UP_WINDOW).unwrap();
    f.control(SessionEvent::Tick).unwrap();
    assert_eq!(f.state(), TurnState::Idle);
    assert_eq!(
        idle.remaining(&f.clock.now()).unwrap(),
        f.session.idle_deadline.remaining(&f.clock.now()).unwrap()
    );
    f.clock
        .advance(IDLE_CHILD_TIMEOUT - FOLLOW_UP_WINDOW)
        .unwrap();
    assert_eq!(f.control(SessionEvent::Tick), Ok(Outcome::Expired));
}

#[test]
fn consent_recovery_cannot_replace_original_retention_with_a_later_deadline() {
    let mut f = Fixture::active();
    f.transcript();
    f.facts.gate = Gate::MissingConsent;
    f.facts.deadline = Deadline::after(f.clock.now(), Duration::from_secs(10)).unwrap();
    f.command(TurnEvent::RequestConversation).unwrap();
    f.clock.advance(Duration::from_secs(9)).unwrap();
    f.facts.gate = Gate::Allowed;
    f.facts.deadline = f.session.grant_deadline.clone();
    f.command(TurnEvent::Recover).unwrap();
    assert_eq!(f.state(), TurnState::ValidatingTranscript);
    assert!(f.session.turn.as_ref().unwrap().operation.is_none());
    f.clock.advance(Duration::from_secs(1)).unwrap();
    f.command(TurnEvent::RequestConversation).unwrap();
    assert_eq!(f.state(), TurnState::Canceled);
    assert_eq!(
        f.interaction
            .budget
            .provider_requests(ConversationProviderRole::Conversation),
        0
    );
}

#[test]
fn startup_ready_and_fault_callbacks_cannot_bypass_readiness_or_ownership() {
    let mut f = Fixture::inactive();
    f.control(SessionEvent::Start).unwrap();
    let mut op = f.session.startup.clone().unwrap();
    op.job = JobId::from_native_sequence(999).unwrap();
    let before = f.snapshot();
    assert_eq!(
        f.session.ready(&op, &f.facts, &f.clock.now()),
        Err(Error::OperationMismatch)
    );
    assert_eq!(f.snapshot(), before);
    let op = f.session.startup.clone().unwrap();
    f.facts.privacy_clear = false;
    assert_eq!(
        f.session.ready(&op, &f.facts, &f.clock.now()),
        Err(Error::Inadmissible)
    );
    assert_eq!(f.snapshot(), before);
    f.control(SessionEvent::Fault).unwrap();
    assert_eq!(f.session.state, SessionState::Faulted);
    assert!(f.session.ready(&op, &f.facts, &f.clock.now()).is_err());
}

#[test]
fn end_fault_and_stop_cover_each_allocated_session_stage() {
    for event in [SessionEvent::End, SessionEvent::Fault, SessionEvent::Stop] {
        for state in [
            SessionState::Starting,
            SessionState::Active,
            SessionState::Paused,
            SessionState::Stopping,
        ] {
            let mut f = Fixture::inactive();
            f.control(SessionEvent::Start).unwrap();
            if state != SessionState::Starting {
                let op = f.session.startup.clone().unwrap();
                f.session.ready(&op, &f.facts, &f.clock.now()).unwrap();
            }
            if state == SessionState::Paused {
                f.control(SessionEvent::Pause).unwrap();
            }
            if state == SessionState::Stopping {
                f.control(SessionEvent::End).unwrap();
            }
            let expected = session_next(state, event);
            match expected {
                Ok(next) => {
                    f.control(event).unwrap();
                    assert_eq!(f.session.state, next);
                }
                Err(error) => {
                    let before = f.snapshot();
                    assert_eq!(f.control(event), Err(error));
                    assert_eq!(f.snapshot(), before);
                }
            }
        }
    }
}

#[test]
fn old_release_ticket_cannot_release_new_capture_or_output_resources() {
    let mut f = Fixture::active();
    f.begin();
    let old = f.ticket(Resource::Input);
    f.command(TurnEvent::Cancel).unwrap();
    f.ack(Resource::Input);
    f.begin();
    f.command(TurnEvent::Cancel).unwrap();
    assert_eq!(
        f.session.cleanup.acknowledge(&FakeReleaseEvidence(old)),
        Err(Error::InvalidAcknowledgement)
    );
    assert!(!f.session.cleanup.clear());
    f.ack(Resource::Input);
    f.reply();
    f.command(TurnEvent::RequestSpeech).unwrap();
    let mut wrong_resource = f.ticket(Resource::Output);
    wrong_resource.resource = Resource::Input;
    f.session.stop();
    assert_eq!(
        f.session
            .cleanup
            .acknowledge(&FakeReleaseEvidence(wrong_resource)),
        Err(Error::InvalidAcknowledgement)
    );
    assert_eq!(
        f.control(SessionEvent::CleanupComplete),
        Err(Error::CleanupPending)
    );
    f.ack(Resource::Output);
    f.control(SessionEvent::CleanupComplete).unwrap();
}

#[test]
fn capture_timeout_result_is_independent_of_timer_or_callback_arrival_order() {
    for callback_first in [false, true] {
        let mut f = Fixture::active();
        f.prepare().unwrap();
        f.command(TurnEvent::BeginCapture).unwrap();
        f.clock.advance(SPEECH_ONSET_WAIT).unwrap();
        if callback_first {
            f.callback(TurnEvent::SpeechOnset, 0, false).unwrap();
        } else {
            f.control(SessionEvent::Tick).unwrap();
        }
        assert_eq!(f.state(), TurnState::Canceled);
        let mut f = Fixture::active();
        f.begin();
        f.clock
            .advance(MAX_CAPTURED_TURN + Duration::from_nanos(1))
            .unwrap();
        if callback_first {
            f.callback(TurnEvent::FinishCapture, 0, false).unwrap();
        } else {
            f.control(SessionEvent::Tick).unwrap();
        }
        assert_eq!(f.state(), TurnState::Failed);
    }
}
