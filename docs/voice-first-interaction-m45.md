# M45 — Expanded Voice-First Interaction

M45 expands reviewed voice input in the active macOS Command experience. It
reuses M35 native transcription and the M43 Jarvis planner. This document covers
the development branch `feature/m45-voice-first-interaction`, reviewed against
M44 main baseline `357028a45d275752c48df52a5dbbd9dda7af1497`. It is not a release
or merge announcement. M44 remains closed; its production behavior is unchanged.

## Explicit workflow

1. Open **Command**. Voice Input is directly below the editable **Ask GHOST** field.
2. Click **Start recording**. Only this click requests audio-only microphone access.
3. Speak while GHOST stays in the foreground. Recording status, elapsed time,
   **Stop recording**, and **Cancel recording** remain visible without a disclosure.
4. Click **Stop recording**, or let the 30-second local deadline stop capture.
5. Review/play the completed recording locally. Choose **Discard recording** or
   **Send to OpenAI for transcription**. Playback and Stop never send audio.
6. Review the returned **Untrusted transcript**. Receiving it never starts planning.
7. Click **Use transcript in Ask GHOST**. If a draft already exists, the button
   instead says **Replace Ask GHOST draft with transcript** and explains replacement.
8. Review or edit the command locally. Click **Prepare request**, review the complete
   outbound text/context, then separately click **Send reviewed request to OpenAI**.
9. Review the resulting untrusted proposal. Each selected step still delegates to
   its existing independent local-request, Google, or personal-memory gate.

Voice input grants no action authority. Spoken confirmation, a transcript,
transcription consent, and a planning hash cannot replace an action confirmation.
There is no automatic planning, batch execution, shell/CLI execution, or provider retry.

## Limits and lifecycle

| Boundary | Behavior |
| --- | --- |
| Audio | At most 30,000 ms / 8 MiB; nonempty recordings only. A delayed recording deadline can cause an overlong clip to be discarded. |
| Format | Runtime-supported, exact allowlist: `audio/webm;codecs=opus`, `audio/webm`, `audio/mp4`, `audio/ogg;codecs=opus`, `audio/ogg`. No fixed format is assumed supported on every Mac. |
| Stop | Freeze capture duration and release tracks before waiting for recorder output. WebKit timer methods are bound to `window`. |
| Finalization | Five seconds maximum. Late output is rejected even if the timeout callback is delayed. **Discard unfinished recording** permits recovery immediately. |
| Hide/minimize/focus loss | Cancel pending permission, active recording, or unfinished finalization. Discard partial bytes and release tracks. Foreground return never resumes capture. |
| Track interruption | Track end/mute, disconnected tracks, recorder errors, or unexpected recorder termination discard unfinished capture. |
| Completed review | A valid completed clip or transcript can remain in memory during foreground changes; nothing is sent automatically. |
| Navigation/unmount | Dispose the component-owned controller, listeners, timers, tracks, audio references, and playback URLs. Ignore stale callbacks and late results. |
| Pending permission | Cancel invalidates that attempt. A late grant releases the obtained tracks; it cannot revive or replace a newer attempt. |
| Transcription response | Existing native limits remain 256 KiB response body and 64 KiB transcript. This is distinct from the smaller Ask GHOST command limit. |
| Ask GHOST adoption | Maximum 8,192 **UTF-8 bytes**, measured with `TextEncoder`; empty/whitespace-only or oversized text is rejected without truncation. |

Focus loss is deliberately conservative: switching applications, opening an OS
dialog, or opening an inspector can cancel unfinished capture. After returning,
click Start again. Cancellation cannot dismiss an OS permission prompt; it can
prevent a later grant from starting GHOST capture.

Successful adoption preserves exact text, replaces the draft explicitly, clears
old planning reviews/proposals, resets personal/project sharing off, clears the
context query, and preserves the selected project binding. Adoption is rejected
while preparation or planning Send is in progress, including stale callbacks.
An adoption failure leaves both the original transcript and previous draft intact.
Edited commands exceeding the byte limit are rejected before Prepare IPC; native
validation remains authoritative for other text/privacy checks.

For example, 4,096 `é` characters or 2,048 single `😀` characters occupy 8,192
UTF-8 bytes. Emoji sequences can use more bytes per visible symbol. Character
count and JavaScript string length do not establish the byte limit.

After explicit Send, navigation or closing GHOST cannot recall a request already
dispatched to OpenAI. Local cleanup does not cancel provider work or retract
transmitted content. An unmounted component ignores its late result and does not
retry. A transport error may mean the provider received the request.

## Permissions, privacy, and credentials

