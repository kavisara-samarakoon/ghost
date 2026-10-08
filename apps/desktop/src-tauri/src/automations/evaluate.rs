use super::{model::*, mutations::AuditEvent, storage::AutomationStore, Result};
use chrono::{DateTime, Datelike, Duration, Utc};
use serde::Serialize;
use std::path::Path;

pub fn occurrence(trigger: &Trigger, now: i64) -> Result<Option<(i64, String)>> {
    timestamp(now)?;
    trigger.validate()?;
    let current = DateTime::<Utc>::from_timestamp(now, 0).ok_or("invalid_schedule")?;
    match trigger {
        Trigger::Once { at } => {
            let due = instant(at)?;
            Ok((due <= current).then(|| {
                (
                    due.timestamp(),
                    format!(
                        "once:{}",
                        due.to_rfc3339_opts(chrono::SecondsFormat::AutoSi, true)
                    ),
                )
            }))
        }
        Trigger::Daily {
            hour,
            minute,
            offset_minutes,
        }
        | Trigger::Weekly {
            hour,
            minute,
            offset_minutes,
            ..
        } => {
            let offset = i64::from(*offset_minutes) * 60;
            let local = current
                .checked_add_signed(Duration::seconds(offset))
                .ok_or("invalid_schedule")?;
            let mut date = local.date_naive();
            let weekly = if let Trigger::Weekly { weekday, .. } = trigger {
                let back = (local.weekday().num_days_from_monday() + 7 - weekday.index()) % 7;
                date = date
                    .checked_sub_signed(Duration::days(i64::from(back)))
                    .ok_or("invalid_schedule")?;
                true
            } else {
                false
            };
            let epoch = |d: chrono::NaiveDate| -> Result<i64> {
                Ok(d.and_hms_opt(u32::from(*hour), u32::from(*minute), 0)
                    .ok_or("invalid_schedule")?
                    .and_utc()
                    .timestamp()
                    - offset)
            };
            if epoch(date)? > now {
                date = date
                    .checked_sub_signed(Duration::days(if weekly { 7 } else { 1 }))
                    .ok_or("invalid_schedule")?;
            }
            let due = epoch(date)?;
            timestamp(due)?;
            Ok(Some((
                due,
                format!(
                    "{}:{}/{:02}:{:02}/{:+}",
                    trigger.kind(),
                    date,
                    hour,
                    minute,
                    offset_minutes
                ),
            )))
        }
        _ => Ok(None),
    }
}
pub fn next_due(def: &Definition, now: i64) -> Result<Option<i64>> {
    if !def.payload.enabled {
        return Ok(None);
    }
    if let Trigger::Once { at } = &def.payload.trigger {
        return Ok(if def.cursor.last_due.is_some() {
            None
        } else {
            Some(instant(at)?.timestamp())
        });
    }
    if let Some((due, _)) = occurrence(&def.payload.trigger, now)? {
        if due >= def.created_at && def.cursor.last_due.is_none_or(|last| due > last) {
            return Ok(Some(due));
        }
        return Ok(Some(
            due + if matches!(def.payload.trigger, Trigger::Weekly { .. }) {
                7 * 86400
            } else {
                86400
            },
        ));
    }
    Ok(None)
}
#[derive(Serialize)]
pub struct Evaluation {
    pub created: Vec<Item>,
    pub pending_count: usize,
    pub audit_recorded: bool,
}
pub fn evaluate_document(
    doc: &mut Document,
    now: i64,
    mut condition: impl FnMut(&Trigger) -> Result<bool>,
) -> Result<(Vec<Item>, Vec<AuditEvent>)> {
    timestamp(now)?;
    doc.validate()?;
    let mut created = vec![];
    let mut events = vec![];
    for def in &mut doc.definitions {
        if !def.payload.enabled {
            continue;
        }
        let candidate = match &def.payload.trigger {
            Trigger::ProjectNoActiveSession { .. } | Trigger::RecentPendingRequestsPresent {} => {
                let active = condition(&def.payload.trigger)?;
                if !active && def.cursor.condition_true {
                    def.cursor.condition_true = false;
                    events.push(AuditEvent::definition(
                        "desktop.automation.rearmed",
                        def,
                        now,
                        "evaluated",
                        None,
                    ));
                }
                if active && !def.cursor.condition_true {
                    Some((
                        now,
                        format!(
                            "condition:{}",
                            def.cursor.edge.checked_add(1).ok_or("limit")?
                        ),
                    ))
                } else {
                    None
                }
            }
            trigger => occurrence(trigger, now)?.filter(|(due, _)| {
                (matches!(trigger, Trigger::Once { .. }) || *due >= def.created_at)
                    && def.cursor.last_due.is_none_or(|last| *due > last)
            }),
        };
        let Some((due, key)) = candidate else {
            continue;
        };
        if created.len() >= MAX_EVALUATION {
            continue;
        }
        if doc.inbox.len() >= MAX_INBOX {
            return Err("limit");
        }
        if doc
            .inbox
            .iter()
            .any(|i| i.automation_id == def.id && i.occurrence_key == key)
        {
            return Err("invalid_state");
        }
        let item = Item {
            version: 1,
            id: uuid::Uuid::new_v4().to_string(),
            automation_id: def.id.clone(),
            automation_title: def.payload.title.clone(),
            task: def.payload.task.clone(),
            triggered_at: now,
            occurrence_key: key.clone(),
            status: ItemStatus::Pending,
        };
        if matches!(
            def.payload.trigger,
            Trigger::ProjectNoActiveSession { .. } | Trigger::RecentPendingRequestsPresent {}
        ) {
            def.cursor.condition_true = true;
            def.cursor.edge = def.cursor.edge.checked_add(1).ok_or("limit")?;
        } else {
            def.cursor.last_due = Some(due);
        }
        events.push(AuditEvent::definition(
            "desktop.automation.triggered",
            def,
            now,
            "evaluated",
            Some(key),
        ));
        doc.inbox.push(item.clone());
        created.push(item);
    }
    doc.validate()?;
    Ok((created, events))
}
pub fn evaluate(store: &AutomationStore, home: &Path, now: i64) -> Result<Evaluation> {
    let mut conditions = std::collections::BTreeMap::new();
    let ((created, pending_count), audit_recorded) = store.transaction(|doc| {
        let (created, events) = evaluate_document(doc, now, |trigger| {
            let key = serde_json::to_string(trigger).map_err(|_| "invalid_trigger")?;
            if let Some(value) = conditions.get(&key) {
                return Ok(*value);
            }
            let value = match trigger {
                Trigger::ProjectNoActiveSession { project_alias } => {
                    crate::snapshot::automation_project_no_active_session(home, project_alias)
                }
                Trigger::RecentPendingRequestsPresent {} => {
                    crate::snapshot::automation_recent_pending_requests(home)
                }
                _ => return Err("invalid_trigger"),
            }
            .map_err(|_| "invalid_state")?;
            conditions.insert(key, value);
            Ok(value)
        })?;
        Ok((
            (
                created,
                doc.inbox
                    .iter()
                    .filter(|i| i.status == ItemStatus::Pending)
                    .count(),
            ),
            events,
        ))
    })?;
    Ok(Evaluation {
        created,
        pending_count,
        audit_recorded,
    })
}
