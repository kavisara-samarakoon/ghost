#!/bin/bash
set -euo pipefail
script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
source "$script_dir/common.sh"

message=''
approved=()
while [[ $# -gt 0 ]]; do
    case "$1" in
        --message)
            [[ $# -ge 2 && -z "$message" ]] || error "Supply --message once with a value."
            message=$2; shift 2 ;;
        --files)
            shift
            [[ $# -gt 0 ]] || error "--files requires explicit file paths."
            approved=("$@")
            break ;;
        *) error "Usage: $0 --message \"Message\" --files path/to/file ..." ;;
    esac
done
require_text Message "$message"
[[ "$message" != *$'\n'* && "$message" != *$'\r'* ]] || error "Use a one-line commit message for typed confirmation."
[[ ${#approved[@]} -gt 0 ]] || error "At least one approved file is required after --files."
require_repo
require_feature_branch
reviewed_branch=$branch
reviewed_head=$(git rev-parse HEAD)
# Literal pathspecs prevent filenames such as '*' or ':(glob)...' from expanding.
export GIT_LITERAL_PATHSPECS=1
git diff --cached --quiet || error "The index already has staged changes. Review/unstage them first; nothing was changed."

for file in "${approved[@]}"; do
    case "$file" in
        ''|/*|.|..|./*|../*|*/./*|*/../*|*/.|*/..|*//*|*/)
            error "Use an explicit repository-root-relative file path without dot components: $file" ;;
    esac
    [[ ! -d "$file" || -L "$file" ]] || error "Directories are not approved file lists: $file"
    if [[ ! -f "$file" && ! -L "$file" ]]; then
        [[ ! -e "$file" ]] || error "Not a regular file or symlink: $file"
        git ls-files --error-unmatch -- "$file" >/dev/null 2>&1 ||
            error "Approved file is missing and is not a tracked deletion: $file"
    fi
done

info "Review branch: $branch (approved paths are relative to $repo_root)."
git status --short
git diff --stat
git diff --check

review_dir=$(mktemp -d "${TMPDIR:-/tmp}/ghost-commit-review.XXXXXX")
staging_started=false
committed=false
cleanup() {
    local exit_code=$?
    trap - EXIT
    if [[ "$staging_started" == true && "$committed" == false ]]; then
        info "Removing this script's approved paths from the index; working files are preserved."
        for file in "${approved[@]}"; do
            git restore --staged -- "$file" || printf '[ERROR] Could not unstage %s; inspect the index.\n' "$file" >&2
        done
    fi
    rm -f -- "$review_dir/staged"
    rmdir -- "$review_dir"
    exit "$exit_code"
}
trap cleanup EXIT

check_approved_index() {
    local staged_file allowed approved_file
    git diff --cached --no-renames --name-only -z > "$review_dir/staged"
    [[ -s "$review_dir/staged" ]] || error "No staged changes to commit."
    while IFS= read -r -d '' staged_file; do
        allowed=false
        for approved_file in "${approved[@]}"; do
            if [[ "$staged_file" == "$approved_file" ]]; then allowed=true; break; fi
        done
        [[ "$allowed" == true ]] || error "Unapproved staged path: $staged_file. Aborting."
    done < "$review_dir/staged"
}

staging_started=true
for file in "${approved[@]}"; do
    git add -- "$file"
done
check_approved_index
info "Staged files and diff summary (review full content with git diff --cached if needed)."
git diff --cached --no-renames --name-only
git diff --cached --stat
git diff --cached --check
reviewed_tree=$(git write-tree)
confirm_exactly "commit \"$message\" with approved files"

# Reject changes made to the index, HEAD, or branch during the review prompt.
require_feature_branch
[[ "$branch" == "$reviewed_branch" && $(git rev-parse HEAD) == "$reviewed_head" ]] ||
    error "Branch or HEAD changed during review."
check_approved_index
[[ $(git write-tree) == "$reviewed_tree" ]] || error "Staged content changed during review."
git diff --cached --check
git commit -m "$message"
committed=true
pass "Committed the approved files."
git status --short
git log --oneline --decorate -5
