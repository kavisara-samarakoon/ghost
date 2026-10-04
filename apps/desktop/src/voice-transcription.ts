import { invoke, isTauri } from "@tauri-apps/api/core";

export const MAX_AUDIO_BYTES = 8 * 1024 * 1024;
export const MAX_DURATION_MS = 30_000;
export const VOICE_MIMES = ["audio/webm;codecs=opus", "audio/webm", "audio/mp4", "audio/ogg;codecs=opus", "audio/ogg"] as const;
export type VoiceMime = typeof VOICE_MIMES[number];
export interface VoiceResult {
  text: string; model: "gpt-transcribe"; audio_bytes: number; duration_ms: number; audit_recorded: boolean;
}
export interface Recording { blob: Blob; mime: VoiceMime; durationMs: number; url: string }
export type VoicePhase = "unavailable" | "idle" | "requesting" | "recording" | "stopping" | "review" | "sending" | "transcript" | "error";
export interface VoiceState {
  phase: VoicePhase; elapsedMs: number; recording: Recording | null; result: VoiceResult | null; message: string | null;
}

const messages: Record<string, string> = {
  unavailable: "Voice capture is unavailable here. Open the macOS desktop app with microphone support.",
  permission: "Microphone permission was denied. No recording was sent.",
  format: "No supported recording format is available.",
  recording: "The microphone recording could not be completed. No recording was sent.",
  too_large: "Recording exceeds the 8 MiB limit and was discarded.",
  duration: "Recording exceeded 30 seconds and was discarded.",
  credential: "OpenAI credential is unavailable. The native process needs OPENAI_API_KEY.",
  audit: "Local voice audit is unavailable. No transcription request was sent.",
  service: "The transcription service failed. GHOST did not retry automatically.",
  transport: "Transcription may have reached the provider. GHOST did not retry automatically. Review this warning before explicitly sending again.",
  transcript: "A safe transcript is unavailable. Provider work may have completed. GHOST did not retry automatically.",
  busy: "A transcription is already in progress. GHOST did not send another request.",
};
export function voiceError(error: unknown): string {
  return typeof error === "string" && Object.prototype.hasOwnProperty.call(messages, error) ? messages[error] : messages.service;
}
export function selectVoiceMime(recorder: Pick<typeof MediaRecorder, "isTypeSupported"> | undefined): VoiceMime | null {
  if (!recorder) return null;
  try { return VOICE_MIMES.find(mime => recorder.isTypeSupported(mime)) ?? null; }
  catch { return null; }
}
export function voiceAvailable(native: boolean, media: Pick<MediaDevices, "getUserMedia"> | undefined, recorder: typeof MediaRecorder | undefined): boolean {
  return native && typeof media?.getUserMedia === "function" && selectVoiceMime(recorder) !== null;
}

// Raw binary IPC, called exclusively by the controller's explicit Send operation.
export async function transcribeRecording(recording: Recording): Promise<VoiceResult> {
  if (!isTauri()) throw "unavailable";
  if (!VOICE_MIMES.includes(recording.mime)) throw "format";
  if (!recording.blob.size) throw "recording";
  if (recording.blob.size > MAX_AUDIO_BYTES) throw "too_large";
  if (!Number.isInteger(recording.durationMs) || recording.durationMs < 1 || recording.durationMs > MAX_DURATION_MS) throw "duration";
  const bytes = new Uint8Array(await recording.blob.arrayBuffer());
  return invoke<VoiceResult>("transcribe_ghost_voice", bytes, { headers: {
    "x-ghost-voice-mime": recording.mime,
    "x-ghost-voice-duration-ms": String(recording.durationMs),
    "x-ghost-voice-confirmed": "send-to-openai",
  } });
}

