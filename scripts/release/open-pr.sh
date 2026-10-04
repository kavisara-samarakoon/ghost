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

wait_for_checks() {
    # Fixed startup budget: one immediate poll plus 24 waits of five seconds.
    # API failures are errors, not evidence that CI has not started yet.
    local attempt count max_attempts=25 poll_interval=5
    for ((attempt=1; attempt<=max_attempts; attempt++)); do
        count=$(gh pr view "$pr_url" --json statusCheckRollup --jq '.statusCheckRollup | length') ||
            error "Could not inspect checks. Review $pr_url manually."
        [[ "$count" =~ ^(0|[1-9][0-9]*)$ ]] || error "Unexpected check information. Review $pr_url manually."
        if [[ "$count" != 0 ]]; then
            info "$count CI checks reported for $pr_url."
            return
        fi
        [[ "$attempt" -lt "$max_attempts" ]] ||
            error "CI checks never appeared after $max_attempts polls (120 seconds of waiting). Review $pr_url manually."
        info "Waiting for CI checks to appear ($attempt/$max_attempts). Retrying in $poll_interval seconds."
        sleep "$poll_interval" || error "Waiting for CI failed. Review $pr_url manually."
    done
}

wait_for_checks
checks_help=$(gh pr checks --help) || error "Cannot inspect gh check support. Review $pr_url manually."
[[ "$checks_help" == *--watch* ]] || error "This gh version cannot watch checks; upgrade gh. Review $pr_url manually."
info "Watching PR checks: $pr_url"
gh pr checks "$pr_url" --watch || error "Checks did not pass or watching failed. Review $pr_url."
# Watching can return zero for skipped checks. Require actual successful states,
# using the same strict gate as both merge scripts, without changing their logic.
(pr=$pr_url; require_passing_checks) || error "Checks did not all complete successfully. Review $pr_url."
pass "Check watching completed with successful CI. Review the results and PR before merging."
info "PR: $pr_url — next: human review, then merge-and-tag only with successful CI."
