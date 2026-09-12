import { useEffect, useRef, useState } from "react";
import type { GhostProject, GhostSnapshot } from "./ghost-snapshot.ts";
import { searchGhostMemory, type SearchResponse } from "./ghost-memory.ts";
import SearchResults from "./SearchResults.tsx";

export default function MemorySearch({ mode, project }: { mode: GhostSnapshot["mode"]; project?: GhostProject }) {
  const [query, setQuery] = useState("");
  const [scope, setScope] = useState("all");
  const [pending, setPending] = useState(false);
  const [response, setResponse] = useState<SearchResponse | null>(null);
  const generation = useRef(0);
  const busy = useRef(false);

  // Discard stale results when project or scope changes; never search on typing or selection.
  useEffect(() => {
    generation.current += 1;
    setResponse(null);
    return () => { generation.current += 1; };
  }, [mode, project?.alias, scope]);

  async function submit() {
    if (busy.current) return;
    busy.current = true;
    const request = ++generation.current;
    setPending(true);
    setResponse(null);
    try {
      const result = await searchGhostMemory(mode, query, scope === "project" ? project?.alias : undefined);
      if (request === generation.current) setResponse(result);
    } finally { busy.current = false; setPending(false); }
  }

  return <div className="command-bar-area memory-search">
    {response && <SearchResults response={response} onDismiss={() => setResponse(null)} />}
    <form role="search" aria-label="Search GHOST memory" onSubmit={(event) => { event.preventDefault(); void submit(); }}>
      <div className="memory-search-options">
        <label htmlFor="memory-scope">Search memory</label>
        <select id="memory-scope" value={scope} onChange={(event) => setScope(event.target.value)} disabled={pending}>
          <option value="all">All projects</option>
          {project && <option value="project">Current project: {project.name}</option>}
        </select>
        <span role="status">{pending ? "Searching local memory…" : mode === "live-local" ? "Local · Read-only" : "Desktop search preview"}</span>
      </div>
      <div className="command-bar">
        <svg className="command-bar-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5" aria-hidden="true">
          <circle cx="10.5" cy="10.5" r="6.5" /><path d="m16 16 5 5" />
        </svg>
        <input className="command-input" type="search" value={query} onChange={(event) => setQuery(event.target.value)}
          maxLength={120} placeholder="Search sessions, decisions, outputs, and drafts…" aria-label="Search local memory"
          autoComplete="off" spellCheck={false} disabled={pending} />
        <button className="command-send" type="submit" aria-label="Search local memory" disabled={pending}>
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5" aria-hidden="true"><path d="M5 12h14m-6-6 6 6-6 6" /></svg>
        </button>
      </div>
    </form>
  </div>;
}
