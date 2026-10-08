use super::{evaluate::*, model::*, mutations::*, storage::AutomationStore, *};
use serde_json::{json, Value};
use std::{
    fs,
    os::unix::fs::{symlink, MetadataExt, PermissionsExt},
    path::PathBuf,
};
const NOW: i64 = 1800000000;
static TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
struct Fixture {
    _guard: std::sync::MutexGuard<'static, ()>,
    _temp: tempfile::TempDir,
    home: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().canonicalize().unwrap().join("home");
        Self {
            _guard: guard,
            _temp: temp,
            home,
        }
    }
    fn store(&self) -> AutomationStore {
        AutomationStore::open(&self.home).unwrap()
    }
    fn path(&self) -> PathBuf {
        self.home.join("automations/automations.json")
    }
    fn write(&self, bytes: &[u8]) {
        self.store();
        fs::write(self.path(), bytes).unwrap();
        fs::set_permissions(self.path(), fs::Permissions::from_mode(0o600)).unwrap();
    }
    fn project(&self) -> PathBuf {
        self.store();
        let p = self._temp.path().canonicalize().unwrap().join("project");
        fs::create_dir_all(p.join(".ghost")).unwrap();
        fs::write(
            self.home.join("projects.yaml"),
            format!(
                "version: 1\nprojects:\n  - alias: synthetic\n    name: Synthetic\n    path: {}\n",
                p.display()
            ),
        )
        .unwrap();
        fs::write(
            p.join(".ghost/project.yaml"),
            format!("alias: synthetic\nname: Synthetic\npath: {}\n", p.display()),
        )
        .unwrap();
        p
    }
}
fn payload() -> Payload {
    Payload {
        title: "Synthetic attention".into(),
        enabled: true,
        trigger: Trigger::Once {
            at: crate::memory::model::iso(NOW).unwrap(),
        },
        task: Task::Reminder {
            message: "Synthetic private reminder".into(),
        },
    }
}
fn definition() -> Definition {
    Definition {
        version: 1,
        id: uuid::Uuid::new_v4().to_string(),
        revision: uuid::Uuid::new_v4().to_string(),
        payload: payload(),
        created_at: NOW - 86400,
        updated_at: NOW - 86400,
        cursor: Cursor::default(),
    }
}
fn doc() -> Document {
    Document {
        version: 1,
        definitions: vec![definition()],
        inbox: vec![],
    }
}
fn create(runtime: &mut Runtime, store: &AutomationStore, home: &std::path::Path) -> Definition {
    let p = runtime
        .prepare(
            store,
            home,
            PrepareInput {
                operation: Operation::Create,
                automation_id: None,
                payload: Some(payload()),
            },
            NOW,
        )
        .unwrap();
    runtime.execute(store, home, execute(&p), NOW).unwrap();
    p.after.unwrap()
}
fn execute(p: &Preview) -> ExecuteInput {
    ExecuteInput {
        request_id: p.request_id.clone(),
        request_sha256: p.request_sha256.clone(),
        confirmation: p.confirmation_phrase.clone(),
    }
}
fn prepare_op(
    rt: &mut Runtime,
    store: &AutomationStore,
    home: &std::path::Path,
    d: &Definition,
    op: Operation,
) -> Preview {
    rt.prepare(
        store,
        home,
        PrepareInput {
            operation: op,
            automation_id: Some(d.id.clone()),
            payload: if op == Operation::Update {
                Some(d.payload.clone())
            } else {
                None
            },
        },
        NOW,
    )
    .unwrap()
}

