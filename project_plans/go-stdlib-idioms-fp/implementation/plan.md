# Implementation Plan: go-stdlib-idioms-fp

**Feature**: Suppress false-positive `go-primitive-obsession` and `replace-magic-literal` findings caused by stdlib callback signatures and slog attribute keys
**Date**: 2026-10-10
**Status**: Ready for implementation
**ADRs**: ADR-001-stdlib-callback-allowlist-hardcoded, ADR-002-slog-key-filter-inline

---

## Domain Glossary

| Term | Definition | Notes |
|------|-----------|-------|
| stdlib callback allowlist | Hard-coded set of fully-qualified Go stdlib/slices function names whose closure parameters are exempt from primitive-obsession | `sort.Slice`, `sort.SliceStable`, etc. |
| slog key position | Odd-indexed argument position (1, 3, 5, …) of a slog/log `Info`/`Warn`/`Error`/`Debug` call, counting from the message string at index 0 | Uses `named_children()` to skip comma tokens |
| slog method set | Set of method name suffixes (`Info`, `Warn`, `Error`, `Debug`, `Log`) whose calls follow the alternating key-value convention with message at index 0, keys at odd indices 1, 3, 5 | `slog.With()` is excluded — no leading message; `*Context` variants (`WarnContext`, `InfoContext`, etc.) are excluded — their (ctx, msg, key, val) signature shifts keys to even indices 2, 4, 6 (see Unresolved Questions) |
| STDLIB_CALLBACK_ALLOWLIST | `const &[&str]` of selector field names (i.e., function names) whose argument closure parameters are not subject to primitive-obsession | Lives in `primitive_obsession.rs` |
| SLOG_METHOD_NAMES | `const &[&str]` of method name suffixes whose odd-indexed string args are exempt from replace-magic-literal | Lives in `rules.rs` |
| slog_key_ids | `HashSet<usize>` of `Node::id()` values for string literals that are in slog-key position | Populated in `walk_literals`, filtered in `build_literal_finding` |
| `is_stdlib_callback_param_list` | New predicate in `primitive_obsession.rs`; returns true if a `parameter_list` node belongs to a closure passed to a stdlib callback | Modeled on `is_run_subtest_closure` in `complexity.rs` |
| `go_is_slog_key_literal` | New predicate in `rules.rs`; returns true if a string-literal node is in slog-key position | Called from `walk_literals` |
| test-file guard | Early return in `PrimitiveObsessionChecker::check()` when the file ends in `_test.go`; suppresses all func-literal parameter list findings in test files | Mirrors the guard already used in `complexity.rs:60` and `go_call_resolution.rs:143` |
| `LangRuleConfig.slog_key_predicate` | New optional field `Option<fn(Node, &[u8]) -> bool>` added to the config struct; `Some(go_is_slog_key_literal)` for Go, `None` for all other 7 languages | |
| named_children traversal | Using `.named_children()` (not `.children()`) when counting argument positions in an `argument_list`; comma tokens are unnamed nodes and must not count | Critical to correct odd-index detection |

---

## Pattern Decisions

