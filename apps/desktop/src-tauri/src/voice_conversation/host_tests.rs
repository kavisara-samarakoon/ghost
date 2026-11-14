//! Fake native receipts and metadata only. No OS/menu/audio/provider activation.
use super::*;
use crate::voice_conversation::clock::{ManualClock, MonotonicClock};
use std::sync::Mutex;

fn binding() -> Binding {
    Binding {
        run: AppRunId::from_native_sequence(1).unwrap(),
        os_session: OsSessionId(NonZeroU64::new(1).unwrap()),
        host: HostId(NonZeroU64::new(1).unwrap()),
        generation: Generation::from_native_sequence(1).unwrap(),
    }
}
fn scope(owner: MicrophoneOwner, background: bool) -> Scope {
    Scope {
        microphone: true,
        manual: owner == MicrophoneOwner::ManualForeground,
        wake: owner == MicrophoneOwner::WakeDetection,
        conversation: owner == MicrophoneOwner::ConversationTurn,
        background_wake: background && owner == MicrophoneOwner::WakeDetection,
        background_conversation: background && owner == MicrophoneOwner::ConversationTurn,
    }
}
fn combined() -> Scope {
    Scope {
        microphone: true,
        manual: true,
        wake: true,
        conversation: true,
        background_wake: true,
        background_conversation: true,
    }
}
const OWNERS: [MicrophoneOwner; 3] = [
    MicrophoneOwner::ManualForeground,
    MicrophoneOwner::WakeDetection,
    MicrophoneOwner::ConversationTurn,
];

struct Fixture {
    clock: ManualClock,
    host: HostModel,
    control: NativeControl,
}
impl Fixture {
    fn new() -> Self {
        let clock = ManualClock::new();
        let host = HostModel::synthetic(binding(), clock.now());
        let control = host.native_control();
        Self {
            clock,
            host,
            control,
        }
    }
    fn event(&mut self, event: Event) -> Result<()> {
        let receipt = self.host.synthetic_observation(event)?;
        self.host.apply(&receipt, &self.clock.now())
    }
    fn menu(&mut self) {
        self.event(Event::MenuPreparing).unwrap();
        self.event(Event::MenuReady {
            keyboard: true,
            accessible: true,
        })
        .unwrap();
    }
    fn authorize(&mut self, scope: Scope) -> EnablePreview {
        if !self.host.permission {
            self.event(Event::PermissionRestored).unwrap();
        }
        let preview = self
            .host
            .prepare_enable(scope, Duration::from_secs(20), &self.clock.now())
            .unwrap();
        self.host
            .enable(&self.control, &preview, &self.clock.now())
            .unwrap();
        preview
    }
    fn capture(&mut self, owner: MicrophoneOwner, acquiring: bool) -> CaptureTicket {
        self.host
            .synthetic_capture(owner, acquiring, &self.clock.now())
            .unwrap()
    }
    fn eligible(&mut self, owner: MicrophoneOwner) -> bool {
        self.host.eligible(owner, &self.clock.now()).unwrap()
    }
    fn clear_input(&mut self) {
        if let Some(capture) = &self.host.capture {
            if capture.phase == CapturePhase::StoppingUnconfirmed {
                let ticket = capture.ticket.clone();
                self.host.release_ack(&ticket, true, true).unwrap();
            }
        }
        if let Some(ticket) = self.host.frame_ticket() {
            self.host.frames_ack(&ticket, true, true).unwrap();
        }
    }
    fn cleanup(&mut self) {
        self.clear_input();
        if let Some(ticket) = self.host.cleanup_ticket() {
            for resource in [Resource::Output, Resource::Workers, Resource::Content] {
                let state = self.host.cleanup.as_ref().unwrap();
                if state.required[resource as usize] && !state.acknowledged[resource as usize] {
                    self.host.resource_ack(&ticket, resource, true).unwrap();
                }
            }
        }
        assert!(self.host.cleanup_complete());
    }
    fn assert_off(&mut self) {
        for owner in OWNERS {
            assert!(!self.eligible(owner));
        }
        assert!(self.host.authorization.is_none());
    }
}

#[test]
fn cold_start_has_no_voice_authority_or_claimed_menu_availability() {
    let mut f = Fixture::new();
    assert_eq!(f.host.application, Application::Running);
    assert_eq!(f.host.window, Window::Visible);
    assert_eq!(f.host.menu, Menu::Unavailable);
    assert!(!f.host.menu_status().stop_reachable);
    assert!(!f.host.shutdown_complete());
    assert_eq!(f.host.effects, Effects::default());
    f.assert_off();
}

#[test]
fn close_to_menu_bar_preserves_only_existing_background_wake_authority() {
    let mut f = Fixture::new();
    f.menu();
    let preview = f.authorize(scope(MicrophoneOwner::WakeDetection, true));
    f.capture(MicrophoneOwner::WakeDetection, false);
    let original = f.host.authorization.as_ref().unwrap().alive.clone();
    f.clock.advance(Duration::from_secs(5)).unwrap();
    f.event(Event::CloseWindow).unwrap();
    assert_eq!(f.host.window, Window::Hidden);
    assert_eq!(f.host.application, Application::Running);
    assert!(original.load(Ordering::Acquire));
    assert!(f.eligible(MicrophoneOwner::WakeDetection));
    assert!(!f.eligible(MicrophoneOwner::ConversationTurn));
    assert!(!f.eligible(MicrophoneOwner::ManualForeground));
    assert!(!f.host.effects.terminate_host);
    assert_eq!(
        preview.0.deadline.remaining(&f.clock.now()).unwrap(),
        Duration::from_secs(15)
    );
    assert!(f.host.menu_status().stop_reachable);
}

#[test]
fn background_conversation_capture_may_continue_without_renderer() {
    let mut f = Fixture::new();
    f.menu();
    f.authorize(scope(MicrophoneOwner::ConversationTurn, true));
    let ticket = f.capture(MicrophoneOwner::ConversationTurn, true);
    f.event(Event::RendererLost).unwrap();
    f.event(Event::HideWindow).unwrap();
    f.host.capture_ready(&ticket, &f.clock.now()).unwrap();
    assert!(f.eligible(MicrophoneOwner::ConversationTurn));
    assert!(!f.host.renderer_available);
    assert_eq!(
        f.host.menu_status().voice_label,
        VoiceLabel::ConversationCapture
    );
    assert!(!f.host.effects.revoke_egress);
}

#[test]
fn close_without_background_scope_revokes_before_claiming_capture_stopped() {
    for owner in OWNERS {
        let mut f = Fixture::new();
        f.menu();
        f.authorize(scope(owner, false));
        f.capture(owner, false);
        let authority = f.host.authorization.as_ref().unwrap().alive.clone();
        f.event(Event::CloseWindow).unwrap();
        assert_eq!(f.host.window, Window::Hidden);
        assert!(!authority.load(Ordering::Acquire));
        assert!(f.host.effects.revoke_voice && f.host.effects.revoke_egress);
        assert_eq!(
            f.host.microphone_status(),
            (owner, MicrophoneReleaseStatus::StoppingUnconfirmed)
        );
        assert_eq!(
            f.host.menu_status().voice_label,
            VoiceLabel::StoppingUnconfirmed
        );
        f.assert_off();
    }
}

#[test]
fn close_hide_minimize_reopen_and_focus_never_mint_authority() {
    for close in [Event::CloseWindow, Event::HideWindow, Event::Minimize] {
        let mut f = Fixture::new();
        f.menu();
        f.event(close).unwrap();
        assert_ne!(f.host.window, Window::Visible);
        f.event(Event::ReopenWindow).unwrap();
        assert!(!f.host.foreground);
        f.event(Event::Focus).unwrap();
        assert!(f.host.foreground);
        assert_eq!(f.host.application, Application::Running);
        f.assert_off();
    }
}

#[test]
fn quit_is_complete_shutdown_request_and_never_a_window_hide() {
    let mut f = Fixture::new();
    f.menu();
    f.authorize(combined());
    f.capture(MicrophoneOwner::ConversationTurn, false);
    f.host.quit(&f.control).unwrap();
    assert_eq!(f.host.window, Window::Visible);
    assert_eq!(f.host.application, Application::ShuttingDown);
    assert!(f.host.effects.terminate_host && f.host.effects.terminate_workers);
    assert!(f.host.effects.stop_output && f.host.effects.clear_content);
    assert!(!f.host.shutdown_complete());
    assert!(matches!(
        f.host.synthetic_observation(Event::ReopenWindow),
        Err(Error::Closed)
    ));
    f.cleanup();
    assert!(!f.host.shutdown_complete());
    f.event(Event::HostTerminated).unwrap();
    assert!(f.host.shutdown_complete());
    f.assert_off();
}

#[test]
fn quit_from_hidden_menu_bar_app_does_not_reopen_or_intercept_quit() {
    let mut f = Fixture::new();
    f.menu();
    f.authorize(scope(MicrophoneOwner::WakeDetection, true));
    f.event(Event::CloseWindow).unwrap();
    f.host.quit(&f.control).unwrap();
    assert_eq!(f.host.window, Window::Hidden);
    assert_eq!(f.host.application, Application::ShuttingDown);
    assert!(f.host.effects.terminate_host);
    assert!(!f.host.menu_status().stop_reachable);
    f.cleanup();
    f.event(Event::HostTerminated).unwrap();
    assert!(f.host.shutdown_complete());
}

#[test]
fn accessible_keyboard_menu_readiness_is_required_before_hiding() {
    for (keyboard, accessible) in [(false, false), (false, true), (true, false)] {
        let mut f = Fixture::new();
        f.authorize(scope(MicrophoneOwner::WakeDetection, true));
        f.capture(MicrophoneOwner::WakeDetection, false);
        f.event(Event::MenuPreparing).unwrap();
        assert_eq!(
            f.event(Event::MenuReady {
                keyboard,
                accessible
            }),
            Err(Error::UnreachableControls)
        );
        assert_eq!(f.host.menu, Menu::Unavailable);
        assert_eq!(f.host.window, Window::Visible);
        assert!(f.host.error_visible);
        assert_eq!(f.event(Event::CloseWindow), Err(Error::UnreachableControls));
        f.assert_off();
    }
}

