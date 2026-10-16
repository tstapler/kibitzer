#!/usr/bin/env bash
# Task 2.2.9c: proves the experiment-critical set never waits on the ship-gated modules.
#
# Copies the tree to a scratch directory, deletes every ship-gated module (a `mod` line carrying a
# `// ship-gated` marker, plus the named lcov/junit/staleness/enrich files and any script whose first
# lines carry a `# ship-gated` marker), greps the experiment-critical sources for use sites of those
# modules, then runs `cargo build` and the test_effectiveness tests in the copy. The grep runs first
# and separately because dropping a `mod` line leaves a dangling `use` that only fails the build and
# hides a real dependency.
#
# Usage: check-partition.sh [--no-build] [--self-test]
#   KIBITZER_PARTITION_SRC  tree to check (default: this repo)
#   CARGO_TARGET_DIR        build cache for the scratch build (default: <repo>/target/partition-check)
set -u

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo="$(cd "$here/../.." && pwd)"
src_root="${KIBITZER_PARTITION_SRC:-$repo}"
te="src/test_effectiveness"

# Files deleted even when no marked `mod` line names them (they may not exist yet).
named_gated=(
    "$te/adapters/lcov.rs"
    "$te/adapters/junit.rs"
    "$te/staleness.rs"
    "$te/enrich.rs"
)

fail() { echo "FAIL: $1" >&2; exit 1; }

# Deletes marked `mod` lines and the files they declare, plus the named files and gated scripts.
drop_ship_gated() {
    local root="$1" file dir name
    while IFS= read -r file; do
        dir="$(dirname "$file")"
        while IFS= read -r name; do
            [ -n "$name" ] || continue
            rm -rf "${dir:?}/$name.rs" "${dir:?}/$name"
            [ "$(basename "$file")" = "mod.rs" ] || [ "$(basename "$file")" = "main.rs" ] || \
                rm -rf "$dir/$(basename "$file" .rs)/$name.rs"
        done < <(grep -E '^[[:space:]]*(pub(\([a-z]+\))?[[:space:]]+)?mod[[:space:]]+[A-Za-z0-9_]+;.*// ship-gated' "$file" \
            | sed -E 's/^.*mod[[:space:]]+([A-Za-z0-9_]+);.*$/\1/')
        sed -i -E '/^[[:space:]]*(pub(\([a-z]+\))?[[:space:]]+)?mod[[:space:]]+[A-Za-z0-9_]+;.*\/\/ ship-gated/d' "$file"
    done < <(grep -rlE '// ship-gated' "$root/$te" --include='*.rs' 2>/dev/null)
    for file in "${named_gated[@]}"; do
        rm -rf "${root:?}/$file" "${root:?}/${file%.rs}"
    done
    if [ -d "$root/scripts/test-effectiveness" ]; then
        for file in "$root"/scripts/test-effectiveness/*; do
            [ -f "$file" ] && head -n 5 "$file" | grep -q '# ship-gated' && rm -f "$file"
        done
    fi
}

# Prints each use site of a ship-gated module in an experiment-critical source; returns 0 when none.
find_use_sites() {
    local root="$1" hits=0 file found
    local -a files=()
    while IFS= read -r file; do files+=("$file"); done < <(
        {
            find "$root/$te/report" -name '*.rs' 2>/dev/null
            find "$root/$te" -maxdepth 2 \( -name 'cli.rs' -o -name '*ranking*' -o -name '*envelope*' \) -name '*.rs' 2>/dev/null
        } | sort -u)
    local adapters='adapters::(lcov|junit)\b'
    local bare='(crate|super|self)::([A-Za-z0-9_]+::)*(staleness|enrich)\b'
    local use_bare='^[[:space:]]*(pub(\([a-z]+\))?[[:space:]]+)?use[[:space:]].*\b(staleness|enrich)\b'
    for file in "${files[@]}"; do
        found="$(grep -nE "$adapters|$bare|$use_bare" "$file" | grep -v '// ship-gated')" || continue
        printf '%s\n' "$found" | sed "s|^|${file#"$root"/}:|"
        hits=1
    done
    [ "$hits" -eq 0 ]
}

# Fake minimal tree exercising the grep stage: a clean tree passes, a seeded use site fails, a marked
# one passes. Names the validation rows of Story 2.2.9c.
self_test() {
    local tmp out
    tmp="$(mktemp -d)" || fail "mktemp"
    # shellcheck disable=SC2064
    trap "rm -rf '$tmp'" EXIT
    mkdir -p "$tmp/$te/report"
    printf 'pub mod report;\npub mod staleness; // ship-gated\n' > "$tmp/$te/mod.rs"
    printf 'pub fn age() {}\n' > "$tmp/$te/staleness.rs"
    printf 'use crate::test_effectiveness::matrix::Tier;\n' > "$tmp/$te/report/ranking.rs"
    printf 'fn main() {}\n' > "$tmp/$te/cli.rs"

    # partition_should_FindNoUseSiteOfShipGatedModules_When_ExperimentCriticalSourcesGrepped
    find_use_sites "$tmp" > /dev/null || fail "self-test: clean tree reported a use site"

    printf 'use crate::test_effectiveness::staleness::age;\n' >> "$tmp/$te/report/ranking.rs"
    out="$(find_use_sites "$tmp")" && fail "self-test: seeded use site was not detected"
    case "$out" in *"report/ranking.rs:2:"*) ;; *) fail "self-test: hit did not name file and line: $out" ;; esac

    printf 'use crate::test_effectiveness::adapters::lcov::read; // ship-gated\n' > "$tmp/$te/report/ranking.rs"
    find_use_sites "$tmp" > /dev/null || fail "self-test: marked use site was flagged"

    printf 'use crate::test_effectiveness::adapters::junit::read;\n' > "$tmp/$te/cli.rs"
    find_use_sites "$tmp" > /dev/null && fail "self-test: junit use site in cli.rs not detected"
    printf 'fn main() {}\n' > "$tmp/$te/cli.rs"

    drop_ship_gated "$tmp"
    [ ! -e "$tmp/$te/staleness.rs" ] || fail "self-test: marked module file survived"
    grep -q 'staleness' "$tmp/$te/mod.rs" && fail "self-test: marked mod line survived"
    echo "self-test ok"
}

build=1
for arg in "$@"; do
    case "$arg" in
        --no-build) build=0 ;;
        --self-test) self_test; exit 0 ;;
        *) fail "unknown argument $arg" ;;
    esac
done

scratch="$(mktemp -d)" || fail "mktemp"
trap 'rm -rf "$scratch"' EXIT
tar -C "$src_root" --exclude=./target --exclude=./.git --exclude=./.claude -cf - . | tar -C "$scratch" -xf - \
    || fail "copying the tree"

drop_ship_gated "$scratch"

if sites="$(find_use_sites "$scratch")"; then
    echo "no use site of adapters::lcov, adapters::junit, staleness or enrich in experiment-critical sources"
else
    echo "$sites" >&2
    fail "experiment-critical sources use a ship-gated module (lines above)"
fi

if [ "$build" -eq 1 ]; then
    export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$repo/target/partition-check}"
    cd "$scratch" || fail "cd scratch"
    cargo build || fail "cargo build without the ship-gated modules"
    cargo test --bin kibitzer test_effectiveness || fail "experiment-critical tests without the ship-gated modules"
fi
echo "partition ok"
