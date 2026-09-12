import { invoke, isTauri } from "@tauri-apps/api/core";
import type { GhostArtifact, GhostProject, GhostSnapshot } from "./ghost-snapshot";

export type ArtifactAction = "open" | "reveal";
export type ArtifactActionState = "opened" | "revealed" | "rejected" | "unavailable";

export const actionMessages: Record<ArtifactActionState, string> = {
  opened: "Opened in your default app.",
  revealed: "Revealed in Finder.",
  rejected: "Action rejected. This artifact is no longer available or allowed.",
  unavailable: "Artifact actions are unavailable here. Try the desktop app.",
};

export function safeArtifactPath(path: string): boolean {
  if (path.length > 512) return false;
  const parts = path.split("/");
  const segment = (name: string) => /^[A-Za-z0-9_-]{1,180}$/.test(name);
  const markdown = (name: string) => name.endsWith(".md") && segment(name.slice(0, -3));
  if (parts[0] === "outputs" && parts.length === 3 && ["codex", "terminal"].includes(parts[1])) {
    return markdown(parts[2]) && new RegExp(`^[0-9]{8}T[0-9]{12}Z-${parts[1]}-output-[a-z0-9_]{8}\\.md$`).test(parts[2]);
  }
  if (parts[0] !== "drafts") return false;
  if (parts.length === 3 && ["context-packs", "next-steps"].includes(parts[1])) return markdown(parts[2]);
  if (parts.length === 4 && parts[1] === "handoffs"
    && ["codex", "chatgpt", "gemini", "antigravity"].includes(parts[2])) return markdown(parts[3]);
  return parts.length === 4 && parts[1] === "update-packs" && segment(parts[2]) && markdown(parts[3]);
}

export function canActOnArtifact(mode: GhostSnapshot["mode"], project: GhostProject | undefined, artifact: GhostArtifact): boolean {
  return mode === "live-local" && !!project?.workspace_exists && project.path_exists
    && /^[a-z0-9-]{1,128}$/.test(project.alias) && safeArtifactPath(artifact.relative_path)
    && project.recent_artifacts.some((item) => item.relative_path === artifact.relative_path);
}

// Called only by artifact buttons. No absolute path or application name crosses IPC.
export async function actOnArtifact(mode: GhostSnapshot["mode"], project: GhostProject | undefined,
  artifact: GhostArtifact, action: ArtifactAction): Promise<ArtifactActionState> {
  if (mode !== "live-local" || !isTauri()) return "unavailable";
  if (!canActOnArtifact(mode, project, artifact) || !["open", "reveal"].includes(action)) return "rejected";
  try {
    const state = await invoke<unknown>(action === "open" ? "open_ghost_artifact" : "reveal_ghost_artifact", {
      project_alias: project!.alias, relative_path: artifact.relative_path,
    });
    if (state === "rejected" || state === "unavailable") return state;
    if (state === (action === "open" ? "opened" : "revealed")) return state as ArtifactActionState;
    return "unavailable";
  } catch {
    // Never display raw IPC errors, which may contain local paths or metadata.
    return "unavailable";
  }
}
