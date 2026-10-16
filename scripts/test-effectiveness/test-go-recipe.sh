#!/usr/bin/env bash
# Runs per-test-go-coverage.sh against throwaway Go modules and the committed fixtures, and asserts
# the output and error messages of Story 2.2.2 (design/ux.md section 8). Needs `go`.
set -euo pipefail

here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
script="$here/per-test-go-coverage.sh"
fixtures="$here/../../testdata/test-effectiveness/gocover"
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

failures=0
fail() {
  printf 'FAIL: %s\n' "$*" >&2
  failures=$((failures + 1))
}
pass() { printf 'ok: %s\n' "$*"; }

assert_eq() { # name expected actual
  if [ "$2" = "$3" ]; then pass "$1"; else fail "$1: expected [$2], got [$3]"; fi
}
assert_contains() { # name haystack needle
  case $2 in *"$3"*) pass "$1" ;; *) fail "$1: [$3] not in [$2]" ;; esac
}

# new_module <dir> <test count>: a module with that many passing tests in package m.
new_module() {
  local dir=$1 n=$2 i
  mkdir -p "$dir"
  printf 'module example.com/m\n\ngo 1.21\n' >"$dir/go.mod"
  printf 'package m\n\nfunc F(n int) int { return n + 1 }\n' >"$dir/m.go"
  {
    printf 'package m\n\nimport "testing"\n'
    for ((i = 1; i <= n; i++)); do
      printf '\nfunc TestT%s(t *testing.T) {\n\tif F(%s) != %s {\n\t\tt.Fatal("bad")\n\t}\n}\n' "$i" "$i" "$((i + 1))"
    done
  } >"$dir/m_test.go"
}

# --- success output, file names and manifest -------------------------------------------------
new_module "$work/ok" 2
out=$(cd "$work/ok" && "$script" ./... 2>/dev/null)
lines=$(printf '%s\n' "$out" | wc -l)
assert_eq "success prints three lines" 3 "$lines"
go_version=$(go env GOVERSION)
assert_contains "line 1 summary" "$(sed -n 1p <<<"$out")" "per-test-go-coverage.sh: 2 tests, mode=isolated, $go_version, commit "
assert_eq "line 2 wrote" "wrote profiles/ (2 files) and profiles/manifest.json" "$(sed -n 2p <<<"$out")"
assert_eq "line 3 next" "next: kibitzer tests report --go-profiles profiles/" "$(sed -n 3p <<<"$out")"
assert_eq "profile names carry the encoded package" \
  "example.com%2Fm__TestT1.out example.com%2Fm__TestT2.out" \
  "$(cd "$work/ok/profiles" && echo ./*.out | sed 's#\./##g')"
manifest=$(cat "$work/ok/profiles/manifest.json")
assert_contains "manifest mode" "$manifest" '"mode": "isolated"'
assert_contains "manifest go version" "$manifest" "\"go_version\": \"$go_version\""
assert_contains "manifest tests_total" "$manifest" '"tests_total": 2'
if [ -e "$work/ok/profiles/.progress.tsv" ]; then fail "progress file left behind"; else pass "no progress scratch file left"; fi

# --- committed fixtures are what the recipe produces ----------------------------------------
for fx in twotest twopkg; do
  rm -rf "${work:?}/regen-$fx"
  (cd "$fixtures/$fx/mod" && "$script" --out "$work/regen-$fx" ./... >/dev/null 2>&1)
  if diff -r -x manifest.json "$work/regen-$fx" "$fixtures/$fx/profiles" >/dev/null; then
    pass "fixture $fx profiles match the recipe output"
  else
    fail "fixture $fx profiles differ from the recipe output"
  fi
done

# --- in_order mode ---------------------------------------------------------------------------
new_module "$work/ord" 3
out=$(cd "$work/ord" && "$script" --mode in_order ./... 2>/dev/null)
assert_contains "in_order summary" "$(sed -n 1p <<<"$out")" "3 tests, mode=in_order"
assert_contains "in_order manifest mode" "$(cat "$work/ord/profiles/manifest.json")" '"mode": "in_order"'
assert_eq "in_order first line of a profile" "mode: count" "$(head -n1 "$work/ord/profiles/example.com%2Fm__TestT2.out")"

# --- errors ----------------------------------------------------------------------------------
mkdir -p "$work/nomod"
status=0
msg=$(cd "$work/nomod" && "$script" ./... 2>&1) || status=$?
assert_eq "no go.mod exits non-zero" 1 "$status"
assert_contains "no go.mod message" "$msg" "error: no go.mod in "
assert_contains "no go.mod hint" "$msg" "; run from a module root"

status=0
msg=$(cd "$work/ok" && PATH=/nonexistent "$(command -v bash)" "$script" ./... 2>&1) || status=$?
assert_eq "go missing exits non-zero" 1 "$status"
assert_eq "go missing message" "error: go not found on PATH" "$msg"

# --- failing test is recorded, not fatal -------------------------------------------------------
new_module "$work/bad" 2
sed -i 's/F(2) != 3/F(2) != 99/' "$work/bad/m_test.go"
out=$(cd "$work/bad" && "$script" ./... 2>/dev/null)
assert_contains "failing summary" "$(sed -n 1p <<<"$out")" ", 1 failing, excluded"
assert_contains "manifest records failing" "$(cat "$work/bad/profiles/manifest.json")" '{"package": "example.com/m", "test": "TestT2"}'

if [ "$failures" -ne 0 ]; then
  printf '%s assertion(s) failed\n' "$failures" >&2
  exit 1
fi
printf 'all assertions passed\n'
