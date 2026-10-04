import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { registerHooks } from "node:module";
import { afterEach, test } from "node:test";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import ts from "typescript";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { MAX_AUDIO_BYTES, MAX_DURATION_MS, VOICE_MIMES, VoiceController, browserVoiceEnvironment,
  selectVoiceMime, transcribeRecording, voiceAvailable, voiceError, type Recording, type VoiceEnvironment, type VoiceResult } from "../src/voice-transcription.ts";

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
function fixture() {
  let now = 0; let nextTimer = 0; let microphoneCalls = 0;
  const tracks = [{ stops: 0, onended: null as null | (() => void), stop() { this.stops++; } }];
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
  };
  const controller = new VoiceController(env);
  return { env, controller, tracks, recorders, sent, timers, revoked, stream,
    setTime(value: number) { now = value; }, microphoneCalls: () => microphoneCalls };
}
async function review(f: ReturnType<typeof fixture>) { await f.controller.start(); f.setTime(1200); f.controller.stop(); assert.equal(f.controller.getState().phase, "review"); }

test("static browser never invokes transcription or requests microphone", async () => {
  mockIPC(() => assert.fail("Static preview must not invoke IPC"));
  await assert.rejects(transcribeRecording(clip()));
  const env = browserVoiceEnvironment(); const controller = new VoiceController(env);
  assert.equal(env.available, false); await controller.start(); assert.equal(controller.getState().phase, "unavailable");
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
  f.setTime(29_000); f.controller.stop(); f.setTime(40_000);
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
  const f = fixture(); await review(f); const old = f.controller.getState().recording;
  const staleStop = f.recorders[0].onstop!; const staleError = f.recorders[0].onerror!;
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
