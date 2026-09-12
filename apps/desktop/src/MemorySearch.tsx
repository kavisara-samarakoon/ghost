import { useEffect, useRef, useState, type FormEvent, type RefObject } from "react";
import type { GhostProject, GhostSnapshot } from "./ghost-snapshot.ts";
import { searchGhostMemory, type SearchResponse } from "./ghost-memory.ts";
import SearchResults from "./SearchResults.tsx";

const compactSearchChips = ["Project plans", "Technical notes", "Decisions", "Ideas"] as const;
export const memoryQueryExamples = [
  "release notes", "next step", "session goal", "decision",
  "handoff", "NEXORA", "SentinelLite AI", "ARM-SecNet",
] as const;

type SearchScope = "all" | "project";
type MemoryDestination = "Projects" | "Sessions" | "Artifacts";

type MemorySearchProps = {
  mode: GhostSnapshot["mode"];
  project?: GhostProject;
  projects?: GhostProject[];
  searchInputRef?: RefObject<HTMLInputElement | null>;
  layout?: "compact" | "page";
  onNavigate?: (page: MemoryDestination) => void;
};

export type MemorySearchViewProps = MemorySearchProps & {
  query: string;
  scope: SearchScope;
  pending: boolean;
  response: SearchResponse | null;
  inputRef: RefObject<HTMLInputElement | null>;
  onQueryChange: (query: string) => void;
  onScopeChange: (scope: SearchScope) => void;
  onSubmit: () => void;
  onDismiss: () => void;
};

// Kept small and exported so the stale-response rule remains directly testable.
export function currentSearchResponse(request: number, generation: number, response: SearchResponse): SearchResponse | null {
  return request === generation ? response : null;
}

export function MemorySearchForm({ mode, project, query, scope, pending, inputRef, onQueryChange, onScopeChange, onSubmit, layout }: MemorySearchViewProps) {
  function handleSubmit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    onSubmit();
  }

  return <form role="search" aria-label="Search GHOST memory" onSubmit={handleSubmit}>
    <div className="memory-search-options">
      <label htmlFor={layout === "page" ? "memory-page-scope" : "memory-scope"}>Scope</label>
      <select id={layout === "page" ? "memory-page-scope" : "memory-scope"} value={scope}
        onChange={(event) => onScopeChange(event.target.value as SearchScope)} disabled={pending}>
        <option value="all">All projects</option>
        {project && <option value="project">Current: {project.name}</option>}
      </select>
      <span role="status" aria-live="polite">{pending ? "Searching local memory…" : mode === "live-local" ? "Local · Read-only" : "Desktop search preview"}</span>
    </div>
    <div className="command-bar">
      <svg className="command-bar-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5" aria-hidden="true">
        <circle cx="10.5" cy="10.5" r="6.5" /><path d="m16 16 5 5" />
      </svg>
      <input ref={inputRef} className="command-input" type="search" value={query} onChange={(event) => onQueryChange(event.target.value)}
        maxLength={120} placeholder="Search sessions, decisions, handoffs, outputs, or project notes…" aria-label="Search local memory"
        autoComplete="off" spellCheck={false} disabled={pending} />
      <button className="command-send" type="submit" aria-label="Search local memory" disabled={pending}>
        {layout === "page" && <span>Search</span>}
        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5" aria-hidden="true"><path d="M5 12h14m-6-6 6 6-6 6" /></svg>
      </button>
    </div>
  </form>;
}