#[test]
fn menu_loss_while_hidden_revokes_and_keeps_error_window_visible() {
    let mut f = Fixture::new();
    f.menu();
    f.authorize(scope(MicrophoneOwner::WakeDetection, true));
    f.capture(MicrophoneOwner::WakeDetection, false);
    f.event(Event::CloseWindow).unwrap();
    f.event(Event::MenuLost).unwrap();
    assert_eq!(f.host.window, Window::Visible);
    assert!(f.host.error_visible);
    assert!(!f.host.menu_status().keyboard_reachable);
    f.cleanup();
    f.menu();
    f.event(Event::Focus).unwrap();
    f.assert_off();
}

#[test]
fn losing_foreground_without_native_menu_cannot_leave_background_capture_active() {
    let mut f = Fixture::new();
    f.authorize(scope(MicrophoneOwner::ConversationTurn, true));
    f.capture(MicrophoneOwner::ConversationTurn, false);
    f.event(Event::Blur).unwrap();
    assert_eq!(f.host.window, Window::Visible);
    assert!(f.host.error_visible);
    assert_eq!(
        f.host.menu_status().voice_label,
        VoiceLabel::StoppingUnconfirmed
    );
    f.assert_off();
}

#[test]
fn menu_stop_is_independent_of_stalled_external_ports_and_renderer() {
    let mut f = Fixture::new();
    f.menu();
    f.authorize(combined());
    f.capture(MicrophoneOwner::ConversationTurn, false);
    f.event(Event::RendererLost).unwrap();
    // Synthetic provider, network, renderer, audit and storage locks. These
    // deliberately remain held; the host/Stop path has no references to them.
    let ports: [Mutex<()>; 5] = std::array::from_fn(|_| Mutex::new(()));
    let guards: Vec<_> = ports.iter().map(|p| p.lock().unwrap()).collect();
    f.host.menu_stop(&f.control).unwrap();
    assert!(ports.iter().all(|p| p.try_lock().is_err()));
    assert_eq!(f.host.menu_status().stop_label, "Stop Listening");
    assert!(f.host.menu_status().stop_reachable);
    f.assert_off();
    f.cleanup();
    assert_eq!(f.host.application, Application::Running);
    drop(guards);
}

#[test]
fn unavailable_menu_does_not_disable_independent_native_stop_or_quit() {
    let mut f = Fixture::new();
    assert_eq!(
        f.host.menu_stop(&f.control),
        Err(Error::UnreachableControls)
    );
    f.host.stop_listening(&f.control).unwrap();
    f.host.quit(&f.control).unwrap();
    assert_eq!(f.host.application, Application::ShuttingDown);
}

#[test]
fn all_manual_foreground_loss_paths_cancel_pending_and_active_m45() {
    for event in [
        Event::Blur,
        Event::Minimize,
        Event::CloseWindow,
        Event::HideWindow,
        Event::NavigateAway,
    ] {
        for acquiring in [false, true] {
            let mut f = Fixture::new();
            f.menu();
            f.authorize(combined());
            let ticket = f.capture(MicrophoneOwner::ManualForeground, acquiring);
            f.event(event).unwrap();
            assert!(f.host.effects.cancel_manual);
            assert_eq!(
                f.host.microphone_status().1,
                MicrophoneReleaseStatus::StoppingUnconfirmed
            );
            assert_eq!(
                f.host.capture_ready(&ticket, &f.clock.now()),
                Err(Error::Stale)
            );
            f.cleanup();
            assert!(!f.eligible(MicrophoneOwner::ManualForeground));
            assert!(f.eligible(MicrophoneOwner::WakeDetection));
            assert!(f.host.capture.is_none()); // eligibility does not start capture
        }
    }
}

#[test]
fn navigation_away_does_not_duplicate_or_cancel_authorized_m45b_lifetime() {
    let mut f = Fixture::new();
    f.authorize(scope(MicrophoneOwner::ConversationTurn, false));
    f.capture(MicrophoneOwner::ConversationTurn, false);
    let epoch = f.host.epoch;
    f.event(Event::NavigateAway).unwrap();
    assert!(f.eligible(MicrophoneOwner::ConversationTurn));
    assert_eq!(f.host.epoch, epoch);
    assert_eq!(
        f.host.capture.as_ref().unwrap().phase,
        CapturePhase::Capturing
    );
}

#[test]
fn wake_background_scope_does_not_authorize_background_conversation() {
    let mut f = Fixture::new();
    f.menu();
    let mut scopes = combined();
    scopes.background_conversation = false;
    f.authorize(scopes);
    f.capture(MicrophoneOwner::ConversationTurn, false);
    f.event(Event::CloseWindow).unwrap();
    assert!(f.host.effects.end_child && f.host.effects.revoke_egress);
    assert!(!f.host.effects.revoke_voice);
    f.cleanup();
    assert!(f.eligible(MicrophoneOwner::WakeDetection));
    assert!(!f.eligible(MicrophoneOwner::ConversationTurn));
    f.event(Event::ReopenWindow).unwrap();
    f.event(Event::Focus).unwrap();
    assert!(!f.eligible(MicrophoneOwner::ConversationTurn));
}

#[test]
fn conversation_background_scope_does_not_authorize_background_wake() {
    let mut f = Fixture::new();
    f.menu();
    let mut scopes = combined();
    scopes.background_wake = false;
    f.authorize(scopes);
    f.capture(MicrophoneOwner::WakeDetection, true);
    f.event(Event::Minimize).unwrap();
    f.cleanup();
    assert!(!f.eligible(MicrophoneOwner::WakeDetection));
    assert!(f.eligible(MicrophoneOwner::ConversationTurn));
    assert!(f.host.capture.is_none());
}

#[test]
fn end_conversation_preserves_independently_authorized_wake_only() {
    let mut f = Fixture::new();
    f.menu();
    f.authorize(combined());
    f.capture(MicrophoneOwner::ConversationTurn, false);
    f.event(Event::EndConversation).unwrap();
    assert!(f.host.effects.end_child && f.host.effects.clear_content);
    assert!(!f.host.effects.revoke_wake);
    f.cleanup();
    assert!(f.eligible(MicrophoneOwner::WakeDetection));
    assert!(!f.eligible(MicrophoneOwner::ConversationTurn));
    assert!(f.host.capture.is_none());
    assert_eq!(f.host.menu_status().voice_label, VoiceLabel::Off);
}

#[test]
fn pause_suspends_all_voice_until_explicit_native_resume_without_renewal() {
    let mut f = Fixture::new();
    f.menu();
    let preview = f.authorize(combined());
    let ticket = f.capture(MicrophoneOwner::ConversationTurn, true);
    f.event(Event::Pause).unwrap();
    assert!(f.host.paused);
    assert_eq!(
        f.host.capture_ready(&ticket, &f.clock.now()),
        Err(Error::Stale)
    );
    assert_eq!(
        f.host.resume(&f.control, &f.clock.now()),
        Err(Error::Unauthorized)
    );
    f.cleanup();
    assert_eq!(f.host.menu_status().voice_label, VoiceLabel::Paused);
    f.event(Event::ReopenWindow).unwrap();
    f.event(Event::Focus).unwrap();
    for owner in OWNERS {
        assert!(!f.eligible(owner));
    }
    f.clock.advance(Duration::from_secs(7)).unwrap();
    f.host.resume(&f.control, &f.clock.now()).unwrap();
    assert!(f.eligible(MicrophoneOwner::ConversationTurn));
    assert_eq!(
        preview.0.deadline.remaining(&f.clock.now()).unwrap(),
        Duration::from_secs(13)
    );
    assert!(Arc::ptr_eq(
        &f.host.authorization.as_ref().unwrap().preview.0,
        &preview.0
    ));
    assert!(f.host.capture.is_none());
}

#[test]
fn pause_does_not_extend_expiry_and_stop_during_pause_requires_fresh_enablement() {
    for expire in [false, true] {
        let mut f = Fixture::new();
        f.authorize(combined());
        f.event(Event::Pause).unwrap();
        f.cleanup();
        if expire {
            f.clock.advance(Duration::from_secs(20)).unwrap();
        } else {
            f.host.stop_listening(&f.control).unwrap();
            f.cleanup();
        }
        assert_eq!(
            f.host.resume(&f.control, &f.clock.now()),
            Err(Error::Unauthorized)
        );
        f.assert_off();
    }
}

#[test]
fn lock_sleep_logout_permission_and_device_loss_revoke_before_cleanup() {
    for event in [
        Event::Lock,
        Event::Sleep,
        Event::Logout,
        Event::PermissionLost,
        Event::DeviceLost,
    ] {
        let mut f = Fixture::new();
        f.menu();
        f.authorize(combined());
        f.capture(MicrophoneOwner::WakeDetection, false);
        let alive = f.host.authorization.as_ref().unwrap().alive.clone();
        f.event(event).unwrap();
        assert!(!alive.load(Ordering::Acquire));
        assert!(f.host.effects.revoke_voice && f.host.effects.revoke_egress);
        assert_eq!(
            f.host.microphone_status().1,
            MicrophoneReleaseStatus::StoppingUnconfirmed
        );
        assert!(!f.host.cleanup_complete());
        f.assert_off();
        assert!(matches!(
            f.host
                .prepare_enable(combined(), Duration::from_secs(10), &f.clock.now()),
            Err(Error::Closed)
        ));
    }
}

