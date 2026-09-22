# Build vs. Buy: replace-magic-literal

Research question: should kibitzer implement Fowler's Replace Magic Literal natively in
`src/rules.rs`'s `SyntaxRulesChecker`, or shell out to / wrap an existing tool?

## 1. Existing OSS options

| Tool | Languages covered | Runtime dependency | Matches Fowler's semantics? | License |
|---|---|---|---|---|
| ESLint `no-magic-numbers` (core + `@typescript-eslint` variant) | JS/TS only | Node.js + full ESLint toolchain (config, plugin resolution) | **No** — flags every non-ignored numeric literal on sight, not "≥2 occurrences." Confirmed via ESLint's own docs: the `ignore` option's default is `[]`, and any number not in it is flagged regardless of whether it repeats once or fifty times ([no-magic-numbers docs](https://eslint.org/docs/latest/rules/no-magic-numbers)). This is a stricter, different rule than the one this project's requirements.md specifies (repetition-based, AC 2). | MIT |
| PMD `AvoidDuplicateLiterals` / `AvoidLiteralsInIfCondition` | Java only — implemented as a Java-specific `AbstractJavaRulechainRule` ([source](https://github.com/pmd/pmd/blob/main/pmd-java/src/main/java/net/sourceforge/pmd/lang/java/rule/errorprone/AvoidDuplicateLiteralsRule.java)); no Kotlin equivalent despite PMD's separate [Kotlin language module](https://pmd.github.io/pmd/pmd_languages_kotlin.html) | JVM + PMD distribution | Yes for Java specifically (this is the closest semantic match to Fowler's rule of any tool surveyed) | BSD-style (PMD project) |
| SonarQube/SonarLint S1192 (duplicated string literals) / S109 (magic numbers) | Multi-language, but only inside the SonarQube/SonarLint platform | Requires the SonarQube server (or SonarLint IDE plugin) — not a standalone CLI kibitzer could shell out to per-file | Yes, close semantic match | SonarQube Community Edition: LGPL v3; full multi-language rule set requires a commercial tier |
| Semgrep | Generic pattern-matching across many languages | Python-based CLI | No — Semgrep's rule model matches individual pattern occurrences; it has no built-in primitive for "this literal value repeats N times across the file," which requires stateful aggregation across matches, not a single pattern match | LGPL 2.1 (engine); some rules gated behind Semgrep's paid registry |
| ast-grep | Generic pattern-matching, used elsewhere in this org's tooling | Rust CLI, embeddable | No built-in aggregation primitive either. Its [rule model](https://ast-grep.github.io/guide/rule-config.html) (atomic / relational / composite rules) matches and constrains individual AST nodes and their structural neighbors; nothing in that model counts *how many times* a matched value recurs elsewhere in the file. That count would have to be computed by post-processing ast-grep's JSON match output in a wrapper script — at which point ast-grep is only supplying "find all string/number literal nodes," and the actual smell logic (grouping by value, filtering the allow-list, checking the named-constant exclusion) still has to be hand-written outside it. | MIT |

**No single existing tool covers kibitzer's actual target set** (Go, TypeScript, Tsx, JavaScript,
Python, Java, Kotlin, Rust) with Fowler's exact repetition semantics. Covering all eight
languages via existing tools would mean wrapping at least three separate ecosystems (ESLint for
JS/TS/Tsx, PMD for Java only, and something hand-built for Go/Python/Kotlin/Rust, none of which
have a magic-literal tool at all) — three toolchain dependencies (Node.js, JVM, ad hoc scripts)
to get worse semantic fidelity and partial coverage, versus one Rust code path that already spans
all eight languages via the existing `LangRuleConfig` table.

## 2. Kibitzer's own "external command" mechanism

`docs/plugins.md`'s opening paragraph frames a plugin/external command as the right tool for "a
specialized checker — a domain-specific linter, a proprietary model, a niche SARIF-emitting tool"
that isn't part of kibitzer's own general-purpose catalog. `docs/suppressing-checks.md` lists the
current default catalog explicitly: `comment-quality-*`, `syntax-rules-*`, `markdown-link-integrity`,
`primitive-obsession`, `duplicate-code`, `duplicate-code-cross-file`, `file-complexity`, and the
Go-specific error-handling checks — all native, all requiring zero `.claude/inspect.json` setup.

A `.claude/inspect.json`-declared `command` check (optionally with `output_format: sarif`,
`docs/output-formats.md`) is per-repo, opt-in, and only active where someone has hand-authored
that config entry and installed whatever binary the command invokes. That directly conflicts with
this project's requirements.md, which frames the smell as "one of the best-established... and the
cheapest to detect" and explicitly targets `default_checks()` (AC 5) so it protects every kibitzer
install without per-repo setup. It also does not solve the coverage problem from Section 1: there
is no single external command that covers all eight languages, so an external-command approach
would still mean either wiring up N separate per-language commands in every consuming repo's
config (the antithesis of "no repo-specific setup") or accepting partial language coverage.

This matches CLAUDE.md's own stated standard: "a new native per-file checker that doesn't need
repo-specific setup... goes in `default_checks()` as part of landing it, not as a follow-up
someone has to remember." The issue's proposal to extend `src/rules.rs` directly, rather than
document an external-command recipe, is consistent with that standard, not a deviation from it.

## 3. LLM-generated bespoke implementation vs. reusing tree-sitter infrastructure

Confirmed: this is not an "LLM algorithm risk" case in the sense that term is used for, e.g., a
fuzzy-matching or ML-scored checker. The core walk is mechanical: visit literal nodes across an
already-parsed tree-sitter tree (the same tree the three existing complexity rules already walk),
bucket by literal value, count occurrences, and filter through a fixed allow-list. `SyntaxRulesChecker`
already owns the tree-sitter integration, the per-language `LangRuleConfig` table, and the
`Finding`-emission plumbing — none of that needs to be reinvented, LLM-authored or otherwise.

One part of the proposed rule is a real name-resolution problem, not a pure AST-shape check, and
is where bespoke-vs-reuse risk is genuinely worth flagging: the exclusion for a literal that's
"the direct initializer of a `const`/`let`/`val`-style binding... referenced elsewhere by name."
That requires:

- Identifying the binding's declared name per language (e.g. Go's `const X = 5`, Python's
  module-level `X = 5` — Python has no `const` keyword, so this exclusion can only ever be a
  naming/scope heuristic there, not a language-enforced guarantee).
- Resolving every other identifier reference in the file back to that same binding, not just a
  textual match of the same name — a same-named local shadowing the constant in another function
  should not count as "referenced elsewhere," and getting this wrong either overclaims coverage
  (silently drops a real duplicate-literal finding because of a spurious name collision) or
  underclaims it (still flags an already-correctly-factored constant, defeating AC 4).
- Doing this per language with no shared scope-resolution infrastructure in kibitzer today —
  `SyntaxRulesChecker`'s existing three rules never need to resolve an identifier to its
  declaration; they only inspect a single function body's shape.

This is the one place in this feature where "mechanical AST walk" is not quite the whole story,
and it deserves the most scrutiny/test coverage of the four rule requirements (AC 2–4) — not because
it's algorithmically novel (it's a bounded, well-understood problem: same-file, non-shadowed,
single-token name binding), but because a shallow implementation (pure string match on identifier
text, no scope awareness) will produce exactly the false positives/negatives the appetite section's
"honest unknown" already flags. Recommend explicitly scoping the initial implementation to the
simplest safe heuristic — match by identifier text within the same file, accepting that a
deliberately-shadowed same-named constant in another function is a known, documented false
negative rather than attempting real lexical scope resolution — and letting the backtest (AC 7–8)
confirm whether that heuristic is good enough in practice, rather than over-building a scope
resolver up front for a "small"-appetite feature.

## 4. Adapting an existing kibitzer construct (`duplicate_code.rs`)

`src/duplicate_code.rs`'s `find_duplicate_blocks` (and its cross-file counterpart
`find_cross_file_duplicates`) already implements "does this thing repeat, and how many times" —
but at the wrong granularity and via the wrong mechanism to reuse directly:

