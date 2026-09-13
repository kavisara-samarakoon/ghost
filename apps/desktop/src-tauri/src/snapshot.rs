//! Read allowlisted local metadata. Only the separate requests module writes pending drafts.
pub mod actions;
mod artifacts;
mod names;
mod reader;
pub mod requests;
pub mod search;
mod session;
mod text;

use reader::{Directory, ReadBudget};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::collections::HashSet;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

const MAX_PROJECTS: usize = 128;

#[derive(Serialize)]
pub struct GhostSnapshot {
    mode: &'static str,
    ghost_home: Option<String>,
    storage_detected: bool,
    project_count: usize,
    projects: Vec<ProjectSnapshot>,
    recent_action_requests: Vec<requests::RecentRequest>,
    warnings: Vec<String>,
    safety: Safety,
}

#[derive(Serialize)]
struct Safety {
    // Guarantees for the snapshot operation, not the separate request save command.
    read_only: bool,
    no_shell_execution: bool,
    no_cli_execution: bool,
    no_ai_calls: bool,
    no_network_calls: bool,
    no_file_writes: bool,
}

#[derive(Serialize)]
struct ProjectSnapshot {
    alias: String,
    name: String,
    path: String,
    path_exists: bool,
    workspace_exists: bool,
    status_preview: Option<String>,
    active_session_goal: Option<String>,
    recent_output_count: Option<usize>,
    active_session: Option<session::Session>,
    recent_artifacts: Vec<artifacts::Artifact>,
    counts: Counts,
    warnings: Vec<String>,
}

#[derive(Default, Serialize)]
struct Counts {
    // Only the pointed-to active session is in scope: this is 0, 1, or unknown.
    sessions: Option<usize>,
    outputs: Option<usize>,
    handoffs: Option<usize>,
    context_packs: Option<usize>,
    next_steps: Option<usize>,
    // Number of safe pack directories, not the number of documents inside them.
    update_packs: Option<usize>,
}

#[derive(Deserialize)]
struct Registry {
    version: u32,
    projects: Vec<Project>,
}

#[derive(Deserialize)]
struct Project {
    alias: String,
    name: String,
    path: PathBuf,
}

impl GhostSnapshot {
    fn empty() -> Self {
        Self {
            mode: "static-preview",
            ghost_home: None,
            storage_detected: false,
            project_count: 0,
            projects: Vec::new(),
            recent_action_requests: Vec::new(),
            warnings: Vec::new(),
            safety: Safety {
                read_only: true,
                no_shell_execution: true,
                no_cli_execution: true,
                no_ai_calls: true,
                no_network_calls: true,
                no_file_writes: true,
            },
        }
    }
}

fn resolve_home(
    override_home: Option<OsString>,
    user_home: Option<PathBuf>,
    cwd: Option<PathBuf>,
) -> Result<PathBuf, &'static str> {
    let path = match override_home {
        Some(value) => {
            let value = value
                .into_string()
                .map_err(|_| "GHOST_HOME is not valid UTF-8.")?;
            if value.trim().is_empty() {
                return Err("GHOST_HOME must not be empty.");
            }
            if value == "~" {
                user_home.ok_or("User home is unavailable.")?
            } else if let Some(suffix) = value.strip_prefix("~/") {
                user_home.ok_or("User home is unavailable.")?.join(suffix)
            } else if value.starts_with('~') {
                return Err("GHOST_HOME only supports the current user's home.");
            } else {
                PathBuf::from(value)
            }
        }
        None => user_home.ok_or("User home is unavailable.")?.join(".ghost"),
    };
    let path = if path.is_absolute() {
        path
    } else {
        cwd.ok_or("Working directory is unavailable.")?.join(path)
    };
    reader::validate_path(&path)?;
    Ok(path)
}

pub fn load_from_environment() -> GhostSnapshot {
    // Environment lookup only; no config discovery or environment-file loading.
    match resolve_home(
        std::env::var_os("GHOST_HOME"),
        std::env::home_dir(),
        std::env::current_dir().ok(),
    ) {
        Ok(home) => load(&home),
        Err(warning) => {
            let mut snapshot = GhostSnapshot::empty();
            snapshot.warnings.push(warning.into());
            snapshot
        }
    }
}

fn warn(warnings: &mut Vec<String>, scope: &str, message: &str) {
    warnings.push(format!("{}: {message}", text::redact(scope)));
}

fn read_yaml<T: DeserializeOwned>(
    directory: &Directory,
    name: &str,
    budget: &mut ReadBudget,
) -> Result<Option<T>, &'static str> {
    directory
        .read(name, budget)?
        .map(|text| {
            serde_yaml_ng::from_str(&text)
                .map_err(|_| "Invalid YAML metadata; contents were not included.")
        })
        .transpose()
}

