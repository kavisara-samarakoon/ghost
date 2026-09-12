import { useRef, useState } from "react";
import { artifactDate, type GhostArtifact, type GhostProject, type GhostSnapshot } from "./ghost-snapshot.ts";
import { actOnArtifact, actionMessages, canActOnArtifact, safeArtifactPath, type ArtifactAction, type ArtifactActionState } from "./artifact-actions.ts";

export const artifactCategories = [
  ["all", "All"], ["context-pack", "Context"], ["handoff", "Handoffs"],
  ["output", "Outputs"], ["next-step", "Next steps"], ["update-pack", "Updates"],
] as const;
export type ArtifactCategory = typeof artifactCategories[number][0];
const categoryLabels: Record<GhostArtifact["kind"], string> = {
  "context-pack": "Context pack", handoff: "Handoff", output: "Output",
  "next-step": "Next step", "update-pack": "Update pack",
};
type ArtifactsPageProps = {
  project?: GhostProject;
  mode: GhostSnapshot["mode"];
  onNavigate?: (page: "Projects" | "Sessions" | "Memory") => void;
};

export function filterArtifacts(project: GhostProject | undefined, query: string, category: ArtifactCategory): GhostArtifact[] {
  const terms = query.trim().toLowerCase().split(/\s+/).filter(Boolean);
  return (project?.recent_artifacts ?? []).filter((artifact) => {
    if (category !== "all" && artifact.kind !== category) return false;
    const fields = [artifact.title, artifact.kind, categoryLabels[artifact.kind], project?.name, project?.alias,
      safeArtifactPath(artifact.relative_path) ? artifact.relative_path : ""];
    const text = fields.join(" ").toLowerCase();
    return terms.every((term) => text.includes(term));
  });
}

export function latestDatedArtifact(artifacts: GhostArtifact[]): GhostArtifact | undefined {
  return artifacts.reduce<GhostArtifact | undefined>((latest, artifact) => {
    const date = Date.parse(artifact.created_at ?? "");
    if (!Number.isFinite(date)) return latest;
    return !latest || date > Date.parse(latest.created_at!) ? artifact : latest;
  }, undefined);
}

export function selectedArtifact(visible: GhostArtifact[], path: string | null): GhostArtifact | undefined {
  return visible.find((artifact) => artifact.relative_path === path) ?? visible[0];
}

type ArtifactsViewProps = ArtifactsPageProps & {
  query: string;
  category: ArtifactCategory;
  selectedPath: string | null;
  onQueryChange: (query: string) => void;
  onCategoryChange: (category: ArtifactCategory) => void;
  onSelect: (path: string) => void;
  onAction: (artifact: GhostArtifact, action: ArtifactAction) => void;
  pendingPath: string | null;
  result: { path: string; state: ArtifactActionState } | null;
};

