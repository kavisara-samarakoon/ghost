import type { GhostProject } from "./ghost-snapshot.ts";

// Presentation-only samples. Always pair these records with static-preview mode.
export const sampleProjects: GhostProject[] = [
  ["nexora", "NEXORA"],
  ["sentinellite", "SentinelLite AI"],
  ["arm-secnet", "ARM-SecNet"],
  ["portfolio", "Portfolio"],
  ["university", "University Work"],
].map(([alias, name], index) => ({
  alias, name, path: "", path_exists: false, workspace_exists: false,
  status_preview: "Sample project · Connect your local GHOST workspace in the desktop app.",
  active_session_goal: null,
  recent_output_count: null,
  active_session: index === 0 ? {
    id: "sample-session", goal_preview: "Add wishlist price alert MVP", status: "active",
    started_at: null, note_preview: "Review the workflow, validate the change, and prepare a concise handoff.",
  } : null,
  recent_artifacts: index === 0 ? [{
    kind: "context-pack", title: "Wishlist alert context", relative_path: "drafts/context-packs/sample.md",
    preview: "Sample context pack outlining the current goal and next review steps.", created_at: null,
  }, {
    kind: "next-step", title: "Validation and handoff", relative_path: "drafts/next-steps/sample.md",
    preview: "Sample next steps: review the implementation, record validation, and prepare the handoff.", created_at: null,
  }] : [],
  counts: { sessions: index === 0 ? 1 : 0, outputs: null, handoffs: null, context_packs: null, next_steps: null, update_packs: null },
  warnings: [],
}));
