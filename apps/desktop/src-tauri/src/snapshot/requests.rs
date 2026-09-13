//! Pending local drafts only. This module never dispatches workflow actions.
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

pub const SAFETY_NOTICE: &str = "Request only. No workflow changes have been made. Review and manually run the matching CLI command. Do not include secrets.";

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(
    tag = "action_type",
    content = "payload",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum Action {
    StartSession(Goal),
    AddSessionNote(Note),
    GenerateNextSteps(Empty),
    CreateHandoff(Handoff),
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Goal {
    goal: String,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Note {
    note: String,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Empty {}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Handoff {
    provider: Provider,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    Codex,
    Chatgpt,
    Gemini,
    Antigravity,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct ActionRequest {
    pub id: String,
    pub created_at: String,
    #[serde(flatten)]
    pub action: Action,
    pub project_alias: String,
    pub preview_title: String,
    pub preview_body: String,
    pub status: String,
    pub safety_notice: String,
}

#[derive(Serialize)]
pub struct SavedRequest {
    path: String,
    audit_recorded: bool,
}

fn body(value: &str) -> Result<(), &'static str> {
    if value.trim().is_empty() || value.len() > 8_000 {
        return Err("Enter between 1 and 8000 bytes of goal or note text.");
    }
    if super::text::redact(value) != value {
        return Err("Remove sensitive values or unsupported control characters before review.");
    }
    Ok(())
}

impl Action {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::StartSession(_) => "start_session",
            Self::AddSessionNote(_) => "add_session_note",
            Self::GenerateNextSteps(_) => "generate_next_steps",
            Self::CreateHandoff(_) => "create_handoff",
        }
    }
    fn preview(&self) -> Result<(&'static str, String), &'static str> {
        Ok(match self {
            Self::StartSession(value) => {
                body(&value.goal)?;
                ("Start session request", format!("Goal: {}", value.goal))
            }
            Self::AddSessionNote(value) => {
                body(&value.note)?;
                ("Session note request", format!("Note: {}", value.note))
            }
            Self::GenerateNextSteps(_) => (
                "Next steps request",
                "Prepare next steps for this project after manual review.".into(),
            ),
            Self::CreateHandoff(value) => (
                "Handoff request",
                format!(
                    "Provider: {}",
                    serde_json::to_value(&value.provider)
                        .unwrap()
                        .as_str()
                        .unwrap()
                ),
            ),
        })
    }
}

pub fn prepare(project_alias: String, action: Action) -> Result<ActionRequest, &'static str> {
    if !super::valid_alias(&project_alias) || super::text::redact(&project_alias) != project_alias {
        return Err("Project alias must contain 1–128 lowercase letters, digits, or hyphens, without secrets.");
    }
    let (title, body) = action.preview()?;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| "Local clock unavailable.")?;
    let date = chrono::DateTime::from_timestamp(now.as_secs() as i64, now.subsec_nanos())
        .ok_or("Local clock unavailable.")?;
    Ok(ActionRequest {
        id: format!("{}-{}", now.as_nanos(), std::process::id()),
        created_at: date.to_rfc3339_opts(chrono::SecondsFormat::Nanos, true),
        action,
        preview_title: title.into(),
        preview_body: format!("Project: {project_alias}\n{body}"),
        project_alias,
        status: "pending".into(),
        safety_notice: SAFETY_NOTICE.into(),
    })
}

impl ActionRequest {
    fn validate(&self) -> Result<(), &'static str> {
        let expected = prepare(self.project_alias.clone(), self.action.clone())?;
        if self.status != "pending"
            || self.preview_title != expected.preview_title
            || self.preview_body != expected.preview_body
            || self.safety_notice != SAFETY_NOTICE
            || self.id.is_empty()
            || self.id.len() > 64
            || !self.id.bytes().all(|c| c.is_ascii_digit() || c == b'-')
            || chrono::DateTime::parse_from_rfc3339(&self.created_at).is_err()
            || !self.created_at.ends_with('Z')
        {
            return Err("Request changed or is invalid. Prepare and review it again.");
        }
        Ok(())
    }

    fn filename(&self) -> Result<String, &'static str> {
        self.validate()?;
        let time = chrono::DateTime::parse_from_rfc3339(&self.created_at)
            .map_err(|_| "Invalid timestamp.")?;
        Ok(format!(
            "{}-{}.json",
            time.format("%Y%m%dT%H%M%S%9fZ"),
            self.id
        ))
    }

    fn audit(&self) -> serde_json::Value {
        serde_json::json!({ "timestamp": self.created_at, "event": "desktop.action_request.created",
            "action_type": self.action.kind(), "project_alias": self.project_alias, "request_id": self.id })
    }
}

pub fn save_from_environment(
    request: ActionRequest,
    confirmed: bool,
) -> Result<SavedRequest, &'static str> {
    if !confirmed {
        return Err("Explicit review confirmation is required.");
    }
    request.validate()?;
    let home = super::resolve_home(
        std::env::var_os("GHOST_HOME"),
        std::env::home_dir(),
        std::env::current_dir().ok(),
    )?;
    save(&home, &request)
}

#[cfg(unix)]
#[path = "requests/storage.rs"]
mod storage;
#[cfg(unix)]
use storage::save;
#[cfg(not(unix))]
fn save(_: &std::path::Path, _: &ActionRequest) -> Result<SavedRequest, &'static str> {
    Err("Safe request storage is unavailable on this platform.")
}

#[derive(Serialize)]
pub struct RecentRequest {
    id: String,
    created_at: String,
    action_type: String,
    project_alias: String,
    status: String,
}

pub(super) fn recent(
    directory: &super::reader::Directory,
    budget: &mut super::reader::ReadBudget,
) -> Result<Vec<RecentRequest>, &'static str> {
    let Some(directory) = directory.child("action-requests")? else {
        return Ok(vec![]);
    };
    let mut names = directory.list(budget)?.names;
    names.sort_unstable();
    let mut result = Vec::new();
    for name in names.into_iter().rev().take(20) {
        let Some(text) = directory.read(&name, budget)? else {
            continue;
        };
        let Ok(request) = serde_json::from_str::<ActionRequest>(&text) else {
            continue;
        };
        if request.filename().ok().as_deref() != Some(&name) {
            continue;
        }
        result.push(RecentRequest {
            id: request.id,
            created_at: request.created_at,
            action_type: request.action.kind().into(),
            project_alias: request.project_alias,
            status: request.status,
        });
        if result.len() == 10 {
            break;
        }
    }
    Ok(result)
}

pub(super) fn safe_filename(name: &str) -> bool {
    name.len() <= 110
        && name.ends_with(".json")
        && name.len() > 30
        && name[..name.len() - 5]
            .bytes()
            .all(|c| c.is_ascii_digit() || matches!(c, b'T' | b'Z' | b'-'))
}

#[cfg(all(test, unix))]
#[path = "requests/tests.rs"]
mod tests;
