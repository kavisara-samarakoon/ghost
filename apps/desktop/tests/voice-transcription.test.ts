import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { registerHooks } from "node:module";
import { afterEach, test } from "node:test";
import { Children, createElement, isValidElement, type ReactElement, type ReactNode } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import ts from "typescript";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { MAX_AUDIO_BYTES, MAX_DURATION_MS, MAX_FINALIZATION_MS, VOICE_MIMES, VoiceController, browserVoiceEnvironment,
  selectVoiceMime, transcribeRecording, voiceAvailable, voiceError, type Recording, type VoiceEnvironment, type VoiceResult, type VoiceLifecycle } from "../src/voice-transcription.ts";
import { JarvisClient, adoptTranscript } from "../src/jarvis/jarvis.ts";

registerHooks({ load(url, context, next) {
  if (url.endsWith(".tsx")) return { format: "module", shortCircuit: true, source: ts.transpileModule(readFileSync(new URL(url), "utf8"), {
    compilerOptions: { jsx: ts.JsxEmit.ReactJSX, module: ts.ModuleKind.ESNext, target: ts.ScriptTarget.ES2020 },
  }).outputText };
  return next(url, context);
} });
const { VoiceView } = await import("../src/VoiceInput.tsx");
Object.assign(globalThis, { window: globalThis, isTauri: false });
afterEach(() => { clearMocks(); Object.assign(globalThis, { isTauri: false }); });
const result: VoiceResult = { text: "Reviewed transcript", model: "gpt-transcribe", audio_bytes: 3, duration_ms: 1200, audit_recorded: true };
function clip(): Recording { return { blob: new Blob([new Uint8Array([1, 2, 3])]), mime: "audio/webm", durationMs: 1200, url: "blob:local-only" }; }
class FakeRecorder {
  state = "inactive"; mimeType = "audio/webm";
  ondataavailable: ((event: { data: Blob }) => void) | null = null;
  onstop: (() => void) | null = null;
  onerror: (() => void) | null = null;
  starts = 0; stops = 0;
  start() { this.state = "recording"; this.starts++; }
  stop() { this.state = "inactive"; this.stops++; this.ondataavailable?.({ data: new Blob([new Uint8Array([1, 2, 3])]) }); this.onstop?.(); }
}
function fixture(lifecycle?: VoiceLifecycle) {
  let now = 0; let nextTimer = 0; let microphoneCalls = 0;
  const tracks = [{ stops: 0, onended: null as null | (() => void), onmute: null as null | (() => void), stop() { this.stops++; } }];
  const stream = { getTracks: () => tracks } as unknown as MediaStream;
  const recorders: FakeRecorder[] = []; const sent: Recording[] = []; const revoked: string[] = [];
  const timers = new Map<number, { fn: () => void; ms: number }>();
  const env: VoiceEnvironment = {
    available: true, mime: "audio/webm", microphone: async () => { microphoneCalls++; return stream; },
    recorder: () => { const recorder = new FakeRecorder(); recorders.push(recorder); return recorder as unknown as MediaRecorder; },
    createURL: () => `blob:local-${recorders.length}`, revokeURL: url => { revoked.push(url); }, now: () => now,
    timeout: (fn, ms) => { timers.set(++nextTimer, { fn, ms }); return nextTimer as unknown as ReturnType<typeof setTimeout>; },
    interval: (fn, ms) => { timers.set(++nextTimer, { fn, ms }); return nextTimer as unknown as ReturnType<typeof setInterval>; },
    clearTimeout: id => { timers.delete(id as unknown as number); }, clearInterval: id => { timers.delete(id as unknown as number); },
    transcribe: async recording => { sent.push(recording); return result; },
    lifecycle,
  };
  const controller = new VoiceController(env);
  return { env, controller, tracks, recorders, sent, timers, revoked, stream,
    setTime(value: number) { now = value; }, microphoneCalls: () => microphoneCalls };
}
async function review(f: ReturnType<typeof fixture>) { await f.controller.start(); f.setTime(1200); f.controller.stop(); assert.equal(f.controller.getState().phase, "review"); }

function lifecycleEnvironment() {
  let foreground = true; let nativeCallback: (() => void) | undefined; let nativeRemovals = 0;
  const listeners = new Map<string, () => void>();
  const lifecycle: VoiceLifecycle = {
    foreground: () => foreground,
    listen: (event, callback) => { listeners.set(event, callback); return () => { listeners.delete(event); }; },
    nativeBlur: async callback => { nativeCallback = callback; return () => { nativeRemovals++; }; },
  };
  return { lifecycle, listeners, nativeRemovals: () => nativeRemovals,
    foreground(value: boolean) { foreground = value; }, emit(event: "visibilitychange" | "blur" | "pagehide") { listeners.get(event)?.(); },
    nativeBlur() { nativeCallback?.(); } };
}
async function stallFinalization(f: ReturnType<typeof fixture>) {
  await f.controller.start(); const recorder = f.recorders[0];
  recorder.stop = () => { recorder.state = "inactive"; recorder.stops++; };
  f.setTime(1200); f.controller.stop(); assert.equal(f.controller.getState().phase, "stopping");
  return recorder;
}
function capturedEvents(recorder: FakeRecorder) {
  return { data: recorder.ondataavailable!, stop: recorder.onstop!, error: recorder.onerror! };
}

