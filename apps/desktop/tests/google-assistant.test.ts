import assert from "node:assert/strict";
import { test } from "node:test";
import { readFileSync } from "node:fs";
import { registerHooks } from "node:module";
import ts from "typescript";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { GoogleAssistantClient, googleError, confirmationPhrase, eventChanges, readPermissions, writePermissions, type PreparedGoogleMutation } from "../src/google-assistant.ts";
registerHooks({ load(url, context, next) {
  if (url.endsWith(".css")) return { format: "module", shortCircuit: true, source: "export default '';" };
  if (url.endsWith(".tsx")) return { format: "module", shortCircuit: true, source: ts.transpileModule(readFileSync(new URL(url), "utf8"), {
    compilerOptions: { jsx: ts.JsxEmit.ReactJSX, module: ts.ModuleKind.ESNext, target: ts.ScriptTarget.ES2020 },
  }).outputText };
  return next(url, context);
} });
const { default: GoogleAssistant } = await import("../src/GoogleAssistant.tsx");
Object.assign(globalThis, { window: globalThis, isTauri: false });

const account = "00000000-0000-4000-8000-000000000001";
function prepared(): PreparedGoogleMutation { return { version: 1, request_id: "synthetic-request", account_id: account, required_permission: "mail_send",
  payload: { action: "send_mail", mail: { to: ["synthetic@example.invalid"], cc: [], subject: "Synthetic subject", body: "Complete synthetic body" } },
  preview: { account_label: "Synthetic account", sender: null, mail: { to: ["synthetic@example.invalid"], cc: [], subject: "Synthetic subject", body: "Complete synthetic body" }, old_event: null, new_event: null, body_bytes: 23 },
  created_at: 1, expires_at: 300, context_sha256: "a".repeat(64), request_sha256: "b".repeat(64) }; }

