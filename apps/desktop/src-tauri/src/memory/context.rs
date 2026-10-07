use super::{hash, model::*, search, Result};
use crate::connectors::assistant::{
    model::{AgendaResult, ContactResult, MailResult, Window},
    runtime::AssistantState,
};
use crate::credentials::AccountId;
use serde::{Deserialize, Serialize};
pub const MAX_ITEMS: usize = 24;
pub const MAX_CONTEXT_BYTES: usize = 48 * 1024;
#[derive(Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Sources {
    pub personal_memory: bool,
    pub project_memory: bool,
    pub gmail: bool,
    pub calendar: bool,
    pub contacts: bool,
}
impl Default for Sources {
    fn default() -> Self {
        Self {
            personal_memory: true,
            project_memory: true,
            gmail: false,
            calendar: false,
            contacts: false,
        }
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Input {
    pub query: String,
    #[serde(default)]
    pub sources: Sources,
    pub project_alias: Option<String>,
    pub account_id: Option<AccountId>,
    pub calendar_window: Option<Window>,
}
#[derive(Serialize)]
pub struct Item {
    pub source: &'static str,
    pub kind: String,
    pub title: String,
    pub content: String,
    pub timestamp: Option<String>,
    pub reference: String,
    pub project_alias: Option<String>,
    pub account_id: Option<AccountId>,
    pub sensitivity: Option<Sensitivity>,
    pub sharing: Option<Sharing>,
    pub instruction_trust: &'static str,
    pub score: u32,
}
#[derive(Serialize)]
pub struct Pack {
    pub version: u8,
    pub query: String,
    pub created_at: String,
    pub sources: Sources,
    pub project_alias: Option<String>,
    pub account_id: Option<AccountId>,
    pub calendar_window: Option<Window>,
    pub items: Vec<Item>,
    pub warnings: Vec<String>,
    pub truncated: bool,
    pub total_bytes: usize,
    pub context_sha256: String,
}
pub type GoogleReads = (
    Option<MailResult>,
    Option<AgendaResult>,
    Option<ContactResult>,
);
pub fn build(
    home: &std::path::Path,
    records: &[Record],
    input: Input,
    now: i64,
    state: &AssistantState,
) -> Result<Pack> {
    build_with(home, records, input, now, |input| {
        crate::connectors::assistant::ipc::read_context(
            state,
            input.account_id.ok_or("invalid_input")?,
            &input.query,
            input.sources.gmail,
            input.sources.calendar,
            input.sources.contacts,
            input.calendar_window.clone(),
        )
    })
}
pub fn build_with(
    home: &std::path::Path,
    records: &[Record],
    input: Input,
    now: i64,
    read: impl FnOnce(&Input) -> Result<GoogleReads>,
) -> Result<Pack> {
    let q = search::query(&input.query)?;
    if input
        .project_alias
        .as_ref()
        .is_some_and(|a| !crate::snapshot::memory_reference_path(a, "status.md"))
    {
        return Err("invalid_source");
    }
    if input.sources.contacts {
        crate::connectors::assistant::model::input_text(&q, 128, false)?;
    }
    let live = input.sources.gmail || input.sources.calendar || input.sources.contacts;
    if live && input.account_id.is_none() {
        return Err("invalid_input");
    }
    if input.sources.calendar {
        input
            .calendar_window
            .as_ref()
            .ok_or("invalid_input")?
            .normalized()?;
    }
    let mut items = Vec::new();
    let mut warnings = Vec::new();
    let mut truncated = false;
    if input.sources.personal_memory {
        let hits = search::search(
            records,
            &search::SearchInput {
                query: q.clone(),
                limit: 20,
            },
            now,
        )?;
        if hits.len() > 8 {
            truncated = true;
            warnings.push("Personal memory capped at 8 items.".into());
        }
        for h in hits.into_iter().take(8) {
            items.push(Item {
                source: "personal_memory",
                kind: serde_json::to_value(h.kind)
                    .unwrap()
                    .as_str()
                    .unwrap()
                    .into(),
                title: h.title,
                content: h.excerpt,
                timestamp: Some(h.updated_at),
                reference: h.memory_id,
                project_alias: None,
                account_id: None,
                sensitivity: Some(h.sensitivity),
                sharing: Some(h.sharing),
                instruction_trust: "data_only",
                score: h.score,
            });
        }
    }
    if input.sources.project_memory {
        let mut response = crate::snapshot::search::load(
            home,
            &q,
            input.project_alias.as_deref(),
            &mut crate::snapshot::reader::ReadBudget::search(),
        );
        response.results.sort_by(|a, b| {
            search::score(&b.title, &b.snippet, &[], b.kind, &q)
                .cmp(&search::score(&a.title, &a.snippet, &[], a.kind, &q))
                .then_with(|| b.created_at.cmp(&a.created_at))
                .then_with(|| a.project_alias.cmp(&b.project_alias))
                .then_with(|| a.relative_path.cmp(&b.relative_path))
        });
        truncated |= response
            .warnings
            .iter()
            .any(|w| w.contains("budget reached"));
        warnings.extend(response.warnings);
        if response.results.len() > 8 {
            truncated = true;
            warnings.push("Project memory capped at 8 items.".into());
        }
        for h in response.results.into_iter().take(8) {
            let score = search::score(&h.title, &h.snippet, &[], h.kind, &q);
            items.push(Item {
                source: "project_memory",
                kind: h.kind.into(),
                title: h.title,
                content: h.snippet,
                timestamp: h.created_at,
                reference: h.relative_path,
                project_alias: Some(h.project_alias),
                account_id: None,
                sensitivity: None,
                sharing: None,
                instruction_trust: "data_only",
                score,
            });
        }
    }
    if live {
        let (mail, agenda, contacts) = read(&input)?;
        if let Some(result) = mail {
            truncated |= result.truncated || result.messages.len() > 5;
            for m in result.messages.into_iter().take(5) {
                let timestamp =
                    chrono::DateTime::<chrono::Utc>::from_timestamp_millis(m.timestamp_ms)
                        .map(|t| t.to_rfc3339_opts(chrono::SecondsFormat::AutoSi, true));
                items.push(external(
                    "google_mail",
                    "mail",
                    m.subject,
                    format!("{} · {}\n{}", m.from, m.date, m.snippet),
                    m.message_id,
                    input.account_id,
                    timestamp,
                    &q,
                ));
            }
        }
        if let Some(result) = agenda {
            truncated |= result.truncated || result.events.len() > 5;
            for e in result.events.into_iter().take(5) {
                items.push(external(
                    "google_calendar",
                    "event",
                    e.fields.summary,
                    format!(
                        "{} — {}\n{}\n{}",
                        serde_json::to_string(&e.fields.start).unwrap(),
                        serde_json::to_string(&e.fields.end).unwrap(),
                        e.fields.location.unwrap_or_default(),
                        e.fields.description.unwrap_or_default()
                    ),
                    e.event_id,
                    input.account_id,
                    e.fields.start.date_time.or(e.fields.start.date),
                    &q,
                ));
            }
        }
        if let Some(result) = contacts {
            truncated |= result.truncated || result.contacts.len() > 5;
            for c in result.contacts.into_iter().take(5) {
                items.push(external(
                    "google_contact",
                    "contact",
                    c.display_name,
                    format!(
                        "{}\n{}\n{}",
                        c.emails.join(", "),
                        c.phones.join(", "),
                        c.organization.unwrap_or_default()
                    ),
                    c.resource_name,
                    input.account_id,
                    None,
                    &q,
                ));
            }
        }
    }
    let project_alias = if input.sources.project_memory {
        input.project_alias
    } else {
        None
    };
    let account_id = if live { input.account_id } else { None };
    let calendar_window = if input.sources.calendar {
        Some(input.calendar_window.ok_or("invalid_input")?.normalized()?)
    } else {
        None
    };
    let mut pack = finish(q, iso(now)?, input.sources, items, warnings, truncated)?;
    pack.project_alias = project_alias;
    pack.account_id = account_id;
    pack.calendar_window = calendar_window;
    pack.seal()?;
    Ok(pack)
}
fn external(
    source: &'static str,
    kind: &str,
    title: String,
    content: String,
    reference: String,
    account_id: Option<AccountId>,
    timestamp: Option<String>,
    query: &str,
) -> Item {
    let clean = crate::connectors::assistant::model::clean;
    let title = clean(&title, 640);
    let content = clean(&content, 4096);
    Item {
        source,
        kind: kind.into(),
        score: search::score(&title, &content, &[], kind, query),
        title,
        content,
        reference: clean(&reference, 512),
        account_id,
        timestamp,
        project_alias: None,
        sensitivity: None,
        sharing: None,
        instruction_trust: "data_only",
    }
}
pub fn finish(
    query: String,
    created_at: String,
    sources: Sources,
    mut items: Vec<Item>,
    mut warnings: Vec<String>,
    mut truncated: bool,
) -> Result<Pack> {
    for item in &mut items {
        item.instruction_trust = "data_only";
    }
    items.sort_by(|a, b| {
        b.score
            .cmp(&a.score)
            .then_with(|| priority(a.source).cmp(&priority(b.source)))
            .then_with(|| sort_time(&b.timestamp).cmp(&sort_time(&a.timestamp)))
            .then_with(|| a.project_alias.cmp(&b.project_alias))
            .then_with(|| a.reference.cmp(&b.reference))
    });
    if items.len() > MAX_ITEMS {
        items.truncate(MAX_ITEMS);
        truncated = true;
    }
    let mut bytes = query.len();
    let mut kept = Vec::new();
    for item in items {
        let size = serde_json::to_vec(&item)
            .map_err(|_| "invalid_input")?
            .len();
        if bytes + size > MAX_CONTEXT_BYTES - 4096 {
            truncated = true;
            continue;
        }
        bytes += size;
        kept.push(item);
    }
    if truncated {
        warnings.push("Context truncated to source, item or byte limits.".into());
    }
    warnings.truncate(16);
    let mut p = Pack {
        version: 1,
        query,
        created_at,
        sources,
        project_alias: None,
        account_id: None,
        calendar_window: None,
        items: kept,
        warnings,
        truncated,
        total_bytes: bytes,
        context_sha256: String::new(),
    };
    p.seal()?;
    Ok(p)
}
fn priority(source: &str) -> u8 {
    match source {
        "personal_memory" => 0,
        "project_memory" => 1,
        "google_mail" => 2,
        "google_calendar" => 3,
        _ => 4,
    }
}

fn sort_time(value: &Option<String>) -> Option<chrono::DateTime<chrono::Utc>> {
    let value = value.as_ref()?;
    chrono::DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|t| t.with_timezone(&chrono::Utc))
        .or_else(|| {
            chrono::NaiveDate::parse_from_str(value, "%Y-%m-%d")
                .ok()?
                .and_hms_opt(0, 0, 0)
                .map(|t| t.and_utc())
        })
}

impl Pack {
    fn seal(&mut self) -> Result<()> {
        // Hash the final normalized pack with only the hash field cleared, including source scope.
        self.context_sha256.clear();
        self.context_sha256 = hash(self)?;
        Ok(())
    }
}