for (const signal of ["visibilitychange", "blur", "pagehide", "native-minimize"] as const) test(`${signal} discards foreground recording without sending or restarting`, async () => {
  const l = lifecycleEnvironment(); const f = fixture(l.lifecycle);
  assert.equal(f.microphoneCalls(), 0); assert.equal(l.listeners.size, 3);
  await f.controller.start(); const stale = capturedEvents(f.recorders[0]);
  f.recorders[0].ondataavailable!({ data: new Blob([new Uint8Array([1, 2])]) });
  l.foreground(false); signal === "native-minimize" ? l.nativeBlur() : l.emit(signal);
  assert.equal(f.controller.getState().phase, "idle"); assert.equal(f.controller.getState().recording, null);
  assert.ok(f.tracks[0].stops > 0); assert.equal(f.timers.size, 0); assert.deepEqual(f.sent, []);
  stale.data({ data: new Blob([new Uint8Array([3])]) }); stale.stop(); stale.error();
  assert.equal(f.controller.getState().phase, "idle");
  await f.controller.start(); assert.equal(f.microphoneCalls(), 1); // Hidden Start is blocked.
  l.foreground(true); l.emit("visibilitychange"); assert.equal(f.microphoneCalls(), 1);
  f.controller.dispose(); assert.equal(l.listeners.size, 0); assert.equal(l.nativeRemovals(), 1);
});
for (const signal of ["visibilitychange", "native-minimize"] as const) test(`${signal} invalidates pending permission and stops late granted tracks`, async () => {
  const l = lifecycleEnvironment(); const f = fixture(l.lifecycle); let grant!: (stream: MediaStream) => void;
  f.env.microphone = () => new Promise(resolve => { grant = resolve; });
  const pending = f.controller.start(); l.foreground(false);
  signal === "native-minimize" ? l.nativeBlur() : l.emit(signal);
  grant(f.stream); await pending;
  assert.equal(f.controller.getState().phase, "idle"); assert.ok(f.tracks[0].stops > 0);
  assert.equal(f.recorders.length, 0); assert.equal(f.timers.size, 0); assert.deepEqual(f.sent, []);
  f.controller.dispose();
});
test("permission resolution rechecks foreground even if a visibility event was missed", async () => {
  const l = lifecycleEnvironment(); const f = fixture(l.lifecycle); let grant!: (stream: MediaStream) => void;
  f.env.microphone = () => new Promise(resolve => { grant = resolve; });
  const pending = f.controller.start(); l.foreground(false); grant(f.stream); await pending;
  assert.ok(f.tracks[0].stops > 0); assert.equal(f.recorders.length, 0); assert.equal(f.timers.size, 0); assert.deepEqual(f.sent, []);
  f.controller.dispose();
});
test("recording ticker catches foreground loss without starting provider work", async () => {
  const l = lifecycleEnvironment(); const f = fixture(l.lifecycle); await f.controller.start();
  l.foreground(false); [...f.timers.values()].find(timer => timer.ms === 100)!.fn();
  assert.equal(f.controller.getState().phase, "idle"); assert.ok(f.tracks[0].stops > 0); assert.equal(f.timers.size, 0); assert.deepEqual(f.sent, []);
  f.controller.dispose();
});
test("recorder finalization times out, discards partial bytes and rejects late events", async () => {
  const f = fixture(); const recorder = await stallFinalization(f); const stale = capturedEvents(recorder);
  stale.data({ data: new Blob([new Uint8Array([1, 2, 3])]) });
  assert.ok(f.tracks[0].stops > 0); assert.equal(f.timers.size, 1);
  [...f.timers.values()].find(timer => timer.ms === MAX_FINALIZATION_MS)!.fn();
  assert.equal(f.controller.getState().phase, "error"); assert.match(f.controller.getState().message!, /five seconds/);
  assert.equal(f.controller.getState().recording, null); assert.equal(f.timers.size, 0);
  stale.data({ data: new Blob([new Uint8Array([4])]) }); stale.stop(); stale.error(); await f.controller.send();
  assert.equal(f.controller.getState().phase, "error"); assert.deepEqual(f.sent, []);
  await f.controller.start(); assert.equal(f.controller.getState().phase, "recording"); f.controller.dispose();
});
test("late final output is rejected even if the finalization timeout callback was delayed", async () => {
  const f = fixture(); const recorder = await stallFinalization(f);
  recorder.ondataavailable!({ data: new Blob([new Uint8Array([1, 2, 3])]) });
  f.setTime(1200 + MAX_FINALIZATION_MS); recorder.onstop!();
  assert.equal(f.controller.getState().phase, "error"); assert.match(f.controller.getState().message!, /five seconds/);
  assert.equal(f.timers.size, 0); assert.deepEqual(f.sent, []);
});
for (const operation of ["discard", "dispose", "background"] as const) test(`${operation} while stopping cancels finalization and makes recorder callbacks inert`, async () => {
  const f = fixture(); const recorder = await stallFinalization(f); const stale = capturedEvents(recorder);
  const deadline = [...f.timers.values()][0].fn; f.controller[operation]();
  stale.data({ data: new Blob([new Uint8Array([1, 2, 3])]) }); stale.stop(); stale.error(); deadline();
  assert.equal(f.timers.size, 0); assert.equal(f.controller.getState().recording, null); assert.deepEqual(f.sent, []);
  assert.equal(f.controller.getState().phase, operation === "dispose" ? "unavailable" : "idle");
});
test("valid recorder output before the finalization deadline remains review-only", async () => {
  const f = fixture(); const recorder = await stallFinalization(f); const stale = capturedEvents(recorder);
  f.setTime(3000); stale.data({ data: new Blob([new Uint8Array([1, 2, 3])]) }); stale.stop();
  const reviewed = f.controller.getState().recording;
  assert.equal(f.controller.getState().phase, "review"); assert.equal(reviewed!.durationMs, 1200); assert.equal(f.timers.size, 0);
  stale.data({ data: new Blob([new Uint8Array([4])]) }); stale.stop(); stale.error();
  assert.equal(f.controller.getState().recording, reviewed); assert.deepEqual(f.sent, []);
  f.controller.dispose();
});
for (const event of ["onended", "onmute"] as const) test(`unexpected audio track ${event} discards capture and releases resources`, async () => {
  const f = fixture(); await f.controller.start(); const interrupted = f.tracks[0][event]!; const stale = capturedEvents(f.recorders[0]);
  interrupted(); assert.equal(f.controller.getState().phase, "error"); assert.match(f.controller.getState().message!, /interrupted/);
  assert.ok(f.tracks[0].stops > 0); assert.equal(f.tracks[0].onended, null); assert.equal(f.tracks[0].onmute, null);
  assert.equal(f.timers.size, 0); stale.stop(); stale.error(); await f.controller.send(); assert.deepEqual(f.sent, []);
  await f.controller.start(); interrupted(); assert.equal(f.controller.getState().phase, "recording"); f.controller.dispose();
});
test("already disconnected audio tracks are rejected before recorder construction", async () => {
  const f = fixture(); Object.assign(f.tracks[0], { readyState: "ended" }); await f.controller.start();
  assert.equal(f.controller.getState().phase, "error"); assert.ok(f.tracks[0].stops > 0);
  assert.equal(f.recorders.length, 0); assert.equal(f.timers.size, 0); assert.deepEqual(f.sent, []);
});
test("device disconnection is detected by the ticker even without an ended event", async () => {
  const f = fixture(); await f.controller.start(); Object.assign(f.tracks[0], { readyState: "ended" });
  [...f.timers.values()].find(timer => timer.ms === 100)!.fn();
  assert.equal(f.controller.getState().phase, "error"); assert.ok(f.tracks[0].stops > 0); assert.equal(f.timers.size, 0);
  assert.equal(f.controller.getState().recording, null); assert.deepEqual(f.sent, []);
});
test("recorder error during finalization clears the deadline and partial recording", async () => {
  const f = fixture(); const recorder = await stallFinalization(f); const stale = capturedEvents(recorder);
  stale.error(); stale.stop(); assert.equal(f.controller.getState().phase, "error"); assert.equal(f.timers.size, 0);
  assert.equal(f.controller.getState().recording, null); assert.deepEqual(f.sent, []);
});
test("unexpected recorder termination with partial bytes is not offered for Send", async () => {
  const f = fixture(); await f.controller.start();
  f.recorders[0].ondataavailable!({ data: new Blob([new Uint8Array([1, 2, 3])]) }); f.recorders[0].onstop!();
  assert.equal(f.controller.getState().phase, "error"); assert.equal(f.controller.getState().recording, null);
  assert.ok(f.tracks[0].stops > 0); assert.equal(f.timers.size, 0); await f.controller.send(); assert.deepEqual(f.sent, []);
});
test("repeated Start and Stop cycles cannot reuse callbacks, timers or playback URLs", async () => {
  const f = fixture(); const staleEvents: ReturnType<typeof capturedEvents>[] = []; const staleTimers: (() => void)[] = [];
  for (let cycle = 0; cycle < 3; cycle++) {
    f.setTime(cycle * 2000); await f.controller.start(); await f.controller.start();
    const snapshot = f.controller.getState(); staleEvents.forEach(events => { events.stop(); events.error(); }); staleTimers.forEach(timer => timer());
    assert.equal(f.controller.getState(), snapshot); assert.equal(f.timers.size, 2);
    staleEvents.push(capturedEvents(f.recorders[cycle])); staleTimers.push(...[...f.timers.values()].map(timer => timer.fn));
    f.setTime(cycle * 2000 + 1000); f.controller.stop(); f.controller.stop();
    assert.equal(f.controller.getState().phase, "review"); assert.equal(f.timers.size, 0);
  }
  assert.equal(f.microphoneCalls(), 3); assert.equal(f.recorders.length, 3); assert.equal(f.revoked.length, 2);
  assert.deepEqual(f.sent, []); f.controller.dispose(); assert.equal(f.revoked.length, 3);
});
test("completed local review survives foreground changes without automatic Send", async () => {
  const l = lifecycleEnvironment(); const f = fixture(l.lifecycle); await review(f); const recording = f.controller.getState().recording;
  l.foreground(false); l.emit("visibilitychange"); l.nativeBlur();
  assert.equal(f.controller.getState().recording, recording); assert.equal(f.controller.getState().phase, "review");
  assert.equal(f.timers.size, 0); assert.deepEqual(f.sent, []); f.controller.dispose();
});
test("Cancel followed by a new Start cannot let an old permission grant stop or replace the new capture", async () => {
  const f = fixture(); const grants: ((stream: MediaStream) => void)[] = [];
  const oldTrack = { stops: 0, stop() { this.stops++; } }; const newTrack = { stops: 0, stop() { this.stops++; } };
  const stream = (track: typeof oldTrack) => ({ getTracks: () => [track] }) as unknown as MediaStream;
  f.env.microphone = () => new Promise(resolve => { grants.push(resolve); });
  const oldStart = f.controller.start(); f.controller.discard(); const newStart = f.controller.start();
  grants[1](stream(newTrack)); await newStart; grants[0](stream(oldTrack)); await oldStart;
  assert.equal(oldTrack.stops, 1); assert.equal(newTrack.stops, 0); assert.equal(f.recorders.length, 1);
  assert.equal(f.controller.getState().phase, "recording"); assert.equal(f.timers.size, 2); assert.deepEqual(f.sent, []);
  f.controller.dispose(); assert.equal(newTrack.stops, 1); assert.equal(f.timers.size, 0);
});
test("native lifecycle registration failure cancels active capture and blocks future recording", async () => {
  const l = lifecycleEnvironment(); let reject!: (error: unknown) => void;
  l.lifecycle.nativeBlur = () => new Promise((_, fail) => { reject = fail; });
  const f = fixture(l.lifecycle); await f.controller.start(); reject(new Error("PRIVATE_NATIVE_ERROR"));
  await Promise.resolve(); await Promise.resolve();
  assert.equal(f.controller.getState().phase, "unavailable"); assert.ok(f.tracks[0].stops > 0); assert.equal(f.timers.size, 0);
  assert.match(f.controller.getState().message!, /window monitoring/); assert.doesNotMatch(f.controller.getState().message!, /PRIVATE/);
  await f.controller.start(); assert.equal(f.microphoneCalls(), 1); assert.deepEqual(f.sent, []);
  f.controller.dispose(); assert.equal(l.listeners.size, 0);
});
test("native listener resolving after disposal is immediately removed and late events stay inert", async () => {
  const l = lifecycleEnvironment(); let ready!: (cleanup: () => void) => void; let callback!: () => void; let removals = 0;
  l.lifecycle.nativeBlur = fn => { callback = fn; return new Promise(resolve => { ready = resolve; }); };
  const f = fixture(l.lifecycle); f.controller.dispose(); const disposed = f.controller.getState();
  ready(() => { removals++; }); await Promise.resolve(); callback(); f.controller.dispose();
  assert.equal(removals, 1); assert.equal(l.listeners.size, 0); assert.equal(f.controller.getState().phase, disposed.phase);
  assert.equal(f.microphoneCalls(), 0); assert.deepEqual(f.sent, []);
});
test("partial lifecycle listener setup failure blocks capture and cleans up registered listeners", async () => {
  const l = lifecycleEnvironment(); const subscribe = l.lifecycle.listen;
  l.lifecycle.listen = (event, callback) => { if (event === "blur") throw new Error("PRIVATE_LISTENER_ERROR"); return subscribe(event, callback); };
  const f = fixture(l.lifecycle); await f.controller.start();
  assert.equal(f.controller.getState().phase, "unavailable"); assert.equal(f.microphoneCalls(), 0); assert.equal(l.listeners.size, 1);
  f.controller.dispose(); assert.equal(l.listeners.size, 0); assert.deepEqual(f.sent, []);
});
test("unavailable static capture installs no lifecycle listeners or native subscriptions", () => {
  const f = fixture(); const lifecycle: VoiceLifecycle = { foreground: () => false, listen: () => { assert.fail("Static capture must not install listeners"); }, nativeBlur: async () => { assert.fail("Static capture must not invoke native APIs"); } };
  const controller = new VoiceController({ ...f.env, available: false, lifecycle }); controller.dispose();
  assert.equal(f.microphoneCalls(), 0); assert.deepEqual(f.sent, []);
});
test("browser lifecycle observes document visibility and the fixed native main-window blur event", async () => {
  const keys = ["document", "navigator", "MediaRecorder", "addEventListener", "removeEventListener"] as const;
  const previous = new Map(keys.map(key => [key, Object.getOwnPropertyDescriptor(globalThis, key)]));
  const f = fixture(); const doc = new EventTarget(); const win = new EventTarget(); let visible = true; let focused = true;
  const calls: { name: string; args: any }[] = []; let controller: VoiceController | undefined;
  class SupportedRecorder extends FakeRecorder { static isTypeSupported(mime: string) { return mime === "audio/webm"; } }
  try {
    Object.defineProperties(doc, { visibilityState: { get: () => visible ? "visible" : "hidden" }, hasFocus: { value: () => focused } });
    const values = { document: doc, navigator: { mediaDevices: { getUserMedia: async () => f.stream } }, MediaRecorder: SupportedRecorder,
      addEventListener: win.addEventListener.bind(win), removeEventListener: win.removeEventListener.bind(win) };
    keys.forEach(key => Object.defineProperty(globalThis, key, { configurable: true, value: values[key] }));
    Object.assign(globalThis, { isTauri: true });
    mockIPC((name, args) => { calls.push({ name, args }); assert.ok(["plugin:event|listen", "plugin:event|unlisten"].includes(name), "Lifecycle must not invoke transcription, planning or execution"); return (args as any).handler ?? null; });
    const browser = browserVoiceEnvironment(); assert.ok(browser.lifecycle); assert.equal(browser.lifecycle.foreground(), true);
    controller = new VoiceController({ ...f.env, lifecycle: browser.lifecycle });
    await Promise.resolve(); await Promise.resolve();
    assert.equal(f.microphoneCalls(), 0); assert.equal(calls[0].name, "plugin:event|listen");
    assert.equal(calls[0].args.event, "tauri://blur"); assert.deepEqual(calls[0].args.target, { kind: "Window", label: "main" });
    await controller.start();
    (globalThis as any).__TAURI_INTERNALS__.runCallback(calls[0].args.handler, { event: "tauri://blur", payload: null });
    assert.equal(controller.getState().phase, "idle"); assert.ok(f.tracks[0].stops > 0); assert.equal(f.timers.size, 0);
    await controller.start(); visible = false; doc.dispatchEvent(new Event("visibilitychange"));
    assert.equal(controller.getState().phase, "idle"); visible = true; focused = false;
    assert.equal(browser.lifecycle.foreground(), false); focused = true;
    await controller.start(); win.dispatchEvent(new Event("blur")); assert.equal(controller.getState().phase, "idle");
    controller.dispose(); await Promise.resolve(); await Promise.resolve();
    assert.deepEqual(calls.map(call => call.name), ["plugin:event|listen", "plugin:event|unlisten"]); assert.deepEqual(f.sent, []);
  } finally {
    controller?.dispose();
    keys.forEach(key => { const descriptor = previous.get(key); if (descriptor) Object.defineProperty(globalThis, key, descriptor); else delete (globalThis as any)[key]; });
  }
});
test("failed async native unlisten cannot revive capture or leak an unhandled rejection", async () => {
  const l = lifecycleEnvironment(); let callback!: () => void;
  l.lifecycle.nativeBlur = async fn => { callback = fn; return async () => { throw new Error("PRIVATE_UNLISTEN_ERROR"); }; };
  const f = fixture(l.lifecycle); await f.controller.start(); f.controller.dispose(); callback();
  await Promise.resolve(); await Promise.resolve();
  assert.equal(f.controller.getState().phase, "unavailable"); assert.equal(l.listeners.size, 0); assert.equal(f.timers.size, 0);
  assert.ok(f.tracks[0].stops > 0); assert.deepEqual(f.sent, []);
});
test("synchronous recorder failure or cancellation during state notification cannot arm capture timers", async () => {
  for (const failure of ["recorder", "recording", "stopping"] as const) {
    const f = fixture();
    if (failure === "recorder") f.env.recorder = () => {
      const recorder = new FakeRecorder(); f.recorders.push(recorder);
      recorder.start = () => { recorder.state = "recording"; recorder.onerror!(); };
      return recorder as unknown as MediaRecorder;
    };
    else f.controller.subscribe(() => { if (f.controller.getState().phase === failure) f.controller.discard(); });
    await f.controller.start(); if (failure === "stopping") f.controller.stop();
    assert.equal(f.timers.size, 0); assert.ok(f.tracks[0].stops > 0); assert.deepEqual(f.sent, []);
    f.controller.dispose();
  }
});
test("cancellation during permission-state notification prevents even a microphone request", async () => {
  const f = fixture(); f.controller.subscribe(() => { if (f.controller.getState().phase === "requesting") f.controller.discard(); });
  await f.controller.start(); assert.equal(f.microphoneCalls(), 0); assert.equal(f.controller.getState().phase, "idle");
  assert.equal(f.recorders.length, 0); assert.equal(f.timers.size, 0); assert.deepEqual(f.sent, []);
});

