use super::{model::*, Result};
use serde::{Deserialize, Serialize};
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SearchInput {
    pub query: String,
    pub limit: usize,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListInput {
    pub filter: ReviewFilter,
    pub kind: Option<Kind>,
    pub limit: usize,
    #[serde(default)]
    pub offset: usize,
}
#[derive(Serialize)]
pub struct Hit {
    pub memory_id: String,
    pub kind: Kind,
    pub title: String,
    pub excerpt: String,
    pub tags: Vec<String>,
    pub sensitivity: Sensitivity,
    pub sharing: Sharing,
    pub source: Source,
    pub updated_at: String,
    pub expires_at: Option<String>,
    pub status: Status,
    pub expired: bool,
    pub score: u32,
}
pub fn query(value: &str) -> Result<String> {
    text(value, 480, false)?;
    if !(2..=120).contains(&value.trim().chars().count()) {
        return Err("invalid_input");
    }
    Ok(value.trim().to_owned())
}
pub fn score(title: &str, content: &str, tags: &[String], kind: &str, query: &str) -> u32 {
    let q = query.to_lowercase();
    let title = title.to_lowercase();
    let content = content.to_lowercase();
    let tags = tags.join(" ").to_lowercase();
    let all = format!("{title} {tags} {content} {kind}");
    let tokens: Vec<_> = q.split_whitespace().collect();
    100 * u32::from(title.contains(&q))
        + 80 * u32::from(tags.contains(&q))
        + 40 * u32::from(content.contains(&q))
        + 20 * u32::from(tokens.iter().all(|t| all.contains(t)))
        + tokens.iter().filter(|t| all.contains(**t)).count() as u32
}
pub fn hit(r: &Record, score: u32, now: i64) -> Hit {
    Hit {
        memory_id: r.memory_id.clone(),
        kind: r.payload.kind,
        title: r.payload.title.clone(),
        excerpt: r.payload.content.chars().take(512).collect(),
        tags: r.payload.tags.clone(),
        sensitivity: r.payload.sensitivity,
        sharing: r.payload.sharing,
        source: r.payload.source.clone(),
        updated_at: r.updated_at.clone(),
        expires_at: r.payload.expires_at.clone(),
        status: r.status,
        expired: r.expired(now),
        score,
    }
}
pub fn search(records: &[Record], input: &SearchInput, now: i64) -> Result<Vec<Hit>> {
    let q = query(&input.query)?;
    if !(1..=20).contains(&input.limit) {
        return Err("invalid_input");
    }
    let mut hits: Vec<_> = records
        .iter()
        .filter(|r| r.status == Status::Active && !r.expired(now))
        .filter_map(|r| {
            let kind = serde_json::to_string(&r.payload.kind).unwrap_or_default();
            let s = score(
                &r.payload.title,
                &r.payload.content,
                &r.payload.tags,
                &kind,
                &q,
            );
            (s > 0).then(|| {
                let mut h = hit(r, s, now);
                h.excerpt = excerpt(&r.payload.content, &q);
                h
            })
        })
        .collect();
    hits.sort_by(|a, b| {
        b.score
            .cmp(&a.score)
            .then_with(|| time(&b.updated_at).ok().cmp(&time(&a.updated_at).ok()))
            .then_with(|| a.memory_id.cmp(&b.memory_id))
    });
    hits.truncate(input.limit);
    Ok(hits)
}
pub fn list(records: &[Record], input: &ListInput, now: i64) -> Result<Vec<Hit>> {
    if !(1..=20).contains(&input.limit) || input.offset > MAX_RECORDS {
        return Err("invalid_input");
    }
    let mut hits: Vec<_> = records
        .iter()
        .filter(|r| input.kind.is_none_or(|k| k == r.payload.kind))
        .filter(|r| match input.filter {
            ReviewFilter::All => true,
            ReviewFilter::Expired => r.expired(now),
            ReviewFilter::Archived => r.status == Status::Archived,
            ReviewFilter::Active => r.status == Status::Active && !r.expired(now),
        })
        .map(|r| hit(r, 0, now))
        .collect();
    hits.sort_by(|a, b| {
        time(&b.updated_at)
            .ok()
            .cmp(&time(&a.updated_at).ok())
            .then_with(|| a.memory_id.cmp(&b.memory_id))
    });
    Ok(hits
        .into_iter()
        .skip(input.offset)
        .take(input.limit)
        .collect())
}

fn excerpt(content: &str, query: &str) -> String {
    let folded = content.to_lowercase();
    let query = query.to_lowercase();
    let byte = folded
        .find(&query)
        .or_else(|| query.split_whitespace().find_map(|q| folded.find(q)))
        .unwrap_or(0);
    let mut folded_bytes = 0;
    let mut start = 0;
    for (i, c) in content.chars().enumerate() {
        if folded_bytes >= byte {
            start = i.saturating_sub(64);
            break;
        }
        folded_bytes += c.to_lowercase().map(char::len_utf8).sum::<usize>();
    }
    let mut result = String::new();
    if start > 0 {
        result.push('…');
    }
    result.extend(content.chars().skip(start).take(510));
    if content.chars().count() > start + 510 {
        result.push('…');
    }
    result
}
