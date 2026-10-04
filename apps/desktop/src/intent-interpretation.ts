import { invoke, isTauri } from "@tauri-apps/api/core";

export const MAX_INTENT_BYTES = 8 * 1024;
export type PlanStep =
  | { action: "start_session"; goal: string }
  | { action: "add_session_note"; note: string }
  | { action: "generate_next_steps" }
  | { action: "create_handoff"; provider: "codex" | "chatgpt" | "gemini" | "antigravity" };
export type PreparedIntent = {
  version: 1; project_alias: string; intent: string; model: string; schema_version: 1;
  request_sha256: string; safety_notice: string;
};
export type IntentProposal = {
  kind: "plan" | "clarify" | "unsupported"; summary: string; steps: PlanStep[];
  project_alias: string; model: string; request_sha256: string; proposal_sha256: string; audit_recorded: boolean;
};
export type SavedPlan = { path: string; plan_sha256: string; audit_recorded: boolean };

export function intentError(error: unknown): string {
  const category = typeof error === "string" ? error : "";
  switch (category) {
    case "unavailable": return "Intent interpretation is available only in the desktop app.";
    case "invalid_intent": return "Enter nonblank intent up to 8 KiB. Remove unsafe formatting and secrets.";
    case "changed_review": return "The reviewed intent changed. Prepare and review it again.";
    case "credential": return "OpenAI credential unavailable in the native app environment.";
    case "audit": return "Local audit unavailable. No interpretation request was sent.";
    case "service": return "The interpretation service failed. GHOST did not retry automatically.";
    case "transport": return "Interpretation may have reached the provider. GHOST did not retry automatically.";
    case "response": case "proposal": return "A safe intent proposal was unavailable. GHOST did not retry automatically.";
    case "busy": return "An interpretation is already in progress.";
    case "save": return "Plan save could not be confirmed. Inspect local intent drafts before saving again.";
    default: return "Intent operation could not be confirmed. GHOST did not retry automatically. Inspect any saved drafts before trying again.";
  }
}
function requireDesktop() { if (!isTauri()) throw "unavailable"; }
export async function prepareIntent(projectAlias: string, text: string): Promise<PreparedIntent> {
  requireDesktop();
  if (!/^[a-z0-9-]{1,128}$/.test(projectAlias) || !text.trim() || new TextEncoder().encode(text).length > MAX_INTENT_BYTES) throw "invalid_intent";
  return invoke<PreparedIntent>("prepare_ghost_intent", { project_alias: projectAlias, intent: text });
}
export async function interpretIntent(prepared: PreparedIntent): Promise<IntentProposal> {
  requireDesktop();
  return invoke<IntentProposal>("interpret_ghost_intent", { prepared, confirmed: true });
}
export async function saveIntentPlan(proposal: IntentProposal): Promise<SavedPlan> {
  requireDesktop();
  if (proposal.kind !== "plan") throw "save";
  return invoke<SavedPlan>("save_ghost_intent_plan", { proposal, confirmed: true });
}
export type IntentEnvironment = {
  available: boolean;
  prepare: typeof prepareIntent;
  interpret: typeof interpretIntent;
  save: typeof saveIntentPlan;
};
export function desktopIntentEnvironment(available: boolean): IntentEnvironment {
  return { available: available && isTauri(), prepare: prepareIntent, interpret: interpretIntent, save: saveIntentPlan };
}
export type IntentState = {
  phase: "unavailable" | "draft" | "preparing" | "review" | "sending" | "proposal" | "saving" | "saved";
  projectAlias: string; text: string; prepared: PreparedIntent | null;
  proposal: IntentProposal | null; saved: SavedPlan | null; message: string | null;
};
function frozen<T>(value: T): T {
  // All DTOs are plain JSON. Isolate reviewed data from mutable caller-owned objects.
  const copy = structuredClone(value);
  const freeze = (item: unknown): void => {
    if (item && typeof item === "object") { Object.values(item).forEach(freeze); Object.freeze(item); }
  };
  freeze(copy); return copy;
}
export class IntentController {
  private env: IntentEnvironment;
  private state: IntentState;
  private listeners = new Set<() => void>();
  private generation = 0;
  private busy = false;
  private disposed = false;
  constructor(env: IntentEnvironment, projectAlias: string) {
    this.env = env;
    this.state = { phase: env.available ? "draft" : "unavailable", projectAlias, text: "", prepared: null, proposal: null, saved: null, message: null };
  }
  getState = (): IntentState => this.state;
  subscribe = (listener: () => void) => { this.listeners.add(listener); return () => { this.listeners.delete(listener); }; };
  private update(next: Partial<IntentState>) {
    if (this.disposed) return;
    this.state = Object.freeze({ ...this.state, ...next }); this.listeners.forEach(listener => listener());
  }
  setProject(projectAlias: string) {
    if (projectAlias === this.state.projectAlias) return;
    this.generation++;
    if (["sending", "proposal", "saving", "saved"].includes(this.state.phase)) {
      // The result remains visibly bound to its reviewed alias; only the next draft follows selection.
      this.update({ projectAlias }); return;
    }
    this.update({ projectAlias, prepared: null, phase: this.env.available ? "draft" : "unavailable", message: null });
  }
  edit(text: string) {
    if (this.disposed || this.busy || !this.env.available) return;
    this.generation++;
    this.update({ text, prepared: null, proposal: null, saved: null, phase: "draft", message: null });
  }
  useTranscript(text: string) { this.edit(text); } // Only called by a user click; zero IPC.
  discard() {
    if (this.disposed || this.busy) return;
    this.generation++;
    this.update({ text: "", prepared: null, proposal: null, saved: null, phase: this.env.available ? "draft" : "unavailable", message: null });
  }
  async prepare() {
    if (this.disposed || this.busy || !this.env.available || this.state.phase !== "draft") return;
    const generation = this.generation;
    const { projectAlias, text } = this.state;
    this.busy = true; this.update({ phase: "preparing", message: null });
    try {
      const prepared = frozen(await this.env.prepare(projectAlias, text));
      if (this.disposed || generation !== this.generation) return;
      this.update({ prepared, phase: "review" });
    } catch (error) {
      if (generation === this.generation) this.update({ phase: "draft", message: intentError(error) });
    } finally { this.busy = false; }
  }
  async send() {
    if (this.disposed || this.busy || !this.env.available || this.state.phase !== "review" || !this.state.prepared) return;
    const prepared = this.state.prepared;
    this.busy = true; this.update({ phase: "sending", message: null });
    try {
      const proposal = frozen(await this.env.interpret(prepared));
      this.update({ proposal, phase: "proposal", message: proposal.audit_recorded ? null : "Proposal received, but completion audit could not be recorded. Do not resend automatically." });
    } catch (error) {
      const changed = this.state.projectAlias !== prepared.project_alias;
      this.update({ phase: changed ? "draft" : "review", prepared: changed ? null : prepared, message: intentError(error) });
    }
    finally { this.busy = false; }
  }
  async save() {
    const proposal = this.state.proposal;
    if (this.disposed || this.busy || !this.env.available || this.state.phase !== "proposal" || proposal?.kind !== "plan") return;
    this.busy = true; this.update({ phase: "saving", message: null });
    try {
      const saved = frozen(await this.env.save(proposal));
      this.update({ saved, phase: "saved", message: saved.audit_recorded ? null : "Plan saved, but save audit could not be recorded. Keep this path; do not create a duplicate automatically." });
    } catch (error) { this.update({ phase: "proposal", message: intentError(error) }); }
    finally { this.busy = false; }
  }
  dispose() { this.disposed = true; this.generation++; this.listeners.clear(); }
}
