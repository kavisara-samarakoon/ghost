import { useEffect, useRef, useState } from "react";
import { artifactDate, loadGhostSnapshot, selectProject, sessionGoal, type SnapshotState } from "./ghost-snapshot";
import LatestWork from "./LatestWork";
import MemorySearch from "./MemorySearch";
import DesktopPages, { pageNames, type DesktopPage } from "./DesktopPages";
import { sampleProjects } from "./preview-projects";
import ghostLogo from "./assets/brand/ghost-logo.png";
import "./App.css";

/**
 * GHOST Command Space — Hero-style command center layout.
 *
 * Local metadata through a read-only Tauri command, with static browser preview.
 */

/* -----------------------------------------------------------------------
   Static preview data
   ----------------------------------------------------------------------- */

const actionSteps = [
  { number: "01", label: "Run validation" },
  { number: "02", label: "Review Codex output" },
  { number: "03", label: "Prepare update pack" },
] as const;

/* -----------------------------------------------------------------------
   Simple inline SVG icons for the project strip
   ----------------------------------------------------------------------- */

function StripIcon({ name }: { name: string }) {
  const n = name.toLowerCase();
  if (n.includes("nexora")) return (
    <svg className="strip-item-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5"><path d="M12 2L2 7l10 5 10-5-10-5z"/><path d="M2 17l10 5 10-5"/><path d="M2 12l10 5 10-5"/></svg>
  );
  if (n.includes("sentinel")) return (
    <svg className="strip-item-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5"><path d="M4 6h16M4 12h16M4 18h8"/></svg>
  );
  if (n.includes("arm") || n.includes("secnet")) return (
    <svg className="strip-item-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5"><path d="M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10z"/></svg>
  );
  if (n.includes("portfolio")) return (
    <svg className="strip-item-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5"><rect x="2" y="7" width="20" height="14" rx="2"/><path d="M16 7V5a2 2 0 0 0-2-2h-4a2 2 0 0 0-2 2v2"/></svg>
  );
  if (n.includes("university") || n.includes("work")) return (
    <svg className="strip-item-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5"><path d="M22 10v6M2 10l10-5 10 5-10 5z"/><path d="M6 12v5c0 1.7 2.7 3 6 3s6-1.3 6-3v-5"/></svg>
  );
  return (
    <svg className="strip-item-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5"><circle cx="12" cy="12" r="3"/><path d="M12 2v4m0 12v4m-10-10h4m12 0h4"/></svg>
  );
}

/* -----------------------------------------------------------------------
   Orbit visualization SVG
   ----------------------------------------------------------------------- */

function OrbitVisualization({ projectNames }: { projectNames: string[] }) {
  const cx = 190, cy = 190;
  const r1 = 65, r2 = 120, r3 = 170;

  // Position project nodes at cardinal directions on the outer ring
  const positions = [
    { x: cx, y: cy - r2, labelY: -18 },          // top
    { x: cx + r2, y: cy, labelY: 18 },            // right
    { x: cx, y: cy + r2, labelY: 22 },            // bottom
    { x: cx - r2, y: cy, labelY: 18 },            // left
  ];

  const names = projectNames.slice(0, 4);

  return (
    <div className="orbit-container">
      <svg className="orbit-svg" viewBox="0 0 380 380" aria-hidden="true">
        {/* Outer ring with slow rotation */}
        <g className="orbit-rotating">
          <circle cx={cx} cy={cy} r={r3} className="orbit-ring" />
          {/* Small accent dots on outer ring */}
          <circle cx={cx} cy={cy - r3} r="2" fill="rgba(77,216,232,0.2)" />
          <circle cx={cx + r3} cy={cy} r="1.5" fill="rgba(77,216,232,0.15)" />
          <circle cx={cx} cy={cy + r3} r="2" fill="rgba(77,216,232,0.2)" />
          <circle cx={cx - r3} cy={cy} r="1.5" fill="rgba(77,216,232,0.15)" />
        </g>

        {/* Middle ring */}
        <circle cx={cx} cy={cy} r={r2} className="orbit-ring orbit-ring-inner" />

        {/* Inner ring */}
        <circle cx={cx} cy={cy} r={r1} className="orbit-ring orbit-ring-inner" />

        {/* Connecting lines from center to nodes */}
        {positions.map((pos, i) => (
          <line key={`line-${i}`} x1={cx} y1={cy} x2={pos.x} y2={pos.y} className="orbit-line" />
        ))}

        {/* Center emblem */}
        <circle cx={cx} cy={cy} r={42} className="orbit-center-glow" />
        <circle cx={cx} cy={cy} r={42} className="orbit-center-ring" />

        {/* Project nodes */}
        {names.map((name, i) => {
          const pos = positions[i];
          if (!pos) return null;
          const labelWidth = Math.max(name.length * 7.5, 60);
          return (
            <g key={`node-${i}`}>
              <circle cx={pos.x} cy={pos.y} r="7" className="orbit-node-dot" />
              <circle cx={pos.x} cy={pos.y} r="12" fill="none" stroke="rgba(77,216,232,0.15)" strokeWidth="1" />
              <rect
                x={pos.x - labelWidth / 2} y={pos.y + pos.labelY - 8}
                width={labelWidth} height="18"
                className="orbit-node-label-bg"
              />
              <text x={pos.x} y={pos.y + pos.labelY + 4} className="orbit-node-label">{name}</text>
            </g>
          );
        })}
      </svg>
      <div className="orbit-center-identity">
        <span className="orbit-logo-frame">
          <img src={ghostLogo} className="orbit-logo-image" alt="GHOST logo" draggable={false} />
        </span>
        <span className="orbit-center-text" aria-hidden="true">GHOST</span>
      </div>
    </div>
  );
}

