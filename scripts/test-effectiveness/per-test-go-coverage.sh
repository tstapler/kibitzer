#!/usr/bin/env bash
# Writes one Go cover profile per Test* function, for `kibitzer tests report --go-profiles`.
#
#   per-test-go-coverage.sh [--mode isolated|in_order] [--out DIR] [PKG_PATTERN...]
#
# Output (default DIR is profiles/):
#   <pkg-encoded>__<TestName>.out   the package import path is percent-encoded into the name
#                                   ('%' -> %25, '/' -> %2F, '_' -> %5F) so the first '__' splits
#                                   package from test and equal test names in two packages stay distinct
#   manifest.json                   go version, commit, mode, failing tests; `mode` is the attribution
#                                   source for the kibitzer adapter
#
# Modes:
#   isolated  (default) each test runs alone: `go test -run '^Test$'`
#   in_order  each test's profile is what it adds after the tests before it in the package ran
#             (cumulative run minus the previous cumulative run); costs O(tests^2) per package
#
# This script runs your repository's test code. kibitzer itself never does.
set -euo pipefail

SELF=per-test-go-coverage.sh

die() {
  printf 'error: %s\n' "$*" >&2
  exit 1
}

usage() {
  sed -n '2,19p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'
}

mode=isolated
out=profiles
pkgs=()
while (($#)); do
  case $1 in
    --mode) [ $# -ge 2 ] || die "--mode needs a value"; mode=$2; shift 2 ;;
    --mode=*) mode=${1#--mode=}; shift ;;
    --out) [ $# -ge 2 ] || die "--out needs a value"; out=$2; shift 2 ;;
    --out=*) out=${1#--out=}; shift ;;
    -h | --help) usage; exit 0 ;;
    --) shift; pkgs+=("$@"); break ;;
    -*) die "unknown option $1" ;;
    *) pkgs+=("$1"); shift ;;
  esac
done
[ ${#pkgs[@]} -gt 0 ] || pkgs=(./...)
case $mode in isolated | in_order) ;; *) die "--mode must be isolated or in_order, got $mode" ;; esac
out=${out%/}

command -v go >/dev/null 2>&1 || die "go not found on PATH"
[ -f go.mod ] || die "no go.mod in $PWD; run from a module root"

# '%' first so the escapes added after it are not re-encoded.
encode_pkg() {
  local s=$1
  s=${s//%/%25}
  s=${s//\//%2F}
  s=${s//_/%5F}
  printf '%s' "$s"
}

json_str() {
  local s=$1
  s=${s//\\/\\\\}
  s=${s//\"/\\\"}
  printf '"%s"' "$s"
}

# Prints "<import path> <TestName>" per test, in package then source order.
list_tests() {
  local listing line pkg
  local pending=()
  listing=$(go test -count=1 -list '^Test' "${pkgs[@]}" 2>&1) || die "go test -list failed:
$listing"
  while IFS= read -r line; do
    case $line in
      Test*) pending+=("$line") ;;
      ok*)
        read -r _ pkg _ <<<"$line"
        local t
        for t in ${pending[@]+"${pending[@]}"}; do printf '%s %s\n' "$pkg" "$t"; done
        pending=()
        ;;
      *) ;;
    esac
  done <<<"$listing"
}

coverpkg=$(IFS=,; printf '%s' "${pkgs[*]}")
covermode='set'
if [ "$mode" = in_order ]; then covermode=count; fi

# run_go_test <import path> <-run regex> <profile path>; returns go test's exit status.
run_go_test() {
  go test -count=1 -run "$2" -covermode="$covermode" -coverpkg="$coverpkg" \
    -coverprofile="$3" "$1" >/dev/null 2>&1
}

# delta <previous cumulative profile> <current cumulative profile>: counts the current run added.
delta_profile() {
  awk 'FILENAME == ARGV[1] { if (FNR > 1) prev[$1 " " $2] = $3; next }
       FNR == 1 { print; next }
       { d = $3 - prev[$1 " " $2]; if (d < 0) d = 0; print $1, $2, d }' "$1" "$2"
}

go_version=$(go env GOVERSION)
commit=$(git rev-parse HEAD 2>/dev/null || true)

mapfile -t tests < <(list_tests)
total=${#tests[@]}
[ "$total" -gt 0 ] || die "no Test* functions found in ${pkgs[*]}"

mkdir -p "$out"
rm -f "$out"/*.out "$out/manifest.json" "$out/manifest.partial.json" "$out/.progress.tsv"
progress="$out/.progress.tsv"
: >"$progress"

failing=0
prev_pkg=""
prev_cum=""
upto=()
scratch=$(mktemp -d)
trap 'rm -rf "$scratch"' EXIT

for entry in "${tests[@]}"; do
  read -r pkg test <<<"$entry"
  file="$out/$(encode_pkg "$pkg")__$test.out"
  status=ok
  if [ "$mode" = isolated ]; then
    run_go_test "$pkg" "^$test\$" "$scratch/cur.out" || status=failing
    [ -f "$scratch/cur.out" ] && mv "$scratch/cur.out" "$file"
  else
    if [ "$pkg" != "$prev_pkg" ]; then
      prev_pkg=$pkg
      upto=()
      prev_cum="$scratch/prev.out"
      printf 'mode: count\n' >"$prev_cum"
    fi
    upto+=("$test")
    regex="^($(IFS='|'; printf '%s' "${upto[*]}"))\$"
    run_go_test "$pkg" "$regex" "$scratch/cum.out" || status=failing
    if [ -f "$scratch/cum.out" ]; then
      delta_profile "$prev_cum" "$scratch/cum.out" >"$file"
      mv "$scratch/cum.out" "$scratch/prev.out"
    fi
  fi
  [ "$status" = ok ] || failing=$((failing + 1))
  printf '%s\t%s\t%s\n' "$pkg" "$test" "$status" >>"$progress"
done

write_manifest() { # write_manifest <path> <partial: 0|1>
  local first=1 pkg test status
  {
    printf '{\n  "schema_version": 1,\n  "tool": "go",\n  "go_version": %s,\n' "$(json_str "$go_version")"
    if [ -n "$commit" ]; then printf '  "commit": %s,\n' "$(json_str "$commit")"; else printf '  "commit": null,\n'; fi
    printf '  "mode": "%s",\n  "tests_total": %s,\n  "failing": [' "$mode" "$total"
    while IFS=$'\t' read -r pkg test status; do
      [ "$status" = failing ] || continue
      [ $first -eq 1 ] || printf ','
      first=0
      printf '\n    {"package": %s, "test": %s}' "$(json_str "$pkg")" "$(json_str "$test")"
    done <"$progress"
    [ $first -eq 1 ] && printf ']' || printf '\n  ]'
    [ "$2" -eq 0 ] || printf ',\n  "partial": true'
    printf '\n}\n'
  } >"$1"
}

write_manifest "$out/manifest.json" 0
rm -f "$progress"

files=$(find "$out" -maxdepth 1 -name '*.out' | wc -l)
short=${commit:0:7}
summary="$SELF: $total tests, mode=$mode, $go_version, commit ${short:-unknown}"
[ "$failing" -eq 0 ] || summary="$summary, $failing failing, excluded"
printf '%s\n' "$summary"
printf 'wrote %s/ (%s files) and %s/manifest.json\n' "$out" "$files" "$out"
printf 'next: kibitzer tests report --go-profiles %s/\n' "$out"
