#!/usr/bin/env bash
# Before/after corpus diff for the fan_in_out distinct-pair fix (docs/go-package-focus.md).
# Usage: scripts/fan-in-out-backtest.sh <baseline-rev> [out-dir]
# Builds <baseline-rev> in a throwaway worktree and the current tree (release), runs the
# coupling, instability and dip-concrete-coupling checks over the corpus repos from
# scripts/clone-backtest-repos.sh, and prints per-check finding counts plus identity
# changes with numbers normalized away. Needs ~several GB of disk under out-dir (default
# ~/.cache), NOT /tmp (often a small tmpfs).
set -euo pipefail

BASE_REV=${1:?baseline rev}
OUT=${2:-$HOME/.cache/kibitzer-fan-in-out-backtest}
ROOT=$(git rev-parse --show-toplevel)
CODE_ROOT=${CODE_ROOT:-$HOME/code/github.com}
REPOS=(kubernetes/kubernetes BurntSushi/ripgrep)
CHECKS=(coupling instability dip-concrete-coupling)

mkdir -p "$OUT"
BASE_TREE="$OUT/base"
[ -d "$BASE_TREE" ] || git -C "$ROOT" worktree add --detach "$BASE_TREE" "$BASE_REV" >/dev/null
(cd "$BASE_TREE" && cargo build --release -q)
(cd "$ROOT" && cargo build --release -q)

for repo in "${REPOS[@]}"; do
  tag=${repo//\//_}
  for chk in "${CHECKS[@]}"; do
    "$BASE_TREE/target/release/kibitzer" check architecture "$chk" "$CODE_ROOT/$repo" >"$OUT/$tag.$chk.before.txt" 2>&1 || true
    "$ROOT/target/release/kibitzer" check architecture "$chk" "$CODE_ROOT/$repo" >"$OUT/$tag.$chk.after.txt" 2>&1 || true
  done
done

python3 -I - "$OUT" <<'PY'
import re, sys, pathlib
out = pathlib.Path(sys.argv[1])
def ident(path):
    s = set()
    for line in path.read_text().splitlines():
        line = re.sub(r"^\S+ ", "", line.strip())
        s.add(re.sub(r"\d+(\.\d+)?", "_", line)[:170])
    return s
print(f"{'repo/check':55} lines(before->after)  identities removed/added")
for before in sorted(out.glob("*.before.txt")):
    after = before.with_name(before.name.replace(".before.", ".after."))
    b, a = ident(before), ident(after)
    nb = len(before.read_text().splitlines()); na = len(after.read_text().splitlines())
    print(f"{before.name[:-11]:55} {nb:>5} -> {na:<5}        -{len(b - a)} / +{len(a - b)}")
PY
echo "cleanup: git worktree remove --force $BASE_TREE"