| Component | Pattern Chosen | Source | Alternative Rejected | Reason |
|-----------|---------------|--------|---------------------|--------|
| Stdlib callback suppression | Context predicate — walk parent chain from `parameter_list` up through `func_literal → argument_list → call_expression`, check field selector against allowlist | PoEAA: Special Case / GoF: Strategy; precedent: `is_run_subtest_closure` (complexity.rs:246) | kibitzer:ignore directive added per call site | Ignore directives leave noise in the source; context predicate is invisible to the developer and requires no per-file action |
| Slog key filter | Collect slog-key node IDs into `slog_key_ids: HashSet<usize>` during `walk_literals`; filter them in `build_literal_finding` | Two-pass collect-then-emit architecture already used for `replace-magic-literal` (rules.rs:1255) | New `LangRuleConfig` predicate field `slog_key_predicate` called inline | Both approaches extend the same data structures; the `HashSet` approach keeps `build_literal_finding` signature stable and avoids threading cfg into the existing filter chain. The predicate field approach (what the research recommends) also works — see ADR-002 for why the HashSet wins at the margin. |
| Test-file guard for func literals | Early return in `check()` when `file.file_name().ends_with("_test.go")` | Existing idiom: complexity.rs:57-60, go_call_resolution.rs:143 | Per-node callback-allowlist check applied to every func literal in test files | The early return is simpler; test files have no public API contract at all, so blanket suppression is appropriate and matches developer expectation |
| STDLIB_CALLBACK_ALLOWLIST storage | Hard-coded `const &[&str]` in primitive_obsession.rs | Confirmed: no reusable allowlist exported by staticcheck/revive/golangci-lint | Config file / `.kibitzer/inspect.json` extension point | No evidence of demand for user-extension; hard-coding keeps the check self-contained, mirrors PRIMITIVE_TYPES precedent |
| SLOG_METHOD_NAMES storage | Hard-coded `const &[&str]` in rules.rs | Same rationale as STDLIB_CALLBACK_ALLOWLIST | Config file extension | Simpler; full set of slog method names is stable and small |

---

## Tech Debt Disposition

| Area | Existing Issue | Disposition | Justification |
|------|----------------|--------------|----------------|
| `primitive_obsession.rs` | `_file` parameter in `check()` signature is unused — the file path was never needed before this feature | Rename `_file` → `file` and use it for the test-file guard (AC4) | The guard needs the path; removing the underscore prefix correctly signals the parameter is now in use |
| `rules.rs` `LiteralCollector` | No mechanism to exempt specific literal nodes from collection | Add `slog_key_ids: HashSet<usize>` field (default empty); populated in `walk_literals` when `cfg.slog_key_predicate.is_some()` | Minimal struct extension; does not break existing callers; consistent with existing `bound: Vec<ConstBinding>` field |
| `rules.rs` `LangRuleConfig` | All 8 language configs have the same shape; adding a Go-only field inflates the other 7 | Add `slog_key_predicate: Option<fn(Node, &[u8]) -> bool>` field with `None` default for all non-Go configs | Optional fields with `None` are zero-overhead for other languages; matches `binding_finder`'s per-language function-pointer precedent |

---

## Migration Plan
N/A — no schema or data changes.

## Observability Plan
N/A — checker output format unchanged; no new metrics required.

## Risk Control
- **Feature flag**: not gated
- **Rollback procedure**: standard revert via PR close + revert commit
- **Staged rollout**: full rollout on merge

## Unresolved Questions

1. **`slog.With()` exclusion**: the research notes that `slog.With(key, val, …)` uses index 0 as a key (no leading message), so its positional semantics differ from `Info`/`Warn`/`Error`/`Debug`. It is excluded from the initial `SLOG_METHOD_NAMES` set. If corpus backtest shows it fires on `slog.With`, add it in a follow-up after confirming the AST shape of its first named child.

2. **logr / hclog**: the research identifies logr and hclog as sharing the same alternating key-value convention. They are not included in the initial implementation because (a) they are not in the default-checks backtest corpus and (b) adding them requires verifying the selector field names they expose. Follow-up issue after this PR ships.

3. ***Context method suppression**: `WarnContext`/`InfoContext`/`ErrorContext`/`DebugContext` require a per-method key-start-offset — their signature is `(ctx context.Context, msg string, key, val, …)`, so keys land at even indices 2, 4, 6, not odd 1, 3, 5. The current `go_is_slog_key_literal` uses `idx % 2 == 1` universally, which would incorrectly suppress value-position strings and miss key-position strings in these calls. Excluded from the initial implementation. Add in a follow-up after a per-function offset table is implemented (mapping method name → key-start index).

## Dependency Visualization