/* -----------------------------------------------------------------------
   Action row (static preview fallback for session panel)
   ----------------------------------------------------------------------- */

function ActionRow({ live }: { live: boolean }) {
  return (
    <div className="next-action-area">
      <div className="next-action-label">{live ? "Suggested workflow \u2022 manual steps" : "Next Action"}</div>
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
   Main App — Hero-style command center
   ----------------------------------------------------------------------- */

function App() {
  const [{ snapshot, notice }, setSnapshot] = useState<SnapshotState>({ snapshot: null, notice: null });
  const [selectedAlias, setSelectedAlias] = useState<string | null>(null);
  const [activePage, setActivePage] = useState<DesktopPage>("Command");
  const searchInputRef = useRef<HTMLInputElement>(null);
  const mainRef = useRef<HTMLElement>(null);

  useEffect(() => {
    if (mainRef.current) mainRef.current.scrollTop = 0;
    if (activePage === "Memory") searchInputRef.current?.focus();
    else mainRef.current?.querySelector("h1")?.focus();
  }, [activePage]);

  useEffect(() => {
    let cancelled = false;
    void loadGhostSnapshot().then((state) => {
      if (!cancelled) setSnapshot(state);
    });
    return () => { cancelled = true; };
  }, []);

  const live = snapshot !== null;
  const projects = snapshot?.projects ?? sampleProjects;
  const project = selectProject(projects, selectedAlias);
  const projectStatus = !project ? "No registered projects"
    : !project.path_exists ? "Project unavailable"
    : !project.workspace_exists ? "Workspace unavailable"
    : project.active_session ? "Session loaded"
    : "Read-only";

  const orbitNames = projects.slice(0, 4).map((item) => item.name);

  function handleNavClick(page: DesktopPage) {
    setActivePage(page);
    if (page === "Memory" && activePage === "Memory") searchInputRef.current?.focus();
  }

  function handleOpenSession() { handleNavClick("Sessions"); }
  function handleSearchMemory() { handleNavClick("Memory"); }

  return (
    <div className="command-space">
      {/* ---- Top navigation ---- */}
      <nav className="hero-nav" aria-label="Main navigation">
        <div className="nav-logo">
          <span className="nav-logo-emblem">
            <img className="nav-logo-image" src={ghostLogo} alt="GHOST logo" draggable={false} />
          </span>
          <span className="nav-logo-text">GHOST</span>
        </div>

        <ul className="nav-links">
          {pageNames.map((item) => (
            <li key={item}>
              <button type="button" className={`nav-link${activePage === item ? " active" : ""}`}
                aria-current={activePage === item ? "page" : undefined}
                onClick={() => handleNavClick(item)}>{item}</button>
            </li>
          ))}
        </ul>

        <div className="nav-spacer" />

        <div className="nav-status" role="status">
          <span className="status-dot" />
          <span>{live ? "Local \u00b7 Secure" : "Static preview"}</span>
        </div>

        <div className="nav-user-icon" aria-label="Settings">
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5" aria-hidden="true">
            <circle cx="12" cy="8" r="4" /><path d="M20 21a8 8 0 1 0-16 0" />
          </svg>
        </div>
      </nav>

      {/* ---- Scrollable main content ---- */}
      <main ref={mainRef} className={activePage === "Command" ? "hero-main" : "pages-main"}>
        {activePage === "Command" ? <>

        {/* ---- Hero section ---- */}
        <section className="hero-section">
          <div className="hero-text">
            <h1 className="hero-headline" tabIndex={-1}>
              Command Space<br />
              <span className="hero-headline-accent">for Your Personal</span><br />
              AI Workflow
            </h1>
            <p className="hero-support">
              Organize projects, sessions, memory, and artifacts — locally and securely.
            </p>
            <div className="hero-buttons">
              <button className="btn-primary" type="button" onClick={handleOpenSession}>
                Open Session
                <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" aria-hidden="true">
                  <path d="M5 12h14m-6-6 6 6-6 6" />
                </svg>
              </button>
              <button className="btn-secondary" type="button" onClick={handleSearchMemory}>
                <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5" aria-hidden="true">
                  <circle cx="10.5" cy="10.5" r="6.5" /><path d="m16 16 5 5" />
                </svg>
                Search Memory
              </button>
            </div>
          </div>

          <OrbitVisualization projectNames={orbitNames} />
        </section>

        {/* ---- Workflow panels ---- */}
        <section className="workflow-section">
          <div className="workflow-panels">

            {/* Panel A: Current Session */}
            <div className="glass-panel" id="panel-session">
              <div className="panel-header">
                <svg className="panel-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5">
                  <rect x="3" y="3" width="18" height="18" rx="2" /><path d="M3 9h18" />
                </svg>
                <span className="panel-title">
                  {live && !project?.active_session ? "Local Project" : "Current Session"}
                </span>
                <span className="panel-badge">
                  <span className="status-dot" />
                  {live ? projectStatus : project?.active_session ? "Sample session" : "Sample project"}
                </span>
              </div>

              <div className="session-content">
                {live && project ? (
                  <>
                    <select className="session-project-select" aria-label="Current project"
                      value={project.alias} onChange={(event) => setSelectedAlias(event.target.value)}>
                      {snapshot.projects.map((item) => <option key={item.alias} value={item.alias}>{item.name}</option>)}
                    </select>
                    <span className="session-goal-text">
                      {sessionGoal(project)}
                    </span>
                    {project.active_session && (
                      <details className="snapshot-notice session-notes" key={project.alias}>
                        <summary>Session notes</summary>
                        {project.active_session.started_at && <span>Started {artifactDate(project.active_session.started_at)}</span>}
                        <p>{project.active_session.note_preview ?? "No session note preview available."}</p>
                      </details>
                    )}
                    {project.status_preview && (
                      <span className="project-status-preview" title={project.status_preview}>{project.status_preview}</span>
                    )}
                  </>
                ) : (
                  <>
                    <span className="session-project-name">{live ? "No registered projects" : project?.name}</span>
                    <span className="session-goal-text">
                      {live ? "Your local registry is empty." : project ? sessionGoal(project) : "Choose a project"}
                    </span>
                    <span className="status-pill">
                      <span className="status-pill-dot" />
                      {live ? "No active session" : project?.active_session ? "Sample session" : "Sample project"}
                    </span>
                  </>
                )}
                {notice && <span className="snapshot-notice" role="status">{notice}</span>}
                {snapshot && snapshot.warnings.length > 0 && (
                  <details className="snapshot-notice">
                    <summary>{snapshot.warnings.length} snapshot notice{snapshot.warnings.length === 1 ? "" : "s"}</summary>
                    <ul>{snapshot.warnings.map((warning, index) => <li key={index}>{warning}</li>)}</ul>
                  </details>
                )}
              </div>
            </div>

            {/* Panel B: Recent Artifacts */}
            <div className="glass-panel" id="panel-artifacts">
              <div className="panel-header">
                <svg className="panel-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5">
                  <path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z" /><path d="M14 2v6h6" />
                </svg>
                <span className="panel-title">Recent Artifacts</span>
                {live && project && project.recent_artifacts.length > 0 && (
                  <span className="panel-badge">
                    {project.recent_artifacts.length} loaded
                  </span>
                )}
              </div>
              {live
                ? <LatestWork key={project?.alias} project={project} mode={snapshot.mode} />
                : <ActionRow live={false} />
              }
            </div>

            {/* Panel C: Memory Search */}
            <div className="glass-panel" id="panel-memory">
              <div className="panel-header">
                <svg className="panel-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5">
                  <circle cx="10.5" cy="10.5" r="6.5" /><path d="m16 16 5 5" />
                </svg>
                <span className="panel-title">Memory Search</span>
              </div>
              <MemorySearch
                mode={snapshot?.mode ?? "static-preview"}
                project={project}
                searchInputRef={searchInputRef}
              />
            </div>

          </div>
        </section>

        </> : <DesktopPages key={activePage} page={activePage} projects={projects} project={project}
          mode={snapshot?.mode ?? "static-preview"} notice={notice} warnings={snapshot?.warnings ?? []}
          onSelect={setSelectedAlias} searchInputRef={searchInputRef} />}
      </main>

      {/* ---- Bottom project strip ---- */}
      <div className="project-strip">
        {projects.slice(0, 5).map((item) => (
          <button type="button" key={item.alias}
            className={`strip-item${project?.alias === item.alias ? " active" : ""}`}
            aria-pressed={project?.alias === item.alias} onClick={() => setSelectedAlias(item.alias)}>
            <StripIcon name={item.name} />
            <span>{item.name}</span>
          </button>
        ))}
      </div>

      {/* ---- Safety strip ---- */}
      <div className="status-strip">
        <span className="status-strip-text">
          {live ? "Local metadata only \u00b7 No commands \u00b7 No file writes" : "Local-first \u00b7 Secure \u00b7 Private \u00b7 No commands \u00b7 No file writes"}
        </span>
      </div>
    </div>
  );
}

export default App;
