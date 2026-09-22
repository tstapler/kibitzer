# Build vs. Buy: `hide-delegate` (chained method/field-access depth) check

Research question: should the chain-depth / Law-of-Demeter check be built as a native
`src/rules.rs` rule (as `requirements.md` proposes) or sourced from an existing tool?

Repo facts checked directly:

- `src/rules.rs:913-930` (`max_nesting_depth`) is a plain recursive `Node` walk keyed off
  a per-language `LangRuleConfig` node-kind table — no `tree_sitter::Query`/`.scm` files
  anywhere in the tree (`grep -rn "Query::new\|tree_sitter::Query" src/` and
  `find . -iname "*.scm"` both empty, excluding `target/`).
- `docs/plugins.md` + `project_plans/checker-plugin-system/research/architecture.md`:
  a plugin is a `Check{ command: Some(path), output_format: Some(Sarif) }` entry — an
  externally installed, per-machine, target-triple-specific **binary**, auto-chained into
  `default_checks()` via `registered_plugin_checks()` (`src/config.rs:797`). SARIF
  responses are parsed by `render_sarif_output` in `src/check.rs:284`, and `{file}`/
  `{changed_lines}` substitution (`src/check.rs:634-645`) already supports per-file
  invocation of an external command — so the *dispatch* mechanism for a command-based
  checker exists and works today, independent of the formal `kibitzer plugin install`
  installer.

## 1. Existing OSS checker/linter with a chain-depth / Law-of-Demeter rule