export interface VoiceEnvironment {
  available: boolean; mime: VoiceMime | null;
  microphone: () => Promise<MediaStream>;
  recorder: (stream: MediaStream, mime: VoiceMime) => MediaRecorder;
  createURL: (blob: Blob) => string; revokeURL: (url: string) => void;
  now: () => number;
  timeout: (fn: () => void, ms: number) => ReturnType<typeof setTimeout>;
  interval: (fn: () => void, ms: number) => ReturnType<typeof setInterval>;
  clearTimeout: (id: ReturnType<typeof setTimeout>) => void;
  clearInterval: (id: ReturnType<typeof setInterval>) => void;
  transcribe: (recording: Recording) => Promise<VoiceResult>;
}
export function browserVoiceEnvironment(): VoiceEnvironment {
  const media = typeof navigator === "undefined" ? undefined : navigator.mediaDevices;
  const recorder = typeof MediaRecorder === "undefined" ? undefined : MediaRecorder;
  const native = isTauri();
  return {
    available: voiceAvailable(native, media, recorder), mime: selectVoiceMime(recorder),
    microphone: () => media!.getUserMedia({ audio: { echoCancellation: true, noiseSuppression: true, autoGainControl: true }, video: false }),
    recorder: (stream, mime) => new recorder!(stream, { mimeType: mime }),
    createURL: blob => URL.createObjectURL(blob), revokeURL: url => URL.revokeObjectURL(url),
    now: () => performance.now(), timeout: setTimeout, interval: setInterval, clearTimeout, clearInterval,
    transcribe: transcribeRecording,
  };
}

