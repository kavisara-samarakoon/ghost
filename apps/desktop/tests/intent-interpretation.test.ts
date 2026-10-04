import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { registerHooks } from "node:module";
import { afterEach, test } from "node:test";
import { createElement, type ReactElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import ts from "typescript";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { IntentController, MAX_INTENT_BYTES, desktopIntentEnvironment, intentError, prepareIntent, interpretIntent, saveIntentPlan,
  type PreparedIntent, type IntentProposal, type IntentEnvironment, type IntentState, type SavedPlan } from "../src/intent-interpretation.ts";

registerHooks({ load(url, context, next) {
  if (url.endsWith(".tsx")) return { format: "module", shortCircuit: true, source: ts.transpileModule(readFileSync(new URL(url), "utf8"), {
    compilerOptions: { jsx: ts.JsxEmit.ReactJSX, module: ts.ModuleKind.ESNext, target: ts.ScriptTarget.ES2020 },
  }).outputText };
  return next(url, context);
} });
const { IntentView } = await import("../src/IntentInterpreter.tsx");
const { VoiceView } = await import("../src/VoiceInput.tsx");
Object.assign(globalThis, { window: globalThis, isTauri: false });
afterEach(() => { clearMocks(); Object.assign(globalThis, { isTauri: false }); });
const prepared: PreparedIntent = { version: 1, project_alias: "example", intent: "Sanitized [REDACTED] intent", model: "gpt-6.1-sol", schema_version: 1,
  request_sha256: "a".repeat(64), safety_notice: "Only the reviewed intent leaves your Mac." };
const proposal: IntentProposal = { kind: "plan", summary: "Inert proposed steps", steps: [
  { action: "start_session", goal: "Review login" }, { action: "add_session_note", note: "Recorded progress" },
  { action: "generate_next_steps" }, { action: "create_handoff", provider: "codex" }], project_alias: "example", model: "gpt-6.1-sol",
  request_sha256: prepared.request_sha256, proposal_sha256: "b".repeat(64), audit_recorded: true };
const saved: SavedPlan = { path: "/isolated/intent-plans/test.json", plan_sha256: "c".repeat(64), audit_recorded: true };
function fixture(available = true) {
  const calls: { name: string; value: unknown }[] = [];
  const env: IntentEnvironment = { available,
    prepare: async (alias, text) => { calls.push({ name: "prepare", value: { alias, text } }); return structuredClone(prepared); },
    interpret: async value => { calls.push({ name: "send", value }); return structuredClone(proposal); },
    save: async value => { calls.push({ name: "save", value }); return structuredClone(saved); } };
  return { calls, env, controller: new IntentController(env, "example") };
}
async function review(f: ReturnType<typeof fixture>) { f.controller.edit("Raw human intent"); await f.controller.prepare(); }
async function propose(f: ReturnType<typeof fixture>) { await review(f); await f.controller.send(); }
function deferred<T>() { let resolve!: (value: T) => void; let reject!: (reason: unknown) => void; const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; }); return { promise, resolve, reject }; }
const noop = () => assert.fail("Rendering must not invoke actions");
function html(state: IntentState) { return renderToStaticMarkup(createElement(IntentView, { state, onEdit: noop, onPrepare: noop, onSend: noop, onSave: noop, onDiscard: noop })); }

