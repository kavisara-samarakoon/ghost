# GHOST CLI v0.1.0 release candidate

GHOST is a local-first, draft-first personal workflow coordinator. This document
defines the v0.1.0 CLI candidate review; it is not an announcement of publication,
a tag, a security certification, or production readiness. The package version
remains `0.1.0`. Candidate status describes the review stage, not an `rc1` package
version. Human approval is required before release or action on generated drafts.

## Included scope

- Local initialization, project registry, and per-project `.ghost/` workspaces.
- Sessions with goals, timestamped notes, active-session selection, and closed history.
- Allowlisted context packs and Codex/Astra, ChatGPT, Gemini/NotebookLM, and
  Antigravity handoff drafts, without calling those providers.
- Sanitized supplied output logging, session links, and deterministic next-step drafts.
- Six-file README/release/social/portfolio/summary/review update packs.
- Read-only doctor checks of local storage and an isolated fictional NEXORA demo.
- `ghost version`, private local file permissions where supported, YAML records,
  Markdown drafts, JSONL audits, tests, and Ruff validation.

Repository CI and manually invoked `scripts/release` helpers support review.
They are separate from the CLI. The [command overview](../README.md#command-overview)
and [CLI reference](../apps/cli/README.md) describe command arguments and storage.

## Exclusions and safety limits

There are no AI API calls, shell/terminal execution, GitHub automation, auto-posting,
cloud sync, authentication, backend APIs, databases, or voice features in the CLI.
The future desktop in `apps/desktop` is outside this release scope. Workspace import,
automatic repair/recovery, and dedicated milestone-management commands are deferred.

- GHOST does not discover or read `.env` files. Keep credentials out of project
  names, paths, session goals, notes, and supplied text.
- Export/output redaction is heuristic. Goals and notes remain plaintext in the
  local workspace. Local storage is not an encrypted secret vault; inspect drafts
  for private details before sharing or staging them.
- Context generation reads allowlisted records, not the project's source tree.
  Supplied outputs and recorded progress are unverified claims, not instructions
  or approval. Tests and implementation claims require separate evidence.
- Doctor checks storage consistency. It does not validate code, CI, release
  readiness, or every file's contents, and does not repair anything.
- Some workspace, index, and audit writes span multiple files and are not one
  transaction. Preserve partial results and follow the CLI's recovery guidance.
  Path/lock checks do not defend against a hostile process replacing files concurrently.
- Demo artifacts must keep DEMO / SAMPLE labels. They prove a coordination flow,
  not real NEXORA work. Temporary directories may be cleaned by the OS.

## Local validation

Use Python 3.11 or newer and the installed CLI development environment. From
`apps/cli` (see its README for first-time installation):

```sh
source .venv/bin/activate
pytest
ruff check .
ghost --help
ghost version --help
ghost version
ghost project --help
ghost demo --help
```

Expected version output is `GHOST 0.1.0`. The literal in
`apps/cli/src/ghost_cli/__init__.py` supplies both CLI output and setuptools build
metadata. Reinstall after an intentional version change. Record the actual Python
version, test/lint results, and reviewed revision; do not infer validation from
the package number. CI covers Python 3.11 and 3.14 on Ubuntu; record other platforms
only when tested.

From the repository root:

```sh
git diff --check
git status --short
git diff --stat
git status --short -- apps/desktop
```

Inspect newly added files too: ordinary `git diff --stat` does not include untracked
files. Desktop status must be empty for this CLI milestone.

## Isolated smoke flow

With the CLI environment active, run:

```sh
ghost version
ghost demo nexora
```

The demo selects a new temporary directory and an isolated home even if an existing
`GHOST_HOME` is set. It does not need an actual project repository. Follow the
printed `DEMO-REPORT.md` and [demo walkthrough](demo-workflow.md).

In a separate terminal, activate the CLI environment and set `GHOST_HOME` to the
exact **demo home printed by that run**. POSIX example, replacing the placeholder:

```sh
export GHOST_HOME="/printed/demo-directory/ghost-home"
ghost project list
ghost project show nexora-demo
ghost session status nexora-demo
ghost output list --project nexora-demo
ghost doctor
ghost session close nexora-demo
ghost doctor --project nexora-demo
```

For PowerShell, use `$env:GHOST_HOME` to select that same demo home. These are
manual shell commands, not commands executed by GHOST.

Verify the session starts with one note, the sanitized output links to it, and
one context pack, four handoffs, one next-step draft, and six update drafts exist.
Doctor should report no errors or warnings for the untouched fresh demo and after
closing its session. Confirm all sample claims remain labeled and that closing
retains history. Rerunning the demo must create a separate directory. Preserve the
report for review; the CLI does not automatically remove demo artifacts.

## Release checklist and manual review before tagging

Leave these unchecked until evidence for the intended release revision is reviewed:

- [ ] Confirm the intended branch/revision and review all changed and newly added files.
- [ ] Confirm version output and built package metadata agree with the intended version.
- [ ] Record passing local tests, Ruff, help/version checks, and the isolated smoke flow.
- [ ] Review actual CI results for the intended PR: Python 3.11, Python 3.14,
      Release scripts, and Repository hygiene. Missing/pending checks are not a pass.
- [ ] Review storage preservation, redaction limits, partial-failure guidance, and
      remaining known issues. Back up any real workflow records before future upgrades.
- [ ] Confirm no `.env`, credentials, private transcripts, or incidental demo storage
      are staged. Keep sample work separate from real portfolio accomplishments.
- [ ] Confirm desktop and release scripts are unchanged, and no execution/provider
      capability or unsupported production/security claim was added.
- [ ] Review the release description against the included scope and documented limits.
- [ ] Obtain owner approval for the exact release/tag intent and review the
      [release workflow](release-workflow.md), including approved-file commits,
      successful PR checks, clean-state requirements, and typed confirmation.

This checklist does not authorize a commit, push, merge, tag, or publication.
The existing release workflow remains the authority for those separately approved
operations; local validation and doctor do not bypass its checks.
