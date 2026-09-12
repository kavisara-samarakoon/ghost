import { useRef, useState } from "react";
import { actionMessages, type ArtifactAction, type ArtifactActionState } from "./artifact-actions.ts";
import { artifactDate } from "./ghost-snapshot.ts";
import { actOnSearchResult, canActOnSearchResult, type SearchResponse, type SearchResult } from "./ghost-memory.ts";

export default function SearchResults({ response, onDismiss }: { response: SearchResponse; onDismiss: () => void }) {
  const busy = useRef(false);
  const [pending, setPending] = useState<string | null>(null);
  const [feedback, setFeedback] = useState<{ key: string; state: ArtifactActionState } | null>(null);
  const keyFor = (result: SearchResult) => `${result.project_alias}/${result.relative_path}`;

  async function handleAction(result: SearchResult, action: ArtifactAction) {
    if (busy.current) return;
    busy.current = true;
    const key = keyFor(result);
    setPending(key);
    setFeedback(null);
    try { setFeedback({ key, state: await actOnSearchResult(response, result, action) }); }
    finally { busy.current = false; setPending(null); }
  }

  return (
    <section className="memory-results" aria-label="Local memory search results">
      <div className="memory-results-heading">
        <h2>Local memory</h2>
        <span role="status">{response.mode === "live-local" ? `${response.results.length} result${response.results.length === 1 ? "" : "s"}` : "Search unavailable"}</span>
        <button type="button" onClick={onDismiss} aria-label="Dismiss search results">Close</button>
      </div>
      {response.query && <p className="memory-query">Results for “{response.query}”</p>}
      {response.warnings.length > 0 && <ul className="memory-notices" role="status">
        {response.warnings.map((warning, index) => <li key={index}>{warning}</li>)}
      </ul>}
      {response.mode === "live-local" && response.results.length === 0 && <p className="memory-empty">No matches in the memory searched. Try another phrase or project.</p>}
      <ul className="memory-result-list">
        {response.results.map((result) => {
          const key = keyFor(result);
          return <li key={key}>
            <div className="memory-result-heading">
              <span className="artifact-kind">{result.kind.replace(/-/g, " ")}</span>
              <h3>{result.title}</h3>
              {result.created_at && <span className="artifact-date">{artifactDate(result.created_at)}</span>}
            </div>
            <p className="memory-snippet">{result.snippet || "No preview text recorded."}</p>
            <span className="artifact-path">{result.project_name} · {result.relative_path}</span>
            {canActOnSearchResult(response, result) && <div className="artifact-actions">
              <button type="button" disabled={pending !== null} onClick={() => void handleAction(result, "open")} aria-label={`Open ${result.title}`}>Open</button>
              <button type="button" disabled={pending !== null} onClick={() => void handleAction(result, "reveal")} aria-label={`Reveal ${result.title}`}>Reveal</button>
              <span className="artifact-action-state" role="status">
                {pending === key ? "Checking artifact…" : feedback?.key === key ? actionMessages[feedback.state] : ""}
              </span>
            </div>}
          </li>;
        })}
      </ul>
    </section>
  );
}
