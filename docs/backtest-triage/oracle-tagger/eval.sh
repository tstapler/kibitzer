#!/usr/bin/env bash
# Runs the oracle tagger over every ground-truth finding and prints a confusion matrix.
# Usage: docs/backtest-triage/oracle-tagger/eval.sh [path/to/kibitzer]   (run from repo root)
set -eu
BIN=${1:-./target/debug/kibitzer}
GT=docs/backtest-triage/oracle-tagger/ground-truth.jsonl
SQ=$HOME/code/github.com/tstapler/stapler-squad
K8S=$HOME/code/github.com/kubernetes/kubernetes
OUT=$(mktemp -d)

jq -r '[.repo,.file]|@tsv' "$GT" | sort -u | while IFS=$'\t' read -r repo file; do
  root=$SQ; [ "$repo" = k8s ] && root=$K8S
  KIBITZER_ORACLE_TAGS=1 "$BIN" check native test-no-assertion-go "$root/$file" 2>&1 \
    | awk -v repo="$repo" -v root="$root/" '
        /^[^ ]+:[0-9]+: \[test-no-assertion\]/ { split($1, a, ":"); f=a[1]; sub(root, "", f); line=a[2] }
        /\[tag=/ { match($0, /\[tag=[^]]*\]/); t=substr($0, RSTART+5, RLENGTH-6); print repo "\t" f "\t" line "\t" t }'
done > "$OUT/tagged.tsv"

jq -R -s -c 'split("\n")[:-1] | map(split("\t") | {repo:.[0], file:.[1], line:(.[2]|tonumber), got:.[3]})[]' \
  "$OUT/tagged.tsv" > "$OUT/tagged.jsonl"

# Join ground truth to tagger output; a GT finding the tagger never reported is "missing".
jq -s -c --slurpfile gt "$GT" '
  (map({key: "\(.repo)|\(.file)|\(.line)", value: .got}) | from_entries) as $got
  | $gt[] | {repo, file, line, want: .tag, got: ($got["\(.repo)|\(.file)|\(.line)"] // "missing")}' \
  "$OUT/tagged.jsonl" > "$OUT/joined.jsonl"

cp "$OUT/joined.jsonl" docs/backtest-triage/oracle-tagger/joined.jsonl

echo "== confusion matrix (rows = ground truth, cols = tagger) =="
jq -s -r '
  (map(.got) | unique) as $cols
  | ((["want\\got"] + $cols) | @tsv),
  (group_by(.want)[] | . as $g | ([$g[0].want] + ($cols | map(. as $c | ($g | map(select(.got == $c)) | length | tostring)))) | @tsv)' \
  "$OUT/joined.jsonl" | column -t -s $'\t'

echo
echo "== per-tag precision / recall =="
jq -s -r '
  (map(.want) + map(.got) | unique) as $tags
  | $tags[] as $t
  | (map(select(.got == $t)) | length) as $pred
  | (map(select(.want == $t)) | length) as $act
  | (map(select(.got == $t and .want == $t)) | length) as $tp
  | [$t, "tp=\($tp)", "predicted=\($pred)", "actual=\($act)",
     "precision=\(if $pred == 0 then "n/a" else (($tp / $pred * 100) | round | tostring) + "%" end)",
     "recall=\(if $act == 0 then "n/a" else (($tp / $act * 100) | round | tostring) + "%" end)"] | @tsv' \
  "$OUT/joined.jsonl" | column -t -s $'\t'

echo
jq -s -r '
  length as $n
  | (map(select(.got != "untagged" and .got != "missing")) | length) as $tagged
  | (map(select(.got == "untagged")) | length) as $untagged
  | (map(select(.want == .got)) | length) as $exact
  | "total=\($n) tagged=\($tagged) untagged=\($untagged) missing=\($n - $tagged - $untagged) exact-match=\($exact)"' \
  "$OUT/joined.jsonl"

echo
echo "== mismatches =="
jq -s -r '.[] | select(.want != .got) | [.want, .got, .file, .line] | @tsv' "$OUT/joined.jsonl" | column -t -s $'\t'
