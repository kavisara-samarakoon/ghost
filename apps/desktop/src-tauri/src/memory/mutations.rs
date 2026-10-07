use super::{hash, model::*, storage::MemoryStore, Result};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
#[derive(Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    CreateMemory,
    UpdateMemory,
    ArchiveMemory,
    DeleteMemory,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PrepareInput {
    pub action: Action,
    pub memory_id: Option<String>,
    pub payload: Option<Payload>,
}
#[derive(Clone, Serialize)]
pub struct Preview {
    pub version: u8,
    pub request_id: String,
    pub action: Action,
    pub memory_id: String,
    pub before: Option<Record>,
    pub after: Option<Record>,
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
    pub memory_id: String,
    pub changed: bool,
    pub audit_recorded: bool,
}
#[derive(Clone, Serialize)]
pub struct AuditEvent {
    pub timestamp: i64,
    pub memory_id: String,
    pub action: Action,
    pub kind: Kind,
    pub result: &'static str,
    pub request_sha256: String,
    pub sensitivity: Sensitivity,
    pub sharing: Sharing,
}
pub struct Pending {
    pub preview: Preview,
    pub deadline: Instant,
}
#[derive(Default)]
pub struct Runtime {
    pub pending: HashMap<String, Pending>,
    pub home: Option<PathBuf>,
}
#[derive(Default)]
pub struct MemoryState(pub std::sync::Mutex<Runtime>);
impl Runtime {
    pub fn bind(&mut self, home: &Path) {
        if self.home.as_deref() != Some(home) {
            self.pending.clear();
            self.home = Some(home.into());
        }
        self.pending.retain(|_, p| p.deadline > Instant::now());
    }
    pub fn prepare(
        &mut self,
        store: &MemoryStore,
        home: &Path,
        input: PrepareInput,
        now: i64,
    ) -> Result<Preview> {
        self.bind(home);
        if self.pending.len() >= 16 {
            return Err("busy");
        }
        let records = store.list()?;
        let before = match input.action {
            Action::CreateMemory => {
                if input.memory_id.is_some() {
                    return Err("invalid_input");
                }
                None
            }
            _ => {
                let id = input.memory_id.as_deref().ok_or("invalid_input")?;
                super::model::id(id)?;
                Some(
                    records
                        .iter()
                        .find(|r| r.memory_id == id)
                        .ok_or("memory_missing")?
                        .clone(),
                )
            }
        };
        let memory_id = before
            .as_ref()
            .map(|r| r.memory_id.clone())
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        let after = match input.action {
            Action::CreateMemory | Action::UpdateMemory => {
                let payload = input.payload.ok_or("invalid_input")?.normalized()?;
                validate_source(home, &payload.source)?;
                let record = Record {
                    version: 1,
                    memory_id: memory_id.clone(),
                    payload,
                    created_at: before
                        .as_ref()
                        .map(|r| r.created_at.clone())
                        .unwrap_or(iso(now)?),
                    updated_at: iso(now)?,
                    status: before.as_ref().map(|r| r.status).unwrap_or(Status::Active),
                };
                record.validate()?;
                duplicate(&records, &record)?;
                Some(record)
            }
            Action::ArchiveMemory => {
                if input.payload.is_some() {
                    return Err("invalid_input");
                }
                let mut r = before.clone().ok_or("memory_missing")?;
                r.status = Status::Archived;
                r.updated_at = iso(now)?;
                r.validate()?;
                Some(r)
            }
            Action::DeleteMemory => {
                if input.payload.is_some() {
                    return Err("invalid_input");
                }
                None
            }
        };
        let mut p = Preview {
            version: 1,
            request_id: uuid::Uuid::new_v4().to_string(),
            action: input.action,
            memory_id,
            before,
            after,
            created_at: now,
            expires_at: now + 300,
            request_sha256: String::new(),
            confirmation_phrase: String::new(),
        };
        p.request_sha256 = digest(&p)?;
        p.confirmation_phrase = phrase(p.action, &p.request_sha256);
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
        store: &MemoryStore,
        home: &Path,
        input: ExecuteInput,
        now: i64,
    ) -> Result<Outcome> {
        self.execute_checked(store, home, input, now, |_| Ok(()))
    }
    pub fn execute_checked(
        &mut self,
        store: &MemoryStore,
        home: &Path,
        input: ExecuteInput,
        now: i64,
        checkpoint: impl FnMut(&str) -> Result<()>,
    ) -> Result<Outcome> {
        // Consume before any confirmation/storage attempt, including failed confirmations.
        if self.home.as_deref() != Some(home) {
            self.pending.clear();
            return Err("changed_review");
        }
        let pending = self
            .pending
            .remove(&input.request_id)
            .ok_or("review_missing")?;
        let p = pending.preview;
        if input.request_id.len() > 36
            || input.request_sha256.len() > 64
            || input.confirmation.len() > 96
        {
            return Err("invalid_input");
        }

        if pending.deadline <= Instant::now() || now >= p.expires_at {
            return Err("review_expired");
        }
        if input.request_sha256 != p.request_sha256
            || digest(&p)? != p.request_sha256
            || input.confirmation != phrase(p.action, &p.request_sha256)
        {
            return Err("changed_review");
        }
        if let Some(r) = &p.after {
            validate_source(home, &r.payload.source)?;
        }
        let r = p
            .after
            .as_ref()
            .or(p.before.as_ref())
            .ok_or("invalid_input")?;
        let event = AuditEvent {
            timestamp: now,
            memory_id: p.memory_id.clone(),
            action: p.action,
            kind: r.payload.kind,
            result: "confirmed",
            request_sha256: p.request_sha256.clone(),
            sensitivity: r.payload.sensitivity,
            sharing: r.payload.sharing,
        };
        let audit_recorded = store.mutate(
            &event,
            |doc| {
                if pending.deadline <= Instant::now() {
                    return Err("review_expired");
                }
                let current = doc.records.iter().find(|r| r.memory_id == p.memory_id);
                if current != p.before.as_ref() {
                    return Err("changed_review");
                }
                if let Some(after) = &p.after {
                    after.validate()?;
                    duplicate(&doc.records, after)?;
                }
                if p.before.is_none() && doc.records.len() >= MAX_RECORDS {
                    return Err("memory_limit");
                }
                doc.records.retain(|r| r.memory_id != p.memory_id);
                if let Some(after) = p.after {
                    doc.records.push(after);
                }
                Ok(())
            },
            checkpoint,
        )?;
        Ok(Outcome {
            memory_id: p.memory_id,
            changed: true,
            audit_recorded,
        })
    }
}
fn validate_source(home: &Path, source: &Source) -> Result<()> {
    match source {
        Source::Manual {} => Ok(()),
        Source::ProjectReference {
            project_alias,
            relative_path,
        } => crate::snapshot::validate_memory_reference(home, project_alias, relative_path)
            .map_err(|_| "invalid_source"),
    }
}
fn duplicate(records: &[Record], r: &Record) -> Result<()> {
    if r.status == Status::Active
        && records.iter().any(|v| {
            v.memory_id != r.memory_id
                && v.status == Status::Active
                && v.payload.duplicate_key() == r.payload.duplicate_key()
        })
    {
        return Err("duplicate_memory");
    }
    Ok(())
}
pub fn phrase(action: Action, hash: &str) -> String {
    format!(
        "{} MEMORY {hash}",
        match action {
            Action::CreateMemory => "SAVE",
            Action::UpdateMemory => "UPDATE",
            Action::ArchiveMemory => "ARCHIVE",
            Action::DeleteMemory => "DELETE",
        }
    )
}
fn digest(p: &Preview) -> Result<String> {
    hash(&(
        p.version,
        &p.request_id,
        p.action,
        &p.memory_id,
        &p.before,
        &p.after,
        p.created_at,
        p.expires_at,
    ))
}