// A finite capture controller. No project, workflow, persistence, or intent parsing input.
export class VoiceController {
  private state: VoiceState;
  private listeners = new Set<() => void>();
  private stream: MediaStream | null = null;
  private recorder: MediaRecorder | null = null;
  private chunks: Blob[] = [];
  private bytes = 0;
  private startedAt = 0;
  private stoppedAt: number | null = null;
  private generation = 0;
  private disposed = false;
  private deadline?: ReturnType<typeof setTimeout>;
  private ticker?: ReturnType<typeof setInterval>;
  private env: VoiceEnvironment;
  constructor(env: VoiceEnvironment) {
    this.env = env;
    this.state = { phase: env.available ? "idle" : "unavailable", elapsedMs: 0, recording: null, result: null, message: env.available ? null : messages.unavailable };
  }
  getState = (): VoiceState => this.state;
  subscribe = (listener: () => void): (() => void) => { this.listeners.add(listener); return () => this.listeners.delete(listener); };
  private update(patch: Partial<VoiceState>) {
    if (this.disposed) return;
    this.state = { ...this.state, ...patch }; this.listeners.forEach(listener => listener());
  }
  private stopTracks() {
    this.stream?.getTracks().forEach(track => { track.onended = null; track.stop(); }); this.stream = null;
    if (this.deadline !== undefined) this.env.clearTimeout(this.deadline);
    if (this.ticker !== undefined) this.env.clearInterval(this.ticker);
    this.deadline = undefined; this.ticker = undefined;
  }
  private releaseRecording() {
    if (this.state.recording) this.env.revokeURL(this.state.recording.url);
    this.chunks = []; this.bytes = 0;
  }
  private cancelCapture() {
    this.stopTracks();
    const recorder = this.recorder; this.recorder = null;
    if (recorder) {
      recorder.ondataavailable = null; recorder.onstop = null; recorder.onerror = null;
      if (recorder.state !== "inactive") { try { recorder.stop(); } catch { /* tracks are already stopped */ } }
    }
    this.chunks = []; this.bytes = 0;
  }
  private fail(code: string) {
    this.generation++; this.cancelCapture(); this.releaseRecording();
    this.update({ phase: "error", recording: null, result: null, message: voiceError(code) });
  }
  async start(): Promise<void> {
    if (this.disposed || !this.env.available || ["requesting", "recording", "stopping", "sending"].includes(this.state.phase)) return;
    const generation = ++this.generation;
    this.releaseRecording();
    this.update({ phase: "requesting", recording: null, result: null, message: null, elapsedMs: 0 });
    let obtained: MediaStream | null = null;
    try {
      obtained = await this.env.microphone();
      if (this.disposed || generation !== this.generation) { obtained.getTracks().forEach(track => track.stop()); return; }
      this.stream = obtained;
      if (!this.env.mime) { this.fail("format"); return; }
      const recorder = this.env.recorder(obtained, this.env.mime);
      this.recorder = recorder;
      const mime = recorder.mimeType || this.env.mime;
      if (!VOICE_MIMES.includes(mime as VoiceMime)) { this.fail("format"); return; }
      recorder.ondataavailable = event => {
        if (generation !== this.generation || this.disposed) return;
        this.bytes += event.data.size;
        if (this.bytes > MAX_AUDIO_BYTES) { this.fail("too_large"); return; }
        if (event.data.size) this.chunks.push(event.data);
      };
      recorder.onerror = () => { if (generation === this.generation && !this.disposed) this.fail("recording"); };
      recorder.onstop = () => {
        if (generation !== this.generation || this.disposed) return;
        this.stopTracks(); this.recorder = null;
        const durationMs = Math.max(1, Math.round((this.stoppedAt ?? this.env.now()) - this.startedAt));
        if (durationMs > MAX_DURATION_MS) { this.fail("duration"); return; }
        const blob = new Blob(this.chunks, { type: mime }); this.chunks = []; this.bytes = 0;
        if (!blob.size) { this.fail("recording"); return; }
        if (blob.size > MAX_AUDIO_BYTES) { this.fail("too_large"); return; }
        try {
          const recording = { blob, mime: mime as VoiceMime, durationMs, url: this.env.createURL(blob) };
          this.update({ phase: "review", recording, elapsedMs: durationMs });
        } catch { this.fail("recording"); }
      };
      obtained.getTracks().forEach(track => { track.onended = () => { if (generation === this.generation && !this.disposed) this.stop(); }; });
      this.startedAt = this.env.now();
      this.stoppedAt = null;
      recorder.start(250);
      this.update({ phase: "recording" });
      this.deadline = this.env.timeout(() => this.stop(), MAX_DURATION_MS);
      this.ticker = this.env.interval(() => this.update({ elapsedMs: Math.min(MAX_DURATION_MS, Math.round(this.env.now() - this.startedAt)) }), 100);
    } catch (error) {
      obtained?.getTracks().forEach(track => track.stop());
      if (generation !== this.generation || this.disposed) return;
      this.fail(error instanceof Error && error.name === "NotAllowedError" ? "permission" : "recording");
    }
  }
  stop(): void {
    if (this.state.phase !== "recording" || !this.recorder) return;
    const recorder = this.recorder;
    // Release the microphone before waiting for the recorder's final data event.
    this.stoppedAt = this.env.now();
    this.stopTracks();
    this.update({ phase: "stopping" });
    try { if (recorder.state !== "inactive") recorder.stop(); }
    catch { this.fail("recording"); }
  }
  async send(): Promise<void> {
    if (this.disposed || this.state.phase !== "review" || !this.state.recording) return;
    const recording = this.state.recording;
    const generation = this.generation;
    this.update({ phase: "sending", message: null }); // synchronous double-click gate
    try {
      const result = await this.env.transcribe(recording);
      if (this.disposed || generation !== this.generation) return;
      this.releaseRecording();
      this.update({ phase: "transcript", recording: null, result, message: result.audit_recorded ? null : "Transcript received, but completion audit could not be recorded. Keep this text; GHOST did not retry automatically." });
    } catch (error) {
      if (this.disposed || generation !== this.generation) return;
      this.update({ phase: "review", message: voiceError(error) });
    }
  }
  discard(): void {
    if (this.state.phase === "sending") return;
    this.generation++; this.cancelCapture(); this.releaseRecording();
    this.update({ phase: this.env.available ? "idle" : "unavailable", recording: null, result: null, elapsedMs: 0, message: null });
  }
  dispose(): void {
    this.generation++; this.cancelCapture(); this.releaseRecording(); this.disposed = true;
    this.state = { ...this.state, recording: null, result: null }; this.listeners.clear();
  }
}
