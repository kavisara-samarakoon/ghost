#!/bin/bash
set -euo pipefail
script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
source "$script_dir/common.sh"

[[ $# -eq 2 && "$1" == --pr ]] || error "Usage: $0 --pr <positive-pr-number>"
pr=$2
[[ "$pr" =~ ^[1-9][0-9]*$ ]] || error "--pr must be a positive PR number."
require_repo
require_clean
require_gh

read_open_milestone() {
    read_milestone_pr "$pr"
    [[ "$pr_number" == "$pr" ]] || error "Returned PR number does not match #$pr."
}
require_reviewed_pr() {
    read_open_milestone
    [[ "$head_branch" == "$reviewed_branch" && "$head_oid" == "$reviewed_head" ]] ||
        error "PR branch or head changed. Review again."
}

read_open_milestone
reviewed_branch=$head_branch
reviewed_head=$head_oid
info "PR #$pr: $pr_title — $head_branch -> $base ($reviewed_head)"
info "$pr_url"
require_passing_checks
require_reviewed_pr
confirm_exactly "merge PR #$pr"

require_clean
require_reviewed_pr
require_passing_checks
# Bind the second CI read to the same PR head and branch as well.
require_reviewed_pr
require_clean
info "Squash-merging PR #$pr and deleting its milestone branch."
gh pr merge "$pr" --squash --delete-branch --match-head-commit "$reviewed_head"

merged=$(gh pr view "$pr" --json state,mergeCommit --jq '[.state, (.mergeCommit.oid // "")] | @tsv') ||
    error "Could not verify merge completion. Inspect PR #$pr manually."
IFS=$'\t' read -r state merge_oid <<< "$merged"
[[ "$state" == MERGED && "$merge_oid" =~ ^([a-f0-9]{40}|[a-f0-9]{64})$ ]] ||
    error "PR is not actually merged or has no valid merge commit (possibly queued). Inspect it manually."
git switch main
git pull --ff-only origin main
require_clean
verify_main() {
    local local_main remote_main
    current_branch
    [[ "$branch" == main ]] || error "PR is merged, but current branch is not main."
    local_main=$(git rev-parse --verify HEAD) || error "Could not resolve local main after merge."
    remote_main=$(git rev-parse --verify refs/remotes/origin/main) || error "Could not resolve origin/main after merge."
    [[ "$local_main" =~ ^([a-f0-9]{40}|[a-f0-9]{64})$ && "$local_main" == "$remote_main" ]] ||
        error "PR is merged, but local main differs from origin/main. Inspect Git state manually."
    git merge-base --is-ancestor "$merge_oid" main || error "PR merge commit is not on main."
}
verify_main
git fetch --prune
require_clean
verify_main
pass "PR #$pr merged. Merge commit: $merge_oid. Current branch: $branch. Working tree is clean."
git status --short
git log --oneline --decorate -8
