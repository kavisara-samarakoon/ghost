import { useRef, useState } from "react";
import { artifactDate, type GhostArtifact, type GhostProject, type GhostSnapshot } from "./ghost-snapshot";
import { actOnArtifact, actionMessages, canActOnArtifact, type ArtifactAction, type ArtifactActionState } from "./artifact-actions";

const kindLabels = {
  output: "Output", handoff: "Handoff", "context-pack": "Context pack",
  "next-step": "Next step", "update-pack": "Update pack",
};

export default function LatestWork({ project, mode }: { project: GhostProject | undefined; mode: GhostSnapshot["mode"] }) {
  const [showAll, setShowAll] = useState(false);
  const [pending, setPending] = useState<string | null>(null);
  const busy = useRef(false);
  const [result, setResult] = useState<{ path: string; state: ArtifactActionState } | null>(null);
  const artifacts = project?.recent_artifacts ?? [];
  const visible = showAll ? artifacts : artifacts.slice(0, 3);
  const hasWarnings = (project?.warnings.length ?? 0) > 0;

  async function handleAction(artifact: GhostArtifact, action: ArtifactAction) {
    if (busy.current) return;
    busy.current = true;
    setPending(artifact.relative_path);
    setResult(null);
    try {
      const state = await actOnArtifact(mode, project, artifact, action);
      setResult({ path: artifact.relative_path, state });
    } finally {
      busy.current = false;
      setPending(null);
    }
  }

  return (
    <section className="next-action-area latest-work" aria-labelledby="latest-work-title">
      <div className="latest-work-heading">
        <h2 id="latest-work-title" className="next-action-label">Latest work</h2>
        {artifacts.length > 0 && <span className="artifact-indicator">Artifacts loaded</span>}
        {artifacts.length > 3 && (
          <button className="artifact-toggle" type="button" onClick={() => setShowAll(!showAll)}
            aria-expanded={showAll} aria-controls="recent-artifact-list">
            {showAll ? "Show latest 3" : `Show all ${artifacts.length}`}
          </button>
        )}
      </div>
      {artifacts.length === 0 ? (
        <p className="artifact-empty">{hasWarnings ? "No artifact previews available. See snapshot notices." : "No recent artifacts recorded."}</p>
      ) : (
        <ul id="recent-artifact-list" className="artifact-list">
          {visible.map((artifact) => (
            <li key={artifact.relative_path}>
              <details className="artifact-detail">
                <summary>
                  <span className="artifact-kind">{kindLabels[artifact.kind]}</span>
                  <span className="artifact-title">{artifact.title}</span>
                  <span className="artifact-date">{artifactDate(artifact.created_at)}</span>
                </summary>
                <div className="artifact-preview">
                  <p>{artifact.preview ?? "No preview text recorded."}</p>
                  <span className="artifact-path">{artifact.relative_path}</span>
                  {canActOnArtifact(mode, project, artifact) && (
                    <div className="artifact-actions">
                      <button type="button" disabled={pending !== null} onClick={() => void handleAction(artifact, "open")}
                        aria-label={`Open ${artifact.title}`}>Open</button>
                      <button type="button" disabled={pending !== null} onClick={() => void handleAction(artifact, "reveal")}
                        aria-label={`Reveal ${artifact.title}`}>Reveal</button>
                      <span className="artifact-action-state" role="status" aria-live="polite">
                        {pending === artifact.relative_path ? "Checking artifact…"
                          : result?.path === artifact.relative_path ? actionMessages[result.state] : ""}
                      </span>
                    </div>
                  )}
                </div>
              </details>
            </li>
          ))}
        </ul>
      )}
      {mode === "live-local" && artifacts.length > 0 && <p className="artifact-safety-note">Only .ghost artifacts can be opened.</p>}
    </section>
  );
}
