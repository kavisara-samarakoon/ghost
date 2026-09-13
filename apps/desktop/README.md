# GHOST Desktop — Command Space

GHOST v0.3.0-alpha is a local macOS workflow cockpit for reviewing projects,
active sessions, memory, and artifacts through a read-only snapshot. Native actions
include click-only Open/Reveal for approved generated artifacts. M29 adds local
pending action requests for the v0.4.0-alpha direction; the package version is unchanged.
See the [release checkpoint](../../docs/release-v0.3.0-alpha.md) for scope and versions.

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
cargo test --locked
cargo check --locked
```

The app and DMG are created under `src-tauri/target/release/bundle/` and remain ignored.
No Apple signing or notarization is configured. Snapshot/search/action code does not
write workflow files, execute shell/CLI commands, call AI/network services, read
`.env` files, or search source code. Native actions require an explicit user click
and revalidate allowlisted local artifacts.

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
they do not execute CLI commands or mutate sessions, notes, outputs, or next-step
drafts. The user must review and manually run/approve real CLI workflow changes.
No request consumer or automatic execution is included.

The only writes are the request file and `desktop-action-audit.jsonl`, plus their
storage directories if missing. Audit events contain timestamp, event name,
action type, project alias, and request ID, excluding goal/note text. Requests and
audit files are created with mode 0600; new directories use 0700. Unix storage
uses descriptor-relative access, rejects symlinks and hard-linked audit files,
and never overwrites an existing request. Other platforms fail closed. The parent
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
