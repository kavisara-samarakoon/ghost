import { useEffect, useState } from "react";
import { artifactDate, loadGhostSnapshot, selectProject, sessionGoal, type SnapshotState } from "./ghost-snapshot";
import LatestWork from "./LatestWork";
import "./App.css";

/**
 * GHOST Command Space — single-screen frontend.
 *
 * Local metadata through a read-only Tauri command, with static browser preview.
 */

/* -----------------------------------------------------------------------
   Orbit visual — SVG rings, spokes, and nodes
   ----------------------------------------------------------------------- */

const previewProjects = ["NEXORA", "ARM-SecNet", "Portfolio", "SentinelLite AI"];
const nodePositions = ["top", "right", "bottom", "left"];

function OrbitVisual({ projects, live, projectCount, outputCount }: {
  projects: string[];
  live: boolean;
  projectCount: number;
  outputCount: number | null;
}) {
  const cx = 210;
  const cy = 210;
  const r1 = 60;   /* inner dashed ring */
  const r2 = 110;  /* middle ring */
  const r3 = 160;  /* outer ring */

  /* Four node positions: top, right, bottom, left */
  const nodes = [
    { x: cx, y: cy - r3 + 10 },
    { x: cx + r3 - 10, y: cy },
    { x: cx, y: cy + r3 - 10 },
    { x: cx - r3 + 10, y: cy },
  ];

  return (
    <div className="orbit-container" role="group" aria-label="Project overview">
      <svg className="orbit-svg" viewBox="0 0 420 420" aria-hidden="true">
        {/* Outer orbit ring */}
        <circle cx={cx} cy={cy} r={r3} fill="none"
          stroke="rgba(77,216,232,0.06)" strokeWidth="1" />

        {/* Middle orbit ring */}
        <circle cx={cx} cy={cy} r={r2} fill="none"
          stroke="rgba(77,216,232,0.08)" strokeWidth="1" />

        {/* Inner dashed orbit ring */}
        <circle cx={cx} cy={cy} r={r1} fill="none"
          stroke="rgba(77,216,232,0.1)" strokeWidth="1"
          strokeDasharray="4 6" />

        {/* Crosshair lines through center */}
        <line x1={cx} y1={cy - r3 - 8} x2={cx} y2={cy + r3 + 8}
          stroke="rgba(77,216,232,0.05)" strokeWidth="1" />
        <line x1={cx - r3 - 8} y1={cy} x2={cx + r3 + 8} y2={cy}
          stroke="rgba(77,216,232,0.05)" strokeWidth="1" />

        {/* Spoke lines from center to each node */}
        {nodes.slice(0, projects.length).map((node, i) => (
          <line key={i} x1={cx} y1={cy} x2={node.x} y2={node.y}
            stroke="rgba(77,216,232,0.06)" strokeWidth="1" />
        ))}

        {/* Small tick marks on outer ring at 45° angles */}
        {[45, 135, 225, 315].map((angle) => {
          const rad = (angle * Math.PI) / 180;
          const ix = cx + (r3 - 6) * Math.cos(rad);
          const iy = cy + (r3 - 6) * Math.sin(rad);
          const ox = cx + (r3 + 6) * Math.cos(rad);
          const oy = cy + (r3 + 6) * Math.sin(rad);
          return (
            <line key={angle} x1={ix} y1={iy} x2={ox} y2={oy}
              stroke="rgba(77,216,232,0.1)" strokeWidth="1" />
          );
        })}
      </svg>

      {/* Central dark orb with core dot */}
      <div className="orbit-core" />
      <div className="orbit-core-ring" />
      <div className="orbit-core-dot" />

      {/* Project nodes */}
      {projects.slice(0, 4).map((name, index) => (
        <div key={index} className={`orbit-node orbit-node-${nodePositions[index]}${index === 0 ? " orbit-node-active" : ""}`}>
          <div className="orbit-node-dot" />
          <span className="orbit-node-label" title={name}>{name}</span>
        </div>
      ))}

      {/* Status info labels */}
      <span className="orbit-info orbit-info-1 orbit-info-active">
        {live ? `${projectCount} registered project${projectCount === 1 ? "" : "s"}` : "Context loaded"}
      </span>
      <span className="orbit-info orbit-info-2">
        {live ? "Read-only snapshot" : "Codex handoff ready"}
      </span>
      <span className="orbit-info orbit-info-3">
        {live ? (outputCount === null ? "Output count unavailable" : `${outputCount} indexed outputs`) : "Validation pending"}
      </span>
    </div>
  );
}

