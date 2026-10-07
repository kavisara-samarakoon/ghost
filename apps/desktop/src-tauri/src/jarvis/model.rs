use super::Result;
use crate::connectors::assistant::model::{EventInput, EventTime, MailInput, Window};
use crate::memory::model::{Kind as MemoryKind, Payload, Sensitivity, Sharing, Source};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
#[derive(Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HandoffProvider {
    Codex,
    Chatgpt,
    Gemini,
    Antigravity,
}
#[derive(Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "capability", rename_all = "snake_case", deny_unknown_fields)]
pub enum Step {
    SearchProjectMemory {
        query: String,
    },
    StartSessionRequest {
        goal: String,
    },
    AddSessionNoteRequest {
        note: String,
    },
    GenerateNextStepsRequest {},
    CreateHandoffRequest {
        provider: HandoffProvider,
    },
    RememberPersonalMemory {
        kind: MemoryKind,
        title: String,
        content: String,
        tags: Vec<String>,
    },
    SearchMail {
        query: String,
    },
    ListAgenda {
        start: String,
        end: String,
    },
    FindFreeTime {
        start: String,
        end: String,
        duration_minutes: u32,
    },
    LookupContact {
        query: String,
    },
    CreateMailDraft {
        to: Vec<String>,
        cc: Vec<String>,
        subject: String,
        body: String,
    },
    SendMail {
        to: Vec<String>,
        cc: Vec<String>,
        subject: String,
        body: String,
    },
    CreateCalendarEvent {
        summary: String,
        description: Option<String>,
        location: Option<String>,
        start: EventTime,
        end: EventTime,
    },
}
#[derive(Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Plan,
    Clarify,
    Unsupported,
}
#[derive(Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Plan {
    pub kind: Kind,
    pub summary: String,
    pub steps: Vec<Step>,
}
pub fn text(value: &str, max: usize, multiline: bool) -> Result<()> {
    crate::connectors::assistant::model::input_text(value, max, multiline)?;
    if value.trim().is_empty() || crate::snapshot::text::redact(value.trim()) != value.trim() {
        return Err("invalid_input");
    }
    Ok(())
}
pub fn alias(value: &str) -> Result<()> {
    if !crate::snapshot::memory_reference_path(value, "status.md") {
        return Err("invalid_input");
    }
    text(value, 128, false)
}
fn local_text(value: &str) -> Result<()> {
    text(value, 8000, true)?;
    let pattern=regex::Regex::new(r"[a-zA-Z][a-zA-Z0-9+.-]*://|\$[A-Za-z_({]|%[A-Za-z_][\w]*%|\{\{|\{%|`|(?:^|\n)\s*(?:sh|bash|zsh|cmd|powershell|git|gh|curl|wget|rm|sudo|python[0-9.]*|node|npm|pnpm)\s").expect("fixed pattern");
    if pattern.is_match(value) {
        return Err("proposal");
    }
    Ok(())
}
impl Step {
    pub fn capability(&self) -> &'static str {
        match self {
            Self::SearchProjectMemory { .. } => "search_project_memory",
            Self::StartSessionRequest { .. } => "start_session_request",
            Self::AddSessionNoteRequest { .. } => "add_session_note_request",
            Self::GenerateNextStepsRequest { .. } => "generate_next_steps_request",
            Self::CreateHandoffRequest { .. } => "create_handoff_request",
            Self::RememberPersonalMemory { .. } => "remember_personal_memory",
            Self::SearchMail { .. } => "search_mail",
            Self::ListAgenda { .. } => "list_agenda",
            Self::FindFreeTime { .. } => "find_free_time",
            Self::LookupContact { .. } => "lookup_contact",
            Self::CreateMailDraft { .. } => "create_mail_draft",
            Self::SendMail { .. } => "send_mail",
            Self::CreateCalendarEvent { .. } => "create_calendar_event",
        }
    }
    fn validate(&self, project: Option<&str>, now: chrono::DateTime<chrono::Utc>) -> Result<()> {
        match self {
            Self::SearchProjectMemory { query } => {
                crate::memory::search::query(query)?;
            }
            Self::StartSessionRequest { goal } | Self::AddSessionNoteRequest { note: goal } => {
                if project.is_none() {
                    return Err("proposal");
                }
                local_text(goal)?;
            }
            Self::GenerateNextStepsRequest {} | Self::CreateHandoffRequest { .. } => {
                if project.is_none() {
                    return Err("proposal");
                }
            }
            Self::RememberPersonalMemory {
                kind,
                title,
                content,
                tags,
            } => {
                let p = Payload {
                    kind: *kind,
                    title: title.clone(),
                    content: content.clone(),
                    tags: tags.clone(),
                    sensitivity: Sensitivity::Standard,
                    sharing: Sharing::LocalOnly,
                    source: Source::Manual {},
                    expires_at: None,
                };
                p.normalized()?;
            }
            Self::SearchMail { query } => text(query, 512, false)?,
            Self::LookupContact { query } => text(query, 128, false)?,
            Self::ListAgenda { start, end } | Self::FindFreeTime { start, end, .. } => {
                Window {
                    start: start.clone(),
                    end: end.clone(),
                }
                .normalized()?;
                if let Self::FindFreeTime {
                    duration_minutes, ..
                } = self
                {
                    if !(5..=480).contains(duration_minutes) {
                        return Err("proposal");
                    }
                }
            }
            Self::CreateMailDraft {
                to,
                cc,
                subject,
                body,
            }
            | Self::SendMail {
                to,
                cc,
                subject,
                body,
            } => {
                MailInput {
                    to: to.clone(),
                    cc: cc.clone(),
                    subject: subject.clone(),
                    body: body.clone(),
                }
                .normalized()?;
            }
            Self::CreateCalendarEvent {
                summary,
                description,
                location,
                start,
                end,
            } => {
                EventInput {
                    summary: summary.clone(),
                    description: description.clone(),
                    location: location.clone(),
                    start: start.clone(),
                    end: end.clone(),
                }
                .normalized(now)?;
            }
        }
        // All leaf strings are checked before an untrusted proposal becomes a DTO.
        reject_strings(&serde_json::to_value(self).map_err(|_| "proposal")?)?;
        Ok(())
    }
}
fn reject_strings(value: &Value) -> Result<()> {
    match value {
        Value::String(s) => {
            crate::connectors::assistant::model::input_text(s, 32768, true)?;
            if crate::snapshot::text::redact(s.trim()) != s.trim() {
                return Err("proposal");
            }
        }
        Value::Array(a) => {
            for v in a {
                reject_strings(v)?;
            }
        }
        Value::Object(o) => {
            for v in o.values() {
                reject_strings(v)?;
            }
        }
        _ => (),
    };
    Ok(())
}
impl Plan {
    pub fn validate(
        &self,
        project: Option<&str>,
        now: chrono::DateTime<chrono::Utc>,
    ) -> Result<()> {
        if match self.kind {
            Kind::Plan => self.steps.is_empty() || self.steps.len() > 8,
            Kind::Clarify | Kind::Unsupported => !self.steps.is_empty(),
        } {
            return Err("proposal");
        }
        text(&self.summary, 2048, true).map_err(|_| "proposal")?;
        for step in &self.steps {
            step.validate(project, now).map_err(|_| "proposal")?;
        }
        Ok(())
    }
}
#[derive(Serialize)]
pub struct Registry {
    pub version: u8,
    pub capabilities: Vec<Descriptor>,
}
#[derive(Serialize)]
pub struct Descriptor {
    pub capability: &'static str,
    pub effect: &'static str,
}
pub const CAPABILITIES: [(&str, &str); 13] = [
    ("search_project_memory", "local_read"),
    ("start_session_request", "pending_local_request"),
    ("add_session_note_request", "pending_local_request"),
    ("generate_next_steps_request", "pending_local_request"),
    ("create_handoff_request", "pending_local_request"),
    ("remember_personal_memory", "confirmed_memory_write"),
    ("search_mail", "explicit_google_read"),
    ("list_agenda", "explicit_google_read"),
    ("find_free_time", "explicit_google_read"),
    ("lookup_contact", "explicit_google_read"),
    ("create_mail_draft", "confirmed_google_write"),
    ("send_mail", "confirmed_google_write"),
    ("create_calendar_event", "confirmed_google_write"),
];
pub fn registry() -> Registry {
    Registry {
        version: super::VERSION,
        capabilities: CAPABILITIES
            .iter()
            .map(|(capability, effect)| Descriptor { capability, effect })
            .collect(),
    }
}
fn object(properties: Value) -> Value {
    let required: Vec<_> = properties
        .as_object()
        .expect("fixed schema")
        .keys()
        .cloned()
        .collect();
    json!({"type":"object","properties":properties,"required":required,"additionalProperties":false})
}
pub fn schema() -> Value {
    let string = json!({"type":"string","maxLength":8000});
    let strings = json!({"type":"array","items":{"type":"string","maxLength":254},"maxItems":5});
    let event_time = object(
        json!({"date_time":{"type":["string","null"],"maxLength":40},"date":{"type":["string","null"],"maxLength":10}}),
    );
    let variant = |name: &str, fields: Value| {
        let mut properties = fields.as_object().unwrap().clone();
        properties.insert("capability".into(), json!({"type":"string","enum":[name]}));
        object(Value::Object(properties))
    };
    let mail = json!({"to":{"type":"array","items":{"type":"string","maxLength":254},"minItems":1,"maxItems":5},"cc":strings,"subject":{"type":"string","minLength":1,"maxLength":256},"body":{"type":"string","minLength":1,"maxLength":32768}});
    object(
        json!({"kind":{"type":"string","enum":["plan","clarify","unsupported"]},"summary":{"type":"string","minLength":1,"maxLength":2048},"steps":{"type":"array","maxItems":8,"items":{"anyOf":[
            variant("search_project_memory",json!({"query":{"type":"string","minLength":2,"maxLength":120}})),
            variant("start_session_request",json!({"goal":string})),variant("add_session_note_request",json!({"note":string})),variant("generate_next_steps_request",json!({})),
            variant("create_handoff_request",json!({"provider":{"type":"string","enum":["codex","chatgpt","gemini","antigravity"]}})),
            variant("remember_personal_memory",json!({"kind":{"type":"string","enum":["identity","preference","person","project","commitment","decision","fact"]},"title":{"type":"string","minLength":1,"maxLength":160},"content":{"type":"string","minLength":1,"maxLength":8192},"tags":{"type":"array","items":{"type":"string","minLength":1,"maxLength":32},"maxItems":12}})),
            variant("search_mail",json!({"query":{"type":"string","minLength":1,"maxLength":512}})),variant("list_agenda",json!({"start":{"type":"string","maxLength":40},"end":{"type":"string","maxLength":40}})),variant("find_free_time",json!({"start":{"type":"string","maxLength":40},"end":{"type":"string","maxLength":40},"duration_minutes":{"type":"integer","minimum":5,"maximum":480}})),variant("lookup_contact",json!({"query":{"type":"string","minLength":1,"maxLength":128}})),
            variant("create_mail_draft",mail.clone()),variant("send_mail",mail),variant("create_calendar_event",json!({"summary":{"type":"string","minLength":1,"maxLength":256},"description":{"type":["string","null"],"maxLength":4096},"location":{"type":["string","null"],"maxLength":512},"start":event_time,"end":event_time}))
        ]}}}),
    )
}