function voiceButtons(node: ReactNode): ReactElement<Record<string, any>>[] {
  const found: ReactElement<Record<string, any>>[] = [];
  Children.forEach(node, child => {
    if (!isValidElement<Record<string, any>>(child)) return;
    if (child.type === "button") found.push(child);
    found.push(...voiceButtons(child.props.children));
  });
  return found;
}

// Exercise the production component's mount effect and cleanup without a DOM dependency.
function voiceMountHarness(env: VoiceEnvironment) {
  const source = readFileSync(new URL("../src/VoiceInput.tsx", import.meta.url), "utf8");
  const ast = ts.createSourceFile("VoiceInput.tsx", source, ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
  const program = ast.statements.filter(node => !ts.isImportDeclaration(node)).map(node => node.getText(ast)).join("\n")
    .replace("export default function VoiceInput", "function VoiceInput").replace(/^export /gm, "");
  const output = ts.transpileModule(program, { compilerOptions: { jsx: ts.JsxEmit.React, module: ts.ModuleKind.ESNext } }).outputText;
  const instances: VoiceController[] = []; const cleanups: (() => void)[] = [];
  let controller: VoiceController | null = null; let mounted = false; let mountEffect: (() => (() => void)) | undefined;
  class TrackedController extends VoiceController {
    constructor(environment: VoiceEnvironment) { super(environment); instances.push(this); }
  }
  const hooks = {
    React: { createElement }, VoiceController: TrackedController, browserVoiceEnvironment: () => env,
    useState: () => [controller, (next: VoiceController) => { controller = next; }],
    useEffect: (effect: () => (() => void)) => { if (!mounted) mountEffect = effect; },
    useSyncExternalStore: (subscribe: VoiceController["subscribe"], snapshot: VoiceController["getState"]) => {
      cleanups.push(subscribe(() => {})); return snapshot();
    },
  };
  const Component = new Function(...Object.keys(hooks), output + "\nreturn VoiceInput;")(...Object.values(hooks));
  return {
    instances,
    mount() { Component({ compact: true }); mounted = true; cleanups.push(mountEffect!()); },
    view(): ReactElement<Record<string, any>> { const connected = Component({ compact: true }); return connected.type(connected.props); },
    unmount() { cleanups.reverse().forEach(cleanup => cleanup()); },
  };
}

test("compact microphone entry has a named keyboard button and starts only on click", () => {
  const f = fixture(); const calls: string[] = [];
  const tree = VoiceView({ compact: true, state: f.controller.getState(), onStart: () => calls.push("start"),
    onStop: () => calls.push("stop"), onSend: () => calls.push("send"), onDiscard: () => calls.push("discard") });
  const html = renderToStaticMarkup(tree); const controls = voiceButtons(tree);
  assert.equal(controls.length, 1); assert.equal(controls[0].props.type, "button");
  assert.equal(controls[0].props["aria-describedby"], "voice-privacy-notice");
  assert.match(html, /Start recording/); assert.match(html, /<svg[^>]*aria-hidden="true"[^>]*focusable="false"/);
  assert.match(html, /stays local until you explicitly send/); assert.doesNotMatch(html, /<details|<summary|autofocus|autoplay/i);
  assert.deepEqual(calls, []); controls[0].props.onClick(); assert.deepEqual(calls, ["start"]);
});

test("compact recording workflow remains visible and keeps Send and Discard explicit", () => {
  const f = fixture(); const noop = () => assert.fail("Rendering must not invoke voice actions");
  for (const phase of ["requesting", "recording", "stopping", "review", "sending"] as const) {
    const tree = VoiceView({ compact: true, state: { ...f.controller.getState(), phase, elapsedMs: 1200,
      recording: phase === "review" || phase === "sending" ? clip() : null }, onStart: noop, onStop: noop, onSend: noop, onDiscard: noop });
    const html = renderToStaticMarkup(tree);
    assert.doesNotMatch(html, /<details|<summary|<[^>]+\shidden(?:\s|=|>)|<(?:section|div|p|button)[^>]*aria-hidden="true"/);
    assert.ok(voiceButtons(tree).every(button => button.props.type === "button"));
    if (phase === "requesting") { assert.match(html, /Requesting microphone permission/); assert.match(html, />Cancel</); }
    if (phase === "recording") { assert.match(html.replace(/<[^>]*>/g, ""), /Recording · 1.2 seconds/); assert.match(html, /Stop recording/); assert.doesNotMatch(html, /Send to OpenAI/); }
    if (phase === "stopping") assert.match(html, /Microphone stopped/);
    if (phase === "review" || phase === "sending") {
      assert.match(html, /<audio controls=""/); assert.doesNotMatch(html, /autoplay/);
      assert.match(html, /will leave your Mac/); assert.match(html, /Send to OpenAI for transcription/); assert.match(html, /Discard recording/);
      assert.ok(voiceButtons(tree).every(button => button.props.disabled === (phase === "sending")));
    }
    if (phase === "sending") assert.match(html, /Sending the reviewed recording/);
  }
});

test("compact VoiceInput mount creates one controller without capture and unmount releases recording", async () => {
  const f = fixture(); const h = voiceMountHarness(f.env);
  mockIPC(() => assert.fail("Mount, capture and unmount must not invoke IPC"));
  try {
    h.mount(); const tree = h.view(); h.view();
    assert.equal(h.instances.length, 1); assert.equal(f.microphoneCalls(), 0); assert.equal(f.timers.size, 0); assert.deepEqual(f.sent, []);
    const controls = voiceButtons(VoiceView(tree.props as Parameters<typeof VoiceView>[0]));
    controls[0].props.onClick(); await Promise.resolve();
    assert.equal(f.microphoneCalls(), 1); assert.equal(h.instances[0].getState().phase, "recording");
    assert.equal(f.timers.size, 2); assert.deepEqual(f.sent, []);
  } finally { h.unmount(); }
  assert.ok(f.tracks[0].stops > 0); assert.equal(f.timers.size, 0); assert.deepEqual(f.sent, []);
});
for (const phase of ["requesting", "recording", "stopping", "review"] as const) test(`owning component navigation/unmount cleans up ${phase} capture and observers`, async () => {
  const f = fixture(); const l = lifecycleEnvironment(); f.env.lifecycle = l.lifecycle;
  const h = voiceMountHarness(f.env); let grant!: (stream: MediaStream) => void;
  if (phase === "requesting") f.env.microphone = () => new Promise(resolve => { grant = resolve; });
  h.mount(); const controller = h.instances[0]; const pending = controller.start();
  if (phase !== "requesting") await pending;
  let stale: ReturnType<typeof capturedEvents> | undefined;
  if (f.recorders.length) {
    stale = capturedEvents(f.recorders[0]);
    if (phase === "stopping") f.recorders[0].stop = () => { f.recorders[0].state = "inactive"; };
    if (phase === "stopping" || phase === "review") { f.setTime(1200); controller.stop(); }
  }
  assert.equal(controller.getState().phase, phase); h.unmount();
  if (phase === "requesting") { grant(f.stream); await pending; }
  stale?.data({ data: new Blob([new Uint8Array([1])]) }); stale?.stop(); stale?.error(); l.nativeBlur();
  await Promise.resolve();
  assert.equal(controller.getState().phase, "unavailable"); assert.equal(controller.getState().recording, null);
  assert.ok(f.tracks[0].stops > 0); assert.equal(f.timers.size, 0); assert.equal(l.listeners.size, 0); assert.equal(l.nativeRemovals(), 1);
  assert.deepEqual(f.sent, []); if (phase === "review") assert.equal(f.revoked.length, 1);
});
test("unmount during Send clears local state but cannot recall the dispatched transcription", async () => {
  const f = fixture(); const l = lifecycleEnvironment(); f.env.lifecycle = l.lifecycle;
  let complete!: (value: VoiceResult) => void;
  f.env.transcribe = recording => { f.sent.push(recording); return new Promise(resolve => { complete = resolve; }); };
  const h = voiceMountHarness(f.env); h.mount(); const controller = h.instances[0];
  await controller.start(); f.setTime(1200); controller.stop(); const sending = controller.send();
  assert.equal(f.sent.length, 1); assert.equal(controller.getState().phase, "sending");
  h.unmount(); complete(result); await sending;
  assert.equal(f.sent.length, 1); assert.equal(controller.getState().result, null); assert.equal(controller.getState().recording, null);
  assert.equal(f.revoked.length, 1); assert.equal(f.timers.size, 0); assert.equal(l.listeners.size, 0);
  const html = renderToStaticMarkup(VoiceView({ state: { phase: "sending", recording: clip(), result: null, message: null, elapsedMs: 1200 }, onStart() {}, onStop() {}, onSend() {}, onDiscard() {} }));
  assert.match(html, /cannot recall a request already dispatched to OpenAI/);
});
test("stopping view offers bounded Discard recovery and recording offers explicit Cancel", () => {
  const f = fixture(); let discards = 0;
  for (const phase of ["recording", "stopping"] as const) {
    const tree = VoiceView({ state: { ...f.controller.getState(), phase }, onStart() {}, onStop() {}, onSend() { assert.fail("Recovery cannot send"); }, onDiscard: () => { discards++; } });
    const html = renderToStaticMarkup(tree); const button = voiceButtons(tree).at(-1)!;
    assert.equal(button.props.children, phase === "recording" ? "Cancel recording" : "Discard unfinished recording");
    assert.equal(button.props.type, "button"); if (phase === "stopping") assert.match(html, /up to five seconds/);
    button.props.onClick();
  }
  assert.equal(discards, 2);
});
test("recording status announces the state without putting each timer tick in a live region", () => {
  const f = fixture();
  for (const elapsedMs of [1200, 1300]) {
    const html = renderToStaticMarkup(VoiceView({ state: { ...f.controller.getState(), phase: "recording", elapsedMs },
      onStart() {}, onStop() {}, onSend() {}, onDiscard() {} }));
    assert.match(html, /<span role="status">Recording<\/span>/);
    assert.doesNotMatch(html, /<p role="status">/);
    assert.match(html, new RegExp(`${(elapsedMs / 1000).toFixed(1)} seconds`));
  }
});

test("compact VoiceInput remains unavailable in static browser preview", () => {
  const env = browserVoiceEnvironment(); let microphoneCalls = 0;
  env.microphone = async () => { microphoneCalls++; assert.fail("Static preview must not request microphone permission"); };
  const h = voiceMountHarness(env);
  mockIPC(() => assert.fail("Static preview must not invoke IPC"));
  try {
    h.mount(); const tree = h.view(); const html = renderToStaticMarkup(tree);
    assert.match(html, /Voice capture is unavailable/); assert.doesNotMatch(html, /Start recording/);
    assert.equal(microphoneCalls, 0); assert.equal(h.instances.length, 1);
  } finally { h.unmount(); }
});

for (const replacesDraft of [false, true]) test(`Ask GHOST transcript adoption is explicit and announces draft replacement=${replacesDraft}`, () => {
  const text = "Reviewed untrusted transcript 😀"; const adopted: string[] = [];
  const state = { phase: "transcript" as const, elapsedMs: 0, recording: null, result: { ...result, text }, message: null };
  const tree = VoiceView({ state, compact: true, transcriptTarget: { available: true, busy: false, replacesDraft },
    onStart() {}, onStop() {}, onSend() { assert.fail("Adoption must not send audio"); }, onDiscard() {}, onUseTranscript: value => adopted.push(value) });
  const html = renderToStaticMarkup(tree); const button = voiceButtons(tree).at(-1)!;
  assert.equal(button.props.children, replacesDraft ? "Replace Ask GHOST draft with transcript" : "Use transcript in Ask GHOST");
  assert.equal(button.props.type, "button"); assert.equal(button.props.disabled, false);
  assert.equal(button.props["aria-describedby"], "voice-handoff-notice");
  assert.match(html, /UTF-8 bytes for Ask GHOST/); assert.match(html, /readonly/i);
  assert.match(html, replacesDraft ? /will replace your existing Ask GHOST draft/ : /untrusted text into an editable local draft/);
  assert.deepEqual(adopted, []); button.props.onClick(); assert.deepEqual(adopted, [text]);
  assert.equal(state.result.text, text);
});

for (const text of ["", " \n\t", "x".repeat(8193), "é".repeat(4097), "😀".repeat(2049)]) {
  test(`rejected handoff keeps the original returned transcript (${new TextEncoder().encode(text).length} bytes)`, async () => {
    const f = fixture(); f.env.transcribe = async () => ({ ...result, text });
    await review(f); await f.controller.send(); const original = f.controller.getState().result;
    let callbacks = 0; const client = new JarvisClient(() => true, async<T>() => { assert.fail("Handoff must not invoke planning or execution"); return {} as T; });
    const tree = VoiceView({ state: f.controller.getState(), compact: true, transcriptTarget: { available: true, busy: false, replacesDraft: true },
      onStart() {}, onStop() {}, onSend() { assert.fail("Handoff must not resend audio"); }, onDiscard() {},
      onUseTranscript: value => { callbacks++; adoptTranscript(client, value, false); } });
    const html = renderToStaticMarkup(tree); const button = voiceButtons(tree).at(-1)!;
    assert.equal(button.props.disabled, true); assert.match(html, /empty|exceeds the 8,192 UTF-8 byte limit/);
    button.props.onClick(); assert.equal(callbacks, 0);
    assert.equal(adoptTranscript(client, text, false).draft, null);
    assert.equal(f.controller.getState().result, original); assert.equal(original!.text, text);
    f.controller.dispose();
  });
}

for (const target of [{ available: true, busy: true, replacesDraft: true }, { available: false, busy: false, replacesDraft: false }]) {
  test(`handoff control rejects busy=${target.busy} available=${target.available}`, () => {
    let callbacks = 0;
    const tree = VoiceView({ state: { phase: "transcript", elapsedMs: 0, recording: null, result, message: null }, transcriptTarget: target,
      onStart() {}, onStop() {}, onSend() {}, onDiscard() {}, onUseTranscript: () => { callbacks++; } });
    const button = voiceButtons(tree).at(-1)!; assert.equal(button.props.disabled, true);
    assert.match(renderToStaticMarkup(tree), target.busy ? /Wait for planning to finish/ : /requires the native desktop app/);
    button.props.onClick(); assert.equal(callbacks, 0);
  });
}

test("static browser never invokes transcription or requests microphone", async () => {
  mockIPC(() => assert.fail("Static preview must not invoke IPC"));
  await assert.rejects(transcribeRecording(clip()));
  const env = browserVoiceEnvironment(); const controller = new VoiceController(env);
  assert.equal(env.available, false); await controller.start(); assert.equal(controller.getState().phase, "unavailable");
});
for (const cleanup of ["stop", "discard", "dispose"] as const) test(`browser timers retain their Window receiver during recording and ${cleanup}`, async t => {
  const f = fixture(); const calls: string[] = [];
  // WebKit rejects browser timer functions when VoiceEnvironment becomes their receiver.
  function requireWindow(receiver: unknown, method: string) {
    if (receiver !== window) throw new TypeError(`Can only call Window.${method} on instances of Window`);
    calls.push(method);
  }
  t.mock.method(window, "setTimeout", function (this: unknown, fn: () => void, ms: number) {
    requireWindow(this, "setTimeout"); return f.env.timeout(fn, ms);
  });
  t.mock.method(window, "setInterval", function (this: unknown, fn: () => void, ms: number) {
    requireWindow(this, "setInterval"); return f.env.interval(fn, ms);
  });
  t.mock.method(window, "clearTimeout", function (this: unknown, id: ReturnType<typeof setTimeout>) {
    requireWindow(this, "clearTimeout"); f.env.clearTimeout(id);
  });
  t.mock.method(window, "clearInterval", function (this: unknown, id: ReturnType<typeof setInterval>) {
    requireWindow(this, "clearInterval"); f.env.clearInterval(id);
  });
  mockIPC(() => assert.fail("Recording and cleanup must not invoke IPC"));
  const browser = browserVoiceEnvironment();
  const controller = new VoiceController({ ...f.env, timeout: browser.timeout, interval: browser.interval,
    clearTimeout: browser.clearTimeout, clearInterval: browser.clearInterval });
  try {
    await controller.start();
    assert.equal(controller.getState().phase, "recording");
    assert.equal(f.recorders[0].starts, 1);
    assert.deepEqual(calls, ["setTimeout", "setInterval"]);
    assert.deepEqual([...f.timers.values()].map(timer => timer.ms), [MAX_DURATION_MS, 100]);
    f.setTime(1200); [...f.timers.values()].find(timer => timer.ms === 100)!.fn();
    assert.equal(controller.getState().elapsedMs, 1200);
    controller[cleanup]();
    assert.deepEqual(calls, cleanup === "stop" ? ["setTimeout", "setInterval", "clearTimeout", "clearInterval", "setTimeout", "clearTimeout"] : ["setTimeout", "setInterval", "clearTimeout", "clearInterval"]);
    assert.equal(f.timers.size, 0); assert.ok(f.tracks[0].stops > 0);
    if (cleanup !== "dispose") assert.equal(controller.getState().phase, cleanup === "stop" ? "review" : "idle");
    assert.deepEqual(f.sent, []);
  } finally { controller.dispose(); }
});
test("feature detection requires native runtime, microphone and an allowlisted recorder format", () => {
  const supported = { isTypeSupported: () => true } as unknown as typeof MediaRecorder;
  assert.equal(voiceAvailable(false, { getUserMedia: async () => { throw "unused"; } }, supported), false);
  assert.equal(voiceAvailable(true, undefined, supported), false);
  assert.equal(voiceAvailable(true, { getUserMedia: async () => { throw "unused"; } }, undefined), false);
  assert.equal(voiceAvailable(true, { getUserMedia: async () => { throw "unused"; } }, supported), true);
  assert.equal(selectVoiceMime({ isTypeSupported: () => false }), null);
  assert.equal(selectVoiceMime({ isTypeSupported: () => { throw "unsupported"; } }), null);
});
test("browser environment requests audio only after explicit Start, with optional processing constraints", async () => {
  const previousNavigator = Object.getOwnPropertyDescriptor(globalThis, "navigator");
  const previousRecorder = Object.getOwnPropertyDescriptor(globalThis, "MediaRecorder");
  const requests: MediaStreamConstraints[] = []; const tracks = [{ stop() {}, onended: null }];
  class SupportedRecorder extends FakeRecorder { static isTypeSupported(mime: string) { return mime === "audio/webm"; } }
  Object.defineProperty(globalThis, "navigator", { configurable: true, value: { mediaDevices: { getUserMedia: async (constraints: MediaStreamConstraints) => { requests.push(constraints); return { getTracks: () => tracks }; } } } });
  Object.defineProperty(globalThis, "MediaRecorder", { configurable: true, value: SupportedRecorder });
  Object.assign(globalThis, { isTauri: true });
  const controller = new VoiceController(browserVoiceEnvironment());
  try {
    assert.deepEqual(requests, []); await controller.start();
    assert.deepEqual(requests, [{ audio: { echoCancellation: true, noiseSuppression: true, autoGainControl: true }, video: false }]);
  } finally {
    controller.dispose();
    if (previousNavigator) Object.defineProperty(globalThis, "navigator", previousNavigator); else delete (globalThis as any).navigator;
    if (previousRecorder) Object.defineProperty(globalThis, "MediaRecorder", previousRecorder); else delete (globalThis as any).MediaRecorder;
  }
});
for (const expected of VOICE_MIMES) test(`MIME selection uses exact runtime-supported ${expected}`, () => {
  const queried: string[] = [];
  assert.equal(selectVoiceMime({ isTypeSupported: mime => { queried.push(mime); return mime === expected; } }), expected);
  assert.ok(queried.every(mime => VOICE_MIMES.includes(mime as typeof expected)));
});
test("constructing and subscribing never starts microphone; Start is local and cannot send", async () => {
  const f = fixture(); f.controller.subscribe(() => {});
  assert.equal(f.microphoneCalls(), 0); await f.controller.start(); await f.controller.start(); await f.controller.send();
  assert.equal(f.microphoneCalls(), 1); assert.equal(f.recorders[0].starts, 1); assert.deepEqual(f.sent, []);
  f.controller.dispose();
});
test("Stop immediately releases tracks, clears timers, and creates local playback without sending", async () => {
  const f = fixture(); await review(f);
  assert.ok(f.tracks[0].stops > 0); assert.equal(f.timers.size, 0); assert.deepEqual(f.sent, []);
  assert.equal(f.controller.getState().recording?.url, "blob:local-1");
  assert.equal(f.controller.getState().recording?.durationMs, 1200);
});
test("30-second automatic stop is local and never sends", async () => {
  const f = fixture(); await f.controller.start(); f.setTime(MAX_DURATION_MS);
  [...f.timers.values()].find(timer => timer.ms === MAX_DURATION_MS)!.fn();
  assert.equal(f.controller.getState().phase, "review"); assert.ok(f.tracks[0].stops > 0); assert.deepEqual(f.sent, []);
});
test("microphone is stopped before UI announces stopping; delayed final data cannot inflate duration", async () => {
  const f = fixture(); await f.controller.start();
  const recorder = f.recorders[0];
  recorder.stop = () => { recorder.state = "inactive"; };
  f.controller.subscribe(() => { if (f.controller.getState().phase === "stopping") assert.ok(f.tracks[0].stops > 0); });
  f.setTime(29_000); f.controller.stop(); f.setTime(33_000);
  recorder.ondataavailable!({ data: new Blob([new Uint8Array([1, 2, 3])]) }); recorder.onstop!();
  assert.equal(f.controller.getState().recording?.durationMs, 29_000); assert.deepEqual(f.sent, []);
});
test("delayed deadline cannot make an over-30-second clip sendable", async () => {
  const f = fixture(); await f.controller.start(); f.setTime(MAX_DURATION_MS + 1); f.controller.stop(); await f.controller.send();
  assert.equal(f.controller.getState().phase, "error"); assert.deepEqual(f.sent, []); assert.ok(f.tracks[0].stops > 0);
});
test("Discard and unmount release tracks, references, object URLs and timers without sending", async () => {
  for (const operation of ["discard", "dispose"] as const) {
    const f = fixture(); await review(f); f.controller[operation]();
    assert.deepEqual(f.revoked, ["blob:local-1"]); assert.equal(f.controller.getState().recording, null); assert.deepEqual(f.sent, []);
    const recording = fixture(); await recording.controller.start(); recording.controller[operation]();
    assert.ok(recording.tracks[0].stops > 0); assert.equal(recording.timers.size, 0);
  }
});
for (const operation of ["discard", "dispose"] as const) test(`pending permission resolution after ${operation} stops obtained tracks`, async () => {
  const f = fixture(); let resolve!: (stream: MediaStream) => void;
  f.env.microphone = () => new Promise(done => { resolve = done; });
  const starting = f.controller.start(); f.controller[operation](); resolve(f.stream); await starting;
  assert.ok(f.tracks[0].stops > 0); assert.equal(f.recorders.length, 0); assert.deepEqual(f.sent, []);
});
test("permission rejection and recorder construction failure stay generic and stop tracks", async () => {
  const f = fixture(); f.env.microphone = async () => { const error = new Error("PRIVATE_RAW_ERROR"); error.name = "NotAllowedError"; throw error; };
  await f.controller.start(); assert.match(f.controller.getState().message!, /permission was denied/);
  assert.ok(!f.controller.getState().message!.includes("PRIVATE_RAW_ERROR"));
  const failed = fixture(); failed.env.recorder = () => { throw new Error("PRIVATE_RAW_ERROR"); };
  await failed.controller.start(); assert.ok(failed.tracks[0].stops > 0); assert.equal(failed.timers.size, 0);
});
test("recorder errors and unexpected termination stop microphone without a send", async () => {
  const f = fixture(); await f.controller.start(); f.recorders[0].onerror!();
  assert.ok(f.tracks[0].stops > 0); assert.equal(f.controller.getState().phase, "error"); assert.deepEqual(f.sent, []);
  const ended = fixture(); await ended.controller.start(); ended.setTime(1000); ended.recorders[0].onstop!();
  assert.ok(ended.tracks[0].stops > 0); assert.deepEqual(ended.sent, []);
});
test("oversized captured audio is discarded before IPC", async () => {
  const f = fixture(); await f.controller.start(); f.recorders[0].ondataavailable!({ data: new Blob([new Uint8Array(MAX_AUDIO_BYTES + 1)]) });
  assert.equal(f.controller.getState().phase, "error"); assert.equal(f.controller.getState().recording, null);
  assert.ok(f.tracks[0].stops > 0); await f.controller.send(); assert.deepEqual(f.sent, []);
});
test("new recording invalidates old bytes, callbacks and playback URL", async () => {
  const f = fixture(); await f.controller.start();
  const staleStop = f.recorders[0].onstop!; const staleError = f.recorders[0].onerror!;
  f.setTime(1200); f.controller.stop(); const old = f.controller.getState().recording;
  await f.controller.start(); staleStop(); staleError();
  assert.equal(f.controller.getState().phase, "recording"); assert.deepEqual(f.revoked, [old!.url]);
  f.setTime(2400); f.controller.stop(); await f.controller.send();
  assert.equal(f.sent.length, 1); assert.notEqual(f.sent[0], old); assert.equal(f.sent[0].url, "blob:local-2");
});
test("changing project selection cannot change the captured recording or Send payload", async () => {
  const f = fixture(); let selectedProject = "example"; await review(f);
  const reviewed = f.controller.getState().recording;
  selectedProject = "another-project"; assert.equal(selectedProject, "another-project");
  await f.controller.send(); assert.equal(f.sent[0], reviewed);
  assert.deepEqual(new Uint8Array(await f.sent[0].blob.arrayBuffer()), new Uint8Array([1, 2, 3]));
  const source = readFileSync(new URL("../src/IntentInterpreter.tsx", import.meta.url), "utf8");
  assert.match(source, /<VoiceInput onUseTranscript=/); // no alias prop or selection-dependent key
});
test("capture construction/start failures, unsupported actual format, and object URL failure stop tracks", async () => {
  for (const failure of ["start", "format", "url"]) {
    const f = fixture();
    const create = f.env.recorder;
    f.env.recorder = (stream, mime) => {
      const recorder = create(stream, mime);
      if (failure === "start") recorder.start = () => { throw new Error("PRIVATE_RECORDER_ERROR"); };
      if (failure === "format") Object.assign(recorder, { mimeType: "audio/unlisted" });
      return recorder;
    };
    if (failure === "url") f.env.createURL = () => { throw new Error("PRIVATE_URL_ERROR"); };
    await f.controller.start(); if (failure === "url") { f.setTime(1000); f.controller.stop(); }
    assert.ok(f.tracks[0].stops > 0); assert.equal(f.controller.getState().phase, "error");
    assert.equal(f.timers.size, 0); assert.deepEqual(f.sent, []);
    assert.ok(!f.controller.getState().message!.includes("PRIVATE_"));
  }
});
test("recording view offers Stop, no Send or automatic playback", () => {
  const f = fixture(); const noop = () => assert.fail("Rendering cannot invoke capture/transcription");
  const html = renderToStaticMarkup(createElement(VoiceView, { state: { ...f.controller.getState(), phase: "recording", elapsedMs: 1200 }, onStart: noop, onStop: noop, onSend: noop, onDiscard: noop }));
  assert.match(html, /Stop recording/); assert.match(html, /maximum 30 seconds/);
  assert.ok(!html.includes("Send to OpenAI")); assert.ok(!html.includes("autoplay"));
});
test("explicit Send invokes once and concurrent sends/recordings cannot duplicate it", async () => {
  const f = fixture(); await review(f); let finish!: (result: VoiceResult) => void;
  f.env.transcribe = recording => { f.sent.push(recording); return new Promise(done => { finish = done; }); };
  const sending = f.controller.send(); await f.controller.send(); await f.controller.start(); f.controller.discard();
  assert.equal(f.sent.length, 1); assert.equal(f.recorders.length, 1); finish(result); await sending;
  assert.equal(f.controller.getState().phase, "transcript"); assert.equal(f.controller.getState().recording, null);
});
test("raw IPC sends only the reviewed bytes and three bounded consent metadata headers", async () => {
  Object.assign(globalThis, { isTauri: true }); mockIPC(() => result);
  const internals = (globalThis as any).__TAURI_INTERNALS__; const original = internals.invoke; const calls: any[] = [];
  internals.invoke = (command: string, args: unknown, options: unknown) => { calls.push({ command, args, options }); return original(command, args, options); };
  assert.deepEqual(await transcribeRecording(clip()), result); assert.equal(calls.length, 1);
  assert.equal(calls[0].command, "transcribe_ghost_voice"); assert.ok(calls[0].args instanceof Uint8Array);
  assert.deepEqual(calls[0].args, new Uint8Array([1, 2, 3]));
  assert.deepEqual(calls[0].options, { headers: { "x-ghost-voice-mime": "audio/webm", "x-ghost-voice-duration-ms": "1200", "x-ghost-voice-confirmed": "send-to-openai" } });
});
for (const invalid of [0, MAX_AUDIO_BYTES + 1]) test(`raw IPC rejects invalid audio size ${invalid}`, async () => {
  Object.assign(globalThis, { isTauri: true }); mockIPC(() => assert.fail("Invalid recording must never invoke"));
  await assert.rejects(transcribeRecording({ ...clip(), blob: new Blob([new Uint8Array(invalid)]) }));
});
test("raw IPC rejects unsupported MIME and duration metadata before invoke", async () => {
  Object.assign(globalThis, { isTauri: true }); mockIPC(() => assert.fail("Invalid metadata must never invoke"));
  await assert.rejects(transcribeRecording({ ...clip(), mime: "https://invalid" as Recording["mime"] }));
  for (const durationMs of [0, -1, 30_001, NaN, 0.5]) await assert.rejects(transcribeRecording({ ...clip(), durationMs }));
});
test("native error details remain generic; transport ambiguity is explicit and never retried", async () => {
  const f = fixture(); await review(f); let calls = 0;
  f.env.transcribe = async () => { calls++; throw "PRIVATE_NATIVE_KEY_AND_PATH"; };
  await f.controller.send(); assert.equal(calls, 1); assert.ok(!f.controller.getState().message!.includes("PRIVATE_NATIVE"));
  f.env.transcribe = async () => { calls++; throw "transport"; }; await f.controller.send();
  assert.equal(calls, 2); assert.match(f.controller.getState().message!, /may have reached the provider/);
  assert.match(f.controller.getState().message!, /did not retry automatically/);
  assert.ok(!voiceError(new Error("PRIVATE_NATIVE_KEY_AND_PATH")).includes("PRIVATE_NATIVE"));
});
test("completed transcript stays usable when audit fails; discard and replacement clear it", async () => {
  const f = fixture(); await review(f); f.env.transcribe = async () => ({ ...result, audit_recorded: false });
  await f.controller.send(); assert.equal(f.controller.getState().result?.text, result.text);
  assert.match(f.controller.getState().message!, /audit could not be recorded/);
  f.controller.discard(); assert.equal(f.controller.getState().result, null);
});
test("playback and transcript UI are literal, inert, and cannot create workflow requests", () => {
  const f = fixture(); const noop = () => assert.fail("Rendering must not perform an action");
  const state = { ...f.controller.getState(), phase: "transcript" as const, result: { ...result, text: '<script>alert("x")</script> ghost orchestrate run' } };
  const html = renderToStaticMarkup(createElement(VoiceView, { state, onStart: noop, onStop: noop, onSend: noop, onDiscard: noop }));
  assert.ok(!html.includes("<script>")); assert.match(html, /&lt;script&gt;/); assert.match(html, /readonly/i);
  assert.match(html, /Transcript only — no workflow action was performed/);
  assert.ok(!html.includes("Save Request")); assert.ok(!html.includes(">Run<"));
  const reviewed = renderToStaticMarkup(createElement(VoiceView, { state: { ...state, phase: "review", result: null, recording: clip() }, onStart: noop, onStop: noop, onSend: noop, onDiscard: noop }));
  assert.match(reviewed, /<audio controls="" controlsList="nodownload" src="blob:local-only"/); assert.match(reviewed, /will leave your Mac/);
});
test("voice files have no project dependency, workflow bridge or data persistence", () => {
  for (const name of ["VoiceInput.tsx", "voice-transcription.ts"]) {
    const source = readFileSync(new URL(`../src/${name}`, import.meta.url), "utf8");
    for (const forbidden of ["prepareActionRequest", "saveActionRequest", "orchestrate", "localStorage", "indexedDB", "fetch(", "dangerouslySetInnerHTML", "projectAlias", "OPENAI_API_KEY:"]) assert.ok(!source.includes(forbidden), forbidden);
  }
});
