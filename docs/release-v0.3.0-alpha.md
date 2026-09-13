# GHOST v0.3.0-alpha — Local MVP Desktop Checkpoint

Prepared release tag: `v0.3.0-alpha`. This checkpoint does not create a tag or publish a release.

- Desktop package, Tauri config, and Cargo package: `0.3.0-alpha`.
- CLI package and `ghost version`: `0.3.0a0` (Python's normalized alpha version).
- Product name: GHOST. CLI workflow and desktop safety behavior are unchanged.

## Summary

This alpha release turns the desktop app into a usable local-first workflow cockpit for
reviewing projects, sessions, memory, and artifacts while preserving strict safety boundaries.

## Highlights

- Command Space now acts as a local MVP cockpit.
- Projects Page v1 for reviewing and selecting loaded workspaces.
- Sessions Page v1 for active goals, notes, status, and related work.
- Artifacts Page v1 for reviewing generated drafts and reports.
- Memory Page v1 with explicit-submit local search.
- Read-only desktop snapshots of approved GHOST workflow metadata.
- Safe Open/Reveal actions for approved generated artifacts.
- Improved desktop CI and native safety validation.
- Official branding and a polished dark/cyan GHOST interface.

## Safety boundaries

- Desktop remains intentionally read-only for workflow writes.
- No shell execution from desktop.
- No CLI execution from desktop.
- No AI or network calls from desktop.
- No automatic publishing, deployment, merge, or release actions.
- Memory search requires an explicit submit.
- CLI remains the controlled path for creating sessions, outputs, context packs, handoffs,
  and update packs.

## Known limitations

- Creating or editing workflow data still requires the CLI.
- Desktop refresh may require reopening the app.
- Search results are limited to approved local GHOST memory locations.
- This alpha is intended for local dogfooding, not public production use.
- The macOS app and DMG are not Apple-signed or notarized.

## Release Assets & Verification

- Artifact: `GHOST_0.3.0-alpha_aarch64.dmg`
- Target: macOS Apple Silicon / `aarch64`
- SHA256: `3ae94ed728819d0d5e1c96da6aa309365692624352a2e27f0227ae546083b783`

Verify the downloaded artifact in Terminal:

```sh
shasum -a 256 GHOST_0.3.0-alpha_aarch64.dmg
```

This is an unsigned and not notarized alpha build. On first open, macOS may
require you to Control-click or right-click `GHOST.app`, choose **Open**, and
confirm **Open**.

## Validation checklist

- [x] CLI tests and Ruff checks.
- [x] Frontend production build and tests.
- [x] Rust native tests and checks.
- [x] Tauri macOS app and DMG package build.
- [ ] Main CI after merge.
- [x] Manual native dogfood validation before commit.
