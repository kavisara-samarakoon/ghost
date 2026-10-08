use super::{model::*, storage::AutomationStore, Result};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Operation {
    Create,
    Update,
    Pause,
    Resume,
    Delete,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PrepareInput {
    pub operation: Operation,
    pub automation_id: Option<String>,
    pub payload: Option<Payload>,
}
#[derive(Clone, Serialize)]
pub struct Preview {
    pub version: u8,
    pub request_id: String,
    pub operation: Operation,
    pub automation_id: String,
    pub before: Option<Definition>,
    pub after: Option<Definition>,
    pub created_at: i64,
    pub expires_at: i64,
    pub request_sha256: String,
    pub confirmation_phrase: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecuteInput {
    pub request_id: String,
    pub request_sha256: String,
    pub confirmation: String,
}
#[derive(Serialize)]
pub struct Outcome {
    pub automation_id: String,
    pub changed: bool,
    pub audit_recorded: bool,
}
#[derive(Clone, Serialize)]
pub struct AuditEvent {
    pub timestamp: i64,
    pub event: &'static str,
    pub automation_id: String,
    pub operation: &'static str,
    pub trigger_kind: &'static str,
    pub task_kind: &'static str,
    pub result: &'static str,
    pub review_sha256: Option<String>,
    pub occurrence_key: Option<String>,
    pub item_id: Option<String>,
}
impl AuditEvent {
    pub fn definition(
        event: &'static str,
        d: &Definition,
        now: i64,
        result: &'static str,
        occurrence_key: Option<String>,
    ) -> Self {
        Self {
            timestamp: now,
            event,
            automation_id: d.id.clone(),
            operation: d.payload.trigger.kind(),
            trigger_kind: d.payload.trigger.kind(),
            task_kind: d.payload.task.kind(),
            result,
            review_sha256: None,
            occurrence_key,
            item_id: None,
        }
    }
}
struct Pending {
    preview: Preview,
    deadline: Instant,
}
#[derive(Default)]
pub struct Runtime {
    home: Option<PathBuf>,
    pending: HashMap<String, Pending>,
}
#[derive(Default)]
pub struct AutomationState(pub std::sync::Mutex<Runtime>);
pub fn phrase(operation: Operation, hash: &str) -> String {
    format!(
        "{} AUTOMATION {hash}",
        match operation {
            Operation::Create => "CREATE",
            Operation::Update => "UPDATE",
            Operation::Pause => "PAUSE",
            Operation::Resume => "RESUME",
            Operation::Delete => "DELETE",
        }
    )
}
pub fn digest(p: &Preview) -> Result<String> {
    crate::memory::hash(&(
        p.version,
        &p.request_id,
        p.operation,
        &p.automation_id,
        &p.before,
        &p.after,
        p.created_at,
        p.expires_at,
    ))
    .map_err(|_| "invalid_input")
}
impl Runtime {
    pub fn prepare(
        &mut self,
        store: &AutomationStore,
        home: &Path,
        input: PrepareInput,
        now: i64,
    ) -> Result<Preview> {
        timestamp(now)?;
        if self.home.as_deref() != Some(home) {
            self.pending.clear();
            self.home = Some(home.into());
        }
        self.pending.retain(|_, p| p.deadline > Instant::now());
        if self.pending.len() >= 16 {
            return Err("busy");
        }
        let doc = store.read_document()?;
        let before = if input.operation == Operation::Create {
            if input.automation_id.is_some() || doc.definitions.len() >= MAX_DEFINITIONS {
                return Err("invalid_input");
            }
            None
        } else {
            let value = input.automation_id.as_deref().ok_or("invalid_input")?;
            id(value)?;
            Some(
                doc.definitions
                    .iter()
                    .find(|d| d.id == value)
                    .ok_or("invalid_state")?
                    .clone(),
            )
        };
        let automation_id = before
            .as_ref()
            .map(|r| r.id.clone())
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        let after = match input.operation {
            Operation::Create | Operation::Update => {
                let payload = input.payload.ok_or("invalid_input")?;
                payload.validate()?;
                payload.bindings(home)?;
                let cursor = before
                    .as_ref()
                    .filter(|r| r.payload.trigger == payload.trigger)
                    .map(|r| r.cursor.clone())
                    .unwrap_or_default();
                Some(Definition {
                    version: 1,
                    id: automation_id.clone(),
                    revision: uuid::Uuid::new_v4().to_string(),
                    payload,
                    created_at: before.as_ref().map(|r| r.created_at).unwrap_or(now),
                    updated_at: now,
                    cursor,
                })
            }
            Operation::Pause | Operation::Resume => {
                if input.payload.is_some() {
                    return Err("invalid_input");
                }
                let mut r = before.clone().ok_or("invalid_state")?;
                let enabled = input.operation == Operation::Resume;
                if r.payload.enabled == enabled {
                    return Err("invalid_state");
                }
                r.payload.enabled = enabled;
                r.updated_at = now;
                r.revision = uuid::Uuid::new_v4().to_string();
                Some(r)
            }
            Operation::Delete => {
                if input.payload.is_some() {
                    return Err("invalid_input");
                }
                None
            }
        };
        if let Some(r) = &after {
            r.validate()?;
        }
        let mut p = Preview {
            version: 1,
            request_id: uuid::Uuid::new_v4().to_string(),
            operation: input.operation,
            automation_id,
            before,
            after,
            created_at: now,
            expires_at: now + 300,
            request_sha256: String::new(),
            confirmation_phrase: String::new(),
        };
        p.request_sha256 = digest(&p)?;
        p.confirmation_phrase = phrase(p.operation, &p.request_sha256);
        self.pending.insert(
            p.request_id.clone(),
            Pending {
                preview: p.clone(),
                deadline: Instant::now() + Duration::from_secs(300),
            },
        );
        Ok(p)
    }
    pub fn execute(
        &mut self,
        store: &AutomationStore,
        home: &Path,
        input: ExecuteInput,
        now: i64,
    ) -> Result<Outcome> {
        if self.home.as_deref() != Some(home) {
            self.pending.clear();
            return Err("changed_review");
        }
        let pending = self
            .pending
            .remove(&input.request_id)
            .ok_or("changed_review")?;
        let p = pending.preview;
        if pending.deadline <= Instant::now() || now >= p.expires_at || now < p.created_at {
            return Err("review_expired");
        }
        if input.request_sha256 != p.request_sha256
            || digest(&p)? != p.request_sha256
            || input.confirmation != phrase(p.operation, &p.request_sha256)
        {
            return Err("changed_review");
        }
        let (_, audit_recorded) = store.transaction(|doc| {
            if pending.deadline <= Instant::now() {
                return Err("review_expired");
            }
            let current = doc.definitions.iter().find(|r| r.id == p.automation_id);
            if current != p.before.as_ref() {
                return Err("changed_review");
            }
            if p.before.is_none() && doc.definitions.len() >= MAX_DEFINITIONS {
                return Err("limit");
            }
            if let Some(r) = &p.after {
                r.validate()?;
                r.payload.bindings(home)?;
            }
            let r = p
                .after
                .as_ref()
                .or(p.before.as_ref())
                .ok_or("invalid_state")?;
            let mut event = AuditEvent::definition(
                match p.operation {
                    Operation::Create => "desktop.automation.created",
                    Operation::Update => "desktop.automation.updated",
                    Operation::Pause => "desktop.automation.paused",
                    Operation::Resume => "desktop.automation.resumed",
                    Operation::Delete => "desktop.automation.deleted",
                },
                r,
                now,
                "confirmed",
                None,
            );
            event.operation = match p.operation {
                Operation::Create => "create",
                Operation::Update => "update",
                Operation::Pause => "pause",
                Operation::Resume => "resume",
                Operation::Delete => "delete",
            };
            event.review_sha256 = Some(p.request_sha256.clone());
            doc.definitions.retain(|r| r.id != p.automation_id);
            if let Some(after) = p.after {
                doc.definitions.push(after);
            }
            Ok(((), vec![event]))
        })?;
        Ok(Outcome {
            automation_id: p.automation_id,
            changed: true,
            audit_recorded,
        })
    }
}
pub fn handle_item(
    store: &AutomationStore,
    item_id: &str,
    status: ItemStatus,
    now: i64,
) -> Result<bool> {
    id(item_id)?;
    timestamp(now)?;
    if status == ItemStatus::Pending {
        return Err("invalid_input");
    }
    let (_, audit) = store.transaction(|doc| {
        let item = doc
            .inbox
            .iter_mut()
            .find(|i| i.id == item_id)
            .ok_or("invalid_state")?;
        if item.status != ItemStatus::Pending {
            return Err("invalid_state");
        }
        item.status = status;
        let event = AuditEvent {
            timestamp: now,
            event: if status == ItemStatus::Acknowledged {
                "desktop.automation.acknowledged"
            } else {
                "desktop.automation.dismissed"
            },
            automation_id: item.automation_id.clone(),
            operation: if status == ItemStatus::Acknowledged {
                "acknowledge"
            } else {
                "dismiss"
            },
            trigger_kind: "inbox",
            task_kind: item.task.kind(),
            result: "confirmed",
            review_sha256: None,
            occurrence_key: Some(item.occurrence_key.clone()),
            item_id: Some(item.id.clone()),
        };
        Ok(((), vec![event]))
    })?;
    Ok(audit)
}
