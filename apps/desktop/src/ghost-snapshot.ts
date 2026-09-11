import { invoke, isTauri } from "@tauri-apps/api/core";

export interface GhostProject {
  alias: string;
  name: string;
  path: string;
  path_exists: boolean;
  workspace_exists: boolean;
  status_preview: string | null;
  active_session_goal: string | null;
  recent_output_count: number | null;
}

export interface GhostSnapshot {
  mode: "live-local" | "static-preview";
  ghost_home: string | null;
  storage_detected: boolean;
  project_count: number;
  projects: GhostProject[];
  warnings: string[];
  safety: {
    read_only: true;
    no_shell_execution: true;
    no_cli_execution: true;
    no_ai_calls: true;
    no_network_calls: true;
    no_file_writes: true;
  };
}

export interface SnapshotState {
  snapshot: GhostSnapshot | null;
  notice: string | null;
}

export async function loadGhostSnapshot(): Promise<SnapshotState> {
  if (!isTauri()) return { snapshot: null, notice: null };
  try {
    const snapshot = await invoke<GhostSnapshot>("load_ghost_snapshot");
    if (snapshot.mode === "live-local" && snapshot.storage_detected) {
      return { snapshot, notice: null };
    }
    return {
      snapshot: null,
      notice: snapshot.warnings.length > 0
        ? "Local metadata is unavailable. Showing sample projects."
        : "No GHOST storage found. Showing sample projects.",
    };
  } catch {
    // IPC errors may include local details; keep the fallback message generic.
    return { snapshot: null, notice: "Local snapshot could not be loaded. Showing sample projects." };
  }
}

export function selectProject(projects: GhostProject[], alias: string | null): GhostProject | undefined {
  return projects.find((project) => project.alias === alias)
    ?? projects.find((project) => project.active_session_goal)
    ?? projects.find((project) => project.workspace_exists)
    ?? projects[0];
}
