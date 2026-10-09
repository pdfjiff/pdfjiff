#!/usr/bin/env bash
# One-time (and re-runnable) GitHub settings for a PDFJiff repository.
#
#   brew install gh && gh auth login        # once; needs admin rights on the repo
#   bash .github/scripts/configure-repo.sh pdfjiff/pdfjiff
#   bash .github/scripts/configure-repo.sh pdfjiff/pdfjiff --dry-run
#
# Sets: description, homepage, topics, features, merge strategy, labels, Dependabot
# alerts and security updates, secret scanning with push protection, private
# vulnerability reporting, read-only default workflow token, and a ruleset that
# protects main. Prints the few settings GitHub has no API for.
set -euo pipefail

REPO="${1:-}"
DRY_RUN=false
[ "${2:-}" = "--dry-run" ] && DRY_RUN=true
[[ "$REPO" =~ ^[A-Za-z0-9-]+/[A-Za-z0-9._-]+$ ]] || { echo "usage: $0 pdfjiff/REPO [--dry-run]" >&2; exit 2; }
command -v gh >/dev/null || { echo "Install the GitHub CLI first: https://cli.github.com" >&2; exit 1; }

run() {
    printf '+ %s\n' "$*"
    if ! $DRY_RUN; then "$@"; fi
}
api() { run gh api -H "Accept: application/vnd.github+json" "$@"; }

echo "== Repository profile"
run gh repo edit "$REPO" \
    --description "Fast, private PDF compression, merging and inspection for your terminal. Local, offline, one binary." \
    --homepage "https://github.com/$REPO#readme" \
    --enable-issues --enable-discussions --enable-wiki=false --enable-projects=false \
    --enable-squash-merge --enable-merge-commit=false --enable-rebase-merge=false \
    --delete-branch-on-merge --allow-update-branch \
    --add-topic pdf,pdf-tools,pdf-compression,compress-pdf,merge-pdf,cli,command-line-tool,rust,privacy,offline

echo "== Labels"
label() { run gh label create "$1" --repo "$REPO" --color "$2" --description "$3" --force; }
label "bug" d73a4a "Something doesn't work as documented"
label "enhancement" a2eeef "New feature or improvement"
label "documentation" 0075ca "Docs, examples and help text"
label "good first issue" 7057ff "Well scoped, with pointers; ideal for a first contribution"
label "help wanted" 008672 "Maintainers would welcome a pull request"
label "needs triage" ededed "Not yet reviewed by a maintainer"
label "question" d876e3 "Usage question; consider Discussions"
label "performance" fbca04 "Speed or memory use"
label "platform: windows" c5def5 "Windows-specific"
label "platform: macos" c5def5 "macOS-specific"
label "platform: linux" c5def5 "Linux-specific"
label "breaking-change" b60205 "Changes CLI flags, JSON output or the library API"
label "skip-changelog" ededed "Leave out of generated release notes"

echo "== Security"
api -X PUT "repos/$REPO/vulnerability-alerts"
api -X PUT "repos/$REPO/automated-security-fixes"
api -X PUT "repos/$REPO/private-vulnerability-reporting"
printf '%s' '{"security_and_analysis":{"secret_scanning":{"status":"enabled"},"secret_scanning_push_protection":{"status":"enabled"}}}' |
    api -X PATCH "repos/$REPO" --input - --silent

echo "== Actions: read-only GITHUB_TOKEN by default (jobs request more themselves)"
api -X PUT "repos/$REPO/actions/permissions/workflow" -f default_workflow_permissions=read -F can_approve_pull_request_reviews=false

echo "== Ruleset protecting main"
# Pull requests must pass "CI passed"; no force pushes or deletion. Repository admins may
# bypass so maintainer sync commits can be pushed directly (see docs/workflow.md).
ruleset='{
  "name": "Protect main",
  "target": "branch",
  "enforcement": "active",
  "conditions": {"ref_name": {"include": ["~DEFAULT_BRANCH"], "exclude": []}},
  "bypass_actors": [{"actor_id": 5, "actor_type": "RepositoryRole", "bypass_mode": "always"}],
  "rules": [
    {"type": "deletion"},
    {"type": "non_fast_forward"},
    {"type": "pull_request", "parameters": {
      "required_approving_review_count": 0, "dismiss_stale_reviews_on_push": true,
      "require_code_owner_review": false, "require_last_push_approval": false,
      "required_review_thread_resolution": true, "allowed_merge_methods": ["squash"]}},
    {"type": "required_status_checks", "parameters": {
      "strict_required_status_checks_policy": false,
      "required_status_checks": [{"context": "CI passed"}]}}
  ]
}'
existing=""
if ! $DRY_RUN; then
    existing="$(gh api "repos/$REPO/rulesets" --jq '.[] | select(.name == "Protect main") | .id' 2>/dev/null || true)"
fi
if [ -n "$existing" ]; then
    printf '%s' "$ruleset" | api -X PUT "repos/$REPO/rulesets/$existing" --input - --silent
else
    printf '%s' "$ruleset" | api -X POST "repos/$REPO/rulesets" --input - --silent
fi

cat <<EOF

Done. These have no API, so do them by hand:
  1. Settings → General → Social preview: upload docs/assets/social-preview.png
  2. Your account: Settings → Password and authentication → enable two-factor authentication
     (and require it for members, if the repository belongs to an organization)
  3. After the first release: Packages → pdfjiff → Package settings → Change visibility → Public
  4. Discussions: keep the Q&A, Ideas and Show and tell categories; pin a welcome post
  5. Optional release channels: see docs/releasing.md (Homebrew tap, Scoop bucket, crates.io)
EOF
