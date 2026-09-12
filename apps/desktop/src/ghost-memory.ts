import { invoke, isTauri } from "@tauri-apps/api/core";
import { safeArtifactPath, type ArtifactAction, type ArtifactActionState } from "./artifact-actions.ts";
import type { GhostSnapshot } from "./ghost-snapshot.ts";

export interface SearchResult {
  project_alias: string;
  project_name: string;
  kind: string;
  title: string;
  relative_path: string;
  snippet: string;
  created_at: string | null;
  openable: boolean;
}

export interface SearchResponse {
  query: string;
  mode: "live-local" | "unavailable";
  results: SearchResult[];
  warnings: string[];
  safety: GhostSnapshot["safety"];
}

function unavailable(message: string): SearchResponse {
  return { query: "", mode: "unavailable", results: [], warnings: [message], safety: {
    read_only: true, no_shell_execution: true, no_cli_execution: true,
    no_ai_calls: true, no_network_calls: true, no_file_writes: true,
  } };
}

export function searchValidation(query: string): string | null {
  const length = Array.from(query.trim()).length;
  if (length < 2) return "Enter at least 2 characters to search local memory.";
  if (length > 120) return "Keep your search to 120 characters or fewer.";
  if (/[\u0000-\u001f\u007f-\u009f]/u.test(query)) return "Use a single line of text to search local memory.";
  return null;
}

export async function searchGhostMemory(mode: GhostSnapshot["mode"], query: string, projectAlias?: string): Promise<SearchResponse> {
  const validation = searchValidation(query);
  if (validation) return unavailable(validation);
  if (mode !== "live-local" || !isTauri()) return unavailable("Local memory search is available in the desktop app. This is a search preview.");
  if (projectAlias !== undefined && !/^[a-z0-9-]{1,128}$/.test(projectAlias)) return unavailable("Choose a registered project to search.");
  try {
    return await invoke<SearchResponse>("search_ghost_memory", {
      query: query.trim(), ...(projectAlias === undefined ? {} : { project_alias: projectAlias }),
    });
  } catch {
    return unavailable("Local search could not be completed. Try again shortly.");
  }
}

export function canActOnSearchResult(response: SearchResponse, result: SearchResult): boolean {
  return isTauri() && response.mode === "live-local" && result.openable === true
    && /^[a-z0-9-]{1,128}$/.test(result.project_alias)
    && (safeArtifactPath(result.relative_path)
      || /^sessions\/[0-9]{8}T[0-9]{12}Z-[a-f0-9]{8}\/(session\.yaml|notes\.md)$/.test(result.relative_path))
    && response.results.includes(result);
}

// User-click only; the M14 native commands revalidate current registry and file identity.
export async function actOnSearchResult(response: SearchResponse, result: SearchResult, action: ArtifactAction): Promise<ArtifactActionState> {
  if (!isTauri() || response.mode !== "live-local") return "unavailable";
  if (!canActOnSearchResult(response, result) || !["open", "reveal"].includes(action)) return "rejected";
  try {
    const state = await invoke<unknown>(action === "open" ? "open_ghost_artifact" : "reveal_ghost_artifact", {
      project_alias: result.project_alias, relative_path: result.relative_path,
    });
    if (state === "rejected" || state === "unavailable") return state;
    return state === (action === "open" ? "opened" : "revealed") ? state as ArtifactActionState : "unavailable";
  } catch { return "unavailable"; }
}
