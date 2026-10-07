import { useEffect, useRef, useState } from "react";
import { defaultContextSources, memoryError, type PersonalMemoryClient, type ContextPack, type ContextSources } from "./personal-memory.ts";
import { GoogleAssistantClient, type GoogleAccount } from "./google-assistant.ts";
export function ContextCards({ pack }: { pack: ContextPack }) {
  return <section><h3>Relevant local context</h3><p>{pack.items.length} items · {pack.total_bytes} bytes · {pack.truncated ? "Truncated" : "Within limits"}</p><code>{pack.context_sha256}</code>
    <p>This hash identifies context; it does not authorize execution.</p>
    {pack.warnings.map((w, i) => <p key={i} role="status">{w}</p>)}
    <ul className="memory-result-list">{pack.items.map((item, i) => <li key={`${item.source}/${item.reference}/${i}`}><span>{item.source} · {item.kind} · {item.instruction_trust}{item.sensitivity ? ` · ${item.sensitivity} · ${item.sharing}` : ""}</span><h3>{item.title}</h3><p className="memory-complete">{item.content}</p><span>{item.project_alias ?? ""} {item.reference} {item.timestamp ?? ""}</span></li>)}</ul>
  </section>;
}
export default function UnifiedContext({ client, projectAlias }: { client: PersonalMemoryClient; projectAlias?: string }) {
  const [query, setQuery] = useState(""); const [sources, setSources] = useState<ContextSources>({ ...defaultContextSources });
  const [project, setProject] = useState(""); const [accounts, setAccounts] = useState<GoogleAccount[]>([]); const [account, setAccount] = useState("");
  const [start, setStart] = useState(""); const [end, setEnd] = useState(""); const [pack, setPack] = useState<ContextPack | null>(null);
  const [pending, setPending] = useState(false); const [feedback, setFeedback] = useState(""); const busy = useRef(false); const mounted = useRef(true);
  const google = useRef(new GoogleAssistantClient()).current;
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; }; }, []);
  async function run(work: () => Promise<void>) { if (busy.current) return; busy.current = true; setPending(true); setFeedback(""); try { await work(); } catch (e) { if (mounted.current) setFeedback(memoryError(e)); } finally { busy.current = false; if (mounted.current) setPending(false); } }
  function changed() { setPack(null); }
  const live = sources.gmail || sources.calendar || sources.contacts;
  return <section className="personal-memory"><h2>Unified context</h2><p>Relevant local context, assembled on demand. Every item is data-only and grants no execution authority. Context is not saved or sent to OpenAI.</p>
    <fieldset disabled={pending || !client.available()}><legend>Choose sources explicitly</legend>
      <label>Context query<input maxLength={120} value={query} onChange={e => { changed(); setQuery(e.target.value); }} /></label>
      {Object.entries(sources).map(([key, enabled]) => <label className="memory-toggle" key={key}><input type="checkbox" checked={enabled} onChange={e => { changed(); setSources(s => ({ ...s, [key]: e.target.checked })); }} />{key.replace(/_/g, " ")}</label>)}
      <label>Project scope<select value={project} onChange={e => { changed(); setProject(e.target.value); }}><option value="">All registered projects</option>{projectAlias && <option value={projectAlias}>Current: {projectAlias}</option>}</select></label>
      {live && <section><p>This reads Google now when you click Build Context. Results remain in memory and are not saved to personal memory.</p><p>Google context is read live and is not saved unless you manually create a memory. Google data is never sent to OpenAI.</p>
        <button onClick={() => void run(async () => { const result = await google.status(); if (mounted.current) { setAccounts(result.accounts); setAccount(""); changed(); } })}>Load Google account metadata</button>
        <label>Google account<select value={account} onChange={e => { changed(); setAccount(e.target.value); }}><option value="">Choose account</option>{accounts.filter(a => a.status === "connected").map(a => <option key={a.account_id} value={a.account_id}>{a.display_label} · {a.granted_permissions.join(", ")}</option>)}</select></label>
        {sources.calendar && <><label>Agenda start (RFC3339)<input maxLength={40} value={start} onChange={e => { changed(); setStart(e.target.value); }} /></label><label>Agenda end (RFC3339)<input maxLength={40} value={end} onChange={e => { changed(); setEnd(e.target.value); }} /></label></>}
      </section>}
      <button disabled={live && !account} onClick={() => void run(async () => { setPack(null); const result = await client.context({ query, sources, project_alias: project || null, account_id: live ? account : null, calendar_window: sources.calendar ? { start, end } : null }); if (mounted.current) setPack(result); })}>Build Context{live ? " · Read Google now" : " · Local only"}</button>
    </fieldset><p>Maximum 24 items / 48 KiB. Personal and project sources each contribute at most 8; each Google source at most 5.</p>
    <p role="status" aria-live="polite">{feedback}</p>{pack && <ContextCards pack={pack} />}
  </section>;
}