The existing macOS bundle contains `NSMicrophoneUsageDescription`; no camera
permission is added. Capture requests audio only, and mounting Command requests
no microphone access. Static browser preview cannot capture, transcribe, adopt
into the native planning path, or perform privileged actions.

Audio and transcripts are held in memory by the voice path. The only voice write
is private metadata in `GHOST_HOME/desktop-voice-audit.jsonl`: event/model, MIME
category, duration, byte counts, and outcome. Audio, transcript content, credentials,
and raw provider errors do not enter this audit. Releasing references is not a
secure-memory-erasure guarantee. Audio is not redacted before transmission: do
not record credentials or information you do not intend to share.

An adopted command is a separate local copy. Discarding the source transcript
does not erase that command. Later explicit workflow-request or memory saves may
persist separately reviewed text under existing authorization gates. M45 adds no
automatic memory ingestion, exports, cloud sync, or transcript persistence.

Transcription and Jarvis planning require `OPENAI_API_KEY` in the **native GHOST
process environment**. A shell variable must be exported and inherited by that
process; a Finder-launched application may not inherit it. For development,
launch `pnpm tauri dev` from an already configured private shell. GHOST does not
read `.env` files, provision/store OpenAI keys in Keychain, or accept a key through
Ask GHOST. Existing Google Keychain credentials do not configure OpenAI.

The inspected native source fixes transcription to `gpt-transcribe` at
`https://api.openai.com/v1/audio/transcriptions`, with one request after Send,
no redirect/proxy/automatic retry, and a durable pre-send metadata audit. These
source settings do not establish live model/account/media compatibility or a
provider retention guarantee.

## Troubleshooting

| Symptom | Local response |
| --- | --- |
| Voice unavailable in browser preview | Open the native macOS application. Browser preview is intentionally inert. |
| Microphone permission denied | Check GHOST's microphone permission in macOS settings and the selected device. Return to GHOST and explicitly Start again. No denied recording was sent. |
| Recording resets after switching apps or minimizing | Expected foreground-only policy. Return and Start again; partial capture is discarded. |
| Permission was granted after Cancel | Expected cleanup of that old attempt. Start a fresh attempt explicitly. |
| Finishing recording does not complete | Discard immediately or wait for the five-second failure. Incomplete output cannot be sent. |
| Track interrupted or recorder error | Reconnect/check the audio device and Start a new recording. No partial recording is automatically transmitted. |
| Window monitoring unavailable | Leave and reopen Command to create a fresh controller. Capture fails closed if observer initialization fails. |
| Transcript cannot be adopted | Check the visible UTF-8 byte count, nonempty text, native runtime, and whether planning is busy. The original text is preserved; oversized content is not silently shortened. |
| Edited command fails Prepare | The command must fit 8,192 UTF-8 bytes and pass existing native secret/control-character validation. Edit locally and prepare again. |
| OpenAI credential unavailable | Configure the native process environment privately, then relaunch as needed. Do not put credentials into the repository, transcript, or command. |
| Audit unavailable before Send | Inspect private storage availability/permissions; native transmission fails closed. |
| Transport or completion-audit warning | Read the warning before any explicit retry. A request may have reached the provider; a received transcript remains usable when completion audit fails. |

## Integration and security review

| Finding | Evidence |
| --- | --- |
| One component-owned capture controller; click-only Start | [VoiceInput](../apps/desktop/src/VoiceInput.tsx), [voice controller](../apps/desktop/src/voice-transcription.ts). Mount/unmount and repeated-attempt tests exercise cleanup. |
| Capture visibility, finalization, interruption and stale-callback gates | [Voice controller and lifecycle observers](../apps/desktop/src/voice-transcription.ts); WebView visibility/blur/pagehide plus fixed native main-window blur subscription. |
| Explicit transcript adoption, byte checks, state reset and busy rejection | [Transcript validator](../apps/desktop/src/transcript-handoff.ts), [Jarvis client](../apps/desktop/src/jarvis/jarvis.ts), [Command](../apps/desktop/src/jarvis/JarvisCommand.tsx). |
| Native transcription and planning authority unchanged | [Native voice](../apps/desktop/src-tauri/src/voice.rs), [native Jarvis review](../apps/desktop/src-tauri/src/jarvis/interpret.rs), [individual step gates](../apps/desktop/src/jarvis/JarvisPlan.tsx). These files are unchanged from the M44 baseline. |
| No new native privilege or frontend provider origin | [Main capability](../apps/desktop/src-tauri/capabilities/default.json), [production CSP](../apps/desktop/src-tauri/tauri.conf.json), [security tests](../apps/desktop/tests/production-security.test.ts). Existing core event permissions support lifecycle observation. |
| M44 remains local attention only | [Automation source](../apps/desktop/src/automations.ts) and native evaluator are unchanged. Existing isolation tests plus Command prefill/adoption tests make no scheduled AI/provider request. |
| Accessibility | Native buttons have explicit labels/types and existing focus styles; privacy/replacement notices are associated with their buttons. Recording state is announced separately from rapid timer ticks. Full VoiceOver validation remains outstanding. |

