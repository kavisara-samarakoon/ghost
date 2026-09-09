# GHOST Agent Instructions

## Project Identity

GHOST is a premium local-first macOS desktop application.

Tagline:

Secure Personal AI Workflow Coordinator

GHOST coordinates personal software, cybersecurity, networking, Codex handoff, validation, documentation, portfolio, and release-preparation workflows.

## Current Stack

- Tauri
- React
- TypeScript
- CSS
- pnpm
- Rust backend later
- SQLite later

Main app path:

apps/desktop

## Product Direction

GHOST must feel like:

- a real macOS desktop app
- minimal
- premium
- spacious
- calm
- secure
- local-first
- developer-focused
- cybersecurity-aware

## Visual Rules

Preserve:

- dark premium background
- deep navy / black surfaces
- strong white text contrast
- subtle cyan / electric blue accents
- thin-line glass UI
- macOS-style app window feeling
- clean spacing
- focused screens

Avoid:

- website dashboard style
- SaaS admin panel style
- game HUD style
- sci-fi poster style
- too much neon
- too many cards
- fake analytics clutter
- random charts
- crowded sidebars
- childish ghost branding
- horror skull feeling

## Logo Rule

Do not redesign the GHOST logo.

Use the approved logo only as a brand asset.

Until the real logo asset is added, use a simple temporary text or letter mark only.

## Current MVP Scope

GHOST v0.1.0 Desktop UI Foundation must include:

- desktop app shell
- Command Space screen
- Workflow Execution screen later
- static project/session data
- no real AI automation yet
- no database yet
- no unsafe terminal execution

## Coding Rules

- Keep components simple.
- Use TypeScript cleanly.
- Avoid unnecessary libraries.
- Avoid complex state management for now.
- Do not add authentication.
- Do not add cloud sync.
- Do not add backend APIs yet.
- Do not add terminal command execution yet.
- Do not modify Git history.
- Do not commit or push unless explicitly requested.

## Validation Commands

From apps/desktop:

    pnpm build
    pnpm tauri dev

Before suggesting a commit, make sure the build passes.