test("static browser Google UI is unavailable and performs no IPC", async () => {
  let calls = 0; const client = new GoogleAssistantClient(() => false, async <T>() => { calls++; return {} as T; });
  const html = renderToStaticMarkup(React.createElement(GoogleAssistant, { client }));
  assert.match(html, /unavailable in browser preview/); assert.match(html, /disabled/);
  await assert.rejects(client.status()); await assert.rejects(client.search(account, "safe"));
  await assert.rejects(client.prepare(account, prepared().payload)); assert.equal(calls, 0);
});
test("read permissions are default and write capabilities require selection", () => {
  assert.deepEqual(readPermissions, ["mail_read", "calendar_read", "contacts_read"]);
  assert.equal(readPermissions.some(p => writePermissions.includes(p)), false);
  const html = renderToStaticMarkup(React.createElement(GoogleAssistant));
  assert.match(html, /Write capabilities — opt in/); assert.match(html, /Review Google permissions/);
  assert.match(html, /Mail send/); assert.doesNotMatch(html, /Open Google consent/);
});
test("config and reads use fixed typed IPC commands", async () => {
  const calls: [string, unknown][] = [];
  const client = new GoogleAssistantClient(() => true, async <T>(command: string, args?: Record<string, unknown>) => { calls.push([command, args]); return {} as T; });
  await client.saveClient("synthetic.apps.googleusercontent.com"); await client.search(account, "in:inbox");
  await client.agenda(account, "2026-10-10T00:00:00Z", "2026-10-11T00:00:00Z"); await client.freeTime(account, "start", "end", 30); await client.contacts(account, "synthetic");
  assert.deepEqual(calls.map(c => c[0]), ["save_google_client_id", "search_google_mail", "list_google_agenda", "find_google_free_time", "lookup_google_contacts"]);
  for (const [, args] of calls) assert.doesNotMatch(JSON.stringify(args), /endpoint|client_secret|access_token|refresh_token|Authorization/);
});
test("connect requires explicit reviewed permissions and never opens a frontend URL", async () => {
  const calls: unknown[] = []; const client = new GoogleAssistantClient(() => true, async <T>(command: string, args?: Record<string, unknown>) => { calls.push([command, args]); return {} as T; });
  await assert.rejects(client.connect("Synthetic", ["mail_read"], false)); assert.equal(calls.length, 0);
  await client.connect("Synthetic", ["mail_read"], true);
  assert.deepEqual(calls, [["connect_google", { input: { display_label: "Synthetic", requested_permissions: ["mail_read"], confirmed: true } }]]);
});
test("preparation is inert and confirmation sends the exact immutable reviewed payload once", async () => {
  const calls: string[] = []; const value = prepared();
  const client = new GoogleAssistantClient(() => true, async <T>(command: string, args?: Record<string, unknown>) => {
    calls.push(command); if (command === "prepare_google_mutation") return value as T;
    assert.deepEqual(args, { input: { prepared: value, confirmation: confirmationPhrase(value) } }); return { operation: "send_mail", provider_id: "abc123", audit_recorded: true } as T;
  });
  const preview = await client.prepare(account, value.payload);
  assert.deepEqual(calls, ["prepare_google_mutation"]); assert.ok(Object.isFrozen(preview)); assert.ok(Object.isFrozen(preview.payload));
  await assert.rejects(client.execute(preview, "SEND"));
  await client.execute(preview, confirmationPhrase(preview)); await assert.rejects(client.execute(preview, confirmationPhrase(preview)));
  assert.deepEqual(calls, ["prepare_google_mutation", "execute_google_mutation"]);
});
test("editing or changing account invalidates a Google preview", async () => {
  let executions = 0; const value = prepared(); const client = new GoogleAssistantClient(() => true, async <T>(command: string) => { if (command === "execute_google_mutation") executions++; return value as T; });
  const preview = await client.prepare(account, value.payload); client.invalidate(); await assert.rejects(client.execute(preview, confirmationPhrase(preview))); assert.equal(executions, 0);
});
test("late preparation cannot restore an invalidated preview", async () => {
  let resolve: ((value: unknown) => void) | undefined;
  const client = new GoogleAssistantClient(() => true, <T>() => new Promise<T>(r => { resolve = r as (value: unknown) => void; }));
  const promise = client.prepare(account, prepared().payload); client.invalidate(); resolve!(prepared()); await assert.rejects(promise);
});
test("failed send cannot reuse the preview and does not retry", async () => {
  let sends = 0; const client = new GoogleAssistantClient(() => true, async <T>(command: string) => { if (command === "execute_google_mutation") { sends++; throw "transport_failed"; } return prepared() as T; });
  const preview = await client.prepare(account, prepared().payload); await assert.rejects(client.execute(preview, confirmationPhrase(preview))); await assert.rejects(client.execute(preview, confirmationPhrase(preview))); assert.equal(sends, 1);
});
test("event diff preserves unchanged fields and makes explicit clearing visible", () => {
  const old = { summary: "Synthetic", description: "Keep unless explicitly cleared", location: "Keep", start: { date: "2026-10-10" }, end: { date: "2026-10-11" } };
  assert.deepEqual(eventChanges(old, { ...old, summary: "Updated" }), { summary: "Updated", description: null, location: null, start: null, end: null });
  assert.equal(eventChanges(old, { ...old, description: null }).description, "");
});
test("provider errors are replaced with stable feedback and uncertain writes require checking Google", () => {
  assert.doesNotMatch(googleError("raw provider body synthetic-secret"), /synthetic-secret|raw provider/);
  assert.match(googleError("transport_failed"), /Check Google|check Google/); assert.match(googleError("conflict"), /fresh preview/);
});
test("disconnect has a separate exact confirmation and no automatic provider revocation", async () => {
  const calls: unknown[] = []; const client = new GoogleAssistantClient(() => true, async <T>(command: string, args?: Record<string, unknown>) => { calls.push([command, args]); return {} as T; });
  await assert.rejects(client.disconnect(account, "DISCONNECT")); assert.equal(calls.length, 0);
  await client.disconnect(account, `DISCONNECT ${account}`); assert.equal((calls[0] as unknown[])[0], "disconnect_google_account");
});
test("Assistant shows Google privacy disclosures and has no browser networking or persistence", () => {
  const html = renderToStaticMarkup(React.createElement(GoogleAssistant));
  for (const text of [/requests go to Google/, /never sends their content to OpenAI/, /complete plain-text body/i, /primary calendar/, /Contacts — read only/]) assert.match(html, text);
  const source = ["GoogleAssistant.tsx", "google-assistant.ts"].map(name => readFileSync(new URL(`../src/${name}`, import.meta.url), "utf8")).join("\n");
  assert.doesNotMatch(source, /fetch\(|XMLHttpRequest|localStorage|sessionStorage|window\.open|dangerouslySetInnerHTML/);
  assert.doesNotMatch(source, /code_verifier|refresh_token:|access_token:/);
});
