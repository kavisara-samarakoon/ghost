import { useState } from "react";
import { PersonalMemoryClient, memoryError } from "./personal-memory.ts";
import type { GhostProject, GhostSnapshot } from "./ghost-snapshot.ts";
import type { RecentRequest } from "./action-requests.ts";
import type { GoogleStatus } from "./google-assistant.ts";
export default function TodayPage({ project, mode, recent = [], connections }: { project?: GhostProject; mode: GhostSnapshot["mode"]; recent?: RecentRequest[]; connections?: GoogleStatus | null }) {
  const [memory] = useState(() => new PersonalMemoryClient()); const [memoryCount, setMemoryCount] = useState<number | null>(null); const [message, setMessage] = useState(""); const [busy, setBusy] = useState(false);
  const session = project?.active_session; const artifact = project?.recent_artifacts[0];
  return <section className="today-page"><header className="desktop-page-header"><div><p className="page-eyebrow">Local overview</p><h1>Today</h1><p>Your current work, without background provider reads.</p></div><span className="badge">{mode === "live-local" ? "Local snapshot" : "Sample overview"}</span></header>
    <div className="today-grid"><section className="surface"><span className="page-eyebrow">Current project</span><h2>{project?.name ?? "No project selected"}</h2><p>{project?.status_preview ?? "No recorded status"}</p><span>{project?.alias}</span></section>
      <section className="surface"><span className="page-eyebrow">Active session</span><h2>{session?.goal_preview ?? "No active session"}</h2><p>{session?.note_preview ?? "No session notes loaded"}</p><span>{session?.started_at}</span></section>
      <section className="surface"><span className="page-eyebrow">Latest loaded artifact</span><h2>{artifact?.title ?? "No artifact loaded"}</h2><p>{artifact?.preview ?? "Review saved work in Artifacts"}</p></section>
      <section className="surface"><span className="page-eyebrow">Private memory / connections</span><h2>{memoryCount === null ? "Memory status not checked" : `${memoryCount} personal memories`}</h2><button disabled={busy || !memory.available()} onClick={async () => { if (busy) return; setBusy(true); try { const status = await memory.status(); setMemoryCount(status.record_count); } catch (e) { setMessage(memoryError(e)); } finally { setBusy(false); } }}>Check local memory status</button><p>{connections ? `${connections.accounts.filter(a => a.status === "connected").length} connected Google accounts in the last explicit check` : "Connection status not checked. Open Connections to review."}</p></section>
    </div><section className="surface"><h2>Recent pending workflow requests</h2><p>Local drafts only. Nothing is executed from this overview.</p>{recent.length ? <ul className="request-list">{recent.slice(0, 10).map(r => <li key={r.id}><strong>{r.action_type.replace(/_/g, " ")}</strong><span>{r.project_alias} · pending · {r.created_at}</span></li>)}</ul> : <p>No recent pending requests loaded.</p>}</section>
    {message && <p role="status">{message}</p>}
  </section>;
}
