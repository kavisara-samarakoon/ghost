import { useEffect, useRef, useState } from "react";
import { GoogleAssistantClient, permissionLabels, readPermissions, writePermissions, googleError, confirmationPhrase, eventChanges,
  type GooglePermission, type GoogleStatus, type GoogleAction, type PreparedGoogleMutation, type MailResult, type AgendaResult,
  type FreeTimeResult, type ContactResult, type CalendarEvent, type EventInput, type EventTime } from "./google-assistant.ts";
import "./GoogleAssistant.css";

function displayTime(value: EventTime) { return value.date_time ?? value.date ?? "Unavailable"; }
function EventPreview({ value }: { value: EventInput }) { return <dl><dt>Summary</dt><dd>{value.summary}</dd><dt>Start</dt><dd>{displayTime(value.start)}</dd><dt>End</dt><dd>{displayTime(value.end)}</dd><dt>Location</dt><dd>{value.location ?? "None"}</dd><dt>Description</dt><dd className="google-prose">{value.description ?? "None"}</dd></dl>; }
export default function GoogleAssistant({ client: supplied }: { client?: GoogleAssistantClient }) {
  const [client] = useState(() => supplied ?? new GoogleAssistantClient());
  const [status, setStatus] = useState<GoogleStatus | null>(null);
  const [clientId, setClientId] = useState(""); const [label, setLabel] = useState("Personal Google");
  const [permissions, setPermissions] = useState<GooglePermission[]>([...readPermissions]);
  const [connectReview, setConnectReview] = useState(false); const [accountId, setAccountId] = useState("");
  const [busy, setBusy] = useState(false); const [notice, setNotice] = useState("");
  const [query, setQuery] = useState(""); const [mail, setMail] = useState<MailResult | null>(null); const [digest, setDigest] = useState(false);
  const [to, setTo] = useState(""); const [cc, setCc] = useState(""); const [subject, setSubject] = useState(""); const [body, setBody] = useState("");
  const [windowStart, setWindowStart] = useState(() => new Date().toISOString());
  const [windowEnd, setWindowEnd] = useState(() => new Date(Date.now() + 7 * 86400000).toISOString());
  const [duration, setDuration] = useState(30); const [agenda, setAgenda] = useState<AgendaResult | null>(null); const [free, setFree] = useState<FreeTimeResult | null>(null);
  const [selectedEvent, setSelectedEvent] = useState<CalendarEvent | null>(null);
  const [summary, setSummary] = useState(""); const [description, setDescription] = useState(""); const [location, setLocation] = useState("");
  const [start, setStart] = useState(""); const [end, setEnd] = useState(""); const [allDay, setAllDay] = useState(false);
  const [contactQuery, setContactQuery] = useState(""); const [contacts, setContacts] = useState<ContactResult | null>(null);
  const [prepared, setPrepared] = useState<PreparedGoogleMutation | null>(null); const [confirmation, setConfirmation] = useState("");
  const [disconnectPhrase, setDisconnectPhrase] = useState(""); const mounted = useRef(true);
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; client.invalidate(); }; }, [client]);
  const available = client.available; const account = status?.accounts.find(a => a.account_id === accountId);
  const allows = (permission: GooglePermission) => account?.status === "connected" && account.granted_permissions.includes(permission);
  function invalidate() { client.invalidate(); setPrepared(null); setConfirmation(""); }
  async function perform(task: () => Promise<void>) {
    if (busy) return; setBusy(true); setNotice("");
    try { await task(); } catch (error) { if (mounted.current) setNotice(googleError(error)); }
    finally { if (mounted.current) setBusy(false); }
  }
  async function reload() { const next = await client.status(); if (mounted.current) { setStatus(next); setClientId(next.config.client_id ?? ""); setAccountId(id => next.accounts.some(a => a.account_id === id) ? id : next.accounts[0]?.account_id ?? ""); } }
  function select(id: string) { invalidate(); setAccountId(id); setMail(null); setAgenda(null); setFree(null); setContacts(null); setSelectedEvent(null); setTo(""); setCc(""); setSubject(""); setBody(""); setSummary(""); setDescription(""); setLocation(""); setStart(""); setEnd(""); setDisconnectPhrase(""); }
  async function prepare(payload: GoogleAction) { const next = await client.prepare(accountId, payload); if (mounted.current) { setPrepared(next); setConfirmation(""); requestAnimationFrame(() => document.getElementById("google-preview-title")?.scrollIntoView({ block: "start" })); } }
  function eventInput(): EventInput { return { summary, description: description || null, location: location || null,
    start: allDay ? { date: start } : { date_time: start }, end: allDay ? { date: end } : { date_time: end } }; }
  function editEvent(event: CalendarEvent) { invalidate(); setSelectedEvent(event); setSummary(event.fields.summary); setDescription(event.fields.description ?? ""); setLocation(event.fields.location ?? ""); setStart(displayTime(event.fields.start)); setEnd(displayTime(event.fields.end)); setAllDay(!!event.fields.start.date); }
  const mutationEnabled = available && !busy && !!account;
  return <section className="google-assistant" aria-labelledby="google-title">
    <header className="desktop-page-header"><div><p className="page-eyebrow">Explicit Google access</p><h1 id="google-title">Assistant</h1><p>Read Google data on request. Review and explicitly confirm every draft, send, and calendar change.</p></div></header>
    {!available && <p className="page-notice">Google integration is unavailable in browser preview. Use the macOS desktop app.</p>}
    <p className="page-notice">Gmail, Calendar and Contacts requests go to Google. Mail digests are computed locally; this area never sends their content to OpenAI.</p>
    {notice && <p role="status" className="page-notice">{notice}</p>}
    {status?.credential_checks_available === false && <p className="page-notice">Credential checks are unavailable. Local account records remain visible for recovery. Unlock Keychain or explicitly retry local disconnect.</p>}
    <fieldset disabled={!available || busy} className="glass-panel google-panel"><legend>Google connection</legend>
      <button type="button" onClick={() => void perform(reload)}>Load local setup and accounts</button>
      <label>Google Desktop OAuth client ID<input value={clientId} maxLength={256} autoComplete="off" spellCheck={false} onChange={e => { setClientId(e.target.value); setConnectReview(false); }} placeholder="Desktop client ID ending in .apps.googleusercontent.com" /></label>
      <button type="button" onClick={() => void perform(async () => { await client.saveClient(clientId); invalidate(); await reload(); setNotice("Client ID saved privately. No Google request was made."); })}>Save client ID locally</button>
      <label>Local account label<input value={label} maxLength={128} onChange={e => { setLabel(e.target.value); setConnectReview(false); }} /></label>
      {[readPermissions, writePermissions].map((group, index) => <div key={index}><h3>{index ? "Write capabilities — opt in" : "Read capabilities"}</h3>
        {group.map(permission => <label className="google-check" key={permission}><input type="checkbox" checked={permissions.includes(permission)} onChange={e => { setPermissions(old => e.target.checked ? [...old, permission] : old.filter(p => p !== permission)); setConnectReview(false); }} />{permissionLabels[permission]}</label>)}</div>)}
      <button type="button" disabled={!status?.config.configured || !permissions.length} onClick={() => setConnectReview(true)}>Review Google permissions</button>
      {connectReview && <div className="google-review"><h3>Connect {label}</h3><p>Only these GHOST permissions will be requested:</p><ul>{permissions.map(p => <li key={p}>{permissionLabels[p]}</li>)}</ul><p>This opens Google consent in your browser. The connection can take up to five minutes.</p>
        <button type="button" onClick={() => void perform(async () => { const result = await client.connect(label, [...permissions], true); if (!mounted.current) return; setConnectReview(false); await reload(); select(result.account.account_id); setNotice(result.audit_recorded ? "Google account connected." : "Account connected; completion audit needs review."); })}>Open Google consent</button></div>}
      <label>Account<select disabled={!status?.accounts.length} value={accountId} onChange={e => select(e.target.value)}><option value="">Select an account</option>{status?.accounts.map(a => <option key={a.account_id} value={a.account_id}>{a.display_label} · {a.status} · {a.account_id.slice(0, 8)}</option>)}</select></label>
      {account && <><p>{account.status} · {account.granted_permissions.map(p => permissionLabels[p]).join(", ")}</p><details><summary>Disconnect this local account</summary><p>The Google grant may remain active until revoked in Google Account settings.</p><label>Type DISCONNECT {accountId}<input value={disconnectPhrase} onChange={e => setDisconnectPhrase(e.target.value)} autoComplete="off" /></label><button type="button" disabled={disconnectPhrase !== `DISCONNECT ${accountId}`} onClick={() => void perform(async () => { const result = await client.disconnect(accountId, disconnectPhrase); select(""); await reload(); setNotice(result.notice); })}>Remove local credentials and metadata</button></details></>}
    </fieldset>
    <fieldset disabled={!available || busy || !account} className="glass-panel google-panel"><legend>Mail</legend>
      <p>This reads Gmail metadata and snippets from Google. Empty search uses recent inbox messages.</p><label>Gmail query<input value={query} maxLength={512} onChange={e => setQuery(e.target.value)} placeholder="in:inbox newer_than:14d" /></label>
      <button type="button" disabled={!allows("mail_read")} onClick={() => void perform(async () => { const result = await client.search(accountId, query); if (mounted.current) setMail(result); })}>Read/search Gmail</button>
      {mail && <><p>{mail.messages.length} results · {mail.unread_in_results} unread in these results{mail.truncated ? " · More results exist" : ""}</p><button type="button" onClick={() => setDigest(!digest)}>{digest ? "Hide" : "Show"} local digest</button>{digest && <pre className="google-prose">{mail.digest}</pre>}
        {mail.messages.map(m => <article className="google-result" key={m.message_id}><strong>{m.subject || "No subject"}</strong><p>{m.from} · {m.date}{m.unread ? " · Unread" : ""}</p><p>{m.snippet}</p></article>)}</>}
      <h3>Plain-text composition</h3><p>Maximum five recipients, no attachments, HTML or Bcc.</p>
      <label>To (comma separated)<input value={to} onChange={e => { setTo(e.target.value); invalidate(); }} /></label><label>Cc (optional)<input value={cc} onChange={e => { setCc(e.target.value); invalidate(); }} /></label>
      <label>Subject<input value={subject} maxLength={256} onChange={e => { setSubject(e.target.value); invalidate(); }} /></label><label>Complete plain-text body<textarea value={body} maxLength={32768} rows={6} onChange={e => { setBody(e.target.value); invalidate(); }} /></label>
      <div className="google-actions">{(["create_mail_draft", "send_mail"] as const).map(action => <button type="button" key={action} disabled={!mutationEnabled || !allows(action === "send_mail" ? "mail_send" : "mail_draft")} onClick={() => void perform(() => prepare({ action, mail: { to: to.split(",").map(s => s.trim()).filter(Boolean), cc: cc.split(",").map(s => s.trim()).filter(Boolean), subject, body } }))}>{action === "send_mail" ? "Prepare Send preview" : "Prepare Save Draft preview"}</button>)}</div>
    </fieldset>
    <fieldset disabled={!available || busy || !account} className="glass-panel google-panel"><legend>Primary calendar</legend>
      <p>This reads agenda/free-busy from Google. Times must be RFC3339 with an explicit offset; all-day event ends are exclusive.</p>
      <label>Window start<input value={windowStart} onChange={e => setWindowStart(e.target.value)} /></label><label>Window end<input value={windowEnd} onChange={e => setWindowEnd(e.target.value)} /></label>
      <button type="button" disabled={!allows("calendar_read")} onClick={() => void perform(async () => { const result = await client.agenda(accountId, windowStart, windowEnd); if (mounted.current) setAgenda(result); })}>Read agenda</button>
      <label>Desired free slot (5–480 minutes)<input type="number" min={5} max={480} value={duration} onChange={e => setDuration(Number(e.target.value))} /></label><button type="button" disabled={!allows("calendar_read")} onClick={() => void perform(async () => { const result = await client.freeTime(accountId, windowStart, windowEnd, duration); if (mounted.current) setFree(result); })}>Find free time</button>
      {free && <div><h3>Free slot candidates</h3>{free.candidates.map((f, i) => <p key={i}>{f.start} → {f.end}</p>)}<details><summary>Busy and free intervals</summary><h4>Busy</h4>{free.busy.map((f, i) => <p key={i}>{f.start} → {f.end}</p>)}<h4>Free</h4>{free.free.map((f, i) => <p key={i}>{f.start} → {f.end}</p>)}</details></div>}
      {agenda && <>{agenda.truncated && <p>More events exist beyond this bounded result.</p>}{agenda.events.map(e => <article className="google-result" key={e.event_id}><strong>{e.fields.summary || "Untitled"}</strong><p>{displayTime(e.fields.start)} → {displayTime(e.fields.end)}</p><p>{e.fields.location}</p><p>{e.fields.description}</p><button type="button" disabled={!allows("calendar_event_update")} onClick={() => editEvent(e)}>Select for update</button></article>)}</>}
      <h3>{selectedEvent ? "Update selected event" : "Create event"}</h3><p>Only the primary calendar. No attendees, recurrence, conferencing or deletion.</p>
      {selectedEvent && <><p>Only changed fields will be patched. Prepare fetches the current event; its ETag must still match. Agenda descriptions are previews: leave unchanged or explicitly replace.</p><button type="button" onClick={() => { setSelectedEvent(null); invalidate(); }}>Switch to event creation</button></>}
      <label>Summary<input value={summary} maxLength={256} onChange={e => { setSummary(e.target.value); invalidate(); }} /></label><label>Location<input value={location} maxLength={512} onChange={e => { setLocation(e.target.value); invalidate(); }} /></label><label>Description<textarea value={description} maxLength={4096} rows={4} onChange={e => { setDescription(e.target.value); invalidate(); }} /></label>
      <label className="google-check"><input type="checkbox" checked={allDay} onChange={e => { setAllDay(e.target.checked); setStart(""); setEnd(""); invalidate(); }} />All day (YYYY-MM-DD)</label>
      <label>Start<input value={start} onChange={e => { setStart(e.target.value); invalidate(); }} placeholder={allDay ? "YYYY-MM-DD" : "YYYY-MM-DDTHH:mm:ss+05:30"} /></label><label>End<input value={end} onChange={e => { setEnd(e.target.value); invalidate(); }} placeholder={allDay ? "YYYY-MM-DD (exclusive)" : "YYYY-MM-DDTHH:mm:ss+05:30"} /></label>
      <button type="button" disabled={!mutationEnabled || !allows(selectedEvent ? "calendar_event_update" : "calendar_event_create")} onClick={() => void perform(() => prepare(selectedEvent ? { action: "update_calendar_event", event_id: selectedEvent.event_id, etag: selectedEvent.etag, changes: eventChanges(selectedEvent.fields, eventInput()) } : { action: "create_calendar_event", event: eventInput() }))}>Prepare {selectedEvent ? "Update" : "Create"} Event preview</button>
    </fieldset>
    <fieldset disabled={!available || busy || !account} className="glass-panel google-panel"><legend>Contacts — read only</legend><p>This reads at most 100 Google contacts, with up to five emails/phones each. Search runs locally on that bounded result.</p><label>Name, email or phone substring<input value={contactQuery} maxLength={128} onChange={e => setContactQuery(e.target.value)} /></label><button type="button" disabled={!allows("contacts_read")} onClick={() => void perform(async () => { const result = await client.contacts(accountId, contactQuery); if (mounted.current) setContacts(result); })}>Read/lookup contacts</button>
      {contacts && <>{contacts.truncated && <p>More contacts exist and are outside this bounded lookup.</p>}{contacts.contacts.map(c => <article className="google-result" key={c.resource_name}><strong>{c.display_name || "Unnamed contact"}</strong><p>{c.emails.join(", ")}</p><p>{c.phones.join(", ")}</p><p>{c.organization}</p></article>)}</>}
    </fieldset>
    {prepared && <section className="glass-panel google-panel google-review" aria-labelledby="google-preview-title"><h2 id="google-preview-title">Exact Google mutation preview</h2><p>Account: {prepared.preview.account_label} · {prepared.account_id}</p>
      <p>{prepared.payload.action === "send_mail" ? "This sends this email now." : prepared.payload.action === "create_mail_draft" ? "This creates a Gmail draft. It will not send." : "This changes an event on your primary calendar."}</p>
      {prepared.preview.mail && <dl><dt>Sender</dt><dd>{prepared.preview.sender ?? "Authenticated Google mailbox (default sender)"}</dd><dt>Date</dt><dd>{new Date(prepared.created_at * 1000).toISOString()}</dd><dt>To</dt><dd>{prepared.preview.mail.to.join(", ")}</dd><dt>Cc</dt><dd>{prepared.preview.mail.cc.join(", ") || "None"}</dd><dt>Subject</dt><dd>{prepared.preview.mail.subject}</dd><dt>Complete body ({prepared.preview.body_bytes} UTF-8 bytes)</dt><dd><pre className="google-prose">{prepared.preview.mail.body}</pre></dd></dl>}
      {prepared.preview.old_event && <><h3>Old values</h3><EventPreview value={prepared.preview.old_event} /></>}{prepared.preview.new_event && <><h3>New values</h3><EventPreview value={prepared.preview.new_event} /></>}
      <p className="google-prose">Digest: {prepared.request_sha256}</p><p>Expires: {new Date(prepared.expires_at * 1000).toISOString()}</p>
      <label>Type exactly: {confirmationPhrase(prepared)}<input value={confirmation} onChange={e => setConfirmation(e.target.value)} autoComplete="off" spellCheck={false} disabled={busy} /></label>
      <button type="button" disabled={busy || confirmation !== confirmationPhrase(prepared)} onClick={() => void perform(async () => { const request = prepared; setPrepared(null); setConfirmation(""); const result = await client.execute(request, confirmation); if (mounted.current) setNotice(`${result.operation} completed (${result.provider_id}).${result.audit_recorded ? "" : " Completion audit needs review; do not repeat the action."}`); })}>Confirm one Google action</button>
      <button type="button" disabled={busy} onClick={invalidate}>Discard preview locally</button>
    </section>}
  </section>;
}
