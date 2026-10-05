# GHOST v0.5.0-alpha — Controlled AI, Voice & Safe Orchestration

Release-preparation source material for M38. This remains an **alpha prerelease**
intended for controlled local dogfooding/testing. Publication is a separate
human-reviewed operation; this document does not establish a tag, GitHub release,
release CI pass, final tagged build, or production readiness.

**Distribution limitation:** the local macOS candidate is not Developer ID signed
or notarized. Trusted public macOS distribution is blocked until signing and
notarization are separately arranged. No default installation path bypasses
Gatekeeper or changes system security settings.

**Live provider status: NOT TESTED.** M38 makes no real OpenAI request, uses no real
OpenAI credential, and transmits no microphone audio. Production claims require
separately authorized live-provider validation. An explicitly unsigned,
owner-controlled/local-dogfood alpha prerelease requires the owner's acceptance
of this disclosed limitation at the later release gate.

## 1. Versions

- Desktop package / Tauri / Cargo: `0.5.0-alpha`
- CLI package / `ghost version`: `0.5.0a0` (`GHOST 0.5.0a0`)
- Planned tag: `v0.5.0-alpha`

Python uses its normalized alpha version. This minor alpha increment covers the
substantial M31–M37 capabilities added after the previous published
[v0.4.0-alpha checkpoint](release-v0.4.0-alpha.md). M38 changes release metadata,
documentation and version validation, without changing runtime behavior.

## 2. Summary

GHOST combines a local workflow CLI with a macOS desktop companion. This candidate
adds explicit CLI request execution, bounded advisory AI review and finite local
orchestration, alongside reviewed desktop voice transcription and proposal-only
intent interpretation. AI output remains untrusted; the user retains authority.

## 3. Highlights

- Review pending desktop Action Requests and explicitly apply them from the CLI.
- Run finite allowlisted local plans after exact SHA-bound confirmation.
- Request one controlled advisory OpenAI project review after outbound preview.
- Capture and review short voice recordings before explicit transcription consent.
- Interpret typed or explicitly selected transcript text as strict proposals.
- Explicitly save inert M34-compatible plan drafts for independent CLI review.
- Guard desktop/CLI compatibility with shared contract fixtures; harden production
  CSP, project identity checks, and newer private storage/audit paths.

## 4. Action Request review/execution

M31 provides read-only `ghost request list/show` for pending desktop requests.
M32 adds `ghost request apply <id>` with exact, fresh `APPLY <id>` confirmation
after review. Only four local actions are available: `start_session`,
`add_session_note`, `generate_next_steps`, and `create_handoff`. Handoff providers
are `codex`, `chatgpt`, `gemini`, and `antigravity`.

Durable claim/completed/failed records prevent replay of an attempted request.
M37 pins and revalidates the reviewed project identity. Desktop request saving
still produces a pending local draft; it never applies that request.

## 5. Controlled AI review

M33 `ghost ai review <alias>` previews sanitized allowlisted context and the task.
Exact `SEND <alias> TO OPENAI` authorizes one bounded Responses API request using
an explicitly supplied model. The response becomes an advisory local draft only.
No tools, workflow execution, automatic retry, source scan, or Git scan is added.
Redaction is heuristic; review the outbound preview for sensitive material.

## 6. Safe orchestration

M34 accepts strict version-1 plans with 1–8 sequential steps drawn from the same
four actions. Full plan preview and exact `RUN <alias> PLAN <sha256>` confirmation
using the full execution fingerprint precede durable claim/audit and dispatch.
There is no retry, resume, output interpolation, or automatic rollback. Earlier
steps may remain applied after a later failure or interruption.

M32 and M34 remain the only local workflow execution mechanisms for their
respective request and plan contracts. Both require fresh exact confirmation.
Other ordinary CLI workflow commands remain explicitly invoked by the user.

## 7. Voice transcription

M35 capture starts only on an explicit push-to-talk click. Recordings stay in
memory, are limited to 30 seconds / 8 MiB, and support local playback/review.
Explicit Send consent authorizes one off-device transcription request to OpenAI
using `gpt-transcribe` at `POST /v1/audio/transcriptions`.

