# Research: Architecture — test-effectiveness-critique

Scope: targeted reads (grep + offset/limit) of `src/checker.rs`, `src/config.rs`, `src/check.rs`,
`src/plugin.rs`, `src/main.rs`, `src/affected.rs`, `src/arch_model.rs`, `src/import_graph.rs`,
`src/hotspots.rs`, `src/checkers/go_table_driven_test.rs`, `src/tree_walk.rs`, `covgate.toml`,
`docs/output-formats.md`, `docs/plugins.md`, `docs/backtesting.md`, and the `affected-tests` /
`checker-plugin-system` / `inline-ignore-syntax` plan docs. Line numbers are against working tree at
`d76c1ab` (master, 2026-10-11). Every claim below was read from the cited line; items marked INFERRED are
design judgment, not code facts.

## 1. The checker model today

### 1a. Per-file native checkers (hook-path)

- `Checker` trait: `name`, `description`, `language() -> Option<Language>`, `file_globs`, and
  `check(&self, file, &CheckContext) -> Result<Vec<Finding>>` (`src/checker.rs:172-184`). A checker
  sees exactly one file: `CheckContext { source, tree }` (`src/checker.rs:164-167`). There is no repo
  handle, no import graph, no other file.
- `Finding` is `{ line: usize, message: String }` and nothing else (`src/checker.rs:54-57`). There is
  no structured field for signal name / measured value / baseline / tool version. The requirements'
  "Observability" contract (signal, raw value, baseline, tool+version per finding) has to be encoded in
  `message` text for a native checker, or carried by a different finding type for a batch report.
- Optional `configure(&options)` lets a checker take per-repo `options` from `.kibitzer/inspect.json`
  (`src/checker.rs:197-208`) — the hook for thresholds.
- Registration + defaults: `default_checks()` chains `core_checks`, `go_exception_checks`,
  `java_exception_checks`, `file_size_checks`, `syntax_rules_checks`, `comment_quality_checks`,
  `prose_checks` (`src/config.rs:759-769`). A new family means a new `*_checks()` fn chained here,
  following `CLAUDE.md`'s "Default check catalog" rule.
- Languages the tree-sitter layer parses: Go, TypeScript, Tsx, JavaScript, Python, Java, Kotlin, Rust
  (`src/checker.rs:70-101`, `Language::ALL`). All eight are available to a per-file checker.
- Closest precedent for a test-quality checker: `go-table-driven-test-candidate`
  (`src/checkers/go_table_driven_test.rs:33-60`). Scope glob `**/*_test.go`
  (`src/checkers/go_table_driven_test.rs:57-59`; registered at `src/config.rs:859-863`), identifies test
  functions by `GoKind::FunctionDeclaration` whose name has prefix `Test` (`:71`, `:142`), skips
  generated files via `file_size::is_generated` (`src/checkers/file_size.rs:23`), and compares
  literal-normalized bodies. This is already a "test-quality" static checker; the family exists in
  embryo.
- Precedent for test-code noise policy: `java-ignored-error` excludes `**/test/**` because 86% of raw
  Cassandra hits were in test code (`src/config.rs:867-875`). A test-quality checker inverts that
  scope; the same corpus can be reused to measure it.

### 1b. Batch reports (never hook-path)

- `ArchitectureAction` variants `ChangeCoupling`, `RootCauseClusters`, `Hotspots`, `Affected`
  (`src/main.rs:213-260`, dispatch at `src/main.rs:578-585`). Each is documented in-code as "Batch-only
  ... Never wired into `default_checks()`/hook mode" (`src/main.rs:210-212`, `:226-230`, `:243-247`).
- Shape: a pure `analyze(path, limit, top_n) -> Result<Vec<T>>` in its own module
  (`src/hotspots.rs:66`) plus a thin `run_*` printing handler in `main.rs` (`src/main.rs:805-817`);
  this is the "pure `analyze()` + thin `run_*`" pattern the affected-tests research recorded
  (`project_plans/affected-tests/research/architecture.md` §3, line 106). Output is human text, not
  `Finding`s; `hotspots` is Go-only (`src/main.rs:247`).
