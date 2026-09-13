import { invoke, isTauri } from "@tauri-apps/api/core";

export const actionTypes = ["start_session", "add_session_note", "generate_next_steps", "create_handoff"] as const;
export const providers = ["codex", "chatgpt", "gemini", "antigravity"] as const;
export type ActionType = typeof actionTypes[number];
export type RequestAction =
  | { action_type: "start_session"; payload: { goal: string } }
  | { action_type: "add_session_note"; payload: { note: string } }
  | { action_type: "generate_next_steps"; payload: Record<string, never> }
  | { action_type: "create_handoff"; payload: { provider: typeof providers[number] } };
export type PreparedRequest = RequestAction & {
  id: string; created_at: string; project_alias: string; preview_title: string;
  preview_body: string; status: "pending"; safety_notice: string;
};
export type RecentRequest = Pick<PreparedRequest, "id" | "created_at" | "project_alias" | "action_type" | "status">;
export type SavedRequest = { path: string; audit_recorded: boolean };

export async function prepareActionRequest(alias: string, action: RequestAction): Promise<PreparedRequest> {
  if (!isTauri()) throw new Error("Open the desktop app to prepare local requests.");
  if (!/^[a-z0-9-]{1,128}$/.test(alias) || !actionTypes.includes(action.action_type)) {
    throw new Error("Choose an allowed action and a valid project alias.");
  }
  try {
    return await invoke<PreparedRequest>("prepare_ghost_action_request", { project_alias: alias, action });
  } catch {
    throw new Error("Request could not be prepared. Check the alias, required text (up to 8000 bytes), and provider. Remove secrets and control characters.");
  }
}

export async function saveActionRequest(request: PreparedRequest, confirmed: boolean): Promise<SavedRequest> {
  if (!isTauri() || !confirmed) throw new Error("Review and explicitly confirm the request in the desktop app.");
  try {
    return await invoke<SavedRequest>("save_ghost_action_request", { request, confirmed: true });
  } catch {
    throw new Error("Save could not be confirmed. Check recent requests and local storage before retrying. No workflow action was performed.");
  }
}
