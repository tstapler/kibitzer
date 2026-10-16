#!/usr/bin/env bash
# Writes one Go cover profile per Test* function, for `kibitzer tests report --go-profiles`.
#
#   per-test-go-coverage.sh [--mode isolated|in_order] [--out DIR] [--resume] [--quiet] [PKG_PATTERN...]
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
# Ctrl-C keeps finished profiles and writes manifest.partial.json; --resume continues from them.
# --quiet hides the progress lines only; the executes-code notice is always printed.
# This script runs your repository's test code. kibitzer itself never does.
set -euo pipefail

SELF=per-test-go-coverage.sh

die() {
  printf 'error: %s\n' "$*" >&2
  exit 1
}

usage() {
  sed -n '2,20p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'
}

mode=isolated
out=profiles
resume=0
quiet=0
pkgs=()
while (($#)); do
  case $1 in
    --mode) [ $# -ge 2 ] || die "--mode needs a value"; mode=$2; shift 2 ;;
    --mode=*) mode=${1#--mode=}; shift ;;
    --out) [ $# -ge 2 ] || die "--out needs a value"; out=$2; shift 2 ;;
    --out=*) out=${1#--out=}; shift ;;
    --resume) resume=1; shift ;;
    --quiet | -q) quiet=1; shift ;;
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

progress="$out/.progress.tsv"
if [ -f "$progress" ] && [ "$resume" -eq 0 ]; then
  die "$out/ holds an interrupted run; re-run with --resume to continue, or delete $out/ to start over"
fi
if [ -f "$progress" ]; then
  [ "$(head -n1 "$progress")" = "#mode	$mode" ] ||
    die "$out/ was started with a different mode; use --mode matching it, or delete $out/ to start over"
fi

# Printed before anything executes repo code (`go test -list` runs package init and TestMain too).
cat >&2 <<EOF
$SELF: this runs your test suite once per test (mode=$mode).
  It executes repo test code with your environment and credentials; run it in a
  checkout and shell you trust. Ctrl-C is safe: finished profiles are kept and
  --resume continues from them.
EOF

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
  local listing line pkg t
  local pending=()
  listing=$(go test -count=1 -list '^Test' "${pkgs[@]}" 2>&1) || die "go test -list failed:
$listing"
  while IFS= read -r line; do
    case $line in
      Test*) pending+=("$line") ;;
      ok*)
        read -r _ pkg _ <<<"$line"
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

gopid=""
# run_go_test <import path> <-run regex> <profile path>; returns go test's exit status.
# Runs in the background and is waited on so a signal reaches the trap while it runs.
run_go_test() {
  local rc=0
  go test -count=1 -run "$2" -covermode="$covermode" -coverpkg="$coverpkg" \
    -coverprofile="$3" "$1" >/dev/null 2>&1 &
  gopid=$!
  wait "$gopid" || rc=$?
  gopid=""
  return "$rc"
}

# delta_profile <previous cumulative profile> <current cumulative profile>: counts the current run added.
delta_profile() {
  awk 'FILENAME == ARGV[1] { if (FNR > 1) prev[$1 " " $2] = $3; next }
       FNR == 1 { print; next }
       { d = $3 - prev[$1 " " $2]; if (d < 0) d = 0; print $1, $2, d }' "$1" "$2"
}

now_ms() {
  local t=${EPOCHREALTIME/./}
  printf '%s' "$((t / 1000))"
}

mmss() { printf '%d:%02d' "$(($1 / 60))" "$(($1 % 60))"; }

go_version=$(go env GOVERSION)
commit=$(git rev-parse HEAD 2>/dev/null || true)

