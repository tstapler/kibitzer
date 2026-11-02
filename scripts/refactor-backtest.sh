#!/usr/bin/env bash
# Dry-run backtest of the refactor engine (docs/refactor-engine.md). Not part of `cargo test`.
# Usage: scripts/refactor-backtest.sh [repo ...]   (default: kubernetes/kubernetes BurntSushi/ripgrep)
# For each repo from scripts/clone-backtest-repos.sh it starts `kibitzer mcp`, takes the 20
# packages with the most symbols, and calls list_refactor_operations, simulate_refactor and
# plan_refactor (moving each package's first non-test file to a sibling package). Nothing is
# applied and no file is written. Fails if the server dies, a response is not valid JSON, or
# stderr contains "panicked". Prints per-repo outcome counts.
set -euo pipefail

ROOT=$(git rev-parse --show-toplevel)
CODE_ROOT=${CODE_ROOT:-$HOME/code/github.com}
TOP_N=${TOP_N:-20}
(cd "$ROOT" && cargo build --release -q)
BIN=${CARGO_TARGET_DIR:-$ROOT/target}/release/kibitzer

REPOS=("$@")
[ ${#REPOS[@]} -gt 0 ] || REPOS=(kubernetes/kubernetes BurntSushi/ripgrep)

status=0
for repo in "${REPOS[@]}"; do
  dir="$CODE_ROOT/$repo"
  if [ ! -d "$dir" ]; then
    echo "missing: $dir (run scripts/clone-backtest-repos.sh)" >&2
    status=1
    continue
  fi
  python3 -I - "$BIN" "$dir" "$repo" "$TOP_N" <<'PY' || status=1
import collections, json, subprocess, sys

binary, repo_dir, repo, top_n = sys.argv[1], sys.argv[2], sys.argv[3], int(sys.argv[4])
proc = subprocess.Popen([binary, "mcp"], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                        stderr=subprocess.PIPE, text=True)
next_id = 0

def rpc(method, params=None, notify=False):
    global next_id
    msg = {"jsonrpc": "2.0", "method": method}
    if params is not None:
        msg["params"] = params
    if not notify:
        next_id += 1
        msg["id"] = next_id
    proc.stdin.write(json.dumps(msg) + "\n")
    proc.stdin.flush()
    if notify:
        return None
    line = proc.stdout.readline()
    if not line:
        raise SystemExit(f"{repo}: server died during {method}")
    return json.loads(line)

def call(tool, args):
    resp = rpc("tools/call", {"name": tool, "arguments": args})
    text = resp["result"]["content"][0]["text"]
    return json.loads(text)

rpc("initialize", {"protocolVersion": "2024-11-05", "capabilities": {},
                   "clientInfo": {"name": "refactor-backtest", "version": "0"}})
rpc("notifications/initialized", notify=True)

counts = collections.Counter()
symbols = collections.Counter()
files_by_pkg = collections.defaultdict(set)
cursor = None
while True:
    args = {"path": repo_dir, "limit": 1000, "include_private": True}
    if cursor:
        args["cursor"] = cursor
    page = call("list_architecture_symbols", args)
    for entry in page["symbols"]:
        f = entry["symbol"]["file"]
        if not f.endswith(".go"):
            continue
        symbols[entry["package"]] += 1
        if not f.endswith("_test.go"):
            files_by_pkg[entry["package"]].add(f)
    cursor = page.get("next_cursor")
    if not cursor:
        break

for pkg, _ in symbols.most_common(top_n):
    ops = call("list_refactor_operations", {"path": repo_dir, "package": pkg})
    counts["listed_ok" if isinstance(ops, list) else "list_error"] += 1
    if not ops or not files_by_pkg[pkg]:
        counts["no_operation"] += 1
        continue
    op = {"kind": "MoveFiles", "source_package": pkg, "target_package": pkg + "_moved",
          "files": [sorted(files_by_pkg[pkg])[0]]}
    report = call("simulate_refactor", {"path": repo_dir, "operation": op})
    counts["feasible" if report.get("feasible") else "infeasible"] += 1
    plan = call("plan_refactor", {"path": repo_dir, "operation": op})
    if "error" in plan:
        counts["plan_refused"] += 1
        continue
    json.loads(json.dumps(plan))
    counts["plan_ok"] += 1

proc.stdin.close()
stderr = proc.stderr.read()
proc.wait(timeout=30)
if "panicked" in stderr:
    print(f"{repo}: PANIC in server stderr", file=sys.stderr)
    sys.exit(1)
print(f"{repo}: {dict(counts)} (top {top_n} Go packages by symbol count, zero panics)")
PY
done
exit $status
