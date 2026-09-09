#!/bin/bash
set -euo pipefail
script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
source "$script_dir/common.sh"

pr=''
tag=''
message=''
while [[ $# -gt 0 ]]; do
    case "$1" in
        --pr) [[ $# -ge 2 && -z "$pr" ]] || error "Supply --pr once with a value."; pr=$2; shift 2 ;;
        --tag) [[ $# -ge 2 && -z "$tag" ]] || error "Supply --tag once with a value."; tag=$2; shift 2 ;;
        --message) [[ $# -ge 2 && -z "$message" ]] || error "Supply --message once with a value."; message=$2; shift 2 ;;
        *) error "Usage: $0 --pr 12 --tag v0.1.0-example --message \"Release description\"" ;;
    esac
done
[[ "$pr" =~ ^[1-9][0-9]*$ ]] || error "--pr must be a positive PR number."
require_text Message "$message"
validate_ref tags "$tag"
require_repo
require_clean
require_gh
require_absent_tag

read_open_pr() {
    local details number state base title
    details=$(gh pr view "$pr" --json number,state,title,headRefName,baseRefName,url,headRefOid \
        --jq '[.number, .state, .headRefName, .baseRefName, .url, .headRefOid, .title] | @tsv') ||
        error "Cannot inspect PR #$pr."
    IFS=$'\t' read -r number state head_branch base pr_url head_oid title <<< "$details"
    [[ "$number" == "$pr" && "$state" == OPEN && "$base" == main ]] ||
        error "PR must be OPEN and target main."
    case "$head_branch" in ''|main|master) error "Refusing to delete protected/empty PR head branch: $head_branch" ;; esac
    [[ "$head_oid" =~ ^[a-f0-9]{40,64}$ ]] || error "PR head commit is missing or invalid."
    info "PR #$pr: $title — $head_branch -> $base ($head_oid)"
    info "$pr_url"
}

require_passing_checks() {
    local checks bucket state seen=false
    gh pr checks "$pr" || error "CI must exist and every check must pass. Checks are absent, pending, failing, or unavailable."
    checks=$(gh pr checks "$pr" --json bucket,state --jq '.[] | [.bucket, .state] | @tsv') ||
        error "Could not verify successful checks. CI is required; no bypass is available."
    [[ -n "$checks" ]] || error "No checks configured. CI must exist before automated merge/tag."
    while IFS=$'\t' read -r bucket state; do
        seen=true
        [[ "$bucket" == pass ]] || error "A check is $bucket ($state); every check must succeed."
        case "$state" in SUCCESS|success) ;; *) error "Check is not completed successfully: $state" ;; esac
    done <<< "$checks"
    [[ "$seen" == true ]] || error "No checks configured. CI is required."
    pass "All reported checks completed successfully."
}

read_open_pr
reviewed_head=$head_oid
require_passing_checks
read_open_pr
[[ "$head_oid" == "$reviewed_head" ]] || error "PR head changed while checking CI. Review again."
confirm_exactly "merge PR #$pr and tag $tag"

require_clean
require_absent_tag
read_open_pr
[[ "$head_oid" == "$reviewed_head" ]] || error "PR head changed during confirmation. Review again."
require_passing_checks
info "Merging PR #$pr and deleting its milestone branch."
gh pr merge "$pr" --merge --delete-branch --match-head-commit "$reviewed_head"

# A merge queue may accept the command without actually merging yet. Never tag it.
merged=$(gh pr view "$pr" --json state,mergeCommit --jq '[.state, (.mergeCommit.oid // "")] | @tsv')
IFS=$'\t' read -r state merge_oid <<< "$merged"
[[ "$state" == MERGED && "$merge_oid" =~ ^[a-f0-9]{40,64}$ ]] ||
    error "PR is not yet merged (possibly queued). No tag created. Inspect the PR manually."
git switch main
git pull --ff-only origin main
require_clean
[[ $(git rev-parse HEAD) == "$(git rev-parse refs/remotes/origin/main)" ]] ||
    error "Local main differs from origin/main. PR is merged but no tag was created."
git merge-base --is-ancestor "$merge_oid" main || error "PR merge commit is not on main; refusing to tag."
require_absent_tag
info "Creating annotated tag $tag on the PR's exact merge commit $merge_oid."
git tag -a "$tag" -m "$message" "$merge_oid"
# Fully qualified ref avoids ambiguity with a branch having the same name.
git push origin "refs/tags/$tag"
git fetch --prune
require_clean
pass "PR merged and annotated tag $tag pushed."
git status
git branch --all
git log --oneline --decorate -8
