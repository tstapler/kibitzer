# Requirements: kibitzer

**Date**: 2026-08-22
**Type**: feature addition
**Complexity**: 3 — system design (multiple epics, appetite TBD until Phase 3 sizing)

## Problem Statement

Tyler is turning `tstapler/kibitzer` into his "persona linter for everything" — the one advisory, diff-aware checker he wires into every project instead of adopting a different language-specific tool per repo. Architecture linting is the one category where that's not true yet: enforcing layer boundaries, dependency direction, package content, and naming conventions today means reaching for an external, language-specific tool per project — `depguard` (Go, via `golangci-lint`), `go-arch-lint` (Go, standalone), `arch-go` (Go, standalone), or `cht-go-lint` (Go, early-stage) — none of which kibitzer's own users get for free, and none of which work outside Go at all.

Kibitzer already has a real head start: `src/architecture_checks.rs` implements `ImportCycleChecker`, `LayeringChecker`, and `CouplingChecker` over a tree-sitter-based `ImportGraph` (`src/import_graph.rs`, currently Go + JS/TS), configured via `ArchitectureConfig` in `src/config.rs` and exposed through the `architecture_assessment` MCP tool (`src/mcp.rs`) with Mermaid dependency-diagram output. What's missing, verified by direct comparison against the four external tools researched:

1. **Arbitrary named-component dependency rules.** `ArchitectureConfig.layers` is a single globally-ordered tier list (`layer_of()` in `architecture_checks.rs:68-76`) — a package belongs to the first layer whose name matches a path segment, and only "higher may depend on lower" is expressible. `depguard` and `go-arch-lint` both support arbitrary pairwise allow/deny rules independent of any total ordering (e.g. "services must not import `net/http`", unrelated to layer position).
2. **Package content rules.** `arch-go`'s `contentsRules` (e.g. "package `internal/model/**` must only contain structs") has no equivalent — kibitzer's architecture checkers only see the import graph, not package contents.
3. **Naming convention rules.** `arch-go`'s `namingRules` (structs implementing interface X must follow a naming pattern) and `cht-go-lint`'s naming rule category — no equivalent.
4. **Import-graph language coverage.** `import_graph.rs::build()` only extracts imports for Go and JS/TS. Python, Java, and Kotlin already have tree-sitter grammars as Cargo dependencies (`tree-sitter-python 0.23`, `tree-sitter-java 0.23.5`, `tree-sitter-kotlin-ng 1.1.0` — verified in `Cargo.toml`) and are already wired for the separate `syntax-rules` complexity checker (`docs/syntax-rules.md`), but the same grammars aren't yet used for import-graph extraction, so none of the architecture checkers run on those languages today.

## Baseline

Today, a Go project wanting depguard/go-arch-lint/arch-go-equivalent architecture enforcement adopts one of those four external tools directly — separate binary or `golangci-lint` config, separate CI step, separate config file/schema to learn, and zero reuse of kibitzer's existing diff-aware/caching/hook infrastructure. A Python, Java, Kotlin, or non-Go/JS/TS project gets no architecture enforcement from kibitzer at all today (`layering`/`coupling`/`import-cycles` silently produce no findings — the import graph has no edges for those languages). Package-content and naming-convention rules have no tool-agnostic equivalent in kibitzer today regardless of language.

## Users / Consumers

Tyler, directly — this is his own tool and workflow. Secondary: anyone else who installs `tstapler/kibitzer` via the Homebrew tap and wires it into their own project's `.claude/inspect.json`, CI, or Claude Code hook (kibitzer is public/OSS with real release automation — `dist-workspace.toml`, `cliff.toml`, a Homebrew tap — but no evidence of external adoption beyond Tyler found in this session).

## Success Metrics