- `affected` is the exception whose stdout is machine-consumed (`go test $(kibitzer ... affected)`;
  `project_plans/affected-tests/research/architecture.md:128`), i.e. there is precedent for a batch
  subcommand being a CI contract.

### 1c. Plugins (external checker binaries)

- A plugin is registered as an ordinary `command` check: `command = "<binary> {file}"`, with
  `output_format` and `scope` from the registry entry (`src/plugin.rs:174-190`), chained after
  `default_checks()` at config load (`src/config.rs:997`). So a plugin is per-file, shell-invoked,
  SARIF-parsed (`docs/output-formats.md:7-12`, SARIF is the only format), and subject to a 30 s
  `COMMAND_TIMEOUT` (`src/check.rs:36`).
- Consequence: the plugin protocol has no notion of "whole-repo batch run" or "ingest this report";
  it takes one file path per invocation. A coverage/mutation tool does not fit it without a wrapper
  that ignores `{file}` (INFERRED).
- Plugins are machine-local, per-arch, not synced (`docs/plugins.md`, "Known limitations"), which
  weakens them as a shipping vehicle for a signal the owner wants on every machine and in CI.

### 1d. Post-passes that apply to any new finding source

- Inline `kibitzer:ignore` is applied to every checker's findings through a single post pass
  (`CLAUDE.md` "Default check catalog"; `project_plans/inline-ignore-syntax/research/architecture.md:25`,
  `:54`: one call in `run_checker_against_source`). It operates on `Finding` + source. Batch reports
  that bypass `run_checker_against_source` do not get it for free (INFERRED from `hotspots.rs` having
  no `Finding` use, `src/main.rs:805-817`).

## 2. Static identification of tests and assertions

What exists:

- Go test-function identification: `FunctionDeclaration` + `Test` prefix
  (`src/checkers/go_table_driven_test.rs:71,142`). No `Benchmark`/`Fuzz`/`Example`, no `t.Run`
  subtests, no assertion detection in the current checker (only whole-body normalization).
- Language-agnostic AST walk: `tree_walk` iterative preorder visitor with a "descend?" return
  (`src/tree_walk.rs:17-35`), plus typed per-language kind enums (`GoKind` used at
  `src/checkers/go_table_driven_test.rs:8,71`; `src/node_kind.rs`, documented in
  `docs/typed-node-kinds.md`).
- Symbol layer: `SymbolKind` is only `Type | Interface | Function | Method` (`src/arch_model.rs:28-33`).
  Test functions are indistinguishable from production functions in `PackageNode.symbols`
  (`src/arch_model.rs:94-98`); there is no `is_test` attribute (grep for `_test.go`/`is_test` in
  `src/arch_model.rs`, `src/import_graph.rs`, `src/affected.rs` returned no test-file classification).
- Non-Go test detection in non-test code: only `#[cfg(test)]` in kibitzer's own Rust sources and glob
  `**/test/**` in `config.rs`. There is no shared "is this a test file" helper (INFERRED gap: each
  checker hand-rolls a glob in `file_globs()`, the same duplication `Language::extensions` was created
  to remove at `src/checker.rs:90-100`).

What is derivable statically with the AST layer (INFERRED, to be validated by prototype):

| Signal | Needs | Languages realistic first |
|---|---|---|
| Test function with zero assertion calls (`assert*`, `require.*`, `expect(`, `t.Error/Fatal`, `assertThat`, `assert` stmt) | Per-file AST: enumerate test fns, scan descendant call/assert nodes | Go, Python, Java, TS/JS |
| Tautological assertion (`assert True`, `assertEquals(x, x)`, `expect(1).toBe(1)`) | Per-file AST, literal/identity compare | Same |
| Mock-only verification (only `verify(...)`/`.toHaveBeenCalled`) | Per-file AST + framework name list | Java/Kotlin, TS/JS |
| Test body identical modulo literals (already shipped for Go) | Per-file AST | Go (extend to others) |
| Test with no call into non-test code | Needs symbol resolution beyond one file (see §3) | Go first |

