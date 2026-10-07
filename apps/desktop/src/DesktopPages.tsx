import { useState, type RefObject } from "react";
import type { GhostProject, GhostSnapshot } from "./ghost-snapshot.ts";
import ArtifactsPage from "./ArtifactsPage.tsx";
import MemoryPage from "./MemoryPage.tsx";
import ProjectsPage from "./ProjectsPage.tsx";
import SessionsPage from "./SessionsPage.tsx";
import GoogleAssistant from "./GoogleAssistant.tsx";
import TodayPage from "./TodayPage.tsx";
import AutomationsPage from "./AutomationsPage.tsx";
import type { AutomationClient, AutomationState, CommandPrefill } from "./automations.ts";
import type { GoogleStatus } from "./google-assistant.ts";
import type { RecentRequest } from "./action-requests.ts";

export const pageNames = ["Command", "Today", "Mail", "Calendar", "Projects", "Memory", "Sessions", "Artifacts", "Automations", "Connections"] as const;
export type DesktopPage = typeof pageNames[number];

type PageProps = {
  automationState?: AutomationState; automationClient?: AutomationClient; onAutomationChanged?: () => Promise<void>;
  onOpenAutomation?: (page: DesktopPage, prefill: CommandPrefill | null, alias: string | null) => void;
  onHandleAutomation?: (id: string, status: "acknowledged" | "dismissed") => void;
  page: Exclude<DesktopPage, "Command">;
  projects: GhostProject[];
  project?: GhostProject;
  mode: GhostSnapshot["mode"];
  notice: string | null;
  warnings: string[];
  onSelect: (alias: string) => void;
  onNavigate?: (page: Exclude<DesktopPage, "Command">) => void;
  searchInputRef: RefObject<HTMLInputElement | null>;
  recentRequests?: RecentRequest[]; connections?: GoogleStatus | null; onConnections?: (status: GoogleStatus) => void;
};

const descriptions = {
  Projects: "Coordinate local projects, sessions, memory, and generated work from one secure workspace.",
  Sessions: "Review active project sessions, goals, notes, and the next safe step without running commands from the desktop.",
  Memory: "Search sessions, decisions, outputs, drafts, and generated context across your local GHOST workspace.",
  Artifacts: "Review context packs, handoffs, outputs, and drafts created by GHOST — without unsafe desktop execution.",
  Assistant: "Explicit Google access",
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
  if (page === "Mail" || page === "Calendar" || page === "Connections") return <GoogleAssistant view={page === "Mail" ? "mail" : page === "Calendar" ? "calendar" : "connections"} initialStatus={props.connections} onStatus={props.onConnections} />;
  if (page === "Today") return <TodayPage project={project} mode={mode} recent={props.recentRequests} connections={props.connections} automationItems={props.automationState?.inbox} onOpenAutomation={props.onOpenAutomation} onHandleAutomation={props.onHandleAutomation} />;
  if (page === "Automations") return <AutomationsPage state={props.automationState} projects={mode === "live-local" ? projects : []} client={props.automationClient} onChanged={props.onAutomationChanged} onOpen={props.onOpenAutomation} onHandle={props.onHandleAutomation} />;
  return <div className={`desktop-page page-${page.toLowerCase()}`}>
    <header className="desktop-page-header">
      <div><p className="page-eyebrow">{page === "Sessions" ? "Workflow continuity" : page === "Artifacts" ? "Generated work" : page === "Memory" ? "Local memory" : mode === "live-local" ? "Your local workspace" : "Static preview · Sample data"}</p><h1 tabIndex={-1}>{page}</h1><p>{descriptions[page]}</p></div>
      {page === "Projects" ? (
        <span className="page-badge projects-mode-badge">{mode === "live-local" ? "Live local read-only" : "Desktop preview"}</span>
      ) : page === "Sessions" || page === "Memory" || page === "Artifacts" ? (
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
    {page === "Memory" && <MemoryPage projects={projects} project={project} mode={mode} searchInputRef={searchInputRef} onNavigate={props.onNavigate} />}
    {page === "Artifacts" && <ArtifactsPage key={project?.alias} project={project} mode={mode} onNavigate={props.onNavigate} />}
  </div>;
}
