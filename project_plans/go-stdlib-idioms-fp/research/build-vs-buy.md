# Build vs. Buy: go-stdlib-idioms-fp

Two false positives to suppress:

1. **`primitive-obsession`**: fires on `sort.Slice(s, func(i, j int) bool { ... })` and
   on same-typed multi-parameter closures in `_test.go` files.
2. **`replace-magic-literal`**: fires on slog key strings at odd argument positions
   (e.g. `log.Warn(msg, "file", name, "err", err)` → flags `"file"` and `"err"`).

---

## Option 1 — Build: inline context-check logic in the existing checkers

Add tree-sitter parent-walk guards directly in `primitive_obsession.rs` and
`walk_literals` in `rules.rs`.

For `primitive-obsession`:

- In `walk()`, before calling `check_parameter_list`, check whether the enclosing
  `func_literal` is passed as an argument to a `call_expression` whose `function` field
  text is in a known stdlib allowlist (`sort.Slice`, `sort.SliceStable`, `slices.SortFunc`,
  etc.).  Three parent hops: `parameter_list → func_literal → argument_list →
  call_expression`. All are structural tree-sitter node relationships — no text scanning
  required outside the final function-name match.
- For test-file closures, the check is simpler: if `ctx.file` ends in `_test.go`, skip
  all `func_literal` parameter lists (consistent with the repo's existing blank-identifier
  suppression rationale — internal fixtures, not public APIs).

For `replace-magic-literal`:

- Add a `slog_key_node_ids: HashSet<usize>` field to `LiteralCollector`.
- In `walk_literals`, after inserting a string literal into `occurrences`, check whether
  its parent is an `argument_list`, whether the `argument_list`'s parent is a
  `call_expression` matching a slog-style function name suffix (`Info`, `Warn`, `Error`,
  `Debug`, `Log`, `*Context`, `With`), and whether the argument is at an odd index ≥ 1
  (counting over `named_children()` to skip comma tokens).  Tag matching nodes in
  `slog_key_node_ids`.
- In `emit_literal_findings`, filter out occurrences whose node ID is in `slog_key_node_ids`.

**Pros**

- Zero new abstractions — the suppression logic sits next to the code that would
  otherwise misfire.  A future reader sees the guard and the check in the same file.
- Consistent with the five existing context-sensitive suppression precedents in the
  codebase: `is_named_return_list`, blank-identifier exclusion, `is_generated`,
  `resolve_excluded_constants`, and the `deep-nesting` `else-if` flattening guard.
- The stdlib-callback allowlist is exhaustive and explicit — no false suppressions on
  user-defined functions with the same name structure.
- The slog guard uses a method-name-suffix allowlist (not a package-prefix guard) — this
  correctly suppresses `log.Warn`, `slog.Warn`, and `logger.Warn` in one list and is
  consistent with the `MAGIC_LITERAL_ALLOWLIST` allowlist pattern already in `rules.rs`.
- No new dependencies.  No external process, CLI, or data file.

**Cons**

- The stdlib allowlist for `primitive-obsession` needs manual maintenance when new
  `slices.*` or `sort.*` callback-taking functions are added.  The list is short (~10
  entries) and stable across Go minor versions, so churn is low.
- The slog guard introduces a second tree traversal concern inside `walk_literals`.
  Keeping it clean requires either (a) a single integrated check path or (b) extracting
  a small `collect_slog_key_ids` helper first — matching the "two-pass collect-then-emit"
  pattern already used in `rules.rs:1259`.

**Verdict: recommended.** Fits the existing codebase idiom at every level — same pattern,
same mechanism, same test discipline.  No design debt introduced.

---

## Option 2 — Use `kibitzer:ignore` directive (document the workaround)

Document that users should add `// kibitzer:ignore primitive-obsession` above `sort.Slice`
call sites and `// kibitzer:ignore replace-magic-literal` above each `log.Warn` call.

**Pros**

- Zero implementation cost in this cycle.
- Works today — the directive is already wired through all entry points (hook, MCP, run).

**Cons**

- This is a **workaround, not a fix**.  Every Go project that uses `sort.Slice` or
  structured logging must add suppressions manually — including future projects kibitzer
  is run against.
- The directives spread across user codebases, not into kibitzer's own source.  A
  kibitzer update cannot retroactively clean them up.
- The `inline-ignore` checker warns on directives that suppress nothing.  If the root
  fix lands later, all the per-call-site directives become `[unused-ignore]` noise.
- `kibitzer:ignore` requires a reason (`-- <why>`) per `docs/suppressing-checks.md`.
  Asking users to write `-- sort.Slice callback signature is dictated by stdlib` on
  every call site fails the "advisory not noise" bar.