#[test]
fn passive_privacy_return_and_focus_never_restore_authority() {
    for (barrier, passive_return) in [
        (Event::Lock, Event::Unlock),
        (Event::Sleep, Event::Wake),
        (Event::PermissionLost, Event::PermissionRestored),
    ] {
        let mut f = Fixture::new();
        f.menu();
        f.authorize(combined());
        f.event(barrier).unwrap();
        f.cleanup();
        f.event(passive_return).unwrap();
        f.event(Event::ReopenWindow).unwrap();
        f.event(Event::Focus).unwrap();
        f.assert_off();
        assert_eq!(
            f.host.resume(&f.control, &f.clock.now()),
            Err(Error::Unauthorized)
        );
        f.authorize(scope(MicrophoneOwner::WakeDetection, true));
        assert!(f.eligible(MicrophoneOwner::WakeDetection));
        assert!(f.host.capture.is_none());
    }
}

#[test]
fn fresh_enablement_after_stop_requires_new_preview_and_complete_cleanup() {
    let mut f = Fixture::new();
    f.event(Event::PermissionRestored).unwrap();
    let old = f
        .host
        .prepare_enable(combined(), Duration::from_secs(20), &f.clock.now())
        .unwrap();
    f.host.stop_listening(&f.control).unwrap();
    assert_eq!(
        f.host.enable(&f.control, &old, &f.clock.now()),
        Err(Error::Stale)
    );
    assert!(matches!(
        f.host
            .prepare_enable(combined(), Duration::from_secs(20), &f.clock.now()),
        Err(Error::Closed)
    ));
    f.cleanup();
    let fresh = f.authorize(combined());
    assert!(!Arc::ptr_eq(&old.0, &fresh.0));
    assert!(f.eligible(MicrophoneOwner::WakeDetection));
}

#[test]
fn privacy_barrier_invalidates_prepared_enablement_and_blocks_new_preview() {
    let mut f = Fixture::new();
    f.event(Event::PermissionRestored).unwrap();
    let preview = f
        .host
        .prepare_enable(combined(), Duration::from_secs(20), &f.clock.now())
        .unwrap();
    f.event(Event::Lock).unwrap();
    f.cleanup();
    assert!(matches!(
        f.host
            .prepare_enable(combined(), Duration::from_secs(20), &f.clock.now()),
        Err(Error::Closed)
    ));
    f.event(Event::Unlock).unwrap();
    assert_eq!(
        f.host.enable(&f.control, &preview, &f.clock.now()),
        Err(Error::Stale)
    );
    f.authorize(combined());
    assert!(f.eligible(MicrophoneOwner::WakeDetection));
}

#[test]
fn logout_requires_another_os_session_host_and_old_controls_cannot_cross_runs() {
    let mut f = Fixture::new();
    f.authorize(combined());
    f.event(Event::Logout).unwrap();
    f.cleanup();
    f.event(Event::PermissionRestored).unwrap();
    assert!(matches!(
        f.host
            .prepare_enable(combined(), Duration::from_secs(10), &f.clock.now()),
        Err(Error::Closed)
    ));
    let fresh = Fixture::new();
    assert_eq!(f.host.stop_listening(&fresh.control), Err(Error::Stale));
    assert_eq!(f.host.quit(&fresh.control), Err(Error::Stale));
    assert!(fresh.host.authorization.is_none());
}

#[test]
fn repeated_native_stop_and_quit_preserve_exact_pending_release_evidence() {
    let mut f = Fixture::new();
    f.authorize(combined());
    let capture = f.capture(MicrophoneOwner::ConversationTurn, true);
    f.host.stop_listening(&f.control).unwrap();
    let cleanup = f.host.cleanup_ticket().unwrap();
    let epoch = f.host.epoch;
    let sequence = f.host.sequence;
    for _ in 0..100 {
        f.host.stop_listening(&f.control).unwrap();
        assert_eq!(f.host.epoch, epoch);
        assert_eq!(f.host.sequence, sequence);
        assert!(f.host.cleanup_ticket().unwrap().0.matches(&cleanup.0));
    }
    f.host.release_ack(&capture, true, true).unwrap();
    let frame = f.host.frame_ticket().unwrap();
    f.host.quit(&f.control).unwrap();
    f.host.quit(&f.control).unwrap();
    assert!(f
        .host
        .frame_ticket()
        .unwrap()
        .cleanup
        .0
        .matches(&frame.cleanup.0));
    f.cleanup();
    f.event(Event::HostTerminated).unwrap();
    assert!(f.host.shutdown_complete());
}

#[test]
fn active_uncertain_and_cleanup_capture_all_keep_quit_completion_unconfirmed() {
    for initial_phase in [
        CapturePhase::Acquiring,
        CapturePhase::Capturing,
        CapturePhase::StoppingUnconfirmed,
        CapturePhase::Cleaning,
    ] {
        let mut f = Fixture::new();
        f.authorize(combined());
        let ticket = f.capture(
            MicrophoneOwner::ConversationTurn,
            initial_phase == CapturePhase::Acquiring,
        );
        if matches!(
            initial_phase,
            CapturePhase::StoppingUnconfirmed | CapturePhase::Cleaning
        ) {
            f.host.stop_listening(&f.control).unwrap();
            if initial_phase == CapturePhase::Cleaning {
                f.host.release_ack(&ticket, true, true).unwrap();
            }
        }
        f.host.quit(&f.control).unwrap();
        f.event(Event::HostTerminated).unwrap();
        assert!(!f.host.shutdown_complete());
        assert_ne!(f.host.microphone_status().0, MicrophoneOwner::Off);
        f.cleanup();
        assert!(f.host.shutdown_complete());
    }
}

#[test]
fn termination_observation_alone_never_proves_resource_cleanup() {
    let mut f = Fixture::new();
    f.authorize(combined());
    f.capture(MicrophoneOwner::WakeDetection, false);
    f.event(Event::HostTerminated).unwrap();
    assert_eq!(f.host.application, Application::TerminationObserved);
    assert!(f.host.effects.terminate_host && f.host.effects.terminate_workers);
    assert!(!f.host.shutdown_complete());
    assert!(matches!(
        f.host.synthetic_observation(Event::ReopenWindow),
        Err(Error::Closed)
    ));
    f.cleanup();
    assert!(f.host.shutdown_complete());
}

#[test]
fn elapsed_time_never_counts_as_release_or_frame_cleanup_evidence() {
    let mut f = Fixture::new();
    f.authorize(combined());
    let capture = f.capture(MicrophoneOwner::WakeDetection, false);
    f.host.quit(&f.control).unwrap();
    f.clock.advance(Duration::from_secs(86400)).unwrap();
    f.host.observe(&f.clock.now()).unwrap();
    assert_eq!(
        f.host.microphone_status().1,
        MicrophoneReleaseStatus::StoppingUnconfirmed
    );
    assert!(f.host.frame_ticket().is_none());
    f.host.release_ack(&capture, true, true).unwrap();
    assert_eq!(
        f.host.microphone_status().1,
        MicrophoneReleaseStatus::Acknowledged
    );
    assert_eq!(f.host.menu_status().voice_label, VoiceLabel::Cleaning);
    assert!(!f.host.shutdown_complete());
    assert_ne!(f.host.microphone_status().0, MicrophoneOwner::Off);
    f.cleanup();
    assert!(!f.host.shutdown_complete());
    f.event(Event::HostTerminated).unwrap();
    assert!(f.host.shutdown_complete());
}

#[test]
fn partial_native_release_or_frame_queue_evidence_never_releases_ownership() {
    for (one, two) in [(false, false), (false, true), (true, false)] {
        let mut f = Fixture::new();
        f.authorize(combined());
        let ticket = f.capture(MicrophoneOwner::ConversationTurn, true);
        f.host.quit(&f.control).unwrap();
        assert_eq!(
            f.host.release_ack(&ticket, one, two),
            Err(Error::CleanupUnconfirmed)
        );
        assert_eq!(
            f.host.microphone_status().1,
            MicrophoneReleaseStatus::StoppingUnconfirmed
        );
        f.host.release_ack(&ticket, true, true).unwrap();
        let frames = f.host.frame_ticket().unwrap();
        assert_eq!(
            f.host.frames_ack(&frames, one, two),
            Err(Error::CleanupUnconfirmed)
        );
        assert!(!f.host.cleanup_complete());
        assert!(f.host.cleanup_failed);
        assert_eq!(f.host.menu_status().voice_label, VoiceLabel::Cleaning);
        f.cleanup();
    }
}

#[test]
fn each_missing_or_failed_output_worker_content_ack_blocks_shutdown() {
    for missing in [Resource::Output, Resource::Workers, Resource::Content] {
        let mut f = Fixture::new();
        f.host.quit(&f.control).unwrap();
        f.event(Event::HostTerminated).unwrap();
        let ticket = f.host.cleanup_ticket().unwrap();
        for resource in [Resource::Output, Resource::Workers, Resource::Content] {
            if resource != missing {
                f.host.resource_ack(&ticket, resource, true).unwrap();
            }
        }
        assert!(!f.host.shutdown_complete());
        assert_eq!(
            f.host.resource_ack(&ticket, missing, false),
            Err(Error::CleanupUnconfirmed)
        );
        assert!(!f.host.shutdown_complete());
        f.host.resource_ack(&ticket, missing, true).unwrap();
        assert!(f.host.shutdown_complete());
    }
}

#[test]
fn stale_duplicate_forged_and_out_of_order_observations_are_rejected() {
    let mut f = Fixture::new();
    let receipt = f.host.synthetic_observation(Event::MenuPreparing).unwrap();
    assert!(matches!(
        f.host.synthetic_observation(Event::PermissionRestored),
        Err(Error::Busy)
    ));
    for sequence in [0, receipt.stamp.sequence + 1, u64::MAX] {
        let mut forged = receipt.clone();
        forged.stamp.sequence = sequence;
        assert_eq!(f.host.apply(&forged, &f.clock.now()), Err(Error::Stale));
    }
    let mut forged = receipt.clone();
    forged.event = Event::PermissionRestored;
    assert_eq!(f.host.apply(&forged, &f.clock.now()), Err(Error::Stale));
    f.host.apply(&receipt, &f.clock.now()).unwrap();
    assert_eq!(f.host.apply(&receipt, &f.clock.now()), Err(Error::Stale));
    assert!(!f.host.permission);
    assert_eq!(f.event(Event::Wake), Err(Error::InvalidTransition));
    assert_eq!(f.event(Event::Unlock), Err(Error::InvalidTransition));
}

