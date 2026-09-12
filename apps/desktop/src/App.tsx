import { useEffect, useState } from "react";
import { artifactDate, loadGhostSnapshot, selectProject, sessionGoal, type SnapshotState } from "./ghost-snapshot";
import LatestWork from "./LatestWork";
import MemorySearch from "./MemorySearch";
import "./App.css";

/**
 * GHOST Command Space — three-column workspace layout.
 *
 * Local metadata through a read-only Tauri command, with static browser preview.
 */

/* -----------------------------------------------------------------------
   Static preview action steps (shown when no snapshot is loaded)
   ----------------------------------------------------------------------- */

const previewProjects = ["NEXORA", "ARM-SecNet", "Portfolio", "SentinelLite AI"];

const actionSteps = [
  { number: "01", label: "Run validation" },
  { number: "02", label: "Review Codex output" },
  { number: "03", label: "Prepare update pack" },
] as const;

function ActionRow({ live }: { live: boolean }) {
  return (
    <div className="next-action-area">
      <div className="next-action-label">{live ? "Suggested workflow • manual steps" : "Next Action"}</div>
      <div className="action-row" role="list" aria-label="Next actions">
        {actionSteps.map((step, index) => (
          <div key={step.number} style={{ display: "contents" }}>
            {index > 0 && <div className="action-connector" />}
            <div
              className={`action-step${index === 0 ? " action-step-active" : ""}`}
              role="listitem"
            >
              <span className="action-step-number">{step.number}</span>
              <span className="action-step-text">{step.label}</span>
            </div>
          </div>
        ))}
      </div>
    </div>
  );
}

/* -----------------------------------------------------------------------
   Main App — three-column workspace
   ----------------------------------------------------------------------- */

function App() {
  const [{ snapshot, notice }, setSnapshot] = useState<SnapshotState>({ snapshot: null, notice: null });
  const [selectedAlias, setSelectedAlias] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    void loadGhostSnapshot().then((state) => {
      if (!cancelled) setSnapshot(state);
    });
    return () => { cancelled = true; };
  }, []);

  const live = snapshot !== null;
  const project = selectProject(snapshot?.projects ?? [], selectedAlias);
  const projectStatus = !project ? "No registered projects"
    : !project.path_exists ? "Project unavailable"
    : !project.workspace_exists ? "Workspace unavailable"
    : project.active_session ? "Session loaded"
    : "Read-only";

  return (
    <div className="command-space">
      {/* ---- Compact top header ---- */}
      <header className="app-header">
        <span className="brand-name">GHOST</span>
        <span className="header-sep" aria-hidden="true" />
        <span className="header-title">COMMAND SPACE</span>
        <span className="header-spacer" />
        <div className="header-status" role="status">
          <span className="status-dot" />
          <span>{live ? "Live local read-only" : "Static preview"}</span>
        </div>
        {live && (
          <span className="header-project-count">
            {snapshot.project_count} project{snapshot.project_count === 1 ? "" : "s"}
          </span>
        )}
      </header>

      {/* ---- Three-column workspace ---- */}
      <div className="workspace">

        {/* -- Left sidebar -- */}
        <aside className="sidebar">
          <span className="sidebar-section-title">Projects</span>

          {live && project ? (
            <>
              <select className="project-select" aria-label="Current project"
                value={project.alias} onChange={(event) => setSelectedAlias(event.target.value)}>
                {snapshot.projects.map((item) => <option key={item.alias} value={item.alias}>{item.name}</option>)}
              </select>

              <ul className="sidebar-project-list">
                {snapshot.projects.map((item) => (
                  <li key={item.alias}
                    className={`sidebar-project-item${item.alias === project.alias ? " active" : ""}`}
                    onClick={() => setSelectedAlias(item.alias)}>
                    <span className="sidebar-project-dot" />
                    <span className="sidebar-project-name">{item.name}</span>
                  </li>
                ))}
              </ul>
            </>
          ) : (
            <ul className="sidebar-project-list">
              {previewProjects.map((name) => (
                <li key={name} className="sidebar-project-item">
                  <span className="sidebar-project-dot" />
                  <span className="sidebar-project-name">{name}</span>
                </li>
              ))}
            </ul>
          )}

          <div className="sidebar-divider" />

          <div className="sidebar-safety">
            <div className="sidebar-safety-badge">
              <span className="status-dot" />
              {live ? projectStatus : "Read-only"}
            </div>
            <div>{live ? "Local metadata only" : "No live connection"}</div>
          </div>
        </aside>

        {/* -- Center panel -- */}
        <main className="center">
          <div className="session-block">
            <span className="session-label">{live && !project?.active_session ? "Local project" : "Current Session"}</span>
            {live && project ? (
              <select className="session-project project-select" aria-label="Current project"
                value={project.alias} onChange={(event) => setSelectedAlias(event.target.value)}>
                {snapshot.projects.map((item) => <option key={item.alias} value={item.alias}>{item.name}</option>)}
              </select>
            ) : <span className="session-project">{live ? "No registered projects" : "NEXORA"}</span>}
            <span className="session-goal">
              {live ? (project ? sessionGoal(project) : "Your local registry is empty.") : "Add wishlist price alert MVP"}
            </span>
            {project?.active_session && (
              <details className="snapshot-notice session-notes" key={project.alias}>
                <summary>Session notes</summary>
                {project.active_session.started_at && <span>Started {artifactDate(project.active_session.started_at)}</span>}
                <p>{project.active_session.note_preview ?? "No session note preview available."}</p>
              </details>
            )}
            {live && project?.status_preview && (
              <span className="project-status-preview" title={project.status_preview}>{project.status_preview}</span>
            )}
            <span className="status-pill">
              <span className="status-pill-dot" />
              {live ? projectStatus : "In Progress"}
            </span>
            {notice && <span className="snapshot-notice" role="status">{notice}</span>}
            {snapshot && snapshot.warnings.length > 0 && (
              <details className="snapshot-notice">
                <summary>{snapshot.warnings.length} snapshot notice{snapshot.warnings.length === 1 ? "" : "s"}</summary>
                <ul>{snapshot.warnings.map((warning, index) => <li key={index}>{warning}</li>)}</ul>
              </details>
            )}
          </div>

          {/* Latest work / action steps */}
          {live ? <LatestWork key={project?.alias} project={project} mode={snapshot.mode} /> : <ActionRow live={false} />}
        </main>

        {/* -- Right panel: memory search -- */}
        <aside className="right-panel">
          <div className="right-panel-inner">
            <div className="right-panel-header">
              <div className="right-panel-title">Memory</div>
              <div className="right-panel-helper">Search sessions, decisions, and artifacts across your projects.</div>
            </div>
            <MemorySearch mode={snapshot?.mode ?? "static-preview"} project={project} />
          </div>
        </aside>
      </div>

      {/* ---- Bottom status strip ---- */}
      <div className="status-strip">
        <span className="status-strip-text">
          {live ? "Local metadata only · No commands · No file writes" : "Local-first · Secrets protected · Manual approval required"}
        </span>
      </div>
    </div>
  );
}

export default App;
