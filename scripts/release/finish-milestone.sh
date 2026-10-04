#!/bin/bash
set -euo pipefail
script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
source "$script_dir/common.sh"

message=''
title=''
body=''
body_file=''
message_seen=false
title_seen=false
body_seen=false
body_file_seen=false
approved=()
while [[ $# -gt 0 ]]; do
    case "$1" in
        --message|--title|--body|--body-file)
            [[ $# -ge 2 && "$2" != --* ]] || error "$1 requires a value."
            case "$1" in
                --message) [[ "$message_seen" == false ]] || error "Supply --message once."; message_seen=true; message=$2 ;;
                --title) [[ "$title_seen" == false ]] || error "Supply --title once."; title_seen=true; title=$2 ;;
                --body) [[ "$body_seen" == false ]] || error "Supply --body once."; body_seen=true; body=$2 ;;
                --body-file) [[ "$body_file_seen" == false ]] || error "Supply --body-file once."; body_file_seen=true; body_file=$2 ;;
            esac
            shift 2 ;;
        --files)
            shift
            [[ $# -gt 0 ]] || error "--files requires explicit paths."
            for file in "$@"; do
                [[ "$file" != --* ]] || error "Place options before --files; use explicit paths only."
            done
            approved=("$@")
            break ;;
        *) error "Usage: $0 --message <message> --title <title> (--body <body> | --body-file <path>) --files <paths...>" ;;
    esac
done
[[ "$message_seen" == true && "$title_seen" == true ]] || error "--message and --title are required exactly once."
[[ "$body_seen" != "$body_file_seen" ]] || error "Supply exactly one of --body or --body-file."
require_text Message "$message"
[[ "$message" != *$'\n'* && "$message" != *$'\r'* ]] || error "Use a one-line commit message for typed confirmation."
require_text Title "$title"
[[ ${#approved[@]} -gt 0 ]] || error "--files requires one or more explicit paths."
if [[ "$body_file_seen" == true ]]; then
    [[ -f "$body_file" && -r "$body_file" ]] || error "PR body file must be readable."
    # Preserve trailing newlines while adapting to open-pr.sh's --body interface.
    body=$(cat -- "$body_file" && printf '.') || error "Could not read PR body file."
    body=${body%.}
fi
require_text Body "$body"
require_repo
require_feature_branch
validate_ref heads "$branch"
starting_branch=$branch
require_gh

# The commit delegate alone stages explicit files and owns the exact commit gate.
"$script_dir/commit-milestone.sh" --message "$message" --files "${approved[@]}"
require_clean
require_feature_branch
[[ "$branch" == "$starting_branch" ]] || error "Feature branch changed after commit."
committed_head=$(git rev-parse HEAD)
"$script_dir/open-pr.sh" --title "$title" --body "$body"
require_clean
require_feature_branch
[[ "$branch" == "$starting_branch" && $(git rev-parse HEAD) == "$committed_head" ]] ||
    error "Branch or committed HEAD changed during PR creation."
read_milestone_pr "$starting_branch"
[[ "$head_branch" == "$starting_branch" && "$head_oid" == "$committed_head" ]] ||
    error "Resolved PR does not match the starting feature branch and committed HEAD."
info "Resolved PR #$pr_number: $pr_url"
"$script_dir/merge-milestone.sh" --pr "$pr_number"
pass "Milestone finished on clean main, synchronized with origin/main."
