#!/usr/bin/env bash
#
# check-node-kind-completeness.sh — CI gate for the typed-node-kind-migration.
#
# WHY THIS IS A CI SCRIPT, NOT A NEW KIBITZER CHECKER
# ----------------------------------------------------
# This is a one-repo, one-migration completeness grep for the
# typed-node-kind-migration project (project_plans/typed-node-kind-migration/),
# not a new generically-applicable inspection. requirements.md's Out-of-Scope
# section rules out building any new codegen/enum-generation mechanism, and a
# native kibitzer checker is a durable, cross-repo capability with its own
# backtest/corpus obligations (see this repo's CLAUDE.md) — a heavier
# commitment than this migration's scope calls for. A plain script scoped to
# the 23 files this migration touched is proportionate to what's actually
# needed: a mechanical presence/absence check, not a new generally-reusable
# check. See plan.md's Story 4.1.2 for the full reasoning.
#
# WHAT IT CHECKS
# ---------------
# 1. Completeness sweep: every raw tree_sitter `Node::kind()` string
#    comparison in the 23 in-scope files must be immediately explained by a
#    nearby comment mentioning "typed-node-kind-migration" — either the
#    standardized `// SEAM(typed-node-kind-migration): ...` banner, or the
#    inline "Non-goal (typed-node-kind-migration...)" prose style used
#    elsewhere in this migration (e.g. src/symbol_extract.rs's
#    find_child_by_kind, src/import_graph.rs's wildcard-marker comments,
#    src/dedup.rs's/src/plugin.rs's std::io::Error::kind() call-outs). An
#    unexplained hit is either a site the migration missed, or a newly
#    introduced one that needs the same treatment.
# 2. Seam-comment/seam-line invariant: the count of
#    `// SEAM(typed-node-kind-migration):` banners in src/checkers/rules.rs
#    and src/symbol_extract.rs must match EXPECTED_SEAM_COUNT below (13 as
#    confirmed at Story 4.1.1's completion: 8 in rules.rs, 5 in
#    symbol_extract.rs). If a future edit changes one of these pinned
#    cross-grammar comparison lines without also touching its banner — or
#    adds/removes a banner without a corresponding pinned site — this
#    diverges and the check fails, rather than drifting silently out of
#    sync in a scoped single-line diff a reviewer might not notice.
#
# If a future, deliberate change legitimately adds or removes a seamed site,
# update EXPECTED_SEAM_COUNT here and re-verify with:
#   rg -c 'SEAM\(typed-node-kind-migration\)' src/checkers/rules.rs src/symbol_extract.rs

set -euo pipefail

cd "$(git rev-parse --show-toplevel)"

if ! command -v rg >/dev/null 2>&1; then
  echo "check-node-kind-completeness: ripgrep (rg) is required but not found on PATH" >&2
  exit 1
fi

# The 23 files touched by the typed-node-kind-migration (Phases 2-4 of
# project_plans/typed-node-kind-migration/implementation/plan.md).
FILES=(
  src/dedup.rs
  src/plugin.rs
  src/checkers/go_ignored_error.rs
  src/checkers/go_blank_imports.rs
  src/checkers/go_type_switch_density.rs
  src/checkers/go_table_driven_test.rs
  src/checkers/go_error_context.rs
  src/checkers/go_bulk_fetch_linear_scan.rs
  src/go_call_resolution.rs
  src/checkers/java_error_context.rs
  src/checkers/java_ignored_error.rs
  src/checkers/java_lost_exception_cause.rs
  src/checkers/java_swallowed_interrupt.rs
  src/checkers/complexity.rs
  src/checkers/complexity_tests.rs
  src/checkers/primitive_obsession.rs
  src/isp_fat_interface.rs
  src/tree_walk.rs
  src/god_class.rs
  src/checkers/rules.rs
  src/symbol_extract.rs
  src/import_graph.rs
  src/declarations.rs
)

