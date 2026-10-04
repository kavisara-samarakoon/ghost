# GHOST Desktop — Command Space

The published GHOST v0.4.0-alpha baseline is the Safe Desktop Action Requests checkpoint: a local macOS
workflow cockpit for reviewing projects, active sessions, memory, and artifacts
through a read-only snapshot. Native actions
include click-only Open/Reveal for approved generated artifacts and M29's local
pending action requests.
See the [release checkpoint](../../docs/release-v0.4.0-alpha.md) for historical scope
and versions. Current unreleased desktop capabilities also include M35 voice and
M36 intent below; versions remain unchanged.

From `apps/desktop`:

```sh
pnpm install --frozen-lockfile
pnpm tauri dev
```

Use `pnpm dev` for the browser preview; local memory and native actions are unavailable there.
Build and validate the local macOS alpha with:

```sh
pnpm build
pnpm test
pnpm tauri build
cd src-tauri
cargo fmt --check
cargo test --locked
cargo check --locked
```

The app and DMG are created under `src-tauri/target/release/bundle/` and remain ignored.
No Apple signing or notarization is configured. Snapshot/search/action code does not
write workflow files, execute shell/CLI commands, call AI/network services, read
`.env` files, or search source code. Native actions require an explicit user click
and revalidate allowlisted local artifacts.

M35 adds a separate **Voice Input** panel: explicitly start a recording (maximum
30 seconds / 8 MiB), stop and play it locally, then explicitly **Send to OpenAI
for transcription**. The native process uses `OPENAI_API_KEY` from its environment
and one fixed `gpt-transcribe` request to the OpenAI transcription endpoint.
Normally launched macOS apps may not inherit shell environment variables; GHOST
does not read credential files or store keys. Browser preview cannot record/send.
Audio and sanitized transcript stay in memory; the only voice write is content-free
`GHOST_HOME/desktop-voice-audit.jsonl`. Transcripts never create Action Requests,
invoke CLI/orchestration, or run workflow actions. No automatic retry occurs.

## M36 — Controlled Intent

The Command page offers **Prepare Interpretation → Review sanitized intent →
Send reviewed intent to OpenAI → Review proposal → optional Save Plan Draft**.
Typed text stays local until Send. Voice transcripts enter the intent draft only
through the explicit **Use transcript as intent** button; that copy is local and
still requires Prepare and Send. Static browser preview cannot prepare/send/save.

The native process makes at most one stateless `gpt-6.1-sol` Responses API request
with strict Structured Outputs. Only sanitized intent is sent; the selected project
alias stays local. No project/context/source files or tools are provided. Native
validation accepts only a 1–8 step inert plan using the existing four M34 actions,
or an empty-step clarification/unsupported result. No action or CLI runs.

Before explicit Save, intent and proposals remain in memory. Save creates a private,
exclusive `GHOST_HOME/intent-plans/<UTC-timestamp>-<uuid>.json` containing only
`version` and `steps`. The bound alias is displayed separately. Its file SHA-256
is distinct from M34's project-bound execution fingerprint: the owner must manually
use `ghost orchestrate preview <alias> --plan <path>` and, if appropriate,
`ghost orchestrate run <alias> --plan <path>` for independent validation and fresh
M34 confirmation. Saving never invokes these commands or creates Action Requests.

Only content-free metadata enters `desktop-intent-audit.jsonl`. Confirmed audit must
be durable before credential lookup or transmission. A received proposal or saved
plan is preserved if its completion audit fails, with a warning; GHOST never retries
or duplicates automatically. Storage rejects symlink redirection, hard links,
unsafe permissions, and entry/descriptor identity changes. New directories use
0700; files use 0600. Transcription and interpretation are the only two controlled
desktop OpenAI paths; both use the native process's `OPENAI_API_KEY`, with no
credential storage, redirects, proxies, or automatic retries.

## M29 — Safe Desktop Action Requests

The Command page offers **Prepare Action → Review Action Request → Save Request**
for `start_session` (goal), `add_session_note` (note), `generate_next_steps`, and
`create_handoff` (codex, chatgpt, gemini, or antigravity). Choose or type a project
alias. Aliases use the existing 1–128 lowercase letter/digit/hyphen format; a
request does not establish that a project or active session exists. Browser/sample
mode cannot save. Editing a reviewed request requires a fresh preview.

Rust validates the action, fields, provider, and reviewed preview before saving
`GHOST_HOME/action-requests/<UTC-timestamp>-<safe-id>.json` (default `~/.ghost`).
Each file has `pending` status and a safety notice. These are local pending drafts:
they do not execute CLI commands or mutate sessions, notes, outputs, next-step
drafts, handoffs, or other workflow records. The desktop does not automatically
publish, merge, deploy, tag, or release. The user must review and manually
run/approve real CLI workflow changes.
The desktop has no request consumer or automatic execution. CLI M32 separately
reviews/applies pending requests with fresh exact `APPLY <id>` confirmation.

The only writes are the request file and `desktop-action-audit.jsonl`, plus their
storage directories if missing. Audit events contain timestamp, event name,
action type, project alias, and request ID, excluding goal/note text. Requests and
audit files are created with mode 0600; new directories use 0700. Unix storage
uses descriptor-relative access, rejects symlinks and hard-linked audit files,
and never overwrites an existing request. File and parent-directory fsync plus
entry/descriptor checks guard completion. Audit appends are bounded to 16 MiB and
reject non-private modes/current-owner mismatches. Partial persistence remains
inert and must be inspected before retry. Other platforms fail closed. The parent
of a custom `GHOST_HOME` must already exist.

Goal/note text is limited to 8000 UTF-8 bytes. Recognizable secrets and unsupported
control characters are rejected using the existing redaction rules; this is not
complete secret detection, so do not enter secrets. Preview and payload retain
the reviewed text. Recent pending metadata is loaded with the snapshot, with at
most 10 entries shown (20 newest candidate filenames inspected, existing directory
and byte limits apply). Snapshot `safety` flags describe the read-only snapshot
operation; the separate request save command has this narrow write scope.

If a request saves but audit writing fails, the UI reports the saved path and
audit failure explicitly. Do not retry by creating a duplicate. This milestone
does not provide a transaction across the two files or an execution approval system.

## Production frontend boundary

Production CSP permits bundled scripts/styles/images, Tauri IPC, and `blob:` only
for local audio playback. It grants no remote script, iframe/object, or OpenAI
frontend origin. Capabilities are main-window-only with fixed native commands;
no generic shell/HTTP/filesystem permissions are granted. OpenAI calls remain
Rust-native. See [M37 readiness evidence](../../docs/release-readiness-m37.md) for
local packaging, distribution blockers, and untested live-provider behavior.