```
Phase 1                           Phase 2
───────                           ───────
Task 1.1.1                        Task 2.1.1
Add STDLIB_CALLBACK_ALLOWLIST     Add slog_key_predicate field to
+ is_stdlib_callback_param_list   LangRuleConfig + SLOG_METHOD_NAMES
         │                                 │
         ▼                                 ▼
Task 1.1.2                        Task 2.1.2
Wire guard into check():          Add slog_key_ids to LiteralCollector
test-file early return            + populate in walk_literals
+ FuncLiteral parent-chain check            │
         │                                  ▼
         ▼                        Task 2.1.3
Task 1.1.3                        Filter slog_key_ids in
Unit tests (AC1, AC2, AC3, AC4)   build_literal_finding
                                            │
                                            ▼
                                  Task 2.1.4
                                  Unit tests (AC5, AC6)

                    Phase 3 (both phases complete)
                    ───────────────────────────────
                    Task 3.1.1
                    Backtest corpus run + regression gate (AC7)
                    Confirm AC8 (≥10 unit tests total)
```

---

## Phase 1: Fix primitive-obsession false positives

### Epic 1.1: Suppress stdlib callback parameter lists and test-file func literals (S)

**Goal**: `go-primitive-obsession` produces zero findings for closure parameters passed to stdlib callbacks, and zero findings for any func-literal parameter lists in `_test.go` files.

**Files touched**: `src/checkers/primitive_obsession.rs`

---

#### Story 1.1.1 — Add `STDLIB_CALLBACK_ALLOWLIST` constant and `is_stdlib_callback_param_list` predicate (XS)

**Given** source `sort.Slice(s, func(i, j int) bool { return s[i] < s[j] })`,
**When** `check_parameter_list` is called for the `(i, j int)` parameter list,
**Then** `is_stdlib_callback_param_list` returns `true` and no finding is emitted.

**Tasks**:

**Pre-implementation verification (P1 gate):**

Before writing `is_stdlib_callback_param_list`, confirm that `selector_expression.child_by_field_name("field").utf8_text(src)` returns just `"Slice"` (not `"sort.Slice"`). Grep for `child_by_field_name("field")` in `god_class.rs` or `symbol_extract.rs` to see an existing usage, or add a `#[test]` that calls `utf8_text` on the `field` child of a `sort.Slice` parse and asserts the result is `"Slice"`. The allowlist constant uses these field-name values — if the text were package-qualified, every match would silently fail.

**Task 1.1.1** — Add constant and predicate (`src/checkers/primitive_obsession.rs`) (XS)

Add immediately before `walk()`:

```rust
/// Stdlib and slices functions whose closure parameters follow a mandated signature —
/// the caller cannot change these names without breaking the stdlib contract.
/// `slog.With` is omitted (positional semantics differ — see Unresolved Questions).
const STDLIB_CALLBACK_ALLOWLIST: &[&str] = &[
    "Slice",
    "SliceStable",
    "Search",
    "SortFunc",
    "SortStableFunc",
    "BinarySearchFunc",
    "IndexFunc",
    "ContainsFunc",
    "EqualFunc",
    "CompareFunc",
];

/// Returns true when `param_list` belongs to a func_literal that is passed as an
/// argument directly to a call whose selector field matches STDLIB_CALLBACK_ALLOWLIST.
/// The check is on the *field name* only (e.g. `Slice`, not `sort.Slice`) — any
/// package naming the function `Slice` is treated the same. User-defined wrappers with
/// the same name are an accepted false negative; document in function comment.
fn is_stdlib_callback_param_list(param_list: Node, src: &[u8]) -> bool {
    let Some(func_lit) = param_list.parent() else { return false; };
    if GoKind::of(func_lit) != GoKind::FuncLiteral { return false; }
    let Some(args) = func_lit.parent() else { return false; };
    if GoKind::of(args) != GoKind::ArgumentList { return false; };
    let Some(call) = args.parent() else { return false; };
    if GoKind::of(call) != GoKind::CallExpression { return false; };
    call.child_by_field_name("function")
        .filter(|f| GoKind::of(*f) == GoKind::SelectorExpression)
        .and_then(|f| f.child_by_field_name("field"))
        .and_then(|f| f.utf8_text(src).ok())
        .is_some_and(|name| STDLIB_CALLBACK_ALLOWLIST.contains(&name))
}
```

