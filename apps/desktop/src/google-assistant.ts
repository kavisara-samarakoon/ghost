import { invoke, isTauri } from "@tauri-apps/api/core";

export const permissionLabels = {
  mail_read: "Mail read", calendar_read: "Calendar read + free/busy", contacts_read: "Contacts read",
  mail_draft: "Mail draft", mail_send: "Mail send", calendar_event_create: "Calendar create", calendar_event_update: "Calendar update",
} as const;
export type GooglePermission = keyof typeof permissionLabels;
export const readPermissions: GooglePermission[] = ["mail_read", "calendar_read", "contacts_read"];
export const writePermissions: GooglePermission[] = ["mail_draft", "mail_send", "calendar_event_create", "calendar_event_update"];
export interface GoogleAccount { account_id: string; provider: "google"; display_label: string; granted_permissions: GooglePermission[]; status: "connected" | "disconnected" }
export interface GoogleStatus { credential_checks_available: boolean; config: { configured: boolean; client_id: string | null }; accounts: GoogleAccount[]; connectors: { provider: "google"; display_name: string; supported_permissions: GooglePermission[]; connection_capability: string; credential_backend_supported: boolean }[] }
export interface MailInput { to: string[]; cc: string[]; subject: string; body: string }
export interface EventTime { date_time?: string; date?: string }
export interface EventInput { summary: string; description: string | null; location: string | null; start: EventTime; end: EventTime }
export interface EventChanges { summary: string | null; description: string | null; location: string | null; start: EventTime | null; end: EventTime | null }
export interface CalendarEvent { event_id: string; etag: string; fields: EventInput; status: string }
export type GoogleAction = { action: "create_mail_draft" | "send_mail"; mail: MailInput }
  | { action: "create_calendar_event"; event: EventInput }
  | { action: "update_calendar_event"; event_id: string; etag: string; changes: EventChanges };