#[test]
fn all_host_binding_generation_and_native_proof_fields_must_match() {
    let mutations: [fn(&mut Stamp); 8] = [
        |s| s.binding.run = AppRunId::from_native_sequence(2).unwrap(),
        |s| s.binding.os_session = OsSessionId(NonZeroU64::new(2).unwrap()),
        |s| s.binding.host = HostId(NonZeroU64::new(2).unwrap()),
        |s| s.binding.generation = Generation::from_native_sequence(2).unwrap(),
        |s| s.epoch = Generation::from_native_sequence(2).unwrap(),
        |s| s.native = Arc::new(()),
        |s| s.nonce = Arc::new(()),
        |s| s.sequence = 99,
    ];
    let mut f = Fixture::new();
    let receipt = f
        .host
        .synthetic_observation(Event::PermissionRestored)
        .unwrap();
    for mutate in mutations {
        let mut forged = receipt.clone();
        mutate(&mut forged.stamp);
        assert_eq!(f.host.apply(&forged, &f.clock.now()), Err(Error::Stale));
        assert!(!f.host.permission);
    }
    f.host.apply(&receipt, &f.clock.now()).unwrap();
}

#[test]
fn identical_numeric_bindings_from_another_native_host_are_foreign() {
    let mut f = Fixture::new();
    let mut other = Fixture::new();
    let receipt = other
        .host
        .synthetic_observation(Event::PermissionRestored)
        .unwrap();
    let local = f
        .host
        .synthetic_observation(Event::PermissionRestored)
        .unwrap();
    assert_eq!(f.host.apply(&receipt, &f.clock.now()), Err(Error::Stale));
    assert_eq!(f.host.quit(&other.control), Err(Error::Stale));
    assert_eq!(f.host.stop_listening(&other.control), Err(Error::Stale));
    assert_eq!(f.host.application, Application::Running);
    f.host.apply(&local, &f.clock.now()).unwrap();
}

#[test]
fn wrong_run_session_host_or_generation_cannot_use_independent_native_controls() {
    let mutations: [fn(&mut Binding); 4] = [
        |b| b.run = AppRunId::from_native_sequence(2).unwrap(),
        |b| b.os_session = OsSessionId(NonZeroU64::new(2).unwrap()),
        |b| b.host = HostId(NonZeroU64::new(2).unwrap()),
        |b| b.generation = Generation::from_native_sequence(2).unwrap(),
    ];
    let mut f = Fixture::new();
    for mutate in mutations {
        let mut control = f.control.clone();
        mutate(&mut control.binding);
        assert_eq!(f.host.stop_listening(&control), Err(Error::Stale));
        assert_eq!(f.host.quit(&control), Err(Error::Stale));
    }
    assert_eq!(f.host.effects, Effects::default());
}

#[test]
fn stop_bypasses_full_observation_slot_and_invalidates_queued_callbacks() {
    let mut f = Fixture::new();
    let pending = f
        .host
        .synthetic_observation(Event::PermissionRestored)
        .unwrap();
    f.host.stop_listening(&f.control).unwrap();
    assert_eq!(f.host.apply(&pending, &f.clock.now()), Err(Error::Stale));
    assert!(f.host.pending.is_none());
    assert!(f.host.pending_enable.is_none());
    assert!(!f.host.permission);
}

#[test]
fn readiness_and_privacy_events_obey_native_transition_order() {
    let mut f = Fixture::new();
    assert_eq!(
        f.event(Event::MenuReady {
            keyboard: true,
            accessible: true
        }),
        Err(Error::InvalidTransition)
    );
    f.event(Event::Lock).unwrap();
    assert_eq!(f.event(Event::Lock), Err(Error::InvalidTransition));
    f.event(Event::Unlock).unwrap();
    assert_eq!(f.event(Event::Unlock), Err(Error::InvalidTransition));
    f.event(Event::Sleep).unwrap();
    assert_eq!(f.event(Event::Sleep), Err(Error::InvalidTransition));
    f.event(Event::Wake).unwrap();
    assert_eq!(f.event(Event::Wake), Err(Error::InvalidTransition));
    f.assert_off();
}

#[test]
fn foreign_altered_and_replayed_release_or_cleanup_evidence_is_inert() {
    let mut f = Fixture::new();
    let mut other = Fixture::new();
    f.authorize(combined());
    other.authorize(combined());
    let local = f.capture(MicrophoneOwner::ConversationTurn, true);
    let foreign = other.capture(MicrophoneOwner::ConversationTurn, true);
    f.host.quit(&f.control).unwrap();
    other.host.quit(&other.control).unwrap();
    assert_eq!(f.host.release_ack(&foreign, true, true), Err(Error::Stale));
    let mut altered = local.clone();
    altered.owner = MicrophoneOwner::WakeDetection;
    assert_eq!(f.host.release_ack(&altered, true, true), Err(Error::Stale));
    f.host.release_ack(&local, true, true).unwrap();
    other.host.release_ack(&foreign, true, true).unwrap();
    assert_eq!(f.host.release_ack(&local, true, true), Err(Error::Stale));
    let foreign_frames = other.host.frame_ticket().unwrap();
    assert_eq!(
        f.host.frames_ack(&foreign_frames, true, true),
        Err(Error::Stale)
    );
    let frames = f.host.frame_ticket().unwrap();
    f.host.frames_ack(&frames, true, true).unwrap();
    assert_eq!(f.host.frames_ack(&frames, true, true), Err(Error::Stale));
    let foreign_cleanup = other.host.cleanup_ticket().unwrap();
    assert_eq!(
        f.host
            .resource_ack(&foreign_cleanup, Resource::Workers, true),
        Err(Error::Stale)
    );
    let cleanup = f.host.cleanup_ticket().unwrap();
    f.host
        .resource_ack(&cleanup, Resource::Workers, true)
        .unwrap();
    assert_eq!(
        f.host.resource_ack(&cleanup, Resource::Workers, true),
        Err(Error::Stale)
    );
}

#[test]
fn cleanup_evidence_from_previous_enablement_cannot_finish_a_new_cycle() {
    let mut f = Fixture::new();
    f.authorize(combined());
    let capture = f.capture(MicrophoneOwner::WakeDetection, false);
    f.host.stop_listening(&f.control).unwrap();
    let old_cleanup = f.host.cleanup_ticket().unwrap();
    f.cleanup();
    f.authorize(combined());
    let current = f.capture(MicrophoneOwner::WakeDetection, true);
    f.host.stop_listening(&f.control).unwrap();
    assert_eq!(f.host.release_ack(&capture, true, true), Err(Error::Stale));
    assert_eq!(
        f.host.resource_ack(&old_cleanup, Resource::Workers, true),
        Err(Error::Stale)
    );
    f.host.release_ack(&current, true, true).unwrap();
    f.cleanup();
}

#[test]
fn clock_domain_failure_and_rollback_revoke_while_stop_quit_and_acks_remain_available() {
    for foreign in [false, true] {
        let mut f = Fixture::new();
        f.authorize(combined());
        f.capture(MicrophoneOwner::ConversationTurn, false);
        let old = f.clock.now();
        f.clock.advance(Duration::from_secs(1)).unwrap();
        f.host.observe(&f.clock.now()).unwrap();
        let bad = if foreign {
            ManualClock::new().now()
        } else {
            old
        };
        assert_eq!(
            f.host.observe(&bad),
            Err(Error::Contract(if foreign {
                ContractError::ClockDomainMismatch
            } else {
                ContractError::TimeReversed
            }))
        );
        assert!(f.host.faulted);
        f.host.stop_listening(&f.control).unwrap();
        f.host.quit(&f.control).unwrap();
        let termination = f.host.synthetic_observation(Event::HostTerminated).unwrap();
        f.host.apply(&termination, &bad).unwrap();
        f.cleanup();
        assert!(f.host.shutdown_complete());
        f.assert_off();
    }
}

#[test]
fn authenticated_privacy_notifications_revoke_even_when_the_clock_is_broken() {
    for event in [
        Event::Lock,
        Event::Sleep,
        Event::Logout,
        Event::PermissionLost,
        Event::HostTerminated,
    ] {
        let mut f = Fixture::new();
        f.authorize(combined());
        f.capture(MicrophoneOwner::WakeDetection, false);
        let receipt = f.host.synthetic_observation(event).unwrap();
        f.host.apply(&receipt, &ManualClock::new().now()).unwrap();
        assert!(f.host.authorization.is_none());
        assert_eq!(
            f.host.microphone_status().1,
            MicrophoneReleaseStatus::StoppingUnconfirmed
        );
        f.cleanup();
    }
}

#[test]
fn sequence_overflow_fails_closed_without_disabling_native_stop_or_quit() {
    let mut f = Fixture::new();
    f.authorize(combined());
    f.capture(MicrophoneOwner::WakeDetection, false);
    f.host.sequence = u64::MAX;
    assert!(matches!(
        f.host.synthetic_observation(Event::Focus),
        Err(Error::Contract(ContractError::Overflow))
    ));
    assert!(f.host.faulted);
    assert!(f.host.authorization.is_none());
    let epoch = f.host.epoch;
    f.host.stop_listening(&f.control).unwrap();
    f.host.quit(&f.control).unwrap();
    assert_eq!(f.host.sequence, u64::MAX);
    assert_eq!(f.host.epoch, epoch);
    f.cleanup();
    assert!(!f.host.shutdown_complete()); // no termination evidence fabricated
}

