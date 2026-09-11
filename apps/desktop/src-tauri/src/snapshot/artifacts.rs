//! Only fixed draft folders and validated output-index paths reach the reader.
use super::{
    names, preview, read_yaml,
    reader::{Directory, ReadBudget},
    text, warn, Counts, Project,
};
use serde::{Deserialize, Serialize};
use std::{collections::HashSet, sync::Arc};

pub(super) const RECENT_LIMIT: usize = 5;

#[derive(Serialize)]
pub(super) struct Artifact {
    pub kind: &'static str,
    pub title: String,
    pub relative_path: String,
    pub preview: Option<String>,
    pub created_at: Option<String>,
}

struct Candidate {
    folder: Arc<Directory>,
    name: String,
    kind: &'static str,
    relative_path: String,
    title: Option<String>,
    created_at: Option<String>,
}

#[derive(Deserialize)]
struct OutputIndex {
    version: u32,
    outputs: Vec<OutputEntry>,
}

#[derive(Deserialize)]
struct OutputEntry {
    id: String,
    project_alias: String,
    #[serde(rename = "type")]
    kind: String,
    title: String,
    path: String,
}

pub(super) fn load(
    workspace: &Directory,
    project: &Project,
    budget: &mut ReadBudget,
    warnings: &mut Vec<String>,
) -> (Vec<Artifact>, Counts) {
    let mut artifacts = Vec::new();
    let (outputs, output_count) = output_candidates(workspace, project, budget, warnings);
    append_recent(outputs, &mut artifacts, project, budget, warnings);
    let mut counts = Counts {
        outputs: output_count,
        ..Counts::default()
    };
    for (kind, path, count) in [
        (
            "context-pack",
            "drafts/context-packs",
            &mut counts.context_packs,
        ),
        ("next-step", "drafts/next-steps", &mut counts.next_steps),
    ] {
        let (candidates, total) =
            draft_candidates(workspace, path, kind, project, budget, warnings);
        *count = total;
        append_recent(candidates, &mut artifacts, project, budget, warnings);
    }
    let mut handoffs = Vec::new();
    counts.handoffs = Some(0);
    for target in ["codex", "chatgpt", "gemini", "antigravity"] {
        let (candidates, total) = draft_candidates(
            workspace,
            &format!("drafts/handoffs/{target}"),
            "handoff",
            project,
            budget,
            warnings,
        );
        counts.handoffs = counts.handoffs.zip(total).map(|(left, right)| left + right);
        handoffs.extend(candidates);
    }
    append_recent(handoffs, &mut artifacts, project, budget, warnings);
    let (updates, total) = update_candidates(workspace, project, budget, warnings);
    counts.update_packs = total;
    append_recent(updates, &mut artifacts, project, budget, warnings);
    artifacts.sort_by(|a, b| {
        b.created_at
            .cmp(&a.created_at)
            .then_with(|| a.relative_path.cmp(&b.relative_path))
    });
    (artifacts, counts)
}

fn descend(root: &Directory, path: &str) -> Result<Option<Directory>, &'static str> {
    let mut parts = path.split('/');
    let Some(mut directory) = root.child(parts.next().ok_or("Empty metadata path.")?)? else {
        return Ok(None);
    };
    for part in parts {
        let Some(child) = directory.child(part)? else {
            return Ok(None);
        };
        directory = child;
    }
    Ok(Some(directory))
}

fn listing(
    folder: &Directory,
    scope: &str,
    project: &Project,
    budget: &mut ReadBudget,
    warnings: &mut Vec<String>,
) -> Result<Vec<String>, &'static str> {
    let listing = folder.list(budget)?;
    if listing.skipped {
        warn(
            warnings,
            &format!("{} {scope}", project.alias),
            "Unsafe or unsupported entries were skipped.",
        );
    }
    Ok(listing.names)
}

fn draft_candidates(
    workspace: &Directory,
    path: &str,
    kind: &'static str,
    project: &Project,
    budget: &mut ReadBudget,
    warnings: &mut Vec<String>,
) -> (Vec<Candidate>, Option<usize>) {
    let result = (|| {
        let Some(folder) = descend(workspace, path)? else {
            return Ok(Vec::new());
        };
        let names = listing(&folder, path, project, budget, warnings)?;
        let folder = Arc::new(folder);
        Ok::<_, &'static str>(
            names
                .into_iter()
                .map(|name| Candidate {
                    folder: Arc::clone(&folder),
                    relative_path: format!("{path}/{name}"),
                    created_at: names::filename_time(&name),
                    name,
                    kind,
                    title: None,
                })
                .collect::<Vec<_>>(),
        )
    })();
    match result {
        Ok(candidates) => {
            let total = candidates.len();
            (candidates, Some(total))
        }
        Err(error) => {
            warn(warnings, &format!("{} {path}", project.alias), error);
            (Vec::new(), None)
        }
    }
}

