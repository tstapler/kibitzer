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
assert_file() { # name path: the file must exist
  if [ -e "$2" ]; then pass "$1"; else fail "$1: $2 is missing"; fi
}
assert_no_file() { # name path: the file must not exist
  if [ -e "$2" ]; then fail "$1: $2 exists"; else pass "$1"; fi
}

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

# --- notice, progress and eta ---------------------------------------------------------------
new_module "$work/prog" 6
all=$(cd "$work/prog" && "$script" ./... 2>&1)
first_notice=$(grep -n 'executes repo test code with your environment and credentials' <<<"$all" | head -n1 | cut -d: -f1)
first_progress=$(grep -n '^\[1/6\]' <<<"$all" | head -n1 | cut -d: -f1)
if [ -n "$first_notice" ] && [ -n "$first_progress" ] && [ "$first_notice" -lt "$first_progress" ]; then
  pass "executes-code notice precedes the first [k/N] line"
else
  fail "notice (line ${first_notice:-none}) does not precede first progress (line ${first_progress:-none})"
fi
assert_eq "progress lines for 6 tests" 6 "$(grep -c '^\[[0-9]/6\] ' <<<"$all")"
assert_eq "no eta before 5 tests finished" 0 "$(grep -E '^\[[1-4]/6\] ' <<<"$all" | grep -c 'eta')"
assert_eq "eta from the 5th test on" 2 "$(grep -E '^\[[5-6]/6\] ' <<<"$all" | grep -c ' | eta about [0-9]*:[0-9][0-9]$' || true)"
assert_contains "progress line shape" "$all" '[1/6] example.com/m TestT1 ('

new_module "$work/quiet" 3
all=$(cd "$work/quiet" && "$script" --quiet ./... 2>&1)
assert_eq "--quiet hides progress" 0 "$(grep -c '^\[' <<<"$all" || true)"
assert_contains "--quiet keeps the notice" "$all" 'executes repo test code with your environment and credentials'

# --- interrupt and --resume ------------------------------------------------------------------
mkdir -p "$work/fakebin"
cat >"$work/fakebin/go" <<EOF
#!/usr/bin/env bash
case " \$* " in *" -coverprofile="*) sleep 0.7 ;; esac
exec "$(command -v go)" "\$@"
EOF
chmod +x "$work/fakebin/go"

mtime() { stat -c %y "$1" 2>/dev/null || stat -f %m "$1"; }
count_profiles() { find "$1" -maxdepth 1 -name '*.out' 2>/dev/null | wc -l; }

new_module "$work/intr" 6
(cd "$work/intr" && PATH="$work/fakebin:$PATH" exec "$script" ./... >"$work/intr.log" 2>&1) &
pid=$!
for _ in $(seq 1 300); do
  [ "$(count_profiles "$work/intr/profiles")" -ge 2 ] && break
  sleep 0.1
done
kill -TERM "$pid"
wait "$pid" || true
msg=$(cat "$work/intr.log")
if [[ $msg =~ interrupted\ at\ ([0-9]+)/6:\ wrote\ profiles/manifest.partial.json\;\ re-run\ with\ --resume\ to\ continue,\ or\ delete\ profiles/\ to\ start\ over ]]; then
  k=${BASH_REMATCH[1]}
  if [ "$k" -ge 2 ] && [ "$k" -lt 6 ]; then pass "interrupt message names $k/6"; else fail "interrupted at $k/6 is out of range"; fi
else
  fail "interrupt message missing or malformed: [$msg]"
fi
assert_no_file "no final manifest.json after interrupt" "$work/intr/profiles/manifest.json"
partial=$(cat "$work/intr/profiles/manifest.partial.json")
assert_contains "partial manifest is marked partial" "$partial" '"partial": true'
assert_contains "partial manifest records finished tests" "$partial" '"test": "TestT1", "status": "ok"'

status=0
msg=$(cd "$work/intr" && "$script" ./... 2>&1) || status=$?
assert_eq "re-run without --resume is refused" 1 "$status"
assert_contains "refusal points at --resume" "$msg" "re-run with --resume to continue, or delete profiles/ to start over"

before=$(mtime "$work/intr/profiles/example.com%2Fm__TestT1.out")
(cd "$work/intr" && "$script" --resume --quiet ./... >"$work/resume.log" 2>&1)
assert_eq "resume does not touch a finished profile" "$before" "$(mtime "$work/intr/profiles/example.com%2Fm__TestT1.out")"
assert_file "resume writes the final manifest.json" "$work/intr/profiles/manifest.json"
assert_no_file "resume removes manifest.partial.json" "$work/intr/profiles/manifest.partial.json"
assert_eq "all six profiles after resume" 6 "$(count_profiles "$work/intr/profiles")"
assert_contains "resume summary counts every test" "$(cat "$work/resume.log")" "6 tests, mode=isolated"

if [ "$failures" -ne 0 ]; then
  printf '%s assertion(s) failed\n' "$failures" >&2
  exit 1
fi
printf 'all assertions passed\n'
