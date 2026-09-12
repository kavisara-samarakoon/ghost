# GHOST v0.2.0-alpha checkpoint

Prepared release tag: `v0.2.0-alpha`. This checkpoint does not create a tag or publish a release.

- Desktop package, Tauri config, and Cargo package: `0.2.0-alpha`.
- CLI package and `ghost version`: `0.2.0a0` (Python's normalized alpha version).
- Product name: GHOST. CLI commands and storage formats are unchanged.

Since the CLI-focused v0.1.0 release:

- Command Space desktop UI with real local project, active-session, and artifact snapshots.
- Allowlisted, click-only Open/Reveal actions and bounded, redacted local memory search.
- Local macOS app/DMG packaging and frontend plus native macOS safety CI.

Local alpha only; no Apple signing or notarization configured. Browser mode remains a preview.
Desktop snapshot/search/actions remain read-only: no shell or CLI execution, AI/network calls,
environment-file reads, source-code search, or workflow file writes. Native permissions remain narrow.
