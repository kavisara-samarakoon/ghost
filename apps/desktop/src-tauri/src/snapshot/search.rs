//! Bounded, on-demand search. Only redacted memory text participates in matching.
use super::{
    names, preview, read_yaml, reader::Directory, reader::ReadBudget, text, Project, Registry,
    Safety,
};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, path::Path, sync::Mutex};

pub(super) const MAX_RESULTS: usize = 20;
pub(super) const SNIPPET_CHARS: usize = 240;
const MAX_WARNINGS: usize = 12;
const MAX_QUERY_CHARS: usize = 120;

#[derive(Serialize)]
pub struct SearchResult {
    pub project_alias: String,
    pub project_name: String,
    pub kind: &'static str,
    pub title: String,
    pub relative_path: String,
    pub snippet: String,
    pub created_at: Option<String>,
    pub openable: bool,
}

#[derive(Serialize)]
pub struct SearchResponse {
    pub query: String,
    pub mode: &'static str,
    pub results: Vec<SearchResult>,
    pub warnings: Vec<String>,
    safety: Safety,
}

impl SearchResponse {
    fn empty(query: &str) -> Self {
        Self {
            query: if query.len() <= MAX_QUERY_CHARS * 4 {
                text::redact(query.trim())
            } else {
                String::new()
            },
            mode: "unavailable",
            results: Vec::new(),
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

fn validate_query(query: &str, alias: Option<&str>) -> Result<(), &'static str> {
    if query.len() > MAX_QUERY_CHARS * 4 || query.trim().chars().count() > MAX_QUERY_CHARS {
        return Err("Keep your search to 120 characters or fewer.");
    }
    if query.trim().chars().count() < 2 {
        return Err("Enter at least 2 characters to search local memory.");
    }
    if query.chars().any(char::is_control) {
        return Err("Use a single line of text to search local memory.");
    }
    if alias.is_some_and(|alias| !super::valid_alias(alias)) {
        return Err("Choose a registered project to search.");
    }
    Ok(())
}

pub fn from_environment(query: &str, alias: Option<&str>) -> SearchResponse {
    let mut response = SearchResponse::empty(query);
    if let Err(message) = validate_query(query, alias) {
        response.warnings.push(message.into());
        return response;
    }
    // One native search at a time, including callers outside the UI's pending guard.
    static SEARCH: Mutex<()> = Mutex::new(());
    let Ok(_guard) = SEARCH.try_lock() else {
        response
            .warnings
            .push("A local search is already running. Try again shortly.".into());
        return response;
    };
    match super::resolve_home(
        std::env::var_os("GHOST_HOME"),
        std::env::home_dir(),
        std::env::current_dir().ok(),
    ) {
        Ok(home) => load(&home, query, alias, &mut ReadBudget::search()),
        Err(_) => {
            response
                .warnings
                .push("Local memory storage is unavailable.".into());
            response
        }
    }
}

pub(super) fn load(
    home: &Path,
    query: &str,
    alias: Option<&str>,
    budget: &mut ReadBudget,
) -> SearchResponse {
    let mut search = Search {
        response: SearchResponse::empty(query),
        needle: query.trim().to_lowercase(),
        budget,
    };
    if let Err(message) = validate_query(query, alias) {
        search.note(message);
        return search.response;
    }
    let registry = (|| {
        let storage = Directory::open(home)?.ok_or("Local memory storage is unavailable.")?;
        let registry = read_yaml::<Registry>(&storage, "projects.yaml", search.budget)?
            .ok_or("The local project registry is unavailable.")?;
        if registry.version != 1 || registry.projects.len() > super::MAX_PROJECTS {
            return Err("The local project registry is invalid or too large.");
        }
        Ok(registry)
    })();
    let Some(registry) = search.checked(registry) else {
        return search.response;
    };
    if alias.is_some_and(|alias| !registry.projects.iter().any(|p| p.alias == alias)) {
        search.note("Choose a registered project to search.");
        return search.response;
    }
    search.response.mode = "live-local";
    for project in &registry.projects {
        if search.done() {
            break;
        }
        if alias.is_some_and(|alias| alias != project.alias) {
            continue;
        }
        if !super::valid_alias(&project.alias)
            || project.name.trim().is_empty()
            || text::redact(&project.alias) != project.alias
            || registry
                .projects
                .iter()
                .filter(|p| p.alias == project.alias || p.path == project.path)
                .count()
                != 1
        {
            search.note("An invalid or ambiguous project was skipped.");
            continue;
        }
        let workspace = (|| {
            let root =
                Directory::open(&project.path)?.ok_or("A registered project is unavailable.")?;
            let workspace = root
                .child(".ghost")?
                .ok_or("A project memory workspace is unavailable.")?;
            let identity = read_yaml::<Project>(&workspace, "project.yaml", search.budget)?
                .ok_or("Project memory identity is unavailable.")?;
            if identity.alias != project.alias || identity.path != project.path {
                return Err("Mismatched project memory identity; project skipped.");
            }
            Ok(workspace)
        })();
        if let Some(workspace) = search.checked(workspace) {
            search.project(&workspace, project);
        }
    }
    search.done();
    search.response
}

struct Search<'a> {
    response: SearchResponse,
    needle: String,
    budget: &'a mut ReadBudget,
}

struct Document {
    path: String,
    kind: &'static str,
    title: Option<String>,
    created_at: Option<String>,
    openable: bool,
}

impl Document {
    fn new(path: &str, kind: &'static str) -> Self {
        Self {
            path: path.into(),
            kind,
            title: None,
            openable: false,
            created_at: path.split('/').rev().find_map(names::filename_time),
        }
    }
}

impl Search<'_> {
    fn note(&mut self, message: &str) {
        if self
            .response
            .warnings
            .iter()
            .any(|warning| warning == message)
        {
            return;
        }
        if self.response.warnings.len() < MAX_WARNINGS {
            self.response.warnings.push(message.into());
        }
    }