All of these are single-file and fit the existing `Checker` trait without architectural change.

## 3. Test → production linkage: can it be derived statically?

- `ArchModel` carries `import_edges` (package-granular `from`/`to`/`file`/`line`;
  `src/import_graph.rs:16-21`), `call_edges` (`from`/`to`/`resolved`/`file`/`line`;
  `src/arch_model.rs:167-173`), `field_accesses`, `type_edges`, and per-file import aliases
  (`src/arch_model.rs:246-269`). Call graph traversal is exposed via MCP `list_callers`/`list_callees`
  for **Go/TS/JS only** (kibitzer MCP server instructions; `go_call_resolution.rs` exists for Go).
- Test files are not segregated in the model: `_test.go` files are members of their package like any
  file (no test-aware code in `import_graph.rs`/`arch_model.rs`/`affected.rs` by grep), so a Go test's
  call edges to production symbols are present in `call_edges` keyed by `file`, and a test file can be
  recovered by `file` suffix (INFERRED; verify `resolved: true` rate on a corpus repo).
- `affected.rs` is package-level reverse reachability for Go only
  (`src/affected.rs:1-9`; `compute_affected` at `src/affected.rs:525`). It answers "which packages
  to test given a diff", not "which production symbols does test T reach". The edge data to answer the
  latter exists (`call_edges`) but no consumer builds a test→symbol map.
- Honest ceiling: static linkage over-approximates (reflection, DI, table-driven dispatch, interface
  calls with `resolved: false`) and is call-graph depth-1 unless a transitive walk is added. It can
  support "this test calls nothing in the package under test" (Go) and "production symbol X has no
  test that references it" (reverse of `unreferenced_symbols.rs`, which already finds unreferenced
  symbols), but cannot support "this test is redundant" — that needs execution data.
- The existing `unreferenced_symbols.rs` is the template: a whole-model pass producing
  per-symbol findings (listed in `src/main.rs` mod list, line 54). A "production symbol with no test
  reference" report is the same shape.

## 4. Where a dynamic / ingest-only signal fits

- Nothing in kibitzer executes tests or ingests coverage/mutation output today. `covgate.toml` is
  kibitzer's own CI diff-coverage gate against `cargo-llvm-cov` JSON (`covgate.toml:1-8`); it is
  configuration for an external gate tool, not a parser in `src/`.
- `command` checks can run an arbitrary process but are per-file with a 30 s timeout
  (`src/check.rs:36`), unsuitable for running a suite or mutation tool.
- `backtest.rs` replays transcript edits through *static* native checkers only
  (`docs/backtesting.md:1-10`); it cannot validate a coverage-based signal. A dynamic signal's
  validation path is the corpus experiment in `docs/backtest-repos.md`, triaged with
  `scripts/backtest-triage.py` (per `CLAUDE.md` "Writing a new check").
- Output precedent for ingesting a structured tool report is SARIF 2.1.0 (`docs/output-formats.md`).
  The ingest-only report would be a second input format class (lcov, Go cover profile, JaCoCo/PIT XML,
  Stryker JSON, `cargo-llvm-cov` JSON, cargo-mutants `outcomes.json`) rather than SARIF; it is the
  first time kibitzer consumes *coverage* as input, and per-test coverage is the rabbit hole the
  requirements flag (requirements.md "Rabbit Holes").

## 5. Precedent from prior plans to stay consistent with

1. **Batch-only reports are a first-class, documented convention** with a pure `analyze()` + thin
   `run_*` split and the explicit "never in `default_checks()`/hook mode" rule
   (`project_plans/affected-tests/research/architecture.md` §3; `src/main.rs:210-212`). Requirements
   NFR ("dynamic signals ... never run in the hook path") matches this exactly.
