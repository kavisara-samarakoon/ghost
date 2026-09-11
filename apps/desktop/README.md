# GHOST Desktop — Command Center Preview

Milestone 9 is a static frontend shell for GHOST v0.1.0, built with the existing
Tauri, React, TypeScript, and Vite stack. GHOST means **GitHub, Handoff, Operations,
Search, and Tracking** — Secure Personal AI Workflow Coordinator.

## Preview locally

From `apps/desktop`, with Node.js and pnpm installed:

```sh
pnpm install --frozen-lockfile
pnpm dev
```

Open the local URL printed by Vite. To validate the frontend:

```sh
pnpm build
pnpm preview
```

`pnpm build` checks TypeScript and creates frontend assets in `dist/`. It does
not compile or validate the native Tauri application.

## Explore the shell

- Command Center combines four sample project cards, a current session, a
  recommended review, recent drafts, safety boundaries, and doctor status.
- Select NEXORA, SentinelLite AI, ARM-SecNet, or Portfolio to change the sample
  session and drafts. Sidebar views retain that selection.
- Handoffs, Outputs, and Update Packs open readable sample drafts in a modal.
  Close with the close button or Escape; keyboard focus returns to the trigger.
- Doctor deliberately reports **Not run** and **Not checked**; no results are
  fabricated. Demo is a frontend walkthrough, not the CLI demo.
- The workflow strip illustrates project → session → output → handoff → next →
  update pack → doctor. It is not an execution progress indicator.

The layout supports the existing 1280 × 840 native window, its 1024 × 720 minimum,
larger desktop windows, and narrower browser previews. Content scrolls within the
desktop workspace. Keyboard focus, a skip link, and reduced-motion preferences
are supported. The existing temporary letter mark is retained; native icons are
unchanged.

## Frontend boundary

All content comes from `src/preview-data.ts`. View selection, project selection,
and open drafts use React state only and reset on reload. There is no persistence,
AI integration, application network request, shell execution, CLI invocation,
GitHub automation, filesystem scanning, or access to `~/.ghost` or real project
workspaces. Drafts are neither generated on disk nor sent anywhere.

Vite environment-file loading is disabled with `envDir: false`. Fonts are system
fonts, and icons are local inline SVGs. No dependencies were added. The existing
native Tauri scaffold is unchanged and is not invoked by this frontend.

## Source layout

- `src/App.tsx`: shell, focused views, shared panels, and draft dialog.
- `src/preview-data.ts`: typed fictional project, artifact, and workflow fixtures.
- `src/Icon.tsx`: small shared set of decorative line icons.
- `src/shell.css`: app layout, navigation, typography, and shared tokens.
- `src/App.css`: panels, draft preview, and responsive content layouts.