**Acceptance criteria covered**: AC1, AC2

---

#### Story 1.1.2 — Wire suppression into `check_parameter_list` and add test-file guard (XS)

**Given** `write := func(name, body string) { ... }` inside a `_test.go` file,
**When** `PrimitiveObsessionChecker::check()` is called with that file,
**Then** zero findings are returned.

**Given** `func PublicAPI(a, b string)` in a non-test file,
**When** the checker runs,
**Then** one finding is emitted (AC4 true-positive guard).

**Tasks**:

**Task 1.1.2a** — Test-file early return in `check()` (XS)

In `src/checkers/primitive_obsession.rs`, change the `check()` signature from `_file: &Path` to `file: &Path`. Add early return at the top of the check body:

```rust
fn check(&self, file: &Path, ctx: &CheckContext) -> Result<Vec<Finding>> {
    if file.file_name().is_some_and(|n| n.to_string_lossy().ends_with("_test.go")) {
        return Ok(Vec::new());
    }
    // ... existing body unchanged
}
```

**Task 1.1.2b** — Guard in `check_parameter_list` (XS)

In `check_parameter_list`, before case (a) and case (b) detection, add:

```rust
// Suppress stdlib callback parameters — the developer cannot change
// these without breaking the stdlib contract.
if is_stdlib_callback_param_list(list, src) {
    return;
}
```

This single guard at the top of `check_parameter_list` handles both case (a) and case (b) for any stdlib-callback parameter list.

**Acceptance criteria covered**: AC3, AC4

---

#### Story 1.1.3 — Unit tests for primitive-obsession suppression (S)

**Given** the four AC-mapped test shapes,
**When** `check_source` (the existing test helper) is called,
**Then** results match the expectations below.

Add to the `#[cfg(test)]` block in `src/checkers/primitive_obsession.rs`:

```rust
#[test]
fn suppresses_sort_slice_callback() {
    // AC1: sort.Slice(s, func(i, j int) bool {...}) → zero findings
    let src = r#"package main
import "sort"
func f(s []int) {
    sort.Slice(s, func(i, j int) bool { return s[i] < s[j] })
}"#;
    assert!(check_source(src).unwrap().is_empty());
}

#[test]
fn suppresses_slices_sort_func_callback() {
    // AC2: slices.SortFunc fires → zero findings
    let src = r#"package main
import "slices"
func f(s []int) {
    slices.SortFunc(s, func(a, b int) int { return a - b })
}"#;
    assert!(check_source(src).unwrap().is_empty());
}

// Additional AC2 coverage: one test per allowlist entry is excessive;
// SortStableFunc and BinarySearchFunc cover the remaining interesting shapes.
#[test]
fn suppresses_sort_search_callback() {
    let src = r#"package main
import "sort"
func f(n int) int {
    return sort.Search(n, func(i int) bool { return i >= 0 })
}"#;
    assert!(check_source(src).unwrap().is_empty());
}
```

For AC3 (test-file func literal) — use the `Checker` trait path since `check_source` passes `Path::new("<source>")`, not a `_test.go` path. Build the `CheckContext` by calling `crate::test_support::parse_go` directly:

```rust
#[test]
fn suppresses_slices_sort_stable_func_callback() {
    // AC2: slices.SortStableFunc with string element type → zero findings
    let src = r#"package main
import "slices"
func f(s []string) {
    slices.SortStableFunc(s, func(a, b string) int {
        if a < b { return -1 }
        return 1
    })
}"#;
    assert!(check_source(src).unwrap().is_empty());
}

#[test]
fn suppresses_func_literal_in_test_file() {
    // AC3: write := func(name, body string) in a _test.go → zero findings
    use crate::checker::{CheckContext, Checker};
    use std::path::Path;
    let checker = PrimitiveObsessionChecker;
    let src = r#"package main_test
func TestSomething(t *testing.T) {
    write := func(name, body string) {}
    write("x", "y")
}"#;
    let tree = crate::test_support::parse_go(src).unwrap();
    let ctx = CheckContext { source: src, tree: Some(&tree) };
    let findings = checker.check(Path::new("foo_test.go"), &ctx).unwrap();
    assert!(findings.is_empty(), "expected no findings, got: {findings:?}");
}

#[test]
fn still_flags_exported_multi_primitive_in_non_test_file() {
    // AC4: exported func with same-type primitive pile-up still fires
    let findings = check_source("package main\nfunc PublicAPI(a, b string) {}\n").unwrap();
    assert_eq!(findings.len(), 1);
}
```