2. **"One model, many views"**: new consumers read `ArchModel` rather than building a second graph
   (`project_plans/affected-tests/research/architecture.md` §5, lines 188-205; also the
   repo auto-memory "architecture-export Feature"). Any static test→production linkage must read
   `ArchModel.call_edges`/`import_edges`, not re-parse.
3. **Extend by merge-point function, don't widen traits/`Check`**
   (`project_plans/checker-plugin-system/research/architecture.md`, "Prior-doc reconciliation" item 1).
   Argues against adding a `Finding` field or a new trait method for batch inputs; add a sibling
   module + CLI verb instead.
4. **New default checks need backtest + corpus + a confirmed-transcript bar**
   (`CLAUDE.md`; `docs/check-ideas.md:132-154` — items without confirmed transcript occurrence are
   explicitly flagged as unbacked). The static-assertion signals must find transcript evidence of
   agents writing assertion-free/tautological tests, or document a corpus-only justification
   (requirements.md Constraints).
5. **Default-on only when no repo-specific setup is needed** (`CLAUDE.md`); anything needing a coverage
   file path is opt-in by construction.

## 6. Candidate integration shapes

### Shape A — static per-file `test-quality-<lang>` checkers in `default_checks()`

- Mechanism: new `Checker`s in `src/checkers/` modeled on `go_table_driven_test.rs`, one per
  language × signal family (or one multi-signal checker per language with a rule table, like
  `syntax-rules-<lang>`/`comment-quality-<lang>`), a new `test_quality_checks()` chained at
  `src/config.rs:759-769`, scoped by test-file globs.
- Signals: no-assertion test, tautological assertion, mock-only verification, literal-only duplicate
  bodies (extend the existing Go one), empty/skip-only test.
- Hook-latency: same budget and path as existing per-file checkers (single parse, no I/O beyond the
  file); the NFR is satisfied by construction. Diff-awareness comes free from the existing changed-line
  scoping, so an agent that just wrote a vacuous test gets told on the edit.
- Observability: only `line` + `message` available (`src/checker.rs:54-57`); the measured value and
  baseline must be rendered into `message`. Tool/version is moot (no external tool).
- Weakness: validity vs. real fault detection is "weakly evidenced" (requirements.md Rabbit Holes);
  cannot say "redundant", only "vacuous". Maximum value-per-cost and the only shape that serves the
  agent-writes-a-test-now consumer.
- Cost class: small-medium (per-language rule tables + backtest on 7 corpus repos).

### Shape B — ingest-only batch report: `kibitzer architecture tests` (name TBD)

- Mechanism: new `src/test_effectiveness.rs` with pure `analyze(inputs, &ArchModel?) -> Vec<TestSignal>`
  and a thin `run_*` in `main.rs`, new `ArchitectureAction` variant alongside `Hotspots`/`Affected`
  (`src/main.rs:213-260`, dispatch `:578-585`). Inputs are files CI/the owner already produced:
  per-test coverage (lcov/Go profile-per-test/`cargo-llvm-cov`), mutation results (PIT XML, Stryker
  JSON, cargo-mutants outcomes). Output: ranked tests with `unique_lines_covered`,
  `mutants_uniquely_killed`, `baseline`, `tool@version` (satisfies the requirements' observability
  contract, which `Finding` cannot).
- Hook-latency: not applicable by design — never in the hook path; same stance as `hotspots`.
- Who runs the tests: **not kibitzer** — answers requirements Open Question 1 as "ingest" (INFERRED
  recommendation). Preserves "advisory, not a test runner".
- Optional static enrichment: join ingested test names to `ArchModel.call_edges` (§3) to explain
  "covers X but asserts nothing" and to scope to the diff (reuse `affected.rs` diff machinery,
  `src/affected.rs:257`).
