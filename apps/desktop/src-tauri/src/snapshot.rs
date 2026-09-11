//! Read only the registry and fixed metadata names. Never resolve paths from metadata files.
mod reader;
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
    warnings: Vec<String>,
    safety: Safety,
}

#[derive(Serialize)]
struct Safety {
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

#[derive(Deserialize)]
struct ActiveSession {
    id: String,
    project_alias: String,
    // CLI v0.1.0 only writes a pointer. Do not follow it into sessions/.
    goal: Option<String>,
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
}

impl GhostSnapshot {
    fn empty() -> Self {
        Self {
            mode: "static-preview",
            ghost_home: None,
            storage_detected: false,
            project_count: 0,
            projects: Vec::new(),
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
    warnings.push(format!("{scope}: {message}"));
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
    snapshot
}

fn load_project(
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
    result.active_session_goal = session_goal(&workspace, project, budget, warnings);
    result.recent_output_count = output_count(&workspace, project, budget, warnings);
    result
}

fn valid_session_id(id: &str) -> bool {
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

fn session_goal(
    workspace: &Directory,
    project: &Project,
    budget: &mut ReadBudget,
    warnings: &mut Vec<String>,
) -> Option<String> {
    let scope = format!("{} active-session.yaml", project.alias);
    match read_yaml::<ActiveSession>(workspace, "active-session.yaml", budget) {
        Ok(Some(pointer))
            if pointer.project_alias == project.alias && valid_session_id(&pointer.id) =>
        {
            let goal = pointer.goal.as_deref().and_then(preview);
            if goal.is_none() {
                warn(warnings, &scope, "Active session pointer found; its goal is outside the snapshot read allowlist.");
            }
            goal
        }
        Ok(Some(_)) => {
            warn(
                warnings,
                &scope,
                "Invalid session pointer; goal unavailable.",
            );
            None
        }
        Ok(None) => None,
        Err(error) => {
            warn(warnings, &scope, error);
            None
        }
    }
}

fn output_count(
    workspace: &Directory,
    project: &Project,
    budget: &mut ReadBudget,
    warnings: &mut Vec<String>,
) -> Option<usize> {
    let mut read = || -> Result<usize, &'static str> {
        let Some(outputs) = workspace.child("outputs")? else {
            return Ok(0);
        };
        let Some(index) = read_yaml::<OutputIndex>(&outputs, "index.yaml", budget)? else {
            return Ok(0);
        };
        let mut ids = HashSet::new();
        if index.version != 1
            || index.outputs.iter().any(|entry| {
                entry.project_alias != project.alias
                    || entry.id.is_empty()
                    || !ids.insert(&entry.id)
            })
        {
            return Err("Invalid output index; count unavailable.");
        }
        // Count index records only. Output paths and output contents are never opened.
        Ok(index.outputs.len())
    };
    match read() {
        Ok(count) => Some(count),
        Err(error) => {
            warn(
                warnings,
                &format!("{} outputs/index.yaml", project.alias),
                error,
            );
            None
        }
    }
}

#[cfg(all(test, unix))]
mod tests;
