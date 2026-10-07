import { useEffect, useRef, useState } from "react";
import { PersonalMemoryClient, blankMemory, privacyChange, memoryKinds, memoryPhrase, memoryError, type MemoryPayload, type MemoryRecord, type MemoryHit, type MemoryPreview, type MemoryAction } from "./personal-memory.ts";
export function MemoryPayloadView({ record }: { record: MemoryRecord }) {
  const p = record.payload;
  return <dl className="personal-preview"><dt>Memory ID / status</dt><dd>{record.memory_id} · {record.status}</dd><dt>Kind</dt><dd>{p.kind}</dd><dt>Title</dt><dd>{p.title}</dd><dt>Complete content</dt><dd className="memory-complete">{p.content}</dd><dt>Tags</dt><dd>{p.tags.join(", ") || "None"}</dd><dt>Privacy</dt><dd>{p.sensitivity} · {p.sharing}</dd><dt>Expiration</dt><dd>{p.expires_at ?? "None"}</dd><dt>Source</dt><dd>{p.source.kind === "manual" ? "Manual" : `${p.source.project_alias} · ${p.source.relative_path}`}</dd><dt>Created / updated</dt><dd>{record.created_at} / {record.updated_at}</dd></dl>;
}
export default function PersonalMemory({ client }: { client: PersonalMemoryClient }) {
  const [form, setForm] = useState<MemoryPayload>(blankMemory); const [tags, setTags] = useState("");
  const [records, setRecords] = useState<MemoryHit[]>([]); const [selected, setSelected] = useState<MemoryRecord | null>(null);
  const [offset,setOffset] = useState(0); const [reviewing,setReviewing] = useState(false);
  const [filter, setFilter] = useState("active"); const [kind, setKind] = useState<MemoryPayload["kind"] | "">(""); const [query, setQuery] = useState("");
  const [preview, setPreview] = useState<MemoryPreview | null>(null); const [confirmation, setConfirmation] = useState("");
  const [pending, setPending] = useState(false); const [feedback, setFeedback] = useState(""); const busy = useRef(false); const mounted = useRef(true);
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; client.invalidate(); }; }, [client]);
  function invalidate() { client.invalidate(); setPreview(null); setConfirmation(""); }
  function edit(p: Partial<MemoryPayload>) { invalidate(); setForm(f => ({ ...f, ...p })); }
  async function run(work: () => Promise<void>) { if (busy.current) return; busy.current = true; setPending(true); setFeedback(""); try { await work(); } catch (e) { if (mounted.current) setFeedback(memoryError(e)); } finally { busy.current = false; if (mounted.current) setPending(false); } }
  async function prepare(action: MemoryAction) {
    invalidate(); const p = await client.prepare({ action, memory_id: action === "create_memory" ? null : selected?.memory_id ?? null, payload: action === "create_memory" || action === "update_memory" ? { ...form, tags: tags.split(",").map(t => t.trim()).filter(Boolean) } : null });
    if (mounted.current) setPreview(p);
  }
  const unavailable = !client.available();
  return <section className="personal-memory">
    <h2>Personal memory</h2><p>Stored only in your private GHOST_HOME as local plaintext private data. Credentials are prohibited.</p>
    <p>Sensitive memories are local-only. GHOST does not send personal memory to OpenAI in M42. Nothing is captured automatically.</p>
    {unavailable && <p role="status">Personal memory is unavailable in browser preview.</p>}
    <fieldset disabled={pending || unavailable}><legend>Review stored memories</legend>
      <label>Review filter<select value={filter} onChange={e => { invalidate(); setFilter(e.target.value); setOffset(0); setReviewing(false); setRecords([]); }}>{["active", "archived", "expired", "all"].map(f => <option key={f}>{f}</option>)}</select></label>
      <label>Kind filter<select value={kind} onChange={e => { setKind(e.target.value as typeof kind);setOffset(0);setReviewing(false); setRecords([]); }}><option value="">All kinds</option>{memoryKinds.map(k => <option key={k}>{k}</option>)}</select></label>
      <button onClick={() => void run(async () => { invalidate(); const r = await client.list(filter, kind || null); if (mounted.current) {setRecords(r);setOffset(0);setReviewing(true);} })}>Review memories</button>
      <label>Search personal memory<input value={query} maxLength={120} onChange={e => setQuery(e.target.value)} /></label>
      <button onClick={() => void run(async () => { invalidate(); const r = await client.search(query); if (mounted.current) {setRecords(r);setReviewing(false);} })}>Search personal</button>
    </fieldset>
    {reviewing && <div className="artifact-actions"><button disabled={pending || offset === 0} onClick={() => void run(async () => {const next = Math.max(0,offset-20);const r=await client.list(filter,kind||null,next);if(mounted.current){setRecords(r);setOffset(next);}})}>Previous memories</button><span>Review offset {offset}</span><button disabled={pending || records.length < 20 || offset >= 500} onClick={() => void run(async () => {const next=offset+20;const r=await client.list(filter,kind||null,next);if(mounted.current){setRecords(r);setOffset(next);}})}>Next memories</button></div>}
    <p>Review returns at most 20 records per page, most recently updated first. Search includes active, unexpired memories only.</p>
    <ul className="memory-result-list">{records.map(r => <li key={r.memory_id}><h3>{r.title}</h3><span>{r.kind} · {r.sensitivity} · {r.sharing} · {r.status}{r.expired ? " · expired" : ""}</span><p>{r.excerpt}</p><button disabled={pending || unavailable} onClick={() => void run(async () => { invalidate(); const record = await client.get(r.memory_id); if (mounted.current) { setSelected(record); setForm(record.payload); setTags(record.payload.tags.join(", ")); } })}>Open full memory</button></li>)}</ul>
    {selected && <section><h3>Review full record</h3><MemoryPayloadView record={selected} /></section>}
    <fieldset disabled={pending || unavailable}><legend>{selected ? "Edit selected memory" : "Create memory"}</legend>
      <button onClick={() => { invalidate(); setSelected(null); setForm(blankMemory()); setTags(""); }}>New memory</button>
      <label>Kind<select value={form.kind} onChange={e => edit({ kind: e.target.value as MemoryPayload["kind"] })}>{memoryKinds.map(k => <option key={k}>{k}</option>)}</select></label>
      <label>Title<input maxLength={160} value={form.title} onChange={e => edit({ title: e.target.value })} /></label>
      <label>Content<textarea rows={6} maxLength={8192} value={form.content} onChange={e => edit({ content: e.target.value })} /></label>
      <label>Tags (comma separated, up to 12)<input value={tags} maxLength={400} onChange={e => { invalidate(); setTags(e.target.value); }} /></label>
      <label>Sensitivity<select value={form.sensitivity} onChange={e => { invalidate(); setForm(privacyChange(form, e.target.value as MemoryPayload["sensitivity"])); }}><option value="standard">standard</option><option value="sensitive">sensitive</option></select></label>
      <label>Sharing<select value={form.sharing} disabled={form.sensitivity === "sensitive"} onChange={e => edit({ sharing: e.target.value as MemoryPayload["sharing"] })}><option value="local_only">local_only</option><option value="provider_allowed">provider_allowed</option></select></label>
      <p>Provider allowed is a future-use policy label; this version does not send it.</p>
      <label>Expiration (optional RFC3339)<input value={form.expires_at ?? ""} maxLength={40} placeholder="2027-01-01T00:00:00Z" onChange={e => edit({ expires_at: e.target.value || null })} /></label>
      <label>Source<select value={form.source.kind} onChange={e => edit({ source: e.target.value === "manual" ? { kind: "manual" } : { kind: "project_reference", project_alias: "", relative_path: "" } })}><option value="manual">manual</option><option value="project_reference">project_reference</option></select></label>
      {form.source.kind === "project_reference" && <>
        <label>Registered project alias<input value={form.source.project_alias} maxLength={128} onChange={e => edit({ source: { ...(form.source as Extract<MemoryPayload["source"], { kind: "project_reference" }>), project_alias: e.target.value } })} /></label>
        <label>Approved relative GHOST memory path<input value={form.source.relative_path} maxLength={512} onChange={e => edit({ source: { ...(form.source as Extract<MemoryPayload["source"], { kind: "project_reference" }>), relative_path: e.target.value } })} /></label>
      </>}
      <div className="artifact-actions"><button onClick={() => void run(() => prepare(selected ? "update_memory" : "create_memory"))}>Prepare {selected ? "update" : "save"}</button>{selected && <><button onClick={() => void run(() => prepare("archive_memory"))}>Prepare archive</button><button onClick={() => void run(() => prepare("delete_memory"))}>Prepare delete / forget</button></>}</div>
    </fieldset>
    {preview && <section className="glass-panel memory-review"><h3>Review {preview.action.replace(/_/g, " ")}</h3><p>Preparation is inert. This preview expires in five minutes.</p>
      {preview.before && <><h4>Current record</h4><MemoryPayloadView record={preview.before} /></>}{preview.after && <><h4>Exact record to store</h4><MemoryPayloadView record={preview.after} /></>}
      {preview.action === "delete_memory" && <p>Removed from GHOST local memory after confirmation. Deleting is not a cryptographic disk erase; backups or filesystem snapshots may retain bytes.</p>}
      <p>Expires: {new Date(preview.expires_at * 1000).toISOString()}</p><code>{memoryPhrase(preview)}</code>
      <label>Exact confirmation<input value={confirmation} disabled={pending} autoComplete="off" spellCheck={false} onChange={e => setConfirmation(e.target.value)} /></label>
      <button disabled={pending || confirmation !== memoryPhrase(preview)} onClick={() => void run(async () => { const p = preview; const phrase = confirmation; setPreview(null); setConfirmation(""); const result = await client.execute(p, phrase); if (mounted.current) { setSelected(null); setRecords([]); setForm(blankMemory()); setTags(""); setFeedback(result.audit_recorded ? (p.action === "delete_memory" ? "Removed from GHOST local memory." : "Local memory changed.") : "Local memory changed, but completion audit needs review. Do not repeat the mutation."); } })}>Confirm local {preview.action.replace(/_/g, " ")}</button>
    </section>}
    <p role="status" aria-live="polite">{feedback}</p>
  </section>;
}
