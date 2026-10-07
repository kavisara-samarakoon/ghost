use super::model::*;
use serde_json::Value;

pub fn string<'a>(v: &'a Value, name: &str) -> Result<&'a str> {
    v.get(name)
        .and_then(Value::as_str)
        .ok_or("invalid_response")
}
fn optional(v: &Value, name: &str, bound: usize) -> Result<Option<String>> {
    match v.get(name) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(clean(s, bound))),
        _ => Err("invalid_response"),
    }
}
pub fn mail_metadata(v: &Value, expected: &str) -> Result<MailMessage> {
    let id = mail_id(string(v, "id")?)?;
    if id != expected {
        return Err("invalid_response");
    }
    let thread_id = mail_id(string(v, "threadId")?)?;
    let timestamp_ms = string(v, "internalDate")?
        .parse::<i64>()
        .map_err(|_| "invalid_response")?;
    let date = chrono::DateTime::<chrono::Utc>::from_timestamp_millis(timestamp_ms)
        .ok_or("invalid_response")?;
    let headers = v
        .pointer("/payload/headers")
        .and_then(Value::as_array)
        .ok_or("invalid_response")?;
    if headers.len() > 30 {
        return Err("invalid_response");
    }
    let mut from = String::new();
    let mut subject = String::new();
    let mut seen = std::collections::BTreeSet::new();
    for header in headers {
        let name = string(header, "name")?.to_ascii_lowercase();
        let value = string(header, "value")?;
        if value.len() > 8192 {
            return Err("invalid_response");
        }
        if matches!(name.as_str(), "from" | "subject" | "date") && !seen.insert(name.clone()) {
            return Err("invalid_response");
        }
        match name.as_str() {
            "from" => from = clean(value, 256),
            "subject" => subject = clean(value, 512),
            _ => (),
        }
    }
    let labels = v
        .get("labelIds")
        .and_then(Value::as_array)
        .ok_or("invalid_response")?;
    if labels.len() > 100 || labels.iter().any(|x| !x.is_string()) {
        return Err("invalid_response");
    }
    Ok(MailMessage {
        message_id: id,
        thread_id,
        from,
        subject,
        date: iso(date),
        snippet: optional(v, "snippet", 1024)?.unwrap_or_default(),
        unread: labels.iter().any(|x| x == "UNREAD"),
        important: labels.iter().any(|x| x == "IMPORTANT"),
        timestamp_ms,
    })
}
pub fn mail_result(mut messages: Vec<MailMessage>, truncated: bool) -> MailResult {
    messages.sort_by(|a, b| {
        b.timestamp_ms
            .cmp(&a.timestamp_ms)
            .then(a.message_id.cmp(&b.message_id))
    });
    let unread = messages.iter().filter(|m| m.unread).count();
    let mut digest = format!(
        "{} recent results; {} unread in these results. Local metadata/snippet digest.\n",
        messages.len(),
        unread
    );
    for m in &messages {
        digest.push_str(&format!(
            "\n{} · {}\n{}{}\n{}\n",
            m.date,
            m.from,
            if m.important {
                "Important label · "
            } else {
                ""
            },
            m.subject,
            m.snippet
        ));
    }
    MailResult {
        messages,
        unread_in_results: unread,
        truncated,
        digest,
    }
}
fn event_time(v: &Value) -> Result<EventTime> {
    let date_time = v
        .get("dateTime")
        .map(|v| v.as_str().map(str::to_owned).ok_or("invalid_response"))
        .transpose()?;
    let date = v
        .get("date")
        .map(|v| v.as_str().map(str::to_owned).ok_or("invalid_response"))
        .transpose()?;
    EventTime { date_time, date }
        .normalized()
        .map_err(|_| "invalid_response")
}
pub fn event(v: &Value, full: bool) -> Result<CalendarEvent> {
    let start = event_time(v.get("start").ok_or("invalid_response")?)?;
    let end = event_time(v.get("end").ok_or("invalid_response")?)?;
    if start.date.is_some() != end.date.is_some() || start.instant()? >= end.instant()? {
        return Err("invalid_response");
    }
    for (field, bound) in [("summary", 256), ("description", 4096), ("location", 512)] {
        if full
            && v.get(field)
                .and_then(Value::as_str)
                .is_some_and(|s| s.len() > bound)
        {
            return Err("invalid_response");
        }
    }
    Ok(CalendarEvent {
        event_id: identifier(string(v, "id")?)?,
        etag: etag(string(v, "etag")?)?,
        status: clean(string(v, "status")?, 32),
        fields: EventInput {
            summary: optional(v, "summary", 256)?.unwrap_or_default(),
            description: optional(v, "description", if full { 4096 } else { 512 })?,
            location: optional(v, "location", 512)?,
            start,
            end,
        },
    })
}
pub fn agenda(v: &Value, limit: usize) -> Result<AgendaResult> {
    let items = v
        .get("items")
        .and_then(Value::as_array)
        .ok_or("invalid_response")?;
    if items.len() > limit {
        return Err("invalid_response");
    }
    let mut events = items
        .iter()
        .map(|v| event(v, false))
        .collect::<Result<Vec<_>>>()?;
    events.sort_by_key(|e| e.fields.start.instant().ok());
    Ok(AgendaResult {
        events,
        truncated: v.get("nextPageToken").is_some_and(Value::is_string),
    })
}
pub fn free_time(v: &Value, window: &Window, minutes: u32) -> Result<FreeTimeResult> {
    if !(5..=480).contains(&minutes) {
        return Err("invalid_input");
    }
    let window = window.normalized()?;
    let start = timestamp(&window.start)?;
    let end = timestamp(&window.end)?;
    let calendar = v.pointer("/calendars/primary").ok_or("invalid_response")?;
    match calendar.get("errors") {
        None | Some(Value::Null) => (),
        Some(Value::Array(errors)) if errors.is_empty() => (),
        Some(Value::Array(_)) => return Err("provider_failed"),
        _ => return Err("invalid_response"),
    }
    let items = calendar
        .get("busy")
        .and_then(Value::as_array)
        .ok_or("invalid_response")?;
    if items.len() > 200 {
        return Err("invalid_response");
    }
    let mut periods = Vec::new();
    for item in items {
        let a = timestamp(string(item, "start")?).map_err(|_| "invalid_response")?;
        let b = timestamp(string(item, "end")?).map_err(|_| "invalid_response")?;
        if a >= b {
            return Err("invalid_response");
        }
        let a = a.max(start);
        let b = b.min(end);
        if a < b {
            periods.push((a, b));
        }
    }
    periods.sort();
    let mut merged: Vec<(chrono::DateTime<chrono::Utc>, chrono::DateTime<chrono::Utc>)> =
        Vec::new();
    for (a, b) in periods {
        if let Some(last) = merged.last_mut() {
            if a <= last.1 {
                last.1 = last.1.max(b);
                continue;
            }
        }
        merged.push((a, b));
    }
    let mut cursor = start;
    let mut free = Vec::new();
    for (a, b) in &merged {
        if cursor < *a {
            free.push(Interval {
                start: iso(cursor),
                end: iso(*a),
            });
        }
        cursor = cursor.max(*b);
    }
    if cursor < end {
        free.push(Interval {
            start: iso(cursor),
            end: iso(end),
        });
    }
    let mut candidates = Vec::new();
    for f in &free {
        let a = timestamp(&f.start)?;
        let b = timestamp(&f.end)?;
        if (b - a).num_minutes() >= minutes as i64 && candidates.len() < 20 {
            candidates.push(Interval {
                start: iso(a),
                end: iso(a + chrono::Duration::minutes(minutes as i64)),
            });
        }
    }
    Ok(FreeTimeResult {
        busy: merged
            .into_iter()
            .map(|(a, b)| Interval {
                start: iso(a),
                end: iso(b),
            })
            .collect(),
        free,
        candidates,
    })
}
pub fn contacts(v: &Value, query: &str) -> Result<ContactResult> {
    input_text(query, 128, false)?;
    let query = query.trim().to_lowercase();
    let empty = Vec::new();
    let connections = match v.get("connections") {
        None => &empty,
        Some(v) => v.as_array().ok_or("invalid_response")?,
    };
    if connections.len() > MAX_CONTACTS {
        return Err("invalid_response");
    }
    fn values(v: &Value, field: &str, key: &str, max: usize) -> Result<Vec<String>> {
        match v.get(field) {
            None => Ok(Vec::new()),
            Some(v) => {
                let array = v.as_array().ok_or("invalid_response")?;
                if array.len() > 50 {
                    return Err("invalid_response");
                }
                array
                    .iter()
                    .take(5)
                    .map(|v| Ok(clean(string(v, key)?, max)))
                    .collect()
            }
        }
    }
    let mut contacts = Vec::new();
    for c in connections {
        let resource = string(c, "resourceName")?;
        let id = resource.strip_prefix("people/").ok_or("invalid_response")?;
        identifier(id)?;
        let name = values(c, "names", "displayName", 256)?
            .into_iter()
            .next()
            .unwrap_or_default();
        let emails = values(c, "emailAddresses", "value", 254)?;
        let phones = values(c, "phoneNumbers", "value", 64)?;
        let organization = values(c, "organizations", "name", 128)?.into_iter().next();
        if query.is_empty()
            || std::iter::once(&name)
                .chain(emails.iter())
                .chain(phones.iter())
                .any(|s| s.to_lowercase().contains(&query))
        {
            contacts.push(Contact {
                resource_name: resource.into(),
                display_name: name,
                emails,
                phones,
                organization,
            });
        }
    }
    Ok(ContactResult {
        contacts,
        truncated: v.get("nextPageToken").is_some_and(Value::is_string),
    })
}