The visible transcript is untrusted text. It never automatically creates a
request, saves a plan, interprets intent, or executes a workflow. Real microphone
capture/provider end-to-end behavior is not exercised during M38.

## 8. Controlled intent interpretation

M36 accepts typed intent or a transcript the user explicitly selects with Use.
Prepare produces the exact outbound review; explicit Send authorizes one OpenAI
Responses request using `gpt-6.1-sol` and strict Structured Outputs.
Only sanitized intent is sent; project binding remains local.

The result is an untrusted proposal, without tools or execution authority. A
separate Save click may write an inert M34-compatible plan draft. Manual CLI M34
review and fresh SHA confirmation remain independent. No desktop execution
bridge exists, and saving a proposal is never execution approval.

## 9. M37 hardening

Shared cross-language fixtures test actual desktop request/plan output against
CLI consumption. Specific hardening includes M32 project identity pinning, M33
private draft/audit checks, durable desktop request saves, voice audit validation
before writable open, and M36 rejection of credential-looking aliases.

Production CSP limits frontend resources and IPC; existing narrow capabilities
grant no generic shell, HTTP, filesystem, or remote access. CI now checks rustfmt
and all release shell scripts. [M37 evidence](release-readiness-m37.md) records
that milestone's regression and packaging results; it is historical evidence,
not proof of this candidate's release CI or final tagged assets.

## 10. Safety and authority boundaries

**AI NEVER OWNS EXECUTION AUTHORITY.**

`USER INTENT → INTERPRET → PREVIEW → EXPLICIT CONFIRMATION → ALLOWLISTED ACTION → AUDIT`

The desktop cannot execute the GHOST CLI, spawn arbitrary processes, run shell
commands or Git, mutate GitHub, publish, deploy, merge, tag, release, directly
execute M32 requests, or directly execute M34 orchestration. Open/Reveal remain
click-only allowlisted operations for approved artifacts. Snapshot/search remain
read-only. AI responses and transcripts are proposal/advisory data only.

## 11. Network/privacy behavior

Snapshot/search, pending requests and inert plan saves remain offline. Desktop
voice and intent send data to OpenAI only after separate explicit consent; those
sent paths are not local-only. CLI M33 separately sends its reviewed task/context
after exact confirmation. The fixed provider paths are audio transcription and
Responses; no automatic fallback/retry or arbitrary endpoint is provided.

`OPENAI_API_KEY` comes from the process environment and is not stored by GHOST.
Native credentials do not reach React. Normally launched macOS apps may not
inherit shell environment variables; there is no Keychain integration.
Audits exclude freeform audio/transcript/intent/goal/note content, but workflow
records and saved drafts remain local plaintext. Do not enter secrets.

## 12. Known limitations

- Alpha prerelease for controlled testing, with current-machine Apple Silicon
  arm64 candidate validation only; no all-macOS compatibility claim.
- No Developer ID signing or notarization; Gatekeeper trusted-distribution
  limitations remain. Never disable Gatekeeper globally as an installation step.
- Live OpenAI account/API access and real microphone/provider end-to-end behavior
  are untested in M38. Documentation checks do not verify account access.
- GUI environment key availability can differ from a shell; no Keychain support.
- No automatic desktop workflow execution, GitHub/release/deployment automation,
  or AI execution authority.
- Heuristic redaction and local plaintext data require human privacy review.
- Local multi-file operations are not a database transaction. Interruption can
  leave partial state and require manual reconciliation; no global crash-recovery
  guarantee or orchestration rollback is provided.

## 13. Validation expectations

M38 requires synchronized versions; CLI pytest/Ruff, wheel/sdist build, pip check
and fresh installed-wheel smoke; frontend tests/build; locked Rust tests/checks
and rustfmt; all release-script syntax/mocks; focused shared contracts; actual
arm64 app/DMG build, metadata inspection and candidate checksum verification.
Record actual results separately from expectations. Local validation does not
establish future PR/release CI success or production readiness.

