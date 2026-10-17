# test-no-assertion-python on servo/servo: triage summary

Repo commit `7821ec2dd43aeeefb1b65c4530732e6a1877f7f5`; kibitzer tip `28492c7` (debug build). Verdicts live in
`test-no-assertion-python.jsonl` (60 records; each note starts with its stratum tag `[A]` or `[B]`; counts below
derived with `jq` over that shard).

## Method

- Candidates: `find . -path ./.git -prune -o ( -name 'test*.py' -o -name '*_test.py' ) -type f`, then
  `kibitzer check native test-no-assertion-python <file>` per file (`xargs -P8`, one atomic write per file).
- Total: **527 findings in 120 files** (the earlier 512 / 120 figure was stale).
- Stratum A (non-vendored, no `third_party/` or `vendor/` path component): **30 findings in 13 files**. 30 <= 120, so all 30 triaged.
- Stratum B (vendored, `tests/wpt/tests/tools/third_party/**`): **497 findings in 107 files** (393 pytest, 39 attrs, 13 pytest-asyncio, 11 websockets, 9 packaging, 8 hpack, 6 pywebsocket3, 5 importlib_metadata, 4 zipp, 3 exceptiongroup, 2 pathlib2, 1 each six, more-itertools, iniconfig, h2). Seeded sample of 30:
  `shuf --random-source=<(yes 42) -n 30 B.tsv` (seed = `yes 42` stream, over the sorted unique `file<TAB>line<TAB>name` list).
- Stratum B is out of scope for a real user; it only shows rule behaviour on pytest/attrs idioms.
- Each verdict was made by opening the function; helpers in other files were located by grep before a finding was called TP or FP.

## Counts and precision

| Stratum | Findings | Triaged | TP smoke-only | TP real | FP | Precision TP/(TP+FP) | Strict (real TP only) |
|---|---|---|---|---|---|---|---|
| A servo non-vendored | 30 | 30 (all) | 17 | 1 | 12 | 18/30 = **60.0%** | 1/30 = 3.3% |
| B servo vendored | 497 | 30 (sample) | 9 | 1 | 20 | 10/30 = **33.3%** | 1/30 = 3.3% |

B's precision is a sample estimate (n=30 of 497), so treat it as roughly +/- 17 points.

## False-positive causes, ranked (A+B = 32)

| Rank | Cause | A | B | Total | Examples |
|---|---|---|---|---|---|
| 1 | pytester `LineMatcher` assertions (`result.stdout.fnmatch_lines`, `re_match_lines`, `no_fnmatch_line`) | 0 | 15 | 15 | `test_terminal.py`, `test_runner.py`, `test_config.py` |
| 2 | Helper in another file: inherited `self.run_wrapper_test` (`functional/base.py:130`, `assertEqual`) x8; `wait_and_assert_no_pause` (`devtools_tests/utils.py:189`, `raise AssertionError`) x3 | 11 | 0 | 11 | `test_handlers.py`, `test_debugger_tab.py` |
| 3 | Test-named function that is fixture data run by an outer pytester test, not a test | 0 | 4 | 4 | `example_scripts/warnings/test_group_warnings_by_message.py`, `collect_pytest_prefix/test_foo.py` |
| 4 | Explicit failure path without `assert`/`raises`: `jsonschema.validate` in `try ... except: print; raise e` | 1 | 0 | 1 | `test_valid.py:88` |
| 5 | Alias of `pytest.raises` (`py.test.raises`) inside a same-file helper `parse_a_error` | 0 | 1 | 1 | `test_iniconfig.py:141` |

## True positives

- Real (2): `test_config.py:365` `test_ssl_env_openssl` (A) is an empty body whose assertions are commented out; `packaging/tests/test_structures.py:14` `test_negative_infinity_repr` (B) is a bare `repr(NegativeInfinity) == "-Infinity"` with no `assert`, a genuine bug (the sibling `test_infinity_repr` has the same defect).
- Smoke-only (26 of 28 TPs): oracle is "does not raise" (`pickle.dumps(c)`, constructing a `ConfigBuilder`, `ConfigBuilder`-style `__init__` calls, attrs validators called with a valid value), a timeout (`future.result(1)`, `wait_for_pause`), or a subprocess exit code (`subprocess.check_call`). Several say so in a comment ("test passes if it doesn't throw", "the pass condition here is essentially don't raise TimeoutError").

## Suggested rule improvements (rule code not changed)

1. Add the pytester/`LineMatcher` assertion methods to the built-in assertion list: `fnmatch_lines`, `fnmatch_lines_random`, `re_match_lines`, `re_match_lines_random`, `no_fnmatch_line`, `no_re_match_line`, `assert_outcomes`, `parseoutcomes`-based checks. Removes all 15 of B's top cause.
2. Cross-file helpers (11 of A's 12 FPs): either treat a call to `self.<name>` / a bare call whose name starts with `assert`/`check`/`verify`/`expect`/`wait_and_assert` as an assertion by name, or resolve `from .base import ...` / `from utils import ...` one hop to a sibling file. The `assertion_helpers` option already works as a manual escape hatch.
3. Treat `raise AssertionError(...)`, and a `raise` inside `except` after logging, as assertions; accept `jsonschema.validate` and `unittest`-style `self.fail`.
4. Resolve `py.test.raises` / `pytest.raises` through attribute aliases (`py.test`), including when wrapped by a same-file helper.
5. Skip `example_scripts/`, `testdata/`, `fixtures/` directories (files executed by an outer test). Removes 4 of B's FPs.
6. Report the two real-defect shapes at higher confidence than the rest: an empty or comment-only body (`pass`), and a bare comparison expression statement (`x == y`) with no `assert`. Both were the only real TPs here.
7. Mark the smoke-only pattern (body whose only statements are construct/pickle/`.result(timeout)`/`check_call`, or a comment containing "doesn't throw"/"don't raise"/"nothing happens") as a lower-severity category so it is not reported as a defect. That is 26 of 28 TPs here.