/* -----------------------------------------------------------------------
   Inline SVG icons
   ----------------------------------------------------------------------- */

function SparkleIcon() {
  return (
    <svg className="command-bar-icon" viewBox="0 0 24 24" fill="none"
      stroke="currentColor" strokeWidth="1.5" strokeLinecap="round"
      strokeLinejoin="round" aria-hidden="true">
      <path d="M12 2v4m0 12v4M4.93 4.93l2.83 2.83m8.48 8.48 2.83 2.83M2 12h4m12 0h4M4.93 19.07l2.83-2.83m8.48-8.48 2.83-2.83" />
    </svg>
  );
}

function SendIcon() {
  return (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor"
      strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round"
      aria-hidden="true">
      <path d="M5 12h14m-6-6 6 6-6 6" />
    </svg>
  );
}

/* -----------------------------------------------------------------------
   Next action steps (timeline style)
   ----------------------------------------------------------------------- */

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
   Main App
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
  const orbitProjects = snapshot
    ? [project, ...snapshot.projects.filter((item) => item.alias !== project?.alias)]
      .filter((item) => item !== undefined).map((item) => item.name)
    : previewProjects;
  const projectStatus = !project ? "No registered projects"
    : !project.path_exists ? "Project unavailable"
    : !project.workspace_exists ? "Workspace unavailable"
    : project.active_session ? "Session loaded"
    : "Read-only";

  return (
    <div className="command-space">
      {/* ---- Top bar ---- */}
      <div className="top-bar">
        <div className="window-dots" aria-hidden="true">
          <span className="window-dot red" />
          <span className="window-dot yellow" />
          <span className="window-dot green" />
        </div>

        <div className="brand-area">
          <span className="brand-name">GHOST</span>
        </div>

        <div className="top-rule" />
        <span className="top-title">COMMAND SPACE</span>
        <div className="top-rule-right" />

        <div className="top-status" role="status">
          <span className="status-dot" />
          <span>{live ? "Live local read-only" : "Static preview"}</span>
        </div>
      </div>

      {/* ---- Main content: greeting + orbit ---- */}
      <div className="main-content">
        <div className="left-panel">
          <div className="greeting">
            <h1>
              Good evening,
              <br />
              Kavisara
            </h1>
            <p className="greeting-subtitle">
              {live
                ? `${snapshot.project_count} registered project${snapshot.project_count === 1 ? "" : "s"} in your local GHOST workspace.`
                : "Your secure AI workflow coordinator is ready."}
            </p>
          </div>

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
        </div>

        <div className="right-panel">
          <OrbitVisual projects={orbitProjects} live={live}
            projectCount={snapshot?.project_count ?? 4} outputCount={project?.recent_output_count ?? null} />
        </div>
      </div>

      {/* ---- Next action row ---- */}
      {live ? <LatestWork key={project?.alias} project={project} mode={snapshot.mode} /> : <ActionRow live={false} />}

      {/* ---- Command bar ---- */}
      <div className="command-bar-area">
        <div className="command-bar">
          <SparkleIcon />
          <input
            className="command-input"
            type="text"
            placeholder={live ? "Read-only snapshot • commands are unavailable" : "Ask GHOST or type a command..."}
            aria-label="Command input"
            readOnly
          />
          <button className="command-send" type="button" aria-label="Send unavailable" disabled>
            <SendIcon />
          </button>
        </div>
      </div>

      {/* ---- Footer safety line ---- */}
      <div className="safety-footer">
        <span className="safety-rule" />
        <span className="safety-text">
          {live ? "Local metadata only • No commands • No file writes" : "Local-first • Secrets protected • Manual approval required"}
        </span>
        <span className="safety-rule" />
      </div>
    </div>
  );
}

export default App;
