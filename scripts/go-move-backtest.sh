#!/usr/bin/env bash
# Backtest of `kibitzer refactor go` on the six standard repos (docs/go-pkg-move-ops.md, "Backtest").
# Usage:
#   scripts/go-move-backtest.sh all                      # the six runs recorded in the doc
#   scripts/go-move-backtest.sh one <host-repo> <label> [--module <dir>] [--pkgs-before P] [--pkgs-after P] -- <refactor go args...>
# Needs the corpus from scripts/clone-backtest-repos.sh (CODE_ROOT, default ~/code/github.com) and
# `go`; cassandra/servo/ripgrep/deno/vscode have no go.mod, so a small real Go module is copied
# into a throwaway git worktree of each (never overwriting host files). Nothing in the corpus
# checkout is modified; worktrees are removed unless KEEP=1. Prints, per run: go build/vet before,
# the tool's status and verify exit codes, go build/vet after, and the diff size.
set -u

ROOT=$(git rev-parse --show-toplevel)
CODE_ROOT=${CODE_ROOT:-$HOME/code/github.com}
BIN=${BIN:-$ROOT/target/release/kibitzer}
WORK=${WORK:-$(mktemp -d)}
export GOWORK=off GOFLAGS=${GOFLAGS:--mod=mod}

one() {
  local host=$1 label=$2 mod="" before="./..." after="./..."; shift 2
  while :; do
    case "${1:-}" in
      --module) mod=$2; shift 2 ;;
      --pkgs-before) before=$2; shift 2 ;;
      --pkgs-after) after=$2; shift 2 ;;
      *) break ;;
    esac
  done
  [ "${1:-}" = "--" ] && shift
  # ONLY=<substring> runs just the matching labels (e.g. ONLY=k8s).
  [ -z "${ONLY:-}" ] || [[ "$label" == *"$ONLY"* ]] || return 0
  local w="$WORK/$label"
  git -C "$host" worktree prune
  git -C "$host" worktree add -q --detach "$w" HEAD || return 2
  echo "[$label] host HEAD $(git -C "$host" rev-parse --short HEAD), $(git -C "$w" ls-files | wc -l) files"
  if [ -n "$mod" ]; then
    (cd "$mod" && git ls-files -z | grep -zv '^\.github/' | xargs -0 -I{} cp --parents -n {} "$w"/ 2>/dev/null)
    cp -n "$mod/go.mod" "$w/go.mod"; [ -f "$mod/go.sum" ] && cp -n "$mod/go.sum" "$w/go.sum"
  fi
  echo "[$label] $(grep ^module "$w/go.mod")"
  (
    cd "$w" || exit 2
    git add -A >/dev/null 2>&1
    git -c user.name=bt -c user.email=bt@example.com commit -qm baseline >/dev/null 2>&1
    # shellcheck disable=SC2086
    go build $before >/dev/null 2>&1; echo "[$label] before: go build $before exit $?"
    # shellcheck disable=SC2086
    go vet $before >/dev/null 2>&1;   echo "[$label] before: go vet $before   exit $?"
    "$BIN" refactor go "$@" --repo "$w" --apply >"$WORK/$label.json" 2>"$WORK/$label.err"
    echo "[$label] kibitzer exit $?"
    python3 -I - "$WORK/$label.json" <<'PY'
import json, sys
try:
    p = json.load(open(sys.argv[1]))
except Exception:
    print("  (no result JSON; see stderr)"); sys.exit()
v = p.get("verify") or {}
print("  status:", p.get("status"), "| verify exit codes:",
      {k: (v[k] or {}).get("exit_code") for k in ("build", "vet", "test") if k in v})
PY
    head -c 300 "$WORK/$label.err"
    # shellcheck disable=SC2086
    go build $after >/dev/null 2>&1; echo "[$label] after:  go build $after exit $?"
    # shellcheck disable=SC2086
    go vet $after >/dev/null 2>&1;   echo "[$label] after:  go vet $after   exit $?"
    git add -A >/dev/null 2>&1; echo "[$label] diff: $(git diff --cached --shortstat HEAD | tail -1)"
  )
  [ "${KEEP:-0}" = 1 ] || git -C "$host" worktree remove --force "$w"
}

case "${1:-}" in
  one) shift; one "$@" ;;
  all)
    H=$CODE_ROOT
    # The module needs Go 1.27 (system Go may be older) and its vendor tree; `./...` is too big.
    # It is a go.work workspace (its vendor tree is built for it), so GOWORK must not be `off`.
    GOWORK= GOFLAGS=-mod=vendor GOTOOLCHAIN=${K8S_GOTOOLCHAIN:-go1.27.0} \
      one "$H/kubernetes/kubernetes" k8s-rename --pkgs-before ./pkg/util/slice --pkgs-after ./pkg/util/sliceutil \
      -- rename-pkg --from k8s.io/kubernetes/pkg/util/slice --to k8s.io/kubernetes/pkg/util/sliceutil --affected-only
    GOWORK= GOFLAGS=-mod=vendor GOTOOLCHAIN=${K8S_GOTOOLCHAIN:-go1.27.0} \
      one "$H/kubernetes/kubernetes" k8s-extract --pkgs-before ./pkg/util/slice --pkgs-after "./pkg/util/slice ./pkg/util/stringsutil" \
      -- extract-decls --from k8s.io/kubernetes/pkg/util/slice --decls CopyStrings,SortStrings \
      --to k8s.io/kubernetes/pkg/util/stringsutil --affected-only
    one "$H/apache/cassandra" cassandra-chi --module "$H/go-chi/chi" -- move-files \
      --src middleware/sunset.go middleware/sunset_test.go --dest-pkg github.com/go-chi/chi/v5/zzmovedsunset
    one "$H/servo/servo" servo-logrus --module "$H/sirupsen/logrus" -- rename-pkg \
      --from github.com/sirupsen/logrus/hooks/test --to github.com/sirupsen/logrus/hooks/testx
    one "$H/BurntSushi/ripgrep" ripgrep-cobra --module "$H/spf13/cobra" -- rename-pkg \
      --from github.com/spf13/cobra/doc --to github.com/spf13/cobra/docx
    one "$H/denoland/deno" deno-cli --module "$H/urfave/cli" -- extract-decls \
      --from github.com/urfave/cli/v3 --decls FlagNames --to github.com/urfave/cli/v3/internal/flagnames
    # vscode ships invalid .go fixtures, so `./...` fails with or without the move: --affected-only.
    one "$H/microsoft/vscode" vscode-viper --module "$H/spf13/viper" -- rename-pkg \
      --from github.com/spf13/viper/internal/encoding/dotenv \
      --to github.com/spf13/viper/internal/encoding/dotenvx --affected-only
    ;;
  *) sed -n 2,9p "$0"; exit 2 ;;
esac
