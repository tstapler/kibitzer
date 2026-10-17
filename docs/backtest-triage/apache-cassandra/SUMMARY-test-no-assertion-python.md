# test-no-assertion-python on apache/cassandra: triage summary

Repo commit `c1030321071ab9f10b2f19d63087ab9313035001`; kibitzer tip `28492c7`. Verdicts live in
`test-no-assertion-python.jsonl` (19 records, stratum tag `[C]`; counts below derived with `jq`).

## Method

- Candidates: 9 `test*.py` / `*_test.py` files, each run with `kibitzer check native test-no-assertion-python <file>`.
- Total: **19 findings in 3 files** (`pylib/cqlshlib/test/test_cql_parsing.py` 9, `test_cqlsh_completion.py` 1, `test_cqlsh_output.py` 9). 19 <= 120, so all 19 triaged; no sampling.
- Other corpora checked for Python tests: `microsoft/vscode` has 8 candidate files and `kubernetes/kubernetes` has 1, both with 0 findings; `denoland/deno` and `gitlabhq/gitlabhq` have none, so they contribute nothing.

## Counts and precision

| Stratum | Findings | Triaged | TP smoke-only | TP real | FP | Precision | Strict (real TP only) |
|---|---|---|---|---|---|---|---|
| C cassandra | 19 | 19 (all) | 0 | 18 | 1 | 18/19 = **94.7%** | 18/19 = 94.7% |

## True positives (18)

All 18 are empty stubs (`def test_x(self): pass`, no skip decorator): 9 in `test_cql_parsing.py`
(`test_parse_batch`, `test_parse_create_table`, ... `test_parse_select_token`), 1 in `test_cqlsh_completion.py`
(`test_complete_in_uuid`), 8 in `test_cqlsh_output.py` (`test_help`, `test_printing_parse_error`, ...,
`test_empty_line`). They pass vacuously and read as unfinished TODOs. Real defects by the Go-summary standard.

## False positive (1)

| Cause | Count | Test |
|---|---|---|
| Helper in another file asserts | 1 | `test_help_types` calls `c.cmd_and_response('help types')`; `run_cqlsh.py:295` asserts `echo == cmd` when `tty` is true. A weak check, but a real assertion. |

## Suggested rule improvements (rule code not changed)

1. Cross-file helper lookup, or an `assertion_helpers` entry for `cmd_and_response`, would clear the one FP.
2. Report the empty-body stub (`pass`, docstring-only, comment-only) as its own higher-confidence finding; here it is 18 of 18 real TPs, so it deserves a separate rule id or severity.

## After round 2

Same commit `c1030321`; `find` + `xargs -P8` over `kibitzer check native <rule> <file>`, debug builds before/after, compared by `file:line` against the shard with `comm`.

| | Before | After |
|---|---|---|
| Findings | 19 | 19 |
| true_positive records still reported | 18 of 18 | 18 of 18 |
| false_positive records still reported | 1 | 1 |

Precision unchanged at 18 / 19 = **94.7%**. The one FP (`test_help_types`, `pylib/cqlshlib/test/test_cqlsh_output.py:898`)
is a cross-file helper (`cmd_and_response` in `run_cqlsh.py`); `assertion_helpers` remains its escape hatch.
