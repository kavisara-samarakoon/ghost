use super::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
pub const MAX_DEFINITIONS: usize = 128;
pub const MAX_INBOX: usize = 512;
pub const MAX_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_EVALUATION: usize = 32;
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Weekday {
    Monday,
    Tuesday,
    Wednesday,
    Thursday,
    Friday,
    Saturday,
    Sunday,
}
impl Weekday {
    pub fn index(self) -> u32 {
        match self {
            Self::Monday => 0,
            Self::Tuesday => 1,
            Self::Wednesday => 2,
            Self::Thursday => 3,
            Self::Friday => 4,
            Self::Saturday => 5,
            Self::Sunday => 6,
        }
    }
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Page {
    Today,
    Mail,
    Calendar,
    Projects,
    Memory,
    Sessions,
    Artifacts,
    Connections,
    Command,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Trigger {
    Once {
        at: String,
    },
    Daily {
        hour: u8,
        minute: u8,
        offset_minutes: i16,
    },
    Weekly {
        weekday: Weekday,
        hour: u8,
        minute: u8,
        offset_minutes: i16,
    },
    ProjectNoActiveSession {
        project_alias: String,
    },
    RecentPendingRequestsPresent {},
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Task {
    Reminder {
        message: String,
    },
    CommandPrompt {
        prompt: String,
        project_alias: Option<String>,
    },
    ReviewPage {
        page: Page,
        message: String,
    },
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Payload {
    pub title: String,
    pub enabled: bool,
    pub trigger: Trigger,
    pub task: Task,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Cursor {
    pub last_due: Option<i64>,
    pub condition_true: bool,
    pub edge: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Definition {
    pub version: u8,
    pub id: String,
    pub revision: String,
    pub payload: Payload,
    pub created_at: i64,
    pub updated_at: i64,
    pub cursor: Cursor,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ItemStatus {
    Pending,
    Acknowledged,
    Dismissed,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Item {
    pub version: u8,
    pub id: String,
    pub automation_id: String,
    pub automation_title: String,
    pub task: Task,
    pub triggered_at: i64,
    pub occurrence_key: String,
    pub status: ItemStatus,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Document {
    pub version: u8,
    pub definitions: Vec<Definition>,
    pub inbox: Vec<Item>,
}
pub fn id(v: &str) -> Result<()> {
    crate::memory::model::id(v).map_err(|_| "invalid_input")
}
pub fn timestamp(v: i64) -> Result<()> {
    if (1..=253402300799).contains(&v) {
        Ok(())
    } else {
        Err("invalid_schedule")
    }
}
pub fn instant(v: &str) -> Result<DateTime<Utc>> {
    crate::memory::model::time(v).map_err(|_| "invalid_schedule")
}
pub fn alias(v: &str) -> Result<()> {
    if v.is_empty()
        || v.len() > 128
        || !v
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
    {
        return Err("invalid_input");
    }
    Ok(())
}
fn title(v: &str) -> Result<()> {
    crate::memory::model::text(v, 640, false).map_err(|_| "invalid_input")?;
    if v.chars().count() > 160 || v.trim() != v {
        return Err("invalid_input");
    }
    Ok(())
}
impl Trigger {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Once { .. } => "once",
            Self::Daily { .. } => "daily",
            Self::Weekly { .. } => "weekly",
            Self::ProjectNoActiveSession { .. } => "project_no_active_session",
            Self::RecentPendingRequestsPresent {} => "recent_pending_requests_present",
        }
    }
    pub fn validate(&self) -> Result<()> {
        match self {
            Self::Once { at } => {
                instant(at)?;
            }
            Self::Daily {
                hour,
                minute,
                offset_minutes,
            }
            | Self::Weekly {
                hour,
                minute,
                offset_minutes,
                ..
            } => {
                if *hour > 23 || *minute > 59 || !(-840..=840).contains(offset_minutes) {
                    return Err("invalid_schedule");
                }
            }
            Self::ProjectNoActiveSession { project_alias } => alias(project_alias)?,
            Self::RecentPendingRequestsPresent {} => (),
        }
        Ok(())
    }
}
impl Task {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Reminder { .. } => "reminder",
            Self::CommandPrompt { .. } => "command_prompt",
            Self::ReviewPage { .. } => "review_page",
        }
    }
    pub fn text(&self) -> &str {
        match self {
            Self::Reminder { message } | Self::ReviewPage { message, .. } => message,
            Self::CommandPrompt { prompt, .. } => prompt,
        }
    }
    pub fn validate(&self) -> Result<()> {
        crate::memory::model::text(self.text(), 8192, true).map_err(|_| "invalid_task")?;
        if let Self::CommandPrompt {
            project_alias: Some(v),
            ..
        } = self
        {
            alias(v)?;
        }
        Ok(())
    }
}
impl Payload {
    pub fn validate(&self) -> Result<()> {
        title(&self.title)?;
        self.trigger.validate()?;
        self.task.validate()
    }
    pub fn bindings(&self, home: &std::path::Path) -> Result<()> {
        if let Trigger::ProjectNoActiveSession { project_alias } = &self.trigger {
            crate::snapshot::validate_project_binding(home, project_alias)
                .map_err(|_| "invalid_input")?;
        }
        if let Task::CommandPrompt {
            project_alias: Some(v),
            ..
        } = &self.task
        {
            crate::snapshot::validate_project_binding(home, v).map_err(|_| "invalid_input")?;
        }
        Ok(())
    }
}
impl Definition {
    pub fn validate(&self) -> Result<()> {
        id(&self.id)?;
        id(&self.revision)?;
        self.payload.validate()?;
        timestamp(self.created_at)?;
        timestamp(self.updated_at)?;
        if self.version != 1 || self.updated_at < self.created_at {
            return Err("invalid_state");
        }
        if let Some(t) = self.cursor.last_due {
            timestamp(t)?;
        }
        Ok(())
    }
}
impl Item {
    pub fn validate(&self) -> Result<()> {
        id(&self.id)?;
        id(&self.automation_id)?;
        title(&self.automation_title)?;
        self.task.validate()?;
        timestamp(self.triggered_at)?;
        if self.version != 1
            || self.occurrence_key.is_empty()
            || self.occurrence_key.len() > 200
            || !self
                .occurrence_key
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b":/-+_".contains(&c))
        {
            return Err("invalid_state");
        }
        Ok(())
    }
}
impl Document {
    pub fn empty() -> Self {
        Self {
            version: 1,
            definitions: vec![],
            inbox: vec![],
        }
    }
    pub fn validate(&self) -> Result<()> {
        if self.version != 1 {
            return Err("invalid_state");
        }
        if self.definitions.len() > MAX_DEFINITIONS || self.inbox.len() > MAX_INBOX {
            return Err("limit");
        }
        let mut ids = std::collections::BTreeSet::new();
        for r in &self.definitions {
            r.validate()?;
            if !ids.insert(&r.id) {
                return Err("invalid_state");
            }
        }
        let mut items = std::collections::BTreeSet::new();
        let mut occurrences = std::collections::BTreeSet::new();
        for r in &self.inbox {
            r.validate()?;
            if !items.insert(&r.id) || !occurrences.insert((&r.automation_id, &r.occurrence_key)) {
                return Err("invalid_state");
            }
        }
        Ok(())
    }
    pub fn bytes(&mut self) -> Result<Vec<u8>> {
        self.validate()?;
        self.definitions.sort_by(|a, b| a.id.cmp(&b.id));
        let bytes = serde_json::to_vec(self).map_err(|_| "storage")?;
        if bytes.len() > MAX_BYTES {
            return Err("limit");
        }
        Ok(bytes)
    }
}
