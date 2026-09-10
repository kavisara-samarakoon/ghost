# Real Project Integration Demo — Milestone 7

This walkthrough demonstrates GHOST's local coordination features using a fictional
NEXORA-style price-alert display proposal. It does not access the owner's NEXORA,
SentinelLite, ARM-SecNet, or portfolio repositories. Sample claims are not evidence
of real implementation, tests, a release, or portfolio accomplishments.

## Run the sample

From the repository root, with the CLI already installed:

```sh
apps/cli/.venv/bin/ghost demo nexora
```

Or use `ghost demo nexora` with that virtual environment activated. No `ghost init`,
`GHOST_HOME` export, existing repository, provider account, or network is needed.
The command takes no path argument: it always allocates a new private temporary
directory and prints its location. Any existing `GHOST_HOME` remains unchanged.
Do not point OS temporary-directory settings into real project repositories;
the demo refuses detected Git/project workspace and `.env` locations.

## What the command creates

| Step | Stored result |
| --- | --- |
| Initialize isolated global storage | `ghost-home/config.yaml`, `projects.yaml`, `audit.jsonl` |
| Register `nexora-demo` | `nexora-demo/.ghost/project.yaml` and initial workspace |
| Seed the fictional proposal | Sample `status.md`, `decisions.md`, `milestones.yaml` |
| Start a session and add a note | Active pointer, session YAML, timestamped note |
| Import fake Codex/Astra output | Sanitized Markdown and session-linked output index |
| Generate context | One context pack from allowlisted sample records |
| Generate handoffs | Codex/Astra, ChatGPT, Gemini/NotebookLM, Antigravity drafts |
| Propose next steps | One deterministic next-step draft with sanitized output excerpts |
| Prepare an update pack | Six review-only audience drafts |
| Inspect storage | Actual doctor findings in printed and saved `DEMO-REPORT.md` |

All artifacts live inside one run directory:

```text
<OS-temp>/ghost-demo-nexora-<unique-suffix>/
  DEMO-REPORT.md
  ghost-home/
  nexora-demo/
    .ghost/
      project.yaml, status.md, decisions.md, milestones.yaml
      active-session.yaml, sessions/, outputs/, audit.jsonl
      drafts/
        context-packs/
        handoffs/{codex,chatgpt,gemini,antigravity}/
        next-steps/
        update-packs/<unique-pack>/
          README-update.md
          release-notes.md
          linkedin-post.md
          portfolio-update.md
          project-summary.md
          chatgpt-review-request.md
```

The fixed sample transcript contains fake credential assignments supplied only
in memory. The normal output logger replaces them with `[REDACTED]` before storage;
next-step and internal update drafts consume that sanitized output. Context packs
and handoffs use workspace/session records, not output transcripts. No `.env`
file, real source tree, Git history, or external project record is read.

## Review manually

1. Open the printed `DEMO-REPORT.md` and follow its relative artifact paths.
2. Check that the project, status, milestones, notes, and drafts are DEMO / SAMPLE.
3. Inspect the sanitized sample output and its link to the active session.
4. Review all four handoffs, next steps, and six update drafts for scope and privacy.
   Keep the sample labels. Nothing in these files approves real work or publication.
5. Review doctor findings. Expected fresh-run outcome: no warnings or errors.
   These are real storage checks, but they do not establish that NEXORA was changed,
   tests passed, or a release is ready. GHOST runs no terminal commands or AI calls.

To inspect or close the demo session, explicitly select the **printed demo home**
in a separate terminal. For POSIX shells, replace the example path with that exact
location (PowerShell users can set `$env:GHOST_HOME` to the same printed path):

```sh
export GHOST_HOME="/printed/demo-directory/ghost-home"
ghost project show nexora-demo
ghost session status nexora-demo
ghost output list --project nexora-demo
ghost doctor
# Optional, after reviewing the sample:
ghost session close nexora-demo
```

These are commands for you to run manually; the demo does not execute them.
Using a separate terminal keeps your ordinary shell's storage selection intact.

## Repeatability, failure, and retention

Rerun `ghost demo nexora` for an independent sample with the same fixed scenario
and sequence. The standard UTC timestamps, IDs, directory suffixes, and paths vary;
byte-identical exports are not promised. Existing runs are never overwritten.

A workflow/storage failure exits nonzero, reports the retained directory, and
leaves an incomplete report plus any finished artifacts. Inspect that directory
before manually removing it; rerunning starts a new demo and does not repair or
resume the partial one. A completed workflow with doctor errors also exits nonzero
and retains the full findings. No failed run grants release or publication approval.

The CLI never automatically deletes a demo. The operating system may clean temporary
storage; copy reviewed sample files to an appropriate demo-only location if you
need to keep them. Release scripts, CI, approved-file commits, and typed release
confirmations remain separate as described in [release-workflow.md](release-workflow.md).