- The directive syntax targets a single finding's line, not a class of patterns.
  For a codebase with 50 `sort.Slice` calls, this is 50 suppressions.

**Verdict: not the right solution.** `kibitzer:ignore` is designed for cases where a
finding is real but locally acceptable. These findings are structurally unreachable
true positives — they will fire on every well-written Go file that uses stdlib sort.
The right answer is fixing the checker.

---

## Option 3 — Reuse or adapt patterns from the existing Go linter ecosystem

### staticcheck (honnef.co/go/tools)

staticcheck's `SA` and `S` checks do not include a `sort.Slice` callback exemption for
any primitive-type or parameter-naming check — staticcheck does not have a `primitive-
obsession` equivalent at all.  Its `QF1001`–`QF1012` quick-fixes and `S1000`–`S1040`
style checks do not cover parameter-type diversity.

### golangci-lint / revive

`revive` has a `flag-parameter` linter, but no `primitive-obsession` equivalent.
`golangci-lint`'s `gocritic` includes a `paramTypeCombine` check that fires on the
`(a, b string)` shape — it has a hardcoded exemption for function literals passed as
arguments that are goroutine bodies, but no general stdlib-callback allowlist.

None of the major Go linters maintain a published allowlist of "stdlib functions whose
callback signatures must use same-typed parameters."  There is no community consensus
list to copy.

### slog key suppression

`golangci-lint`'s `sloglint` linter (added in 2023) does enforce slog argument structure
— but in the opposite direction: it flags *incorrect* `slog.Info(key, value)` ordering,
not repeated key strings.  It does not flag `replace-magic-literal` false positives
because it does not implement that rule.

### Verdict

No existing Go linter exports an allowlist or suppression pattern that kibitzer can
directly import or copy.  The community has not codified "stdlib callback signatures are
exempt from parameter-type diversity checks" as a named, versioned artifact.  We build
from the same first principles every linter author has used, but informed by the
research above: a short, explicit, maintainable allowlist in the checker source.

---

## Option 4 — Generic "callback signature allowlist" mechanism vs. hard-coding

### Hard-code the specific stdlib functions

Embed a `const STDLIB_CALLBACK_ALLOWLIST: &[&str]` in `primitive_obsession.rs` naming
the ~10 `sort.*` / `slices.*` functions.

**Pros**: zero mechanism overhead; dead simple; the list does not change between Go
1.21 and any foreseeable 1.x minor.

**Cons**: a new stdlib function (e.g. `maps.Collect` in a future Go version) requires a
source change.

### Generic "callback signature allowlist" mechanism

Add a `.kibitzer/inspect.json` key (e.g. `"primitive_obsession_callback_allowlist"`) that
projects can extend, or expose a config-level `allowlist` field on the check.

**Pros**: projects with in-house callback-taking functions could suppress those too.

**Cons**:
- No evidence of demand beyond stdlib cases — the requirements doc cites only `sort.*`
  and `slices.*` from a single real filing.
- Config-level keys add surface area to `config.rs`/schema: new fields, new docs, new
  migration considerations.
- A generic mechanism does not eliminate the need for the stdlib list to ship in the
  defaults; it only adds a user-extension point.  The defaults still need to be right.
- kibitzer's existing pattern for "known-safe context" decisions is an in-source
  `const` list (see `MAGIC_LITERAL_ALLOWLIST`, `PRIMITIVE_TYPES`), not a config file.

### Verdict: hard-code for now, make it easy to extend later

Hard-code the stdlib allowlist in `primitive_obsession.rs` as a named `const`.  Name it
`STDLIB_CALLBACK_ALLOWLIST` so a future PR that adds config-level extension has a clear
attachment point.  Do not add the config mechanism until there is a second real demand
signal — following the repo's existing precedent of promoting a hard-coded threshold to
config only after a backtest confirms the need (see `MAGIC_LITERAL_MIN_OCCURRENCES`'s
ADR-001 note).

For slog, the method-name-suffix list (`SLOG_METHOD_NAMES`) is similarly hard-coded.
It covers `slog.*`, `log.*`, and any `logger.*` wrapper — the full practical population.

---

## Summary

| Option | Verdict |
|--------|---------|
| **1. Build: inline context-check** | **Recommended** — consistent with all existing FP suppression patterns; no new abstractions |
| **2. `kibitzer:ignore` directive** | Workaround only — wrong tool for a class of structural false positives |
| **3. Reuse Go linter ecosystem patterns** | No directly reusable artifact exists; patterns confirm the approach but nothing to import |
| **4. Generic allowlist mechanism vs. hard-code** | Hard-code the stdlib list now; name it for future extension; no config mechanism yet |
