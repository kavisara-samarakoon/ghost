import { useEffect, useState, useSyncExternalStore } from "react";
import { browserVoiceEnvironment, VoiceController, type VoiceState } from "./voice-transcription.ts";
import { MAX_COMMAND_BYTES, utf8Bytes, transcriptHandoffError, type TranscriptTarget } from "./transcript-handoff.ts";

export function VoiceView({ state, onStart, onStop, onSend, onDiscard, onUseTranscript, compact = false, transcriptTarget }: {
  state: VoiceState; onStart: () => void; onStop: () => void; onSend: () => void; onDiscard: () => void; onUseTranscript?: (text: string) => void; compact?: boolean; transcriptTarget?: TranscriptTarget;
}) {
  const clip = state.recording;
  const handoffError = transcriptTarget && state.result ? transcriptHandoffError(state.result.text) : null;
  const startButton = ["idle", "error"].includes(state.phase) && <button type="button" className={compact ? "btn-secondary voice-start" : "btn-primary"} onClick={onStart} aria-describedby="voice-privacy-notice">
    {compact && <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5" aria-hidden="true" focusable="false"><rect x="9" y="3" width="6" height="12" rx="3" /><path d="M5 10v2a7 7 0 0 0 14 0v-2M12 19v3M8 22h8" /></svg>}
    Start recording</button>;
  return <section className={`glass-panel voice-input${compact ? " voice-input-compact" : ""}`} aria-labelledby="voice-input-title">
    {compact ? <div className="voice-entry"><h2 id="voice-input-title">Voice Input</h2>{startButton}</div> : <div className="command-panel-heading"><div><p className="page-eyebrow">Explicit microphone capture</p><h2 id="voice-input-title">Voice Input</h2></div>
      <span className="page-badge">Transcript only</span></div>}
    <p id="voice-privacy-notice">Microphone recording stays local until you explicitly send it for transcription.</p>
    <p>Voice input never runs workflow actions automatically.</p>
    <p>Unfinished capture is discarded if GHOST is hidden, minimized, loses focus, or you leave this page.</p>
    {!compact && startButton}
    {state.phase === "requesting" && <><p role="status">Requesting microphone permission…</p><button type="button" className="btn-secondary" onClick={onDiscard}>Cancel</button></>}
    {state.phase === "recording" && <div className="voice-recording"><p><span className="voice-recording-dot" aria-hidden="true" /><span role="status">Recording</span> · {(state.elapsedMs / 1000).toFixed(1)} seconds · maximum 30 seconds</p>
      <button type="button" className="btn-secondary" onClick={onStop}>Stop recording</button>
      <button type="button" className="btn-secondary" onClick={onDiscard}>Cancel recording</button></div>}
    {state.phase === "stopping" && <><p role="status">Microphone stopped. Finishing the local recording (up to five seconds)…</p>
      <button type="button" className="btn-secondary" onClick={onDiscard}>Discard unfinished recording</button></>}
    {clip && <div className="voice-review"><p>Review recording · {(clip.durationMs / 1000).toFixed(1)} seconds · approximately {(clip.blob.size / 1024).toFixed(1)} KiB</p>
      <audio controls controlsList="nodownload" src={clip.url} preload="metadata" />
      <p>This recording will leave your Mac and be sent to OpenAI for transcription. The resulting transcript remains untrusted text and will not execute anything.</p>
      <p>After Send, leaving this page or closing GHOST cannot recall a request already dispatched to OpenAI.</p>
      <div className="hero-buttons"><button type="button" className="btn-primary" disabled={state.phase !== "review"} onClick={onSend}>Send to OpenAI for transcription</button>
        <button type="button" className="btn-secondary" disabled={state.phase === "sending"} onClick={onDiscard}>Discard recording</button></div></div>}
    {state.phase === "sending" && <p role="status">Sending the reviewed recording for transcription…</p>}
    {state.result && <div className="voice-transcript"><label htmlFor="voice-transcript">Untrusted transcript</label>
      <textarea id="voice-transcript" readOnly value={state.result.text} rows={7} />
      <p>Transcript only — no workflow action was performed.</p>
      {transcriptTarget && <><p>{utf8Bytes(state.result.text).toLocaleString()} / {MAX_COMMAND_BYTES.toLocaleString()} UTF-8 bytes for Ask GHOST.</p>
        <p id="voice-handoff-notice">{handoffError ?? (!transcriptTarget.available ? "Transcript adoption requires the native desktop app." : transcriptTarget.busy ? "Wait for planning to finish before using this transcript. Your draft stays unchanged." : transcriptTarget.replacesDraft ? "Using this transcript will replace your existing Ask GHOST draft and reset context sharing off. Review the transcript before replacing it." : "Using this transcript copies untrusted text into an editable local draft with context sharing off. Planning still requires Prepare, review and a separate Send.")}</p></>}
      <div className="hero-buttons"><button type="button" className="btn-primary" onClick={onStart}>Record another clip</button>
        <button type="button" className="btn-secondary" onClick={onDiscard}>Discard transcript</button>
        {onUseTranscript && <button type="button" className="btn-secondary" disabled={transcriptTarget ? !transcriptTarget.available || transcriptTarget.busy || handoffError !== null : false}
          aria-describedby={transcriptTarget ? "voice-handoff-notice" : undefined} onClick={() => {
            if (transcriptTarget && (!transcriptTarget.available || transcriptTarget.busy || handoffError)) return;
            onUseTranscript(state.result!.text);
          }}>{transcriptTarget ? transcriptTarget.replacesDraft ? "Replace Ask GHOST draft with transcript" : "Use transcript in Ask GHOST" : "Use transcript as intent"}</button>}</div></div>}
    {state.message && <p className="voice-feedback" role="status">{state.message}</p>}
  </section>;
}

export default function VoiceInput({ onUseTranscript, compact = false, transcriptTarget }: { onUseTranscript?: (text: string) => void; compact?: boolean; transcriptTarget?: TranscriptTarget }) {
  const [controller, setController] = useState<VoiceController | null>(null);
  useEffect(() => {
    const instance = new VoiceController(browserVoiceEnvironment());
    setController(instance);
    return () => instance.dispose();
  }, []);
  return controller ? <VoiceConnected controller={controller} onUseTranscript={onUseTranscript} compact={compact} transcriptTarget={transcriptTarget} /> : <section className={`glass-panel voice-input${compact ? " voice-input-compact" : ""}`}><h2>Voice Input</h2><p role="status">Checking local microphone support…</p></section>;
}
function VoiceConnected({ controller, onUseTranscript, compact, transcriptTarget }: { controller: VoiceController; onUseTranscript?: (text: string) => void; compact: boolean; transcriptTarget?: TranscriptTarget }) {
  const state = useSyncExternalStore(controller.subscribe, controller.getState);
  return <VoiceView state={state} onStart={() => { void controller.start(); }} onStop={() => controller.stop()}
    onSend={() => { void controller.send(); }} onDiscard={() => controller.discard()} onUseTranscript={onUseTranscript} compact={compact} transcriptTarget={transcriptTarget} />;
}
