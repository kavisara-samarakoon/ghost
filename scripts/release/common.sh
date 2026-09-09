#!/bin/bash
# Shared helpers. These scripts require Bash 3.2+, Git, and (for PRs) a recent gh.
set -euo pipefail

info() { printf '\n[INFO] %s\n' "$*"; }
pass() { printf '\n[PASS] %s\n' "$*"; }
error() { printf '\n[ERROR] %s\n' "$*" >&2; exit 1; }
trap 'error "Command failed at line $LINENO. Review the output and repository state before retrying."' ERR

require_repo() {
    command -v git >/dev/null 2>&1 || error "Git is required."
    repo_root=$(git rev-parse --show-toplevel 2>/dev/null) || error "Run inside a Git working tree."
    cd -- "$repo_root"
    git rev-parse --verify HEAD >/dev/null 2>&1 || error "An initial repository commit is required."
}

require_clean() {
    local status
    status=$(git status --porcelain --untracked-files=all)
    if [[ -n "$status" ]]; then
        printf '%s\n' "$status"
        error "Working tree is not clean. Review and finish existing changes first."
    fi
    pass "Working tree is clean."
}

current_branch() {
    branch=$(git symbolic-ref --quiet --short HEAD) || error "Detached HEAD is not supported."
}

require_feature_branch() {
    current_branch
    case "$branch" in main|master) error "Refusing to operate on $branch." ;; esac
}

require_text() {
    [[ "$2" == *[![:space:]]* ]] || error "$1 must not be empty."
}

validate_ref() {
    local kind=$1 value=$2
    case "$value" in
        ''|-*|*[[:space:]]*|*..*|*'@{'*|*\\*|*//*|*/)
            error "Unsafe $kind name: $value" ;;
    esac
    git check-ref-format "refs/$kind/$value" >/dev/null 2>&1 || error "Invalid Git $kind name: $value"
    if [[ "$kind" == heads ]]; then
        git check-ref-format --branch "$value" >/dev/null 2>&1 || error "Invalid branch name: $value"
    fi
}

require_origin() {
    git remote get-url origin >/dev/null 2>&1 || error "An origin remote is required."
}

require_absent_ref() {
    local ref=$1 code
    if git show-ref --verify --quiet "$ref"; then
        error "$ref already exists locally."
    else
        code=$?
        [[ "$code" -eq 1 ]] || error "Could not check local ref $ref."
    fi
    # Exit 2 means no matching ref; transport/authentication failures are not absence.
    if git ls-remote --exit-code origin "$ref"; then
        error "$ref already exists on origin."
    else
        code=$?
        [[ "$code" -eq 2 ]] || error "Could not verify $ref on origin (exit $code)."
    fi
}

require_absent_tag() {
    local code
    if git show-ref --verify --quiet "refs/tags/$tag"; then
        error "Tag $tag already exists locally."
    else
        code=$?
        [[ "$code" -eq 1 ]] || error "Could not check local tag $tag."
    fi
    if git ls-remote --exit-code --tags origin "refs/tags/$tag"; then
        error "Tag $tag already exists on origin."
    else
        code=$?
        [[ "$code" -eq 2 ]] || error "Could not verify remote tag absence (exit $code)."
    fi
}

require_gh() {
    command -v gh >/dev/null 2>&1 || error "Install GitHub CLI (gh) first."
    require_origin
    local remote repository
    remote=$(git remote get-url origin)
    case "$remote" in
        https://*) repository=${remote#https://} ;;
        git@*:*) repository=${remote#git@}; repository=${repository/:/\/} ;;
        ssh://git@*/*) repository=${remote#ssh://git@} ;;
        *) error "Use a standard HTTPS or git@host origin URL for GitHub operations." ;;
    esac
    repository=${repository%.git}
    [[ "$repository" =~ ^[a-zA-Z0-9.-]+/[a-zA-Z0-9_.-]+/[a-zA-Z0-9_.-]+$ ]] ||
        error "Origin must identify a GitHub host/owner/repository without credentials or URL options."
    # Bind gh to origin, even if the user's default gh repository points elsewhere.
    export GH_REPO="$repository"
    gh auth status --hostname "${repository%%/*}" || error "Authenticate gh for origin's host first."
    pass "GitHub operations target $GH_REPO."
}

confirm_exactly() {
    local expected=$1 response
    printf '\nType exactly: %s\n> ' "$expected"
    IFS= read -r response || error "Confirmation was not received."
    [[ "$response" == "$expected" ]] || error "Confirmation did not match. Operation cancelled."
}