Testing uses existing `node:test` mocks, literal component rendering, a reused
Command hook harness, and the component mount-effect harness. The latter two
compile production TSX for injected hooks; they are not a browser/React DOM or
real macOS execution proof. No new harness/compiler path or dependency was added
for final integration. The former React placeholder warnings were resolved by a
function component stub in the existing Command harness. Production code does
not contain dynamic source execution.

The owner reports native macOS microphone recording/playback, compact voice UI,
minimize cancellation demonstrated by video, and application-switching
cancellation. Those checks were not independently repeated during this final
automated review. **Live provider transcription remains unverified without API
credentials in the reported test environment.** Live planning after transcription,
provider/account compatibility, exhaustive device/OS interruption cases, and
formal VoiceOver checks must not be inferred from mocked tests.

Remaining native limitations are unchanged: declared audio duration/MIME are
validated without decoding media, transcription consent uses the existing main
window and fixed confirmation marker, and cancellation does not revoke provider
work. Stronger recording-review binding, credential provisioning, live-provider
validation, signing/notarization, and distribution compatibility belong to
separate M46 production hardening. They are not completed M45 claims.

## Final local validation

Frontend commands ran from `apps/desktop`; Rust commands ran from
`apps/desktop/src-tauri`. Native validation was included because M45 relies on
the existing Tauri event permissions, transcription boundary and planner gates.
No Rust source, capabilities, dependencies, version, or release configuration changed.

| Check | Executed result |
| --- | --- |
| `node --test tests/voice-transcription.test.ts tests/jarvis.test.ts tests/intent-interpretation.test.ts tests/automations.test.ts tests/production-security.test.ts` | 207 passed; 0 failed/skipped. |
| `pnpm test` | 299 passed; 0 failed/skipped. |
| `pnpm build` | TypeScript and Vite production build passed. |
| Frontend test/build warnings | No runtime React warnings or Vite build warnings observed. Test titles mentioning warning behavior are not runtime warnings. |
| `cargo fmt --check` | Passed. |
| `cargo test --locked --offline` | 344 native tests passed; 0 failed/ignored. Main/doc-test targets contained zero tests. |
| `cargo check --locked --offline` | Passed using cached dependencies. |
| Documentation links | 28 local links/image targets across the modified Markdown files resolved; external URLs were not fetched. |
| Scope comparison with `main` | M44 production modules, native source/configuration/capabilities, CLI, CI, dependencies/lockfiles, navigation, and delegated action gates have no diff. |
| `git diff --check` | Passed. |

The branch is ready for PR preparation and CI review within this documented M45
scope. No commit, push, PR, merge, tag or release was performed in this review.
GitHub CI has not run for these uncommitted changes. Live provider compatibility
and production distribution readiness remain unverified follow-ups.

## Proposed future roadmap — not implemented in M45

| Proposed requirement | Work needed before implementation |
| --- | --- |
| Gemini free-tier development integration | Investigate an explicit native development adapter; verify eligible models, free-tier quotas/terms, credential handling, data policy, and response validation when scheduled. No current free-tier availability or capacity is promised. |
| Provider-independent AI architecture | Design reviewed native provider adapters with common bounded request/result contracts, separate credentials, allowlisted endpoints, and unchanged independent approval gates. |
| ElevenLabs or native TTS spoken replies | Evaluate a separately controlled speech-output path, voice/device support, playback interruption and privacy; cloud speech output needs its own outbound consent. |
| Natural conversational voice sessions | Define explicit session start/stop, turn boundaries, bounded buffers, interruption, idle timeouts, context review, and no voice-based action approval. |
| Explicit opt-in local “Hey GHOST” wake word | Evaluate local detection and a visible listening indicator with default-off consent. No detection engine or automatic capture trigger is implemented. |
| Background wake-word support while GHOST runs | Specify separately authorized listening while the app is running; no OS daemon, automatic launch, or background listener is added by M45. |
| Separate background-listener security/lifecycle policy | Review OS permissions, bounded local buffers, retention, focus/sleep/lock/device transitions, stop controls and audit metadata before changing today's foreground-only policy. Detection alone must not authorize provider sharing or actions. |

These are proposed requirements, not shipped capabilities, approved provider
integrations, or assigned delivery dates. M46 production hardening remains
separate from introducing these new features.
Existing `gemini` handoff drafts are offline artifacts, not a Gemini API integration.
