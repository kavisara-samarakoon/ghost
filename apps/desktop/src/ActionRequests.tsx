import { useRef, useState } from "react";
import { actionTypes, providers, prepareActionRequest, saveActionRequest,
  type ActionType, type PreparedRequest, type RecentRequest, type RequestAction } from "./action-requests.ts";
import type { GhostProject } from "./ghost-snapshot.ts";

export function RequestReview({ request, busy, onSave, onEdit }: {
  request: PreparedRequest; busy: boolean; onSave: () => void; onEdit: () => void;
}) {
  return <section className="request-review" aria-labelledby="request-review-title">
    <h3 id="request-review-title">Review Action Request</h3>
    <strong>{request.preview_title}</strong>
    <pre>{request.preview_body}</pre>
    <p>{request.safety_notice}</p>
    <p>Save Request confirms this reviewed content as a local pending draft.</p>
    <div className="hero-buttons">
      <button type="button" className="btn-primary" disabled={busy} onClick={onSave}>Save Request</button>
      <button type="button" className="btn-secondary" disabled={busy} onClick={onEdit}>Edit Request</button>
    </div>
  </section>;
}

export default function ActionRequests({ projects, project, available, recent = [], onSaved }: {
  projects: GhostProject[]; project?: GhostProject; available: boolean;
  recent?: RecentRequest[]; onSaved?: () => void;
}) {
  const [actionType, setActionType] = useState<ActionType>("start_session");
  // Follow the loaded selection until the user explicitly chooses/types an alias.
  const [alias, setAlias] = useState<string | null>(null);
  const projectAlias = alias ?? project?.alias ?? "";
  const [text, setText] = useState("");
  const [provider, setProvider] = useState<typeof providers[number]>("codex");
  const [review, setReview] = useState<PreparedRequest | null>(null);
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState<string | null>(null);
  const [saved, setSaved] = useState<RecentRequest[]>([]);
  const inFlight = useRef(false);
  const needsText = actionType === "start_session" || actionType === "add_session_note";
  const pending = [...saved, ...recent.filter(item => !saved.some(local => local.id === item.id))].slice(0, 10);

  async function prepare() {
    if (inFlight.current || !available) return;
    inFlight.current = true; setBusy(true); setMessage(null);
    const action: RequestAction = actionType === "start_session" ? { action_type: actionType, payload: { goal: text.trim() } }
      : actionType === "add_session_note" ? { action_type: actionType, payload: { note: text.trim() } }
      : actionType === "create_handoff" ? { action_type: actionType, payload: { provider } }
      : { action_type: actionType, payload: {} };
    try { setReview(await prepareActionRequest(projectAlias.trim(), action)); }
    catch (error) { setMessage((error as Error).message); }
    finally { inFlight.current = false; setBusy(false); }
  }

  async function save() {
    if (!review || inFlight.current || !available) return;
    inFlight.current = true; setBusy(true);
    try {
      const result = await saveActionRequest(review, true);
      setSaved(items => [{ id: review.id, created_at: review.created_at, action_type: review.action_type,
        project_alias: review.project_alias, status: review.status }, ...items].slice(0, 10));
      setMessage(`Request saved locally: ${result.path}${result.audit_recorded ? "" : " — Audit could not be recorded. Keep this path; do not save a duplicate."}`);
      setReview(null); setText(""); onSaved?.();
    } catch (error) { setMessage((error as Error).message); }
    finally { inFlight.current = false; setBusy(false); }
  }

  return <section className="glass-panel action-requests" aria-labelledby="action-requests-title">
    <div className="command-panel-heading"><div><p className="page-eyebrow">Request-only desktop actions</p>
      <h2 id="action-requests-title">Action Requests</h2></div><span className="page-badge">Local pending drafts</span></div>
    <p>Prepare, review, and save a request. Sessions, notes, outputs, and drafts stay unchanged. Do not include secrets.</p>
    {!available && <p>Open the desktop app with local project metadata to prepare requests. Sample projects cannot save requests.</p>}
    {!review ? <form onSubmit={event => { event.preventDefault(); void prepare(); }}>
      <fieldset disabled={busy || !available}>
        <div className="request-fields">
          <label>Action type<select value={actionType} onChange={event => { setActionType(event.target.value as ActionType); setText(""); setMessage(null); }}>
            {actionTypes.map(type => <option key={type} value={type}>{type.replace(/_/g, " ")}</option>)}
          </select></label>
          <label>Project alias<input required maxLength={128} pattern="[a-z0-9-]{1,128}" list="request-projects" value={projectAlias} onChange={event => setAlias(event.target.value)} /></label>
          <datalist id="request-projects">{projects.map(item => <option key={item.alias} value={item.alias}>{item.name}</option>)}</datalist>
        </div>
        {needsText && <label>{actionType === "start_session" ? "Goal text" : "Note text"}
          <textarea required maxLength={8000} rows={4} value={text} onChange={event => setText(event.target.value)} /></label>}
        {actionType === "create_handoff" && <label>Provider<select value={provider} onChange={event => setProvider(event.target.value as typeof provider)}>
          {providers.map(value => <option key={value} value={value}>{value}</option>)}</select></label>}
        <button type="submit" className="btn-primary">Prepare Action</button>
      </fieldset>
    </form> : <RequestReview request={review} busy={busy} onSave={() => { void save(); }} onEdit={() => { setReview(null); setMessage(null); }} />}
    {busy && <p role="status">{review ? "Saving local request…" : "Preparing preview…"}</p>}
    {message && <p className="request-feedback" role="status">{message}</p>}
    <p>Run the matching CLI command manually after review. Requests are never executed by the desktop.</p>
    <h3>Recent pending requests</h3>
    {pending.length ? <ul className="request-list">{pending.map(item => <li key={item.id}>
      <strong>{item.action_type.replace(/_/g, " ")}</strong><span>{item.project_alias} · pending · {item.created_at}</span>
    </li>)}</ul> : <p>No recent pending requests loaded.</p>}
  </section>;
}