test("static mode prevents all prepare send save calls and shows unavailable", async () => {
  mockIPC(() => assert.fail("Static preview must not invoke IPC"));
  await assert.rejects(prepareIntent("example", "intent")); await assert.rejects(interpretIntent(prepared)); await assert.rejects(saveIntentPlan(proposal));
  const f = fixture(false); f.controller.edit("intent"); await f.controller.prepare(); await f.controller.send(); await f.controller.save();
  assert.equal(f.calls.length, 0); assert.equal(f.controller.getState().phase, "unavailable");
  assert.match(html(f.controller.getState()), /unavailable in static preview/);
  assert.equal(desktopIntentEnvironment(true).available, false);
});
test("typing is entirely local until Prepare, which calls offline preparation only", async () => {
  const f = fixture(); f.controller.edit("typed intent"); assert.deepEqual(f.calls, []);
  await f.controller.prepare(); assert.deepEqual(f.calls.map(call => call.name), ["prepare"]);
  assert.equal(f.controller.getState().prepared?.intent, prepared.intent); assert.equal(f.controller.getState().phase, "review");
});
test("prepared review displays exact sanitized intent model alias and full SHA literally", async () => {
  const f = fixture(); await review(f); const output = html(f.controller.getState());
  assert.match(output, /Sanitized \[REDACTED\] intent/); assert.ok(!output.includes("Raw human intent"));
  assert.ok(output.includes(prepared.request_sha256)); assert.match(output, /gpt-6.1-sol/); assert.match(output, /Bound project/);
  assert.match(output, /readonly/i); assert.match(output, /Send reviewed intent to OpenAI/);
});
test("editing invalidates prepared request and requires Prepare again before Send", async () => {
  const f = fixture(); await review(f); f.controller.edit("changed");
  assert.equal(f.controller.getState().prepared, null); await f.controller.send(); assert.deepEqual(f.calls.map(call => call.name), ["prepare"]);
});
test("project selection change invalidates reviewed request and pending preparation", async () => {
  const f = fixture(); await review(f); f.controller.setProject("another");
  assert.equal(f.controller.getState().prepared, null); await f.controller.send(); assert.equal(f.calls.length, 1);
  const pending = deferred<PreparedIntent>(); f.env.prepare = () => pending.promise; const preparing = f.controller.prepare();
  f.controller.setProject("third"); pending.resolve(prepared); await preparing; assert.equal(f.controller.getState().prepared, null); assert.equal(f.controller.getState().projectAlias, "third");
});
test("send is impossible before Prepare and never saves a proposal automatically", async () => {
  const f = fixture(); f.controller.edit("draft"); await f.controller.send(); assert.deepEqual(f.calls, []);
  await review(f); await f.controller.send(); assert.deepEqual(f.calls.map(call => call.name), ["prepare", "send"]);
  assert.equal(f.controller.getState().saved, null);
});
test("double Send invokes interpretation once and sends exactly frozen reviewed object", async () => {
  const f = fixture(); await review(f); const gate = deferred<IntentProposal>(); let count = 0;
  f.env.interpret = async value => { count++; assert.deepEqual(value, prepared); assert.ok(Object.isFrozen(value)); return gate.promise; };
  const first = f.controller.send(); const second = f.controller.send(); await second; assert.equal(count, 1);
  gate.resolve(proposal); await first; await f.controller.send(); assert.equal(count, 1);
});
test("project changes during send cannot rebind the returned proposal", async () => {
  const f = fixture(); await review(f); const gate = deferred<IntentProposal>(); f.env.interpret = () => gate.promise;
  const sending = f.controller.send(); f.controller.setProject("another"); gate.resolve(proposal); await sending;
  assert.equal(f.controller.getState().proposal?.project_alias, "example"); assert.match(html(f.controller.getState()), /example/);
  await f.controller.save(); assert.equal((f.calls.at(-1)!.value as IntentProposal).project_alias, "example");
});
test("transport error after a project change requires fresh preparation", async () => {
  const f = fixture(); await review(f); const gate = deferred<IntentProposal>(); f.env.interpret = () => gate.promise;
  const sending = f.controller.send(); f.controller.setProject("another"); gate.reject("transport"); await sending;
  assert.equal(f.controller.getState().phase, "draft"); assert.equal(f.controller.getState().prepared, null);
});
test("interpretation error never echoes raw native details and never retries", async () => {
  const f = fixture(); await review(f); let count = 0;
  f.env.interpret = async () => { count++; throw "PRIVATE_NATIVE_SECRET_PATH"; }; await f.controller.send();
  assert.equal(count, 1); assert.ok(!f.controller.getState().message!.includes("PRIVATE_NATIVE"));
  assert.match(intentError("transport"), /may have reached the provider.*did not retry automatically/);
  assert.ok(!intentError(new Error("PRIVATE_NATIVE_SECRET")).includes("PRIVATE_NATIVE"));
});
for (const category of ["unavailable", "invalid_intent", "changed_review", "credential", "audit", "service", "transport", "response", "proposal", "busy", "save"]) {
  test(`safe error category ${category} has a useful generic message`, () => assert.ok(intentError(category).length > 20));
}
test("plan renders every exact step and values literally with separate Save Plan Draft", async () => {
  const f = fixture(); await propose(f); const state = f.controller.getState();
  const unsafe = { ...proposal, summary: '<script>summary</script>', steps: [
    { action: "start_session" as const, goal: "<b>goal</b>" }, { action: "add_session_note" as const, note: "<i>note</i>" },
    { action: "generate_next_steps" as const }, { action: "create_handoff" as const, provider: "gemini" as const }] };
  const output = html({ ...state, proposal: unsafe }); assert.ok(!output.includes("<script>")); assert.match(output, /&lt;b&gt;goal/); assert.match(output, /&lt;i&gt;note/);
  for (const step of unsafe.steps) assert.ok(output.includes(step.action));
  assert.match(output, /4 steps/); assert.match(output, /AI-generated proposal — not executed/); assert.match(output, /Save Plan Draft/);
  assert.ok(!output.includes(">Run<")); assert.ok(!output.includes(">Execute<")); assert.ok(!output.includes(">Apply<")); assert.ok(!output.includes("Save Request"));
});
for (const kind of ["clarify", "unsupported"] as const) test(`${kind} is inert and cannot save`, async () => {
  const f = fixture(); f.env.interpret = async () => ({ ...proposal, kind, steps: [], summary: kind === "clarify" ? "Which provider?" : "Capability unsupported." });
  await propose(f); await f.controller.save(); assert.deepEqual(f.calls.map(call => call.name), ["prepare"]); // replaced fake interpretation has no logging
  assert.ok(!html(f.controller.getState()).includes("Save Plan Draft")); assert.equal(f.controller.getState().saved, null);
});
test("Save is separate, double Save does not duplicate, and saved path/hash remain visible", async () => {
  const f = fixture(); await propose(f); const gate = deferred<SavedPlan>(); let count = 0;
  f.env.save = async value => { count++; assert.deepEqual(value, proposal); return gate.promise; };
  const first = f.controller.save(); await f.controller.save(); assert.equal(count, 1); gate.resolve(saved); await first; await f.controller.save(); assert.equal(count, 1);
  const output = html(f.controller.getState()); assert.ok(output.includes(saved.path)); assert.ok(output.includes(saved.plan_sha256));
  assert.match(output, /Plan draft only. No workflow action was performed/); assert.match(output, /different from M34/); assert.match(output, /ghost orchestrate/);
});
test("Save cannot run while interpretation is in flight", async () => {
  const f = fixture(); await review(f); const gate = deferred<IntentProposal>(); f.env.interpret = () => gate.promise;
  const sending = f.controller.send(); await f.controller.save(); assert.ok(!f.calls.some(call => call.name === "save")); gate.resolve(proposal); await sending;
});
test("audit failure preserves both proposal and saved path with explicit warnings", async () => {
  const f = fixture(); f.env.interpret = async () => ({ ...proposal, audit_recorded: false }); await propose(f);
  assert.equal(f.controller.getState().proposal?.steps.length, 4); assert.match(f.controller.getState().message!, /completion audit/);
  f.env.save = async () => ({ ...saved, audit_recorded: false }); await f.controller.save();
  assert.equal(f.controller.getState().saved?.path, saved.path); assert.match(f.controller.getState().message!, /do not create a duplicate/);
});
test("save and prepare errors never echo private values", async () => {
  const f = fixture(); f.env.prepare = async () => { throw "PRIVATE_INTENT_PATH"; }; await review(f);
  assert.ok(!f.controller.getState().message!.includes("PRIVATE"));
  f.env.prepare = async () => prepared; await propose(f); f.env.save = async () => { throw "PRIVATE_PLAN_PATH"; }; await f.controller.save(); assert.ok(!f.controller.getState().message!.includes("PRIVATE"));
});
test("explicit transcript use copies locally and still needs Prepare then Send", async () => {
  const f = fixture(); assert.equal(f.controller.getState().text, "");
  f.controller.useTranscript("Visible reviewed transcript"); assert.equal(f.controller.getState().text, "Visible reviewed transcript"); assert.deepEqual(f.calls, []);
  await f.controller.send(); assert.deepEqual(f.calls, []); await f.controller.prepare(); assert.deepEqual(f.calls.map(call => call.name), ["prepare"]);
  await f.controller.send(); assert.deepEqual(f.calls.map(call => call.name), ["prepare", "send"]);
});
test("transcript display never copies or invokes IPC; only explicit button click calls local callback", () => {
  const f = fixture(); let callbacks = 0;
  const element = VoiceView({ state: { phase: "transcript", elapsedMs: 0, recording: null, result: { text: "Visible transcript", model: "gpt-transcribe", audio_bytes: 3, duration_ms: 1000, audit_recorded: true }, message: null },
    onStart: noop, onStop: noop, onSend: noop, onDiscard: noop, onUseTranscript: text => { callbacks++; f.controller.useTranscript(text); } });
  assert.equal(callbacks, 0); assert.equal(f.controller.getState().text, "");
  const walk = (node: unknown): ReactElement<{ children?: unknown; onClick?: () => void }> | undefined => {
    if (Array.isArray(node)) { for (const child of node) { const found = walk(child); if (found) return found; } }
    if (node && typeof node === "object" && "props" in node) { const el = node as ReactElement<{ children?: unknown; onClick?: () => void }>; if (el.props.children === "Use transcript as intent") return el; return walk(el.props.children); }
  };
  const button = walk(element); assert.ok(button); button.props.onClick!(); assert.equal(callbacks, 1); assert.equal(f.controller.getState().text, "Visible transcript"); assert.deepEqual(f.calls, []);
});
test("a new transcript does not replace review until explicitly selected and no copy is automatic", async () => {
  const f = fixture(); await review(f); const prior = f.controller.getState().prepared;
  // Arrival/display is independent from the intent controller. Only useTranscript is the transfer gate.
  html(f.controller.getState()); assert.equal(f.controller.getState().prepared, prior);
  f.controller.useTranscript("New explicitly selected transcript"); assert.equal(f.controller.getState().prepared, null); assert.equal(f.calls.length, 1);
});
test("review and proposal are immutable and outputs cannot silently mutate frozen state", async () => {
  const f = fixture(); const mutable = structuredClone(prepared); f.env.prepare = async () => mutable; await review(f);
  mutable.intent = "mutated"; assert.equal(f.controller.getState().prepared?.intent, prepared.intent);
  await f.controller.send(); assert.ok(Object.isFrozen(f.controller.getState().proposal?.steps));
});
test("disposal discards late preparation and interpretation results", async () => {
  const f = fixture(); f.controller.edit("draft"); const gate = deferred<PreparedIntent>(); f.env.prepare = () => gate.promise;
  const pending = f.controller.prepare(); f.controller.dispose(); gate.resolve(prepared); await pending; assert.equal(f.controller.getState().prepared, null);
});
test("IPC wrappers expose three fixed commands with no caller model endpoint schema or tools", async () => {
  Object.assign(globalThis, { isTauri: true }); const calls: { name: string; args: unknown }[] = [];
  mockIPC((name, args) => { calls.push({ name, args }); return name === "prepare_ghost_intent" ? prepared : name === "interpret_ghost_intent" ? proposal : saved; });
  await prepareIntent("example", "input"); await interpretIntent(prepared); await saveIntentPlan(proposal);
  assert.deepEqual(calls, [{ name: "prepare_ghost_intent", args: { project_alias: "example", intent: "input" } },
    { name: "interpret_ghost_intent", args: { prepared, confirmed: true } }, { name: "save_ghost_intent_plan", args: { proposal, confirmed: true } }]);
});
test("frontend bounds reject invalid inputs without IPC", async () => {
  Object.assign(globalThis, { isTauri: true }); mockIPC(() => assert.fail("Invalid intent must not invoke"));
  for (const text of ["", " \n", "x".repeat(MAX_INTENT_BYTES + 1), "é".repeat(MAX_INTENT_BYTES / 2 + 1)]) await assert.rejects(prepareIntent("example", text));
  for (const alias of ["", "../alias", "UPPER"]) await assert.rejects(prepareIntent(alias, "input"));
  await assert.rejects(saveIntentPlan({ ...proposal, kind: "clarify", steps: [] }));
});
test("intent source has no workflow Action Request orchestration or networking bridge", () => {
  for (const name of ["IntentInterpreter.tsx", "intent-interpretation.ts"]) {
    const source = readFileSync(new URL(`../src/${name}`, import.meta.url), "utf8");
    for (const forbidden of ["prepareActionRequest", "saveActionRequest", "fetch(", "localStorage", "indexedDB", "dangerouslySetInnerHTML", "OPENAI_API_KEY", "api.openai.com", "dispatch_local", "orchestrate_run"]) assert.ok(!source.includes(forbidden), forbidden);
  }
});
test("Tauri input permission is limited to four consent-gated main-window commands", () => {
  const capability = JSON.parse(readFileSync(new URL("../src-tauri/capabilities/default.json", import.meta.url), "utf8"));
  assert.deepEqual(capability.windows, ["main"]); assert.equal(capability.remote, undefined);
  assert.ok(capability.permissions.includes("ghost-controlled-input"));
  const permission = readFileSync(new URL("../src-tauri/permissions/ghost-controlled-input.toml", import.meta.url), "utf8");
  const commands = JSON.parse(permission.split("commands.allow = ")[1].trim());
  assert.deepEqual(commands, ["transcribe_ghost_voice", "prepare_ghost_intent", "interpret_ghost_intent", "save_ghost_intent_plan"]);
});
