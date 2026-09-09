# Release workflow — Milestone 2.5

These are manually invoked repository maintenance scripts, separate from the GHOST
CLI. They do not give GHOST terminal execution or GitHub automation capabilities.
The active application remains `apps/cli`; the desktop is future UI.

## Prerequisites

- Bash 3.2 or newer and Git. The scripts locate the repository root from your
  current directory; run them inside the intended working tree.
- A local `main` branch and an `origin` remote. Start from clean `main`, with no
  unpublished commits. Pulls use `--ff-only` and abort on divergent history.
- For PR operations, a recent GitHub CLI (`gh`) with `pr checks --json` and
  `pr merge --match-head-commit` support, authenticated with push/PR/merge access.
- `origin` must use a standard HTTPS, `git@host:owner/repo.git`, or
  `ssh://git@host/owner/repo.git` GitHub URL. GitHub Enterprise hostnames work with
  corresponding authentication. Custom SSH aliases, URL credentials, and alternate
  ports are not supported by the origin parser. The scripts bind `gh` to origin,
  rather than trusting a different default GitHub repository.

From the repository root, enable direct invocation if needed:

```sh
chmod +x scripts/release/start-milestone.sh \
  scripts/release/commit-milestone.sh \
  scripts/release/open-pr.sh \
  scripts/release/merge-and-tag.sh
```

`common.sh` supplies shared safety and output helpers and is sourced automatically.
Each script prints `[INFO]`, `[PASS]`, and `[ERROR]` messages. A failure exits nonzero.

## 1. Start a focused milestone

Inspect `git status --short` and `git branch --show-current` first. From clean main:

```sh
scripts/release/start-milestone.sh feature/example-milestone
```

The script refuses invalid Git branch names, main/master, dirty working trees,
existing local/remote branches, and failures to verify remote absence. It switches
to main, pulls `origin main` without creating a merge commit, verifies main matches
origin, then creates the new branch and prints its status. It never stashes work,
resets main, or deletes an existing branch. A pull failure may leave you on main;
inspect status before retrying.

## 2. Implement and validate locally

Use Codex/Astra or manual edits for the focused milestone. Keep CLI implementation
in `apps/cli`, and leave `apps/desktop` unchanged unless explicitly requested.

The CLI uses Python/pip and `pyproject.toml`, not a root JavaScript package manager.
The future desktop has its own pnpm lockfile. The helpers do not choose validation
commands automatically. For a CLI milestone, from the repository root:

```sh
cd apps/cli
source .venv/bin/activate
pytest
ruff check .
ghost --help
ghost session --help
cd ../..
```

For release script changes:

```sh
bash -n scripts/release/common.sh
bash -n scripts/release/start-milestone.sh
bash -n scripts/release/commit-milestone.sh
bash -n scripts/release/open-pr.sh
bash -n scripts/release/merge-and-tag.sh
python3 -m unittest discover -s scripts/release/tests -v
git diff --check
```

The helper tests use fake Git and GitHub commands in temporary directories. They
do not create real commits, tags, PRs, merges, or pushes and need no network access.

## 3. Commit only approved files

Review the changes, then pass an explicit list of files relative to the repository
root. Quote each filename containing spaces. Directories, absolute paths, dot
components, and pathspec expansion are disallowed. Renames require both the old
and new paths; tracked deletions are supported.

```sh
scripts/release/commit-milestone.sh \
  --message "Add example feature" \
  --files \
  apps/cli/src/ghost_cli/example.py \
  apps/cli/tests/test_example.py
```

Use a non-blank, one-line message. `--files` must be last: every remaining argument
is a literal approved path. The script refuses main/master and detached HEAD. It
also refuses any pre-staged changes, preserving your existing index for review.

It prints working-tree status and a diff summary, checks whitespace, stages only
the individual approved paths, and checks that every staged change is approved.
After reviewing the staged summary (and full `git diff --cached` if needed), type:

```text
commit "Add example feature" with approved files
```