#[test]
fn final_sequence_is_accepted_once_without_wrapping_or_resetting_counters() {
    let mut f = Fixture::new();
    f.host.sequence = u64::MAX - 1;
    let last = f
        .host
        .synthetic_observation(Event::PermissionRestored)
        .unwrap();
    assert_eq!(last.stamp.sequence, u64::MAX);
    f.host.apply(&last, &f.clock.now()).unwrap();
    assert_eq!(f.host.apply(&last, &f.clock.now()), Err(Error::Stale));
    assert!(matches!(
        f.host.synthetic_observation(Event::Focus),
        Err(Error::Contract(ContractError::Overflow))
    ));
    assert!(f.host.authorization.is_none());
    assert_eq!(f.host.sequence, u64::MAX);
}

#[test]
fn generation_exhaustion_revokes_before_cleanup_and_never_reenables() {
    let mut f = Fixture::new();
    f.authorize(combined());
    f.capture(MicrophoneOwner::ConversationTurn, false);
    f.host.epoch = Generation::from_native_sequence(u64::MAX).unwrap();
    f.host.stop_listening(&f.control).unwrap();
    assert!(f.host.faulted);
    assert!(f.host.authorization.is_none());
    assert_eq!(
        f.host.microphone_status().1,
        MicrophoneReleaseStatus::StoppingUnconfirmed
    );
    f.host.quit(&f.control).unwrap();
    f.cleanup();
    f.event(Event::HostTerminated).unwrap();
    assert!(f.host.shutdown_complete());
    f.assert_off();
}

#[test]
fn generation_overflow_during_pause_or_selective_cancellation_fully_revokes() {
    for event in [Event::Pause, Event::EndConversation, Event::NavigateAway] {
        let mut f = Fixture::new();
        f.authorize(combined());
        let owner = if event == Event::NavigateAway {
            MicrophoneOwner::ManualForeground
        } else {
            MicrophoneOwner::ConversationTurn
        };
        f.capture(owner, true);
        let authority = f.host.authorization.as_ref().unwrap().alive.clone();
        f.host.epoch = Generation::from_native_sequence(u64::MAX).unwrap();
        f.event(event).unwrap();
        assert!(f.host.faulted);
        assert!(!authority.load(Ordering::Acquire));
        f.assert_off();
        assert_eq!(
            f.host.microphone_status().1,
            MicrophoneReleaseStatus::StoppingUnconfirmed
        );
        f.host.stop_listening(&f.control).unwrap();
        f.host.quit(&f.control).unwrap();
        f.cleanup();
    }
}

#[test]
fn deadline_overflow_and_exact_expiry_produce_no_authority_or_release_proof() {
    let mut f = Fixture::new();
    f.event(Event::PermissionRestored).unwrap();
    f.clock.advance(Duration::MAX).unwrap();
    assert!(matches!(
        f.host
            .prepare_enable(combined(), Duration::from_secs(1), &f.clock.now()),
        Err(Error::Contract(ContractError::Overflow))
    ));
    f.assert_off();
    let mut f = Fixture::new();
    f.authorize(combined());
    f.capture(MicrophoneOwner::WakeDetection, false);
    f.clock.advance(Duration::from_secs(20)).unwrap();
    f.assert_off();
    assert_eq!(
        f.host.microphone_status().1,
        MicrophoneReleaseStatus::StoppingUnconfirmed
    );
}

#[test]
fn native_enablement_preview_is_bounded_consumed_and_fresh_with_original_deadline() {
    let mut f = Fixture::new();
    f.event(Event::PermissionRestored).unwrap();
    let preview = f
        .host
        .prepare_enable(
            scope(MicrophoneOwner::WakeDetection, true),
            MAX_WAKE_ARMING,
            &f.clock.now(),
        )
        .unwrap();
    assert!(matches!(
        f.host
            .prepare_enable(combined(), Duration::from_secs(10), &f.clock.now()),
        Err(Error::Busy)
    ));
    f.clock.advance(Duration::from_secs(5 * 60)).unwrap();
    assert_eq!(
        f.host.enable(&f.control, &preview, &f.clock.now()),
        Err(Error::Unauthorized)
    );
    assert_eq!(
        f.host.enable(&f.control, &preview, &f.clock.now()),
        Err(Error::Stale)
    );
    assert!(f.host.authorization.is_none());
    let fresh = f.authorize(combined());
    assert_eq!(
        f.host.enable(&f.control, &fresh, &f.clock.now()),
        Err(Error::Stale)
    );
}

#[test]
fn invalid_scopes_and_excessive_lifetimes_cannot_escalate_host_eligibility() {
    let mut f = Fixture::new();
    f.event(Event::PermissionRestored).unwrap();
    let mut invalid = combined();
    invalid.microphone = false;
    assert!(matches!(
        f.host
            .prepare_enable(invalid, Duration::from_secs(10), &f.clock.now()),
        Err(Error::Unauthorized)
    ));
    invalid = scope(MicrophoneOwner::ManualForeground, false);
    invalid.background_wake = true;
    assert!(matches!(
        f.host
            .prepare_enable(invalid, Duration::from_secs(10), &f.clock.now()),
        Err(Error::Unauthorized)
    ));
    for owner in OWNERS {
        let scope = scope(owner, false);
        assert!(matches!(
            f.host.prepare_enable(
                scope,
                scope.maximum() + Duration::from_nanos(1),
                &f.clock.now()
            ),
            Err(Error::Unauthorized)
        ));
    }
    f.assert_off();
}

#[test]
fn accessible_native_labels_distinguish_listening_paused_cleanup_and_off() {
    for owner in OWNERS {
        let mut f = Fixture::new();
        f.menu();
        f.authorize(combined());
        assert_eq!(f.host.menu_status().voice_label.text(), "Off");
        let capture = f.capture(owner, true);
        assert_eq!(
            f.host.menu_status().voice_label.text(),
            "Starting/unconfirmed"
        );
        f.host.capture_ready(&capture, &f.clock.now()).unwrap();
        let expected = match owner {
            MicrophoneOwner::ManualForeground => "Manual recording",
            MicrophoneOwner::WakeDetection => "Listening locally",
            _ => "Conversation capture",
        };
        assert_eq!(f.host.menu_status().voice_label.text(), expected);
        f.event(Event::Pause).unwrap();
        assert_eq!(
            f.host.menu_status().voice_label.text(),
            "Stopping/unconfirmed"
        );
        f.host.release_ack(&capture, true, true).unwrap();
        assert_eq!(
            f.host.menu_status().voice_label.text(),
            "Input released; cleanup pending"
        );
        f.cleanup();
        assert_eq!(f.host.menu_status().voice_label.text(), "Paused");
        f.host.stop_listening(&f.control).unwrap();
        assert_eq!(f.host.menu_status().voice_label.text(), "Off");
    }
}

// FU01 regression fixtures. Original P1-11 tests above remain unchanged.
fn fu01_check_renderer_loss_cancels_manual(acquiring: bool) {
    let mut f = Fixture::new();
    f.menu();
    f.authorize(combined());
    let capture = f.capture(MicrophoneOwner::ManualForeground, acquiring);
    let authority = f.host.authorization.as_ref().unwrap().alive.clone();
    f.event(Event::RendererLost).unwrap();
    assert!(f.host.effects.cancel_manual);
    assert!(f.host.blocked_modes[0]);
    assert!(!f.host.renderer_available);
    assert!(!f.host.effects.revoke_voice);
    assert!(authority.load(Ordering::Acquire));
    assert_eq!(
        f.host.microphone_status(),
        (
            MicrophoneOwner::ManualForeground,
            MicrophoneReleaseStatus::StoppingUnconfirmed
        )
    );
    assert_eq!(
        f.host.capture_ready(&capture, &f.clock.now()),
        Err(Error::Stale)
    );
    assert!(f.host.frame_ticket().is_none());
    let mut foreign = capture.clone();
    foreign.stamp.nonce = Arc::new(());
    assert_eq!(f.host.release_ack(&foreign, true, true), Err(Error::Stale));
    assert_eq!(
        f.host.release_ack(&capture, false, true),
        Err(Error::CleanupUnconfirmed)
    );
    f.clock.advance(Duration::from_secs(2)).unwrap();
    f.host.observe(&f.clock.now()).unwrap();
    assert_eq!(
        f.host.microphone_status().1,
        MicrophoneReleaseStatus::StoppingUnconfirmed
    );
    f.host.release_ack(&capture, true, true).unwrap();
    assert_eq!(
        f.host.microphone_status().0,
        MicrophoneOwner::ManualForeground
    );
    assert!(!f.host.cleanup_complete());
    let frames = f.host.frame_ticket().unwrap();
    assert_eq!(
        f.host.frames_ack(&frames, true, false),
        Err(Error::CleanupUnconfirmed)
    );
    f.host.frames_ack(&frames, true, true).unwrap();
    assert_eq!(f.host.microphone_status().0, MicrophoneOwner::Off);
    assert!(f.host.cleanup_complete());
    assert_eq!(
        f.host.capture_ready(&capture, &f.clock.now()),
        Err(Error::Stale)
    );
    assert!(!f.eligible(MicrophoneOwner::ManualForeground));
    assert!(matches!(
        f.host
            .synthetic_capture(MicrophoneOwner::ManualForeground, true, &f.clock.now()),
        Err(Error::Unauthorized)
    ));
    assert!(f.eligible(MicrophoneOwner::WakeDetection));
    assert!(f.eligible(MicrophoneOwner::ConversationTurn));
    assert!(f.host.capture.is_none());
}

#[test]
fn fu01_renderer_loss_cancels_acquiring_manual_capture() {
    fu01_check_renderer_loss_cancels_manual(true);
}

#[test]
fn fu01_renderer_loss_cancels_active_manual_capture() {
    fu01_check_renderer_loss_cancels_manual(false);
}

