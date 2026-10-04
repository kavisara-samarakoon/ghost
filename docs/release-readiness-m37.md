# M37 release-readiness evidence

Date: 2026-10-04. Branch: `feature/m37-production-hardening`.
Expected base and unchanged final HEAD: `c96e3ffc5a7fc9c89a962f60d23233a4a3b243b0`.
The initial tree was clean; this report describes uncommitted M37 changes, not an
update to main or a published release. Desktop stays `0.4.0-alpha`; CLI stays
`0.4.0a0`. Historical release notes remain unchanged.

## Scope and verdict

**READY FOR RELEASE PREPARATION** after normal review/merge of these changes.
This is **not a production-readiness or public-distribution approval**. Code,
contract, regression, packaging and documentation evidence supports starting a
separate release-preparation task. Apple Developer signing/notarization and
live-provider/real-device validation remain prerequisites for distribution claims.
No next release version is chosen.

Audited: active CLI request/review/orchestration/dispatcher and local workflow
storage; desktop Action Requests, voice and intent controllers; native snapshot,
artifact/search, request/voice/intent storage and fixed HTTP clients; Tauri
configuration/permissions; CI, release helpers/tests, lockfiles and version sources.

## Findings and actions

| Finding | Classification | Evidence / action |
| --- | --- | --- |
| Production CSP was null | CONFIRMED — FIX REQUIRED | Installed Tauri schema and official documentation confirm protection requires a policy. Added bundled-only policy, IPC connect sources, media-only blob playback and explicit object/frame restrictions; 3 frontend configuration regressions pass. |
| Template Cargo metadata | CONFIRMED — FIX REQUIRED | Replaced `A Tauri App`/`you` with GHOST description and Kavisara Samarakoon. Version/name/identifier unchanged. |
| CI missed two release scripts and rustfmt | CONFIRMED — FIX REQUIRED | Added rustfmt installation/check and explicit fail-fast loop over all seven shell scripts. A single `bash -n` with multiple filenames only checks the first. |
| Active documentation lagged M31–M36 | CONFIRMED — FIX REQUIRED | Corrected five active documents; preserved all historical release notes. |
| No shared producer/consumer contract proof | CONFIRMED — FIX REQUIRED | Added 7 request fixtures and 4 plan fixtures, consumed by both Rust producer and Python consumer tests. |
| Desktop accepted noncanonical UTC request timestamps | CONFIRMED — FIX REQUIRED | CLI requires 9 fractional digits; desktop previously accepted broader RFC3339. Reject noncanonical fractions/offsets, year zero, leap seconds before save. |
| Desktop request storage lacked owner/full mode, chain/file identity, bounds and parent fsync checks | CONFIRMED — FIX REQUIRED | Added narrow descriptor-relative checks around durable operations; never unlink a possibly replaced name on failure. Saved inert drafts survive final audit failure; no overwrite or automatic duplicate. |
| M32 review pinned request/home but not project binding | CONFIRMED — FIX REQUIRED | Freeze registered record/root/workspace identity and validate project metadata before claim and immediately before dispatch. Replaced roots/workspaces, changed registry/metadata and newly resolved aliases fail closed. |
| M33 draft save lacked private directory/file identity checks and audit size bound | CONFIRMED — FIX REQUIRED | Verify draft chain/file and frozen identity around fsync; fsync created parents; cap security-gate audit appends at 16 MiB. Tests inject replacement/mode failures without network retry. |
| Voice audit only checked existing permissions after writable open | PARTIAL / NEEDS HARDENING — ADDRESSED | Added pre-open validation consistent with M36 so opening cannot mask unsafe special mode bits. Existing symlink/hardlink/FIFO/permission regressions retained. |
| M36 credential-looking aliases could enter audit and were rejected by M34 | CONFIRMED — FIX REQUIRED | Native alias validation now uses existing redactor; prepare/send/save reject these bindings. No ordinary alias semantics changed. |
| Execution authority / fixed network paths | CONFIRMED — ALREADY SAFE | Exact fresh CLI confirmations, fixed four-way dispatcher, inert desktop output, no execution bridges; existing failure tests and static inspection pass. |
| Release accelerator helper coverage | CONFIRMED — ALREADY SAFE | Existing tests cover ordinary merge and finish ordering, two gates, changed head/CI, queued/failed merge and partial sync. No runtime release script edits needed. |
| Process/compile/spawn/environment scan hits | FALSE POSITIVE | `std::process::id` reads a PID; `re.compile` builds regexes; `spawn_blocking` runs bounded native operations; TS `this.env` is injected controller environment. No process launch or dotenv loader. |
| Clippy style warnings / broader utility refactor | OPTIONAL — DO NOT CHANGE | Clippy reports 2 existing `unused_unit` warnings in voice/intent path-validation code. Not added to CI; no suppression or unrelated style cleanup. |
| Apple distribution trust | EXTERNAL RELEASE BLOCKER | Only linker ad-hoc signature, no Team ID or sealed resources; no Developer ID/notarization. Gatekeeper does not accept the bundle. |
| Live API/account and hardware behavior | EXTERNAL RELEASE BLOCKER for production claims / NOT TESTED | Official contracts checked; no real API/key/audio, real microphone or packaged interactive consent smoke performed. |

