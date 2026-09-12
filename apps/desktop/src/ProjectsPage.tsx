import { artifactDate, sessionGoal, type GhostProject, type GhostSnapshot } from "./ghost-snapshot.ts";

type ProjectMode = GhostSnapshot["mode"];
export type ProjectsPageProps = {
  projects: GhostProject[];
  project?: GhostProject;
  mode: ProjectMode;
  query: string;
  onQueryChange: (query: string) => void;
  onSelect: (alias: string) => void;
  onNavigate?: (page: "Sessions" | "Artifacts") => void;
};

export function projectStatus(project: GhostProject, mode: ProjectMode): string {
  if (mode === "static-preview") return "Sample project";
  if (!project.path_exists) return "Project unavailable";
  if (!project.workspace_exists) return "Workspace unavailable";
  return "Read-only";
}

export function filterProjects(projects: GhostProject[], query: string, mode: ProjectMode): GhostProject[] {
  const terms = query.trim().toLowerCase().split(/\s+/).filter(Boolean);
  return projects.filter((project) => {
    const fields = [project.name, project.alias, project.path, project.status_preview,
      projectStatus(project, mode), project.active_session ? "Active session" : project.counts.sessions === 0 ? "Idle" : "Session unavailable"];
    const text = fields.filter(Boolean).join(" ").toLowerCase();
    return terms.every((term) => text.includes(term));
  });
}

export function projectOverview(projects: GhostProject[]) {
  const sessionsKnown = projects.every((project) => project.active_session || project.counts.sessions !== null);
  return {
    total: projects.length,
    activeSessions: sessionsKnown ? projects.reduce((count, project) => count + (project.active_session ? 1 : project.counts.sessions ?? 0), 0) : null,
    loadedArtifacts: projects.reduce((count, project) => count + project.recent_artifacts.length, 0),
  };
}

function SelectedProject({ project, mode, onNavigate }: Pick<ProjectsPageProps, "project" | "mode" | "onNavigate">) {
  if (!project) return null;
  return <details className="glass-panel project-detail" key={project.alias}>
    <summary><span className="page-eyebrow">Selected project</span><strong>{project.name}</strong><span className="project-detail-toggle">Details</span></summary>
    <div className="project-detail-content">
      <div>
        <dl className="project-detail-fields">
          <div><dt>Alias</dt><dd>{project.alias}</dd></div>
          <div><dt>Configured location</dt><dd className="project-location">{project.path || (mode === "static-preview" ? "Sample workspace · No local path" : "Not available")}</dd></div>
          <div><dt>Session goal</dt><dd>{sessionGoal(project)}</dd></div>
        </dl>
        {project.status_preview && <div className="project-recorded-status"><h3>Recorded status</h3><p>{project.status_preview}</p></div>}
      </div>
      <div className="project-detail-work">
        <h3>Recent loaded work</h3>
        {project.recent_artifacts.length ? <ul>{project.recent_artifacts.slice(0, 3).map((artifact) => <li key={artifact.relative_path}>
          <span>{artifact.kind.replace(/-/g, " ")}</span><strong>{artifact.title}</strong>
          {artifact.created_at && <time dateTime={artifact.created_at}>{artifactDate(artifact.created_at)}</time>}
        </li>)}</ul> : <p>No artifact previews loaded for this project.</p>}
        <div className="project-next-actions">
          {onNavigate && <><button type="button" onClick={() => onNavigate("Sessions")}>View session</button><button type="button" onClick={() => onNavigate("Artifacts")}>Review artifacts</button></>}
          <p>Continue from the CLI. This page only selects and reviews existing project metadata.</p>
        </div>
      </div>
    </div>
    {project.warnings.length > 0 && <div className="project-detail-notices"><h3>Project notices</h3><ul>{project.warnings.map((warning, index) => <li key={index}>{warning}</li>)}</ul></div>}
  </details>;
}

export default function ProjectsPage({ projects, project, mode, query, onQueryChange, onSelect, onNavigate }: ProjectsPageProps) {
  const overview = projectOverview(projects);
  const visible = filterProjects(projects, query, mode);
  const preview = mode === "static-preview";
  const selectionHidden = project && !visible.some((item) => item.alias === project.alias);

  return <div className="projects-workspace">
    <dl className="projects-overview" aria-label={preview ? "Sample project overview" : "Project overview"}>
      <div className="glass-panel"><dt>Total projects</dt><dd>{overview.total}</dd></div>
      <div className="glass-panel"><dt>Active sessions</dt><dd>{overview.activeSessions ?? "—"}</dd>{overview.activeSessions === null && <span>Not available for all projects</span>}</div>
      <div className="glass-panel"><dt>Selected project</dt><dd className="overview-project-name" title={project?.name}>{project?.name ?? "—"}</dd></div>
      <div className="glass-panel"><dt>Loaded artifacts</dt><dd>{overview.loadedArtifacts}</dd><span>Recent snapshot previews</span></div>
    </dl>
    {preview && <p className="projects-preview-note">Preview projects and counts are samples. Register your own projects from the CLI using <code>ghost project add</code>.</p>}
    {projects.length === 0 ? <div className="glass-panel page-panel page-empty"><h2>No registered projects</h2><p>Add projects from the CLI using <code>ghost project add</code>, then reopen GHOST to load the updated registry.</p></div> : <>
      <div className="projects-filter">
        <label htmlFor="project-filter">Filter projects</label>
        <div className="projects-filter-row">
          <input id="project-filter" type="search" value={query} onChange={(event) => onQueryChange(event.target.value)}
            placeholder="Name, alias, location, or status" autoComplete="off" spellCheck={false} maxLength={200} aria-describedby="project-filter-count" />
          {query && <button type="button" onClick={() => onQueryChange("")}>Clear filter</button>}
          <span id="project-filter-count" role="status">{visible.length} of {projects.length} projects</span>
        </div>
        {selectionHidden && <p className="projects-filter-notice">Your selected project is outside this filter. Selection is preserved.</p>}
      </div>
      <SelectedProject project={project} mode={mode} onNavigate={onNavigate} />
      {visible.length === 0 ? <div className="glass-panel page-panel page-empty"><h2>No projects match your search</h2><p>Try a project name, alias, location, or status. Clear the filter to see all registered projects.</p></div> :
        <div className="project-page-list" aria-label="Projects">
          {visible.map((item) => {
            const selected = item.alias === project?.alias;
            const recentArtifact = item.recent_artifacts[0];
            return <button key={item.alias} type="button" className={`glass-panel project-page-card${selected ? " selected" : ""}`}
              aria-pressed={selected} onClick={() => onSelect(item.alias)}>
              <div className="project-card-heading"><span className="page-eyebrow">{item.alias}</span>{selected && <span className="project-selected-badge">Selected</span>}</div>
              <h2>{item.name}</h2>
              <div className="project-card-badges"><span className="page-badge">{projectStatus(item, mode)}</span>{item.active_session && <span className="page-badge">{preview ? "Sample active session" : "Active session"}</span>}</div>
              <p className="project-card-goal">{sessionGoal(item)}</p>
              <span className="project-card-location">{item.path || (preview ? "Sample workspace · No local path" : "Location unavailable")}</span>
              <div className="project-card-latest"><span>Recent work</span><strong>{recentArtifact?.title ?? "No artifact previews loaded"}</strong></div>
              <div className="project-card-footer"><span>{item.recent_artifacts.length} loaded artifacts</span><span>{selected ? "Current project" : "Select project"}</span></div>
            </button>;
          })}
        </div>}
    </>}
  </div>;
}
