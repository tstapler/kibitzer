#!/usr/bin/env bash
# Scans experiment artifacts for secret-shaped strings (Story 0.3.5, NFR3).
# Usage: secret-scan.sh --staged        scan staged files under the scoped paths (pre-commit mode)
#        secret-scan.sh <file|dir>...   scan the given paths
#        secret-scan.sh --install       install this script as the repo pre-commit hook (opt-in)
# Exit 1 and name file:line:pattern on a match; exit 0 otherwise.
set -u
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo="$(cd "$here/../.." && pwd)"
patterns="$here/secret-patterns.txt"
scope_re='^(project_plans/test-effectiveness-critique/experiment/results/|project_plans/test-effectiveness-critique/experiment/.*\.log$|docs/backtest-triage/)'

if [ "${1:-}" = "--install" ]; then
  hook="$(git -C "$repo" rev-parse --git-path hooks/pre-commit)"
  case "$hook" in /*) ;; *) hook="$repo/$hook" ;; esac
  mkdir -p "$(dirname "$hook")"
  printf '#!/usr/bin/env bash\nexec "%s" --staged\n' "$here/secret-scan.sh" > "$hook"
  chmod +x "$hook"
  echo "installed pre-commit hook at $hook"
  exit 0
fi

files=()
if [ "${1:-}" = "--staged" ]; then
  while IFS= read -r f; do
    [[ "$f" =~ $scope_re ]] && files+=("$repo/$f")
  done < <(git -C "$repo" diff --cached --name-only --diff-filter=ACM)
else
  for p in "$@"; do
    if [ -d "$p" ]; then
      while IFS= read -r f; do files+=("$f"); done < <(find "$p" -type f)
    else
      files+=("$p")
    fi
  done
fi

status=0
for f in "${files[@]+"${files[@]}"}"; do
  [ -f "$f" ] || continue
  while IFS= read -r pat; do
    case "$pat" in ''|'#'*) continue ;; esac
    hit="$(grep -nP -m1 -- "$pat" "$f" 2>/dev/null | head -1)"
    if [ -n "$hit" ]; then
      echo "secret-scan: $f:${hit%%:*} matches pattern: $pat" >&2
      status=1
    fi
  done < "$patterns"
done
[ "$status" -ne 0 ] && echo "secret-scan: commit refused; remove the match or scrub the artifact" >&2
exit $status