PATTERN='\.kind\(\)\s*(==|!=)|\.kind\(\)\.contains\(|match\s+\S+\.kind\(\)'
# Wording actually used across the 23 files' explaining comments — not just the
# standardized `// SEAM(typed-node-kind-migration): ...` banner, but also the
# looser "Non-goal"/anonymous-token prose the migration used elsewhere (e.g.
# "typed-node-kind migration" without a hyphen before "migration" in
# src/import_graph.rs, or a bare "anonymous token(s) ... no <Lang>Kind variant"
# call-out with no migration-name mention at all in
# src/checkers/go_error_context.rs). Deliberately broad since this check is
# scoped to only these 23 pre-vetted files.
EXPLAIN_MARKER='typed-node-kind[- ]migration|SEAM\(|anonymous (token|punctuation|keyword)|non-goal|named["'"'"']?\s*[:=]\s*false|std::io::Error::kind'
# How far above a hit to look for its explaining comment. Sized to cover the
# longest real doc-comment block seen in this migration: symbol_extract.rs's
# find_child_by_kind carries a single shared "Non-goal" doc comment that also
# covers two sibling functions (java_is_exported/kotlin_is_exported) defined
# below it — the farthest real hit (symbol_extract.rs:136) sits 33 lines below
# its explaining comment. Sized with headroom above that.
CONTEXT_WINDOW=40
SEAM_FILES=(src/checkers/rules.rs src/symbol_extract.rs)
EXPECTED_SEAM_COUNT=13

fail=0

for f in "${FILES[@]}"; do
  if [[ ! -f "$f" ]]; then
    echo "check-node-kind-completeness: in-scope file '$f' no longer exists — update this script's FILES list" >&2
    fail=1
    continue
  fi

  while IFS=: read -r line_no _rest; do
    [[ -z "$line_no" ]] && continue
    hit_text=$(sed -n "${line_no}p" "$f")
    trimmed_hit=$(sed -e 's/^[[:space:]]*//' <<<"$hit_text")
    # A hit inside a comment line (doc prose that quotes the pattern textually
    # to describe code elsewhere, e.g. import_graph.rs:628) isn't a real
    # `.kind()` call site — skip it.
    if [[ "$trimmed_hit" == //* ]]; then
      continue
    fi
    start=$(( line_no - CONTEXT_WINDOW ))
    (( start < 1 )) && start=1
    context=$(sed -n "${start},${line_no}p" "$f")
    if ! rg -qi -- "$EXPLAIN_MARKER" <<<"$context"; then
      echo "UNEXPLAINED: $f:$line_no: $trimmed_hit" >&2
      fail=1
    fi
  done < <(rg -n --no-heading "$PATTERN" "$f")
done

# --- Invariant 2: seam banner count vs. pinned seam-site count ---
SEAM_BANNER='SEAM\(typed-node-kind-migration\):'
actual_seam_count=0
for f in "${SEAM_FILES[@]}"; do
  count=$(rg -c -- "$SEAM_BANNER" "$f" 2>/dev/null || true)
  actual_seam_count=$(( actual_seam_count + ${count:-0} ))
done

if [[ "$actual_seam_count" -ne "$EXPECTED_SEAM_COUNT" ]]; then
  echo "SEAM COUNT MISMATCH: found $actual_seam_count '// SEAM(typed-node-kind-migration):' banners across ${SEAM_FILES[*]}, expected $EXPECTED_SEAM_COUNT." >&2
  echo "Either a seam banner was added/removed without updating EXPECTED_SEAM_COUNT in this script, or a pinned seam line was edited without updating its banner. Re-verify with:" >&2
  echo "  rg -n '$SEAM_BANNER' ${SEAM_FILES[*]}" >&2
  fail=1
fi

if [[ "$fail" -ne 0 ]]; then
  echo "" >&2
  echo "check-node-kind-completeness FAILED — see project_plans/typed-node-kind-migration/decisions/ADR-001-leave-cross-grammar-kind-comparisons-unmigrated.md for the seam policy." >&2
  exit 1
fi

echo "check-node-kind-completeness: OK ($actual_seam_count seam banners; 0 unexplained .kind() string comparisons across ${#FILES[@]} in-scope files)"