    fn checked<T>(&mut self, result: Result<T, &'static str>) -> Option<T> {
        match result {
            Ok(value) => Some(value),
            Err(message) => {
                self.note(message);
                None
            }
        }
    }

    fn done(&mut self) -> bool {
        if self.response.results.len() == MAX_RESULTS {
            self.note(
                "Showing the first 20 matches. Refine your search for more specific results.",
            );
            return true;
        }
        if self.budget.exhausted() {
            self.note(
                "Search budget reached; some local memory was not searched. Try one project.",
            );
            return true;
        }
        false
    }

    fn read(&mut self, workspace: &Directory, path: &str) -> Option<String> {
        if self.done() {
            return None;
        }
        let result = (|| {
            if !names::search_path(path) {
                return Err("A file outside the memory allowlist was skipped.");
            }
            let (folder, name) = path.rsplit_once('/').unwrap_or(("", path));
            if folder.is_empty() {
                return workspace.read(name, self.budget);
            }
            let Some(directory) = descend(workspace, folder)? else {
                return Ok(None);
            };
            directory.read(name, self.budget)
        })();
        self.checked(result).flatten()
    }

    fn yaml<T: for<'de> Deserialize<'de>>(&mut self, content: &str) -> Option<T> {
        self.checked(
            serde_yaml_ng::from_str(content)
                .map_err(|_| "Invalid YAML metadata was skipped; contents were not included."),
        )
    }

    fn files(&mut self, workspace: &Directory, path: &str) -> Vec<String> {
        if self.done() {
            return Vec::new();
        }
        let result = (|| {
            let Some(folder) = descend(workspace, path)? else {
                return Ok(None);
            };
            folder.search_list(self.budget).map(Some)
        })();
        let Some(listing) = self.checked(result).flatten() else {
            return Vec::new();
        };
        if listing.skipped {
            self.note("Unsafe or unsupported memory entries were skipped.");
        }
        let mut names = listing.names;
        // Timestamp filenames put recent memory first, with deterministic ordering.
        names.sort_by(|a, b| b.cmp(a));
        names
    }

    fn add(&mut self, workspace: &Directory, project: &Project, document: Document, content: &str) {
        if self.response.results.len() == MAX_RESULTS {
            return;
        }
        // Do not echo secret-looking storage identifiers or make them actionable.
        if text::redact(&document.path) != document.path {
            return;
        }
        let clean = text::redact(content);
        let title = document
            .title
            .as_deref()
            .and_then(preview)
            .or_else(|| {
                clean
                    .lines()
                    .find_map(|line| line.trim().strip_prefix("# "))
                    .and_then(preview)
            })
            .unwrap_or_else(|| {
                document
                    .path
                    .rsplit('/')
                    .next()
                    .unwrap_or("Local memory")
                    .into()
            });
        let flat = clean.split_whitespace().collect::<Vec<_>>().join(" ");
        if ![
            title.as_str(),
            document.kind,
            document.path.as_str(),
            flat.as_str(),
        ]
        .iter()
        .any(|value| value.to_lowercase().contains(&self.needle))
        {
            return;
        }
        let openable = document.openable
            && names::action_path(&document.path)
            && nonexecutable_file(workspace, &document.path);
        self.response.results.push(SearchResult {
            project_alias: project.alias.clone(),
            project_name: preview(&project.name).unwrap_or_else(|| project.alias.clone()),
            kind: document.kind,
            title,
            relative_path: document.path,
            snippet: snippet(&flat, &self.needle),
            created_at: document.created_at,
            openable,
        });
    }

