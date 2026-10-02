# GHOST v0.4.0-alpha — Safe Desktop Action Requests

This document prepares the v0.4.0-alpha release checkpoint. It does not create a
tag or publish a release. M30 updates version metadata and corrects stale safety
documentation/UI wording; it does not change M29 Action Request behavior.

## Versions

- Desktop package / Tauri / Cargo: `0.4.0-alpha`
- CLI package / `ghost version`: `0.4.0a0` (`GHOST 0.4.0a0`)

Python uses the normalized alpha package version. CLI commands and workflow
storage behavior are unchanged by this checkpoint.

## M29 capability

The desktop Command page supports **Prepare Action -> Review Action Request ->
explicit Save Request**. Editing a reviewed request requires a fresh preview.

Supported request types:

- `start_session`: a project alias and goal
- `add_session_note`: a project alias and note
- `generate_next_steps`: a project alias
- `create_handoff`: a project alias and provider

Supported handoff providers are `codex`, `chatgpt`, `gemini`, and `antigravity`.
These are requested operations only; saving does not perform them.

## Safety boundary

Requests are pending local drafts. No request consumer or automatic executor
exists. Saving a request does not execute the GHOST CLI or mutate sessions, notes,
outputs, next-step drafts, handoffs, or other workflow records. The desktop does
not execute shell commands, make AI/network calls, or automatically publish,
merge, deploy, tag, release, or otherwise mutate GitHub.

Request and audit storage use the existing narrow local write path:

- `GHOST_HOME/action-requests/<UTC-timestamp>-<safe-id>.json`
- `GHOST_HOME/desktop-action-audit.jsonl`
- Their storage directories, when needed (`GHOST_HOME` defaults to `~/.ghost`)

Rust revalidates the action, fields, provider, and reviewed preview before saving.
Request/audit files use mode 0600 and new directories use 0700. Descriptor-relative
Unix storage rejects symlinks and hard-linked audit files and never overwrites an
existing request. Audit metadata excludes goal/note text.

Snapshot/search remain read-only; their `no_file_writes` safety flags describe
those operations. Open/Reveal remain explicit, allowlisted native actions for
approved generated artifacts.

Pending request files must be treated as untrusted drafts by any future consumer
and require fresh human confirmation before any workflow change. A saved preview
or request is not execution approval.

## Known limitations

- Browser/sample mode cannot save requests. Native local mode is required.
- Alias validation does not establish that a project or active session exists.
- Goal/note text is limited to 8000 UTF-8 bytes. Recognizable secrets and
  unsupported control characters are rejected, but detection is heuristic;
  do not enter secrets. Reviewed text remains in the local request draft.
- Recent pending metadata shows at most 10 entries after inspecting the 20 newest
  candidate filenames, subject to existing directory and byte limits.
- Safe request saving fails closed outside Unix; a custom `GHOST_HOME` parent
  must already exist.
- Request saving and audit appending are not one transaction. If the request
  saves but auditing fails, the UI reports the saved path and audit failure;
  inspect them rather than creating a duplicate request.
- No execution approval system or request processing is included. Actual workflow
  changes require separate human review and manually approved CLI use.
- Apple signing and notarization are not configured for the macOS alpha.

## Release assets and verification

The expected Apple Silicon asset filename is `GHOST_0.4.0-alpha_aarch64.dmg`.
The final v0.4.0-alpha DMG has not been built as part of this preparation. No
SHA256 value is recorded here; checksum/asset verification is recorded only after
the final build.

This checkpoint does not claim that M30 post-merge `main` CI has passed or that a
v0.4.0-alpha tag or release exists. Publication remains a separate reviewed step
under the [release workflow](release-workflow.md).

The root README retains the currently published v0.3.0-alpha download, release,
and checksum information until v0.4.0-alpha is actually published. Historical
v0.2/v0.3 release documents remain unchanged.