| Tool | Language | Rule | Usable as subprocess from Rust? |
|---|---|---|---|
| PMD `LawOfDemeter` (`category/java/design.xml`) | Java only | Flags `b.getC().doIt()`-style chains reaching through a parameter/field; configurable `trustRadius` (was `trustRatio`); has had builder-pattern exclusion work landed ([PR #2010](https://github.com/pmd/pmd/pull/2010)) and known FP reports for lambdas ([#1014](https://github.com/pmd/pmd/issues/1014)), UTF-8 charset literals ([#1605](https://github.com/pmd/pmd/issues/1605)) | **Yes, technically** — PMD supports `-f sarif`/`--format sarif` since 6.31.0 ([docs](https://docs.pmd-code.org/latest/pmd_userdocs_report_formats.html)), and kibitzer's `command`+`Sarif` check shape already exists. But PMD ships as a JVM app in a zip with launcher scripts, not a self-contained per-target-triple native binary — it doesn't fit the formal `kibitzer plugin install` fetch/checksum/target-triple model (`docs/plugins.md`) without a wrapper; it would have to be a hand-authored `.claude/inspect.json` `command` entry (Java-only) requiring a local JVM + PMD install, bypassing the plugin installer entirely. |
| ESLint core | JS/TS | No official Law-of-Demeter or chain-depth rule. `newline-per-chained-call`'s `ignoreChainWithDepth` is a **formatting** knob (forces newlines), not a complexity gate — it doesn't fail/flag a chain as too deep. | N/A |
| `eslint-plugin-chain-max-length` | JS/TS | Third-party, npm-only, v1.0.1, scoped narrowly to *array method* chains (`.map().filter()...`), not general dot-chains/field access; no visible ongoing maintenance signal. | Weak fit even where present — wrong scope, unproven maintenance. |
| RuboCop | Ruby | No cop exists; a 2014 feature request ([rubocop#720](https://github.com/rubocop/rubocop/issues/720)) was never implemented, explicitly because of high false-positive risk on `map/select/reject` chains — the exact fluent-builder problem this project's requirements already call out. | N/A — not a target language anyway. |
| Rust clippy | Rust | No lint found for chain length or Law of Demeter (checked clippy's lint index and GitHub). | N/A |
| golangci-lint / staticcheck | Go | No chain-depth or Law-of-Demeter lint found. | N/A |
| detekt | Kotlin | Has general complexity/LOC rules; no chain-depth or Law-of-Demeter rule found. | N/A |

**Takeaway**: real prior art exists for exactly one of the seven target languages (Java,
via PMD), and even there it doesn't fit the plugin system's binary-distribution model
cleanly. Five of the six remaining languages (Go, Kotlin, Rust, Python, and — modulo the
weak npm package — JS/TS) have **no comparable OSS rule at all**; the community
consensus in the RuboCop thread is that fluent-chain false positives are exactly why
nobody has shipped one. There is no "buy once, cover all seven languages" option.

**Verdict: Not recommended (buy).** No single external tool covers the language matrix;
the one partial match (PMD/Java) doesn't fit the plugin binary-distribution shape and
would still leave 6/7 languages native-only, defeating the point of buying.

## 2. SaaS/managed

Not applicable. This is a local/CI diff-aware lint rule that must run per-file, offline,
in the same process as every other `rules.rs` check (`kibitzer run`, pre-commit hooks,
agent-invoked `run_checks`). A hosted/SaaS code-quality API would add a network
dependency and latency to a check that today is a synchronous in-process AST walk, and
none of PMD/ESLint/clippy/etc. are offered as SaaS for this specific rule anyway. Moving
on.

## 3. LLM-generated bespoke algorithm vs. adapting proven logic

The algorithm itself — walk an expression node, count consecutive dot-accesses
(method call or field access) along its "spine," compare to a threshold — is structurally
near-identical to `max_nesting_depth` (`src/rules.rs:913-930`): a recursive `Node` walk
driven by a per-language node-kind table (`LangRuleConfig`), same `Finding`/`[rule-id]`
message convention, same `RuleMeta`/`CATALOG` registration pattern used by
`deep-nesting`/`long-function`/`long-parameter-list`. There is no proven external
algorithm to "adapt" beyond this — PMD's `LawOfDemeter` implementation is itself a fairly
simple visitor (per its own issue history) applying a trust-radius heuristic, not a
sophisticated algorithm worth porting.

**This argues strongly for build**: the complexity is low, the pattern-match to existing
in-repo code is close to exact, and there's no external logic complex or valuable enough
to justify importing even if a clean import path existed (it doesn't — see §1 and §4).
The one genuinely hard sub-problem — the same-return-type fluent-builder heuristic from a
syntactic-only AST — is repo-specific work either way; no external tool solves it for
kibitzer's exact "no type-checker, tree-sitter syntax only" constraint, since PMD's
builder-exclusion logic (PR #2010) runs on a type-resolved AST, not tree-sitter's raw
syntax tree.

## 4. Fork/adapt: existing tree-sitter query (`.scm`) code for method chains

None exists in kibitzer today (verified: no `.scm` files, no `tree_sitter::Query` usage
anywhere in `src/`). The whole `rules.rs` family — including `deep-nesting`, the closest
structural analog — is hand-rolled `Node` recursion against `LangRuleConfig`, not
tree-sitter's declarative query language. Query files for chain/member-expression
detection do exist in other tools' ecosystems (e.g. syntax-highlighting queries in
`nvim-treesitter`, structural-search patterns in `semgrep`/`ast-grep` rule packs), but
adopting one here would mean introducing a second, inconsistent extension mechanism
(`tree_sitter::Query` + `.scm` grammar-specific query files) alongside the imperative
walk every other check in the file uses — for a rule whose walk is, per §3, not complex
enough to need it. The existing code style is itself the strongest argument against
forking query-based code: consistency with `deep-nesting` costs nothing extra, whereas a
`.scm`-based approach would need new query-compilation plumbing this codebase doesn't
have.

**Verdict: Not recommended (fork/adapt).** No off-the-shelf chain-detection queries were
found even by language ecosystem, and adopting the query mechanism itself would be an
architectural regression relative to matching `deep-nesting`'s existing shape.

## Summary table

| Option | Pros | Cons | Verdict |
|---|---|---|---|
| **Native `rules.rs` check** (as `requirements.md` proposes) | Matches existing `deep-nesting` shape almost exactly; covers all 7 target languages uniformly; no new dependency, no new check tier; algorithm is low-complexity | Repo-specific work for the same-type builder heuristic has no shortcut | **Recommended** |
| **External linter via `command`/plugin (PMD LawOfDemeter)** | Real prior art for Java; PMD's `-f sarif` output is directly consumable by kibitzer's existing SARIF parser | Java-only — leaves 6/7 languages uncovered; requires a local JVM + PMD install; doesn't fit the plugin installer's native-binary/target-triple model, so it'd have to be hand-authored per repo, contradicting `default_checks()`'s "active with no `.claude/inspect.json` required" acceptance criterion; explicitly excluded by `requirements.md`'s non-goals ("No new checker capability/tier") | **Not recommended** |
| **SaaS/managed** | N/A | Wrong shape for an offline per-file AST rule; no vendor offers this rule specifically | **Not recommended** |
| **Fork/adapt external `.scm` query code** | Query files exist in other ecosystems for related node kinds | None found specifically for chain-depth; would introduce a second, inconsistent extension mechanism into `rules.rs` | **Not recommended** |

## Final recommendation

Build natively, exactly as `requirements.md` specifies: a `rules.rs` rule mirroring
`deep-nesting`'s `LangRuleConfig`-driven recursive walk, with `MAX_CHAIN_DEPTH` (or
similar) as a named `const`, registered in `CATALOG` and `default_checks()`. No OSS
tool covers the full seven-language matrix; the one partial match (PMD/Java) is
Java-only, doesn't fit the plugin system's binary-distribution model, and is explicitly
ruled out by the requirements' own non-goals. The chain-walk algorithm is simple enough,
and close enough to `max_nesting_depth`'s existing shape, that there is no complexity
saved by looking further afield — the only nontrivial design work (the same-return-type
builder-suppression heuristic under a syntax-only AST) is unavoidable repo-specific work
regardless of build-vs-buy, since no external tool solves it under kibitzer's
type-checker-free constraint either.