- Weakness: per-test coverage is not a standard artifact of most tools; the owner/CI must produce it
  (per-test runs), and format parsers multiply per language. Strongest validated signals (mutation
  kill sets) live only here. Can output text/JSON for an LLM adjudicator to consume (hybrid in
  requirements Alternatives).
- Cost class: large (parsers × formats, corpus runs are the wall-clock blocker).

### Shape C — plugin wrapping external tools

- Mechanism: ship a `kibitzer-tests-*` plugin binary registered via `kibitzer plugin install`
  (`docs/plugins.md`), run as a `command: "<bin> {file}"` SARIF check (`src/plugin.rs:174-190`).
- Fit problems: per-file invocation vs. whole-suite data; 30 s timeout (`src/check.rs:36`); SARIF
  only (`docs/output-formats.md`); machine-local registry not synced (`docs/plugins.md`). Plausible
  only as a thin per-test-file wrapper over a *cached* report (INFERRED), which re-implements Shape B
  outside the repo with less observability.
- Cost class: small to build, high ongoing friction; recommended only as a distribution channel for
  language-specific tools after B's ingest format is defined, not as the primary design.

### Recommended layering (A + B, C deferred)

A for hook-time vacuity signals; B for ranked dynamic signals; B's output also exposable as MCP
`run_checks`-style prose for the LLM-adjudicator pass. They share only a naming/suppression
convention (`docs/suppressing-checks.md`), so they can land independently.

## 7. Disposition (SDD rule) for the touched area

| Area | Disposition | Reason |
|---|---|---|
| Per-file checker registry / `default_checks()` / `Checker` trait | **Extend as-is** | A new `*_checks()` chain + new `src/checkers/*.rs` is the established pattern (`src/config.rs:759-769`); no trait change needed. |
| Batch-report CLI (`ArchitectureAction`) | **Extend as-is** | Four existing siblings define the `analyze()`+`run_*` pattern (`src/main.rs:213-260`). `main.rs` is 1182 lines, but adding one variant + one `run_*` is within the pattern. |
| Test-file / test-function identification | **Isolate via seam** | No shared helper exists (§2); every checker hand-rolls globs and `Test` prefix logic. Introduce one small `test_files` seam (language → test glob + test-fn predicate) that new checkers use and the existing `go-table-driven-test-candidate` can migrate to later, rather than refactoring existing checkers first. |
| `ArchModel` test→symbol linkage (only if Shape B enrichment is built) | **Extend as-is** | `call_edges`/`import_edges` already contain test-file edges (§3); add a consumer, not a model change (precedent: affected-tests §5). Add an `is_test` view derived from `file`, not a new stored field, to avoid cache-schema churn (`ModelCache`). |
| `Finding` (line + message only) | **Isolate via seam** | Batch report defines its own output struct with signal/value/baseline/tool fields; do not widen `Finding` (checker-plugin-system precedent, §5 item 3). Revisit only if A needs structured data for `report_false_positive`. |
| Plugin protocol | **Leave alone** | Not a fit for whole-suite input (§1c); not on the critical path. |

No refactor-first is warranted for any touched area: nothing read blocks the integration, and the one
real gap (test-file detection duplication) is cheaper to bridge with a seam than to fix across existing
checkers first.

## 8. Open items surfaced for the planning phase

1. Which static signals have a confirmed transcript occurrence? Needs a `kibitzer check backtest` pass
   with a prototype checker (the `docs/check-ideas.md:132-154` bar).
2. `resolved: true` rate of `call_edges` from `_test.go` files on kubernetes/ripgrep-class repos —
   decides whether the static test→production linkage is usable at all.
3. Per-test coverage acquisition strategy per language (who runs, how many runs) — blocks Shape B's
   input spec; research-dimension handoff, not decidable from kibitzer code.
4. Whether findings from Shape B need `kibitzer:ignore` / accepted-findings support
   (`src/accepted_findings.rs`, `docs/accepting-findings.md`) — they key on file+line, which per-test
   results have, but the post-pass is wired at `run_checker_against_source`, which batch reports skip.
