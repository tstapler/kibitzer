# Build-vs-buy: conditional-logic smell checks

Covers three Fowler checks proposed for kibitzer: `decompose-conditional`,
`consolidate-conditional`, and `remove-control-flag`
([refactoring-catalog-analysis.md, Chapter 10 table](../../docs/refactoring-catalog-analysis.md),
issue [#55](https://github.com/tstapler/kibitzer/issues/55)).

---

## 1. Existing linter coverage — Go ecosystem

### decompose-conditional

**What it detects:** a condition expression that is too complex to read in-place
and should be extracted into a named function (Fowler §10.1).

No Go linter (staticcheck, go-critic, golangci-lint, revive) implements this
at the per-condition level. The closest approximations are:

| Tool | Rule | What it actually checks | Relationship |
|------|------|-------------------------|--------------|
| go-critic | `nestingReduce` (style, opinionated, experimental) | nesting depth could be reduced; `bodyWidth` default 5 stmts | directional match; fires on structure, not condition complexity |
| kibitzer | `deep-nesting` | control-flow nesting > 4 | already ships; prerequisite signal |
| kibitzer | `extract-variable` | expression nesting ≥ 5 in condition/return | covers part of the territory |
| PMD (Java) | `CyclomaticComplexity` | McCabe ≥ 10 per method (default) | method-level, not condition-level; Java only |
| PMD (Java) | `CognitiveComplexity` | cognitive complexity ≥ 15 per method (default) | same scope; Java only |

**Gap:** No existing Go tool detects that a *specific condition expression* is
too complex to read inline. Full Fowler detection requires per-condition
complexity scoring that kibitzer does not yet have (tracked in issue #32,
"File/function complexity walker").

### consolidate-conditional

**What it detects:** a run of sequential `if` statements all returning or
assigning the same value, which should be merged into one combined condition
(Fowler §10.3).

| Tool | Rule | What it actually checks | Notes |
|------|------|-------------------------|-------|
| go-critic | `ifElseChain` (style, default on; `minThreshold=2`) | repeated if-else chain → suggests `switch` | different refactoring: about switch replacement, not same-value consolidation |
| go-critic | `dupBranchBody` (diagnostic, default on) | identical code in both branches of one conditional | flags same code in if/else, not same return across sequential ifs |
| PMD (Java) | `CollapsibleIfStatements` | nested `if` inside outer `if` merges with `&&` | nested parent-child only, not sequential; different from Fowler |
| ESLint | `no-lonely-if` | `if` as sole statement inside `else` block | structural case of PMD's rule; JS only |
| jSparrow (Java) | `CollapseIfStatements` | merges nested ifs with `&&`, introduces local var for long conditions | Java-specific IDE plugin |

**Gap:** No existing Go linter (or any mainstream multi-language tool) implements
Fowler's exact definition — sequential independent `if` stmts returning/assigning
the *same* value. What ships is a narrower, nested-parent-child variant (PMD) or
a different refactoring entirely (go-critic's switch suggestion).

**Key documented FP patterns for the nearest analogues:**
- go-critic `ifElseChain`: one Gitea config disables it alongside `singleCaseSwitch`
  with the note that it fires in cases where no other structure was possible.
  The checker itself documents: "Permits single else or else-if; repeated
  else-if or else + else-if will trigger suggestion."
- refactoring.guru's Consolidate Conditional Expression page states explicitly:
  "make sure that the conditionals don't have any 'side effects' or otherwise
  modify something" — this is the core FP risk for any implementation: each
  `if err != nil { return err }` guard clause is structurally identical to a
  consolidate-conditional candidate but is idiomatic Go and must not be flagged.

### remove-control-flag

**What it detects:** a `bool` variable used only to exit a loop — written only as
`true`/`false` and read only in the loop's condition — that should be replaced
with a direct `break`/`return` (Fowler §10.9).

No implementation found in go-critic's 108 checkers
([go-critic.github.io/overview](https://go-critic.github.io/overview)),
staticcheck, or revive. This pattern is not documented as a named check in any
mainstream Go or cross-language analyzer surveyed.

**Structural signature:** tight and distinguishable from result accumulators:
1. Variable type is `bool` (statically known).
2. Only ever assigned the literal `true` or `false`.
3. Only read in a `for` loop condition (not passed to functions, not returned).
4. The flag is set to its exit value *inside* the loop body.

A result accumulator (`found := false; ... found = true; return found`) is read
*after* the loop, so condition 3 rules it out. The main FP class is a flag that
is also passed to a helper function or checked after the loop — both easily
excluded by the same read-site scan that establishes condition 3.

---

## 2. Industry precedent — non-Go analyzers

### consolidate-conditional across ecosystems

| Tool / language | Rule | Match to Fowler? |
|-----------------|------|-----------------|
| PMD / Java | `CollapsibleIfStatements` | Partial: nested (parent+child) ifs, not sequential same-value ifs |
| ESLint / JS | `no-lonely-if` | Partial: `if` sole stmt in `else`, same structural subset |
| jSparrow / Java | `CollapseIfStatements` | Partial: nested, adds local var for long conditions |
| SonarQube | RSPEC-3973 | Unrelated: indentation without braces issue |

**Conclusion:** No mainstream analyzer fully implements Fowler's sequential
same-outcome consolidation. All shipping rules detect the nested-parent-child
subset (which corresponds to a different, narrower refactoring). Fowler's version
remains un-implemented in the ecosystem, which means kibitzer would be first —
no prior-art FP catalog to borrow from.

### remove-control-flag across ecosystems

No shipping implementation found in PMD, SonarQube, ESLint, Rubocop, or
SpotBugs rule catalogs (search terms: "control flag", "loop flag", "boolean loop
sentinel"). This is consistent with the tight structural definition being Go-AST-
friendly but rarely documented as a named check.

---

## 3. ADR-001 false-positive bar

Kibitzer's established bar, from ADR-001 (`project_plans/replace-magic-literal/decisions/ADR-001-magic-literal-exclusion-and-threshold-strategy.md`)
and the `docs/backtest-triage/` corpus sessions: a default-on check must pass
corpus backtest with **FP rate < 40%** on a sampled finding set before shipping.
Prior checks learned this lesson empirically: `duplicate-code` shipped at ≥2
occurrences, found ~60% FP noise, and was walked back to ≥3; `replace-magic-literal`
was held at ≥3 from the start after backtesting showed 76% FP at ≥2.

Applying this to the three candidates:

| Check | Expected FP risk | Reason |
|-------|-----------------|--------|
| `decompose-conditional` | High | No existing Go implementation; condition-complexity scoring is new infrastructure (#32); "complex enough" threshold is not established; risks over-flagging idiomatic multi-clause conditions |
| `consolidate-conditional` | High | Sequential guard-clause pattern (`if err != nil { return err }`) is the canonical Go idiom and structurally identical to a true positive; no prior-art FP catalog; Fowler's own pre-condition (no side effects in conditions) cannot be mechanically verified without data-flow analysis |
| `remove-control-flag` | Low–medium | Structural signature is tight; no prior implementations to compare against, but conditions 1–4 are individually checkable from the AST; main FP class (flag also read after loop or passed to function) is eliminable in the same scan |

---

## 4. Build recommendation

**Build natively, not via `command` check.** The `command`-check path
(delegating to `go vet`, golangci-lint, etc.) is not viable here: none of the
three checks exist in any Go linter. Even if they were added upstream, deferring
to a `command` check would add an external tool dependency and lose language
portability — kibitzer's tree-sitter approach already supports Go, TypeScript,
Java, Rust, and more in one codebase.

Kibitzer's existing `SyntaxRulesChecker`/`rules.rs` infrastructure (tree-sitter,
`LangRuleConfig`, per-language `GoKind` node-kind enums) is the right target.
`remove-control-flag` fits as a new native per-file checker like
`flag_argument.rs` or `go_ignored_error.rs`; `consolidate-conditional` and
`decompose-conditional` fit the `rules.rs` per-file rule-catalog pattern.

**Staged build order:**

1. **`remove-control-flag` first.** Clearest structural signature, no existing
   ecosystem analogue (so no alternative for users), and the structural
   conditions are individually verifiable. Ship as a native per-file checker,
   Go-first. Must pass the ≥60% TP rate bar on backtest before default-on.

2. **`consolidate-conditional` second, with guard-clause exclusion pre-committed.**
   The guard-clause FP class (`if err != nil { return err }`) must be explicitly
   excluded before shipping default-on — it is the dominant false positive and is
   structurally distinguishable (condition checks a nil-able type, branch is a
   single early-return). Corpus backtest is mandatory before merge.

3. **`decompose-conditional` deferred.** It depends on issue #32 (per-function
   complexity walker) for the condition-complexity signal. Without that
   infrastructure, the check can only proxy on condition-expression nesting —
   which `extract-variable` (nesting ≥ 5) already covers. No new check is
   warranted until #32 ships and provides a per-condition complexity score
   kibitzer can threshold on.

**AST approach:** tree-sitter throughout (not `go/ast`). Go-critic uses `go/ast`
because it ships as a Go package; kibitzer's grammar-agnostic design makes
tree-sitter the right choice. The `GoKind` typed-node-kind infrastructure
(`docs/typed-node-kinds.md`) already covers the Go AST nodes needed for all
three checks (`for_statement`, `if_statement`, `assignment_statement`,
`var_declaration`, `short_var_declaration`).

---

## 5. Sources verified

- go-critic checker catalog (108 checks, 2026): [https://go-critic.github.io/overview](https://go-critic.github.io/overview)
  — no "decompose conditional," "consolidate conditional," or "control flag" checks exist
- PMD Java design rules: [https://pmd.github.io/pmd/pmd_rules_java_design.html](https://pmd.github.io/pmd/pmd_rules_java_design.html)
  — `CollapsibleIfStatements` is nested-parent-child only; `CyclomaticComplexity` threshold 10; `CognitiveComplexity` threshold 15
- Refactoring.guru, Consolidate Conditional Expression: [https://refactoring.guru/consolidate-conditional-expression](https://refactoring.guru/consolidate-conditional-expression)
  — explicit FP warning: side effects in conditions invalidate the consolidation
- Kibitzer ADR-001 (40% FP bar): `project_plans/replace-magic-literal/decisions/ADR-001-magic-literal-exclusion-and-threshold-strategy.md`
- Refactoring catalog analysis (issue #55 bundle): `docs/refactoring-catalog-analysis.md:258–264`
- go-critic `ifElseChain` FP in real project: [Gitea golangci config](https://gitea.psi.ch/gitea/gitea/commit/5b94a07ab5245bd56f49507f17aff5b265aa29db) disables `ifElseChain`/`singleCaseSwitch`
- staticcheck checks: [https://staticcheck.dev/changes/2019.1](https://staticcheck.dev/changes/2019.1) — no control-flag or consolidate-conditional rules
