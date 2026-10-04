import { useEffect, useState, useSyncExternalStore } from "react";
import { browserVoiceEnvironment, VoiceController, type VoiceState } from "./voice-transcription.ts";

export function VoiceView({ state, onStart, onStop, onSend, onDiscard, onUseTranscript }: {
  state: VoiceState; onStart: () => void; onStop: () => void; onSend: () => void; onDiscard: () => void; onUseTranscript?: (text: string) => void;
}) {
  const clip = state.recording;
  return <section className="glass-panel voice-input" aria-labelledby="voice-input-title">
    <div className="command-panel-heading"><div><p className="page-eyebrow">Explicit microphone capture</p><h2 id="voice-input-title">Voice Input</h2></div>
      <span className="page-badge">Transcript only</span></div>
    <p>Microphone recording stays local until you explicitly send it for transcription.</p>
    <p>Voice input never runs workflow actions automatically.</p>
    {["idle", "error"].includes(state.phase) && <button type="button" className="btn-primary" onClick={onStart}>Start recording</button>}
    {state.phase === "requesting" && <><p role="status">Requesting microphone permission…</p><button type="button" className="btn-secondary" onClick={onDiscard}>Cancel</button></>}
    {state.phase === "recording" && <div className="voice-recording"><p role="status"><span className="voice-recording-dot" aria-hidden="true" />Recording · {(state.elapsedMs / 1000).toFixed(1)} seconds · maximum 30 seconds</p>
      <button type="button" className="btn-secondary" onClick={onStop}>Stop recording</button></div>}
    {state.phase === "stopping" && <p role="status">Microphone stopped. Finishing the local recording…</p>}
    {clip && <div className="voice-review"><p>Review recording · {(clip.durationMs / 1000).toFixed(1)} seconds · approximately {(clip.blob.size / 1024).toFixed(1)} KiB</p>
      <audio controls controlsList="nodownload" src={clip.url} preload="metadata" />
      <p>This recording will leave your Mac and be sent to OpenAI for transcription. The resulting transcript remains untrusted text and will not execute anything.</p>
      <div className="hero-buttons"><button type="button" className="btn-primary" disabled={state.phase !== "review"} onClick={onSend}>Send to OpenAI for transcription</button>
        <button type="button" className="btn-secondary" disabled={state.phase === "sending"} onClick={onDiscard}>Discard recording</button></div></div>}
    {state.phase === "sending" && <p role="status">Sending the reviewed recording for transcription…</p>}
    {state.result && <div className="voice-transcript"><label htmlFor="voice-transcript">Untrusted transcript</label>
      <textarea id="voice-transcript" readOnly value={state.result.text} rows={7} />
      <p>Transcript only — no workflow action was performed.</p>
      <div className="hero-buttons"><button type="button" className="btn-primary" onClick={onStart}>Record another clip</button>
        <button type="button" className="btn-secondary" onClick={onDiscard}>Discard transcript</button>
        {onUseTranscript && <button type="button" className="btn-secondary" onClick={() => onUseTranscript(state.result!.text)}>Use transcript as intent</button>}</div></div>}
    {state.message && <p className="voice-feedback" role="status">{state.message}</p>}
  </section>;
}

export default function VoiceInput({ onUseTranscript }: { onUseTranscript?: (text: string) => void }) {
  const [controller, setController] = useState<VoiceController | null>(null);
  useEffect(() => {
    const instance = new VoiceController(browserVoiceEnvironment());
    setController(instance);
    return () => instance.dispose();
  }, []);
  return controller ? <VoiceConnected controller={controller} onUseTranscript={onUseTranscript} /> : <section className="glass-panel voice-input"><h2>Voice Input</h2><p>Checking local microphone support…</p></section>;
}
function VoiceConnected({ controller, onUseTranscript }: { controller: VoiceController; onUseTranscript?: (text: string) => void }) {
  const state = useSyncExternalStore(controller.subscribe, controller.getState);
  return <VoiceView state={state} onStart={() => { void controller.start(); }} onStop={() => controller.stop()}
    onSend={() => { void controller.send(); }} onDiscard={() => controller.discard()} onUseTranscript={onUseTranscript} />;
}
