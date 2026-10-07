import { useEffect, useRef, useState } from "react";
import { localRequest, memorySuggestion, googleMutation, type JarvisProposal, type JarvisStep } from "./jarvis.ts";
import type { GhostProject, GhostSnapshot } from "../ghost-snapshot.ts";
import ActionRequests from "../ActionRequests.tsx";
import PersonalMemory from "../PersonalMemory.tsx";
import { PersonalMemoryClient } from "../personal-memory.ts";
import { GoogleAssistantClient, googleError, type GoogleStatus, type PreparedGoogleMutation } from "../google-assistant.ts";
import { GoogleMutationReview } from "../GoogleAssistant.tsx";
import { searchGhostMemory, type SearchResponse } from "../ghost-memory.ts";
import SearchResults from "../SearchResults.tsx";
export const capabilityLabels: Record<JarvisStep["capability"], string> = {
  search_project_memory: "Search project memory", start_session_request: "Draft a session request", add_session_note_request: "Draft a session note request", generate_next_steps_request: "Draft a next-steps request", create_handoff_request: "Draft a handoff request", remember_personal_memory: "Review a memory suggestion", search_mail: "Search Gmail", list_agenda: "Read Calendar agenda", find_free_time: "Find free time", lookup_contact: "Look up contacts", create_mail_draft: "Create a Gmail draft", send_mail: "Send an email", create_calendar_event: "Create a Calendar event",
};
export function PlanCard({ step, index, active, onReview, status = "Proposed", disabled = false }: { step: JarvisStep; index: number; active: boolean; onReview: () => void; status?: string; disabled?: boolean }) {
  return <article className="plan-card"><div className="plan-number">{index + 1}</div><div className="plan-copy"><h3>{capabilityLabels[step.capability] ?? "Unsupported step"}</h3><span>{status} · {step.capability}</span><details><summary>Proposed values</summary><pre>{JSON.stringify(step, null, 2)}</pre></details></div><button disabled={disabled} aria-expanded={active} onClick={onReview}>{active ? "Close review" : "Review step"}</button></article>;
}
export default function JarvisPlan({ proposal, projects, currentProject, mode, onRequestSaved }: { proposal: JarvisProposal; projects: GhostProject[]; currentProject: string | null; mode: GhostSnapshot["mode"]; onRequestSaved?: () => void }) {
  const [finished,setFinished] = useState<Record<number,string>>({});
  const [active, setActive] = useState<number | null>(null); const [status, setStatus] = useState<GoogleStatus | null>(null); const [account, setAccount] = useState(""); const [message, setMessage] = useState(""); const [busy, setBusy] = useState(false);
  const [google] = useState(() => new GoogleAssistantClient()); const mounted = useRef(true);
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; google.invalidate(); }; }, [google]);
  const hasGoogle = proposal.plan.steps.some(s => ["search_mail", "list_agenda", "find_free_time", "lookup_contact", "create_mail_draft", "send_mail", "create_calendar_event"].includes(s.capability));
  return <section className="jarvis-plan"><div className="section-heading"><h2>{proposal.plan.kind === "plan" ? "Proposed plan" : proposal.plan.kind === "clarify" ? "A little more detail" : "Outside GHOST’s capabilities"}</h2><span className="badge">Not executed</span></div><p className="proposal-summary">{proposal.plan.summary}</p>
    <p>Bound project: {proposal.project_alias ?? "None"}. Review one step at a time. A planning hash grants no action authority.</p>
    {currentProject !== proposal.project_alias && <p className="notice">Selection changed. This proposal remains bound to {proposal.project_alias ?? "no project"}; it will not silently use the current selection.</p>}
    <details><summary>Proposal integrity</summary><code>{proposal.proposal_sha256}</code></details>
    {hasGoogle && <div className="account-toolbar"><button disabled={!google.available || busy} onClick={async () => { if (busy) return; setBusy(true); try { const next = await google.status(); if (mounted.current) setStatus(next); } catch (e) { if (mounted.current) setMessage(googleError(e)); } finally { if (mounted.current) setBusy(false); } }}>Load local Google accounts</button><label>Account for Google steps<select value={account} disabled={busy} onChange={e => { google.invalidate(); setActive(null); setAccount(e.target.value); }}><option value="">Choose an account explicitly</option>{status?.accounts.filter(a => a.status === "connected").map(a => <option key={a.account_id} value={a.account_id}>{a.display_label} · {a.granted_permissions.join(", ")}</option>)}</select></label></div>}
    {proposal.plan.kind === "plan" && proposal.plan.steps.map((step, index) => <div key={index}><PlanCard step={step} index={index} active={active === index} status={finished[index] ?? "Proposed"} disabled={busy || !!finished[index]} onReview={() => { google.invalidate(); setActive(a => a === index ? null : index); }} />
      {active === index && <StepWorkspace key={`${index}/${account}`} step={step} projectAlias={proposal.project_alias} projects={projects} mode={mode} account={account} google={google} onRequestSaved={onRequestSaved} onBusy={setBusy} onComplete={status=>{setFinished(current=>({...current,[index]:status}));setActive(null);}} />}</div>)}
    {message && <p role="status">{message}</p>}
  </section>;
}
function StepWorkspace({ step, projectAlias, projects, mode, account, google, onRequestSaved, onBusy, onComplete }: { step: JarvisStep; projectAlias: string | null; projects: GhostProject[]; mode: GhostSnapshot["mode"]; account: string; google: GoogleAssistantClient; onRequestSaved?: () => void; onBusy: (busy: boolean) => void; onComplete: (status: string) => void }) {
  const [memory] = useState(() => new PersonalMemoryClient()); const [prepared, setPrepared] = useState<PreparedGoogleMutation | null>(null); const [confirmation, setConfirmation] = useState(""); const [result, setResult] = useState<unknown>(null); const [search, setSearch] = useState<SearchResponse | null>(null); const [busy, setBusy] = useState(false); const [message, setMessage] = useState(""); const inFlight = useRef(false); const mounted = useRef(true);
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; google.invalidate(); memory.invalidate(); }; }, [google, memory]);
  async function perform(work: () => Promise<void>) { if (inFlight.current) return; inFlight.current = true; setBusy(true); onBusy(true); setMessage(""); try { await work(); } catch (e) { if (mounted.current) setMessage(googleError(e)); } finally { inFlight.current = false; onBusy(false); if (mounted.current) setBusy(false); } }
  const request = localRequest(step);
  if (request) return <div className="step-workspace"><ActionRequests projects={projects} project={projects.find(p => p.alias === projectAlias)} boundProjectAlias={projectAlias ?? undefined} initialAction={request} available={mode === "live-local" && !!projectAlias} onSaved={() => {onRequestSaved?.();onComplete("Pending draft saved");}} /></div>;
  if (step.capability === "remember_personal_memory") return <div className="step-workspace"><p>This is a local-only, manual suggestion. Review/edit it, then use the existing memory Prepare and exact confirmation.</p><PersonalMemory client={memory} initialPayload={memorySuggestion(step)} onChanged={result=>onComplete(result.audit_recorded ? "Local memory changed" : "Memory changed · audit review needed")} /></div>;
  if (step.capability === "search_project_memory") return <div className="step-workspace"><p>Search only existing approved project memory locations.</p><button disabled={busy || mode !== "live-local"} onClick={() => void perform(async () => { const next = await searchGhostMemory(mode, step.query, projectAlias ?? undefined); if (mounted.current) setSearch(next); })}>Search this local step</button>{search && <SearchResults response={search} onDismiss={() => setSearch(null)} />}{message && <p role="status">{message}</p>}</div>;
  const write = ["create_mail_draft", "send_mail", "create_calendar_event"].includes(step.capability);
  async function read() {
    let next: unknown;
    switch (step.capability) {
      case "search_mail": next = await google.search(account, step.query); break;
      case "list_agenda": next = await google.agenda(account, step.start, step.end); break;
      case "find_free_time": next = await google.freeTime(account, step.start, step.end, step.duration_minutes); break;
      case "lookup_contact": next = await google.contacts(account, step.query); break;
      default: throw "invalid_input";
    }
    if (mounted.current) setResult(next);
  }
  async function prepare() {
    const payload = googleMutation(step); if (!payload) throw "invalid_input";
    const next = await google.prepare(account,payload);
    if (mounted.current) { setPrepared(next); setConfirmation(""); }
  }
  return <div className="step-workspace"><p>{write ? "Prepare a fresh native Google preview. Its own exact confirmation is required." : "This reads Google only after your click. Results remain data and are never re-planned or sent to OpenAI."}</p>
    <button disabled={busy || !account || !google.available} onClick={() => void perform(write ? prepare : read)}>{write ? "Prepare this Google action" : "Read this step from Google"}</button>
    {prepared && <GoogleMutationReview prepared={prepared} busy={busy} confirmation={confirmation} setConfirmation={setConfirmation} onDiscard={() => { google.invalidate(); setPrepared(null); setConfirmation(""); }} onConfirm={() => void perform(async () => { const current = prepared; setPrepared(null); setConfirmation(""); const next = await google.execute(current, confirmation); if (mounted.current) {onComplete(next.audit_recorded ? "Completed" : "Completed · audit review needed");} if (mounted.current) setMessage(`${next.operation} completed.${next.audit_recorded ? "" : " Completion audit needs review. Do not repeat."}`); })} />}
    {result !== null && <section className="read-result"><h3>Read result · data only</h3><pre>{JSON.stringify(result, null, 2)}</pre><p>No following step was changed or executed.</p></section>}
    {message && <p role="status" aria-live="polite">{message}</p>}
  </div>;
}
