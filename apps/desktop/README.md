# GHOST Desktop — Command Space

GHOST v0.3.0-alpha is a local macOS workflow cockpit for reviewing projects,
active sessions, memory, and artifacts through a read-only snapshot. Native actions
remain limited to click-only Open/Reveal for approved generated artifacts.
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
