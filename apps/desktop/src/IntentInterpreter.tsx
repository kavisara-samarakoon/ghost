import { useEffect, useState, useSyncExternalStore } from "react";
import VoiceInput from "./VoiceInput.tsx";
import { desktopIntentEnvironment, IntentController, type IntentState } from "./intent-interpretation.ts";

export function IntentView({ state, onEdit, onPrepare, onSend, onSave, onDiscard }: {
  state: IntentState; onEdit: (text: string) => void; onPrepare: () => void; onSend: () => void; onSave: () => void; onDiscard: () => void;
}) {
  const busy = ["preparing", "sending", "saving"].includes(state.phase);
  const prepared = state.prepared; const proposal = state.proposal;
  return <section className="glass-panel intent-input" aria-labelledby="intent-title">
    <div className="command-panel-heading"><div><p className="page-eyebrow">Human-reviewed proposals</p><h2 id="intent-title">Controlled Intent</h2></div><span className="page-badge">Draft only</span></div>
    <p>GHOST will not execute this intent. Prepare first to review exactly what would be sent for AI interpretation.</p>
    {state.phase === "unavailable" ? <p>Intent interpretation is unavailable in static preview. Open the desktop app to prepare an intent.</p> : <>
      {(["draft", "preparing"].includes(state.phase)) && <div className="intent-draft">
        <p>Selected project: <strong>{state.projectAlias || "Choose a project"}</strong></p>
        <label htmlFor="intent-text">Human intent · maximum 8 KiB UTF-8</label>
        <textarea id="intent-text" value={state.text} disabled={busy} rows={5} onChange={event => onEdit(event.target.value)} />
        <button type="button" className="btn-primary" disabled={busy || !state.projectAlias || !state.text.trim()} onClick={onPrepare}>Prepare Interpretation</button>
      </div>}
      {prepared && !proposal && <div className="intent-review">
        <p>Bound project: <strong>{prepared.project_alias}</strong> · Model: {prepared.model}</p>
        {prepared.project_alias !== state.projectAlias && <p>Project selection changed. This in-flight request remains bound to {prepared.project_alias}.</p>}
        <p>Request SHA-256: <code>{prepared.request_sha256}</code></p>
        <label htmlFor="reviewed-intent">Exact sanitized outbound intent</label>
        <textarea id="reviewed-intent" readOnly rows={6} value={prepared.intent} />
        <p>{prepared.safety_notice}</p>
        <p>This intent will leave your Mac and be sent to OpenAI. No project files or context will be sent.</p>
        <div className="hero-buttons"><button type="button" className="btn-primary" disabled={busy} onClick={onSend}>Send reviewed intent to OpenAI</button>
          <button type="button" className="btn-secondary" disabled={busy} onClick={() => onEdit(state.text)}>Edit intent</button>
          <button type="button" className="btn-secondary" disabled={busy} onClick={onDiscard}>Discard</button></div>
      </div>}
      {proposal && <div className="intent-proposal">
        <h3>{proposal.kind === "plan" ? "AI-generated proposal — not executed" : proposal.kind === "clarify" ? "Clarification needed" : "Unsupported intent"}</h3>
        <p>Bound project: <strong>{proposal.project_alias}</strong> · {proposal.steps.length} steps</p>
        {proposal.project_alias !== state.projectAlias && <p>Project selection changed. This proposal stays bound to {proposal.project_alias}; discard it to prepare for the newly selected project.</p>}
        <pre>{proposal.summary}</pre>
        <ol>{proposal.steps.map((step, index) => <li key={index}><strong>{step.action}</strong>
          {step.action === "start_session" && <pre>{step.goal}</pre>}
          {step.action === "add_session_note" && <pre>{step.note}</pre>}
          {step.action === "create_handoff" && <p>Local draft provider: {step.provider}</p>}
        </li>)}</ol>
        <p>Proposal SHA-256: <code>{proposal.proposal_sha256}</code></p>
        {!state.saved && <div className="hero-buttons">{proposal.kind === "plan" && <button type="button" className="btn-primary" disabled={busy} onClick={onSave}>Save Plan Draft</button>}
          <button type="button" className="btn-secondary" disabled={busy} onClick={onDiscard}>Discard Proposal</button></div>}
      </div>}
      {state.saved && <div className="intent-saved"><h3>Plan draft saved</h3><pre>{state.saved.path}</pre><p>Saved file SHA-256: <code>{state.saved.plan_sha256}</code></p>
        <p>Plan draft only. No workflow action was performed. Run it manually through <code>ghost orchestrate</code> for a fresh preview and M34 confirmation.</p>
        <p>The saved file hash is different from M34’s project-bound execution fingerprint.</p>
        <button type="button" className="btn-secondary" onClick={onDiscard}>Discard local proposal</button></div>}
      {busy && <p role="status">{state.phase === "preparing" ? "Preparing locally…" : state.phase === "sending" ? "Interpreting the reviewed intent…" : "Saving the reviewed plan draft…"}</p>}
    </>}
    {state.message && <p className="intent-feedback" role="status">{state.message}</p>}
  </section>;
}

export default function IntentInterpreter({ controller }: { controller: IntentController }) {
  const state = useSyncExternalStore(controller.subscribe, controller.getState);
  return <IntentView state={state} onEdit={text => controller.edit(text)} onPrepare={() => { void controller.prepare(); }}
    onSend={() => { void controller.send(); }} onSave={() => { void controller.save(); }} onDiscard={() => controller.discard()} />;
}
export function useIntentController(projectAlias: string, available: boolean) {
  const [controller, setController] = useState<IntentController | null>(null);
  useEffect(() => {
    const instance = new IntentController(desktopIntentEnvironment(available), projectAlias);
    setController(instance); return () => instance.dispose();
  }, [available]);
  useEffect(() => { controller?.setProject(projectAlias); }, [controller, projectAlias]);
  return controller;
}

export function ControlledIntentWorkspace({ projectAlias, available }: { projectAlias: string; available: boolean }) {
  const controller = useIntentController(projectAlias, available);
  return <><VoiceInput onUseTranscript={controller ? text => controller.useTranscript(text) : undefined} />
    {controller ? <IntentInterpreter controller={controller} /> : <section className="glass-panel intent-input"><h2>Controlled Intent</h2><p>Checking desktop availability…</p></section>}</>;
}
