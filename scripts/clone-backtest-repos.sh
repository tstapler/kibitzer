#!/usr/bin/env bash
# Clones the real-world backtest corpus (docs/backtest-repos.md) into
# ~/code/github.com/<owner>/<repo>, shallow and idempotent — safe to re-run; an
# existing clone is left untouched rather than re-fetched, since these repos are used
# for one-shot structural checks, not for staying current with upstream.
set -euo pipefail

CODE_ROOT="${CODE_ROOT:-$HOME/code/github.com}"

REPOS=(
  "kubernetes/kubernetes"
  "apache/cassandra"
  "servo/servo"
  "BurntSushi/ripgrep"
  "denoland/deno"
  "microsoft/vscode"
)

for repo in "${REPOS[@]}"; do
  dest="$CODE_ROOT/$repo"
  if [ -d "$dest/.git" ]; then
    echo "skip (already cloned): $repo"
    continue
  fi
  echo "cloning: $repo -> $dest"
  mkdir -p "$(dirname "$dest")"
  git clone --depth 1 "https://github.com/$repo.git" "$dest"
done

echo
echo "tstapler/stapler-squad is deliberately not cloned here — use your existing local" \
     "checkout (e.g. ~/Programming/stapler-squad). See docs/backtest-repos.md."
