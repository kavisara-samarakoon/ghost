import { useEffect, useRef, useState } from "react";
import { loadGhostSnapshot, selectProject, type GhostProject, type GhostSnapshot, type SnapshotState } from "./ghost-snapshot";
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

const commandActions = [
  { page: "Projects", label: "Review Projects", detail: "Choose the local workspace you are currently working on." },
  { page: "Sessions", label: "Continue Session", detail: "Inspect the current goal, notes, and session context." },
  { page: "Memory", label: "Search Memory", detail: "Find prior decisions, outputs, and next steps with explicit search." },
  { page: "Artifacts", label: "Review Artifacts", detail: "Open generated drafts and reports through approved controls." },
] as const satisfies ReadonlyArray<{ page: Exclude<DesktopPage, "Command">; label: string; detail: string }>;

type CommandPageProps = {
  projects: GhostProject[];
  project?: GhostProject;
  mode: GhostSnapshot["mode"];
  notice: string | null;
  warnings: string[];
  onSelect: (alias: string) => void;
  onNavigate: (page: Exclude<DesktopPage, "Command">) => void;
};

export function CommandPage({ projects, project, mode, notice, warnings, onSelect, onNavigate }: CommandPageProps) {
  const preview = mode === "static-preview";
  const session = project?.active_session;
  const artifact = project?.recent_artifacts[0];
  const projectPath = project?.path || "—";

  return <div className="command-page">
    <section className="hero-section command-hero" aria-labelledby="command-title">
      <div className="hero-text">
        <p className="page-eyebrow command-eyebrow">Local-first workflow command</p>
        <h1 className="hero-headline" id="command-title">
          Command Space<br />
          <span className="hero-headline-accent">for Your Personal</span><br />
          AI Workflow
        </h1>
        <p className="hero-support">
          Coordinate projects, sessions, memory, and generated artifacts from one secure local workspace.
        </p>
        <div className="hero-buttons">
          <button className="btn-primary" type="button" onClick={() => onNavigate("Sessions")}>
            Continue Session
            <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" aria-hidden="true">
              <path d="M5 12h14m-6-6 6 6-6 6" />
            </svg>
          </button>
          <button className="btn-secondary" type="button" onClick={() => onNavigate("Memory")}>
            <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5" aria-hidden="true">
              <circle cx="10.5" cy="10.5" r="6.5" /><path d="m16 16 5 5" />
            </svg>
            Search Memory
          </button>
        </div>
      </div>

      <OrbitVisualization projectNames={projects.slice(0, 4).map((item) => item.name)} />
    </section>

    <section className="command-cockpit" aria-label="Command workspace overview">
      <div className="command-cockpit-grid">
        <section className="glass-panel command-workflow" aria-labelledby="current-workflow-title">
          <div className="command-panel-heading">
            <div>
              <p className="page-eyebrow">Today’s workspace</p>
              <h2 id="current-workflow-title">Current workflow</h2>
            </div>
            <span className="page-badge">{preview ? "Desktop preview" : "Live local read-only"}</span>
          </div>

          {projects.length > 0 && <label className="command-project-picker">Selected project
            <select value={project?.alias ?? ""} onChange={(event) => onSelect(event.target.value)}>
              {projects.map((item) => <option key={item.alias} value={item.alias}>{item.name}</option>)}
            </select>
          </label>}

          <dl className="command-workflow-facts">
            <div><dt>Selected project</dt><dd>{project?.name ?? "—"}</dd></div>
            <div><dt>Project path</dt><dd className="command-path" title={projectPath}>{projectPath}</dd></div>
            <div><dt>Active session</dt><dd>{session?.id ?? "No active session"}</dd><span>{session ? session.status : project ? "Manual start from CLI" : "—"}</span></div>
            <div><dt>Latest artifact</dt><dd>{artifact?.title ?? "No artifact selected"}</dd><span>{artifact ? artifact.kind.replace(/-/g, " ") : "—"}</span></div>
            <div><dt>Loaded projects</dt><dd>{projects.length}</dd><span>{preview ? "Sample project metadata" : "Current local snapshot"}</span></div>
            <div><dt>Local mode</dt><dd>{preview ? "Static preview" : "Local · Secure"}</dd><span>Read-only desktop snapshot</span></div>
          </dl>

          {notice && <p className="command-notice" role="status">{notice}</p>}
          {warnings.length > 0 && <details className="command-notice"><summary>Local metadata notices ({warnings.length})</summary><ul>{warnings.map((warning, index) => <li key={index}>{warning}</li>)}</ul></details>}
        </section>

        <section className="glass-panel command-next" aria-labelledby="command-next-title">
          <div className="command-panel-heading">
            <div>
              <p className="page-eyebrow">Frontend navigation only</p>
              <h2 id="command-next-title">Suggested next steps</h2>
            </div>
            <span className="page-badge">Choose where to review</span>
          </div>
          <div className="command-action-grid">
            {commandActions.map((action, index) => <button key={action.page} type="button" className="command-action-card"
              aria-label={action.label} onClick={() => onNavigate(action.page)}>
              <span className="command-action-number">0{index + 1}</span>
              <span className="command-action-copy"><strong>{action.label}</strong><span>{action.detail}</span></span>
              <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.7" aria-hidden="true"><path d="M5 12h14m-6-6 6 6-6 6" /></svg>
            </button>)}
          </div>
        </section>
      </div>

      <div className="command-guidance-grid">
        <section className="glass-panel command-safety" aria-labelledby="command-safety-title">
          <div className="command-panel-heading">
            <div><p className="page-eyebrow">Human in the loop</p><h2 id="command-safety-title">Local MVP status</h2></div>
            <span className="status-pill"><span className="status-pill-dot" />Safe boundary</span>
          </div>
          <ul>
            <li>Desktop data is a read-only local snapshot.</li>
            <li>Open and Reveal are limited to approved generated artifacts.</li>
            <li>Memory search runs only after explicit submit.</li>
            <li>Generated work remains draft-first.</li>
            <li>No automatic publishing, deployment, or merge.</li>
            <li>No shell or CLI execution from the desktop yet.</li>
          </ul>
        </section>

        <section className="glass-panel command-flow" aria-labelledby="command-flow-title">
          <div className="command-panel-heading">
            <div><p className="page-eyebrow">Recommended flow</p><h2 id="command-flow-title">Move through work safely</h2></div>
            <span className="page-badge">Manual today</span>
          </div>
          <ol>
            <li><span>01</span><p><strong>Select or review a project</strong><small>Confirm the workspace in Projects.</small></p></li>
            <li><span>02</span><p><strong>Check the active session</strong><small>Review its goal, notes, and status.</small></p></li>
            <li><span>03</span><p><strong>Search memory</strong><small>Retrieve previous decisions with an explicit query.</small></p></li>
            <li><span>04</span><p><strong>Review artifacts</strong><small>Inspect generated drafts and reports.</small></p></li>
            <li><span>05</span><p><strong>Continue from the CLI</strong><small>Create new sessions and outputs there until safe write actions arrive.</small></p></li>
          </ol>
        </section>
      </div>
    </section>
  </div>;
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
  function handleNavClick(page: DesktopPage) {
    setActivePage(page);
    if (page === "Memory" && activePage === "Memory") searchInputRef.current?.focus();
  }

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
        {activePage === "Command" ? <CommandPage projects={projects} project={project}
          mode={snapshot?.mode ?? "static-preview"} notice={notice} warnings={snapshot?.warnings ?? []}
          onSelect={setSelectedAlias} onNavigate={handleNavClick} /> : <DesktopPages key={activePage} page={activePage} projects={projects} project={project}
          mode={snapshot?.mode ?? "static-preview"} notice={notice} warnings={snapshot?.warnings ?? []}
          onSelect={setSelectedAlias} onNavigate={handleNavClick} searchInputRef={searchInputRef} />}
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