#[test]
fn fu01_renderer_unavailable_blocks_new_manual_enablement_and_capture() {
    let mut f = Fixture::new();
    f.menu();
    f.authorize(combined());
    f.event(Event::RendererLost).unwrap();
    assert!(!f.eligible(MicrophoneOwner::ManualForeground));
    f.event(Event::ReopenWindow).unwrap();
    f.event(Event::Focus).unwrap();
    assert!(!f.eligible(MicrophoneOwner::ManualForeground));
    f.host.stop_listening(&f.control).unwrap();
    f.cleanup();
    for scope in [scope(MicrophoneOwner::ManualForeground, false), combined()] {
        assert!(matches!(
            f.host
                .prepare_enable(scope, Duration::from_secs(10), &f.clock.now()),
            Err(Error::Unauthorized)
        ));
    }
    assert!(matches!(
        f.host
            .synthetic_capture(MicrophoneOwner::ManualForeground, false, &f.clock.now()),
        Err(Error::Unauthorized)
    ));
}

#[test]
fn fu01_renderer_loss_invalidates_pending_manual_enablement() {
    let mut f = Fixture::new();
    f.menu();
    f.event(Event::PermissionRestored).unwrap();
    let preview = f
        .host
        .prepare_enable(combined(), Duration::from_secs(20), &f.clock.now())
        .unwrap();
    f.event(Event::RendererLost).unwrap();
    assert!(f.host.pending_enable.is_none());
    assert_eq!(
        f.host.enable(&f.control, &preview, &f.clock.now()),
        Err(Error::Stale)
    );
    f.assert_off();
}

#[test]
fn fu01_renderer_loss_preserves_independent_native_background_enablement() {
    let mut f = Fixture::new();
    f.menu();
    f.event(Event::PermissionRestored).unwrap();
    let preview = f
        .host
        .prepare_enable(
            scope(MicrophoneOwner::WakeDetection, true),
            Duration::from_secs(20),
            &f.clock.now(),
        )
        .unwrap();
    f.event(Event::RendererLost).unwrap();
    f.host.enable(&f.control, &preview, &f.clock.now()).unwrap();
    assert!(f.eligible(MicrophoneOwner::WakeDetection));
    assert!(!f.eligible(MicrophoneOwner::ManualForeground));
    assert!(f.host.capture.is_none());
}

#[test]
fn fu01_renderer_loss_preserves_scoped_background_jobs_and_original_deadlines() {
    for owner in [
        MicrophoneOwner::WakeDetection,
        MicrophoneOwner::ConversationTurn,
    ] {
        for acquiring in [false, true] {
            let mut f = Fixture::new();
            f.menu();
            let preview = f.authorize(scope(owner, true));
            let capture = f.capture(owner, acquiring);
            let epoch = f.host.epoch;
            f.clock.advance(Duration::from_secs(3)).unwrap();
            f.event(Event::RendererLost).unwrap();
            assert_eq!(f.host.epoch, epoch);
            if acquiring {
                f.host.capture_ready(&capture, &f.clock.now()).unwrap();
            }
            f.event(Event::HideWindow).unwrap();
            assert!(f.eligible(owner));
            assert_eq!(
                preview.0.deadline.remaining(&f.clock.now()).unwrap(),
                Duration::from_secs(17)
            );
            assert!(!f.host.effects.revoke_voice);
            assert!(f.host.menu_status().stop_reachable);
        }
    }
}

#[test]
fn fu01_renderer_loss_without_native_controls_fails_closed() {
    let mut f = Fixture::new();
    f.authorize(combined());
    f.capture(MicrophoneOwner::ConversationTurn, false);
    f.event(Event::RendererLost).unwrap();
    assert!(f.host.effects.revoke_voice);
    assert!(f.host.error_visible);
    assert_eq!(f.host.window, Window::Visible);
    assert_eq!(
        f.host.microphone_status().1,
        MicrophoneReleaseStatus::StoppingUnconfirmed
    );
    f.assert_off();
}

#[test]
fn fu01_renderer_loss_preempts_pending_focus_and_blocks_late_manual_start() {
    let mut f = Fixture::new();
    f.menu();
    f.authorize(combined());
    let capture = f.capture(MicrophoneOwner::ManualForeground, true);
    let focus = f.host.synthetic_observation(Event::Focus).unwrap();
    let lost = f
        .host
        .synthetic_observation(Event::RendererLost)
        .expect("renderer loss must preempt Focus");
    assert_eq!(f.host.apply(&focus, &f.clock.now()), Err(Error::Stale));
    f.host.apply(&lost, &f.clock.now()).unwrap();
    assert_eq!(
        f.host.capture_ready(&capture, &f.clock.now()),
        Err(Error::Stale)
    );
    f.cleanup();
    assert!(f.eligible(MicrophoneOwner::WakeDetection));
    assert!(!f.eligible(MicrophoneOwner::ManualForeground));
}

fn fu01_check_urgent_preempts_focus(event: Event) {
    let mut f = Fixture::new();
    f.menu();
    f.authorize(combined());
    let capture = f.capture(MicrophoneOwner::ConversationTurn, true);
    f.event(Event::Blur).unwrap();
    let authority = f.host.authorization.as_ref().unwrap().alive.clone();
    let focus = f.host.synthetic_observation(Event::Focus).unwrap();
    let urgent = f
        .host
        .synthetic_observation(event)
        .expect("urgent native notification must preempt Focus");
    assert_eq!(f.host.apply(&focus, &f.clock.now()), Err(Error::Stale));
    f.host.apply(&urgent, &f.clock.now()).unwrap();
    assert!(!authority.load(Ordering::Acquire));
    assert!(f.host.authorization.is_none());
    assert_eq!(
        f.host.microphone_status().1,
        MicrophoneReleaseStatus::StoppingUnconfirmed
    );
    assert_eq!(f.host.apply(&focus, &f.clock.now()), Err(Error::Stale));
    assert_eq!(f.host.apply(&urgent, &f.clock.now()), Err(Error::Stale));
    assert_eq!(
        f.host.capture_ready(&capture, &f.clock.now()),
        Err(Error::Stale)
    );
    assert!(!f.host.foreground);
    f.assert_off();
    f.cleanup();
    assert_eq!(f.host.shutdown_complete(), event == Event::HostTerminated);
}

#[test]
fn fu01_lock_preempts_pending_focus() {
    fu01_check_urgent_preempts_focus(Event::Lock);
}

#[test]
fn fu01_sleep_preempts_pending_focus() {
    fu01_check_urgent_preempts_focus(Event::Sleep);
}

#[test]
fn fu01_permission_loss_preempts_pending_focus() {
    fu01_check_urgent_preempts_focus(Event::PermissionLost);
}

#[test]
fn fu01_host_termination_preempts_pending_focus() {
    fu01_check_urgent_preempts_focus(Event::HostTerminated);
}

#[test]
fn fu01_other_native_privacy_barriers_preempt_pending_focus() {
    for event in [Event::Logout, Event::DeviceLost, Event::MenuLost] {
        fu01_check_urgent_preempts_focus(event);
    }
}

#[test]
fn fu01_urgent_preemption_keeps_all_native_identity_and_anti_forgery_checks() {
    let mut f = Fixture::new();
    f.authorize(combined());
    let focus = f.host.synthetic_observation(Event::Focus).unwrap();
    let mut forged = focus.clone();
    forged.event = Event::Lock;
    assert_eq!(f.host.apply(&forged, &f.clock.now()), Err(Error::Stale));
    let urgent = f.host.synthetic_observation(Event::Lock).unwrap();
    let mutations: [fn(&mut Stamp); 8] = [
        |s| s.binding.run = AppRunId::from_native_sequence(2).unwrap(),
        |s| s.binding.os_session = OsSessionId(NonZeroU64::new(2).unwrap()),
        |s| s.binding.host = HostId(NonZeroU64::new(2).unwrap()),
        |s| s.binding.generation = Generation::from_native_sequence(2).unwrap(),
        |s| s.epoch = Generation::from_native_sequence(2).unwrap(),
        |s| s.native = Arc::new(()),
        |s| s.nonce = Arc::new(()),
        |s| s.sequence = 0,
    ];
    for mutate in mutations {
        let mut forged = urgent.clone();
        mutate(&mut forged.stamp);
        assert_eq!(f.host.apply(&forged, &f.clock.now()), Err(Error::Stale));
        assert!(!f.host.locked);
        assert!(f.host.authorization.is_some());
    }
    let mut foreign = Fixture::new();
    let foreign_urgent = foreign.host.synthetic_observation(Event::Lock).unwrap();
    assert_eq!(
        f.host.apply(&foreign_urgent, &f.clock.now()),
        Err(Error::Stale)
    );
    let mut forged = urgent.clone();
    forged.event = Event::HostTerminated;
    assert_eq!(f.host.apply(&forged, &f.clock.now()), Err(Error::Stale));
    f.host.apply(&urgent, &f.clock.now()).unwrap();
    assert!(f.host.locked);
    assert_eq!(f.host.apply(&urgent, &f.clock.now()), Err(Error::Stale));
}

#[test]
fn fu01_preempted_permission_restore_cannot_restore_state_after_privacy_revocation() {
    let mut f = Fixture::new();
    f.menu();
    f.authorize(combined());
    let stale = f
        .host
        .synthetic_observation(Event::PermissionRestored)
        .unwrap();
    let lost = f.host.synthetic_observation(Event::PermissionLost).unwrap();
    f.host.apply(&lost, &f.clock.now()).unwrap();
    f.cleanup();
    assert_eq!(f.host.apply(&stale, &f.clock.now()), Err(Error::Stale));
    assert!(!f.host.permission);
    f.event(Event::PermissionRestored).unwrap();
    f.assert_off();
    f.authorize(scope(MicrophoneOwner::WakeDetection, true));
    assert_eq!(f.host.apply(&stale, &f.clock.now()), Err(Error::Stale));
    assert!(f.eligible(MicrophoneOwner::WakeDetection));
}

