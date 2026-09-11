import { useState } from "react";
import { artifactDate, type GhostProject } from "./ghost-snapshot";

const kindLabels = {
  output: "Output", handoff: "Handoff", "context-pack": "Context pack",
  "next-step": "Next step", "update-pack": "Update pack",
};

export default function LatestWork({ project }: { project: GhostProject | undefined }) {
  const [showAll, setShowAll] = useState(false);
  const artifacts = project?.recent_artifacts ?? [];
  const visible = showAll ? artifacts : artifacts.slice(0, 3);
  const hasWarnings = (project?.warnings.length ?? 0) > 0;

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
                </div>
              </details>
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}
