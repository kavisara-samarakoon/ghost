use super::*;
use serde_json::json;
use std::cell::{Cell, RefCell};
use std::io::{self, Cursor};

const FAKE_KEY: &str = "fake-test-credential-only";
fn audio() -> Audio {
    validate(
        "main",
        &InvokeBody::Raw(vec![1, 2, 3]),
        Some("audio/webm"),
        Some("1200"),
        Some(CONFIRMED),
    )
    .unwrap()
}
fn parsed(text: &str) -> String {
    parse_response(
        200,
        None,
        Cursor::new(serde_json::to_vec(&json!({"text": text})).unwrap()),
    )
    .unwrap()
}
#[test]
fn wrong_window_body_confirmation_are_rejected_before_any_side_effect() {
    let raw = InvokeBody::Raw(vec![1]);
    let cases = [
        ("other", &raw, Some(CONFIRMED)),
        ("main", &InvokeBody::Json(json!([1, 2])), Some(CONFIRMED)),
        ("main", &raw, None),
        ("main", &raw, Some("SEND-TO-OPENAI")),
        ("main", &raw, Some("send-to-openai ")),
    ];
    for (window, body, marker) in cases {
        let audit = Cell::new(0);
        let lookup = Cell::new(0);
        let network = Cell::new(0);
        let outcome =
            validate(window, body, Some("audio/webm"), Some("1200"), marker).and_then(|audio| {
                transcribe_with(
                    audio,
                    |_| {
                        audit.set(audit.get() + 1);
                        Ok(())
                    },
                    || {
                        lookup.set(lookup.get() + 1);
                        Some(FAKE_KEY.into())
                    },
                    |_, _| {
                        network.set(network.get() + 1);
                        Ok("unused".into())
                    },
                )
            });
        assert!(outcome.is_err());
        assert_eq!((audit.get(), lookup.get(), network.get()), (0, 0, 0));
    }
}
#[test]
fn native_concurrency_lease_rejects_overlap_and_releases_on_drop() {
    let lease = VoiceLease::acquire().unwrap();
    assert!(matches!(VoiceLease::acquire(), Err("busy")));
    drop(lease);
    assert!(VoiceLease::acquire().is_ok());
}
#[test]
fn raw_audio_size_is_authoritatively_bounded() {
    for size in [0, MAX_AUDIO_BYTES + 1] {
        assert!(validate(
            "main",
            &InvokeBody::Raw(vec![1; size]),
            Some("audio/webm"),
            Some("1"),
            Some(CONFIRMED)
        )
        .is_err());
    }
    assert!(validate(
        "main",
        &InvokeBody::Raw(vec![1; MAX_AUDIO_BYTES]),
        Some("audio/mp4"),
        Some("30000"),
        Some(CONFIRMED)
    )
    .is_ok());
}
#[test]
fn metadata_is_bounded_and_exact() {
    for mime in [
        None,
        Some("audio/wav"),
        Some("audio/webm\r\nx:evil"),
        Some("https://elsewhere.invalid"),
        Some("audio/webm;filename=../../other"),
        Some("audio/webm "),
    ] {
        assert!(validate(
            "main",
            &InvokeBody::Raw(vec![1]),
            mime,
            Some("1"),
            Some(CONFIRMED)
        )
        .is_err());
    }
    for duration in [
        None,
        Some(""),
        Some("0"),
        Some("-1"),
        Some("1.5"),
        Some("30001"),
        Some("123456"),
        Some(" 10"),
        Some("1\n"),
        Some("NaN"),
    ] {
        assert!(validate(
            "main",
            &InvokeBody::Raw(vec![1]),
            Some("audio/webm"),
            duration,
            Some(CONFIRMED)
        )
        .is_err());
    }
}
#[test]
fn every_mime_has_a_fixed_filename_and_category() {
    for (mime, filename, category) in [
        ("audio/webm;codecs=opus", "voice.webm", "webm"),
        ("audio/webm", "voice.webm", "webm"),
        ("audio/mp4", "voice.mp4", "mp4"),
        ("audio/ogg;codecs=opus", "voice.ogg", "ogg"),
        ("audio/ogg", "voice.ogg", "ogg"),
    ] {
        let validated = validate(
            "main",
            &InvokeBody::Raw(vec![1]),
            Some(mime),
            Some("100"),
            Some(CONFIRMED),
        )
        .unwrap();
        assert_eq!(validated.filename, filename);
        assert_eq!(validated.mime, mime);
        assert_eq!(validated.category, category);
    }
}
#[test]
fn credential_unavailable_or_invalid_never_invokes_transport() {
    for key in [
        None,
        Some(""),
        Some("   "),
        Some("bad\r\nheader"),
        Some("bad\0header"),
        Some("é"),
        Some("space key"),
    ] {
        let calls = Cell::new(0);
        let audits = Cell::new(0);
        let outcome = transcribe_with(
            audio(),
            |_| {
                audits.set(audits.get() + 1);
                Ok(())
            },
            || key.map(str::to_owned),
            |_, _| {
                calls.set(calls.get() + 1);
                Ok("unused".into())
            },
        );
        assert!(matches!(outcome, Err("credential")));
        assert_eq!(calls.get(), 0);
        assert_eq!(audits.get(), 1);
    }
    assert!(credential(Some("x".repeat(1025))).is_err());
}
#[test]
fn confirmed_audit_then_credential_then_one_transport_then_completion_audit() {
    let sequence = RefCell::new(Vec::new());
    let metadata = RefCell::new(Vec::new());
    let outcome = transcribe_with(
        audio(),
        |event| {
            sequence.borrow_mut().push(event.result);
            metadata
                .borrow_mut()
                .push(serde_json::to_value(event).unwrap());
            Ok(())
        },
        || {
            sequence.borrow_mut().push("credential");
            Some(FAKE_KEY.into())
        },
        |recording, _| {
            sequence.borrow_mut().push("transport");
            assert_eq!(recording.bytes, vec![1, 2, 3]);
            Ok(parsed("Ordinary transcript"))
        },
    )
    .unwrap();
    assert_eq!(
        *sequence.borrow(),
        ["confirmed", "credential", "transport", "completed"]
    );
    assert!(outcome.audit_recorded);
    assert_eq!(outcome.text, "Ordinary transcript");
    for event in metadata.borrow().iter() {
        let serialized = event.to_string();
        assert!(!serialized.contains(FAKE_KEY));
        assert!(!serialized.contains("Ordinary transcript"));
        assert!(event.get("audio").is_none());
        assert!(event.get("text").is_none());
        assert_eq!(event["model"], MODEL);
        assert_eq!(event["audio_bytes"], 3);
        assert_eq!(event["duration_ms"], 1200);
        assert_eq!(
            event.as_object().unwrap().len(),
            if event["result"] == "completed" { 8 } else { 7 }
        );
    }
    assert!(metadata.borrow()[0].get("transcript_bytes").is_none());
    assert_eq!(metadata.borrow()[1]["transcript_bytes"], outcome.text.len());
}
#[test]
fn confirmed_audit_failure_blocks_credential_and_transport() {
    let outcome = transcribe_with(
        audio(),
        |_| Err("PRIVATE_AUDIT_ERROR"),
        || panic!("Credential must not be looked up"),
        |_, _| panic!("Transport must not be invoked"),
    );
    assert!(matches!(outcome, Err("audit")));
}
#[test]
fn completion_audit_failure_preserves_transcript_and_does_not_retry() {
    let network = Cell::new(0);
    let audits = Cell::new(0);
    let outcome = transcribe_with(
        audio(),
        |_| {
            audits.set(audits.get() + 1);
            if audits.get() == 2 {
                Err("audit")
            } else {
                Ok(())
            }
        },
        || Some(FAKE_KEY.into()),
        |_, _| {
            network.set(network.get() + 1);
            Ok("Keep this transcript".into())
        },
    )
    .unwrap();
    assert_eq!(outcome.text, "Keep this transcript");
    assert!(!outcome.audit_recorded);
    assert_eq!(network.get(), 1);
    assert_eq!(audits.get(), 2);
}
#[test]
fn transport_failure_is_terminal_and_completion_audit_is_not_written() {
    let network = Cell::new(0);
    let events = RefCell::new(Vec::new());
    let outcome = transcribe_with(
        audio(),
        |event| {
            events.borrow_mut().push(event.event);
            Ok(())
        },
        || Some(FAKE_KEY.into()),
        |_, _| {
            network.set(network.get() + 1);
            Err(TRANSPORT_ERROR)
        },
    );
    assert!(matches!(outcome, Err("transport")));
    assert_eq!(network.get(), 1);
    assert_eq!(*events.borrow(), ["desktop.voice_transcription.confirmed"]);
}
#[test]
fn invalid_transcript_never_writes_completion_audit() {
    let count = Cell::new(0);
    assert!(transcribe_with(
        audio(),
        |_| {
            count.set(count.get() + 1);
            Ok(())
        },
        || Some(FAKE_KEY.into()),
        |_, _| Ok("\u{200b}\u{202e}\0".into())
    )
    .is_err());
    assert_eq!(count.get(), 1);
}
#[test]
fn production_request_has_exact_fixed_destination_and_multipart_fields() {
    let client = client().unwrap();
    let input = audio();
    let mut request = build_request(&client, &input, FAKE_KEY).unwrap();
    assert_eq!(request.method(), reqwest::Method::POST);
    assert_eq!(request.url().as_str(), ENDPOINT);
    assert_eq!(request.url().scheme(), "https");
    assert_eq!(request.url().host_str(), Some("api.openai.com"));
    assert_eq!(request.url().port_or_known_default(), Some(443));
    assert_eq!(request.url().path(), "/v1/audio/transcriptions");
    let header = request
        .headers()
        .get(reqwest::header::AUTHORIZATION)
        .unwrap();
    assert!(header.is_sensitive());
    assert!(header.to_str().unwrap().starts_with("Bearer "));
    let bytes = request.body_mut().as_mut().unwrap().buffer().unwrap();
    let body = String::from_utf8_lossy(bytes);
    assert_eq!(body.matches("Content-Disposition: form-data;").count(), 3);
    for expected in [
        "name=\"file\"; filename=\"voice.webm\"",
        "name=\"model\"",
        "gpt-transcribe",
        "name=\"response_format\"",
        "json",
    ] {
        assert!(body.contains(expected));
    }
    for forbidden in [
        FAKE_KEY,
        "name=\"prompt\"",
        "name=\"tools\"",
        "name=\"instructions\"",
        "name=\"project\"",
    ] {
        assert!(!body.contains(forbidden));
    }
}
#[test]
fn network_policy_explicitly_disables_redirects_proxies_retries_and_preserves_tls() {
    let source = include_str!("../voice.rs");
    for required in [
        "https_only(true)",
        "Policy::none()",
        "no_proxy()",
        "retry::never()",
        "from_secs(10)",
        "from_secs(45)",
    ] {
        assert!(source.contains(required));
    }
    assert!(!source.contains("danger_accept_invalid"));
    assert_eq!(source.matches("client.execute(request)").count(), 1);
}
struct NoRead;
impl Read for NoRead {
    fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
        panic!("Rejected response must never be read")
    }
}
#[test]
fn response_read_timeout_or_failure_reports_only_safe_ambiguity_category() {
    struct Failure(io::ErrorKind);
    impl Read for Failure {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            Err(io::Error::new(self.0, "PRIVATE_RAW_ERROR"))
        }
    }
    for kind in [io::ErrorKind::TimedOut, io::ErrorKind::ConnectionReset] {
        assert!(matches!(
            parse_response(200, None, Failure(kind)),
            Err("transport")
        ));
    }
}
#[test]
fn http_errors_and_redirects_are_rejected_without_reading_or_exposing_body() {
    for status in [301, 302, 307, 308, 400, 401, 429, 500, 503] {
        assert!(matches!(
            parse_response(status, None, NoRead),
            Err("service")
        ));
    }
}
#[test]
fn oversized_content_length_rejected_before_read() {
    assert!(matches!(
        parse_response(200, Some(MAX_RESPONSE_BYTES as u64 + 1), NoRead),
        Err("transcript")
    ));
}
#[test]
fn lengthless_and_underdeclared_oversized_body_are_bounded() {
    for length in [None, Some(1)] {
        let mut cursor = Cursor::new(vec![b' '; MAX_RESPONSE_BYTES + 100]);
        assert!(matches!(
            parse_response(200, length, &mut cursor),
            Err("transcript")
        ));
        assert_eq!(cursor.position(), MAX_RESPONSE_BYTES as u64 + 1);
    }
}
#[test]
fn malformed_utf8_json_top_level_or_text_rejected_safely() {
    for body in [
        b"\xff".as_slice(),
        b"{",
        b"[]",
        b"null",
        b"true",
        b"{}",
        br#"{"text":null}"#,
        br#"{"text":1}"#,
        br#"{"text":[]}"#,
    ] {
        assert!(matches!(
            parse_response(200, None, Cursor::new(body)),
            Err("transcript")
        ));
    }
    assert_eq!(parsed("Résumé 中文 🎙️"), "Résumé 中文 🎙️");
}
#[test]
fn huge_transcript_rejected_before_returning_to_frontend() {
    let body =
        serde_json::to_vec(&json!({"text":"é".repeat(MAX_TRANSCRIPT_BYTES / 2 + 1)})).unwrap();
    assert!(parse_response(200, None, Cursor::new(body)).is_err());
}
#[test]
fn sanitization_strips_format_controls_and_terminal_escape_spoofing() {
    for character in ['\u{200b}', '\u{202e}', '\u{2066}', '\u{2069}'] {
        let clean = sanitize(&format!("Résumé 中文{character}"), FAKE_KEY).unwrap();
        assert_eq!(clean, "Résumé 中文");
    }
    assert_eq!(
        sanitize(
            "\x1b[31mVisible\x1b[0m\x1b]8;;https://unsafe\x07\0\u{009b}\r\n\ttext",
            FAKE_KEY
        )
        .unwrap(),
        "Visible\n\ttext"
    );
    for empty in ["", " \n\t", "\u{200b}\u{202e}", "\0\u{009b}"] {
        assert!(sanitize(empty, FAKE_KEY).is_err());
    }
    let clean = sanitize(&format!("Provider returned {FAKE_KEY}"), FAKE_KEY).unwrap();
    assert!(!clean.contains(FAKE_KEY));
}
#[test]
fn frontend_result_contains_only_sanitized_text_and_safe_metadata() {
    let outcome = transcribe_with(
        audio(),
        |_| Ok(()),
        || Some(FAKE_KEY.into()),
        |_, _| Ok(format!("Résumé 中文\u{202e} {FAKE_KEY}")),
    )
    .unwrap();
    let value = serde_json::to_value(outcome).unwrap();
    assert_eq!(value.as_object().unwrap().len(), 5);
    assert!(!value.to_string().contains(FAKE_KEY));
    assert!(!value["text"].as_str().unwrap().contains('\u{202e}'));
    assert_eq!(value["model"], MODEL);
}
#[test]
fn voice_boundary_has_no_workflow_files_or_execution_imports() {
    for source in [include_str!("../voice.rs"), include_str!("storage.rs")] {
        for forbidden in [
            "Command::",
            "std::process",
            "subprocess",
            "shell",
            "orchestration",
            "requests::",
            "action-requests",
            "projects.yaml",
            "project.yaml",
            "std::net",
            "Responses",
            "base64",
            "tempfile",
            "create_handoff",
            "start_session",
            "add_note",
        ] {
            assert!(
                !source.contains(forbidden),
                "Forbidden boundary: {forbidden}"
            );
        }
    }
}