#[test]
fn fu01_urgent_preemption_at_sequence_exhaustion_revokes_without_wrapping() {
    let mut f = Fixture::new();
    f.authorize(combined());
    f.capture(MicrophoneOwner::ConversationTurn, false);
    f.host.sequence = u64::MAX - 1;
    let focus = f.host.synthetic_observation(Event::Focus).unwrap();
    assert!(matches!(
        f.host.synthetic_observation(Event::Lock),
        Err(Error::Contract(ContractError::Overflow))
    ));
    assert_eq!(f.host.sequence, u64::MAX);
    assert!(f.host.faulted);
    assert!(f.host.authorization.is_none());
    assert_eq!(f.host.apply(&focus, &f.clock.now()), Err(Error::Stale));
    assert_eq!(
        f.host.microphone_status().1,
        MicrophoneReleaseStatus::StoppingUnconfirmed
    );
    f.host.stop_listening(&f.control).unwrap();
    f.host.quit(&f.control).unwrap();
    f.cleanup();
    assert!(!f.host.shutdown_complete());
}

#[test]
fn fu01_urgent_preemption_at_generation_exhaustion_preserves_exact_cleanup() {
    let mut f = Fixture::new();
    f.authorize(combined());
    let capture = f.capture(MicrophoneOwner::ConversationTurn, true);
    f.host.epoch = Generation::from_native_sequence(u64::MAX).unwrap();
    let focus = f.host.synthetic_observation(Event::Focus).unwrap();
    let urgent = f.host.synthetic_observation(Event::Sleep).unwrap();
    f.host.apply(&urgent, &f.clock.now()).unwrap();
    assert!(f.host.sleeping && f.host.faulted);
    assert!(f.host.authorization.is_none());
    assert_eq!(f.host.apply(&focus, &f.clock.now()), Err(Error::Stale));
    f.host.release_ack(&capture, true, true).unwrap();
    f.cleanup();
    f.host.stop_listening(&f.control).unwrap();
    f.host.quit(&f.control).unwrap();
}

#[test]
fn fu01_urgent_preemption_and_renderer_cancellation_do_not_depend_on_clock_health() {
    for event in [
        Event::Lock,
        Event::Sleep,
        Event::PermissionLost,
        Event::HostTerminated,
        Event::RendererLost,
    ] {
        let mut f = Fixture::new();
        f.menu();
        f.authorize(combined());
        let capture = f.capture(MicrophoneOwner::ManualForeground, true);
        let focus = f.host.synthetic_observation(Event::Focus).unwrap();
        let urgent = f.host.synthetic_observation(event).unwrap();
        f.host.apply(&urgent, &ManualClock::new().now()).unwrap();
        assert_eq!(f.host.apply(&focus, &f.clock.now()), Err(Error::Stale));
        assert_eq!(
            f.host.capture_ready(&capture, &f.clock.now()),
            Err(Error::Stale)
        );
        assert_eq!(
            f.host.microphone_status().1,
            MicrophoneReleaseStatus::StoppingUnconfirmed
        );
        f.cleanup();
    }
}

#[test]
fn fu01_ordinary_events_cannot_displace_an_already_pending_privacy_notification() {
    let mut f = Fixture::new();
    let urgent = f.host.synthetic_observation(Event::Lock).unwrap();
    assert!(matches!(
        f.host.synthetic_observation(Event::Focus),
        Err(Error::Busy)
    ));
    assert!(matches!(
        f.host.synthetic_observation(Event::PermissionRestored),
        Err(Error::Busy)
    ));
    f.host.apply(&urgent, &f.clock.now()).unwrap();
    assert!(f.host.locked);
}

// FU02: a collision is a native delivery fault, never another event queue.
fn fu02_check_renderer_collision(next: Event) {
    for owner in [
        MicrophoneOwner::WakeDetection,
        MicrophoneOwner::ConversationTurn,
    ] {
        for acquiring in [false, true] {
            let mut f = Fixture::new();
            f.menu();
            f.authorize(scope(owner, true));
            let capture = f.capture(owner, acquiring);
            let authority = f.host.authorization.as_ref().unwrap().alive.clone();
            let pending = f.host.synthetic_observation(Event::RendererLost).unwrap();
            let sequence = f.host.sequence;
            assert!(f.host.synthetic_observation(next).is_err());
            assert!(
                !authority.load(Ordering::Acquire),
                "urgent collision retained voice authority"
            );
            assert!(f.host.faulted && f.host.error_visible);
            assert!(f.host.effects.revoke_voice && f.host.effects.revoke_egress);
            assert!(f.host.pending.is_none());
            assert_eq!(f.host.sequence, sequence);
            assert_eq!(f.host.apply(&pending, &f.clock.now()), Err(Error::Stale));
            assert_eq!(
                f.host.capture_ready(&capture, &f.clock.now()),
                Err(Error::Stale)
            );
            assert_eq!(
                f.host.microphone_status(),
                (owner, MicrophoneReleaseStatus::StoppingUnconfirmed)
            );
            f.assert_off();
            f.cleanup();
            f.assert_off();
        }
    }
}

#[test]
fn fu02_pending_renderer_loss_followed_by_lock_revokes_immediately() {
    fu02_check_renderer_collision(Event::Lock);
}

#[test]
fn fu02_pending_renderer_loss_followed_by_sleep_revokes_immediately() {
    fu02_check_renderer_collision(Event::Sleep);
}

#[test]
fn fu02_pending_renderer_loss_followed_by_permission_loss_revokes_immediately() {
    fu02_check_renderer_collision(Event::PermissionLost);
}

#[test]
fn fu02_pending_lock_followed_by_termination_keeps_shutdown_unconfirmed() {
    let mut f = Fixture::new();
    f.menu();
    f.authorize(combined());
    f.capture(MicrophoneOwner::WakeDetection, false);
    let pending = f.host.synthetic_observation(Event::Lock).unwrap();
    assert!(f.host.synthetic_observation(Event::HostTerminated).is_err());
    assert_eq!(f.host.application, Application::ShuttingDown);
    assert!(f.host.effects.terminate_host && f.host.effects.terminate_workers);
    assert!(f.host.faulted && f.host.error_visible);
    assert!(!f.host.shutdown_complete());
    assert_eq!(f.host.apply(&pending, &f.clock.now()), Err(Error::Stale));
    f.cleanup();
    assert!(!f.host.shutdown_complete());
    // Collision supplied no accepted termination receipt. A fresh exact native
    // receipt is still required, independently of local resource ACKs.
    f.event(Event::HostTerminated).unwrap();
    assert!(f.host.shutdown_complete());
    f.assert_off();
}

#[test]
fn fu02_pending_permission_loss_cannot_be_replaced_by_less_restrictive_notifications() {
    for next in [
        Event::RendererLost,
        Event::MenuLost,
        Event::DeviceLost,
        Event::Sleep,
    ] {
        let mut f = Fixture::new();
        f.menu();
        f.authorize(combined());
        f.capture(MicrophoneOwner::ConversationTurn, false);
        let pending = f.host.synthetic_observation(Event::PermissionLost).unwrap();
        assert!(f.host.synthetic_observation(next).is_err());
        f.assert_off();
        assert!(f.host.faulted);
        assert_eq!(f.host.apply(&pending, &f.clock.now()), Err(Error::Stale));
        f.cleanup();
        // Subsequent correctly issued, less restrictive events cannot un-fault
        // the run or manufacture replacement authority.
        f.event(Event::RendererLost).unwrap();
        f.event(Event::PermissionRestored).unwrap();
        f.assert_off();
    }
}

#[test]
fn fu02_all_urgent_collision_orders_fault_without_adding_a_pending_slot() {
    let urgent = [
        Event::RendererLost,
        Event::Lock,
        Event::Sleep,
        Event::Logout,
        Event::PermissionLost,
        Event::DeviceLost,
        Event::MenuLost,
        Event::HostTerminated,
    ];
    for first in urgent {
        for second in urgent {
            let mut f = Fixture::new();
            f.menu();
            f.authorize(combined());
            f.capture(MicrophoneOwner::WakeDetection, false);
            let pending = f.host.synthetic_observation(first).unwrap();
            let sequence = f.host.sequence;
            assert!(f.host.synthetic_observation(second).is_err());
            assert!(
                f.host.faulted,
                "collision {first:?} then {second:?} was not fail-closed"
            );
            assert!(f.host.error_visible);
            assert!(f.host.pending.is_none());
            assert_eq!(f.host.sequence, sequence);
            assert_eq!(f.host.apply(&pending, &f.clock.now()), Err(Error::Stale));
            f.assert_off();
            assert!(!f.host.shutdown_complete());
            if first == Event::HostTerminated || second == Event::HostTerminated {
                assert_eq!(f.host.application, Application::ShuttingDown);
                assert!(f.host.effects.terminate_host);
            }
        }
    }
}