M38 local regression results: 1,145 CLI tests, Ruff, wheel/sdist build, pip check
and offline fresh-wheel smoke passed; 137 frontend tests and production build
passed; rustfmt, 133 native tests and locked Cargo check passed. All seven release
scripts passed syntax validation; mocks passed 59 tests plus 184 subtests. Focused
shared contracts passed 11 CLI cases and two native tests over all 11 fixtures.
Sandbox DNS/permission failures were resolved by approved normal-environment
validation, without runtime fixes or dependency-version changes.

The local arm64 app and `GHOST_0.5.0-alpha_aarch64.dmg` built successfully.
Bundle product/identifier, both `0.5.0-alpha` versions, microphone-only privacy
metadata, DMG integrity and candidate checksum sidecar verified. Read-only mounted
DMG app files matched the built app exactly. The automatic linker/ad-hoc signature
has no Team ID or sealed resources; strict signature verification and Gatekeeper
assessment both reject it with “code has no resources but signature indicates
they must be present.” This is an external blocker for trusted public macOS
distribution, explicitly disclosed for owner-controlled unsigned alpha review.

**PACKAGED INTERACTIVE SMOKE: NOT TESTED.** No clean normal isolated launch was
established; the app was not launched and macOS security settings were unchanged.
Only the current arm64 machine was validated. Bundle minimum macOS metadata is
10.13 while the executable's linker minimum is 11.0; older-OS support is unverified.

Official OpenAI documentation checked during M38 supports the configured
[transcription model](https://developers.openai.com/api/docs/models/gpt-transcribe)
and [transcription endpoint](https://developers.openai.com/api/docs/guides/speech-to-text),
[GPT-6.1 Sol](https://developers.openai.com/api/docs/models/gpt-6.1-sol),
[Responses](https://developers.openai.com/api/docs/guides/text), and
[strict Structured Outputs](https://developers.openai.com/api/docs/guides/structured-outputs?api-mode=responses).
This is contract documentation verification only; **LIVE PROVIDER: NOT TESTED**.

**PRODUCTION CLAIMS: LIVE PROVIDER VALIDATION REQUIRED.**

**OWNER-CONTROLLED / LOCAL-DOGFOOD ALPHA:** the later owner release gate must
explicitly accept the unverified live-provider state and unsigned package limits.

## 14. Planned release assets

- Primary GitHub prerelease asset: `GHOST_0.5.0-alpha_aarch64.dmg`
- Checksum sidecar: `GHOST_0.5.0-alpha_aarch64.dmg.sha256`

Preserve the established DMG + checksum policy. CLI
`ghost_cli-0.5.0a0-py3-none-any.whl` and `ghost_cli-0.5.0a0.tar.gz` are local
release-candidate packaging evidence, not planned GitHub assets or PyPI uploads.
No asset upload or publication occurs in M38.

**CANDIDATE ONLY — MUST BE RECOMPUTED FROM EXACT TAGGED COMMIT BEFORE PUBLICATION.**
Candidate hashes belong only in explicitly labelled candidate evidence. No final
tagged-release checksum is known or asserted here.

## 15. Final publication verification requirements

After separate human review: commit only approved preparation files, push the
release branch, open/review its PR, and wait for every CI gate. Use the reviewed
merge/tag helper with `v0.5.0-alpha`; none of these operations occurs in M38.

Before publication, independently verify local HEAD, tag and merged main commit
identity. Rebuild from the exact tagged commit, inspect the bundle metadata again,
recompute SHA-256, create the final basename-only `.sha256` sidecar and verify it.
Upload only those verified final DMG/checksum artifacts, create the GitHub release
as a prerelease, and verify uploaded digest/size and the release page. The M38
candidate must never automatically become the published asset.

Read-only M38 GitHub inspection reports `main` unprotected with no applicable
rulesets and required-check enforcement off: **RELEASE PROCESS FOLLOW-UP**.
Reviewed local release helpers and CI gates are not server-enforced branch
protection. No repository settings are altered. See the
[release workflow](release-workflow.md) for the separate human-controlled gates.
