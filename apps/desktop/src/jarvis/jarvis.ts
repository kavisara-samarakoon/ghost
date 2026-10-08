import { invoke, isTauri } from "@tauri-apps/api/core";
import { blankMemory, type MemoryKind, type MemoryPayload } from "../personal-memory.ts";
import type { EventTime, GoogleAction } from "../google-assistant.ts";
import type { RequestAction } from "../action-requests.ts";
import { MAX_COMMAND_BYTES, utf8Bytes, transcriptHandoffError } from "../transcript-handoff.ts";
export type JarvisStep =
  | { capability: "search_project_memory"; query: string }
  | { capability: "start_session_request"; goal: string }
  | { capability: "add_session_note_request"; note: string }
  | { capability: "generate_next_steps_request" }
  | { capability: "create_handoff_request"; provider: "codex" | "chatgpt" | "gemini" | "antigravity" }
  | { capability: "remember_personal_memory"; kind: MemoryKind; title: string; content: string; tags: string[] }
  | { capability: "search_mail" | "lookup_contact"; query: string }
  | { capability: "list_agenda"; start: string; end: string }
  | { capability: "find_free_time"; start: string; end: string; duration_minutes: number }
  | { capability: "create_mail_draft" | "send_mail"; to: string[]; cc: string[]; subject: string; body: string }
  | { capability: "create_calendar_event"; summary: string; description: string | null; location: string | null; start: EventTime; end: EventTime };
export interface Sharing { personal_memory: boolean; project_memory: boolean }
export const noSharing: Sharing = { personal_memory: false, project_memory: false };
export interface OutboundItem { source: "personal_memory" | "project_memory"; kind: string; title: string; content: string; project_alias: string | null; instruction_trust: "data_only" }
export interface JarvisInput { command: string; project_alias: string | null; context_query: string | null; sharing: Sharing }
export interface JarvisReview extends JarvisInput { version: 1; schema_version: 1; model: string; created_at: number; expires_at: number; context: OutboundItem[]; outbound_input: string; outbound_bytes: number; request_sha256: string; safety_notice: string }
export interface JarvisProposal { version: 1; plan: { kind: "plan" | "clarify" | "unsupported"; summary: string; steps: JarvisStep[] }; project_alias: string | null; model: string; request_sha256: string; proposal_sha256: string; audit_recorded: boolean }
export function immutable<T>(value: T): T { const copy = structuredClone(value); const freeze = (v: unknown) => { if (v && typeof v === "object") { Object.values(v).forEach(freeze); Object.freeze(v); } }; freeze(copy); return copy; }
export function jarvisError(error: unknown): string {
  const messages: Record<string, string> = { unavailable: "Open the native desktop app to use GHOST planning.", invalid_input: "Enter a bounded command without credentials or unsupported formatting.", invalid_context: "Context could not be shared safely. Review its policy, query and bounds.", forbidden_context: "That context source is not allowed to leave the Mac.", changed_review: "The outbound review changed. Prepare a fresh request.", review_expired: "The five-minute outbound review expired.", credential: "The native OpenAI credential is unavailable.", audit: "The private pre-send audit failed. Nothing was sent.", service: "The provider rejected the request. No automatic retry occurred.", transport: "The provider may have received this request. No automatic retry occurred.", response: "A valid response was unavailable. No automatic retry occurred.", proposal: "The response did not pass the finite planner contract.", busy: "Another interpretation is in progress." };
  return typeof error === "string" && messages[error] ? messages[error] : "Planning failed. No automatic retry occurred.";
}
type Invoker = <T>(command: string, args?: Record<string, unknown>) => Promise<T>;
export class JarvisClient {
  private native: () => boolean; private rpc: Invoker; private generation = 0; private pending: JarvisReview | null = null; private busy = false;
  constructor(native: () => boolean = isTauri, rpc: Invoker = invoke) { this.native = native; this.rpc = rpc; }
  get available() { return this.native(); }
  get inProgress() { return this.busy; }
  invalidate() { this.generation++; this.pending = null; }
  async prepare(input: JarvisInput): Promise<JarvisReview> {
    if (!this.available) throw "unavailable"; if (this.busy) throw "busy";
    if (!input.command.trim() || utf8Bytes(input.command) > MAX_COMMAND_BYTES) throw "invalid_input";
    this.invalidate(); const generation = this.generation; this.busy = true;
    try { const review = immutable(await this.rpc<JarvisReview>("prepare_jarvis_request", { input }));
      if (generation !== this.generation) throw "changed_review";
      this.pending = review; return review;
    } finally { this.busy = false; }
  }
  async send(review: JarvisReview): Promise<JarvisProposal> {
    if (!this.available) throw "unavailable"; if (this.busy) throw "busy"; if (this.pending !== review) throw "changed_review";
    this.pending = null; this.busy = true;
    try { return immutable(await this.rpc<JarvisProposal>("interpret_jarvis_request", { input: { review, confirmed: true } })); }
    finally { this.busy = false; }
  }
}
export type TranscriptAdoption =
  | { error: string; draft: null }
  | { error: null; draft: { command: string; sharing: Sharing; query: string; review: null; proposal: null } };
export function adoptTranscript(client: JarvisClient, text: string, busy: boolean): TranscriptAdoption {
  if (!client.available) return { error: "Open the native desktop app to use a transcript in Ask GHOST.", draft: null };
  if (busy || client.inProgress) return { error: "Planning is in progress. The Ask GHOST draft was not replaced; the transcript remains available.", draft: null };
  const error = transcriptHandoffError(text);
  if (error) return { error, draft: null };
  client.invalidate();
  return { error: null, draft: { command: text, sharing: { ...noSharing }, query: "", review: null, proposal: null } };
}
export function memorySuggestion(step: Extract<JarvisStep, { capability: "remember_personal_memory" }>): MemoryPayload {
  return { ...blankMemory(), kind: step.kind, title: step.title, content: step.content, tags: [...step.tags] };
}
export function localRequest(step: JarvisStep): RequestAction | null {
  switch (step.capability) {
    case "start_session_request": return { action_type: "start_session", payload: { goal: step.goal } };
    case "add_session_note_request": return { action_type: "add_session_note", payload: { note: step.note } };
    case "generate_next_steps_request": return { action_type: "generate_next_steps", payload: {} };
    case "create_handoff_request": return { action_type: "create_handoff", payload: { provider: step.provider } };
    default: return null;
  }
}

export function googleMutation(step: JarvisStep): GoogleAction | null {
  switch (step.capability) {
    case "create_mail_draft": case "send_mail": return { action: step.capability, mail: { to: [...step.to], cc: [...step.cc], subject: step.subject, body: step.body } };
    case "create_calendar_event": return { action: "create_calendar_event", event: { summary: step.summary, description: step.description, location: step.location, start: step.start, end: step.end } };
    default: return null;
  }
}