#[cfg(unix)]
mod audit_storage {
    use super::super::storage::AuditStore;
    use super::*;
    use std::fs;
    use std::os::unix::fs::{symlink, MetadataExt, PermissionsExt};
    fn fixture() -> (tempfile::TempDir, std::path::PathBuf) {
        let root = tempfile::tempdir().unwrap();
        let home = root.path().canonicalize().unwrap().join("ghost-home");
        (root, home)
    }
    fn event() -> AuditEvent {
        AuditEvent::new(&audio(), None).unwrap()
    }
    #[test]
    fn only_private_content_free_audit_is_created_and_file_parent_are_synced() {
        let (_root, home) = fixture();
        let store = AuditStore::open(&home).unwrap();
        let checkpoints = RefCell::new(Vec::new());
        store
            .test_append(event(), |phase| {
                checkpoints.borrow_mut().push(phase.to_owned());
                Ok(())
            })
            .unwrap();
        assert_eq!(
            *checkpoints.borrow(),
            ["opened", "file-synced", "parent-synced"]
        );
        assert_eq!(fs::read_dir(&home).unwrap().count(), 1);
        let path = home.join("desktop-voice-audit.jsonl");
        let metadata = path.metadata().unwrap();
        assert_eq!(metadata.mode() & 0o7777, 0o600);
        assert_eq!(metadata.nlink(), 1);
        assert_eq!(home.metadata().unwrap().mode() & 0o7777, 0o700);
        let raw = fs::read_to_string(&path).unwrap();
        assert!(!raw.contains(FAKE_KEY));
        let value: serde_json::Value = serde_json::from_str(raw.trim()).unwrap();
        assert_eq!(value["event"], "desktop.voice_transcription.confirmed");
        assert!(value.get("text").is_none());
        store.append(event()).unwrap();
        assert_eq!(fs::read_to_string(path).unwrap().lines().count(), 2);
    }
    #[test]
    fn symlinked_home_and_parent_are_rejected() {
        let (root, home) = fixture();
        let target = root.path().join("target");
        fs::create_dir(&target).unwrap();
        symlink(&target, &home).unwrap();
        assert!(AuditStore::open(&home).is_err());
        assert!(AuditStore::open(&home.join("child")).is_err());
        assert_eq!(fs::read_dir(target).unwrap().count(), 0);
    }
    #[test]
    fn env_relative_parent_and_nonprivate_home_rejected() {
        let (root, home) = fixture();
        for path in [
            root.path().join(".ENV-secrets"),
            root.path().join("a/../home"),
            std::path::PathBuf::from("relative"),
        ] {
            assert!(AuditStore::open(&path).is_err());
        }
        fs::create_dir(&home).unwrap();
        fs::set_permissions(&home, fs::Permissions::from_mode(0o755)).unwrap();
        assert!(AuditStore::open(&home).is_err());
    }
    #[test]
    fn unsafe_audit_targets_rejected_without_touching_external_data() {
        for kind in [
            "symlink",
            "hardlink",
            "directory",
            "fifo",
            "mode",
            "setuid",
            "oversize",
        ] {
            let (root, home) = fixture();
            let store = AuditStore::open(&home).unwrap();
            let outside = root.path().join("external");
            fs::write(&outside, "PRIVATE_EXTERNAL").unwrap();
            let path = home.join("desktop-voice-audit.jsonl");
            match kind {
                "symlink" => symlink(&outside, &path).unwrap(),
                "hardlink" => fs::hard_link(&outside, &path).unwrap(),
                "directory" => fs::create_dir(&path).unwrap(),
                "fifo" => {
                    use std::os::unix::ffi::OsStrExt;
                    let path = std::ffi::CString::new(path.as_os_str().as_bytes()).unwrap();
                    // Only create an isolated FIFO fixture; no process or network call.
                    assert_eq!(unsafe { libc::mkfifo(path.as_ptr(), 0o600) }, 0);
                }
                _ => {
                    store.append(event()).unwrap();
                    if kind == "mode" || kind == "setuid" {
                        fs::set_permissions(
                            &path,
                            fs::Permissions::from_mode(if kind == "mode" { 0o644 } else { 0o4600 }),
                        )
                        .unwrap();
                    } else {
                        fs::OpenOptions::new()
                            .write(true)
                            .open(&path)
                            .unwrap()
                            .set_len(16 * 1024 * 1024)
                            .unwrap();
                    }
                }
            }
            assert!(store.append(event()).is_err());
            assert_eq!(fs::read_to_string(outside).unwrap(), "PRIVATE_EXTERNAL");
        }
    }
    #[test]
    fn entry_replacement_unlink_and_hardlink_detected_at_each_durable_checkpoint() {
        for phase in ["opened", "file-synced", "parent-synced"] {
            for attack in ["replace", "unlink", "hardlink"] {
                let (root, home) = fixture();
                let store = AuditStore::open(&home).unwrap();
                let path = home.join("desktop-voice-audit.jsonl");
                let outcome = store.test_append(event(), |current| {
                    if current == phase {
                        if attack == "hardlink" {
                            fs::hard_link(&path, root.path().join("extra-link")).unwrap();
                        } else {
                            if attack == "replace" {
                                fs::rename(&path, home.join("original")).unwrap();
                            } else {
                                fs::remove_file(&path).unwrap();
                            }
                            fs::write(&path, "PRIVATE_REPLACEMENT").unwrap();
                            fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
                        }
                    }
                    Ok(())
                });
                assert!(outcome.is_err());
                if attack != "hardlink" {
                    assert_eq!(fs::read_to_string(path).unwrap(), "PRIVATE_REPLACEMENT");
                }
            }
        }
    }
    #[test]
    fn fsync_uncertainty_fails_closed_before_transport() {
        for phase in ["file-synced", "parent-synced"] {
            let (_root, home) = fixture();
            let store = AuditStore::open(&home).unwrap();
            let outcome = transcribe_with(
                audio(),
                |event| {
                    store.test_append(event, |current| {
                        if current == phase {
                            Err("audit")
                        } else {
                            Ok(())
                        }
                    })
                },
                || panic!("No credential lookup"),
                |_, _| panic!("No transport"),
            );
            assert!(matches!(outcome, Err("audit")));
        }
    }
    #[test]
    fn home_replacement_during_provider_work_preserves_transcript_without_redirecting_audit() {
        let (_root, home) = fixture();
        let store = AuditStore::open(&home).unwrap();
        let outcome = transcribe_with(
            audio(),
            |event| store.append(event),
            || Some(FAKE_KEY.into()),
            |_, _| {
                fs::rename(&home, home.with_extension("original")).unwrap();
                fs::create_dir(&home).unwrap();
                fs::set_permissions(&home, fs::Permissions::from_mode(0o700)).unwrap();
                Ok("Keep received text".into())
            },
        )
        .unwrap();
        assert_eq!(outcome.text, "Keep received text");
        assert!(!outcome.audit_recorded);
        assert_eq!(fs::read_dir(home).unwrap().count(), 0);
    }
    #[test]
    fn concurrent_appends_remain_complete_content_free_lines() {
        let (_root, home) = fixture();
        let store = AuditStore::open(&home).unwrap();
        std::thread::scope(|scope| {
            for _ in 0..8 {
                scope.spawn(|| store.append(event()).unwrap());
            }
        });
        let raw = fs::read_to_string(home.join("desktop-voice-audit.jsonl")).unwrap();
        assert_eq!(raw.lines().count(), 8);
        for line in raw.lines() {
            assert!(serde_json::from_str::<serde_json::Value>(line).is_ok());
        }
    }
}
