//! Storage names are validated before opening any directory or file.
use chrono::{DateTime, SecondsFormat, Utc};

pub fn safe_segment(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 180
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
}

pub fn markdown(name: &str) -> bool {
    name.strip_suffix(".md").is_some_and(safe_segment)
}

pub fn search_path(path: &str) -> bool {
    if path.len() > 512 {
        return false;
    }
    match path.split('/').collect::<Vec<_>>().as_slice() {
        ["status.md" | "decisions.md" | "milestones.yaml" | "active-session.yaml"] => true,
        ["outputs", "index.yaml"] => true,
        ["outputs", "codex" | "terminal", file] => markdown(file),
        _ => action_path(path),
    }
}

/// Exact workspace-relative action categories; never normalize untrusted paths.
pub fn action_path(path: &str) -> bool {
    if path.len() > 512 {
        return false;
    }
    let parts: Vec<_> = path.split('/').collect();
    match parts.as_slice() {
        ["outputs", kind, file] => file
            .strip_suffix(".md")
            .is_some_and(|id| markdown(file) && output_path(id, kind, path)),
        ["drafts", "context-packs" | "next-steps", file] => markdown(file),
        ["drafts", "handoffs", "codex" | "chatgpt" | "gemini" | "antigravity", file] => {
            markdown(file)
        }
        ["drafts", "update-packs", pack, file] => safe_segment(pack) && markdown(file),
        ["sessions", id, "session.yaml" | "notes.md"] => session_id(id),
        _ => false,
    }
}

pub fn session_id(id: &str) -> bool {
    let bytes = id.as_bytes();
    bytes.len() == 31
        && bytes[8] == b'T'
        && &bytes[21..23] == b"Z-"
        && bytes[..8]
            .iter()
            .chain(&bytes[9..21])
            .all(u8::is_ascii_digit)
        && bytes[23..]
            .iter()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(b))
}

pub fn timestamp(value: &str) -> Option<String> {
    DateTime::parse_from_rfc3339(value).ok().map(|date| {
        date.with_timezone(&Utc)
            .to_rfc3339_opts(SecondsFormat::Micros, true)
    })
}

pub fn filename_time(name: &str) -> Option<String> {
    let prefix = name.get(..23)?;
    if !session_id(&format!("{prefix}00000000")) {
        return None;
    }
    timestamp(&format!(
        "{}-{}-{}T{}:{}:{}.{}Z",
        &prefix[..4],
        &prefix[4..6],
        &prefix[6..8],
        &prefix[9..11],
        &prefix[11..13],
        &prefix[13..15],
        &prefix[15..21]
    ))
}

pub fn output_path(id: &str, kind: &str, path: &str) -> bool {
    if !matches!(kind, "codex" | "terminal") || filename_time(id).is_none() {
        return false;
    }
    let Some(suffix) = id
        .get(23..)
        .and_then(|s| s.strip_prefix(&format!("{kind}-output-")))
    else {
        return false;
    };
    suffix.len() == 8
        && suffix
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
        && path == format!("outputs/{kind}/{id}.md")
}
