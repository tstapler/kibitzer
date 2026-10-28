# coverage.py fixtures

Inputs for `src/test_effectiveness/adapters/coveragepy.rs`.

## `three-test/` (real run)

`coverage.json` was produced by running `three-test/project/run.py` with coverage.py **7.13.4**
(`/usr/bin/python3`, `dynamic_context = test_function`, `coverage json --show-contexts` equivalent via
`Coverage.json_report(show_contexts=True)`). The tiny project (`app.py`, `test_app.py`) is original to this repo.
Tests: `test_pos`, `test_neg`, `test_noassert`; `app.py` line 2 is shared, line 4 is `test_pos` + `test_noassert`,
line 3 is unique to `test_neg`. Context names are `module.function` (no `|run` suffix: that comes from pytest-cov).

`coverage.json.run.json` is a hand-written sidecar (Story 2.2.9 schema) stating `attribution: isolated`.

### Recorded plausibility range

For this shape the fraction of tests with identical coverage is 2/3 and with zero unique coverage is 2/3. The adapter
treats a fraction above `0.9` (suites of at least 3 tests) as implausible; see `PLAUSIBLE_MAX_FRACTION` in the adapter.

## Hand-written inputs

The setup-phase (`|setup`), empty-context and unknown-version cases are built inline in the adapter's tests, hand-written
to the documented coverage.py 7.x JSON format; a real pytest-cov (`--cov-context=test`) run is pending because pytest
is not installed locally. Producer version assumed for those: 7.13.4.
