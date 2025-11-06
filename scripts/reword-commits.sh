#!/usr/bin/env bash
set -euo pipefail

# Non-interactively reword commit subjects based on docs/rebase-plan.md
# - Parses lines marked "- Reword → <new title>" (and optionally "- Optional reword → ...")
# - Rewrites only the subject line, preserving the body/footers and authorship
# - Uses git filter-branch to avoid interactive rebase
#
# Usage:
#   scripts/reword-commits.sh [--apply] [--include-optional] [--branch <branch>]
#
# Defaults:
#   - dry-run (preview only) unless --apply is provided
#   - uses current branch unless --branch is provided
#
# Notes:
#   - Creates backup refs under refs/original/...
#   - After running with --apply, force-push your branch:
#       git push --force-with-lease origin <branch>

PLAN_FILE="docs/rebase-plan.md"
APPLY=0
INCLUDE_OPTIONAL=0
BRANCH=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --apply)
      APPLY=1; shift ;;
    --include-optional)
      INCLUDE_OPTIONAL=1; shift ;;
    --branch)
      BRANCH=${2:-}; shift 2 ;;
    -h|--help)
      grep '^#' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *)
      echo "Unknown argument: $1" >&2; exit 2 ;;
  esac
done

if [[ ! -f "$PLAN_FILE" ]]; then
  echo "Plan file not found: $PLAN_FILE" >&2
  exit 1
fi

if [[ -z "$BRANCH" ]]; then
  BRANCH=$(git rev-parse --abbrev-ref HEAD)
fi

# Ensure working tree clean before history rewrite
if [[ "$APPLY" -eq 1 ]]; then
  if [[ -n "$(git status --porcelain)" ]]; then
    echo "Working tree not clean. Commit or stash changes before applying." >&2
    exit 1
  fi
fi

# Extract mapping of short SHA -> new subject from plan
tmp_map_short=$(mktemp)
awk -v include_optional="$INCLUDE_OPTIONAL" '
  function trim(s){ gsub(/^ +| +$/,"",s); return s }
  BEGIN { sha="" }
  /^- [0-9a-f]{7} / {
    # Example: "- bdd4e7b Initial commit"
    sha=$2
  }
  /^[[:space:]]*- Reword/ {
    line=$0
    sub(/^[[:space:]]*- Reword[^>]*→[[:space:]]*/, "", line)
    print sha "\t" line
  }
  /^[[:space:]]*- Optional reword/ {
    if (include_optional=="1") {
      line=$0
      sub(/^[[:space:]]*- Optional reword[^>]*→[[:space:]]*/, "", line)
      print sha "\t" line
    }
  }
' "$PLAN_FILE" > "$tmp_map_short"

if [[ ! -s "$tmp_map_short" ]]; then
  echo "No reword entries found in $PLAN_FILE (did you mean --include-optional?)." >&2
  rm -f "$tmp_map_short"
  exit 1
fi

# Resolve short SHAs to full commit SHAs and build final map
map_file=$(mktemp)
while IFS=$'\t' read -r short_sha new_subject; do
  if [[ -z "$short_sha" || -z "$new_subject" ]]; then continue; fi
  full_sha=$(git rev-parse --verify "$short_sha^{commit}")
  printf '%s\t%s\n' "$full_sha" "$new_subject" >> "$map_file"
done < "$tmp_map_short"
rm -f "$tmp_map_short"

echo "Planned rewordings (oldest→newest order not guaranteed):"
cat "$map_file" | while IFS=$'\t' read -r full_sha new_subject; do
  old_subject=$(git log -n1 --pretty=%s "$full_sha")
  printf '  %s\n    from: %s\n    to:   %s\n' "${full_sha:0:7}" "$old_subject" "$new_subject"
done

if [[ "$APPLY" -eq 0 ]]; then
  echo
  echo "Dry run only. Re-run with --apply to rewrite $BRANCH."
  exit 0
fi

export FILTER_BRANCH_SQUELCH_WARNING=1

echo
echo "Rewriting commit subjects on branch: $BRANCH"
echo "Backups will be created under refs/original/"

# Use filter-branch with a message filter that replaces the first line if mapped
MAPFILE="$map_file" \
git filter-branch -f --msg-filter '
  sha="$GIT_COMMIT"
  mapfile="'$map_file'"
  # Grab the commit message from stdin
  IFS= read -r first || true
  rest=$(cat)
  line=$(grep -F "^$sha\t" "$mapfile" || true)
  if [ -n "$line" ]; then
    newsubj="${line#*\t}"
    printf "%s\n" "$newsubj"
    printf "%s" "$rest"
  else
    if [ -n "$first" ]; then printf "%s\n" "$first"; fi
    printf "%s" "$rest"
  fi
' -- "$BRANCH"

echo
echo "Done. Verify with: git log --oneline"
echo "If satisfied, push with: git push --force-with-lease origin $BRANCH"

