import { artifactDate, type GhostArtifact, type GhostProject, type GhostSnapshot } from "./ghost-snapshot.ts";
import LatestWork from "./LatestWork.tsx";

type SessionsPageProps = {
  projects: GhostProject[];
  project?: GhostProject;
  mode: GhostSnapshot["mode"];
  onNavigate?: (page: "Projects" | "Memory" | "Artifacts") => void;
};

export function activeSessionCount(projects: GhostProject[]): number | null {
  if (projects.some((project) => !project.active_session && project.counts.sessions === null)) return null;
  return projects.reduce((count, project) => count + (project.active_session ? 1 : project.counts.sessions ?? 0), 0);
}

export function latestArtifactTime(artifacts: GhostArtifact[]): string | null {
  return artifacts.reduce<string | null>((latest, artifact) => {
    const timestamp = artifact.created_at;
    if (!timestamp || !Number.isFinite(Date.parse(timestamp))) return latest;
    return !latest || Date.parse(timestamp) > Date.parse(latest) ? timestamp : latest;
  }, null);
}

function sessionStatus(project: GhostProject | undefined): string {
  if (!project) return "No project selected";
  if (project.active_session) return "Active session";
  return project.counts.sessions === 0 ? "No active session" : "Active session unavailable";
}

export default function SessionsPage({ projects, project, mode, onNavigate }: SessionsPageProps) {
  const session = project?.active_session;
  const count = activeSessionCount(projects);
  const status = sessionStatus(project);
  const latestTime = latestArtifactTime(project?.recent_artifacts ?? []);
  const preview = mode === "static-preview";

  return <div className="sessions-workspace">
    <dl className="sessions-overview" aria-label={preview ? "Sample session overview" : "Session overview"}>
      <div className="glass-panel"><dt>Active sessions</dt><dd>{count ?? "—"}</dd><span>{count === null ? "Some session metadata is unavailable" : "Across loaded projects"}</span></div>
      <div className="glass-panel"><dt>Selected project</dt><dd className="sessions-overview-text">{project?.name ?? "—"}</dd><span>{project?.alias ?? "Choose a registered project"}</span></div>
      <div className="glass-panel"><dt>Session status</dt><dd className="sessions-overview-text">{status}</dd><span>{preview ? "Sample metadata" : "Recorded in the local snapshot"}</span></div>
      <div className="glass-panel"><dt>Loaded artifacts</dt><dd>{project ? project.recent_artifacts.length : "—"}</dd><span>Selected project previews</span></div>
    </dl>
    {preview && <p className="sessions-preview-note">Static preview · Sample data. Sessions and work shown here are examples.</p>}

    <div className="sessions-content-grid">
      <section className={`glass-panel page-panel session-current${session ? " has-active-session" : ""}`} aria-labelledby="session-current-title">
        <div className="page-panel-heading"><h2 id="session-current-title">{project?.name ?? "Current session"}</h2><span className="page-badge">{preview ? `Sample · ${status}` : status}</span></div>
        {project && <p className="session-alias">Project alias · {project.alias}</p>}
        {session ? <>
          <p className="page-eyebrow">Active session goal</p>
          <p className="session-page-goal">{session.goal_preview}</p>
          <dl className="session-record-meta">
            <div><dt>Session ID</dt><dd>{session.id}</dd></div>
            <div><dt>Started</dt><dd>{artifactDate(session.started_at)}</dd></div>
            <div><dt>Latest dated project artifact</dt><dd>{latestTime ? artifactDate(latestTime) : "Date unavailable"}</dd></div>
          </dl>
        </> : <div className="page-empty">
          <h3>{status}</h3>
          <p>{!project ? "Select a registered project to review its session. Add projects from the CLI using ghost project add."
            : project.counts.sessions === 0 ? "Sessions are started from the CLI. Your active goal and notes will appear here after reloading the local snapshot."
            : "The snapshot does not contain readable active-session details. Check the session in the CLI before starting another."}</p>
          {project?.counts.sessions === 0 && <p className="session-start-guidance">Start a session from the CLI with <code>{"ghost session start <project-alias> --goal <goal>"}</code></p>}
        </div>}

        <div className="session-context" aria-label="Recorded session and project context">
          <h3>Recorded context</h3>
          <p className="page-secondary">Latest available previews; a complete session history is not loaded.</p>
          <ul className="session-context-list">
            {session && <li><h4>Session notes</h4><p>{session.note_preview || "No session notes recorded yet."}</p></li>}
            <li><h4>Project status</h4><p>{project?.status_preview || "No recorded project status available."}</p></li>
          </ul>
        </div>
        {!!project?.warnings.length && <details className="session-project-notices"><summary>Project notices ({project.warnings.length})</summary><ul>{project.warnings.map((warning, index) => <li key={index}>{warning}</li>)}</ul></details>}
      </section>

      <section className="glass-panel page-panel session-related-work" aria-labelledby="session-related-title">
        <div className="page-panel-heading"><h2 id="session-related-title">Related work</h2><span className="page-badge">{project?.recent_artifacts.length ?? 0} loaded</span></div>
        <p className="page-secondary">Recent outputs and drafts for the selected project. These records are not linked to a specific session.</p>
        <LatestWork key={project?.alias} project={project} mode={mode} />
      </section>
    </div>

    <section className="glass-panel page-panel session-next-step" aria-labelledby="session-next-title">
      <div><p className="page-eyebrow">Next safe step</p><h2 id="session-next-title">Continue from CLI</h2><p>Review the latest output, continue your session in the CLI, then reopen GHOST to load updated metadata.</p></div>
      <nav className="session-next-links" aria-label="Session next steps">
        {onNavigate ? <>
          <button type="button" onClick={() => onNavigate("Artifacts")}>Review artifacts<span>Open generated drafts and latest outputs</span></button>
          <button type="button" onClick={() => onNavigate("Memory")}>Search memory<span>Find session context</span></button>
          <button type="button" onClick={() => onNavigate("Projects")}>View projects<span>Review your workspace</span></button>
        </> : <p>Use Artifacts to review generated drafts, Memory to search session context, and Projects to review your workspace.</p>}
      </nav>
    </section>
  </div>;
}
