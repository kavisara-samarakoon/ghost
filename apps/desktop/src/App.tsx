import { useEffect, useRef, useState, type RefObject } from "react";
import { isTauri } from "@tauri-apps/api/core";
import { loadGhostSnapshot, selectProject, type GhostProject, type GhostSnapshot, type SnapshotState } from "./ghost-snapshot.ts";
import DesktopPages, { pageNames, type DesktopPage } from "./DesktopPages.tsx";
import { sampleProjects } from "./preview-projects.ts";
import ghostLogo from "./assets/brand/ghost-logo.png";
import JarvisCommand from "./jarvis/JarvisCommand.tsx";
import ActionRequests from "./ActionRequests.tsx";
import type { GoogleStatus } from "./google-assistant.ts";
import type { RecentRequest } from "./action-requests.ts";
import "./App.css";
export const navigationGroups = [ { label: "Workspace", pages: ["Command", "Today", "Mail", "Calendar", "Projects", "Memory"] }, { label: "Library", pages: ["Sessions", "Artifacts"] }, { label: "System", pages: ["Automations", "Connections"] } ] as const;
export function Sidebar({ active, onNavigate, local }: { active: DesktopPage; onNavigate: (page: DesktopPage) => void; local: boolean }) {
  return <aside className="app-sidebar"><div className="sidebar-brand"><span className="brand-frame"><img src={ghostLogo} alt="GHOST logo" /></span><span>GHOST</span><small>Personal workspace</small></div>
    <nav aria-label="Main navigation">{navigationGroups.map(group => <section key={group.label}><h2>{group.label}</h2>{group.pages.map(page => <button key={page} className={active === page ? "selected" : ""} aria-current={active === page ? "page" : undefined} onClick={() => onNavigate(page)}><span className="nav-symbol" aria-hidden="true">{({ Command: "⌘", Today: "◷", Mail: "✉", Calendar: "▦", Projects: "□", Memory: "◇", Sessions: "◴", Artifacts: "▤", Automations: "⏱", Connections: "↔" } as const)[page]}</span><span>{page}</span>{page === "Automations" && <small>Next</small>}</button>)}</section>)}</nav>
    <footer><span className="local-dot" />{local ? "Local workspace" : "Static preview"}<p>AI plans. You decide.</p></footer>
  </aside>;
}
export function commandShortcut(event: Pick<KeyboardEvent, "metaKey" | "key" | "repeat">): boolean { return event.metaKey && event.key.toLowerCase() === "k" && !event.repeat; }
export function CommandPage({ projects, project, mode, onSelect, onNavigate, notice, warnings, recentRequests, onRequestSaved, commandInputRef }: { projects: GhostProject[]; project?: GhostProject; mode: GhostSnapshot["mode"]; onSelect: (alias: string) => void; onNavigate: (page: "Projects" | "Sessions" | "Memory" | "Artifacts") => void; notice: string | null; warnings: string[]; recentRequests?: RecentRequest[]; onRequestSaved?: () => void; commandInputRef?: RefObject<HTMLTextAreaElement | null> }) {
  return <section className="command-page"><header className="desktop-page-header"><div><p className="page-eyebrow">Your personal assistant</p><h1 id="command-title">Command</h1><p>Prepare a request. Review a proposal. Choose one safe step.</p></div><span className="badge">Planner only</span></header>
    <div className="command-project-picker"><label>Current project<select value={project?.alias ?? ""} onChange={event => onSelect(event.target.value)}><option value="">No project</option>{projects.map(p => <option key={p.alias} value={p.alias}>{p.name}{mode === "static-preview" ? " · sample" : ""}</option>)}</select></label><span>{mode === "live-local" ? "Live local read-only" : "Sample projects · no native binding"}</span></div>
    {notice && <p className="notice" role="status">{notice}</p>}{warnings.length > 0 && <details><summary>Local metadata notices ({warnings.length})</summary>{warnings.map((w, i) => <p key={i}>{w}</p>)}</details>}
    <JarvisCommand projectAlias={mode === "live-local" ? project?.alias ?? null : null} projects={projects} mode={mode} inputRef={commandInputRef} onRequestSaved={onRequestSaved} />
    <nav className="command-quick-links" aria-label="Local review destinations">{(["Projects", "Sessions", "Memory", "Artifacts"] as const).map((page, i) => <button key={page} className="command-action-card" aria-label={["Review Projects", "Continue Session", "Search Memory", "Review Artifacts"][i]} onClick={() => onNavigate(page)}>{page}</button>)}</nav>
    <details className="manual-requests"><summary>Manual local workflow requests</summary><ActionRequests projects={projects} project={project} available={mode === "live-local"} recent={recentRequests} onSaved={onRequestSaved} /></details>
  </section>;
}
export default function App() {
  const [{ snapshot, notice }, setSnapshot] = useState<SnapshotState>({ snapshot: null, notice: null }); const [selectedAlias, setSelectedAlias] = useState<string | null>(null); const [activePage, setActivePage] = useState<DesktopPage>("Command"); const [focusRequest, setFocusRequest] = useState(0); const [connections, setConnections] = useState<GoogleStatus | null>(null);
  const commandInputRef = useRef<HTMLTextAreaElement>(null); const searchInputRef = useRef<HTMLInputElement>(null); const mainRef = useRef<HTMLElement>(null);
  useEffect(() => { let cancelled = false; void loadGhostSnapshot().then(next => { if (!cancelled) setSnapshot(next); }); return () => { cancelled = true; }; }, []);
  useEffect(() => { if (mainRef.current) mainRef.current.scrollTop = 0; if (activePage === "Memory") searchInputRef.current?.focus(); if (activePage === "Command" && focusRequest) commandInputRef.current?.focus(); }, [activePage, focusRequest]);
  useEffect(() => { const key = (event: KeyboardEvent) => { if (commandShortcut(event)) { event.preventDefault(); setActivePage("Command"); setFocusRequest(n => n + 1); } }; window.addEventListener("keydown", key); return () => window.removeEventListener("keydown", key); }, []);
  const projects = snapshot?.projects ?? sampleProjects; const project = selectedAlias === "" ? undefined : selectProject(projects, selectedAlias); const mode = snapshot?.mode ?? "static-preview";
  function navigate(page: DesktopPage) { setActivePage(page); if (page === "Command") setFocusRequest(n => n + 1); }
  return <div className="app-shell"><Sidebar active={activePage} onNavigate={navigate} local={!!snapshot} /><div className="main-workspace"><div className="workspace-toolbar"><span>{activePage}</span><span className="toolbar-status">{snapshot ? "Local snapshot" : "Static preview"}</span><button disabled={!isTauri()} onClick={() => void loadGhostSnapshot().then(setSnapshot)}>Refresh local snapshot</button><button className="command-key" onClick={() => navigate("Command")} aria-label="Focus Ask GHOST">⌘ K</button></div>
    <main className="main-scroll" ref={mainRef}><div className="workspace-content">{activePage === "Command" ? <CommandPage projects={projects} project={project} mode={mode} onSelect={setSelectedAlias} onNavigate={navigate} notice={notice} warnings={snapshot?.warnings ?? []} recentRequests={snapshot?.recent_action_requests} onRequestSaved={() => void loadGhostSnapshot().then(setSnapshot)} commandInputRef={commandInputRef} /> : <DesktopPages key={activePage} page={activePage} projects={projects} project={project} mode={mode} notice={notice} warnings={snapshot?.warnings ?? []} onSelect={setSelectedAlias} onNavigate={navigate} searchInputRef={searchInputRef} recentRequests={snapshot?.recent_action_requests} connections={connections} onConnections={setConnections} />}</div></main>
    <footer className="app-safety">No shell/CLI execution · Local workflow requests remain drafts · Google changes require preview and confirmation</footer>
  </div></div>;
}
// Keep navigation finite and statically defined; no proposal selects an application route.
export { pageNames };
