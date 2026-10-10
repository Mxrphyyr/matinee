#!/usr/bin/env bash
# Puts this folder on GitHub as a new repository, using the GitHub CLI.
#   ./publish-to-github.sh                     github.com/<you>/matinee, public
#   ./publish-to-github.sh my-name --private   another name, private
set -euo pipefail
cd "$(dirname "$0")"
name="${1:-matinee}"
visibility="${2:---public}"
if ! command -v gh >/dev/null; then
  echo "This needs the GitHub CLI (gh)."
  echo "  AerynOS: sudo moss install 'binary(gh)'"
  echo "  Others:  https://cli.github.com"
  exit 1
fi
gh auth status >/dev/null 2>&1 || gh auth login --web --git-protocol https --scopes workflow
# Pushing the release workflow (.github/workflows) needs GitHub's "workflow" permission.
if ! gh auth status 2>&1 | grep -q "'workflow'"; then
  echo "Allowing gh to push GitHub Actions workflows (needed once)…"
  gh auth refresh -h github.com -s workflow
fi
if [ ! -d .git ]; then
  git init -b main
  git add -A
  git commit -m "Matinee $(sed -n 's/^version *= *"\(.*\)"/\1/p' Cargo.toml | head -n 1)"
fi
if git remote get-url origin >/dev/null 2>&1; then
  git push -u origin main
else
  gh repo create "$name" "$visibility" --source=. --remote=origin --push \
    --description "A private, ad-free YouTube client for Linux. Rust, GTK 4, libadwaita and mpv."
fi
echo "Published: $(gh repo view --json url -q .url)"
echo "GitHub is building the first release now; see the Actions tab, then Releases."
