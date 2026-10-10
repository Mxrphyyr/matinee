#!/usr/bin/env bash
# Commits everything that changed and pushes it to GitHub, first bringing down
# anything changed on GitHub itself (such as README edits made on the website).
#   ./sync-to-github.sh "What changed"
set -euo pipefail

# One function, read in full before it runs: pulling can rewrite this very
# file mid-run, and bash would otherwise go on reading the new bytes.
main() {
  cd "$(dirname "$0")"
  # Pushing changes to .github/workflows over HTTPS needs the "workflow" permission.
  if command -v gh >/dev/null && git remote get-url origin 2>/dev/null | grep -q '^https://github.com/' \
    && ! gh auth status 2>&1 | grep -q "'workflow'"; then
    echo "Allowing gh to push GitHub Actions workflows (needed once)…"
    gh auth refresh -h github.com -s workflow
  fi
  git add -A
  if git diff --cached --quiet; then
    echo "Nothing new to commit."
  else
    git commit -m "${1:-Update Matinee}"
  fi
  # Bring down edits made on GitHub, with your commit placed on top of them.
  if ! git pull --rebase; then
    echo
    echo "Your changes and GitHub's edit the same lines, so git stopped to ask."
    echo "Run 'git status' to see which files, or 'git rebase --abort' to undo."
    exit 1
  fi
  git push
}
main "$@"; exit
