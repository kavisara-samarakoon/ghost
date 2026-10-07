import { useEffect, useRef, useState, type RefObject } from "react";
import { isTauri } from "@tauri-apps/api/core";
import { loadGhostSnapshot, selectProject, type GhostProject, type GhostSnapshot, type SnapshotState } from "./ghost-snapshot.ts";
import DesktopPages, { pageNames, type DesktopPage } from "./DesktopPages.tsx";
import { sampleProjects } from "./preview-projects.ts";
import ghostLogo from "./assets/brand/ghost-logo.png";
import JarvisCommand from "./jarvis/JarvisCommand.tsx";
import ActionRequests from "./ActionRequests.tsx";
import { AutomationClient, automationError, startAutomationPolling, type AutomationState, type CommandPrefill } from "./automations.ts";
import type { AutomationItem } from "./automations.ts";
import type { GoogleStatus } from "./google-assistant.ts";
import type { RecentRequest } from "./action-requests.ts";
import "./App.css";
export const navigationGroups = [ { label: "Workspace", pages: ["Command", "Today", "Mail", "Calendar", "Projects", "Memory"] }, { label: "Library", pages: ["Sessions", "Artifacts"] }, { label: "System", pages: ["Automations", "Connections"] } ] as const;
export function Sidebar({ active, onNavigate, local, pendingCount = 0 }: { pendingCount?: number; active: DesktopPage; onNavigate: (page: DesktopPage) => void; local: boolean }) {
  return <aside className="app-sidebar"><div className="sidebar-brand"><span className="brand-frame"><img src={ghostLogo} alt="GHOST logo" /></span><span>GHOST</span><small>Personal workspace</small></div>
    <nav aria-label="Main navigation">{navigationGroups.map(group => <section key={group.label}><h2>{group.label}</h2>{group.pages.map(page => <button key={page} className={active === page ? "selected" : ""} aria-current={active === page ? "page" : undefined} onClick={() => onNavigate(page)}><span className="nav-symbol" aria-hidden="true">{({ Command: "⌘", Today: "◷", Mail: "✉", Calendar: "▦", Projects: "□", Memory: "◇", Sessions: "◴", Artifacts: "▤", Automations: "⏱", Connections: "↔" } as const)[page]}</span><span>{page}</span>{page === "Automations" && pendingCount > 0 && <small aria-label="Pending automation attention items">{pendingCount}</small>}</button>)}</section>)}</nav>
    <footer><span className="local-dot" />{local ? "Local workspace" : "Static preview"}<p>AI plans. You decide.</p></footer>
  </aside>;
}
export function commandShortcut(event: Pick<KeyboardEvent, "metaKey" | "key" | "repeat">): boolean { return event.metaKey && event.key.toLowerCase() === "k" && !event.repeat; }
export function CommandPage({ projects, project, mode, onSelect, onNavigate, notice, warnings, recentRequests, onRequestSaved, commandInputRef, prefill, onPrefillConsumed }: { prefill?: CommandPrefill | null; onPrefillConsumed?: () => void; projects: GhostProject[]; project?: GhostProject; mode: GhostSnapshot["mode"]; onSelect: (alias: string) => void; onNavigate: (page: "Projects" | "Sessions" | "Memory" | "Artifacts") => void; notice: string | null; warnings: string[]; recentRequests?: RecentRequest[]; onRequestSaved?: () => void; commandInputRef?: RefObject<HTMLTextAreaElement | null> }) {
  return <section className="command-page"><header className="desktop-page-header"><div><p className="page-eyebrow">Your personal assistant</p><h1 id="command-title">Command</h1><p>Prepare a request. Review a proposal. Choose one safe step.</p></div><span className="badge">Planner only</span></header>
    <div className="command-project-picker"><label>Current project<select value={project?.alias ?? ""} onChange={event => onSelect(event.target.value)}><option value="">No project</option>{projects.map(p => <option key={p.alias} value={p.alias}>{p.name}{mode === "static-preview" ? " · sample" : ""}</option>)}</select></label><span>{mode === "live-local" ? "Live local read-only" : "Sample projects · no native binding"}</span></div>
    {notice && <p className="notice" role="status">{notice}</p>}{warnings.length > 0 && <details><summary>Local metadata notices ({warnings.length})</summary>{warnings.map((w, i) => <p key={i}>{w}</p>)}</details>}
    <JarvisCommand projectAlias={mode === "live-local" ? project?.alias ?? null : null} projects={projects} mode={mode} inputRef={commandInputRef} prefill={prefill} onPrefillConsumed={onPrefillConsumed} onRequestSaved={onRequestSaved} />
    <nav className="command-quick-links" aria-label="Local review destinations">{(["Projects", "Sessions", "Memory", "Artifacts"] as const).map((page, i) => <button key={page} className="command-action-card" aria-label={["Review Projects", "Continue Session", "Search Memory", "Review Artifacts"][i]} onClick={() => onNavigate(page)}>{page}</button>)}</nav>
    <details className="manual-requests"><summary>Manual local workflow requests</summary><ActionRequests projects={projects} project={project} available={mode === "live-local"} recent={recentRequests} onSaved={onRequestSaved} /></details>
  </section>;
}
export default function App() {
  const [{ snapshot, notice }, setSnapshot] = useState<SnapshotState>({ snapshot: null, notice: null }); const [selectedAlias, setSelectedAlias] = useState<string | null>(null); const [activePage, setActivePage] = useState<DesktopPage>("Command"); const [focusRequest, setFocusRequest] = useState(0); const [connections, setConnections] = useState<GoogleStatus | null>(null);
  const [automationClient] = useState(() => new AutomationClient());
  const [automationState, setAutomationState] = useState<AutomationState>({ definitions: [], inbox: [] });
  const [automationMessage, setAutomationMessage] = useState("");
  const [pendingPrefill, setPendingPrefill] = useState<CommandPrefill | null>(null);
  const commandInputRef = useRef<HTMLTextAreaElement>(null); const searchInputRef = useRef<HTMLInputElement>(null); const mainRef = useRef<HTMLElement>(null);
  useEffect(() => { let cancelled = false; void loadGhostSnapshot().then(next => { if (!cancelled) setSnapshot(next); }); return () => { cancelled = true; }; }, []);
  useEffect(() => { if (mainRef.current) mainRef.current.scrollTop = 0; if (activePage === "Memory") searchInputRef.current?.focus(); if (activePage === "Command" && focusRequest) commandInputRef.current?.focus(); }, [activePage, focusRequest]);
  useEffect(() => { const key = (event: KeyboardEvent) => { if (commandShortcut(event)) { event.preventDefault(); setActivePage("Command"); setFocusRequest(n => n + 1); } }; window.addEventListener("keydown", key); return () => window.removeEventListener("keydown", key); }, []);
  useEffect(() => startAutomationPolling(automationClient.available(), async () => {
    const result = await automationClient.evaluate();
    const state = await automationClient.load();
    return { state, audited: result.audit_recorded };
  }, result => { setAutomationState(result.state); setAutomationMessage(result.audited ? "" : "Automation completion audit needs review. No task was executed."); }, error => setAutomationMessage(automationError(error)), {
    visible: () => document.visibilityState === "visible", now: () => Date.now(),
    interval: (callback, ms) => window.setInterval(callback, ms), clear: id => window.clearInterval(id as number),
    listen: (name, callback) => { const target = name === "focus" ? window : document; target.addEventListener(name, callback); return () => target.removeEventListener(name, callback); },
  }), [automationClient]);
  const projects = snapshot?.projects ?? sampleProjects; const project = selectedAlias === "" ? undefined : selectProject(projects, selectedAlias); const mode = snapshot?.mode ?? "static-preview";
  function navigate(page: DesktopPage) { setActivePage(page); if (page === "Command") setFocusRequest(n => n + 1); }
  async function refreshAutomations() { if (!automationClient.available()) return; setAutomationState(await automationClient.load()); }
  function openDue(page: DesktopPage, prefill: CommandPrefill | null, alias: string | null) {
    if (!automationClient.available()) return;
    if (alias && (!snapshot || !snapshot.projects.some(p => p.alias === alias))) { setAutomationMessage("The saved project binding is unavailable. Review the pending item; nothing was prepared."); return; }
    if (prefill) { setSelectedAlias(alias ?? ""); setPendingPrefill(prefill); }
    navigate(page);
  }
  async function handleDue(id: string, status: "acknowledged" | "dismissed") {
    try { const audit = status === "acknowledged" ? await automationClient.acknowledge(id) : await automationClient.dismiss(id); await refreshAutomations(); setAutomationMessage(audit ? "" : "Item status changed; completion audit needs review."); } catch (error) { setAutomationMessage(automationError(error)); }
  }
  const pendingCount = automationState.inbox.filter((item: AutomationItem) => item.status === "pending").length;
  return <div className="app-shell"><Sidebar active={activePage} onNavigate={navigate} local={!!snapshot} pendingCount={pendingCount} /><div className="main-workspace"><div className="workspace-toolbar"><span>{activePage}</span><span className="toolbar-status">{snapshot ? "Local snapshot" : "Static preview"}</span><button disabled={!isTauri()} onClick={() => void loadGhostSnapshot().then(setSnapshot)}>Refresh local snapshot</button><button className="command-key" onClick={() => navigate("Command")} aria-label="Focus Ask GHOST">⌘ K</button></div>
    <main className="main-scroll" ref={mainRef}><div className="workspace-content">{activePage === "Command" ? <CommandPage projects={projects} project={project} mode={mode} onSelect={setSelectedAlias} onNavigate={navigate} notice={notice} warnings={snapshot?.warnings ?? []} recentRequests={snapshot?.recent_action_requests} onRequestSaved={() => void loadGhostSnapshot().then(setSnapshot)} commandInputRef={commandInputRef} prefill={pendingPrefill} onPrefillConsumed={() => setPendingPrefill(null)} /> : <DesktopPages key={activePage} page={activePage} projects={projects} project={project} mode={mode} notice={notice} warnings={snapshot?.warnings ?? []} onSelect={setSelectedAlias} onNavigate={navigate} searchInputRef={searchInputRef} recentRequests={snapshot?.recent_action_requests} connections={connections} onConnections={setConnections} automationState={automationState} automationClient={automationClient} onAutomationChanged={refreshAutomations} onOpenAutomation={openDue} onHandleAutomation={(id, status) => void handleDue(id, status)} />}</div></main>
    {automationMessage && <p className="notice" role="status">{automationMessage}</p>}<footer className="app-safety">No shell/CLI execution · Local workflow requests remain drafts · Google changes require preview and confirmation</footer>
  </div></div>;
}
// Keep navigation finite and statically defined; no proposal selects an application route.
export { pageNames };