    fn markdown(&mut self, workspace: &Directory, project: &Project, document: Document) {
        if let Some(content) = self.read(workspace, &document.path) {
            self.add(workspace, project, document, &content);
        }
    }

    fn project(&mut self, workspace: &Directory, project: &Project) {
        for (path, kind) in [("status.md", "status"), ("decisions.md", "decision")] {
            self.markdown(workspace, project, Document::new(path, kind));
        }
        if let Some(content) = self.read(workspace, "milestones.yaml") {
            if let Some(index) = self.yaml::<Milestones>(&content) {
                if index.version == 1
                    && index.milestones.len() <= super::reader::MAX_DIRECTORY_ENTRIES
                {
                    self.add(
                        workspace,
                        project,
                        Document::new("milestones.yaml", "milestone"),
                        &content,
                    );
                } else {
                    self.note("Invalid or oversized milestones metadata was skipped.");
                }
            }
        }
        self.sessions(workspace, project);
        self.outputs(workspace, project);
        for (path, kind) in [
            ("drafts/context-packs", "context-pack"),
            ("drafts/next-steps", "next-step"),
            ("drafts/handoffs/codex", "handoff"),
            ("drafts/handoffs/chatgpt", "handoff"),
            ("drafts/handoffs/gemini", "handoff"),
            ("drafts/handoffs/antigravity", "handoff"),
        ] {
            self.drafts(workspace, project, path, kind);
        }
        for pack in self.files(workspace, "drafts/update-packs") {
            if self.done() {
                break;
            }
            self.drafts(
                workspace,
                project,
                &format!("drafts/update-packs/{pack}"),
                "update-pack",
            );
        }
    }

    fn drafts(&mut self, workspace: &Directory, project: &Project, path: &str, kind: &'static str) {
        for name in self.files(workspace, path) {
            if self.done() {
                break;
            }
            let mut document = Document::new(&format!("{path}/{name}"), kind);
            document.openable = true;
            self.markdown(workspace, project, document);
        }
    }

    fn sessions(&mut self, workspace: &Directory, project: &Project) {
        let pointer = self
            .read(workspace, "active-session.yaml")
            .and_then(|content| self.yaml::<Pointer>(&content));
        let active_id = pointer.and_then(|pointer| {
            if pointer.project_alias == project.alias && names::session_id(&pointer.id) {
                let content = format!("Active session {}", pointer.id);
                self.add(
                    workspace,
                    project,
                    Document::new("active-session.yaml", "active-session"),
                    &content,
                );
                Some(pointer.id)
            } else {
                self.note("Invalid active session identity was skipped.");
                None
            }
        });
        for id in self.files(workspace, "sessions") {
            if self.done() {
                break;
            }
            let path = format!("sessions/{id}/session.yaml");
            let Some(content) = self.read(workspace, &path) else {
                continue;
            };
            let Some(record) = self.yaml::<SessionRecord>(&content) else {
                continue;
            };
            if record.id != id
                || record.project_alias != project.alias
                || record.goal.trim().is_empty()
                || !matches!(record.status.as_str(), "active" | "closed")
                || (record.status == "active" && record.closed_at.is_some())
            {
                self.note("Invalid session identity or status was skipped.");
                continue;
            }
            let created_at = record
                .started_at
                .as_deref()
                .and_then(names::timestamp)
                .or_else(|| names::filename_time(&id));
            let openable = active_id.as_deref() == Some(id.as_str()) && record.status == "active";
            let mut document = Document::new(&path, "session");
            document.title = Some(record.goal.clone());
            document.created_at = created_at.clone();
            document.openable = openable;
            self.add(
                workspace,
                project,
                document,
                &format!("{}\n{}", record.goal, record.status),
            );
            let mut notes = Document::new(&format!("sessions/{id}/notes.md"), "session-notes");
            notes.title = Some(format!("Session notes: {}", record.goal));
            notes.created_at = created_at;
            notes.openable = openable;
            self.markdown(workspace, project, notes);
        }
    }