export function ArtifactsView({ project, mode, onNavigate, query, category, selectedPath,
  onQueryChange, onCategoryChange, onSelect, onAction, pendingPath, result }: ArtifactsViewProps) {
  const artifacts = project?.recent_artifacts ?? [];
  const visible = filterArtifacts(project, query, category);
  const selected = selectedArtifact(visible, selectedPath);
  const latest = latestDatedArtifact(artifacts);
  const eligible = artifacts.filter((artifact) => canActOnArtifact(mode, project, artifact)).length;
  const preview = mode === "static-preview";
  function clearFilters() { onQueryChange(""); onCategoryChange("all"); }

  return <div className="artifacts-workspace">
    <dl className="artifacts-overview" aria-label={preview ? "Sample artifact overview" : "Artifact overview"}>
      <div className="glass-panel"><dt>Total artifacts</dt><dd>{artifacts.length}</dd><span>Loaded for the selected project</span></div>
      <div className="glass-panel"><dt>Selected project</dt><dd className="artifacts-overview-text" title={project?.name}>{project?.name ?? "—"}</dd><span>{project?.alias ?? "Choose a registered project"}</span></div>
      <div className="glass-panel"><dt>Latest artifact</dt><dd className="artifacts-overview-text" title={latest?.title}>{latest?.title ?? "—"}</dd><span>{latest ? `Latest dated · ${artifactDate(latest.created_at)}` : "No dated artifacts loaded"}</span></div>
      <div className="glass-panel"><dt>Available actions</dt><dd className="artifacts-overview-text">{eligible ? "Open · Reveal" : "Preview only"}</dd><span>{eligible ? `${eligible} eligible artifact${eligible === 1 ? "" : "s"} · Rechecked on use` : "Review loaded summaries"}</span></div>
    </dl>
    {preview && <p className="artifacts-preview-note">Static preview · Sample data. Open and Reveal are available for eligible artifacts in a live local workspace.</p>}

    {artifacts.length > 0 && <div className="artifacts-filters">
      <div className="artifacts-filter-heading"><label htmlFor="artifact-filter">Filter loaded artifacts</label><p className="artifacts-match-count" role="status">{visible.length} of {artifacts.length} artifacts</p></div>
      <div className="artifacts-filter-row">
        <input id="artifact-filter" type="search" value={query} maxLength={256} placeholder="Title, type, project, or relative path"
          onChange={(event) => onQueryChange(event.target.value)} aria-describedby="artifact-filter-scope" />
        {(query || category !== "all") && <button type="button" onClick={clearFilters}>Clear filters</button>}
      </div>
      <p id="artifact-filter-scope">Searches only loaded metadata for {project?.name}. Change the current project to review another workspace.</p>
      <div className="artifact-category-filters" role="group" aria-label="Artifact category">
        {artifactCategories.map(([value, label]) => <button key={value} type="button" aria-pressed={category === value}
          onClick={() => onCategoryChange(value)}>{label}<span>{value === "all" ? artifacts.length : artifacts.filter((artifact) => artifact.kind === value).length}</span></button>)}
      </div>
    </div>}

    {visible.length === 0 ? <section className="glass-panel page-panel artifacts-empty" aria-label="Artifact results">
      <h2>{artifacts.length ? "No artifacts match your filters" : "No recent artifacts recorded"}</h2>
      <p>{artifacts.length ? "Try another title or category, or clear the filters to review all loaded artifacts."
        : project ? "No artifact previews are loaded for this project. CLI workflows create the generated work you can review here." : "Select a registered project to review its generated work."}</p>
      {artifacts.length > 0 && <button type="button" onClick={clearFilters}>Clear filters</button>}
    </section> : <div className="artifacts-browser">
      <section aria-labelledby="artifact-collection-title">
        <div className="page-panel-heading"><h2 id="artifact-collection-title">Loaded work</h2><span className="page-secondary">Select an artifact to review</span></div>
        <ul className="artifact-card-list">
          {visible.map((artifact) => <li key={artifact.relative_path}>
            <button type="button" className={`glass-panel artifact-select-card${selected === artifact ? " selected" : ""}`}
              aria-pressed={selected === artifact} aria-controls="selected-artifact" onClick={() => onSelect(artifact.relative_path)}>
              <span className="artifact-card-meta"><span>{categoryLabels[artifact.kind]}</span><span>{artifactDate(artifact.created_at)}</span></span>
              <strong>{artifact.title}</strong>
              <span className="artifact-card-project">{project?.name} · {project?.alias}</span>
              <span className="artifact-card-summary">{artifact.preview || "No preview text recorded."}</span>
              <span className="artifact-card-path">{safeArtifactPath(artifact.relative_path) ? artifact.relative_path : "Relative path unavailable"}</span>
            </button>
          </li>)}
        </ul>
      </section>
      {selected && <section id="selected-artifact" className="glass-panel page-panel artifact-selected" aria-labelledby="selected-artifact-title">
        <p className="page-eyebrow">Selected artifact</p>
        <h2 id="selected-artifact-title">{selected.title}</h2>
        <dl className="artifact-selected-meta">
          <div><dt>Category</dt><dd>{categoryLabels[selected.kind]}</dd></div>
          <div><dt>Project</dt><dd>{project?.name} · {project?.alias}</dd></div>
          <div><dt>Created</dt><dd>{artifactDate(selected.created_at)}</dd></div>
          <div><dt>Relative path</dt><dd className="artifact-card-path">{safeArtifactPath(selected.relative_path) ? selected.relative_path : "Relative path unavailable"}</dd></div>
        </dl>
        <div className="artifact-selected-preview"><h3>Recorded preview</h3><p>{selected.preview || "No preview text recorded."}</p></div>
        {canActOnArtifact(mode, project, selected) ? <div className="artifact-actions">
          <button type="button" disabled={pendingPath !== null} aria-label={`Open ${selected.title}`} onClick={() => onAction(selected, "open")}>Open</button>
          <button type="button" disabled={pendingPath !== null} aria-label={`Reveal ${selected.title}`} onClick={() => onAction(selected, "reveal")}>Reveal</button>
          <span className="artifact-action-state" role="status" aria-live="polite">{pendingPath === selected.relative_path ? "Checking artifact…" : result?.path === selected.relative_path ? actionMessages[result.state] : ""}</span>
        </div> : <p className="page-secondary">{preview ? "Sample preview · Native actions are unavailable." : "Open and Reveal are unavailable for this artifact or workspace."}</p>}
        <p className="artifact-safety-note">{eligible ? "Only eligible .ghost artifacts can be opened. Files are rechecked when you choose an action." : "This view uses the summary already loaded with your project."}</p>
      </section>}
    </div>}

    {!!project?.warnings.length && <details className="page-notice"><summary>Project notices ({project.warnings.length})</summary><ul>{project.warnings.map((warning, index) => <li key={index}>{warning}</li>)}</ul></details>}
    <section className="glass-panel page-panel artifacts-guidance" aria-labelledby="artifacts-guidance-title">
      <div><h2 id="artifacts-guidance-title">Create your next artifact from CLI</h2><p>Generate artifacts from the CLI using <code>ghost context pack</code>, <code>ghost handoff</code>, <code>ghost output add</code>, or <code>ghost update-pack</code>. Reopen GHOST to load updated metadata.</p></div>
      {onNavigate && <nav aria-label="Artifact next steps">
        <button type="button" onClick={() => onNavigate("Sessions")}>View session</button>
        <button type="button" onClick={() => onNavigate("Memory")}>Search memory</button>
        <button type="button" onClick={() => onNavigate("Projects")}>View projects</button>
      </nav>}
    </section>
  </div>;
}

export default function ArtifactsPage(props: ArtifactsPageProps) {
  const [query, setQuery] = useState("");
  const [category, setCategory] = useState<ArtifactCategory>("all");
  const [selectedPath, setSelectedPath] = useState<string | null>(null);
  const [pendingPath, setPendingPath] = useState<string | null>(null);
  const [result, setResult] = useState<{ path: string; state: ArtifactActionState } | null>(null);
  const busy = useRef(false);

  async function handleAction(artifact: GhostArtifact, action: ArtifactAction) {
    if (busy.current) return;
    busy.current = true;
    setPendingPath(artifact.relative_path);
    setResult(null);
    try {
      const state = await actOnArtifact(props.mode, props.project, artifact, action);
      setResult({ path: artifact.relative_path, state });
    } finally {
      busy.current = false;
      setPendingPath(null);
    }
  }

  return <ArtifactsView {...props} query={query} category={category} selectedPath={selectedPath}
    onQueryChange={setQuery} onCategoryChange={setCategory} onSelect={setSelectedPath}
    onAction={(artifact, action) => void handleAction(artifact, action)} pendingPath={pendingPath} result={result} />;
}
