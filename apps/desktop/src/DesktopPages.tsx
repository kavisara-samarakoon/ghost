import { useState, type RefObject } from "react";
import { artifactDate, sessionGoal, type GhostProject, type GhostSnapshot } from "./ghost-snapshot.ts";
import LatestWork from "./LatestWork.tsx";
import MemorySearch from "./MemorySearch.tsx";
import ProjectsPage from "./ProjectsPage.tsx";

export const pageNames = ["Command", "Projects", "Sessions", "Memory", "Artifacts"] as const;
export type DesktopPage = typeof pageNames[number];

type PageProps = {
  page: Exclude<DesktopPage, "Command">;
  projects: GhostProject[];
  project?: GhostProject;
  mode: GhostSnapshot["mode"];
  notice: string | null;
  warnings: string[];
  onSelect: (alias: string) => void;
  onNavigate?: (page: "Sessions" | "Artifacts") => void;
  searchInputRef: RefObject<HTMLInputElement | null>;
};

const descriptions = {
  Projects: "Coordinate local projects, sessions, memory, and generated work from one secure workspace.",
  Sessions: "Return to the goal, notes, and latest work for your selected project.",
  Memory: "Search local sessions, decisions, outputs, and drafts.",
  Artifacts: "Review the outputs and drafts that carry your work forward.",
};

function EmptyState({ title, children }: { title: string; children: string }) {
  return <div className="page-empty"><h2>{title}</h2><p>{children}</p></div>;
}

function ProjectPicker({ projects, project, onSelect }: Pick<PageProps, "projects" | "project" | "onSelect">) {
  if (!projects.length) return null;
  return <label className="page-project-picker">Current project
    <select value={project?.alias ?? ""} onChange={(event) => onSelect(event.target.value)}>
      {projects.map((item) => <option key={item.alias} value={item.alias}>{item.name}</option>)}
    </select>
  </label>;
}

function SessionsPage({ project, mode }: Pick<PageProps, "project" | "mode">) {
  const session = project?.active_session;
  return <div className="session-page-grid">
    <section className="glass-panel page-panel session-overview">
      <div className="page-panel-heading"><h2>{project?.name ?? "Current session"}</h2><span className="page-badge">{mode === "static-preview" ? "Sample" : session ? "Active session" : "No active session"}</span></div>
      {session ? <>
        <span className="page-eyebrow">Session goal</span>
        <p className="session-page-goal">{session.goal_preview}</p>
        {session.started_at && <p className="page-secondary">Started {artifactDate(session.started_at)}</p>}
        <div className="session-page-notes"><h3>Latest notes</h3><p>{session.note_preview ?? "No session notes recorded yet."}</p></div>
      </> : <EmptyState title={project ? sessionGoal(project) : "No project selected"}>Start or continue from the CLI. Your session overview appears here when local metadata is loaded.</EmptyState>}
      <div className="page-guidance"><strong>Start or continue from the CLI</strong><p>Keep session changes in your local GHOST workflow. Reopen this app to load the latest metadata.</p></div>
    </section>
    <section className="glass-panel page-panel"><div className="page-panel-heading"><h2>Latest work</h2></div><LatestWork key={project?.alias} project={project} mode={mode} /></section>
  </div>;
}

export default function DesktopPages(props: PageProps) {
  const [projectQuery, setProjectQuery] = useState("");
  const { page, project, mode, projects, onSelect, notice, warnings, searchInputRef } = props;
  return <div className={`desktop-page page-${page.toLowerCase()}`}>
    <header className="desktop-page-header">
      <div><p className="page-eyebrow">{mode === "live-local" ? "Your local workspace" : "Static preview · Sample data"}</p><h1 tabIndex={-1}>{page}</h1><p>{descriptions[page]}</p></div>
      {page === "Projects" ? <span className="page-badge projects-mode-badge">{mode === "live-local" ? "Live local read-only" : "Desktop preview"}</span> : <ProjectPicker projects={projects} project={project} onSelect={onSelect} />}
    </header>
    {notice && <p className="page-notice" role="status">{notice}</p>}
    {warnings.length > 0 && <details className="page-notice"><summary>Local metadata notices ({warnings.length})</summary><ul>{warnings.map((warning, i) => <li key={i}>{warning}</li>)}</ul></details>}
    {page === "Projects" && <ProjectsPage {...props} query={projectQuery} onQueryChange={setProjectQuery} />}
    {page === "Sessions" && <SessionsPage project={project} mode={mode} />}
    {page === "Memory" && <section className="glass-panel page-panel memory-page-panel" aria-label="Search workspace memory"><MemorySearch mode={mode} project={project} searchInputRef={searchInputRef} /></section>}
    {page === "Artifacts" && <section className="glass-panel page-panel artifacts-page-panel">
      <div className="page-panel-heading"><h2>{project ? `${project.name} · Recent artifacts` : "Recent artifacts"}</h2><span className="page-badge">{project?.recent_artifacts.length ?? 0} loaded</span></div>
      <p className="page-secondary">{mode === "static-preview" ? "Sample drafts for preview. Open and Reveal are available for eligible files in a live local workspace." : "Select a file to read its recorded preview and available actions."}</p>
      <LatestWork key={project?.alias} project={project} mode={mode} />
    </section>}
  </div>;
}