- It's a **text-line-window hash**, not an AST walk: it slides a `MIN_BLOCK_LINES`-line window
  over trimmed source lines and groups by exact string match of the whole window
  (`find_duplicate_blocks`, `src/duplicate_code.rs`). It has no concept of a "literal node" at
  all — it would treat `x = 5` and `y = 5` as unrelated unless the surrounding lines also matched
  verbatim, and conversely could accidentally match `5` inside an unrelated numeric context (e.g.
  a line number in a comment) since it never touches the parse tree.
- Its unit of comparison is a multi-line block (min 2 lines by convention here), not a single
  token/literal value. A magic-literal check needs the opposite granularity: single-token
  identity across arbitrarily distant, structurally unrelated locations in the file.
- Its occurrence-counting mechanism — group candidate windows into a hash map keyed by
  normalized text, then filter to groups meeting `MIN_OCCURRENCES` — is a reusable *pattern*
  (`HashMap<Key, Vec<Location>>`, filter by count), and this project's `walk_blocks`-style whole-
  tree walk should adopt that same shape: walk all literal nodes, key a map by literal value, and
  filter to entries with `len() >= 2`. But that's reusing an idiom already established elsewhere
  in this file (`long-function`'s threshold-and-filter pattern is the same idiom too), not reusing
  `duplicate_code.rs`'s code.

**Verdict for this section**: no fork/adapt-in-place opportunity. The tokenization/hashing
machinery in `duplicate_code.rs` operates on raw text lines and is not shaped to extract typed
literal values, still less to distinguish a numeric/string literal node from surrounding code or
apply the named-constant exclusion. The new check should be written as a new `walk_blocks`-style
whole-tree walk directly against `LangRuleConfig`'s existing per-language node-kind table, adding
one new table column (literal node kinds) the same way the three existing rules each added their
own columns — following the *shape* of `duplicate_code.rs`'s occurrence-counting logic, not its
implementation.

## Recommendation

| Option | Verdict |
|---|---|
| ESLint `no-magic-numbers` (wrap/shell out) | **Not recommended** — JS/TS only, wrong semantics (flags every literal, not repeats), and requires a Node.js/ESLint toolchain dependency kibitzer doesn't otherwise have. |
| PMD `AvoidDuplicateLiterals` (wrap/shell out) | **Not recommended** — Java only, no Kotlin coverage despite PMD's own Kotlin module, requires a JVM dependency. |
| SonarQube/SonarLint S109/S1192 (wrap/shell out) | **Not recommended** — requires the SonarQube platform or a commercial tier for full multi-language coverage; not a lightweight per-file CLI kibitzer could invoke. |
| Semgrep or ast-grep pattern rules (wrap/shell out) | **Not recommended** — neither's rule model has a built-in "value repeats N times" aggregation primitive; the actual smell logic would have to be hand-written in a wrapper script regardless, at which point the pattern-matching layer adds an extra process/dependency for no semantic gain over doing the AST walk directly in kibitzer. |
| Kibitzer's own external-`command`/plugin mechanism | **Not recommended** for this feature — it's the right tool for a specialized, opt-in, repo-specific checker, not for a general-purpose, always-on smell across 8 languages; using it here would also reintroduce the same N-tools-for-8-languages coverage gap and contradict this project's own default-catalog acceptance criteria (AC 5). |
| Native implementation in `src/rules.rs`'s `SyntaxRulesChecker` | **Recommended** — only option covering all 8 target languages with consistent semantics, zero new runtime dependency, automatic `default_checks()` registration via the existing `syntax_rules_checks()` chain, and a natural fit alongside the three existing complexity rules' `LangRuleConfig`-driven design. |

**Overall verdict: build natively in `src/rules.rs`.** No existing OSS tool or kibitzer mechanism
covers this project's actual requirements (all 8 languages, repetition-based semantics, zero
repo-setup, native `Finding` output). The implementation is a mechanical AST walk over
infrastructure kibitzer already owns, with one genuinely tricky sub-problem — the named-constant
"referenced elsewhere" exclusion — that warrants the most test/backtest attention but does not
change the build-vs-buy conclusion, since no external tool solves that sub-problem better than a
same-file identifier-text heuristic would.