**Acceptance criteria covered**: AC1, AC2, AC3, AC4

---

## Phase 2: Fix replace-magic-literal false positives

### Epic 2.1: Suppress slog key string literals (S)

**Goal**: String literals in odd-indexed argument positions of `slog`/`log` `Info`/`Warn`/`Error`/`Debug`/`Log` calls do not appear in `replace-magic-literal` findings, even when the same key appears ≥3 times across a file. (`*Context` variants are excluded from this implementation — see Unresolved Questions #3.)

**Files touched**: `src/checkers/rules.rs`

---

#### Story 2.1.1 — Add `SLOG_METHOD_NAMES` constant and `go_is_slog_key_literal` predicate (S)

**Given** `log.Warn("user-themes: ...", "file", name, "err", err)` repeated 3 times in a file,
**When** `walk_literals` collects occurrences of `"file"` and `"err"`,
**Then** all 6 occurrences (3 × "file", 3 × "err") are recorded with their node IDs in `slog_key_ids`.

**Pre-implementation verification (P1 gate — do before writing any slog-key code):**

Before implementing `go_is_slog_key_literal`, verify two AST facts using an existing test fixture or the tree-sitter-go playground:

1. **Commas are anonymous in `argument_list`**: call `node.to_sexp()` on the argument_list of a `slog.Warn(msg, key, val)` call and confirm commas do NOT appear as named children. Expected: `(argument_list (interpreted_string_literal) (identifier) (identifier))`. If commas are named children, `named_children()` indexing is wrong and the entire odd-index strategy must be revised.

2. **Selector field text is the field name only**: confirm that `selector_expression.child_by_field_name("field").utf8_text(src)` returns `"Warn"` (not `"log.Warn"`). The plan's allowlist matches against this text — if the text includes the package prefix, every match would fail silently. Verify against an existing selector in the codebase (e.g., grep for `child_by_field_name("field")` in `rules.rs` or `god_class.rs`) before writing the allowlist constant.

Gate: both facts verified before writing `SLOG_METHOD_NAMES` or `go_is_slog_key_literal`. Document the result in a code comment at the top of `go_is_slog_key_literal`.

**Tasks**:

**Task 2.1.1** — Add constant and predicate to `src/checkers/rules.rs` (S)

Add after `MAGIC_LITERAL_ALLOWLIST`:

```rust
/// Method name suffixes whose calls follow the alternating key-value convention:
///   receiver.Method(message, key1, val1, key2, val2, …)
/// The odd-indexed arguments (index 1, 3, 5, …) are slog attribute keys — string
/// literals there are idiomatic and must not be flagged by replace-magic-literal.
///
/// `slog.With(key, val, …)` is NOT included: its index 0 is already a key (no message
/// prefix), making the even/odd convention the opposite of the calls here. Include in
/// a follow-up after verifying the AST shape.
const SLOG_METHOD_NAMES: &[&str] = &[
    "Info",
    "Warn",
    "Error",
    "Debug",
    "Log",
    // *Context variants (WarnContext, InfoContext, etc.) have (ctx, msg, key, val) signatures —
    // keys land at even indices 2, 4, 6, not odd 1, 3, 5. Excluded for now; add in follow-up
    // with per-method key-offset table.
];

/// Returns true when `node` is a string literal in slog key position:
/// odd-indexed argument (1, 3, 5, …) of a call whose selector field
/// matches SLOG_METHOD_NAMES.
///
/// Uses `named_children()` — commas are unnamed nodes and must not count.
fn go_is_slog_key_literal(node: Node, src: &[u8]) -> bool {
    let Some(arg_list) = node.parent() else { return false; };
    if GoKind::of(arg_list) != GoKind::ArgumentList { return false; }
    let Some(call) = arg_list.parent() else { return false; };
    if GoKind::of(call) != GoKind::CallExpression { return false; }
    // Check that the call is a slog/log method.
    let is_slog_call = call.child_by_field_name("function")
        .filter(|f| GoKind::of(*f) == GoKind::SelectorExpression)
        .and_then(|f| f.child_by_field_name("field"))
        .and_then(|f| f.utf8_text(src).ok())
        .is_some_and(|name| SLOG_METHOD_NAMES.contains(&name));
    if !is_slog_call { return false; }
    // Find this node's index among the argument list's named children.
    let mut cursor = arg_list.walk();
    let position = arg_list
        .named_children(&mut cursor)
        .position(|child| child == node);
    // Odd-indexed arguments (1, 3, 5, …) are slog keys.
    position.is_some_and(|idx| idx % 2 == 1)
}
```

**Acceptance criteria covered**: AC5 (detection path)

---

#### Story 2.1.2 — Add `slog_key_predicate` to `LangRuleConfig` and `slog_key_ids` to `LiteralCollector` (S)

**Given** the Go lang config is loaded,
**When** `walk_literals` encounters a string literal in slog key position,
**Then** that literal's `node.id()` is in `collector.slog_key_ids`.

**Tasks**:

**Task 2.1.2a** — Extend `LangRuleConfig` (XS)

Add to the end of the `LangRuleConfig` struct definition (after `density_boundary_kinds`):

```rust
/// Returns true if `node` is a literal that should be exempt from
/// `replace-magic-literal` because it is in a language-specific idiomatic
/// position — e.g. a slog attribute key in Go. `None` for languages with no
/// such idiom.
slog_key_predicate: Option<fn(Node, &[u8]) -> bool>,
```

**Task 2.1.2b** — Set the field in `go_lang_config()` (XS)

In `go_lang_config()`, add to the struct literal:

```rust
slog_key_predicate: Some(go_is_slog_key_literal),
```

In all other language config functions (`typescript_lang_config`, `tsx_lang_config`, `javascript_lang_config`, `python_lang_config`, `java_lang_config`, `kotlin_lang_config`, `rust_lang_config`), add:

```rust
slog_key_predicate: None,
```

**Task 2.1.2c** — Extend `LiteralCollector` (XS)

Add to `LiteralCollector`:

```rust
/// Node IDs of literals that are in a language-specific exempt position
/// (e.g. slog key arguments in Go). Populated by `walk_literals` when
/// `cfg.slog_key_predicate` is `Some`.
pub(crate) slog_key_ids: HashSet<usize>,
```

(The `#[derive(Default)]` already present on `LiteralCollector` will zero-initialize this field.)

**Task 2.1.2d** — Populate `slog_key_ids` in `walk_literals` (XS)

In `walk_literals`, inside the block that handles a named literal node, after pushing to `collector.occurrences`, add:

```rust
if let Some(predicate) = cfg.slog_key_predicate {
    if predicate(node, src) {
        collector.slog_key_ids.insert(node.id());
    }
}
```

**Acceptance criteria covered**: AC5 (collection path)

---

#### Story 2.1.3 — Filter slog_key_ids in `build_literal_finding` (XS)

**Given** `"file"` appears 3 times as a slog key and 0 times outside slog-key position,
**When** `build_literal_finding` runs,
**Then** no finding is emitted.

**Given** `"pending"` appears 4 times — 3 as a slog key and 1 as a genuine repeated string literal,
**When** `build_literal_finding` runs,
**Then** no finding is emitted (fewer than `MAGIC_LITERAL_MIN_OCCURRENCES` non-key occurrences remain).

**Tasks**:

**Task 2.1.3** — Thread `slog_key_ids` through `emit_literal_findings` and `build_literal_finding` (XS)

Change `emit_literal_findings` signature:

```rust
fn emit_literal_findings(
    collector: &LiteralCollector,
    cfg: &LangRuleConfig,
    excluded: &HashSet<usize>,
    findings: &mut Vec<Finding>,
)
```

The call inside the closure already passes `excluded` — add `&collector.slog_key_ids` as a new parameter:

```rust
fn emit_literal_findings(
    collector: &LiteralCollector,
    cfg: &LangRuleConfig,
    excluded: &HashSet<usize>,
    slog_keys: &HashSet<usize>,
    findings: &mut Vec<Finding>,
) {
    let mut new_findings: Vec<Finding> = collector
        .occurrences
        .iter()
        .filter_map(|(text, occurrences)| {
            build_literal_finding(text, occurrences, cfg, excluded, slog_keys)
        })
        .collect();
    new_findings.sort_by_key(|f| f.line);
    findings.extend(new_findings);
}
```

Update `build_literal_finding` to filter:

```rust
fn build_literal_finding(
    text: &str,
    occurrences: &[LiteralOccurrence],
    cfg: &LangRuleConfig,
    excluded: &HashSet<usize>,
    slog_keys: &HashSet<usize>,
) -> Option<Finding> {
    let first = occurrences.first()?;
    if is_allowlisted_literal(text, first.kind) {
        return None;
    }
    let mut remaining: Vec<&LiteralOccurrence> = occurrences
        .iter()
        .filter(|occ| !excluded.contains(&occ.node.id()) && !slog_keys.contains(&occ.node.id()))
        .collect();
    // ... rest unchanged
}
```

Update the call site in `check()`:

```rust
emit_literal_findings(&collector, &cfg, &excluded, &collector.slog_key_ids, &mut findings);
```

**Acceptance criteria covered**: AC5, AC6

---

#### Story 2.1.4 — Unit tests for slog key suppression (S)

**Given** the four AC-mapped test shapes for `replace-magic-literal`,
**When** `SyntaxRulesChecker` for Go runs,
**Then** results match the expectations below.

Add to the `#[cfg(test)]` block in `src/checkers/rules.rs` (or in a new `rules_slog_tests.rs` if the existing test file grows unwieldy):

```rust
#[test]
fn suppresses_slog_key_string_repeated_3_times() {
    // AC5: "file" in slog key position, repeated 3 times → no finding
    let src = r#"package main
import "log/slog"
func f(a, b, c string) {
    slog.Warn("msg1", "file", a)
    slog.Warn("msg2", "file", b)
    slog.Warn("msg3", "file", c)
}"#;
    let findings = magic_literal_findings(Language::Go, tree_sitter_go::LANGUAGE.into(), src);
    assert!(findings.is_empty(), "expected no findings, got: {findings:?}");
}

#[test]
fn suppresses_log_warn_key_string_repeated_3_times() {
    // AC5: log.Warn variant
    let src = r#"package main
import "log"
func f(a, b, c string) {
    log.Warn("msg", "err", a)
    log.Warn("msg", "err", b)
    log.Warn("msg", "err", c)
}"#;
    let findings = magic_literal_findings(Language::Go, tree_sitter_go::LANGUAGE.into(), src);
    assert!(findings.is_empty(), "expected no findings, got: {findings:?}");
}

#[test]
fn still_flags_genuine_repeated_string_outside_slog_key_position() {
    // AC6: "pending" appears 3 times as a non-key string → fires
    let src = r#"package main
func f() string { return "pending" }
func g() string { return "pending" }
func h() string { return "pending" }
"#;
    let findings = magic_literal_findings(Language::Go, tree_sitter_go::LANGUAGE.into(), src);
    assert!(!findings.is_empty(), "expected a finding for 'pending'");
}

#[test]
fn still_flags_slog_context_variant_key_string() {
    // *Context variants are NOT in SLOG_METHOD_NAMES (their key-start index differs);
    // a repeated key string in WarnContext is still a candidate for replace-magic-literal.
    let src = r#"package main
import "log/slog"
import "context"
func f(ctx context.Context, a, b, c string) {
    slog.WarnContext(ctx, "msg1", "file", a)
    slog.WarnContext(ctx, "msg2", "file", b)
    slog.WarnContext(ctx, "msg3", "file", c)
}"#;
    let findings = magic_literal_findings(Language::Go, tree_sitter_go::LANGUAGE.into(), src);
    // "file" appears 3 times; *Context suppression not implemented yet — finding expected
    assert!(!findings.is_empty(), "expected finding for 'file' in WarnContext (not yet suppressed)");
}

#[test]
fn still_flags_non_slog_value_string() {
    // AC6: "active" appears at value positions (even index 2) in slog.Warn calls — not a key,
    // not suppressed, so replace-magic-literal should still fire
    let src = r#"package main
import "log/slog"
func f() {
    slog.Warn("check1", "status", "active")
    slog.Warn("check2", "status", "active")
    slog.Warn("check3", "status", "active")
}"#;
    let findings = magic_literal_findings(Language::Go, tree_sitter_go::LANGUAGE.into(), src);
    assert!(!findings.is_empty(), "expected finding for 'active' at value position");
}
```

**Acceptance criteria covered**: AC5, AC6

---

## Phase 3: Backtest corpus validation and AC8 coverage gate

### Epic 3.1: Corpus validation (S)

**Goal**: Both checkers pass the standard backtest corpus with no new true-positive regressions, and the total new unit test count reaches ≥10 (AC7, AC8).

---

#### Story 3.1.1 — Backtest corpus run (S)

**Given** the standard corpus (`kubernetes/kubernetes`, `BurntSushi/ripgrep`, `stapler-squad`) cloned under `scripts/clone-backtest-repos.sh`,
**When** `kibitzer check native go-primitive-obsession <file>` and `kibitzer check native syntax-rules-go <file>` are run over each repo,
**Then** no new findings appear that were absent before this PR, and any suppressed findings are confirmed false positives by the `backtest-triage.py` workflow.

**Tasks**:

**Task 3.1.1** — Run corpus, triage new findings (M)

1. Ensure repos are cloned: `bash scripts/clone-backtest-repos.sh`
2. Capture pre-patch finding baseline for both checkers: `git stash push -u -m "go-stdlib-idioms-fp-baseline"` then immediately capture the entry's SHA with `git stash list --format='%H %gs' | head -1`. Run the corpus against the stashed (pre-patch) state. Restore with `git stash apply <sha>` (not `pop`, to avoid racing with other sessions), then drop it by re-finding its index: `git stash list --format='%H %gs' | awk '/go-stdlib-idioms-fp-baseline/{print NR-1}' | head -1` → `git stash drop stash@{N}`.
3. Apply the PR, run again, diff the two outputs with `scripts/backtest-triage.py`.
4. Any new true-positive findings (findings that correctly identify real problems the PR introduced) must be fixed before merge. New suppressions (findings that disappeared) must be confirmed as the intended false positives described in the requirements.
5. Record the triage result in `docs/backtest-triage/<date>/go-stdlib-idioms-fp.jsonl`.

**Acceptance criteria covered**: AC7

---

#### Story 3.1.2 — AC8 unit test count audit (XS)

**Given** all tests from Stories 1.1.3 and 2.1.4 are committed,
**When** `grep -c '#\[test\]'` is run over the changed test blocks,
**Then** the count is ≥10.

Expected count: 6 (Story 1.1.3: `suppresses_sort_slice_callback`, `suppresses_slices_sort_func_callback`, `suppresses_sort_search_callback`, `suppresses_slices_sort_stable_func_callback`, `suppresses_func_literal_in_test_file`, `still_flags_exported_multi_primitive_in_non_test_file`) + 5 (Story 2.1.4: `suppresses_slog_key_string_repeated_3_times`, `suppresses_log_warn_key_string_repeated_3_times`, `still_flags_genuine_repeated_string_outside_slog_key_position`, `still_flags_slog_context_variant_key_string`, `still_flags_non_slog_value_string`) = 11 new unit tests — satisfies AC8 (≥10). Existing tests are not counted toward this bar.

**Acceptance criteria covered**: AC8