mapfile -t tests < <(list_tests)
total=${#tests[@]}
[ "$total" -gt 0 ] || die "no Test* functions found in ${pkgs[*]}"

scratch=$(mktemp -d)
mkdir -p "$out"
if [ ! -f "$progress" ]; then
  rm -f "$out"/*.out "$out/manifest.json" "$out/manifest.partial.json"
  printf '#mode\t%s\n' "$mode" >"$progress"
fi

# finished tests from an earlier run: "<pkg>\t<test>" -> status
declare -A finished=()
while IFS=$'\t' read -r pkg test status; do
  case $pkg in '#'*) continue ;; esac
  finished["$pkg	$test"]=$status
done <"$progress"

finished_count() { grep -vc '^#' "$progress" || true; }

# write_manifest <path> <partial: 0|1>; reads the progress file.
write_manifest() {
  local first=1 pkg test status fin
  {
    printf '{\n  "schema_version": 1,\n  "tool": "go",\n  "go_version": %s,\n' "$(json_str "$go_version")"
    if [ -n "$commit" ]; then printf '  "commit": %s,\n' "$(json_str "$commit")"; else printf '  "commit": null,\n'; fi
    printf '  "mode": "%s",\n  "tests_total": %s,\n  "failing": [' "$mode" "$total"
    while IFS=$'\t' read -r pkg test status; do
      case $pkg in '#'*) continue ;; esac
      [ "$status" = failing ] || continue
      [ $first -eq 1 ] || printf ','
      first=0
      printf '\n    {"package": %s, "test": %s}' "$(json_str "$pkg")" "$(json_str "$test")"
    done <"$progress"
    if [ $first -eq 1 ]; then printf ']'; else printf '\n  ]'; fi
    if [ "$2" -eq 1 ]; then
      first=1
      printf ',\n  "partial": true,\n  "finished": ['
      while IFS=$'\t' read -r pkg test status; do
        case $pkg in '#'*) continue ;; esac
        [ $first -eq 1 ] || printf ','
        first=0
        fin=$(printf '{"package": %s, "test": %s, "status": "%s"}' "$(json_str "$pkg")" "$(json_str "$test")" "$status")
        printf '\n    %s' "$fin"
      done <"$progress"
      if [ $first -eq 1 ]; then printf ']'; else printf '\n  ]'; fi
    fi
    printf '\n}\n'
  } >"$1"
}

interrupted() {
  trap '' INT TERM
  if [ -n "$gopid" ]; then
    pkill -TERM -P "$gopid" 2>/dev/null || true
    kill "$gopid" 2>/dev/null || true
  fi
  write_manifest "$out/manifest.partial.json" 1
  printf '%s: interrupted at %s/%s: wrote %s/manifest.partial.json; re-run with --resume to continue, or delete %s/ to start over\n' \
    "$SELF" "$(finished_count)" "$total" "$out" "$out" >&2
  rm -rf "$scratch"
  exit 130
}
trap interrupted INT TERM
trap 'rm -rf "$scratch"' EXIT

tty=0
if [ -t 2 ]; then tty=1; fi
step=$((total / 10))
[ "$step" -ge 1 ] || step=1
started_ms=$(now_ms)
ran=0
done_so_far=$(finished_count)

report_progress() { # report_progress <pkg> <test> <test seconds as s.d>
  [ "$quiet" -eq 0 ] || return 0
  if [ "$tty" -eq 0 ] && [ "$done_so_far" -ne "$total" ] && [ $((done_so_far % step)) -ne 0 ]; then
    return 0
  fi
  local elapsed_s=$((($(now_ms) - started_ms) / 1000)) line
  line=$(printf '[%*d/%d] %s %s (%ss) | elapsed %s' "${#total}" "$done_so_far" "$total" "$1" "$2" "$3" "$(mmss "$elapsed_s")")
  if [ "$ran" -ge 5 ]; then
    line="$line | eta about $(mmss $((elapsed_s * (total - done_so_far) / ran)))"
  fi
  printf '%s\n' "$line" >&2
}

failing=0
prev_pkg=""
upto=()
need_prev=0
for entry in "${tests[@]}"; do
  read -r pkg test <<<"$entry"
  file="$out/$(encode_pkg "$pkg")__$test.out"
  key="$pkg	$test"

  if [ "$mode" = in_order ] && [ "$pkg" != "$prev_pkg" ]; then
    prev_pkg=$pkg
    upto=()
    need_prev=0
    printf 'mode: count\n' >"$scratch/prev.out"
  fi

  if [ -n "${finished[$key]+set}" ] && { [ -f "$file" ] || [ "${finished[$key]}" = failing ]; }; then
    [ "${finished[$key]}" != failing ] || failing=$((failing + 1))
    if [ "$mode" = in_order ]; then
      upto+=("$test")
      need_prev=1
    fi
    continue
  fi

  t0=$(now_ms)
  status=ok
  if [ "$mode" = isolated ]; then
    rm -f "$scratch/cur.out"
    run_go_test "$pkg" "^$test\$" "$scratch/cur.out" || status=failing
    if [ -f "$scratch/cur.out" ]; then mv "$scratch/cur.out" "$file"; fi
  else
    if [ "$need_prev" -eq 1 ]; then
      # Resuming mid-package: rebuild the cumulative profile of the finished prefix without touching its files.
      run_go_test "$pkg" "^($(IFS='|'; printf '%s' "${upto[*]}"))\$" "$scratch/prev.out" || true
      need_prev=0
    fi
    upto+=("$test")
    rm -f "$scratch/cum.out"
    run_go_test "$pkg" "^($(IFS='|'; printf '%s' "${upto[*]}"))\$" "$scratch/cum.out" || status=failing
    if [ -f "$scratch/cum.out" ]; then
      delta_profile "$scratch/prev.out" "$scratch/cum.out" >"$scratch/delta.out"
      mv "$scratch/delta.out" "$file"
      mv "$scratch/cum.out" "$scratch/prev.out"
    fi
  fi
  [ "$status" = ok ] || failing=$((failing + 1))
  printf '%s\t%s\t%s\n' "$pkg" "$test" "$status" >>"$progress"
  ran=$((ran + 1))
  done_so_far=$((done_so_far + 1))
  elapsed_ms=$(($(now_ms) - t0))
  report_progress "$pkg" "$test" "$((elapsed_ms / 1000)).$(((elapsed_ms % 1000) / 100))"
done

write_manifest "$out/manifest.json" 0
rm -f "$progress" "$out/manifest.partial.json"

files=$(find "$out" -maxdepth 1 -name '*.out' | wc -l)
short=${commit:0:7}
summary="$SELF: $total tests, mode=$mode, $go_version, commit ${short:-unknown}"
[ "$failing" -eq 0 ] || summary="$summary, $failing failing, excluded"
printf '%s\n' "$summary"
printf 'wrote %s/ (%s files) and %s/manifest.json\n' "$out" "$files" "$out"
printf 'next: kibitzer tests report --go-profiles %s/\n' "$out"