fn output_candidates(
    workspace: &Directory,
    project: &Project,
    budget: &mut ReadBudget,
    warnings: &mut Vec<String>,
) -> (Vec<Candidate>, Option<usize>) {
    let result = (|| {
        let Some(folder) = workspace.child("outputs")? else {
            return Ok((Vec::new(), Some(0)));
        };
        let index = read_yaml::<OutputIndex>(&folder, "index.yaml", budget)?
            .ok_or("Output index.yaml is missing.")?;
        if index.version != 1 || index.outputs.len() > super::reader::MAX_DIRECTORY_ENTRIES {
            return Err("Invalid or oversized output index; outputs unavailable.");
        }
        let mut ids = HashSet::new();
        let mut entries = Vec::new();
        let mut complete = true;
        for entry in index.outputs {
            if entry.project_alias != project.alias
                || !names::output_path(&entry.id, &entry.kind, &entry.path)
                || entry.title.trim().is_empty()
                || !ids.insert(entry.id.clone())
            {
                complete = false;
                continue;
            }
            entries.push(entry);
        }
        if !complete {
            warn(
                warnings,
                &project.alias,
                "Unsafe or mismatched output index entries skipped; output count unavailable.",
            );
        }
        let count = complete.then_some(entries.len());
        entries.sort_by(|a, b| b.id.cmp(&a.id));
        let mut candidates = Vec::new();
        for entry in entries.into_iter().take(RECENT_LIMIT) {
            match folder.child(&entry.kind) {
                Ok(Some(directory)) => candidates.push(Candidate {
                    folder: Arc::new(directory),
                    name: format!("{}.md", entry.id),
                    kind: "output",
                    relative_path: entry.path,
                    title: preview(&entry.title),
                    created_at: names::filename_time(&entry.id),
                }),
                Ok(None) => warn(
                    warnings,
                    &project.alias,
                    "An indexed output directory is missing.",
                ),
                Err(error) => warn(warnings, &format!("{} outputs", project.alias), error),
            }
        }
        Ok((candidates, count))
    })();
    match result {
        Ok(result) => result,
        Err(error) => {
            warn(
                warnings,
                &format!("{} outputs/index.yaml", project.alias),
                error,
            );
            (Vec::new(), None)
        }
    }
}

fn update_candidates(
    workspace: &Directory,
    project: &Project,
    budget: &mut ReadBudget,
    warnings: &mut Vec<String>,
) -> (Vec<Candidate>, Option<usize>) {
    let result = (|| {
        let Some(folder) = descend(workspace, "drafts/update-packs")? else {
            return Ok((Vec::new(), Some(0)));
        };
        let mut packs = listing(&folder, "drafts/update-packs", project, budget, warnings)?;
        let count = packs.len();
        packs.sort_by(|a, b| {
            names::filename_time(b)
                .cmp(&names::filename_time(a))
                .then_with(|| b.cmp(a))
        });
        let mut candidates = Vec::new();
        for pack in packs.into_iter().take(RECENT_LIMIT) {
            let path = format!("drafts/update-packs/{pack}");
            let read = (|| {
                let directory = folder
                    .child(&pack)?
                    .ok_or("Update pack directory is missing.")?;
                let files = listing(&directory, &path, project, budget, warnings)?;
                Ok::<_, &'static str>((Arc::new(directory), files))
            })();
            match read {
                Ok((directory, files)) => {
                    candidates.extend(files.into_iter().map(|name| Candidate {
                        folder: Arc::clone(&directory),
                        relative_path: format!("{path}/{name}"),
                        created_at:
                            names::filename_time(&name).or_else(|| names::filename_time(&pack)),
                        name,
                        kind: "update-pack",
                        title: None,
                    }))
                }
                Err(error) => warn(warnings, &format!("{} {path}", project.alias), error),
            }
        }
        Ok((candidates, Some(count)))
    })();
    match result {
        Ok(result) => result,
        Err(error) => {
            warn(warnings, &format!("{} update packs", project.alias), error);
            (Vec::new(), None)
        }
    }
}

fn append_recent(
    mut candidates: Vec<Candidate>,
    artifacts: &mut Vec<Artifact>,
    project: &Project,
    budget: &mut ReadBudget,
    warnings: &mut Vec<String>,
) {
    candidates.sort_by(|a, b| {
        b.created_at
            .cmp(&a.created_at)
            .then_with(|| a.relative_path.cmp(&b.relative_path))
    });
    for candidate in candidates.into_iter().take(RECENT_LIMIT) {
        let scope = format!("{} {}", project.alias, candidate.relative_path);
        match candidate.folder.read(&candidate.name, budget) {
            Ok(Some(content)) => {
                // Redact the complete bounded document before extracting its title or preview.
                let clean = text::redact(&content);
                let heading = clean
                    .lines()
                    .find_map(|line| line.trim().strip_prefix("# "));
                let title = candidate
                    .title
                    .or_else(|| heading.and_then(preview))
                    .unwrap_or_else(|| {
                        preview(&candidate.name).unwrap_or_else(|| "Untitled artifact".into())
                    });
                artifacts.push(Artifact {
                    kind: candidate.kind,
                    title,
                    relative_path: candidate.relative_path,
                    preview: text::excerpt(&clean, 600, false),
                    created_at: candidate.created_at,
                });
            }
            Ok(None) => warn(warnings, &scope, "Artifact file is missing."),
            Err(error) => warn(warnings, &scope, error),
        }
    }
}
