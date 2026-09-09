#!/bin/bash
set -euo pipefail
script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
source "$script_dir/common.sh"

title=''
body=''
while [[ $# -gt 0 ]]; do
    case "$1" in
        --title) [[ $# -ge 2 && -z "$title" ]] || error "Supply --title once with a value."; title=$2; shift 2 ;;
        --body) [[ $# -ge 2 && -z "$body" ]] || error "Supply --body once with a value."; body=$2; shift 2 ;;
        *) error "Usage: $0 --title \"Title\" --body \"Summary, validation, and scope.\"" ;;
    esac
done
require_text Title "$title"
require_text Body "$body"
require_repo
require_clean
require_feature_branch
validate_ref heads "$branch"
require_gh

info "Pushing $branch to origin without force."
git push -u origin "$branch"
info "Creating a PR targeting main."
pr_url=$(gh pr create --base main --head "$branch" --title "$title" --body "$body") ||
    error "PR creation failed. The branch was pushed; inspect existing PRs before retrying."
[[ "$pr_url" == https://* ]] || error "gh did not return a PR URL. Inspect the repository's PRs."
pass "PR created: $pr_url"
info "Human review and a separate merge decision are still required. This script never merges."

count=$(gh pr view "$pr_url" --json statusCheckRollup --jq '.statusCheckRollup | length') ||
    error "Could not inspect checks. Review $pr_url manually."
[[ "$count" =~ ^[0-9]+$ ]] || error "Unexpected check information. Review $pr_url manually."
if [[ "$count" -eq 0 ]]; then
    info "No checks are reported. CI may be unconfigured or not started yet; checks have NOT passed."
    info "Configure CI or wait for it to start, then run gh pr checks '$pr_url' --watch."
else
    checks_help=$(gh pr checks --help) || error "Cannot inspect gh check support. Review $pr_url manually."
    if [[ "$checks_help" == *--watch* ]]; then
        info "Watching PR checks."
        gh pr checks "$pr_url" --watch || error "Checks did not pass or watching failed. Review $pr_url."
        pass "Check watching completed. Review the results and PR before merging."
    else
        info "This gh version cannot watch checks; upgrade gh or inspect $pr_url manually. No pass is claimed."
    fi
fi
info "PR: $pr_url — next: human review, then merge-and-tag only with successful CI."