## Cross-component proof

Action Request: `contracts/action-request-v1/` has all four actions and all four
handoff providers (7 records). Native `shared_contract_requests_are_exact_desktop_saved_output`
prepares each action, substitutes deterministic machine ID/time, saves through the
actual desktop storage implementation and compares the entire persisted JSON with
the shared fixture. Python consumes each same fixture through strict parser,
lookup, held review and exact-confirmed apply using workflow spies (7 cases).
This tests ID/time/alias/status, preview/safety text and exact payload shapes.

Plan: `contracts/orchestration-plan-v1/` contains 4 plans, each including all four
actions and a different allowlisted handoff provider. Native
`shared_contract_plans_are_exact_native_validated_saved_bytes` locally validates
the proposal and explicit save, persists through actual M36 storage/audit, compares
exact saved UTF-8 bytes with the same fixture and verifies file SHA. Python reads
each same file through M34 parser/preparation/CLI preview (4 cases). No alias,
summary, model or AI metadata enters the file. M34's alias-bound execution
fingerprint is deliberately different from file SHA. Unknown fields/actions/providers
are rejected by the existing strict native/Python schema tests.

## Security boundaries and limits

AI never owns execution authority. Voice → visible transcript → explicit local
Use → Prepare → exact outbound review → explicit Send → validated inert proposal
→ explicit optional Save → STOP. Execution requires a separate manual M34 review
and exact full SHA confirmation. Pending Action Requests independently require
fresh exact M32 confirmation. Shared dispatcher remains unchanged with all four
exact action/payload pairings, including `EmptyPayload` for `generate_next_steps`.

Only application network paths: CLI M33 HTTPS `api.openai.com:443/v1/responses`;
native M35 `https://api.openai.com/v1/audio/transcriptions` (`gpt-transcribe`);
native M36 `https://api.openai.com/v1/responses` (`gpt-6.1-sol`, strict Structured
Outputs). Hosts/paths are application constants, redirects/proxies/retries disabled
or unsupported. HTTP bodies are capped at 256 KiB; M35 audio is capped at 8 MiB /
30 seconds; transcript at 64 KiB; M36 intent at 8 KiB, summary at 2 KiB and goal/note
at 8000 bytes. M33 task is capped at 16 KiB and total logical input at 128 KiB.
M36 sends only sanitized intent, with project binding local. No source/context,
conversation, tools or prior response chaining is sent. Native credentials never
reach React, audits, persisted drafts or returned errors. Missing/invalid keys fail
without transport. GUI apps may not inherit `OPENAI_API_KEY` from a shell.

Production CSP: `default-src 'self'; script-src 'self'; style-src 'self'; img-src
'self'; media-src 'self' blob:; connect-src ipc: http://ipc.localhost; object-src
'none'; frame-src 'none'; base-uri 'none'; form-action 'none'`. No wildcard,
unsafe-eval/unsafe-inline, remote scripts or OpenAI frontend origin. Installed
Tauri codegen supplies bundled script/style hashes/nonces. Capability set remains
main-window `core:default`, `ghost-local-artifacts`, `ghost-controlled-input`; no
generic shell/HTTP/fs capability or remote IPC domain. Approved Open/Reveal remains
an allowlisted native artifact operation, not a shell/process execution bridge.

Newer private write paths use no-follow descriptor traversal, regular/single-link
owned 0600 files, 0700 private directories, exclusive creates/native no-overwrite
moves, entry/descriptor identity and file/parent fsync. Audits omit freeform
intent/goal/note/transcript/audio/key data. Required pre-network/pre-dispatch audit
failure blocks work; post-provider audit failures preserve in-memory results or
saved drafts and warn without retry. M32/M34 retain claims/uncertain terminal
states instead of replay, resume or orchestration rollback.

