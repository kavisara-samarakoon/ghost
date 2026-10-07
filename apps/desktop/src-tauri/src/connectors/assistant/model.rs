use crate::credentials::AccountId;
use base64::{
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
    Engine,
};
use chrono::{DateTime, NaiveDate, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};

pub type Result<T> = std::result::Result<T, &'static str>;
pub const MAX_MESSAGES: usize = 20;
pub const MAX_EVENTS: usize = 50;
pub const MAX_CONTACTS: usize = 100;

pub fn input_text(value: &str, max: usize, multiline: bool) -> Result<()> {
    if value.len() > max
        || value
            .chars()
            .any(|c| c.is_control() && !(multiline && matches!(c, '\n' | '\t')))
        || regex::Regex::new(r"\p{Cf}")
            .expect("fixed pattern")
            .is_match(value)
    {
        return Err("invalid_input");
    }
    Ok(())
}
pub fn clean(value: &str, max: usize) -> String {
    let text = crate::snapshot::text::redact(value);
    let text: String = text.chars().filter(|c| !c.is_control()).collect();
    let mut end = text.len().min(max);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].trim().to_owned()
}
pub fn identifier(value: &str) -> Result<String> {
    if value.is_empty()
        || value.len() > 512
        || !value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-'))
        || crate::snapshot::text::redact(value) != value
    {
        return Err("invalid_response");
    }
    Ok(value.into())
}
pub fn mail_id(value: &str) -> Result<String> {
    if value.is_empty() || value.len() > 64 || !value.bytes().all(|c| c.is_ascii_hexdigit()) {
        return Err("invalid_response");
    }
    Ok(value.into())
}
pub fn etag(value: &str) -> Result<String> {
    if value.len() < 3
        || value.len() > 256
        || !value.starts_with('"')
        || !value.ends_with('"')
        || !value.bytes().all(|b| (0x21..=0x7e).contains(&b))
        || value.contains("[REDACTED]")
        || crate::snapshot::text::redact(value) != value
    {
        return Err("invalid_response");
    }
    Ok(value.into())
}
pub fn address(value: &str) -> Result<String> {
    if value.len() > 254 || !value.is_ascii() {
        return Err("invalid_input");
    }
    let (local, domain) = value.split_once('@').ok_or("invalid_input")?;
    if local.is_empty()
        || local.len() > 64
        || local.starts_with('.')
        || local.ends_with('.')
        || local.contains("..")
        || !local
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b".!#$%&'*+-/=?^_`{|}~".contains(&c))
        || !domain.contains('.')
        || domain.split('.').any(|part| {
            part.is_empty()
                || part.len() > 63
                || part.starts_with('-')
                || part.ends_with('-')
                || !part.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-')
        })
    {
        return Err("invalid_input");
    }
    Ok(format!("{local}@{}", domain.to_ascii_lowercase()))
}
#[derive(Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct MailInput {
    pub to: Vec<String>,
    pub cc: Vec<String>,
    pub subject: String,
    pub body: String,
}
impl MailInput {
    pub fn normalized(&self) -> Result<Self> {
        if self.to.is_empty() || self.to.len() + self.cc.len() > 5 {
            return Err("invalid_input");
        }
        let to = self
            .to
            .iter()
            .map(|s| {
                input_text(s, 254, false)?;
                address(s.trim())
            })
            .collect::<Result<Vec<_>>>()?;
        let cc = self
            .cc
            .iter()
            .map(|s| {
                input_text(s, 254, false)?;
                address(s.trim())
            })
            .collect::<Result<Vec<_>>>()?;
        let mut seen = std::collections::BTreeSet::new();
        for v in to.iter().chain(cc.iter()) {
            if !seen.insert(v.to_ascii_lowercase()) {
                return Err("invalid_input");
            }
        }
        input_text(&self.subject, 256, false)?;
        let subject = self.subject.trim().to_owned();
        if subject.is_empty() {
            return Err("invalid_input");
        }
        let body = self.body.replace("\r\n", "\n");
        input_text(&body, 32768, true)?;
        if body.trim().is_empty() {
            return Err("invalid_input");
        }
        Ok(Self {
            to,
            cc,
            subject,
            body,
        })
    }
    pub fn raw(&self) -> Result<String> {
        self.raw_at(None, 0)
    }
    pub fn raw_at(&self, sender: Option<&str>, created_at: u64) -> Result<String> {
        let mail = self.normalized()?;
        // RFC 2047 encoded words are folded below 75 characters at UTF-8 boundaries.
        let mut words = Vec::new();
        let mut chunk = String::new();
        for c in mail.subject.chars() {
            if chunk.len() + c.len_utf8() > 42 {
                words.push(format!("=?UTF-8?B?{}?=", STANDARD.encode(chunk.as_bytes())));
                chunk.clear();
            }
            chunk.push(c);
        }
        if !chunk.is_empty() {
            words.push(format!("=?UTF-8?B?{}?=", STANDARD.encode(chunk.as_bytes())));
        }
        let date = chrono::DateTime::<Utc>::from_timestamp(
            i64::try_from(created_at).map_err(|_| "invalid_input")?,
            0,
        )
        .ok_or("invalid_input")?
        .to_rfc2822();
        let mut mime = format!("Date: {date}\r\n");
        if let Some(sender) = sender {
            mime.push_str(&format!("From: {}\r\n", address(sender)?));
        }
        // No caller-controlled sender: the `me` mailbox is authenticated by Google.
        mime.push_str(&format!("To: {}\r\n", mail.to.join(",\r\n ")));
        if !mail.cc.is_empty() {
            mime.push_str(&format!("Cc: {}\r\n", mail.cc.join(",\r\n ")));
        }
        mime.push_str(&format!("Subject: {}\r\nMIME-Version: 1.0\r\nContent-Type: text/plain; charset=UTF-8\r\nContent-Transfer-Encoding: base64\r\n\r\n",words.join("\r\n ")));
        let encoded = STANDARD.encode(mail.body.replace('\n', "\r\n").as_bytes());
        for line in encoded.as_bytes().chunks(76) {
            mime.push_str(std::str::from_utf8(line).map_err(|_| "invalid_input")?);
            mime.push_str("\r\n");
        }
        Ok(URL_SAFE_NO_PAD.encode(mime.as_bytes()))
    }
}
pub fn timestamp(value: &str) -> Result<DateTime<Utc>> {
    if value.len() > 40 {
        return Err("invalid_input");
    }
    DateTime::parse_from_rfc3339(value)
        .map(|dt| dt.with_timezone(&Utc))
        .map_err(|_| "invalid_input")
}
pub fn iso(value: DateTime<Utc>) -> String {
    value.to_rfc3339_opts(SecondsFormat::AutoSi, true)
}
#[derive(Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Window {
    pub start: String,
    pub end: String,
}
impl Window {
    pub fn normalized(&self) -> Result<Self> {
        let start = timestamp(&self.start)?;
        let end = timestamp(&self.end)?;
        if start >= end || (end - start).num_seconds() > 31 * 86400 {
            return Err("invalid_input");
        }
        Ok(Self {
            start: iso(start),
            end: iso(end),
        })
    }
}
#[derive(Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct EventTime {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub date_time: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub date: Option<String>,
}
impl EventTime {
    pub fn normalized(&self) -> Result<Self> {
        match (&self.date_time, &self.date) {
            (Some(value), None) => Ok(Self {
                date_time: Some(iso(timestamp(value)?)),
                date: None,
            }),
            (None, Some(value)) if value.len() == 10 => {
                let date =
                    NaiveDate::parse_from_str(value, "%Y-%m-%d").map_err(|_| "invalid_input")?;
                if date.format("%Y-%m-%d").to_string() != *value {
                    return Err("invalid_input");
                }
                Ok(self.clone())
            }
            _ => Err("invalid_input"),
        }
    }
    pub fn instant(&self) -> Result<DateTime<Utc>> {
        match (&self.date_time, &self.date) {
            (Some(value), None) => timestamp(value),
            (None, Some(value)) => Ok(NaiveDate::parse_from_str(value, "%Y-%m-%d")
                .map_err(|_| "invalid_input")?
                .and_hms_opt(0, 0, 0)
                .ok_or("invalid_input")?
                .and_utc()),
            _ => Err("invalid_input"),
        }
    }
    pub fn provider(&self) -> serde_json::Value {
        match &self.date_time {
            Some(value) => serde_json::json!({"dateTime":value}),
            None => serde_json::json!({"date":self.date}),
        }
    }
}
#[derive(Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct EventInput {
    pub summary: String,
    pub description: Option<String>,
    pub location: Option<String>,
    pub start: EventTime,
    pub end: EventTime,
}
impl EventInput {
    pub fn normalized(&self, now: DateTime<Utc>) -> Result<Self> {
        input_text(&self.summary, 256, false)?;
        if self.summary.trim().is_empty() {
            return Err("invalid_input");
        }
        if let Some(s) = &self.description {
            input_text(s, 4096, true)?;
        }
        if let Some(s) = &self.location {
            input_text(s, 512, false)?;
        }
        let start = self.start.normalized()?;
        let end = self.end.normalized()?;
        let a = start.instant()?;
        let b = end.instant()?;
        if start.date.is_some() != end.date.is_some()
            || a >= b
            || (b - a).num_seconds() > 31 * 86400
            || a < now - chrono::Duration::days(30)
            || b > now + chrono::Duration::days(366)
        {
            return Err("invalid_input");
        }
        Ok(Self {
            summary: self.summary.trim().into(),
            description: self.description.clone(),
            location: self.location.clone(),
            start,
            end,
        })
    }
    pub fn provider(&self) -> serde_json::Value {
        let mut value = serde_json::json!({"summary":self.summary,"start":self.start.provider(),"end":self.end.provider()});
        if let Some(s) = &self.description {
            value["description"] = serde_json::json!(s);
        }
        if let Some(s) = &self.location {
            value["location"] = serde_json::json!(s);
        }
        value
    }
}
#[derive(Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct EventChanges {
    pub summary: Option<String>,
    pub description: Option<String>,
    pub location: Option<String>,
    pub start: Option<EventTime>,
    pub end: Option<EventTime>,
}
impl EventChanges {
    pub fn apply(&self, old: &EventInput, now: DateTime<Utc>) -> Result<EventInput> {
        if self.summary.is_none()
            && self.description.is_none()
            && self.location.is_none()
            && self.start.is_none()
            && self.end.is_none()
        {
            return Err("invalid_input");
        }
        EventInput {
            summary: self.summary.clone().unwrap_or_else(|| old.summary.clone()),
            description: self.description.clone().or_else(|| old.description.clone()),
            location: self.location.clone().or_else(|| old.location.clone()),
            start: self.start.clone().unwrap_or_else(|| old.start.clone()),
            end: self.end.clone().unwrap_or_else(|| old.end.clone()),
        }
        .normalized(now)
    }
    pub fn normalized(&self, new: &EventInput) -> Self {
        Self {
            summary: self.summary.as_ref().map(|_| new.summary.clone()),
            description: self
                .description
                .as_ref()
                .map(|_| new.description.clone().unwrap_or_default()),
            location: self
                .location
                .as_ref()
                .map(|_| new.location.clone().unwrap_or_default()),
            start: self.start.as_ref().map(|_| new.start.clone()),
            end: self.end.as_ref().map(|_| new.end.clone()),
        }
    }
    pub fn provider(&self) -> serde_json::Value {
        let mut value = serde_json::Map::new();
        if let Some(s) = &self.summary {
            value.insert("summary".into(), serde_json::json!(s));
        }
        if let Some(s) = &self.description {
            value.insert("description".into(), serde_json::json!(s));
        }
        if let Some(s) = &self.location {
            value.insert("location".into(), serde_json::json!(s));
        }
        if let Some(s) = &self.start {
            value.insert("start".into(), s.provider());
        }
        if let Some(s) = &self.end {
            value.insert("end".into(), s.provider());
        }
        serde_json::Value::Object(value)
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MailSearch {
    pub account_id: AccountId,
    pub query: String,
    pub limit: usize,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgendaInput {
    pub account_id: AccountId,
    pub window: Window,
    pub limit: usize,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FreeTimeInput {
    pub account_id: AccountId,
    pub window: Window,
    pub duration_minutes: u32,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContactsInput {
    pub account_id: AccountId,
    pub query: String,
}

#[derive(Serialize)]
pub struct MailMessage {
    pub message_id: String,
    pub thread_id: String,
    pub from: String,
    pub subject: String,
    pub date: String,
    pub snippet: String,
    pub unread: bool,
    pub important: bool,
    pub timestamp_ms: i64,
}
#[derive(Serialize)]
pub struct MailResult {
    pub messages: Vec<MailMessage>,
    pub unread_in_results: usize,
    pub truncated: bool,
    pub digest: String,
}
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CalendarEvent {
    pub event_id: String,
    pub etag: String,
    pub fields: EventInput,
    pub status: String,
}
#[derive(Serialize)]
pub struct AgendaResult {
    pub events: Vec<CalendarEvent>,
    pub truncated: bool,
}
#[derive(Serialize, Clone, PartialEq, Eq)]
pub struct Interval {
    pub start: String,
    pub end: String,
}
#[derive(Serialize)]
pub struct FreeTimeResult {
    pub busy: Vec<Interval>,
    pub free: Vec<Interval>,
    pub candidates: Vec<Interval>,
}
#[derive(Serialize)]
pub struct Contact {
    pub resource_name: String,
    pub display_name: String,
    pub emails: Vec<String>,
    pub phones: Vec<String>,
    pub organization: Option<String>,
}
#[derive(Serialize)]
pub struct ContactResult {
    pub contacts: Vec<Contact>,
    pub truncated: bool,
}