Only that exact text authorizes the commit. The script rechecks the branch, HEAD,
staged paths, and staged content after confirmation. If cancelled or staging fails,
it attempts to unstage its approved paths while preserving working files and any
unrelated staged paths. Inspect the index if cleanup reports an error. Normal Git
hooks still run; use trusted hooks and do not run competing Git/index operations
during review. A successful commit prints status and the last five commits.

## 4. Open a PR and watch checks

```sh
scripts/release/open-pr.sh \
  --title "Add example feature" \
  --body "Summary: focused feature. Validation: pytest and Ruff passed. Scope: CLI only."
```

From a clean feature branch, this checks GitHub authentication, pushes the branch
with upstream tracking, and creates a PR targeting main. Title and body are passed
as quoted arguments, never evaluated as shell code. Multiline bodies are supported.
It watches reported checks when the installed `gh` supports watching, prints the
PR URL, and leaves review and merging to a human.

If no checks are reported, it explicitly does not claim a pass. CI may not be
configured or may not have started yet. If PR creation or watching fails after the
push, the branch/PR may already exist; inspect it before retrying. The script does
not merge or treat an existing PR as a newly created one.

## 5. Review, merge, and tag

After reviewing the code, validation, scope, and CI results:

```sh
scripts/release/merge-and-tag.sh \
  --pr 12 \
  --tag v0.1.0-example-milestone \
  --message "Project v0.1.0 Example milestone"
```

This requires a clean working tree, an open PR targeting main, and a valid tag
that exists neither locally nor on origin. It refuses PR heads named main/master,
so branch deletion cannot remove them. All reported checks must be completed
successfully; pending, failed, cancelled, skipped, neutral, and unknown outcomes
are rejected. It never uses an admin bypass or silently enables a checks override.

PR checks now run through [GitHub Actions CI](../.github/workflows/ci.yml) on pull
requests targeting `main` and pushes to `main`. Expect all four check results:
`CLI (Python 3.11)`, `CLI (Python 3.14)`, `Release scripts`, and `Repository hygiene`.
These cover CLI pytest/Ruff, Bash syntax and mocked release-safety tests, whitespace
across the PR/push diff, and tracked Python bytecode/cache files. CI has read-only
repository permissions and does not commit, push, merge, or tag.

**Wait for all four checks to complete successfully and review the PR before typing
the merge/tag confirmation.** No reported checks, or any reported pending, skipped,
or failing check, still block the helper; local tests do not bypass CI. The helper
does not enforce a fixed list of check names. If checks have not appeared yet,
wait and inspect the PR's Actions run. A PR with no reported checks cannot use
automated merge/tag. Repository owners must separately configure required status
checks in branch protection/rulesets to enforce them for merges outside the helper;
adding this workflow does not change those settings.

Type the exact confirmation:

```text
merge PR #12 and tag v0.1.0-example-milestone
```

The script rechecks cleanliness, tag absence, PR identity, and checks after the
prompt. It merges with a merge commit, pins the reviewed PR head, and requests
deletion of the milestone branch. It confirms the PR actually merged, rather than
merely entering a merge queue, then switches to main and pulls without merging.

After rechecking tag absence, it creates an annotated tag on the PR's exact merge
commit, verifies that commit is on main, and pushes the fully qualified tag ref.
This avoids accidentally tagging a later unrelated commit or pushing a same-named
branch. It fetches/prunes and prints final status, branches, and recent history.

## Safety and partial completion

- Never force-push, overwrite tags, delete main, or stage an entire directory.
- Network/authentication errors during remote-ref checks are failures, not proof
  that a branch/tag is absent. Existing refs always require a different name.
- Ignored files do not make Git's working tree dirty. Review local data and secrets
  before approving any file; these scripts are not secret scanners.
- Human review, typed confirmation, and repository branch protection remain
  necessary. GitHub state can change concurrently; configure required checks and
  review rules on main to enforce policy on the server as well.
- Merge, branch deletion, and tag push are separate operations. They cannot be
  rolled back as one transaction. If a merge is queued, a pull fails, or a tag push
  fails, inspect the PR and local/remote refs. Do not blindly rerun the script: the
  PR may already be merged or an annotated tag may already exist locally.
- These helpers never automatically commit or push merely because documentation
  or tests were updated. Run them only when you intend the stated operation.