#[test]
fn fu02_foreign_and_forged_urgent_receipts_cannot_trigger_or_bypass_collision_handling() {
    let mut f = Fixture::new();
    f.menu();
    f.authorize(scope(MicrophoneOwner::WakeDetection, true));
    let pending = f.host.synthetic_observation(Event::RendererLost).unwrap();
    let mutations: [fn(&mut Stamp); 8] = [
        |s| s.binding.run = AppRunId::from_native_sequence(2).unwrap(),
        |s| s.binding.os_session = OsSessionId(NonZeroU64::new(2).unwrap()),
        |s| s.binding.host = HostId(NonZeroU64::new(2).unwrap()),
        |s| s.binding.generation = Generation::from_native_sequence(2).unwrap(),
        |s| s.epoch = Generation::from_native_sequence(2).unwrap(),
        |s| s.sequence = 0,
        |s| s.native = Arc::new(()),
        |s| s.nonce = Arc::new(()),
    ];
    for mutate in mutations {
        let mut forged = pending.clone();
        mutate(&mut forged.stamp);
        assert_eq!(f.host.apply(&forged, &f.clock.now()), Err(Error::Stale));
    }
    for event in [Event::Lock, Event::HostTerminated] {
        let mut forged = pending.clone();
        forged.event = event;
        assert_eq!(f.host.apply(&forged, &f.clock.now()), Err(Error::Stale));
        let mut foreign = Fixture::new();
        let receipt = foreign.host.synthetic_observation(event).unwrap();
        assert_eq!(f.host.apply(&receipt, &f.clock.now()), Err(Error::Stale));
    }
    assert!(!f.host.faulted);
    assert!(f.eligible(MicrophoneOwner::WakeDetection));
    assert_eq!(f.host.application, Application::Running);
    assert!(f.host.synthetic_observation(Event::Lock).is_err());
    f.assert_off();
    assert!(f.host.faulted);
    assert_eq!(f.host.apply(&pending, &f.clock.now()), Err(Error::Stale));
    let fresh = f.host.synthetic_observation(Event::RendererLost).unwrap();
    assert_eq!(f.host.apply(&pending, &f.clock.now()), Err(Error::Stale));
    f.host.apply(&fresh, &f.clock.now()).unwrap();
    assert_eq!(f.host.apply(&fresh, &f.clock.now()), Err(Error::Stale));
    f.assert_off();
}

#[test]
fn fu02_collision_invalidates_prepared_native_enablement_before_confirmation() {
    let mut f = Fixture::new();
    f.menu();
    f.event(Event::PermissionRestored).unwrap();
    let preview = f
        .host
        .prepare_enable(
            scope(MicrophoneOwner::WakeDetection, true),
            Duration::from_secs(20),
            &f.clock.now(),
        )
        .unwrap();
    let pending = f.host.synthetic_observation(Event::RendererLost).unwrap();
    assert!(f.host.synthetic_observation(Event::Lock).is_err());
    assert!(f.host.pending_enable.is_none());
    assert!(f.host.faulted);
    assert_eq!(
        f.host.enable(&f.control, &preview, &f.clock.now()),
        Err(Error::Stale)
    );
    assert_eq!(f.host.apply(&pending, &f.clock.now()), Err(Error::Stale));
    f.cleanup();
    assert!(matches!(
        f.host.prepare_enable(
            scope(MicrophoneOwner::WakeDetection, true),
            Duration::from_secs(20),
            &f.clock.now()
        ),
        Err(Error::Closed)
    ));
    f.assert_off();
}

#[test]
fn fu02_collision_shutdown_requires_separate_release_frames_resources_and_termination_evidence() {
    let mut f = Fixture::new();
    f.authorize(combined());
    let capture = f.capture(MicrophoneOwner::ConversationTurn, true);
    let pending = f.host.synthetic_observation(Event::Lock).unwrap();
    assert!(f.host.synthetic_observation(Event::HostTerminated).is_err());
    assert_eq!(f.host.application, Application::ShuttingDown);
    f.clock.advance(Duration::from_secs(100)).unwrap();
    f.host.observe(&f.clock.now()).unwrap();
    assert_eq!(
        f.host.microphone_status().1,
        MicrophoneReleaseStatus::StoppingUnconfirmed
    );
    assert!(f.host.frame_ticket().is_none());
    assert_eq!(
        f.host.release_ack(&capture, false, true),
        Err(Error::CleanupUnconfirmed)
    );
    f.host.release_ack(&capture, true, true).unwrap();
    let frames = f.host.frame_ticket().unwrap();
    assert_eq!(
        f.host.frames_ack(&frames, true, false),
        Err(Error::CleanupUnconfirmed)
    );
    assert_eq!(
        f.host.microphone_status().0,
        MicrophoneOwner::ConversationTurn
    );
    let termination = f.host.synthetic_observation(Event::HostTerminated).unwrap();
    let mut forged = termination.clone();
    forged.stamp.nonce = Arc::new(());
    assert_eq!(f.host.apply(&forged, &f.clock.now()), Err(Error::Stale));
    assert_eq!(f.host.application, Application::ShuttingDown);
    f.host.apply(&termination, &f.clock.now()).unwrap();
    assert!(!f.host.shutdown_complete());
    f.host.frames_ack(&frames, true, true).unwrap();
    assert!(!f.host.shutdown_complete());
    let cleanup = f.host.cleanup_ticket().unwrap();
    f.host
        .resource_ack(&cleanup, Resource::Output, true)
        .unwrap();
    f.host
        .resource_ack(&cleanup, Resource::Content, true)
        .unwrap();
    assert_eq!(
        f.host.resource_ack(&cleanup, Resource::Workers, false),
        Err(Error::CleanupUnconfirmed)
    );
    assert!(!f.host.shutdown_complete());
    f.host
        .resource_ack(&cleanup, Resource::Workers, true)
        .unwrap();
    assert!(f.host.shutdown_complete());
    assert!(f.host.faulted);
    assert_eq!(f.host.apply(&pending, &f.clock.now()), Err(Error::Stale));
}

#[test]
fn fu02_collision_at_counter_exhaustion_does_not_need_a_new_sequence_or_generation() {
    let mut f = Fixture::new();
    f.menu();
    f.authorize(combined());
    let capture = f.capture(MicrophoneOwner::WakeDetection, true);
    f.host.epoch = Generation::from_native_sequence(u64::MAX).unwrap();
    f.host.sequence = u64::MAX - 1;
    let pending = f.host.synthetic_observation(Event::RendererLost).unwrap();
    assert!(f.host.synthetic_observation(Event::HostTerminated).is_err());
    assert!(f.host.authorization.is_none());
    assert!(f.host.faulted && f.host.error_visible);
    assert_eq!(f.host.sequence, u64::MAX);
    assert_eq!(
        f.host.epoch,
        Generation::from_native_sequence(u64::MAX).unwrap()
    );
    assert_eq!(f.host.application, Application::ShuttingDown);
    assert_eq!(f.host.apply(&pending, &f.clock.now()), Err(Error::Stale));
    f.host.release_ack(&capture, true, true).unwrap();
    f.cleanup();
    f.host.stop_listening(&f.control).unwrap();
    f.host.quit(&f.control).unwrap();
    assert!(matches!(
        f.host.synthetic_observation(Event::HostTerminated),
        Err(Error::Contract(ContractError::Overflow))
    ));
    assert!(!f.host.shutdown_complete());
}

#[test]
fn fu02_collision_after_clock_failure_retains_unconfirmed_termination_without_clock_reads() {
    let mut f = Fixture::new();
    f.authorize(combined());
    f.capture(MicrophoneOwner::ConversationTurn, false);
    let bad_time = ManualClock::new().now();
    assert!(f.host.observe(&bad_time).is_err());
    let pending = f.host.synthetic_observation(Event::Lock).unwrap();
    assert!(f.host.synthetic_observation(Event::HostTerminated).is_err());
    assert_eq!(f.host.application, Application::ShuttingDown);
    assert!(f.host.error_visible && f.host.faulted);
    assert_eq!(f.host.apply(&pending, &bad_time), Err(Error::Stale));
    let termination = f.host.synthetic_observation(Event::HostTerminated).unwrap();
    f.host.apply(&termination, &bad_time).unwrap();
    f.cleanup();
    assert!(f.host.shutdown_complete());
}

#[test]
fn fu02_collision_fault_cannot_be_cleared_by_passive_events_or_permission_renewal() {
    let mut f = Fixture::new();
    f.menu();
    f.authorize(combined());
    let pending = f.host.synthetic_observation(Event::PermissionLost).unwrap();
    assert!(f.host.synthetic_observation(Event::RendererLost).is_err());
    assert!(f.host.faulted);
    f.cleanup();
    for event in [
        Event::PermissionRestored,
        Event::ReopenWindow,
        Event::Focus,
        Event::RendererLost,
    ] {
        f.event(event).unwrap();
        f.assert_off();
    }
    assert_eq!(f.host.apply(&pending, &f.clock.now()), Err(Error::Stale));
    assert_eq!(
        f.host.resume(&f.control, &f.clock.now()),
        Err(Error::Unauthorized)
    );
    assert!(matches!(
        f.host.prepare_enable(
            scope(MicrophoneOwner::WakeDetection, true),
            Duration::from_secs(20),
            &f.clock.now()
        ),
        Err(Error::Closed)
    ));
    assert!(matches!(
        f.host
            .synthetic_capture(MicrophoneOwner::WakeDetection, false, &f.clock.now()),
        Err(Error::Unauthorized)
    ));
}

#[test]
fn fu02_collision_preserves_existing_release_and_frame_cleanup_identities() {
    for cleaning in [false, true] {
        let mut f = Fixture::new();
        f.authorize(combined());
        let capture = f.capture(MicrophoneOwner::ManualForeground, true);
        f.host.stop_listening(&f.control).unwrap();
        let cleanup = f.host.cleanup_ticket().unwrap();
        if cleaning {
            f.host.release_ack(&capture, true, true).unwrap();
        }
        let frame_ticket = f.host.frame_ticket();
        let pending = f.host.synthetic_observation(Event::PermissionLost).unwrap();
        assert!(f.host.synthetic_observation(Event::RendererLost).is_err());
        assert!(f.host.faulted);
        assert!(f.host.cleanup_ticket().unwrap().0.matches(&cleanup.0));
        assert_eq!(f.host.apply(&pending, &f.clock.now()), Err(Error::Stale));
        if let Some(frames) = frame_ticket {
            assert_eq!(
                f.host.microphone_status().1,
                MicrophoneReleaseStatus::Acknowledged
            );
            f.host.frames_ack(&frames, true, true).unwrap();
        } else {
            assert_eq!(
                f.host.microphone_status().1,
                MicrophoneReleaseStatus::StoppingUnconfirmed
            );
            f.host.release_ack(&capture, true, true).unwrap();
        }
        f.cleanup();
        f.assert_off();
    }
}