These are not global filesystem transactions or a defense against an all-powerful
hostile same-user process. Legacy domain updates use existing atomic replacement
and recovery semantics, and not every legacy audit append has parent fsync.
Identity verification covers defined checkpoints, not every subsequent instant.
Redaction is heuristic. Interrupted/multi-file local actions can leave partial
state; manual reconciliation remains required. No broad legacy-I/O rewrite was
introduced in this milestone.

Official contract references checked without a provider request:
[Tauri CSP](https://v2.tauri.app/security/csp/),
[GPT-Transcribe](https://developers.openai.com/api/docs/models/gpt-transcribe),
[GPT-6.1 Sol](https://developers.openai.com/api/docs/models/gpt-6.1-sol),
[Structured Outputs](https://developers.openai.com/api/docs/guides/structured-outputs).
Documentation compatibility is not live account/endpoint verification.

## Validation evidence

Local platform: arm64 macOS; CLI Python 3.11.15; Node 26.8.1; pnpm 12.3.4;
Rust 1.98.1. CI additionally runs CLI Python 3.14; that matrix was not run locally.

| Area | Actual command | Result |
| --- | --- | --- |
| Focused CLI hardening | `./.venv/bin/pytest tests/test_action_requests.py tests/test_orchestration.py tests/test_ai.py -q` | 814 passed |
| Full CLI, final CLI code | `./.venv/bin/pytest -q` | 1145 passed, 14.85 s; run once |
| CLI lint | `./.venv/bin/ruff check .` | PASS |
| Python packaging | `./.venv/bin/python -m build` and `--no-isolation` variant | PASS: sdist + wheel; isolated build fetched declared setuptools with approval after sandbox DNS failure; no project/global dependency changes |
| Dependency integrity | `./.venv/bin/python -m pip check` | No broken requirements |
| CLI command smoke | `ghost version`, root/request/orchestrate/ai/doctor help | 6 commands pass without storage writes |
| Isolated local smoke | init; temporary registration; session start/status/note; next/context/handoff; doctor; shared-plan orchestration preview | 10 commands pass; combined 16-command smoke, only synthetic temp state |
| Fresh wheel smoke | Temporary venv, `pip install --no-index --no-deps <wheel>`; version/root/orchestrate help | PASS; actual module loaded from installed wheel, dependencies reused from validated CLI environment |
| Frontend | `pnpm test` | 136 passed |
| Frontend production build | `pnpm build` | PASS; also rerun automatically by final package build |
| Native format | `cargo fmt --check` | PASS |
| Full native | `cargo test --locked` | Final 133 passed; rerun after final alias hardening |
| Native check | `cargo check --locked` | PASS |
| Optional Clippy | `cargo clippy --locked --all-targets -- -D warnings` | NOT GREEN: 2 pre-existing unused-unit style warnings; omitted from CI |
| Release syntax | `bash -c 'for script in scripts/release/*.sh; do bash -n "$script" || exit 1; done'` | PASS: all 7 scripts |
| Release mocks | `apps/cli/.venv/bin/python -m pytest scripts/release/tests -q` | 59 tests + 184 subtests passed, 288.16 s |
| Explicit CLI contracts | `./.venv/bin/pytest tests/test_action_requests.py tests/test_orchestration.py -k shared_contract -q` | 11 passed |
| Explicit native contracts | `cargo test --locked shared_contract -- --nocapture` | 2 tests passed, exercising all 11 fixtures |
| Root whitespace | `git diff --check` | PASS |
| Artifact hygiene / versions | status, check-ignore, protected-source diffs and version field inspection | PASS; no generated build artifacts tracked/untracked; versions unchanged |

Full application suites include the existing no-auto-send/save/execute, exact
confirmation, malformed/tool output, response bounds, audit privacy, no-retry,
partial success and filesystem redirection tests. New coverage: 22 CLI cases,
3 frontend cases and 7 native tests (parameterized attack loops exercise more cases).

## Local package evidence

Final `pnpm tauri build` passed, producing both bundles. The first sandboxed attempt
built the app but could not finish DMG; the approved out-of-sandbox build and final
post-alias rebuild succeeded. API and Apple/updater signing credential environment
variables were removed from build children. No developer signing/notarization
credentials or upload were used. The app was not launched.

- Wheel: `apps/cli/dist/ghost_cli-0.4.0a0-py3-none-any.whl`, 78,042 bytes.
- Wheel SHA-256: `e1db7a5fb42c66dc718c5f6148465f030b9a152fdad45fc17082866fc319cf08`.
- Source archive: `apps/cli/dist/ghost_cli-0.4.0a0.tar.gz`.
- App: `/Users/kavisara/IdeaProjects/ghost/apps/desktop/src-tauri/target/release/bundle/macos/GHOST.app`; total regular-file payload 10,557,643 bytes (not filesystem allocation).
- Executable: `Contents/MacOS/desktop`, 8,577,792 bytes; `file` reports Mach-O 64-bit arm64, matching this machine.
- DMG: `/Users/kavisara/IdeaProjects/ghost/apps/desktop/src-tauri/target/release/bundle/dmg/GHOST_0.4.0-alpha_aarch64.dmg`, 8,883,262 bytes.
- DMG SHA-256: `fc98f8e3ff8e2d553175f437fe9783691a888fe020ed3dd5dd668b12d0cb3eba` (local artifact only, not a published release checksum).
- `CFBundleIdentifier`: `com.kavisara.ghost`.
- `CFBundleShortVersionString` and `CFBundleVersion`: `0.4.0-alpha`.
- `NSMicrophoneUsageDescription`: “GHOST uses the microphone only when you explicitly start a voice recording.” No camera privacy key.
- `codesign -dv --verbose=4`: `Signature=adhoc`, linker-signed, `TeamIdentifier=not set`, `Info.plist=not bound`, `Sealed Resources=none`. This is not Developer ID signing.
- Final read-only `spctl -a -vv`: exit 1, “code has no resources but signature indicates they must be present”; no accepted Gatekeeper result, settings unchanged.
- Declared bundle minimum macOS: 10.13; linker minimum is 11.0, and only arm64/current-machine packaging was validated. No older-OS compatibility claim.

## Readiness matrix and required follow-up

| Area | Status | Reason |
| --- | --- | --- |
| Code | PASS | Full regressions and focused failure tests pass; no new feature/action/provider. |
| Security boundaries | PASS WITH FOLLOW-UP | Hardened checkpoints and authority/privacy tests pass; manual packaged UI/CSP/microphone validation and documented legacy-I/O limitations remain. |
| Cross-contract compatibility | PASS | Shared actual producer output and consumer acceptance/preview/confirmed mocked dispatch proof. |
| Local packaging | PASS | Wheel/sdist, frontend, arm64 app and DMG produced. |
| Public macOS distribution | EXTERNAL BLOCKER | No Developer ID signing or notarization; Gatekeeper assessment unsuccessful. |
| Live OpenAI compatibility | NOT TESTED | Official model/API/schema docs checked, but no live request/account/key/media verification. |
| Documentation | PASS | Active scope/authority/release workflow corrected; historical checkpoint preserved. |
| CI / release tooling | PASS WITH FOLLOW-UP | Required local gates pass and coverage complete; remote PR CI/branch rules not inspected; Clippy remains optional follow-up. |
| Overall | PASS WITH FOLLOW-UP | Ready for separate release preparation, not publication or production claims. |

Release preparation must review/merge this tree, run independent PR CI, deliberately
select release metadata later, arrange Developer ID/notarization, validate packaged
UI/privacy/capture on a real Mac and perform separately authorized live-provider
smoke with synthetic intent/audio. None of those release mutations occur here.

## Final security review — explicit answers

| # | Question | Answer |
| --- | --- | --- |
| 1 | Desktop executes workflow actions? | No; only inert saves, approved artifact operations and controlled interpretation/transcription. |
| 2 | Desktop invokes GHOST CLI? | No. |
| 3 | Desktop invokes shell/process commands? | No shell/process-launch API; PID lookup and bounded spawn_blocking are not process execution. |
| 4 | Desktop invokes Git/GitHub? | No. |
| 5 | AI output reaches dispatcher directly? | No; a separately human-reviewed CLI file and fresh confirmation are required. |
| 6 | Voice auto-triggers interpretation? | No; explicit local Use, Prepare and Send are separate. |
| 7 | Interpretation auto-saves? | No. |
| 8 | Saved plan auto-executes? | No. |
| 9 | Pending request auto-applies? | No. |
| 10 | M32 fresh exact confirmation? | Yes: APPLY <id>. |
| 11 | M34 fresh exact SHA confirmation? | Yes: RUN <alias> PLAN <full SHA-256>. |
| 12 | Application network paths? | Only M33 Responses review, M35 audio transcription and M36 Responses interpretation. |
| 13 | Hosts/paths fixed? | Yes, api.openai.com and the two fixed HTTPS paths above. |
| 14 | Redirects disabled? | Yes; native policy none; stdlib M33 does not follow redirects. |
| 15 | Proxies disabled? | Yes; native no_proxy; M33 direct HTTPSConnection ignores proxy environment. |
| 16 | Retries disabled? | Yes, one explicit transport invocation; no fallback/loop. |
| 17 | Response bodies bounded? | Yes, 256 KiB for all three provider paths. |
| 18 | Credential reaches frontend? | No. |
| 19 | Credential reaches app audit/disk? | No; header-only provider credential and explicit echoed-key sanitization. |
| 20 | Project/source content reaches M36? | No, only sanitized human intent. |
| 21 | Desktop request → M31/M32 proven? | Yes, 7 shared fixtures with actual native save and strict CLI consumption/confirmed fake dispatch. |
| 22 | M36 plan → M34 proven? | Yes, 4 exact native-saved fixtures consumed by M34 parser/preparation/preview. |
| 23 | Actions/providers synchronized? | Yes, shared fixtures exercise all four actions/providers; both strict schemas test invalid enums. |
| 24 | Unknown fields/actions/providers cross stored contracts? | No; strict action/step shapes and emitted JSON exclude them; CLI rejects unknown top-level fields. |
| 25 | Symlink replacement redirects sensitive writes? | Tested hardened write paths reject redirection at checked boundaries; no blanket hostile-process guarantee for legacy domain I/O. |
| 26 | Hardlinks defeat private/audit checks? | No for reviewed security-critical files; regular/single-link entry and descriptor checks remain enforced. |
| 27 | New files private? | Yes, 0600 files / 0700 private storage directories. |
| 28 | Exclusive/no-overwrite preserved? | Yes; exclusive creation and native no-replace lifecycle transitions. |
| 29 | Durability documented honestly? | Yes; fsync gates and partial-failure semantics, no global transaction/crash-recovery claim. |
| 30 | Ambiguous failure auto-duplicates work? | No retry, replay, resume or automatic duplicate save. |
| 31 | Production CSP active/least privilege? | Yes in built config/binary and tests; actual interactive packaged behavior remains manual follow-up. |
| 32 | Frontend CSP permits OpenAI? | No. |
| 33 | Unnecessary shell/http/fs capabilities? | No; fixed main-window commands only, no generic plugin grants. |
| 34 | Microphone permission limited? | Yes, microphone-only usage description; no camera or new entitlement. |
| 35 | Static browser inert? | Yes for native inputs/actions; existing frontend tests pass. |
| 36 | CLI wheel builds? | Yes. |
| 37 | Frontend builds? | Yes. |
| 38 | Tauri app builds? | Yes. |
| 39 | DMG builds? | Yes, approved local build. |
| 40 | Architecture correct? | Yes, arm64 on arm64 machine. |
| 41 | Bundle metadata correct? | Established identifier, unchanged alpha versions and microphone description verified; distribution metadata/signature still requires separate preparation. |
| 42 | Apple Developer signing configured? | No; only automatic linker ad-hoc signature. |
| 43 | Notarization configured? | No. |
| 44 | Absence reported as blocker? | Yes, public distribution EXTERNAL BLOCKER. |
| 45 | Syntax checks cover all helpers? | Yes, fail-fast loop over all seven current scripts. |
| 46 | merge/finish tests exist? | Yes; existing mocked release suite passes including their gates/failure paths. |
| 47 | Merge/tag human gated? | Yes; ordinary merge and release merge/tag require distinct exact phrases; finish has two gates. |
| 48 | M37 no release mutation? | Yes; no commit, push, PR, merge, tag, release, upload, deployment or version bump. |

## Exact changed files and purpose

| File | Reason |
| --- | --- |
| `.github/workflows/ci.yml` | Check every release shell script; install rustfmt and gate native formatting. |
| `AGENTS.md` | Correct active CLI/desktop authority boundaries. |
| `README.md` | Correct published checkpoint vs unreleased capabilities and safety/CI statements. |
| `apps/cli/README.md` | Document current M31–M34 confirmations, storage, and failure boundaries. |
| `apps/cli/src/ghost_cli/ai.py` | Verify private draft identities/directory chain; bound audit appends; fsync created parents and revalidate local phases. |
| `apps/cli/src/ghost_cli/request_execution.py` | Pin/revalidate registered project, root/workspace identities and recorded metadata during review/apply. |
| `apps/cli/tests/test_action_requests.py` | Consume shared desktop contracts with mocked confirmed dispatch; test project redirection. |
| `apps/cli/tests/test_ai.py` | Test bounded/private audits and post-fsync draft/file/directory replacement. |
| `apps/cli/tests/test_orchestration.py` | Consume exact M36 saved plan fixtures through M34 parsing/preparation/preview. |
| `apps/desktop/README.md` | Clarify current capabilities, CLI execution separation, CSP and storage durability. |
| `apps/desktop/src-tauri/Cargo.toml` | Replace template description/author without changing package name/version. |
| `apps/desktop/src-tauri/src/intent.rs` | Reject credential-looking bound aliases before prepare/send/save or content-free audits. |
| `apps/desktop/src-tauri/src/intent/tests.rs` | Compare actual saved plan bytes against shared fixtures; reject secret-like alias bindings. |
| `apps/desktop/src-tauri/src/snapshot/requests.rs` | Require canonical nanosecond UTC timestamps compatible with CLI consumer. |
| `apps/desktop/src-tauri/src/snapshot/requests/storage.rs` | Durable private descriptor-relative saves/audits with identity checks, bounds, parent fsync and safe partial failures. |
| `apps/desktop/src-tauri/src/snapshot/requests/tests.rs` | Producer contract and adversarial storage/timestamp regression proof. |
| `apps/desktop/src-tauri/src/voice/storage.rs` | Check existing unsafe audit mode/owner/type/link count before writable open. |
| `apps/desktop/src-tauri/src/voice/tests.rs` | Prove unsafe existing audit modes fail before writable-open checkpoint. |
| `apps/desktop/src-tauri/tauri.conf.json` | Enable least-privilege production CSP without version/identifier change. |
| `apps/desktop/tests/production-security.test.ts` | Guard CSP, main-window capabilities, fixed bundle identity/version and microphone-only privacy. |
| `contracts/action-request-v1/add_session_note.json` | Shared synthetic Action Request v1 producer/consumer fixture. |
| `contracts/action-request-v1/create_handoff-antigravity.json` | Shared synthetic Action Request v1 producer/consumer fixture. |
| `contracts/action-request-v1/create_handoff-chatgpt.json` | Shared synthetic Action Request v1 producer/consumer fixture. |
| `contracts/action-request-v1/create_handoff-codex.json` | Shared synthetic Action Request v1 producer/consumer fixture. |
| `contracts/action-request-v1/create_handoff-gemini.json` | Shared synthetic Action Request v1 producer/consumer fixture. |
| `contracts/action-request-v1/generate_next_steps.json` | Shared synthetic Action Request v1 producer/consumer fixture. |
| `contracts/action-request-v1/start_session.json` | Shared synthetic Action Request v1 producer/consumer fixture. |
| `contracts/orchestration-plan-v1/antigravity.json` | Shared synthetic M36/M34 version-1 plan fixture. |
| `contracts/orchestration-plan-v1/chatgpt.json` | Shared synthetic M36/M34 version-1 plan fixture. |
| `contracts/orchestration-plan-v1/codex.json` | Shared synthetic M36/M34 version-1 plan fixture. |
| `contracts/orchestration-plan-v1/gemini.json` | Shared synthetic M36/M34 version-1 plan fixture. |
| `docs/release-readiness-m37.md` | Record integrated audit evidence, limits, package inspection and readiness verdict. |
| `docs/release-workflow.md` | Document ordinary merge/finish helpers, two human gates, current CI and all-script syntax checking. |

## Confirmations

No real OpenAI request, real API credential use, real microphone transmission,
real ~/.ghost access, real project mutation, or real workflow data mutation outside
isolated synthetic tests/temp smoke. No application shell/CLI or Git/GitHub
execution path added, no Action Request/orchestration auto-execution, no GitHub
mutation, commit, push, PR, merge, tag, release/upload, deployment, history change,
or version bump. Repository/tooling validation commands are not product execution
capabilities. Generated build outputs remain ignored; stop uncommitted.
