use super::Result;
use chrono::{DateTime, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
pub const MAX_RECORDS: usize = 512;
pub const MAX_BYTES: usize = 2 * 1024 * 1024;
macro_rules! choices {
    ($name:ident { $($variant:ident),+ }) => {
        #[derive(Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Debug)]
        #[serde(rename_all="snake_case")]
        pub enum $name { $($variant),+ }
    };
}
choices!(Kind {
    Identity,
    Preference,
    Person,
    Project,
    Commitment,
    Decision,
    Fact
});
choices!(Sensitivity {
    Standard,
    Sensitive
});
choices!(Sharing {
    LocalOnly,
    ProviderAllowed
});
choices!(Status { Active, Archived });
choices!(ReviewFilter {
    Active,
    Archived,
    Expired,
    All
});
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Source {
    Manual {},
    ProjectReference {
        project_alias: String,
        relative_path: String,
    },
}
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Payload {
    pub kind: Kind,
    pub title: String,
    pub content: String,
    pub tags: Vec<String>,
    pub sensitivity: Sensitivity,
    pub sharing: Sharing,
    pub source: Source,
    pub expires_at: Option<String>,
}
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Record {
    pub version: u8,
    pub memory_id: String,
    pub payload: Payload,
    pub created_at: String,
    pub updated_at: String,
    pub status: Status,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Document {
    pub version: u8,
    pub records: Vec<Record>,
}
pub fn id(value: &str) -> Result<()> {
    let uuid = uuid::Uuid::parse_str(value).map_err(|_| "invalid_input")?;
    if uuid.get_version_num() != 4
        || uuid.get_variant() != uuid::Variant::RFC4122
        || uuid.to_string() != value
    {
        return Err("invalid_input");
    }
    Ok(())
}
pub fn time(value: &str) -> Result<DateTime<Utc>> {
    if value.len() > 40 {
        return Err("invalid_input");
    }
    let t = DateTime::parse_from_rfc3339(value).map_err(|_| "invalid_input")?;
    if t.timestamp() < 1 || t.timestamp() > 253402300799 {
        return Err("invalid_input");
    }
    Ok(t.with_timezone(&Utc))
}
pub fn iso(seconds: i64) -> Result<String> {
    DateTime::<Utc>::from_timestamp(seconds, 0)
        .map(|t| t.to_rfc3339_opts(SecondsFormat::Secs, true))
        .ok_or("invalid_input")
}
pub fn text(value: &str, max: usize, multiline: bool) -> Result<()> {
    if value.is_empty()
        || value.len() > max
        || value.trim().is_empty()
        || value
            .chars()
            .any(|c| c.is_control() && !(multiline && c == '\n'))
    {
        return Err("invalid_input");
    }
    if crate::snapshot::text::redact(value) != value {
        return Err("secret_rejected");
    }
    Ok(())
}
impl Payload {
    pub fn normalized(&self) -> Result<Self> {
        // Validate controls before trimming; normalization must not hide rejected input.
        for (value, multiline) in [(self.title.as_str(), false), (self.content.as_str(), true)] {
            if value
                .chars()
                .any(|c| c.is_control() && !(multiline && c == '\n'))
            {
                return Err("invalid_input");
            }
        }
        if self.tags.iter().any(|t| t.chars().any(char::is_control)) {
            return Err("invalid_input");
        }
        if self.title.len() > 640
            || self.content.len() > 8192
            || self.tags.len() > 12
            || self.tags.iter().any(|t| t.len() > 128)
        {
            return Err("invalid_input");
        }
        let mut p = self.clone();
        p.title = p.title.trim().to_owned();
        p.content = p.content.trim().to_owned();
        text(&p.title, 640, false)?;
        if p.title.chars().count() > 160 {
            return Err("invalid_input");
        }
        text(&p.content, 8192, true)?;
        if p.tags.len() > 12 {
            return Err("invalid_input");
        }
        let mut seen = std::collections::BTreeSet::new();
        for tag in &mut p.tags {
            *tag = tag.trim().to_owned();
            text(tag, 128, false)?;
            if tag.chars().count() > 32 || !seen.insert(tag.to_lowercase()) {
                return Err("invalid_input");
            }
        }
        p.tags.sort();
        if p.sensitivity == Sensitivity::Sensitive && p.sharing != Sharing::LocalOnly {
            return Err("invalid_privacy");
        }
        if let Some(t) = &p.expires_at {
            p.expires_at = Some(time(t)?.to_rfc3339_opts(SecondsFormat::AutoSi, true));
        }
        if let Source::ProjectReference {
            project_alias,
            relative_path,
        } = &p.source
        {
            text(project_alias, 128, false)?;
            text(relative_path, 512, false)?;
            if !crate::snapshot::memory_reference_path(project_alias, relative_path) {
                return Err("invalid_source");
            }
        }
        Ok(p)
    }
    pub fn duplicate_key(&self) -> String {
        let mut tags: Vec<_> = self.tags.iter().map(|t| t.to_lowercase()).collect();
        tags.sort();
        serde_json::to_string(&(
            self.kind,
            self.title
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
                .to_lowercase(),
            self.content
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
                .to_lowercase(),
            tags,
        ))
        .expect("fixed schema")
    }
}
impl Record {
    pub fn validate(&self) -> Result<()> {
        id(&self.memory_id)?;
        if self.version != 1
            || self.payload.normalized()? != self.payload
            || time(&self.updated_at)? < time(&self.created_at)?
            || self.payload.expires_at.as_ref().is_some_and(|t| {
                time(t).map_or(true, |t| {
                    t < time(&self.created_at).unwrap_or(DateTime::<Utc>::MAX_UTC)
                })
            })
        {
            return Err("invalid_input");
        }
        Ok(())
    }
    pub fn expired(&self, now: i64) -> bool {
        self.payload.expires_at.as_ref().is_some_and(|t| {
            time(t).map_or(true, |t| {
                t.timestamp() <= now
                    && t <= DateTime::<Utc>::from_timestamp(now, 0)
                        .unwrap_or(DateTime::<Utc>::MAX_UTC)
            })
        })
    }
    // Jarvis may consume this policy only through its separate reviewed outbound request.
    pub fn eligible_for_provider(&self, now: i64) -> bool {
        self.status == Status::Active
            && !self.expired(now)
            && self.payload.sensitivity == Sensitivity::Standard
            && self.payload.sharing == Sharing::ProviderAllowed
    }
}
impl Document {
    pub fn empty() -> Self {
        Self {
            version: 1,
            records: Vec::new(),
        }
    }
    pub fn validate(&self) -> Result<()> {
        if self.version != 1 || self.records.len() > MAX_RECORDS {
            return Err("corrupt_memory");
        }
        let mut ids = std::collections::BTreeSet::new();
        let mut duplicates = std::collections::BTreeSet::new();
        for r in &self.records {
            r.validate().map_err(|_| "corrupt_memory")?;
            if !ids.insert(&r.memory_id)
                || (r.status == Status::Active && !duplicates.insert(r.payload.duplicate_key()))
            {
                return Err("corrupt_memory");
            }
        }
        Ok(())
    }
    pub fn bytes(&mut self) -> Result<Vec<u8>> {
        self.validate()?;
        self.records.sort_by(|a, b| a.memory_id.cmp(&b.memory_id));
        let bytes = serde_json::to_vec(self).map_err(|_| "storage_failed")?;
        if bytes.len() > MAX_BYTES {
            return Err("memory_limit");
        }
        Ok(bytes)
    }
}
