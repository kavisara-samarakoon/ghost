#!/bin/bash
set -euo pipefail
script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
source "$script_dir/common.sh"

[[ $# -eq 1 ]] || error "Usage: scripts/release/start-milestone.sh feature/example-milestone"
new_branch=$1
validate_ref heads "$new_branch"
case "$new_branch" in main|master) error "Choose a milestone branch, not $new_branch." ;; esac
require_repo
require_clean
require_origin
require_absent_ref "refs/heads/$new_branch"
git show-ref --verify --quiet refs/heads/main || error "A local main branch is required."

info "Updating main with a fast-forward-only pull."
git switch main
git pull --ff-only origin main
[[ $(git rev-parse HEAD) == "$(git rev-parse refs/remotes/origin/main)" ]] ||
    error "Local main has unpublished commits. Reconcile it with origin/main before starting."
require_clean
info "Creating $new_branch."
git switch -c "$new_branch"
pass "Milestone branch is ready."
git branch --show-current
git status --short