fn valid_alias(alias: &str) -> bool {
    !alias.is_empty()
        && alias.len() <= 128
        && alias
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

fn preview(text: &str) -> Option<String> {
    let text = text::redact(text);
    let clean: String = text
        .chars()
        .filter(|c| !c.is_control() || c.is_whitespace())
        .collect();
    let clean = clean.split_whitespace().collect::<Vec<_>>().join(" ");
    if clean.is_empty() {
        return None;
    }
    let mut result: String = clean.chars().take(240).collect();
    if clean.chars().count() > 240 {
        result.push('…');
    }
    Some(result)
}

fn load(home: &Path) -> GhostSnapshot {
    let mut snapshot = GhostSnapshot::empty();
    snapshot.ghost_home = Some(home.to_string_lossy().into_owned());
    let directory = match Directory::open(home) {
        Ok(Some(directory)) => directory,
        Ok(None) => return snapshot,
        Err(error) => {
            warn(&mut snapshot.warnings, "GHOST_HOME", error);
            return snapshot;
        }
    };
    snapshot.storage_detected = true;
    let mut budget = ReadBudget::default();
    let registry: Registry = match read_yaml(&directory, "projects.yaml", &mut budget) {
        Ok(Some(registry)) => registry,
        Ok(None) => {
            warn(
                &mut snapshot.warnings,
                "projects.yaml",
                "Registry is missing.",
            );
            return snapshot;
        }
        Err(error) => {
            warn(&mut snapshot.warnings, "projects.yaml", error);
            return snapshot;
        }
    };
    if registry.version != 1 || registry.projects.len() > MAX_PROJECTS {
        warn(
            &mut snapshot.warnings,
            "projects.yaml",
            "Expected version 1 and at most 128 projects.",
        );
        return snapshot;
    }
    snapshot.mode = "live-local";
    let registered_count = registry.projects.len();
    let mut aliases = HashSet::new();
    let mut paths = HashSet::new();
    for project in registry.projects {
        if !valid_alias(&project.alias)
            || project.name.trim().is_empty()
            || reader::validate_path(&project.path).is_err()
        {
            warn(
                &mut snapshot.warnings,
                "projects.yaml",
                "Skipped an invalid or unsafe project entry.",
            );
            continue;
        }
        if !aliases.insert(project.alias.clone()) || !paths.insert(project.path.clone()) {
            warn(
                &mut snapshot.warnings,
                "projects.yaml",
                "Skipped a duplicate project entry.",
            );
            continue;
        }
        snapshot
            .projects
            .push(load_project(&project, &mut budget, &mut snapshot.warnings));
    }
    snapshot.project_count = snapshot.projects.len();
    if registered_count > 0 && snapshot.projects.is_empty() {
        snapshot.mode = "static-preview";
    }
    // Optional requests use the remaining budget after core workflow metadata.
    match requests::recent(&directory, &mut budget) {
        Ok(requests) => snapshot.recent_action_requests = requests,
        Err(error) => warn(&mut snapshot.warnings, "action-requests", error),
    }
    snapshot
}

fn load_project(
    project: &Project,
    budget: &mut ReadBudget,
    warnings: &mut Vec<String>,
) -> ProjectSnapshot {
    let mut local_warnings = Vec::new();
    let mut result = read_project(project, budget, &mut local_warnings);
    warnings.extend(local_warnings.iter().cloned());
    result.warnings = local_warnings;
    result
}

fn read_project(
    project: &Project,
    budget: &mut ReadBudget,
    warnings: &mut Vec<String>,
) -> ProjectSnapshot {
    let mut result = ProjectSnapshot {
        alias: project.alias.clone(),
        name: preview(&project.name).unwrap_or_else(|| project.alias.clone()),
        path: project.path.to_string_lossy().into_owned(),
        path_exists: false,
        workspace_exists: false,
        status_preview: None,
        active_session_goal: None,
        recent_output_count: None,
        active_session: None,
        recent_artifacts: Vec::new(),
        counts: Counts::default(),
        warnings: Vec::new(),
    };
    let root = match Directory::open(&project.path) {
        Ok(Some(root)) => root,
        Ok(None) => {
            warn(warnings, &project.alias, "Project directory is missing.");
            return result;
        }
        Err(error) => {
            warn(warnings, &project.alias, error);
            return result;
        }
    };
    result.path_exists = true;
    let workspace = match root.child(".ghost") {
        Ok(Some(workspace)) => workspace,
        Ok(None) => {
            warn(warnings, &project.alias, "GHOST workspace is missing.");
            return result;
        }
        Err(error) => {
            warn(warnings, &project.alias, error);
            return result;
        }
    };
    result.workspace_exists = true;
    // Verify workspace identity, but never use its path as a read destination.
    match read_yaml::<Project>(&workspace, "project.yaml", budget) {
        Ok(Some(record)) if record.alias == project.alias && record.path == project.path => {}
        Ok(_) => {
            warn(
                warnings,
                &project.alias,
                "Missing or mismatched project.yaml; workspace metadata skipped.",
            );
            return result;
        }
        Err(error) => {
            warn(warnings, &format!("{} project.yaml", project.alias), error);
            return result;
        }
    }
    match workspace.read("status.md", budget) {
        Ok(text) => result.status_preview = text.as_deref().and_then(preview),
        Err(error) => warn(warnings, &format!("{} status.md", project.alias), error),
    }
    let (session, session_count) = session::load(&workspace, project, budget, warnings);
    let (artifacts, mut counts) = artifacts::load(&workspace, project, budget, warnings);
    counts.sessions = session_count;
    result.active_session_goal = session.as_ref().map(|session| session.goal_preview.clone());
    result.recent_output_count = counts.outputs;
    result.active_session = session;
    result.recent_artifacts = artifacts;
    result.counts = counts;
    result
}

#[cfg(all(test, unix))]
mod tests;