export interface PreparedGoogleMutation {
  version: 1; request_id: string; account_id: string; required_permission: GooglePermission; payload: GoogleAction;
  preview: { account_label: string; sender: string | null; mail: MailInput | null; old_event: EventInput | null; new_event: EventInput | null; body_bytes: number | null };
  created_at: number; expires_at: number; context_sha256: string; request_sha256: string;
}
export interface MailResult { messages: { message_id: string; thread_id: string; from: string; subject: string; date: string; snippet: string; unread: boolean; important: boolean; timestamp_ms: number }[]; unread_in_results: number; truncated: boolean; digest: string }
export interface AgendaResult { events: CalendarEvent[]; truncated: boolean }
export interface Interval { start: string; end: string }
export interface FreeTimeResult { busy: Interval[]; free: Interval[]; candidates: Interval[] }
export interface ContactResult { contacts: { resource_name: string; display_name: string; emails: string[]; phones: string[]; organization: string | null }[]; truncated: boolean }
export interface MutationResult { operation: string; provider_id: string; audit_recorded: boolean }
export interface ConnectResult { account: GoogleAccount; audit_recorded: boolean }
const commands = ["get_google_assistant_status", "save_google_client_id", "connect_google", "search_google_mail", "get_google_mail_digest", "list_google_agenda", "find_google_free_time", "lookup_google_contacts", "prepare_google_mutation", "execute_google_mutation", "disconnect_google_account"] as const;
type Command = typeof commands[number];
type Invoke = <T>(command: string, args?: Record<string, unknown>) => Promise<T>;
const messages: Record<string, string> = {
  unavailable: "Google integration requires the macOS desktop app.", busy: "A Google operation is already in progress.",
  client_required: "Save a Google Desktop OAuth client ID first.", invalid_client: "Enter a valid Desktop client ID ending in .apps.googleusercontent.com.",
  accounts_exist: "Disconnect existing local accounts before changing the Google client ID.", invalid_input: "Check the fields and their limits.",
  permission_missing: "This account has not granted the required GHOST permission.", auth_required: "Authorization expired or was rejected. Retry explicitly, or reconnect the account.",
  reconnect_required: "Reconnect this account through Google consent.", storage_failed: "Private connector storage could not be accessed.", audit_failed: "The private connector audit could not be recorded.",
  browser_failed: "The browser could not be opened. The connection flow stopped.", consent_denied: "Google consent was declined.",
  timeout: "The operation timed out. For a write, check Google before preparing another attempt.",
  transport_failed: "The network outcome may be uncertain. For a write, check Google before preparing another attempt.",
  provider_failed: "Google rejected the operation. No automatic retry occurred.", invalid_response: "Google returned an unsupported response. For a write, check Google before preparing another attempt.",
  conflict: "The event changed in Google. Reload the agenda and prepare a fresh preview.", changed_review: "The reviewed request changed. Prepare a fresh preview.",
  unsupported_event: "This event includes guests, recurrence or special event features. Update it in Google Calendar.",
  already_used: "That preview was already attempted or is unavailable. Check Google before preparing again.", review_expired: "The preview expired. Prepare again.",
  confirmation_required: "Review and enter the exact confirmation phrase.", reconciliation_required: "Local connection cleanup is incomplete. Reload account status and retry local disconnect before reconnecting.",
};
export function googleError(error: unknown): string { return typeof error === "string" && messages[error] ? messages[error] : "Google operation unavailable. No automatic retry occurred."; }
function immutable<T>(value: T): T {
  if (value && typeof value === "object") { Object.values(value).forEach(immutable); Object.freeze(value); }
  return value;
}
export function confirmationPhrase(prepared: PreparedGoogleMutation): string {
  const phrases = { create_mail_draft: "SAVE DRAFT", send_mail: "SEND MAIL", create_calendar_event: "CREATE EVENT", update_calendar_event: "UPDATE EVENT" };
  return `${phrases[prepared.payload.action]} ${prepared.request_sha256}`;
}
export function eventChanges(old: EventInput, current: EventInput): EventChanges {
  return { summary: old.summary === current.summary ? null : current.summary,
    description: old.description === current.description ? null : current.description ?? "",
    location: old.location === current.location ? null : current.location ?? "",
    start: JSON.stringify(old.start) === JSON.stringify(current.start) ? null : current.start,
    end: JSON.stringify(old.end) === JSON.stringify(current.end) ? null : current.end };
}
export class GoogleAssistantClient {
  private pending: PreparedGoogleMutation | null = null;
  private sending = false;
  private generation = 0;
  private native: () => boolean;
  private rpc: Invoke;
  constructor(native: () => boolean = isTauri, rpc: Invoke = invoke) { this.native = native; this.rpc = rpc; }
  get available() { return this.native(); }
  invalidate() { this.pending = null; this.generation++; }
  private call<T>(command: Command, input?: unknown): Promise<T> {
    if (!this.native()) return Promise.reject("unavailable");
    return this.rpc<T>(command, input === undefined ? undefined : { input });
  }
  status() { return this.call<GoogleStatus>("get_google_assistant_status"); }
  saveClient(client_id: string) { this.invalidate(); return this.call<GoogleStatus["config"]>("save_google_client_id", { client_id }); }
  connect(display_label: string, requested_permissions: GooglePermission[], confirmed: boolean) {
    if (!confirmed || !requested_permissions.length) return Promise.reject("confirmation_required");
    this.invalidate(); return this.call<ConnectResult>("connect_google", { display_label, requested_permissions, confirmed });
  }
  search(account_id: string, query: string, digest = false) { return this.call<MailResult>(digest ? "get_google_mail_digest" : "search_google_mail", { account_id, query, limit: 20 }); }
  agenda(account_id: string, start: string, end: string) { return this.call<AgendaResult>("list_google_agenda", { account_id, window: { start, end }, limit: 50 }); }
  freeTime(account_id: string, start: string, end: string, duration_minutes: number) { return this.call<FreeTimeResult>("find_google_free_time", { account_id, window: { start, end }, duration_minutes }); }
  contacts(account_id: string, query: string) { return this.call<ContactResult>("lookup_google_contacts", { account_id, query }); }
  async prepare(account_id: string, payload: GoogleAction): Promise<PreparedGoogleMutation> {
    if (this.sending) throw "busy";
    this.invalidate(); const generation = this.generation;
    const prepared = await this.call<PreparedGoogleMutation>("prepare_google_mutation", { account_id, payload });
    if (generation !== this.generation || prepared.account_id !== account_id) throw "changed_review";
    this.pending = immutable(prepared); return this.pending;
  }
  async execute(prepared: PreparedGoogleMutation, confirmation: string): Promise<MutationResult> {
    if (this.sending || this.pending !== prepared || confirmation !== confirmationPhrase(prepared)) throw "confirmation_required";
    this.sending = true; this.pending = null;
    try { return await this.call<MutationResult>("execute_google_mutation", { prepared, confirmation }); }
    finally { this.sending = false; }
  }
  disconnect(account_id: string, confirmation: string) {
    if (confirmation !== `DISCONNECT ${account_id}`) return Promise.reject("confirmation_required");
    this.invalidate(); return this.call<{ local_credentials_removed: boolean; notice: string; audit_recorded: boolean }>("disconnect_google_account", { account_id, confirmation });
  }
}
