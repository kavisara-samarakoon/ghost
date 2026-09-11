use super::{
    names, preview, read_yaml,
    reader::{Directory, ReadBudget},
    text, warn, Project,
};
use serde::{Deserialize, Serialize};

#[derive(Serialize)]
pub(super) struct Session {
    pub id: String,
    pub goal_preview: String,
    pub status: &'static str,
    pub started_at: Option<String>,
    pub note_preview: Option<String>,
}

#[derive(Deserialize)]
struct Pointer {
    id: String,
    project_alias: String,
}

#[derive(Deserialize)]
struct Record {
    id: String,
    project_alias: String,
    goal: String,
    status: String,
    started_at: Option<String>,
    closed_at: Option<String>,
}

pub(super) fn load(
    workspace: &Directory,
    project: &Project,
    budget: &mut ReadBudget,
    warnings: &mut Vec<String>,
) -> (Option<Session>, Option<usize>) {
    match read(workspace, project, budget, warnings) {
        Ok(session) => {
            let count = usize::from(session.is_some());
            (session, Some(count))
        }
        Err(error) => {
            warn(
                warnings,
                &format!("{} active session", project.alias),
                error,
            );
            (None, None)
        }
    }
}

fn read(
    workspace: &Directory,
    project: &Project,
    budget: &mut ReadBudget,
    warnings: &mut Vec<String>,
) -> Result<Option<Session>, &'static str> {
    let Some(pointer) = read_yaml::<Pointer>(workspace, "active-session.yaml", budget)? else {
        return Ok(None);
    };
    if pointer.project_alias != project.alias || !names::session_id(&pointer.id) {
        return Err("Invalid active-session.yaml identity; session skipped.");
    }
    let sessions = workspace
        .child("sessions")?
        .ok_or("Active session directory is missing.")?;
    let folder = sessions
        .child(&pointer.id)?
        .ok_or("Active session directory is missing.")?;
    let record = read_yaml::<Record>(&folder, "session.yaml", budget)?
        .ok_or("Active session.yaml is missing.")?;
    if record.id != pointer.id
        || record.project_alias != project.alias
        || record.status != "active"
        || record.closed_at.is_some()
        || record.goal.trim().is_empty()
    {
        return Err(
            "Session identity or active status does not match its pointer; session skipped.",
        );
    }
    let goal = preview(&record.goal).ok_or("Active session goal is empty.")?;
    let started_at = record.started_at.as_deref().and_then(names::timestamp);
    if record.started_at.is_some() && started_at.is_none() {
        warn(
            warnings,
            &project.alias,
            "Invalid active session start time; timestamp omitted.",
        );
    }
    let note_preview = match folder.read("notes.md", budget) {
        Ok(Some(notes)) => text::excerpt(&notes, 600, true),
        Ok(None) => {
            warn(
                warnings,
                &project.alias,
                "Active session notes.md is missing.",
            );
            None
        }
        Err(error) => {
            warn(warnings, &format!("{} session notes", project.alias), error);
            None
        }
    };
    Ok(Some(Session {
        id: pointer.id,
        goal_preview: goal,
        status: "active",
        started_at,
        note_preview,
    }))
}
