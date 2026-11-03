# replace-magic-literal: numeric Rust noise (item efe0c3c7, issue #142)

The backlog item lists 0 formal acceptance criteria; these are derived from its "Suggested direction".

1. Float spellings of allowlisted values (`0.0`, `1.0`, `0.`, `1e0`) are treated as `0`/`1`; `2`/`2.0` and `0.5` are allowlisted.
2. Numerics inside Rust `#[cfg(test)]` / `#![cfg(test)]` items neither fire nor count toward production totals.
3. A string that is only a format placeholder (`"{p}"`, `"{}"`, `"{:?}"`) is never flagged.
4. Backtested before/after on the real-world corpus (`docs/backtest-repos.md`), recorded in `docs/syntax-rules.md`.

Research, plan and validation already exist from the earlier pass in
`project_plans/fix-magic-literal-noise/` (research.md, plan.md, validation.md); the implementation landed
in a65a089. This file is the requirements entry point only.
