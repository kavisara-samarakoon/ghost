import { useState, type RefObject } from "react";
import type { GhostProject, GhostSnapshot } from "./ghost-snapshot.ts";
import LatestWork from "./LatestWork.tsx";
import MemorySearch from "./MemorySearch.tsx";
import ProjectsPage from "./ProjectsPage.tsx";
import SessionsPage from "./SessionsPage.tsx";

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
  onNavigate?: (page: Exclude<DesktopPage, "Command">) => void;
  searchInputRef: RefObject<HTMLInputElement | null>;
};

const descriptions = {
  Projects: "Coordinate local projects, sessions, memory, and generated work from one secure workspace.",
  Sessions: "Review active project sessions, goals, notes, and the next safe step without running commands from the desktop.",
  Memory: "Search local sessions, decisions, outputs, and drafts.",
  Artifacts: "Review the outputs and drafts that carry your work forward.",
};

function ProjectPicker({ projects, project, onSelect }: Pick<PageProps, "projects" | "project" | "onSelect">) {
  if (!projects.length) return null;
  return <label className="page-project-picker">Current project
    <select value={project?.alias ?? ""} onChange={(event) => onSelect(event.target.value)}>
      {projects.map((item) => <option key={item.alias} value={item.alias}>{item.name}</option>)}
    </select>
  </label>;
}

export default function DesktopPages(props: PageProps) {
  const [projectQuery, setProjectQuery] = useState("");
  const { page, project, mode, projects, onSelect, notice, warnings, searchInputRef } = props;
  return <div className={`desktop-page page-${page.toLowerCase()}`}>
    <header className="desktop-page-header">
      <div><p className="page-eyebrow">{page === "Sessions" ? "Workflow continuity" : mode === "live-local" ? "Your local workspace" : "Static preview · Sample data"}</p><h1 tabIndex={-1}>{page}</h1><p>{descriptions[page]}</p></div>
      {page === "Projects" ? (
        <span className="page-badge projects-mode-badge">{mode === "live-local" ? "Live local read-only" : "Desktop preview"}</span>
      ) : page === "Sessions" ? (
        <div className="sessions-header-controls">
          <span className="page-badge">{mode === "live-local" ? "Live local read-only" : "Desktop preview"}</span>
          <ProjectPicker projects={projects} project={project} onSelect={onSelect} />
        </div>
      ) : <ProjectPicker projects={projects} project={project} onSelect={onSelect} />}
    </header>
    {notice && <p className="page-notice" role="status">{notice}</p>}
    {warnings.length > 0 && <details className="page-notice"><summary>Local metadata notices ({warnings.length})</summary><ul>{warnings.map((warning, i) => <li key={i}>{warning}</li>)}</ul></details>}
    {page === "Projects" && <ProjectsPage {...props} query={projectQuery} onQueryChange={setProjectQuery} />}
    {page === "Sessions" && <SessionsPage projects={projects} project={project} mode={mode} onNavigate={props.onNavigate} />}
    {page === "Memory" && <section className="glass-panel page-panel memory-page-panel" aria-label="Search workspace memory"><MemorySearch mode={mode} project={project} searchInputRef={searchInputRef} /></section>}
    {page === "Artifacts" && <section className="glass-panel page-panel artifacts-page-panel">
      <div className="page-panel-heading"><h2>{project ? `${project.name} · Recent artifacts` : "Recent artifacts"}</h2><span className="page-badge">{project?.recent_artifacts.length ?? 0} loaded</span></div>
      <p className="page-secondary">{mode === "static-preview" ? "Sample drafts for preview. Open and Reveal are available for eligible files in a live local workspace." : "Select a file to read its recorded preview and available actions."}</p>
      <LatestWork key={project?.alias} project={project} mode={mode} />
    </section>}
  </div>;
}
