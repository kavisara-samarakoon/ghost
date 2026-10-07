import { invoke, isTauri } from "@tauri-apps/api/core";
export const memoryKinds = ["identity", "preference", "person", "project", "commitment", "decision", "fact"] as const;
export type MemoryKind = typeof memoryKinds[number];
export type MemorySource = { kind: "manual" } | { kind: "project_reference"; project_alias: string; relative_path: string };
export interface MemoryPayload {
  kind: MemoryKind; title: string; content: string; tags: string[];
  sensitivity: "standard" | "sensitive"; sharing: "local_only" | "provider_allowed";
  source: MemorySource; expires_at: string | null;
}
export interface MemoryRecord { version: 1; memory_id: string; payload: MemoryPayload; created_at: string; updated_at: string; status: "active" | "archived" }
export interface MemoryHit { memory_id: string; kind: MemoryKind; title: string; excerpt: string; tags: string[]; sensitivity: MemoryPayload["sensitivity"]; sharing: MemoryPayload["sharing"]; source: MemorySource; updated_at: string; expires_at: string | null; status: "active" | "archived"; expired: boolean; score: number }
export type MemoryAction = "create_memory" | "update_memory" | "archive_memory" | "delete_memory";
export interface MemoryPrepare { action: MemoryAction; memory_id: string | null; payload: MemoryPayload | null }
export interface MemoryPreview {
  version: 1; request_id: string; action: MemoryAction; memory_id: string; before: MemoryRecord | null; after: MemoryRecord | null;
  created_at: number; expires_at: number; request_sha256: string; confirmation_phrase: string;
}
export interface MemoryOutcome { memory_id: string; changed: boolean; audit_recorded: boolean }
export interface ContextSources { personal_memory: boolean; project_memory: boolean; gmail: boolean; calendar: boolean; contacts: boolean }
export const defaultContextSources: ContextSources = { personal_memory: true, project_memory: true, gmail: false, calendar: false, contacts: false };
export interface ContextInput { query: string; sources: ContextSources; project_alias: string | null; account_id: string | null; calendar_window: { start: string; end: string } | null }
export interface ContextItem { source: string; kind: string; title: string; content: string; timestamp: string | null; reference: string; project_alias: string | null; account_id: string | null; sensitivity: MemoryPayload["sensitivity"] | null; sharing: MemoryPayload["sharing"] | null; instruction_trust: "data_only"; score: number }
export interface ContextPack { version: 1; query: string; created_at: string; sources: ContextSources; project_alias: string | null; account_id: string | null; calendar_window: { start: string; end: string } | null; items: ContextItem[]; warnings: string[]; truncated: boolean; total_bytes: number; context_sha256: string }
export function blankMemory(): MemoryPayload { return { kind: "fact", title: "", content: "", tags: [], sensitivity: "standard", sharing: "local_only", source: { kind: "manual" }, expires_at: null }; }
export function privacyChange(payload: MemoryPayload, sensitivity: MemoryPayload["sensitivity"]): MemoryPayload { return { ...payload, sensitivity, sharing: sensitivity === "sensitive" ? "local_only" : payload.sharing }; }
export function memoryPhrase(p: MemoryPreview): string { return `${({ create_memory: "SAVE", update_memory: "UPDATE", archive_memory: "ARCHIVE", delete_memory: "DELETE" } as const)[p.action]} MEMORY ${p.request_sha256}`; }
export function memoryError(value: unknown): string {
  const messages: Record<string, string> = {
    secret_rejected: "Credentials cannot be stored in personal memory. Nothing was saved.", invalid_input: "Check field bounds, dates and required values.", invalid_privacy: "Sensitive memories must be local-only.", invalid_source: "Choose an existing approved project memory reference.", duplicate_memory: "An exact active duplicate already exists.", memory_limit: "The local memory size or record limit was reached.",
    changed_review: "The review changed. Prepare a fresh preview.", review_expired: "This review expired. Prepare again.", review_missing: "This review was consumed or is unavailable. Prepare again.",
    corrupt_memory: "Memory storage is corrupt or invalid. No automatic repair was attempted.", audit_failed: "Audit could not be recorded. Memory was not changed.",
    write_outcome_uncertain: "Memory may have changed. Review local records before preparing again.", storage_failed: "Private memory storage could not be checked.", busy: "Another operation is running. Try again when it finishes.",
    unavailable: "Personal memory is available only in the native desktop app.", permission_missing: "The selected Google account lacks a requested read permission.", auth_required: "Google authentication is required.", reconnect_required: "Reconnect the Google account before reading context.",
  };
  return typeof value === "string" && messages[value] ? messages[value] : "Operation failed. Review current records before preparing again.";
}
type Command = "get_personal_memory_status" | "list_personal_memories" | "search_personal_memory" | "get_personal_memory" | "prepare_personal_memory_mutation" | "execute_personal_memory_mutation" | "build_unified_context";
type Invoker = <T>(command: string, args?: Record<string, unknown>) => Promise<T>;
function freeze<T>(value: T): T { if (value && typeof value === "object") { Object.values(value).forEach(freeze); Object.freeze(value); } return value; }
export class PersonalMemoryClient {
  private native: () => boolean; private invoke: Invoker; private generation = 0; private pending: MemoryPreview | null = null; private preparing = false;
  constructor(native: () => boolean = isTauri, invoker: Invoker = invoke) { this.native = native; this.invoke = invoker; }
  available() { return this.native(); }
  invalidate() { this.generation++; this.pending = null; }
  private call<T>(command: Command, input: unknown): Promise<T> { if (!this.native()) return Promise.reject("unavailable"); return this.invoke<T>(command, { input }); }
  status() { return this.call<{ record_count: number; plaintext: boolean; provider_transmission: boolean }>("get_personal_memory_status", {}); }
  list(filter: string, kind: MemoryKind | null = null, offset = 0) { return this.call<MemoryHit[]>("list_personal_memories", { filter, kind, limit: 20, offset }); }
  search(query: string) { return this.call<MemoryHit[]>("search_personal_memory", { query, limit: 20 }); }
  get(memory_id: string) { return this.call<MemoryRecord>("get_personal_memory", { memory_id }); }
  context(input: ContextInput) { return this.call<ContextPack>("build_unified_context", input); }
  async prepare(input: MemoryPrepare): Promise<MemoryPreview> {
    if (this.preparing) throw "busy";
    this.invalidate(); const generation = this.generation; this.preparing = true;
    try { const p = await this.call<MemoryPreview>("prepare_personal_memory_mutation", input);
      if (generation !== this.generation) throw "changed_review";
      if (p.confirmation_phrase !== memoryPhrase(p)) throw "changed_review";
      this.pending = freeze(p); return this.pending;
    } finally { this.preparing = false; }
  }
  async execute(p: MemoryPreview, confirmation: string): Promise<MemoryOutcome> {
    if (this.pending !== p || confirmation !== memoryPhrase(p)) throw "changed_review";
    this.invalidate();
    return this.call<MemoryOutcome>("execute_personal_memory_mutation", { request_id: p.request_id, request_sha256: p.request_sha256, confirmation });
  }
}
