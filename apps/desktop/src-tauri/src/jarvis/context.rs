use super::{model, Result};
use crate::memory::{
    model::Record,
    search::{search, SearchInput},
};
use serde::{Deserialize, Serialize};
#[derive(Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct Sharing {
    pub personal_memory: bool,
    pub project_memory: bool,
}
// No Google origin exists in this envelope. Never accept a local ContextPack as provider input.
#[derive(Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Origin {
    PersonalMemory,
    ProjectMemory,
}
#[derive(Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Item {
    pub source: Origin,
    pub kind: String,
    pub title: String,
    pub content: String,
    pub project_alias: Option<String>,
    pub instruction_trust: Trust,
}
#[derive(Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Trust {
    DataOnly,
}
pub fn safe(value: &str) -> Result<()> {
    model::text(value, 8192, true).map_err(|_| "invalid_context")?;
    static PATH: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    if PATH.get_or_init(||regex::Regex::new(r#"(?i)(?:/Users/|/home/|/Volumes/|file://|[a-z]:[\\/]|(?:^|[\s"'(])/(?:[^\s/]+/)+[^\s]*)"#).expect("fixed pattern")).is_match(value){return Err("invalid_context");}
    Ok(())
}
pub fn personal(records: &[Record], query: &str, now: i64) -> Result<Vec<Item>> {
    let eligible: Vec<_> = records
        .iter()
        .filter(|r| r.eligible_for_provider(now))
        .cloned()
        .collect();
    search(
        &eligible,
        &SearchInput {
            query: query.into(),
            limit: 8,
        },
        now,
    )?
    .into_iter()
    .map(|h| {
        safe(&h.title)?;
        safe(&h.excerpt)?;
        Ok(Item {
            source: Origin::PersonalMemory,
            kind: serde_json::to_value(h.kind)
                .unwrap()
                .as_str()
                .unwrap()
                .into(),
            title: h.title,
            content: h.excerpt,
            project_alias: None,
            instruction_trust: Trust::DataOnly,
        })
    })
    .collect()
}
pub fn project(response: crate::snapshot::search::SearchResponse) -> Result<Vec<Item>> {
    if response.mode != "live-local" {
        return Err("invalid_context");
    }
    response
        .results
        .into_iter()
        .take(6)
        .map(|h| {
            safe(&h.title)?;
            safe(&h.snippet)?;
            model::alias(&h.project_alias).map_err(|_| "invalid_context")?;
            Ok(Item {
                source: Origin::ProjectMemory,
                kind: h.kind.into(),
                title: h.title,
                content: h.snippet,
                project_alias: Some(h.project_alias),
                instruction_trust: Trust::DataOnly,
            })
        })
        .collect()
}
pub fn load(
    home: &std::path::Path,
    sharing: &Sharing,
    query: Option<&str>,
    alias: Option<&str>,
    now: i64,
) -> Result<Vec<Item>> {
    if !sharing.personal_memory && !sharing.project_memory {
        return Ok(Vec::new());
    }
    let query = crate::memory::search::query(query.ok_or("invalid_context")?)
        .map_err(|_| "invalid_context")?;
    let mut items = Vec::new();
    if sharing.personal_memory {
        let store =
            crate::memory::storage::MemoryStore::open(home).map_err(|_| "invalid_context")?;
        items.extend(personal(
            &store.list().map_err(|_| "invalid_context")?,
            &query,
            now,
        )?);
    }
    if sharing.project_memory {
        items.extend(project(crate::snapshot::search::load(
            home,
            &query,
            alias,
            &mut crate::snapshot::reader::ReadBudget::search(),
        ))?);
    }
    Ok(items)
}