    fn outputs(&mut self, workspace: &Directory, project: &Project) {
        let mut titles = HashMap::new();
        if let Some(content) = self.read(workspace, "outputs/index.yaml") {
            if let Some(index) = self.yaml::<OutputIndex>(&content) {
                if index.version == 1 && index.outputs.len() <= super::reader::MAX_DIRECTORY_ENTRIES
                {
                    let mut safe_index = String::new();
                    for entry in &index.outputs {
                        if entry.project_alias != project.alias
                            || entry.title.trim().is_empty()
                            || !names::output_path(&entry.id, &entry.kind, &entry.path)
                            || index
                                .outputs
                                .iter()
                                .filter(|other| other.path == entry.path || other.id == entry.id)
                                .count()
                                != 1
                            || text::redact(&entry.path) != entry.path
                        {
                            self.note("Invalid or ambiguous output index entries were skipped.");
                            continue;
                        }
                        safe_index.push_str(&format!(
                            "{}\n{}\n",
                            text::redact(&entry.title),
                            entry.path
                        ));
                        titles.insert(entry.path.clone(), entry.title.clone());
                    }
                    self.add(
                        workspace,
                        project,
                        Document::new("outputs/index.yaml", "output-index"),
                        &safe_index,
                    );
                } else {
                    self.note("Invalid or oversized output index was skipped.");
                }
            }
        }
        for kind in ["codex", "terminal"] {
            let folder = format!("outputs/{kind}");
            for file in self.files(workspace, &folder) {
                if self.done() {
                    break;
                }
                let path = format!("{folder}/{file}");
                let mut document = Document::new(&path, "output");
                document.title = titles.get(&path).cloned();
                document.openable = document.title.is_some();
                self.markdown(workspace, project, document);
            }
        }
    }
}

fn descend(root: &Directory, path: &str) -> Result<Option<Directory>, &'static str> {
    let mut parts = path.split('/');
    let Some(mut folder) = root.child(parts.next().ok_or("Missing memory folder.")?)? else {
        return Ok(None);
    };
    for part in parts {
        let Some(child) = folder.child(part)? else {
            return Ok(None);
        };
        folder = child;
    }
    Ok(Some(folder))
}

fn nonexecutable_file(workspace: &Directory, path: &str) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let Some((folder, name)) = path.rsplit_once('/') else {
            return false;
        };
        let file = descend(workspace, folder)
            .ok()
            .flatten()
            .and_then(|dir| dir.open_file(name).ok().flatten());
        file.and_then(|file| file.metadata().ok())
            .is_some_and(|metadata| metadata.mode() & 0o111 == 0)
    }
    #[cfg(not(unix))]
    {
        let _ = (workspace, path);
        false
    }
}

fn snippet(clean: &str, needle: &str) -> String {
    // Locate the match in original character coordinates (Unicode lowercase can expand).
    let mut start = 0;
    if let Some(byte) = clean.to_lowercase().find(needle) {
        let mut folded_bytes = 0;
        for (index, character) in clean.chars().enumerate() {
            if folded_bytes >= byte {
                start = index.saturating_sub(48);
                break;
            }
            folded_bytes += character.to_lowercase().map(char::len_utf8).sum::<usize>();
        }
    }
    let length = clean.chars().count();
    let mut snippet = String::new();
    if start > 0 {
        snippet.push('…');
    }
    snippet.extend(clean.chars().skip(start).take(SNIPPET_CHARS - 2));
    if length > start + SNIPPET_CHARS - 2 {
        snippet.push('…');
    }
    snippet
}

#[derive(Deserialize)]
struct Pointer {
    id: String,
    project_alias: String,
}

#[derive(Deserialize)]
struct SessionRecord {
    id: String,
    project_alias: String,
    goal: String,
    status: String,
    started_at: Option<String>,
    closed_at: Option<String>,
}

#[derive(Deserialize)]
struct Milestones {
    version: u32,
    milestones: Vec<serde_yaml_ng::Value>,
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
    title: String,
    path: String,
    #[serde(rename = "type")]
    kind: String,
}