- A single project's `.claude/inspect.json` can express, using kibitzer's own config conventions (no new config file, no separate binary): (a) at least one arbitrary named-component allow/deny dependency rule that an ordered `layers` list cannot express, (b) at least one package-content rule, (c) at least one naming-convention rule — and `kibitzer check native <checker> <path>` / the `architecture_assessment` MCP tool correctly flags a deliberately-introduced violation of each.
- The import graph correctly extracts import/dependency edges for at least one of Python, Java, or Kotlin (stretch: all three), verified by a test fixture per language analogous to the existing Go/TS fixtures in `import_graph.rs`'s test module.
- Every existing config using `layers:` today (if any exist in Tyler's other repos) continues to parse and produce identical findings after the change — verified by running kibitzer's existing `import_graph.rs`/`architecture_checks.rs` test suite unmodified plus a new regression test asserting `layers` desugars to the equivalent new-schema rules.

## Appetite

TBD — Phase 3 (plan) sizes effort per gap (dependency-rule engine, content rules, naming rules, per-language import-graph extraction ×3) and proposes sequencing/cutoff; full scope was chosen for requirements purposes but is not a fixed commitment to ship all four in one PR.

## Constraints

None hard — personal OSS project, no deadline, no external team dependency. Existing release process (`cargo-dist`, tag-triggered) applies unchanged; this ships as a normal version-bump release once merged to `master`.

## Non-functional Requirements

- **Performance SLO**: Architecture checks already run only on the `batch` trigger, never `PostToolUse` (`config.rs:57-58`'s existing comment: "rebuilding the import graph on every `PostToolUse` edit is too expensive") — this constraint carries forward unchanged; new rule categories (content/naming) must not change that trigger restriction.
- **Scalability**: Not applicable beyond "whole-repo batch scan," matching existing `import-cycles`/`layering`/`coupling` behavior.
- **Security classification**: Public — kibitzer is OSS, no regulated or confidential data involved.
- **Data residency**: Not applicable.

## Scope

### In Scope

- New/extended `ArchitectureConfig` schema supporting arbitrary named-component definitions (glob-mapped, like `go-arch-lint`'s `components`) and arbitrary per-component allow/deny dependency rules (not just a total order).
- `layers: Vec<String>` continues to parse, desugaring internally into the new component/rule model (per the config-compat decision below) — no breaking change to any existing config.
- A package-content rule category (at minimum: "package X may/must-only contain declaration kind Y" — struct/interface/function, matching `arch-go`'s `contentsRules` as the reference shape).
- A naming-convention rule category (at minimum: "types implementing interface X must match naming pattern Y", matching `arch-go`'s `namingRules` as the reference shape).
- Import-graph extraction extended to Python, Java, and Kotlin using the tree-sitter grammars already present as Cargo dependencies, following the same per-language care already demonstrated in `rules.rs`'s `lang_config()` for syntax-rules (verified node kinds, not guessed).
- Existing checkers (`ImportCycleChecker`, `LayeringChecker`, `CouplingChecker`) continue to work unmodified against the extended graph and new config once a project's import graph covers more languages.
- `architecture_assessment` MCP tool and Mermaid diagram output extended to reflect any new checker findings, without a redesign of that surface.

### Out of Scope

- DDD-specific rule categories (aggregate boundaries, repository contracts, value-object immutability — `cht-go-lint`'s differentiator). Explicitly deferred per the original research: that tool is 3-star/unproven and its rule categories are not requested here.
- Config-file schema compatibility with any of the four external tools' YAML (depguard, go-arch-lint, arch-go, cht-go-lint) — kibitzer's own config conventions win, per the original scoping instruction to this feature.
- ~~Migrating kibitzer itself to self-check with the new rules~~ — **moved to Phase 7 scope** (decided after Phase 2 research): Phase 7 ships a minimal `.claude/inspect.json` for the kibitzer repo itself exercising the new dependency/content/naming rules, proving the feature end-to-end on a real repo before merge.
- Multi-language import-graph coverage for any language beyond Python/Java/Kotlin (e.g. Rust, C#) — not requested, no existing tree-sitter grammar dependency to build on.

## Rabbit Holes

- **Package-content rules require a new code-inspection pass, not just graph traversal.** Content/naming rules need to walk each file's tree-sitter AST for top-level declarations (kind + name), which is new plumbing distinct from both `import_graph.rs` (import edges only) and `rules.rs` (function-body-scoped complexity rules) — likely a third parsing pass per file, not a trivial extension of either existing module. Phase 3 should size this as its own epic, not a sub-task of dependency rules.
- **Per-language import extraction is not uniform.** The existing Go/JS/TS extraction (`import_graph.rs:103-` for Go, `:213-` for JS) is language-specific hand-written tree-sitter node walking. Python (`import`/`from...import`), Java (`import` statements, package-qualified), and Kotlin (`import` with wildcard/alias support) each have distinct grammar shapes — this is 3 separate implementation efforts, not one generic pass, mirroring the per-language node-kind documentation already written out in `docs/syntax-rules.md`.
- **"Arbitrary allow/deny rules" can re-litigate `go-arch-lint` vs. `arch-go` vs. `depguard`'s three different rule-expression styles** (adjacency list vs. file-glob deny/allow vs. component `mayDependOn`) — Phase 2/3 should pick one shape deliberately (informed by kibitzer's existing glob-scope conventions elsewhere in `config.rs`) rather than trying to support all three.

## Alternatives Considered

- **Just tell users to adopt the external tool matching their language** (status quo) — rejected: defeats the stated goal of kibitzer being the one persona linter across projects/languages.
- **Shell out to the external tools from kibitzer via `command` checks** (kibitzer already supports arbitrary shell commands per `Check.command`) — rejected as the primary approach: still requires installing a separate binary per language/project, no shared config schema, no reuse of kibitzer's diff-aware/caching infrastructure; native in-process checkers are kibitzer's established pattern for everything else (`checker.rs`'s registry, `architecture_checks.rs`'s registry).
- **Adopt `arch-go`'s Go library directly as a dependency** rather than reimplementing content/naming rules — not evaluated in depth this session; Phase 2 research should at least confirm whether `arch-go`'s core rule-evaluation logic is usable as a library (vs. CLI-only) before committing to a from-scratch reimplementation, since reuse could shrink the content/naming epics substantially if viable.

## Feasibility Risks

- Tree-sitter grammar versions for Python/Java/Kotlin are already pinned and in use (`Cargo.toml`) for syntax-rules, but import-graph extraction touches different node kinds (`import_statement`-family nodes vs. function/control-flow nodes) that haven't been verified against these specific grammar versions yet — same verification discipline `rules.rs`'s `lang_config()` comments already demonstrate (real `to_sexp()` output, not assumed node names) will be needed per language.
- Backward-compatible `layers`-as-sugar (confirmed direction below) adds a small but real maintenance surface: the desugaring logic itself needs a regression test, not just "old tests still pass by coincidence."

## Observability Requirements

Standard `kibitzer check`/MCP tool output is sufficient — architecture findings already flow through the same `{file}:{line}: message` / `ArchFinding` convention as every other checker. No new logging, metrics, or alerting needed; this is a local CLI/MCP tool, not a running service.

## Risk Control

Not needed — low risk. This is an additive, backward-compatible change to a local dev-tool CLI (confirmed: `layers` continues to parse and produce identical findings). Normal PR review + kibitzer's own test suite is sufficient; no feature flag, staged rollout, or rollback procedure beyond "revert the release tag" is warranted.

## Open Questions

*(resolved after Phase 2 research + user follow-up — kept here for the record)*

- ~~Which rule-expression shape fits kibitzer's existing config conventions best?~~ **Resolved**: named `Component{name, paths}` glob-mapped (reusing `glob.rs::matches_scope`) + `DependencyRule{may_depend_on/deny_depend_on}` — the `go-arch-lint`-style shape (`components:` + `deps:`), not `arch-go`'s inline-glob-per-rule style. See `research/architecture.md` and `research/ux.md`.
- ~~Is `arch-go`'s core Go library reusable in-process, or CLI-only?~~ **Resolved**: not portable as code (Go-stdlib-coupled — `go/ast` type-switches, Go-specific), but its rule **schema** (glob-matched components, `should_only`/`should_not` dependency/content rule pairs) is worth porting as a reference design. See `research/build-vs-buy.md`.
- Exact sequencing/cutoff across the 4 sub-epics once Phase 3 sizes each — appetite remains deliberately TBD; this is Phase 3's job, not a research question.
- ~~Does Tyler want dogfooding folded into Phase 7 or left as a follow-up?~~ **Resolved**: folded into Phase 7 — see updated Scope section above.

**New open items surfaced by research** (for Phase 3 to resolve, not blocking):
- Kotlin's import-statement tree-sitter node (`import`, confirmed via the actual pinned grammar's repo, not the older `import_header` from a different Kotlin grammar) has no bundled query examples anywhere — kibitzer would be first to write this extraction query. Java's `import_declaration` is also positional-only (no named fields). Both need real `to_sexp()` fixture verification in Phase 5, not assumed node shapes — flagged consistently by 2 of 6 research agents.
- Recommended architecture: content/naming rules get a **separate third trait+registry** (`DeclarationChecker`/`DeclarationGraph`) rather than widening `ArchitectureChecker`'s signature — keeps `ImportCycleChecker`/`LayeringChecker`/`CouplingChecker` literally unmodified. See `research/architecture.md`.
