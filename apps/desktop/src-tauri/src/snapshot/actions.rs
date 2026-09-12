//! Click-only actions. Every request resolves storage afresh; frontend paths are never trusted.
use super::{
    artifacts, names, read_yaml, reader::Directory, reader::ReadBudget, session, Project, Registry,
};
use std::{
    fs::File,
    path::{Path, PathBuf},
};

#[cfg(target_os = "macos")]
mod macos;

#[derive(Clone, Copy)]
pub enum Action {
    Open,
    Reveal,
}

#[derive(Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ActionState {
    Opened,
    Revealed,
    Rejected,
    Unavailable,
}

pub(super) struct Target {
    home: PathBuf,
    alias: String,
    relative_path: String,
    path: PathBuf,
    file: File,
}

impl Target {
    pub(super) fn revalidate(&self) -> Result<(), &'static str> {
        let current = resolve(&self.home, &self.alias, &self.relative_path)?;
        if current.path != self.path || !same_file(&self.file, &current.file)? {
            return Err("Artifact changed; action rejected.");
        }
        Ok(())
    }
}

#[cfg(unix)]
fn same_file(left: &File, right: &File) -> Result<bool, &'static str> {
    use std::os::unix::fs::MetadataExt;
    let left = left.metadata().map_err(|_| "Artifact unavailable.")?;
    let right = right.metadata().map_err(|_| "Artifact unavailable.")?;
    Ok(left.is_file()
        && right.is_file()
        && left.nlink() == 1
        && right.nlink() == 1
        && left.dev() == right.dev()
        && left.ino() == right.ino())
}

#[cfg(not(unix))]
fn same_file(_: &File, _: &File) -> Result<bool, &'static str> {
    Ok(false)
}

pub(super) fn resolve(
    home: &Path,
    alias: &str,
    relative_path: &str,
) -> Result<Target, &'static str> {
    // Check the complete string before filesystem access, including empty, encoded and hidden names.
    if !super::valid_alias(alias) || !names::action_path(relative_path) {
        return Err("Artifact is outside the allowlist.");
    }
    let mut budget = ReadBudget::default();
    let storage = Directory::open(home)?.ok_or("Storage unavailable.")?;
    let registry = read_yaml::<Registry>(&storage, "projects.yaml", &mut budget)?
        .ok_or("Registry unavailable.")?;
    if registry.version != 1 || registry.projects.len() > super::MAX_PROJECTS {
        return Err("Registry unavailable.");
    }
    let matches: Vec<_> = registry
        .projects
        .iter()
        .filter(|project| project.alias == alias)
        .collect();
    if matches.len() != 1 {
        return Err("Project unavailable.");
    }
    let project = matches[0];
    if project.name.trim().is_empty()
        || registry
            .projects
            .iter()
            .filter(|record| record.path == project.path)
            .count()
            != 1
    {
        return Err("Project identity is ambiguous.");
    }
    let root = Directory::open(&project.path)?.ok_or("Project unavailable.")?;
    let workspace = root.child(".ghost")?.ok_or("Workspace unavailable.")?;
    let record = read_yaml::<Project>(&workspace, "project.yaml", &mut budget)?
        .ok_or("Workspace identity unavailable.")?;
    if record.alias != alias || record.path != project.path {
        return Err("Workspace identity changed.");
    }
    if relative_path.starts_with("outputs/")
        && !artifacts::allows_action(&workspace, project, relative_path, &mut budget)?
    {
        return Err("Output is no longer registered.");
    }
    if relative_path.starts_with("sessions/") {
        let (active, _) = session::load(&workspace, project, &mut budget, &mut Vec::new());
        if !active.is_some_and(|active| relative_path.split('/').nth(1) == Some(active.id.as_str()))
        {
            return Err("Session is no longer active.");
        }
    }
    let mut parts = relative_path.split('/').collect::<Vec<_>>();
    let name = parts.pop().ok_or("Artifact unavailable.")?;
    let mut directory = workspace;
    for part in parts {
        directory = directory
            .child(part)?
            .ok_or("Artifact folder unavailable.")?;
    }
    let file = directory.open_file(name)?.ok_or("Artifact unavailable.")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if file.metadata().map_err(|_| "Artifact unavailable.")?.mode() & 0o111 != 0 {
            return Err("Executable artifacts are excluded.");
        }
    }
    Ok(Target {
        home: home.to_owned(),
        alias: alias.into(),
        relative_path: relative_path.into(),
        path: project.path.join(".ghost").join(relative_path),
        file,
    })
}

pub(super) fn perform(
    home: &Path,
    alias: &str,
    relative_path: &str,
    action: Action,
    dispatch: impl FnOnce(&Target, Action) -> ActionState,
) -> ActionState {
    let Ok(target) = resolve(home, alias, relative_path) else {
        return ActionState::Rejected;
    };
    if target.revalidate().is_err() {
        return ActionState::Rejected;
    }
    dispatch(&target, action)
}

pub fn from_environment(alias: &str, relative_path: &str, action: Action) -> ActionState {
    let Ok(home) = super::resolve_home(
        std::env::var_os("GHOST_HOME"),
        std::env::home_dir(),
        std::env::current_dir().ok(),
    ) else {
        return ActionState::Rejected;
    };
    perform(&home, alias, relative_path, action, dispatch)
}

#[cfg(target_os = "macos")]
fn dispatch(target: &Target, action: Action) -> ActionState {
    macos::dispatch(target, action)
}

// The general opener launches helper processes. Do not silently use it on other platforms.
#[cfg(not(target_os = "macos"))]
fn dispatch(_: &Target, _: Action) -> ActionState {
    ActionState::Unavailable
}
