import { useEffect, useRef, useState, type RefObject } from "react";
import { JarvisClient, noSharing, jarvisError, type JarvisReview, type JarvisProposal } from "./jarvis.ts";
import JarvisPlan from "./JarvisPlan.tsx";
import VoiceInput from "../VoiceInput.tsx";
import type { CommandPrefill } from "../automations.ts";
import type { GhostProject, GhostSnapshot } from "../ghost-snapshot.ts";
export function OutboundReview({ review, busy, onSend, onDismiss }: { review: JarvisReview; busy: boolean; onSend: () => void; onDismiss: () => void }) {
  return <section className="review-panel" aria-label="Provider review"><div className="section-heading"><h2>Review what leaves your Mac</h2><span className="badge">OpenAI · {review.outbound_bytes.toLocaleString()} bytes</span></div>
    <p>This is a planning request. It cannot authorize actions.</p><dl className="detail-grid"><dt>Model</dt><dd>{review.model}</dd><dt>Local project binding</dt><dd>{review.project_alias ?? "None"} · stays local unless included in explicitly shared project context</dd></dl>
    <h3>Exact command</h3><pre>{review.command}</pre>
    <h3>Shared context ({review.context.length})</h3>{review.context.length === 0 && <p>No memory or project context will be shared.</p>}
    {review.context.map((item, index) => <article className="context-row" key={index}><span className="badge">{item.source} · {item.kind}{item.project_alias ? ` · ${item.project_alias}` : ""} · data only</span><h4>{item.title}</h4><pre>{item.content}</pre></article>)}
    <details><summary>Exact outbound input and integrity details</summary><pre>{review.outbound_input}</pre><p>SHA-256 <code>{review.request_sha256}</code></p><p>Expires {new Date(review.expires_at * 1000).toISOString()}</p></details>
    <p>{review.safety_notice}</p><div className="button-row"><button className="btn-primary" disabled={busy} onClick={onSend}>Send reviewed request to OpenAI</button><button disabled={busy} onClick={onDismiss}>Discard review</button></div>
  </section>;
}
export default function JarvisCommand({ projectAlias, projects, mode, inputRef, client: supplied, onRequestSaved, prefill, onPrefillConsumed }: { projectAlias: string | null; projects: GhostProject[]; mode: GhostSnapshot["mode"]; inputRef?: RefObject<HTMLTextAreaElement | null>; client?: JarvisClient; onRequestSaved?: () => void; prefill?: CommandPrefill | null; onPrefillConsumed?: () => void }) {
  const [client] = useState(() => supplied ?? new JarvisClient()); const [command, setCommand] = useState(""); const [sharing, setSharing] = useState({ ...noSharing }); const [query, setQuery] = useState("");
  const [review, setReview] = useState<JarvisReview | null>(null); const [proposal, setProposal] = useState<JarvisProposal | null>(null);
  const [busy, setBusy] = useState(false); const [sending,setSending] = useState(false); const [message, setMessage] = useState(""); const inFlight = useRef(false); const mounted = useRef(true);
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; client.invalidate(); }; }, [client]);
  useEffect(() => { client.invalidate(); setReview(null); }, [client, projectAlias]);
  useEffect(() => {
    if (!prefill || !client.available) return;
    client.invalidate(); setCommand(prefill.text); setSharing({ ...noSharing }); setQuery(""); setReview(null); setProposal(null); setMessage("");
    onPrefillConsumed?.();
  }, [client, prefill, onPrefillConsumed]);
  function invalidate() { client.invalidate(); setReview(null); setProposal(null); setMessage(""); }
  async function perform(work: () => Promise<void>) { if (inFlight.current) return; inFlight.current = true; setBusy(true); setMessage(""); try { await work(); } catch (error) { if (mounted.current) setMessage(jarvisError(error)); } finally { inFlight.current = false; if (mounted.current) setBusy(false); } }
  return <section className="jarvis-workspace" onKeyDown={event => { if (event.key === "Escape" && !busy && review) { client.invalidate(); setReview(null); event.stopPropagation(); } }}>
    <form onSubmit={event => { event.preventDefault(); void perform(async () => { const next = await client.prepare({ command, project_alias: projectAlias, context_query: sharing.personal_memory || sharing.project_memory ? query : null, sharing }); if (mounted.current) { setProposal(null); setReview(next); } }); }}>
      <label className="ask-label" htmlFor="ask-ghost">Ask GHOST</label><textarea id="ask-ghost" ref={inputRef} autoComplete="off" spellCheck={false} placeholder="What would you like to work on?" value={command} rows={3} maxLength={8192} disabled={busy || !client.available} onChange={event => { invalidate(); setCommand(event.target.value); }} />
      <div className="command-sharing"><span>AI context · off by default</span><label><input type="checkbox" checked={sharing.personal_memory} disabled={busy || !client.available} onChange={e => { invalidate(); setSharing(s => ({ ...s, personal_memory: e.target.checked })); }} />Share approved personal memory</label><label><input type="checkbox" checked={sharing.project_memory} disabled={busy || !client.available} onChange={e => { invalidate(); setSharing(s => ({ ...s, project_memory: e.target.checked })); }} />Share reviewed project context</label></div>
      {sharing.personal_memory && <p className="sharing-policy">Only standard, active, unexpired provider-allowed personal memories are eligible.</p>}
      {(sharing.personal_memory || sharing.project_memory) && <label>Context search query<input value={query} maxLength={120} disabled={busy} onChange={e => { invalidate(); setQuery(e.target.value); }} placeholder="e.g. meeting preference, API decision" /></label>}
      <div className="command-submit"><p>Planner only. Google data is never shared with OpenAI.</p><button type="submit" className="btn-primary" disabled={busy || !client.available || !command.trim()}>Prepare request</button></div>
    </form>
    {!client.available && <p className="notice">Static preview. Planning and privileged actions require the native desktop app.</p>}
    {busy && <p role="status">{sending ? "Sending the reviewed request to OpenAI…" : "Preparing local review…"}</p>}
    {review && <OutboundReview review={review} busy={busy} onDismiss={() => { client.invalidate(); setReview(null); }} onSend={() => void perform(async () => { const current = review; setReview(null); setSending(true); try { const result = await client.send(current); if (mounted.current) { setProposal(result); if (!result.audit_recorded) setMessage("Proposal received; completion audit needs review. No resend occurred."); } } finally { if(mounted.current)setSending(false); } })} />}
    {proposal && <JarvisPlan key={proposal.proposal_sha256} proposal={proposal} projects={projects} currentProject={projectAlias} mode={mode} onRequestSaved={onRequestSaved} />}
    {message && <p className="notice" role="status" aria-live="polite">{message}</p>}
    <details className="voice-disclosure"><summary>Use reviewed voice transcription</summary><VoiceInput onUseTranscript={text => { if (!inFlight.current) { invalidate(); setCommand(text); } }} /></details>
  </section>;
}