export function MemorySearchView(props: MemorySearchViewProps) {
  const { mode, project, projects = [], query, scope, pending, response, inputRef, layout = "compact", onNavigate } = props;
  const preview = mode === "static-preview";

  function selectExample(example: string) {
    props.onQueryChange(example);
    inputRef.current?.focus();
  }

  const form = <MemorySearchForm {...props} layout={layout} />;
  const results = response && <SearchResults response={response} onDismiss={props.onDismiss} />;
  const empty = !response && !pending && <div className="memory-empty-state">
    <svg className="memory-empty-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.2" aria-hidden="true">
      <circle cx="10.5" cy="10.5" r="6.5" /><path d="m16 16 5 5" />
    </svg>
    <div>
      <strong>{preview ? "Desktop search preview" : query ? "Ready to search" : "Search your local workspace"}</strong>
      <span className="memory-empty-text">{preview
        ? "No local files are searched in browser preview. Open the native desktop app and submit a query to search approved memory locations."
        : query ? "Your query is pre-filled. Press Search when you are ready; suggestions never search automatically."
          : "Enter a query to review sessions, decisions, outputs, drafts, context packs, handoffs, project notes, and generated summaries."}</span>
    </div>
  </div>;

  if (layout === "compact") return <div className="memory-search memory-search-compact">
    {form}
    <div className="memory-chips">
      {compactSearchChips.map((chip) => <button key={chip} type="button" className="memory-chip" onClick={() => selectExample(chip)}>{chip}</button>)}
    </div>
    {results}
    {empty}
  </div>;

  return <div className="memory-search memory-search-page">
    <dl className="memory-overview" aria-label={preview ? "Sample memory overview" : "Memory overview"}>
      <div className="glass-panel"><dt>Selected project</dt><dd title={project?.name}>{project?.name ?? "—"}</dd><span>{project?.alias ?? "No registered project selected"}</span></div>
      <div className="glass-panel"><dt>Search scope</dt><dd>{scope === "project" && project ? "Current project" : "All projects"}</dd><span>{scope === "project" && project ? project.name : "Across the loaded registry"}</span></div>
      <div className="glass-panel"><dt>Loaded projects</dt><dd>{projects.length}</dd><span>{preview ? "Sample project metadata" : "Available to the current snapshot"}</span></div>
      <div className="glass-panel"><dt>Memory source</dt><dd>{preview ? "Preview only" : "Approved locations"}</dd><span>Read-only · Explicit submit</span></div>
    </dl>
    {preview && <p className="memory-preview-note">Static preview · Sample project metadata. Memory results are not simulated and no local search is performed.</p>}

    <div className="memory-page-grid">
      <section className="glass-panel page-panel memory-search-primary" aria-labelledby="memory-search-title">
        <div className="page-panel-heading">
          <div><p className="page-eyebrow">Read-only retrieval</p><h2 id="memory-search-title">Search local memory</h2></div>
          <span className="page-badge">Manual search only</span>
        </div>
        <p className="memory-search-intro">Choose a scope, enter a phrase, then submit when ready. Changing scope or project clears earlier results.</p>
        {form}
        {results}
        {empty}
      </section>

      <aside className="memory-support" aria-label="Memory search guidance">
        <section className="glass-panel page-panel memory-query-guide" aria-labelledby="memory-query-guide-title">
          <p className="page-eyebrow">Query examples</p>
          <h2 id="memory-query-guide-title">Start with a phrase</h2>
          <p>Suggestions fill the search field only. Review or edit the query, then submit it yourself.</p>
          <div className="memory-chips">
            {memoryQueryExamples.map((example) => <button key={example} type="button" className="memory-chip" onClick={() => selectExample(example)}>{example}</button>)}
          </div>
        </section>

        <section className="glass-panel page-panel memory-safety" aria-labelledby="memory-safety-title">
          <p className="page-eyebrow">Safe interpretation</p>
          <h2 id="memory-safety-title">Review snippets with context</h2>
          <ul>
            <li>Results are read-only snippets from approved GHOST memory locations.</li>
            <li>Use Artifacts to open generated files safely.</li>
            <li>Search does not scan source code or .env files.</li>
          </ul>
        </section>

        {onNavigate && <nav className="glass-panel page-panel memory-next-links" aria-label="Memory next steps">
          <p className="page-eyebrow">Continue reviewing</p>
          <button type="button" onClick={() => onNavigate("Projects")}>Projects<span>Change or review workspace selection</span></button>
          <button type="button" onClick={() => onNavigate("Sessions")}>Sessions<span>Review goals and recorded notes</span></button>
          <button type="button" onClick={() => onNavigate("Artifacts")}>Artifacts<span>Review generated files safely</span></button>
        </nav>}
      </aside>
    </div>
  </div>;
}

export default function MemorySearch({ mode, project, projects, searchInputRef, layout = "compact", onNavigate }: MemorySearchProps) {
  const [query, setQuery] = useState("");
  const [scope, setScope] = useState<SearchScope>("all");
  const [pending, setPending] = useState(false);
  const [response, setResponse] = useState<SearchResponse | null>(null);
  const generation = useRef(0);
  const busy = useRef(false);
  const localInputRef = useRef<HTMLInputElement>(null);
  const inputRef = searchInputRef ?? localInputRef;

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
      const current = currentSearchResponse(request, generation.current, result);
      if (current) setResponse(current);
    } finally {
      busy.current = false;
      setPending(false);
    }
  }

  return <MemorySearchView mode={mode} project={project} projects={projects} searchInputRef={searchInputRef}
    layout={layout} onNavigate={onNavigate} query={query} scope={scope} pending={pending} response={response}
    inputRef={inputRef} onQueryChange={setQuery} onScopeChange={setScope} onSubmit={() => void submit()}
    onDismiss={() => setResponse(null)} />;
}
