#!/usr/bin/env bash
# Commits everything that changed and pushes it to GitHub.
#   ./sync-to-github.sh "What changed"
set -euo pipefail
cd "$(dirname "$0")"
git add -A
if git diff --cached --quiet; then
  echo "Nothing new to commit."
else
  git commit -m "${1:-Update Matinee}"
fi
git push