#[test]
fn strict_version() {
    let mut d = doc();
    d.version = 2;
    assert!(d.validate().is_err());
    let mut r = definition();
    r.version = 2;
    assert!(r.validate().is_err());
}
#[test]
fn canonical_uuid4() {
    for v in [
        "not-an-id",
        "00000000-0000-1000-8000-000000000001",
        "00000000-0000-4000-C000-000000000001",
        "AAAAAAAA-AAAA-4AAA-8AAA-AAAAAAAAAAAA",
    ] {
        assert!(id(v).is_err());
    }
    assert!(id("00000000-0000-4000-8000-000000000001").is_ok());
}
#[test]
fn unknown_fields() {
    let mut p = serde_json::to_value(payload()).unwrap();
    p["executor"] = json!("synthetic");
    assert!(serde_json::from_value::<Payload>(p).is_err());
    let mut d = serde_json::to_value(doc()).unwrap();
    d["private_blob"] = json!({});
    assert!(serde_json::from_value::<Document>(d).is_err());
}
#[test]
fn unknown_trigger_and_task() {
    assert!(
        serde_json::from_value::<Trigger>(json!({"kind":"cron","expression":"* * * * *"})).is_err()
    );
    assert!(serde_json::from_value::<Task>(json!({"kind":"shell","command":"synthetic"})).is_err());
    assert!(serde_json::from_value::<Task>(
        json!({"kind":"review_page","page":"browser","message":"synthetic"})
    )
    .is_err());
}
#[test]
fn trigger_fields_are_strict() {
    assert!(serde_json::from_value::<Trigger>(
        json!({"kind":"once","at":"2027-01-01T00:00:00Z","url":"synthetic"})
    )
    .is_err());
    assert!(serde_json::from_value::<Task>(
        json!({"kind":"reminder","message":"synthetic","action":"synthetic"})
    )
    .is_err());
}
#[test]
fn text_bounds() {
    let mut p = payload();
    p.title = "x".repeat(161);
    assert!(p.validate().is_err());
    p = payload();
    p.task = Task::Reminder {
        message: "x".repeat(8193),
    };
    assert!(p.validate().is_err());
    p = payload();
    p.task = Task::Reminder {
        message: "é".repeat(4096),
    };
    assert!(p.validate().is_ok());
}
#[test]
fn secrets_rejected() {
    let mut p = payload();
    p.task = Task::Reminder {
        message: "OPENAI_API_KEY=sk-synthetic-secret-not-real".into(),
    };
    assert!(p.validate().is_err());
    p = payload();
    p.title = "password=synthetic-private".into();
    assert!(p.validate().is_err());
}
#[test]
fn control_characters_rejected() {
    for c in ['\0', '\t', '\r', '\u{7f}'] {
        let mut p = payload();
        p.task = Task::Reminder {
            message: format!("synthetic{c}text"),
        };
        assert!(p.validate().is_err());
    }
    let mut p = payload();
    p.task = Task::Reminder {
        message: "synthetic\nmultiline".into(),
    };
    assert!(p.validate().is_ok());
}
#[test]
fn schedule_bounds() {
    for (h, m, o) in [(24, 0, 0), (0, 60, 0), (0, 0, 841), (0, 0, -841)] {
        assert!(Trigger::Daily {
            hour: h,
            minute: m,
            offset_minutes: o
        }
        .validate()
        .is_err());
    }
    assert!(Trigger::Once {
        at: "tomorrow 9".into()
    }
    .validate()
    .is_err());
}
#[test]
fn once_before_at_after() {
    let t = payload().trigger;
    assert!(occurrence(&t, NOW - 1).unwrap().is_none());
    assert_eq!(occurrence(&t, NOW).unwrap().unwrap().0, NOW);
    assert_eq!(occurrence(&t, NOW + 1).unwrap().unwrap().0, NOW);
}
#[test]
fn fractional_once_not_early() {
    let t = Trigger::Once {
        at: "2027-01-15T08:00:00.500Z".into(),
    };
    let sec = instant("2027-01-15T08:00:00Z").unwrap().timestamp();
    assert!(occurrence(&t, sec).unwrap().is_none());
    assert!(occurrence(&t, sec + 1).unwrap().is_some());
}
#[test]
fn once_fires_once() {
    let mut d = doc();
    assert_eq!(
        evaluate_document(&mut d, NOW, |_| panic!(
            "time trigger read local conditions"
        ))
        .unwrap()
        .0
        .len(),
        1
    );
    assert!(evaluate_document(&mut d, NOW + 86400, |_| panic!())
        .unwrap()
        .0
        .is_empty());
}
#[test]
fn daily_fixed_offsets() {
    let now = instant("2027-01-15T03:00:00Z").unwrap().timestamp();
    let t = Trigger::Daily {
        hour: 8,
        minute: 30,
        offset_minutes: 330,
    };
    let (due, key) = occurrence(&t, now).unwrap().unwrap();
    assert_eq!(due, now);
    assert_eq!(key, "daily:2027-01-15/08:30/+330");
    let west = Trigger::Daily {
        hour: 9,
        minute: 0,
        offset_minutes: -240,
    };
    assert_eq!(
        occurrence(&west, instant("2027-01-15T13:00:00Z").unwrap().timestamp())
            .unwrap()
            .unwrap()
            .1,
        "daily:2027-01-15/09:00/-240"
    );
}
#[test]
fn weekly_fixed_offset() {
    let now = instant("2027-01-18T03:30:00Z").unwrap().timestamp();
    let t = Trigger::Weekly {
        weekday: Weekday::Monday,
        hour: 9,
        minute: 0,
        offset_minutes: 330,
    };
    assert_eq!(
        occurrence(&t, now).unwrap().unwrap(),
        (now, "weekly:2027-01-18/09:00/+330".into())
    );
    assert_eq!(occurrence(&t, now - 1).unwrap().unwrap().0, now - 7 * 86400);
}
#[test]
fn clock_rollback_no_duplicate() {
    let mut d = doc();
    evaluate_document(&mut d, NOW, |_| Ok(false)).unwrap();
    assert!(evaluate_document(&mut d, NOW - 1, |_| Ok(false))
        .unwrap()
        .0
        .is_empty());
    assert!(evaluate_document(&mut d, NOW, |_| Ok(false))
        .unwrap()
        .0
        .is_empty());
}
#[test]
fn multi_day_catch_up_one_item() {
    let mut d = doc();
    d.definitions[0].payload.trigger = Trigger::Daily {
        hour: 8,
        minute: 30,
        offset_minutes: 330,
    };
    let result = evaluate_document(&mut d, NOW + 30 * 86400, |_| Ok(false)).unwrap();
    assert_eq!(result.0.len(), 1);
    assert!(evaluate_document(&mut d, NOW + 30 * 86400, |_| Ok(false))
        .unwrap()
        .0
        .is_empty());
}
#[test]
fn evaluation_cap_and_progress() {
    let mut d = Document::empty();
    for _ in 0..65 {
        d.definitions.push(definition());
    }
    assert_eq!(
        evaluate_document(&mut d, NOW, |_| Ok(false))
            .unwrap()
            .0
            .len(),
        32
    );
    assert_eq!(
        evaluate_document(&mut d, NOW, |_| Ok(false))
            .unwrap()
            .0
            .len(),
        32
    );
    assert_eq!(
        evaluate_document(&mut d, NOW, |_| Ok(false))
            .unwrap()
            .0
            .len(),
        1
    );
}
#[test]
fn disabled_ignored() {
    let mut d = doc();
    d.definitions[0].payload.enabled = false;
    assert!(evaluate_document(&mut d, NOW, |_| panic!())
        .unwrap()
        .0
        .is_empty());
}
#[test]
fn condition_edges() {
    let mut d = doc();
    d.definitions[0].payload.trigger = Trigger::RecentPendingRequestsPresent {};
    for (value, expected) in [
        (false, 0),
        (false, 0),
        (true, 1),
        (true, 0),
        (false, 0),
        (true, 1),
    ] {
        assert_eq!(
            evaluate_document(&mut d, NOW, |_| Ok(value))
                .unwrap()
                .0
                .len(),
            expected
        );
    }
    assert_eq!(d.inbox[0].occurrence_key, "condition:1");
    assert_eq!(d.inbox[1].occurrence_key, "condition:2");
}
#[test]
fn condition_state_survives_restart() {
    let f = Fixture::new();
    let store = f.store();
    let mut d = doc();
    d.definitions[0].payload.trigger = Trigger::RecentPendingRequestsPresent {};
    store
        .transaction(|doc| {
            *doc = d;
            let (_, events) = evaluate_document(doc, NOW, |_| Ok(true))?;
            Ok(((), events))
        })
        .unwrap();
    let mut restarted = f.store().read_document().unwrap();
    assert!(evaluate_document(&mut restarted, NOW + 60, |_| Ok(true))
        .unwrap()
        .0
        .is_empty());
}
#[test]
fn registered_project_condition() {
    let f = Fixture::new();
    let p = f.project();
    assert!(crate::snapshot::automation_project_no_active_session(&f.home, "synthetic").unwrap());
    assert!(crate::snapshot::automation_project_no_active_session(&f.home, "../unsafe").is_err());
    assert!(crate::snapshot::automation_project_no_active_session(&f.home, "missing").is_err());
    fs::write(
        p.join(".ghost/active-session.yaml"),
        "id: bad\nproject_alias: synthetic\n",
    )
    .unwrap();
    assert!(crate::snapshot::automation_project_no_active_session(&f.home, "synthetic").is_err());
}
#[test]
fn global_pending_condition_uses_bounded_reader() {
    let f = Fixture::new();
    f.store();
    assert!(!crate::snapshot::automation_recent_pending_requests(&f.home).unwrap());
    let request = crate::snapshot::requests::prepare(
        "synthetic".into(),
        serde_json::from_value(
            json!({"action_type":"start_session","payload":{"goal":"Synthetic"}}),
        )
        .unwrap(),
    )
    .unwrap();
    fs::create_dir(f.home.join("action-requests")).unwrap();
    fs::write(
        f.home.join("action-requests").join(format!(
            "{}-{}.json",
            chrono::DateTime::parse_from_rfc3339(&request.created_at)
                .unwrap()
                .format("%Y%m%dT%H%M%S%9fZ"),
            request.id
        )),
        serde_json::to_vec(&request).unwrap(),
    )
    .unwrap();
    assert!(crate::snapshot::automation_recent_pending_requests(&f.home).unwrap());
}
#[test]
fn condition_bindings_revalidated() {
    let f = Fixture::new();
    f.project();
    let store = f.store();
    let mut rt = Runtime::default();
    let mut p = payload();
    p.trigger = Trigger::ProjectNoActiveSession {
        project_alias: "synthetic".into(),
    };
    let preview = rt
        .prepare(
            &store,
            &f.home,
            PrepareInput {
                operation: Operation::Create,
                automation_id: None,
                payload: Some(p),
            },
            NOW,
        )
        .unwrap();
    fs::remove_file(f.home.join("projects.yaml")).unwrap();
    assert_eq!(
        rt.execute(&store, &f.home, execute(&preview), NOW).err(),
        Some("invalid_input")
    );
    assert!(store.read_document().unwrap().definitions.is_empty());
}
#[test]
fn private_modes() {
    let f = Fixture::new();
    create(&mut Runtime::default(), &f.store(), &f.home);
    for path in [&f.home, &f.home.join("automations")] {
        assert_eq!(fs::metadata(path).unwrap().mode() & 0o7777, 0o700);
    }
    for path in [
        f.path(),
        f.home.join("automations/.automation-lock"),
        f.home.join("desktop-automation-audit.jsonl"),
    ] {
        assert_eq!(fs::metadata(path).unwrap().mode() & 0o7777, 0o600);
    }
}
#[test]
fn symlink_document_rejected() {
    let f = Fixture::new();
    f.store();
    let outside = f._temp.path().join("outside");
    fs::write(&outside, b"synthetic").unwrap();
    symlink(&outside, f.path()).unwrap();
    assert_eq!(f.store().read_document().err(), Some("storage"));
}
#[test]
fn symlink_directory_rejected() {
    let f = Fixture::new();
    f.store();
    fs::remove_dir(f.home.join("automations")).unwrap();
    symlink(f._temp.path(), f.home.join("automations")).unwrap();
    assert!(AutomationStore::open(&f.home).is_err());
}
#[test]
fn hardlink_document_rejected() {
    let f = Fixture::new();
    f.write(&serde_json::to_vec(&Document::empty()).unwrap());
    fs::hard_link(f.path(), f._temp.path().join("alias")).unwrap();
    assert_eq!(f.store().read_document().err(), Some("storage"));
}
#[test]
fn owner_validation() {
    assert!(storage::owner_valid(rustix::process::geteuid().as_raw()));
    assert!(!storage::owner_valid(
        rustix::process::geteuid().as_raw().wrapping_add(1)
    ));
}
#[test]
fn unsafe_modes_rejected() {
    let f = Fixture::new();
    f.write(&serde_json::to_vec(&Document::empty()).unwrap());
    fs::set_permissions(f.path(), fs::Permissions::from_mode(0o644)).unwrap();
    assert!(f.store().read_document().is_err());
    fs::set_permissions(
        f.home.join("automations"),
        fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    assert!(AutomationStore::open(&f.home).is_err());
}
#[test]
fn corruption_fail_closed() {
    let f = Fixture::new();
    for bytes in [
        b"broken".as_slice(),
        b"{\"version\":2,\"definitions\":[],\"inbox\":[]}".as_slice(),
        b"".as_slice(),
    ] {
        f.write(bytes);
        assert!(f.store().read_document().is_err());
    }
}
#[test]
fn record_limit() {
    let mut d = Document::empty();
    for _ in 0..129 {
        d.definitions.push(definition());
    }
    assert_eq!(d.validate().err(), Some("limit"));
}
#[test]
fn bounded_document() {
    let f = Fixture::new();
    f.write(&vec![b' '; MAX_BYTES + 1]);
    assert_eq!(f.store().read_document().err(), Some("invalid_state"));
}
#[test]
fn atomic_replacement_keeps_complete_doc() {
    let f = Fixture::new();
    let store = f.store();
    let mut rt = Runtime::default();
    let d = create(&mut rt, &store, &f.home);
    let before = fs::metadata(f.path()).unwrap().ino();
    let p = prepare_op(&mut rt, &store, &f.home, &d, Operation::Pause);
    rt.execute(&store, &f.home, execute(&p), NOW).unwrap();
    assert_ne!(fs::metadata(f.path()).unwrap().ino(), before);
    assert!(
        !store.read_document().unwrap().definitions[0]
            .payload
            .enabled
    );
}
#[test]
fn reentrant_lock_is_busy() {
    let f = Fixture::new();
    let store = f.store();
    store
        .transaction(|_| {
            assert_eq!(store.read_document().err(), Some("busy"));
            Ok(((), vec![]))
        })
        .unwrap();
}
#[test]
fn workspace_home_rejected() {
    let f = Fixture::new();
    fs::create_dir_all(&f.home).unwrap();
    fs::create_dir(f.home.join(".git")).unwrap();
    fs::set_permissions(&f.home, fs::Permissions::from_mode(0o700)).unwrap();
    assert!(AutomationStore::open(&f.home).is_err());
    assert!(AutomationStore::open(&f.home.join("nested")).is_err());
}
#[test]
fn prepare_preview_no_write() {
    let f = Fixture::new();
    let p = Runtime::default()
        .prepare(
            &f.store(),
            &f.home,
            PrepareInput {
                operation: Operation::Create,
                automation_id: None,
                payload: Some(payload()),
            },
            NOW,
        )
        .unwrap();
    assert!(!f.path().exists());
    assert_eq!(p.request_sha256, digest(&p).unwrap());
    assert_eq!(p.after.as_ref().unwrap().payload.task, payload().task);
}
#[test]
fn review_hash_deterministic() {
    let f = Fixture::new();
    let p = Runtime::default()
        .prepare(
            &f.store(),
            &f.home,
            PrepareInput {
                operation: Operation::Create,
                automation_id: None,
                payload: Some(payload()),
            },
            NOW,
        )
        .unwrap();
    assert_eq!(digest(&p).unwrap(), digest(&p.clone()).unwrap());
    let mut changed = p.clone();
    changed.after.as_mut().unwrap().payload.title = "Changed".into();
    assert_ne!(digest(&p).unwrap(), digest(&changed).unwrap());
}
#[test]
fn exact_confirmation_consumes_failure() {
    let f = Fixture::new();
    let store = f.store();
    let mut rt = Runtime::default();
    let p = rt
        .prepare(
            &store,
            &f.home,
            PrepareInput {
                operation: Operation::Create,
                automation_id: None,
                payload: Some(payload()),
            },
            NOW,
        )
        .unwrap();
    let mut bad = execute(&p);
    bad.confirmation += " ";
    assert_eq!(
        rt.execute(&store, &f.home, bad, NOW).err(),
        Some("changed_review")
    );
    assert_eq!(
        rt.execute(&store, &f.home, execute(&p), NOW).err(),
        Some("changed_review")
    );
    assert!(store.read_document().unwrap().definitions.is_empty());
}
#[test]
fn five_minute_expiry() {
    let f = Fixture::new();
    let store = f.store();
    let mut rt = Runtime::default();
    let p = rt
        .prepare(
            &store,
            &f.home,
            PrepareInput {
                operation: Operation::Create,
                automation_id: None,
                payload: Some(payload()),
            },
            NOW,
        )
        .unwrap();
    assert_eq!(
        rt.execute(&store, &f.home, execute(&p), NOW + 300).err(),
        Some("review_expired")
    );
}
#[test]
fn clock_rollback_review_rejected() {
    let f = Fixture::new();
    let store = f.store();
    let mut rt = Runtime::default();
    let p = rt
        .prepare(
            &store,
            &f.home,
            PrepareInput {
                operation: Operation::Create,
                automation_id: None,
                payload: Some(payload()),
            },
            NOW,
        )
        .unwrap();
    assert_eq!(
        rt.execute(&store, &f.home, execute(&p), NOW - 1).err(),
        Some("review_expired")
    );
}
#[test]
fn create_single_use_no_overwrite() {
    let f = Fixture::new();
    let store = f.store();
    let mut rt = Runtime::default();
    let p = rt
        .prepare(
            &store,
            &f.home,
            PrepareInput {
                operation: Operation::Create,
                automation_id: None,
                payload: Some(payload()),
            },
            NOW,
        )
        .unwrap();
    rt.execute(&store, &f.home, execute(&p), NOW).unwrap();
    assert_eq!(
        rt.execute(&store, &f.home, execute(&p), NOW).err(),
        Some("changed_review")
    );
    assert_eq!(store.read_document().unwrap().definitions.len(), 1);
}
#[test]
fn stale_update_pause_resume_delete_rejected() {
    let f = Fixture::new();
    let store = f.store();
    for operation in [
        Operation::Update,
        Operation::Pause,
        Operation::Resume,
        Operation::Delete,
    ] {
        let mut rt = Runtime::default();
        let mut d = create(&mut rt, &store, &f.home);
        if operation == Operation::Resume {
            let p = prepare_op(&mut rt, &store, &f.home, &d, Operation::Pause);
            rt.execute(&store, &f.home, execute(&p), NOW).unwrap();
            d = store
                .read_document()
                .unwrap()
                .definitions
                .into_iter()
                .find(|r| r.id == d.id)
                .unwrap();
        }
        let old = prepare_op(&mut rt, &store, &f.home, &d, operation);
        let change = prepare_op(&mut rt, &store, &f.home, &d, Operation::Update);
        rt.execute(&store, &f.home, execute(&change), NOW).unwrap();
        assert_eq!(
            rt.execute(&store, &f.home, execute(&old), NOW).err(),
            Some("changed_review")
        );
    }
}
#[test]
fn task_snapshot_immutable() {
    let f = Fixture::new();
    let store = f.store();
    let mut rt = Runtime::default();
    let d = create(&mut rt, &store, &f.home);
    evaluate(&store, &f.home, NOW).unwrap();
    let mut changed = d.payload.clone();
    changed.task = Task::Reminder {
        message: "New synthetic text".into(),
    };
    let before = store.read_document().unwrap().definitions[0].clone();
    let p = rt
        .prepare(
            &store,
            &f.home,
            PrepareInput {
                operation: Operation::Update,
                automation_id: Some(before.id),
                payload: Some(changed),
            },
            NOW,
        )
        .unwrap();
    rt.execute(&store, &f.home, execute(&p), NOW).unwrap();
    assert_eq!(store.read_document().unwrap().inbox[0].task, d.payload.task);
}
#[test]
fn audit_contains_no_task_or_title() {
    let f = Fixture::new();
    let store = f.store();
    create(&mut Runtime::default(), &store, &f.home);
    evaluate(&store, &f.home, NOW).unwrap();
    let audit = fs::read_to_string(f.home.join("desktop-automation-audit.jsonl")).unwrap();
    assert!(!audit.contains("Synthetic private reminder"));
    assert!(!audit.contains("Synthetic attention"));
    assert!(audit.contains("desktop.automation.triggered"));
    for line in audit.lines() {
        let value: Value = serde_json::from_str(line).unwrap();
        assert!(value.get("task").is_none());
        assert!(value.get("prompt").is_none());
    }
}
#[test]
fn acknowledge_and_dismiss_only_status() {
    let f = Fixture::new();
    let store = f.store();
    create(&mut Runtime::default(), &store, &f.home);
    evaluate(&store, &f.home, NOW).unwrap();
    let before = store.read_document().unwrap();
    let item = &before.inbox[0];
    assert!(handle_item(&store, &item.id, ItemStatus::Acknowledged, NOW).unwrap());
    assert_eq!(
        handle_item(&store, &item.id, ItemStatus::Dismissed, NOW).err(),
        Some("invalid_state")
    );
    let after = store.read_document().unwrap();
    assert_eq!(before.definitions, after.definitions);
    assert_eq!(before.inbox[0].task, after.inbox[0].task);
    assert_eq!(after.inbox[0].status, ItemStatus::Acknowledged);
}
#[test]
fn inbox_bound_no_silent_discard() {
    let f = Fixture::new();
    let store = f.store();
    let mut document = doc();
    for n in 0..MAX_INBOX {
        document.inbox.push(Item {
            version: 1,
            id: uuid::Uuid::new_v4().to_string(),
            automation_id: document.definitions[0].id.clone(),
            automation_title: "Synthetic".into(),
            task: payload().task,
            triggered_at: NOW,
            occurrence_key: format!("condition:{n}"),
            status: ItemStatus::Pending,
        });
    }
    f.write(&serde_json::to_vec(&document).unwrap());
    assert_eq!(evaluate(&store, &f.home, NOW).err(), Some("limit"));
    assert_eq!(store.read_document().unwrap(), document);
}
#[test]
fn evaluator_only_inbox_and_cursor() {
    let f = Fixture::new();
    let store = f.store();
    let d = create(&mut Runtime::default(), &store, &f.home);
    let first = evaluate(&store, &f.home, NOW).unwrap();
    assert_eq!(first.created.len(), 1);
    assert_eq!(first.pending_count, 1);
    assert!(evaluate(&f.store(), &f.home, NOW + 86400)
        .unwrap()
        .created
        .is_empty());
    let after = store.read_document().unwrap();
    assert_eq!(after.definitions[0].payload, d.payload);
    assert!(!f.home.join("memory").exists());
    assert!(!f.home.join("action-requests").exists());
}
#[test]
fn main_window_only() {
    assert!(ipc::main_window("main").is_ok());
    assert_eq!(ipc::main_window("secondary").err(), Some("unavailable"));
}
