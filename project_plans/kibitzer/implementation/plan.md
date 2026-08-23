# Implementation Plan: kibitzer — Native Architecture Linting

**Feature**: Native, in-process dependency/content/naming architecture rules (replacing
depguard/go-arch-lint/arch-go/cht-go-lint for Tyler's projects) plus Python/Java/Kotlin
import-graph coverage, built on kibitzer's existing `ArchitectureChecker`/`ImportGraph` stack.
**Date**: 2026-08-22
**Status**: Ready for implementation
**ADRs**: [ADR-001: content/naming rules get a third checker trait, not a widened `ArchitectureChecker`](../decisions/ADR-001-declaration-checker-third-trait.md)
**Total Appetite**: ~26–34 hours across 8 phases. Sized as one feature-branch effort merged in
phase order (each phase is independently mergeable/revertable — see Sequencing below); not a
commitment to ship all 4 gaps in one PR if time runs out after Phase 3.

| Phase | Gap | Size | Hours |
|---|---|---|---|
| 0 | Schema foundation (blocks all others) | S | 2–3 |
| 1 | Gap 1: dependency rules | M | 4–5 |
| 2 | Gap 2: content rules (+ decl. infra, Go/JS) | M/L | 5–7 |
| 3 | Gap 3: naming rules (v1, scope-narrowed) | S | 2–3 |
| 4 | Gap 4a: Java + Kotlin import/decl coverage | M (highest risk: Kotlin has no reference queries) | 4–5 |
| 5 | Gap 4b: Python import/decl coverage | M | 3–4 |
| 6 | UX polish (bracket-prefix, counts, mermaid) | S | 2 |
| 7 | Dogfooding (synthetic fixture + real `stapler-squad` migration proof) | S/M | 3.5–4.5 |

**Sequencing rationale**: Phase 0 locks the `Component`/`DependencyRule`/`ContentRule`/`NamingRule`
schema *together*, per pitfalls.md's sequencing warning — Phase 0's Story 0.1.1 explicitly
sketches all three rule shapes before any checker is implemented, so Phase 1 doesn't lock a
schema that Phase 2/3 then have to revise. Gaps are then implemented in dependency order
(schema → dependency rules → content rules → naming rules, since naming rules reuse content
rules' `DeclarationGraph`), then the two riskiest, most self-contained gaps (multi-language
extraction) last, ordered Java/Kotlin-before-Python because Java/Kotlin share one generic
resolver (architecture.md §5) while Python needs a bespoke one — cheaper gap first. UX polish
and dogfooding close out once every rule category exists to demonstrate.

---

## Domain Glossary

| Term | Definition | Notes |
|------|-----------|-------|
| `ArchitectureChecker` | Existing trait: `check(&ImportGraph, &ArchitectureConfig) -> Vec<ArchFinding>`. | Unmodified. `src/architecture_checks.rs:19`. |
| `ArchFinding` | Existing finding shape (`file: Option<PathBuf>`, `line: Option<usize>`, `message: String`). *Gains* `severity_override: Option<Severity>` (Story 1.1.3, fixing design/ux.md's headline gap) — set unconditionally to `Some(Severity::Advisory)` by every zero-match advisory (component-glob, naming-rule), so a zero-match finding can never render as `[blocking]` regardless of the configuring `Check`'s severity. | Additive/`#[serde(default)]`; otherwise reused unmodified by every checker. `src/architecture_checks.rs`. |
| `CheckResult.findings` | *(new field, Story 1.1.3)* `Vec<ArchFinding>` populated by `run_architecture_check`, empty for non-architecture-check results (e.g. `SYNTAX_RULES_CHECKERS`) — gives `mcp.rs` per-finding severity data instead of only the flattened `output: String`. | `#[serde(default)]`, cached in `cache.json` per the existing `command`-field precedent (`src/check.rs:26-32`). `src/check.rs`. |
| `zero_match_advisory<T>` | *(new, Story 1.1.3)* Shared free function auditing a declared-item slice (`Component`s or `NamingRule`s) against whichever graph the caller has, producing one advisory `ArchFinding` per zero-match item. Reused by `ComponentDependencyChecker` (Phase 1, against `ImportGraph`) and `ContentChecker`/`NamingChecker` (Phase 2/3, against `DeclarationGraph`) — replaces the previously-duplicated, `ContentChecker`-excluded per-checker logic (architecture-review.md; design/ux.md gap #2). | `src/architecture_checks.rs`. |
| `ImportGraph` | Existing directory/package-granularity graph of import edges. | `src/import_graph.rs:22`. |
| `Component` | *(new)* A named, glob-mapped set of graph-node/file-path identifiers (`name`, `paths: Vec<String>`), the unit dependency/content/naming rules reference. | `src/config.rs`. Value Object. |
| `DependencyRule` | *(new)* Per-component allow-list (`may_depend_on: Option<Vec<String>>`) and/or deny-list (`deny_depend_on: Vec<String>`) of other component names. | `src/config.rs`. |
| `ContentRule` | *(new)* Per-component allowed-declaration-kind list (`allowed_kinds: Vec<String>`), e.g. "domain may only contain struct". | `src/config.rs`. |
| `NamingRule` | *(new)* Per-component, per-`DeclKind` regex pattern a declaration's name must match. | `src/config.rs`, v1 is component/glob-conditioned only (no interface-implementation matching — see Unresolved). |
| `ComponentDependencyChecker` | *(new)* `ArchitectureChecker` impl (`name() == "component-deps"`) evaluating `DependencyRule`s over `ImportGraph` edges. | `src/architecture_checks.rs`. |
| `DeclKind` | *(new)* Enum of declaration kinds content/naming rules can reference: `Struct`, `Class`, `Interface`, `Enum`, `Function`. | `src/declarations.rs`. |
| `Declaration` | *(new)* One extracted top-level declaration: `name`, `kind: DeclKind`, `file`, `line`, `component: Option<String>`. | `src/declarations.rs`. |
| `DeclarationGraph` | *(new)* Whole-repo collection of `Declaration`s, built once per batch run, analogous to `ImportGraph` but for declarations rather than imports. | `src/declarations.rs`. |
| `DeclarationChecker` | *(new)* Third checker trait: `check(&DeclarationGraph, &ArchitectureConfig) -> Vec<ArchFinding>`. Own registry/lookup, parallel to `ArchitectureChecker`. | `src/declaration_checks.rs`. See ADR-001. |
| `ContentChecker` | *(new)* `DeclarationChecker` impl (`name() == "content-rules"`) evaluating `ContentRule`s. | `src/declaration_checks.rs`. |
| `NamingChecker` | *(new)* `DeclarationChecker` impl (`name() == "naming-rules"`) evaluating `NamingRule`s. | `src/declaration_checks.rs`. |
| `AnyArchitectureChecker` | *(new)* Thin enum (`Import(Box<dyn ArchitectureChecker>)` / `Declaration(Box<dyn DeclarationChecker>)`) merging both registries at three dispatch sites: `config.rs::validate()`, `check.rs::run_architecture_check` (Story 1.2.1), and — once Phase 2's `declaration_checks` registry exists — `check.rs::check_native_against_git_head_repo` (Story 2.2.3, fixing adversarial-review.md's BLOCKER: the git-HEAD-baseline downgrade path only looked up `ArchitectureChecker`s and silently never downgraded a blocking `content-rules`/`naming-rules` violation). | `src/check.rs`. |
| `component_of` | *(new)* Free function: first-declared-glob-match-wins resolver from a graph-node/file-path string to a `Component.name`, mirroring `layer_of()`'s `.position()` semantics. | `src/config.rs`. |
| `effective_components` / `effective_dependency_rules` | *(new)* `ArchitectureConfig` accessor methods that append the `layers`-desugared `Component`/`DependencyRule` set to any explicitly declared ones — the backward-compat seam. | `src/config.rs`. |
| `normalize_package_identity` | *(new)* Converts a dot-separated package identity (Java `com.example.domain`, Kotlin, Python) to `/`-separated, so `Component.paths` globs (which use `/`-segment semantics) work uniformly across all 6 languages. | `src/import_graph.rs`. |
| `QualifiedImportLangConfig` | *(new)* Per-language table (mirrors `rules.rs::LangRuleConfig`) parameterizing the shared Go/Java/Kotlin "self/manifest-declared qualified identity" import-resolution family. | `src/import_graph.rs`. |
| `build_qualified_name_language` | *(new)* One generic import-extraction function driven by `QualifiedImportLangConfig`, replacing `build_go`'s hand-written loop and reused for Java/Kotlin. | `src/import_graph.rs`. |
| `build_python` | *(new)* Bespoke Python import extractor (relative-dot-counting + `__init__.py`-rooted absolute resolution) — doesn't fit the qualified-name family. | `src/import_graph.rs`. |

---

## Pattern Decisions

| Component | Pattern Chosen | Source | Alternative Rejected | Reason |
|---|---|---|---|---|
| Content/naming rules dispatch | Third trait + registry (`DeclarationChecker`) | architecture.md §4, adopted as-is (see ADR-001) | (a) Widen `ArchitectureChecker::check()`'s signature to take both graphs; (b) fold declaration walking into `import_graph.rs` | (a) touches all 3 existing checkers' call sites, violating "existing checkers continue to work unmodified" more literally than a parallel trait; (b) conflates two different resolution algorithms (import-path resolution vs. declaration enumeration) and two different graph granularities (package vs. per-file) into one module |
| Rule-expression shape | Named `Component{name, paths}` + `DependencyRule{component, may_depend_on, deny_depend_on}` | architecture.md §2 / build-vs-buy.md §3, UX research §1 (all three independently converge) | (a) depguard's file-glob-deny/allow (no named-component indirection); (b) arch-go's inline-glob-per-rule (no reusable identity) | (a)/(b) both conflate "what is this component" with "what can it depend on," and neither gives content/naming rules (Gap 2/3) a name to reference — go-arch-lint's separation composes across all 3 new rule categories |
| `layers` backward compat | Desugar via `effective_components()`/`effective_dependency_rules()` accessor methods, `layers` field/`LayeringChecker` untouched | architecture.md §3 | Rewrite `LayeringChecker` to internally call the new component model | Rewriting risks behavior drift on the one thing requirements.md's Success Metrics demands byte-for-byte (`layers` findings identical) — desugar-as-a-parallel-accessor keeps the old code path provably unchanged |
| Component→node resolution | First-declared-glob-match wins | architecture.md §2 (mirrors `layer_of()`), pitfalls.md §2 | (a) most-specific-glob wins; (b) config-time overlap-rejection error | `layer_of()` is kibitzer's own tested precedent for this exact ambiguity class (exact-name-match, not glob, but same "first in declaration order" resolution); (b) requires a general glob-overlap detector (two globs can overlap on paths that don't exist yet) — real algorithmic scope not requested by any research doc |
| Dependency-rule default (component declared, no explicit `DependencyRule`) | Deny-by-default (component may depend only on itself) | UX research §1 ("go-arch-lint's answer... deny-by-default, fails closed... more defensible default for kibitzer too") | Allow-by-default / unconstrained (matching `layer_of()`'s "no match = ignored") | `layer_of()`'s "ignored" precedent is about a *node not matching any component at all* (kept, unchanged, §"Component→node resolution" above) — a *declared* component with zero rules is a different case UX research addresses directly; advisory tooling meant to catch drift shouldn't silently pass an unconfigured component |
| Allow vs. deny precedence (both apply to the same target) | Deny wins | pitfalls.md §2 ("pick and document... e.g. deny always wins over allow"), matches depguard | Allow wins / undefined | Matches the one concretely-named external-tool precedent (depguard) research found; "undefined" fails the "no completion claim without proof" bar this plan is held to |
| Naming-rule scope (v1) | Component/glob-conditioned only (`NamingRule{component, kind, pattern}`) — no `Implements(interface)` scope | features.md §2, architecture.md §4 ("flag for Phase 3 sizing... document as a limitation") | Ship `Implements(X)` interface-conditioned naming (arch-go parity) in v1 | Go interface satisfaction is structural and cross-file (architecture.md §4) — correctly resolving "does struct S implement interface I" needs a whole-package method-set index kibitzer has no equivalent of yet; the multi-interface-conflict case has "no precedent at all" (features.md §2). Scope-narrowing this out of v1 is explicitly offered as legitimate by two research docs |
| Per-rule severity | Deferred (not in v1) | features.md §4 flags as "likely wanted," not required | Add `severity: Option<Severity>` to `DependencyRule`/`ContentRule`/`NamingRule` now | Requires new pass/fail aggregation logic (which per-rule severity wins the outer `Check.severity`-driven pass/fail) not accounted for anywhere else in `check.rs`; not named in requirements.md's Success Metrics; adding it now risks the schema-lock-in Phase 0 is trying to avoid on a dimension nobody asked for yet |
| Java/Kotlin/Python graph-node identity | Normalize dot-separated package identity to `/`-separated at graph-build time (`normalize_package_identity`) | New catch, this plan (not in prior research) | Leave Java/Kotlin identity dot-separated (`com.example.domain`), require users to write dot-separated component globs | `glob_to_regex`/`matches_scope` is `/`-segment-aware only (`**/domain/**`) — a dot-separated identity string has *zero* `/` characters, so **no glob pattern in the new schema could ever match a Java/Kotlin package** without this normalization. This is a correctness bug in the schema as described by architecture.md if left unaddressed — caught during this planning pass, see Unresolved Questions |
| Zero-match advisory severity (design/ux.md's headline gap: zero-match findings could render `[blocking]`) | Option (a): `ArchFinding.severity_override: Option<Severity>`, set unconditionally to `Advisory` by every zero-match finding; `CheckResult` gains a parallel `findings: Vec<ArchFinding>` field so `mcp.rs` reads per-finding severity instead of only the flattened `output: String` (Story 1.1.3) | design/ux.md, "Verified, headline gap" section | (b) Split zero-match findings into a separate always-advisory output channel/section | (a) keeps `ArchFinding` as the one finding shape every checker returns (matches UX AC 9: "no new finding shape"), and reuses two existing precedents — `Severity` already derives `Serialize`/`Deserialize` (`src/config.rs:9`), and `CheckResult` already caches additive `#[serde(default)]` fields to `cache.json` (the `command` field, `src/check.rs:26-32`); (b) would need a second finding vector threaded through every checker's return type and every call site that already assumes one `Vec<ArchFinding>`, a larger diff for the same outcome |
| Git-HEAD-baseline downgrade for Declaration-kind checkers | `check_native_against_git_head_repo` dispatches through `lookup_any_architecture_checker` (Story 1.2.1) and branches on `AnyArchitectureChecker::Import`/`Declaration`, building the matching graph type (`ImportGraph` or `DeclarationGraph`) against the git-HEAD snapshot (Story 2.2.3) | adversarial-review.md's BLOCKER finding | Leave `content-rules`/`naming-rules` without the HEAD-baseline downgrade (accept the inconsistency) | Without this, a blocking `content-rules`/`naming-rules` check always blocks on any violation, including ones that predate the current edit — unlike every other native architecture checker. `component-deps` (Import-kind, Phase 1) already gets this correctly today since `architecture_checks::lookup` already covers it — confirmed by reading `src/check.rs:867-931` directly — so only the two Declaration-kind checkers, which don't exist until Phase 2/3, needed a fix; it lands in Phase 2 (Story 2.2.3) once `declaration_checks::registry()` is real |
| Dependency-rule edge-target validity ("graph-membership guard") | `ComponentDependencyChecker::check()` requires `graph.nodes.contains(&edge.to) && graph.nodes.contains(&edge.from)` before resolving either side via `component_of()` — an explicit, checker-level re-assertion of the invariant `build_go`/`build_js`/`build_qualified_name_language` already establish at graph-construction time (an edge only ever targets a node built from a walked repo-local file) | pre-mortem.md P1 #3(i) | Trust that every `ImportGraph` builder (present and future, including Phase 4/5's language extractors) will always preserve that invariant, with no independent check at the consumer | A consumer-side check is one extra `.contains()` per edge and fails safe if a future extractor bug ever lets an edge target an unresolved/external path — without it, a `Component.paths` glob that happens to textually match a third-party import path's segment (e.g. `**/infra/**` matching `some-vendor/infra-client`) would silently produce a false-positive `component-deps` violation against code that was never part of the project |
| Java-import positional walk | `kind()`-filtered positional child walk (no field names) | pitfalls.md §1 / build-vs-buy.md §4 (verified via real `node-types.json`) | Reuse Go/JS's `child_by_field_name("path"/"source")` pattern | `import_declaration` "declares no named fields at all" — confirmed via the grammar's own schema, not assumed by analogy |
| Kotlin-import walk | Same `kind()`-filtered positional walk, node kind `"import"` (not `"import_header"`) | build-vs-buy.md §4 (verified via `gh api` against the actual pinned `tree-sitter-grammars/tree-sitter-kotlin`) | Trust the `fwcd/tree-sitter-kotlin` fork's `import_header` node name (an older, different grammar) | Two research passes disagreed until the pinned grammar's real source was fetched — this plan uses the verified answer, not the first guess |

---

## Migration Plan

Additive-only. No existing config field changes shape or meaning. `layers: Vec<String>` continues
to parse exactly as today (`#[serde(default)]`, unchanged struct field) — the new `components`,
`dependency_rules`, `content_rules`, `naming_rules` fields are all `#[serde(default)]` `Vec`s that
default to empty, so every existing `.claude/inspect.json` in Tyler's other repos parses unchanged
and (per the Phase 1 golden-regression tests) produces byte-identical `LayeringChecker` findings.
No data migration, no config-file rewrite tooling needed.

## Observability Plan
- **Logs**: None new — architecture findings flow through the same `{file}:{line}: {message}` /
  `ArchFinding` convention every other checker uses (requirements.md's Observability Requirements
  section already settles this as sufficient).
- **Metrics**: None — local CLI/MCP tool, no running service.
- **Alerts**: None.

## Risk Control
- **Feature flag**: None — new `architecture_checker` names (`component-deps`, `content-rules`,
  `naming-rules`) are opt-in by construction: a project's `.claude/inspect.json` must explicitly
  add a `Check` referencing one. No existing check name changes behavior.
- **Rollback procedure**: Revert the release tag (per requirements.md's Risk Control section,
  already the accepted procedure for this repo). Each phase below is an independently revertable
  commit range.
- **Staged rollout**: None needed (personal OSS tool, no deployment).

## Unresolved Questions

1. **Requirements.md's exact CLI phrasing (`kibitzer check native <checker> <path>`) doesn't fit
   whole-repo checkers.** `checker::registry()`/`CheckCommand::Native` (`src/main.rs:68`) is the
   *per-file* `Checker` trait's CLI verb; the new `component-deps`/`content-rules`/`naming-rules`
   checkers are whole-repo (`ArchitectureChecker`/`DeclarationChecker`), like `import-cycles` and
   `layering` already are. Those two don't have a CLI verb today either — `check.rs::run_architecture_check`
   builds a `cmd_str` of `"kibitzer check architecture {arch_name}"` for display purposes
   (`src/check.rs:797`) that **no such subcommand currently exists** to run directly; verification
   has only ever gone through `kibitzer run <dir> --trigger batch` or the `architecture_assessment`
   MCP tool. **Resolution adopted in this plan**: Phase 1 (Story 1.2.2 below) adds the missing
   `kibitzer check architecture <name> <dir>` subcommand — it was already implied by `run_architecture_check`'s
   own `cmd_str`, and gives every new checker (including the two `DeclarationChecker`s) a direct,
   testable CLI entry point, satisfying the spirit of requirements.md's success metric even though
   the literal command name/shape it wrote doesn't exist as described.
2. **Dogfooding target cannot literally be kibitzer's own Rust source.** Verified via
   `git ls-files` against `/home/tstapler/code/github.com/tstapler/kibitzer`: the repo is 100% Rust
   (plus docs/assets) — zero `.go`/`.ts`/`.py`/`.java`/`.kt` files exist anywhere in it. Rust
   import/declaration extraction is explicitly listed as Out of Scope in requirements.md. This
   means `ImportGraph::build()`/`DeclarationGraph::build()` would produce **zero nodes** against
   kibitzer's actual source tree, regardless of how this feature is implemented — none of the new
   rule categories could ever fire there. Requirements.md's Phase 7 scope note ("adds a minimal
   `.claude/inspect.json` to the kibitzer repo itself") and architecture.md's forward-looking
   Kibitzer-module-as-components sketch both implicitly assumed Rust coverage existed or would be
   added — an oversight not caught until this planning pass. **Resolution adopted in this plan**
   (Phase 7 below): the `.claude/inspect.json` lives at the kibitzer repo root (literally "in the
   kibitzer repo," satisfying the literal requirement) but its new architecture checks are `scope`-restricted
   to a small checked-in Go fixture project at `testdata/dogfood-architecture/` — exercising all
   three new rule categories against real Go source using the already-mature Go extractor. This is
   a deliberate deviation from a literal "kibitzer's own module layout as components" reading;
   flagged here rather than silently substituted.
3. **Whether `docs/check-ideas.md` should get a note that this feature isn't evidenced-need-backed**
   (features.md §4's explicit caution against overclaiming this in Phase 4/validate) — not this
   plan's job to resolve, carried forward for whoever runs Phase 4.
4. **Post-merge overlap between `layering` and `component-deps`.** Nothing prevents a config
   registering both, double-reporting the same violation (architecture.md §3). No enforcement
   added (matches architecture.md's own "no enforcement needed at parse time" call) — worth a
   `docs/` note recommending `component-deps` supersede `layering` for new configs, but writing
   that doc is out of this plan's scope (no doc file named in requirements.md's Scope).

## Post-Ship Follow-up (Requires User Input)

**Real-repo adoption proof — RESOLVED 2026-08-23.** (pre-mortem.md P1 #5)

This section originally posed an open question for Tyler ("which real repo, if any, currently uses
`go-arch-lint`/`depguard`/`arch-go`, and would you migrate it as the adoption proof?"). Tyler answered
it directly: `tstapler/stapler-squad` — a real, actively-maintained Go project with a real
`depguard` config in `.golangci.yml` (3 rules, fetched via `gh api
repos/tstapler/stapler-squad/contents/.golangci.yml`). That answer is no longer an open question —
it is now **Epic 7.2** below (`stapler-squad` depguard migration), landed alongside the synthetic
`testdata/dogfood-architecture/` fixture (Epic 7.1), not in place of it. The synthetic fixture still
earns its place: kibitzer's own CI checks out kibitzer's own repo, and a real external repo can't be
a hermetic, checked-in test fixture for that — see Epic 7.1's rationale, unchanged.

## Dependency Visualization

```
Phase 0 (schema foundation)
  Component/DependencyRule/ContentRule/NamingRule types + effective_*() + component_of()
        │
        ├──────────────────────────────┬───────────────────────────────┐
        ▼                               ▼                               │
  Phase 1 (Gap 1: deps)          Phase 2 (Gap 2: content)               │
  ComponentDependencyChecker     DeclarationGraph + ContentChecker      │
  + AnyArchitectureChecker       (needs Phase 0's Component type        │
    dispatch + CLI verb           for Declaration.component)            │
  + zero_match_advisory<T>       + Story 2.2.3: extends the git-HEAD-   │
    (already HEAD-baseline-        baseline downgrade (Story 1.2.1) to  │
    downgradeable — Phase 1's      Declaration-kind checkers, closing   │
    checkers all lived in the      the BLOCKER — content-rules/naming-  │
    pre-existing registry)         rules only get correct downgrade     │
                                    behavior once this story lands      │
        │                               │                               │
        │                               ▼                               │
        │                        Phase 3 (Gap 3: naming)                │
        │                        NamingChecker (reuses Phase 2's        │
        │                        DeclarationGraph + declaration_checks  │
        │                        registry)                              │
        │                               │                               │
        └───────────────┬───────────────┘                               │
                         ▼                                               │
              Phase 4 (Gap 4a: Java/Kotlin)         Phase 5 (Gap 4b: Python)
              extends import_graph.rs +              extends import_graph.rs +
              declarations.rs dispatch                declarations.rs dispatch
              (needs Phase 1's ImportGraph            (independent of Phase 4,
               consumer + Phase 2's decl              can run in parallel with it)
               extraction pattern to extend)
                         │                                               │
                         └───────────────┬───────────────────────────────┘
                                         ▼
                              Phase 6 (UX polish)
                              bracket-prefix, counts, recommendation_for,
                              mermaid subgraphs — touches mcp.rs, cutting
                              across every checker from Phases 1–5
                                         │
                                         ▼
                              Phase 7 (Dogfooding)
                              testdata/dogfood-architecture/ + .claude/inspect.json
                              exercising component-deps + content-rules + naming-rules
```

---

## Phase 0: Schema Foundation

### Epic 0.1: Rule Schema & Config Plumbing
**Goal**: Lock the `Component`/`DependencyRule`/`ContentRule`/`NamingRule` types and the
`layers`-desugar accessors together, before any checker consumes them — the pitfalls.md
sequencing-risk mitigation. No checker logic yet; this epic only adds types, parses them, and
validates references.

#### Story 0.1.1: Add `Component`/`DependencyRule`/`ContentRule`/`NamingRule` types to `ArchitectureConfig`
**As a** kibitzer config author, **I want** to declare named components and rules referencing them
in `.claude/inspect.json`, **so that** later checkers (Phases 1–3) have a schema to consume.
**Acceptance Criteria**:
- `ArchitectureConfig` gains `components: Vec<Component>`, `dependency_rules: Vec<DependencyRule>`,
  `content_rules: Vec<ContentRule>`, `naming_rules: Vec<NamingRule>`, all `#[serde(default)]`.
  - *Given* the JSON `{"architecture": {"layers": ["domain","infra"]}}` (no new fields present),
    *When* it's parsed via `serde_json::from_str::<Config>`, *Then* `config.architecture.components`,
    `.dependency_rules`, `.content_rules`, `.naming_rules` are all empty `Vec`s and
    `config.architecture.layers == vec!["domain", "infra"]` — unchanged from today.
- A `components` entry parses `name` and `paths`.
  - *Given* `{"architecture": {"components": [{"name": "domain", "paths": ["**/domain", "**/domain/**"]}]}}`,
    *When* parsed, *Then* `config.architecture.components[0].name == "domain"` and
    `.paths == vec!["**/domain".to_string(), "**/domain/**".to_string()]`.
**Files**: `src/config.rs`

##### Task 0.1.1a: Add `Component` struct (~3 min)
- Add `#[derive(Debug, Clone, Deserialize)] pub struct Component { pub name: String, #[serde(default)] pub paths: Vec<String> }` above `ArchitectureConfig` in `src/config.rs`.
- Files: `src/config.rs`

##### Task 0.1.1b: Add `DependencyRule` struct (~3 min)
- Add `#[derive(Debug, Clone, Default, Deserialize)] pub struct DependencyRule { pub component: String, #[serde(default)] pub may_depend_on: Option<Vec<String>>, #[serde(default)] pub deny_depend_on: Vec<String> }`.
- Files: `src/config.rs`

##### Task 0.1.1c: Add `ContentRule` and `NamingRule` structs (~4 min)
- Add `pub struct ContentRule { pub component: String, pub allowed_kinds: Vec<String> }` and
  `pub struct NamingRule { pub component: String, pub kind: String, pub pattern: String }`, both
  `#[derive(Debug, Clone, Deserialize)]`.
- Files: `src/config.rs`

##### Task 0.1.1d: Extend `ArchitectureConfig` with the four new `#[serde(default)]` fields (~3 min)
- Add `components`, `dependency_rules`, `content_rules`, `naming_rules` fields with doc comments
  cross-referencing `layers` (note it desugars into these — see Story 0.1.2).
- Files: `src/config.rs`

##### Task 0.1.1e: Unit tests for new-field parsing and backward-compat (~5 min)
- Add `component_parses_name_and_paths`, `existing_layers_only_config_still_parses_with_empty_new_fields`
  tests per the Given/When/Then above.
- Files: `src/config.rs`

#### Story 0.1.2: `layers` desugar accessors + `component_of` resolver
**As a** checker implementer, **I want** `ArchitectureConfig::effective_components()`/
`effective_dependency_rules()` and a shared `component_of()` resolver, **so that** every new
checker (Phases 1–3) gets `layers` backward-compat for free and shares one node-to-component
resolution algorithm.
**Acceptance Criteria**:
- `effective_components()` on `layers: vec!["handlers", "domain", "infra"]` (no explicit
  `components`) desugars each layer into 4 glob patterns reproducing `layer_of()`'s
  "segment anywhere" exact-match semantics.
  - *Given* `ArchitectureConfig { layers: vec!["handlers".into(), "domain".into(), "infra".into()], ..Default::default() }`,
    *When* `.effective_components()` is called, *Then* the result contains
    `Component { name: "domain".into(), paths: vec!["domain".into(), "domain/**".into(), "**/domain".into(), "**/domain/**".into()] }`.
- `effective_dependency_rules()` produces one rule per layer (not one per pair), with
  `may_depend_on = Some(layers[i..].to_vec())`.
  - *Given* the same 3-layer config, *When* `.effective_dependency_rules()` is called, *Then* the
    rule for `component: "domain"` (index 1) has `may_depend_on == Some(vec!["domain".into(), "infra".into()])`.
- `component_of(node, components)` returns the first component (in declaration order) whose
  `paths` glob-matches `node`, or `None`.
  - *Given* `components = [Component{name:"handlers",paths:vec!["**/handlers".into()]}, Component{name:"domain",paths:vec!["**/domain".into()]}]`
    and `node = "dogfood.example/app/domain"`, *When* `component_of(node, &components)` is called,
    *Then* it returns `Some("domain")`.
  - *Given* `node = "dogfood.example/app/vendor"` (matches no declared component),
    *When* `component_of(node, &components)` is called, *Then* it returns `None` (ignored, matching
    `layer_of()`'s existing "no match = ignored" precedent — see Pattern Decisions).
- Declaring both `layers: ["domain", ...]` and an explicit `Component{name: "domain", ...}` is a
  hard config-load error (name collision).
  - *Given* `{"architecture": {"layers": ["domain"], "components": [{"name": "domain", "paths": ["x/**"]}]}}`,
    *When* `find_config`/`validate` runs, *Then* it returns an `Err` whose message contains
    `"component 'domain' is declared both explicitly and via 'layers'"`.
**Files**: `src/config.rs`

##### Task 0.1.2a: `desugar_layers_to_components` free function (~5 min)
- `fn desugar_layers_to_components(layers: &[String]) -> Vec<Component>` — one `Component` per
  layer name with the 4-glob-pattern expansion from the acceptance criterion above.
- Files: `src/config.rs`

##### Task 0.1.2b: `desugar_layers_to_rules` free function (~4 min)
- `fn desugar_layers_to_rules(layers: &[String]) -> Vec<DependencyRule>` — for layer at index `i`,
  `DependencyRule { component: layers[i].clone(), may_depend_on: Some(layers[i..].to_vec()), deny_depend_on: vec![] }`.
- Files: `src/config.rs`

##### Task 0.1.2c: `effective_components`/`effective_dependency_rules` methods on `ArchitectureConfig` (~4 min)
- `impl ArchitectureConfig { pub fn effective_components(&self) -> Vec<Component> { self.components.iter().cloned().chain(desugar_layers_to_components(&self.layers)).collect() } }` and the
  `effective_dependency_rules` equivalent.
- Files: `src/config.rs`

##### Task 0.1.2d: `component_of` free function + `matches_scope` reuse (~4 min)
- `pub fn component_of<'a>(node: &str, components: &'a [Component]) -> Option<&'a str> { components.iter().find(|c| crate::glob::matches_scope(node, &c.paths)).map(|c| c.name.as_str()) }`.
- Files: `src/config.rs`

##### Task 0.1.2e: Layers/components name-collision validation (~4 min)
- In `validate()`, before other checks: for each `layer` in `config.architecture.layers`, if any
  `config.architecture.components` entry has the same `name`, bail with the message from the
  acceptance criterion.
- Files: `src/config.rs`

##### Task 0.1.2f: Unit tests for desugar + collision + `component_of` (~5 min)
- Add `layers_desugar_to_four_glob_patterns_per_layer`, `layers_desugar_to_suffix_allow_lists_not_pairwise`,
  `component_of_returns_first_declaration_order_match`, `component_of_returns_none_for_unmatched_node`,
  `rejects_layer_and_component_name_collision` per the Given/When/Then above.
- Files: `src/config.rs`

#### Story 0.1.3: Validate rule references (unknown component, "did you mean")
**As a** kibitzer config author, **I want** a typo'd component reference in a `dependency_rules`/
`content_rules`/`naming_rules` entry to fail fast at config-load time, **so that** I don't silently
get zero enforcement.
**Acceptance Criteria**:
- A `DependencyRule.component` (or a name inside `may_depend_on`/`deny_depend_on`) not matching any
  declared `Component.name` is a hard error with a Levenshtein-distance-≤2 suggestion.
  - *Given* `{"architecture": {"components": [{"name":"handlers","paths":["**/handlers"]}], "dependency_rules": [{"component": "hanlders", "may_depend_on": []}]}}`,
    *When* `find_config` runs, *Then* it returns an `Err` whose message equals (module the leading
    `{config_path}: `) `"architecture rule references undefined component 'hanlders' — declared components are: handlers (did you mean 'handlers'?)"`.
- Same treatment for `ContentRule.component` and `NamingRule.component`.
  - *Given* `{"architecture": {"components": [{"name":"domain","paths":["**/domain"]}], "content_rules": [{"component": "doamin", "allowed_kinds": ["struct"]}]}}`,
    *When* `find_config` runs, *Then* it returns an `Err` containing `"undefined component 'doamin'"` and `"(did you mean 'domain'?)"`.
**Files**: `src/config.rs`

##### Task 0.1.3a: Hand-rolled Levenshtein distance helper (~5 min)
- `fn levenshtein(a: &str, b: &str) -> usize` — classic DP table, no crate (matches build-vs-buy's
  "no new dependency" recommendation for hand-rolled small algorithms).
- Files: `src/config.rs`

##### Task 0.1.3b: `validate_component_references` helper + wire into `validate()` (~5 min)
- New function collecting every component name referenced across `dependency_rules` (`component`,
  `may_depend_on` entries, `deny_depend_on` entries), `content_rules`, `naming_rules`; for each not
  present in `effective_components()`'s names, bail with the message shape above, using
  `levenshtein` to pick a same-or-closer suggestion (distance ≤ 2, else omit the "(did you mean...)" clause).
- Files: `src/config.rs`

##### Task 0.1.3c: Unit tests for unknown-reference errors (~4 min)
- `rejects_unknown_component_in_dependency_rule`, `rejects_unknown_component_in_content_rule`,
  `rejects_unknown_component_in_naming_rule`, `unknown_component_error_omits_suggestion_when_no_close_match`.
- Files: `src/config.rs`

---

## Phase 1: Gap 1 — Dependency Rules

### Epic 1.1: `ComponentDependencyChecker`
**Goal**: A new `ArchitectureChecker` implementing arbitrary named-component allow/deny rules,
registered alongside (not replacing) `ImportCycleChecker`/`LayeringChecker`/`CouplingChecker`.

#### Story 1.1.1: Implement and register `ComponentDependencyChecker`
**As a** kibitzer user, **I want** a `"component-deps"` architecture checker expressing rules an
ordered `layers` list can't (e.g. "services must not import `net/http`"), **so that** I get
depguard/go-arch-lint-equivalent enforcement natively.
**Acceptance Criteria**:
- An edge between two different components violates a closed-world `may_depend_on` allow-list.
  - *Given* `graph.edges = [ImportEdge{from: "dogfood.example/app/domain", to: "dogfood.example/app/infra", file: "domain/domain.go", line: 5}]`,
    `components = [Component{name:"domain",paths:vec!["**/domain".into()]}, Component{name:"infra",paths:vec!["**/infra".into()]}]`,
    `dependency_rules = [DependencyRule{component:"domain".into(), may_depend_on: Some(vec!["domain".into()]), deny_depend_on: vec![]}]`,
    *When* `ComponentDependencyChecker.check(&graph, &config)` runs, *Then* it returns exactly one
    `ArchFinding` with `file: Some("domain/domain.go".into())`, `line: Some(5)`, and
    `message == "[component-deps] domain (dogfood.example/app/domain) imports infra (dogfood.example/app/infra) — 'domain' may depend on: domain"`.
- A `deny_depend_on` match is a violation even if the same target also appears in `may_depend_on` (deny wins).
  - *Given* the same edge, `dependency_rules = [DependencyRule{component:"domain".into(), may_depend_on: Some(vec!["infra".into()]), deny_depend_on: vec!["infra".into()]}]`,
    *When* checked, *Then* it still returns one finding — deny-wins precedence overrides the
    conflicting allow entry.
- A component declared but with no `DependencyRule` entry defaults to deny (may only depend on itself).
  - *Given* `components = [Component{name:"domain",...}, Component{name:"infra",...}]`, `dependency_rules = []`,
    and the same edge, *When* checked, *Then* it returns one finding (deny-by-default, per Pattern Decisions).
- Same-component edges are never violations.
  - *Given* an edge where `component_of(from) == component_of(to) == Some("domain")`,
    *When* checked, *Then* no finding is produced regardless of any declared rule.
- An edge where either endpoint matches no component is ignored (existing precedent, unchanged).
  - *Given* `graph.edges = [edge("dogfood.example/app/domain", "fmt")]` and no component matches `"fmt"`,
    *When* checked, *Then* no finding is produced.
- **(pre-mortem.md P1 #3(i))** An edge whose target string is not itself an actual `graph.nodes`
  entry — i.e. wasn't built from a walked repo-local file, such as an external/third-party import —
  is never treated as a dependency-rule violation, even when a declared `Component.paths` glob
  syntactically matches the string. Verified against `src/import_graph.rs`: `build_go` only inserts
  a graph node for a resolved local package (`graph.nodes.insert(pkg.clone())`, line 113) and only
  ever pushes an edge when `graph.nodes.contains(&import_path)` already holds (line 134); `build_js`
  likewise only inserts local directories as nodes (line 220) and only resolves edges for relative
  (`./`/`../`) specifiers against `known_files`, skipping bare/package specifiers outright (lines
  240-242) — so today, an edge's `to` is *already* guaranteed to be a real graph node by construction
  for both existing extractors. This criterion makes that invariant an explicit, independently-
  checked property of `ComponentDependencyChecker` itself (defense-in-depth), rather than an implicit
  assumption a future language extractor (Phase 4/5's `build_qualified_name_language`) could silently
  break.
  - *Given* `components = [Component{name:"infra", paths: vec!["**/infra/**".into()]}]`,
    `dependency_rules = [DependencyRule{component:"domain".into(), may_depend_on: Some(vec!["domain".into()]), deny_depend_on: vec![]}]`,
    and `graph = ImportGraph{nodes: BTreeSet::from(["dogfood.example/app/domain".into()]), edges: vec![ImportEdge{from: "dogfood.example/app/domain".into(), to: "some-vendor/infra-client".into(), file: "domain/domain.go".into(), line: 5}]}`
    (`"some-vendor/infra-client"` is an external/unresolved import path — absent from `graph.nodes`,
    matching how `build_go`/`build_js` would actually treat it; this test constructs the edge
    directly, since today's extractors would never let it reach `graph.edges` in the first place, to
    prove the checker doesn't rely solely on that invariant holding elsewhere),
    *When* `ComponentDependencyChecker.check(&graph, &config)` runs, *Then* it returns **zero**
    findings — `"some-vendor/infra-client"` textually matches the `infra` component's `**/infra/**`
    glob, but because it isn't present in `graph.nodes`, it's never resolved to a component or
    evaluated against any rule.
**Files**: `src/architecture_checks.rs`

##### Task 1.1.1a: `ComponentDependencyChecker` struct + `check()` core logic (~5 min)
- New zero-sized struct; `check()` iterates `graph.edges`, resolves `component_of(&edge.from, &components)`/
  `component_of(&edge.to, &components)` via `config.rs::component_of` and `config.effective_components()`,
  skips `None`/`None` or equal components, looks up the matching `DependencyRule` from
  `config.effective_dependency_rules()`, applies deny-wins-then-allow-list-then-deny-by-default per
  the acceptance criteria, and formats the `[component-deps]` message shown above.
- Files: `src/architecture_checks.rs`

##### Task 1.1.1b: Register in `architecture_checks::registry()` (~2 min)
- Add `Box::new(ComponentDependencyChecker)` to the `vec![...]` in `registry()`.
- Files: `src/architecture_checks.rs`

##### Task 1.1.1c: Unit tests for the first 5 acceptance criteria above (~5 min, may split into 2 tasks if over budget)
- `component_deps_flags_disallowed_edge`, `component_deps_deny_wins_over_allow`,
  `component_deps_denies_by_default_with_no_rule`, `component_deps_ignores_same_component_edges`,
  `component_deps_ignores_unmapped_nodes`.
- Files: `src/architecture_checks.rs`

##### Task 1.1.1d: Graph-membership guard on edge targets before component resolution (~3 min)
- In `check()`, before calling `component_of` on `edge.to`/`edge.from`, require
  `graph.nodes.contains(&edge.to) && graph.nodes.contains(&edge.from)` — skip the edge (no finding)
  if either side isn't an actual `graph.nodes` entry. Doc-comment this as deliberate defense-in-depth
  (see the acceptance criterion above for the verified `build_go`/`build_js` precedent this restates
  explicitly rather than assumes).
- Files: `src/architecture_checks.rs`

##### Task 1.1.1e: Unit test for the graph-membership guard (~3 min)
- `component_deps_ignores_glob_matching_external_import_not_in_graph_nodes` per the acceptance
  criterion above.
- Files: `src/architecture_checks.rs`

#### Story 1.1.2: `layers`-desugar golden regression tests
**As a** kibitzer maintainer, **I want** proof `layers` desugars to identical findings, **so that**
every existing config in Tyler's other repos keeps working exactly as before (requirements.md's
Success Metrics, verbatim).
**Acceptance Criteria**:
- For each of the 5 existing `LayeringChecker` test fixtures (`layering_flags_a_reverse_dependency`,
  `layering_allows_a_forward_dependency`, `layering_ignores_packages_outside_the_declared_layers`,
  `layering_with_no_declared_layers_has_no_findings`), running `ComponentDependencyChecker` over
  `config.effective_components()`/`effective_dependency_rules()` on the *same* graph produces a
  `Vec<ArchFinding>` that is set-equal on `(file, line)` pairs to `LayeringChecker`'s own output
  (message text differs by design — `[component-deps]` vs. `layering violation:` — only
  file/line/violation-count must match).
  - *Given* the `layering_flags_a_reverse_dependency` fixture (`app/infra` → `app/domain`,
    `layers: ["domain", "infra"]`), *When* both checkers run, *Then* `LayeringChecker` returns 1
    finding at `(edge.file, edge.line)` and `ComponentDependencyChecker` (via `effective_*`) also
    returns exactly 1 finding at the same `(file, line)`.
- Adversarial segment-exact-match case: a package segment `domain2` must NOT match a component
  named `domain`.
  - *Given* `graph.nodes = ["app/domain2"]`, edge `app/domain2 -> app/infra`, `layers: ["domain", "infra"]`,
    *When* both checkers run, *Then* both return **zero** findings — `domain2` matches neither
    `layer_of()`'s exact-segment match nor the desugared `**/domain`/`**/domain/**` globs (glob
    anchors on the full segment, not a substring).
**Files**: `src/architecture_checks.rs`

##### Task 1.1.2a: Golden-comparison test helper (~4 min)
- `fn assert_same_findings_by_location(layering: &[ArchFinding], component_deps: &[ArchFinding])`
  comparing sorted `(file, line)` multisets.
- Files: `src/architecture_checks.rs`

##### Task 1.1.2b: Port the 4 existing `LayeringChecker` fixtures through both checkers (~5 min)
- One test per fixture, each building the graph once and asserting both checkers via the helper.
- Files: `src/architecture_checks.rs`

##### Task 1.1.2c: Adversarial `domain2`-vs-`domain` regression test (~4 min)
- `layers_desugar_does_not_substring_match_segment_names` per the acceptance criterion.
- Files: `src/architecture_checks.rs`

#### Story 1.1.3: Zero-match component glob → advisory finding (never masquerades as blocking)
**As a** kibitzer user, **I want** a component whose glob matches zero files to be surfaced as an
advisory — even when the `Check` that surfaces it is `"severity": "blocking"` — instead of
silently never firing or wrongly failing the run, **so that** I notice a stale/typo'd path pattern
without a fresh project or narrowed `--scope` run getting blocked on a legitimate zero-match.

This story also fixes design/ux.md's headline verified gap: `src/mcp.rs:176-183` derives `level`
once per `Check` (from `Check.severity`) and applies it to every line of that check's output, so a
zero-match advisory from a `"severity": "blocking"` `component-deps`/`content-rules`/`naming-rules`
check previously rendered as `[blocking]` and failed the run — contradicting the explicit UX design
requirement that "zero matches" never hard-fails (see Pattern Decisions' "Zero-match advisory
severity" row for the option (a) vs (b) tradeoff). It introduces the plumbing (`ArchFinding.severity_override`,
`CheckResult.findings`) and a shared `zero_match_advisory<T>` helper that Phase 2's `ContentChecker`
(Story 2.2.1) and Phase 3's `NamingChecker` (Story 3.1.2) both reuse — also closing
architecture-review.md's "duplicated zero-match audit logic across `ComponentDependencyChecker` and
`NamingChecker`, with `ContentChecker` inconsistently excluded" finding and design/ux.md gap #2 (the
zero-match-component advisory previously only fired when a `component-deps` check was configured, so
a `content-rules`/`naming-rules`-only config never saw a typo'd component glob flagged) in the same
change.

**Acceptance Criteria**:
- `ArchFinding` gains `severity_override: Option<Severity>`; `CheckResult` gains
  `findings: Vec<ArchFinding>` — both `#[serde(default)]` and additive, so an existing `cache.json`
  written before these fields existed deserializes unchanged (same precedent as the existing
  `command` field, `src/check.rs:26-32`).
  - *Given* a `cache.json` blob written before this story lands (no `findings`/`severity_override`
    keys anywhere), *When* `Cache::load` deserializes it, *Then* it succeeds — every cached
    `CheckResult.findings` is `vec![]` and every cached `ArchFinding.severity_override` is `None` —
    with no cache invalidation.
- A declared `Component` whose `paths` match no node in the current `ImportGraph` produces exactly
  one advisory-shaped `ArchFinding` per batch run (not per rule referencing it), and that finding's
  `severity_override` is always `Some(Severity::Advisory)` — regardless of the configuring `Check`'s
  own severity.
  - *Given* `components = [Component{name:"domain",paths:vec!["**/domain".into()]}]`,
    `graph.nodes = ["app/handlers", "app/infra"]` (no `domain`), and a
    `Check{architecture_checker: Some("component-deps"), severity: Severity::Blocking, ...}`, *When*
    `ComponentDependencyChecker.check()` runs, *Then* the returned findings include one with
    `file: None`, `line: None`, `severity_override: Some(Severity::Advisory)`, and
    `message == "[component] component 'domain' (glob '**/domain') matched 0 nodes in the import graph — rules referencing it will never fire"`
    (category tag is `[component]`, not `[component-deps]` — a checker-independent tag, since Phase
    2/3's `ContentChecker`/`NamingChecker` emit the identical finding kind too).
  - *Given* the same blocking `Check`, *When* `architecture_assessment` renders that finding, *Then*
    its output line is prefixed `[advisory]`, not `[blocking]`, even though `Check.severity` is
    `Blocking`.
- A real (non-zero-match) violation from the same blocking `Check` still renders `[blocking]` —
  `severity_override` only ever narrows a finding toward `Advisory`, it never overrides a real
  violation's severity upward or downward.
  - *Given* the same blocking `Check` also produces a real `component-deps` violation finding
    (`severity_override: None`), *When* `architecture_assessment` renders it, *Then* that line is
    still prefixed `[blocking]`.
- `mcp.rs::architecture_assessment` renders each architecture-checker finding's *effective* level as
  `finding.severity_override.unwrap_or(result.severity)`, sourced from `result.findings` — the
  pre-existing per-line loop over `result.output.lines()` remains the path for check kinds that never
  populate `findings` (e.g. `SYNTAX_RULES_CHECKERS`, which build their `CheckResult` via `run_check`,
  not `run_architecture_check`).
- The zero-match-component advisory is computed by one shared, generic helper — not duplicated per
  checker — so `ComponentDependencyChecker` (this story), `ContentChecker` (Story 2.2.1), and
  `NamingChecker` (Story 3.1.2) all reuse it.
  - *Given* `fn zero_match_advisory<T>(declared: &[T], is_matched: impl Fn(&T) -> bool, describe: impl Fn(&T) -> String) -> Option<ArchFinding>`
    defined once in `src/architecture_checks.rs`, *When* `ComponentDependencyChecker::check()` calls
    it once per `component in config.effective_components()` with `is_matched` checking
    `graph.nodes`, *Then* it produces the same finding as the previous (now-removed) bespoke
    zero-match loop.
**Files**: `src/architecture_checks.rs`, `src/check.rs`, `src/mcp.rs`

##### Task 1.1.3a: `ArchFinding.severity_override: Option<Severity>` + derive `Serialize`/`Deserialize` on `ArchFinding` (~3 min)
- Add the field (`#[serde(default)]`) and the two derives — `Severity` already derives both
  (`src/config.rs:9`), so no further plumbing is needed for the field itself to (de)serialize.
- Files: `src/architecture_checks.rs`

##### Task 1.1.3b: `CheckResult.findings: Vec<ArchFinding>` field, `#[serde(default)]` (~2 min)
- Files: `src/check.rs`

##### Task 1.1.3c: `run_architecture_check` populates `findings` alongside the existing `output` string (~3 min)
- `output`/`combined`'s formatting is unchanged (still no bracket prefix — Story 1.2.2's CLI
  subcommand reads `output` directly and must keep printing unwrapped `{file}:{line}: {message}`
  lines per design/ux.md surface (b): "this path bypasses `Check.severity` entirely"); `findings` is
  the new, additional field carrying the same `Vec<ArchFinding>` structurally.
- Files: `src/check.rs`

##### Task 1.1.3d: Generic `zero_match_advisory<T>` helper in `src/architecture_checks.rs` (~4 min)
- Signature per the acceptance criterion above; returns a finding with `file: None`, `line: None`,
  `severity_override: Some(Severity::Advisory)`, and `message` built from `describe(item)` plus the
  caller-supplied noun for what was searched (e.g. `"nodes in the import graph"` vs. `"declarations"`)
  — parameterize that noun too so Phase 2/3 callers produce the right wording.
- Files: `src/architecture_checks.rs`

##### Task 1.1.3e: Zero-match detection in `ComponentDependencyChecker::check()`, using the helper against `graph.nodes` (~4 min)
- Replaces the previous bespoke zero-match loop; `[component]` tag, message shape per the acceptance
  criterion above.
- Files: `src/architecture_checks.rs`

##### Task 1.1.3f: `mcp.rs::architecture_assessment` renders from `result.findings` when non-empty (~5 min)
- For `Check`s with `architecture_checker.is_some()`, replace the
  `for finding_line in result.output.lines()` loop with a loop over `result.findings`, formatting
  `"[{level}] {file}:{line}: {message}"` (or `"[{level}] {message}"` when `file`/`line` are `None`,
  matching the existing `combined`-formatting rules in `run_architecture_check`) per finding, where
  `level = finding.severity_override.unwrap_or(result.severity)`. The `SYNTAX_RULES_CHECKERS` loop
  below it (`src/mcp.rs:189-230`) is untouched — it builds `CheckResult` via `run_check`, which never
  populates `findings`.
- Files: `src/mcp.rs`

##### Task 1.1.3g: Unit tests (~5 min)
- `component_deps_flags_zero_match_component_as_advisory_even_when_check_is_blocking`,
  `cache_json_without_findings_field_deserializes_with_empty_findings` (round-trips an old-shape JSON
  blob missing the new keys), `architecture_assessment_renders_zero_match_advisory_as_advisory_under_a_blocking_check`
  (mcp.rs integration test), `architecture_assessment_still_renders_a_real_blocking_violation_as_blocking`.
- Files: `src/architecture_checks.rs`, `src/check.rs`, `src/mcp.rs`

### Epic 1.2: Dispatch Plumbing (`AnyArchitectureChecker`)
**Goal**: The one merge point architecture.md §6 calls for, wiring `component-deps` (and, once
Phase 2/3 land, `content-rules`/`naming-rules`) into `validate()` and the batch/MCP dispatch path —
implemented now so Phase 2/3 only add registry entries, not new dispatch code.

#### Story 1.2.1: `AnyArchitectureChecker` enum + `run_architecture_check` rewrite
**As a** kibitzer maintainer, **I want** one dispatch point resolving a check name against both the
`ArchitectureChecker` and (future) `DeclarationChecker` registries, **so that** `validate()` and
batch execution don't hardcode "always build an `ImportGraph`."
**Acceptance Criteria**:
- `config.rs::validate()` accepts `architecture_checker: "component-deps"` (currently would reject
  it as "unknown architecture checker" since it isn't in `architecture_checks::registry()` yet —
  wait, it now is, from Story 1.1.1b — this criterion instead proves the *dispatch* function looks
  through both registries generically, ready for Phase 2's `content-rules`, which doesn't exist yet).
  - *Given* `{"checks": [{"name": "n", "architecture_checker": "content-rules", "severity": "advisory", "triggers": ["batch"]}]}`
    parsed **before Phase 2 lands** (i.e. `declaration_checks::registry()` doesn't exist yet — this
    specific criterion is re-verified once Phase 2's Task 2.1.3b adds the module; until then this
    exact input is expected to fail with `"unknown architecture checker 'content-rules'"`, proving
    the dual-lookup correctly reports "not found in either registry" rather than crashing).
- `run_architecture_check` continues to have its existing signature
  (`fn(&Check, &Path, &[PathBuf], &ArchitectureConfig) -> Result<CheckResult>`) — no caller in
  `check.rs`/`mcp.rs` needs to change.
  - *Given* a `Check{architecture_checker: Some("component-deps"), ...}` and a real Go fixture repo,
    *When* `run_architecture_check` is called exactly as `mcp.rs`/`check.rs` already call it today,
    *Then* it returns a `CheckResult` (compiles unchanged at every existing call site).
**Files**: `src/config.rs`, `src/check.rs`

##### Task 1.2.1a: `AnyArchitectureChecker` enum (~3 min)
- In `src/check.rs` (or a new tiny `src/architecture_dispatch.rs` if `check.rs` is already large —
  prefer `check.rs` since it already owns `run_architecture_check`): `enum AnyArchitectureChecker { Import(Box<dyn crate::architecture_checks::ArchitectureChecker>), Declaration(Box<dyn crate::declaration_checks::DeclarationChecker>) }`
  — this task stub-references `declaration_checks` before it exists; gate behind `#[allow(dead_code)]`
  or defer the `Declaration` variant's real use to Phase 2 (Task 2.2.2a) if `declaration_checks.rs`
  doesn't exist yet at this point in sequencing. **Sequencing note**: since Phase 0→1→2 is the
  declared order, do this task's `Declaration` arm as a forward-reference against a Phase 2 module
  path that will exist once Phase 2 lands; if implementing Phase 1 strictly before Phase 2, stub
  `declaration_checks::lookup` as returning `None` unconditionally in a placeholder module now, then
  replace it in Phase 2 (Task 2.1.3b) — avoids a forward-compile dependency ordering problem.
- Files: `src/check.rs`

##### Task 1.2.1b: `lookup_any_architecture_checker(name) -> Option<AnyArchitectureChecker>` (~3 min)
- Tries `architecture_checks::lookup(name)` first, falls back to `declaration_checks::lookup(name)`.
- Files: `src/check.rs`

##### Task 1.2.1c: Rewrite `run_architecture_check` to branch on the enum (~5 min)
- `Import(checker)` path: existing behavior (build `ImportGraph`, call `.check(&graph, arch_config)`).
- `Declaration(checker)` path (placeholder until Phase 2): build `DeclarationGraph` via
  `crate::declarations::build(repo_root, files, &arch_config.effective_components())`, call
  `.check(&graph, arch_config)`.
- Files: `src/check.rs`

##### Task 1.2.1d: Update `config.rs::validate()`'s unknown-architecture-checker branch to use `lookup_any_architecture_checker` (~3 min)
- Replace the direct `crate::architecture_checks::lookup(arch_name).is_none()` check with the dual-registry lookup.
- Files: `src/config.rs`

##### Task 1.2.1e: Regression tests for existing `import-cycles`/`layering`/`coupling` dispatch (~4 min)
- Confirm `rejects_unknown_architecture_checker_name` and `accepts_architecture_checker_with_batch_trigger`
  (existing tests, `src/config.rs:313`/`344`) still pass unmodified — no new test needed if they
  pass as-is; this task is "run `cargo test config::` and confirm zero regressions," not new code.
- Files: `src/config.rs` (verification only)

#### Story 1.2.2: `kibitzer check architecture <name> <dir>` CLI subcommand
**As a** kibitzer user, **I want** a direct CLI verb to run one architecture/declaration checker
against a directory, **so that** I can verify a rule fires without spinning up the MCP server or a
full batch run (closing the gap `run_architecture_check`'s own `cmd_str` already implied existed —
see Unresolved Questions #1).
**Acceptance Criteria**:
- `kibitzer check architecture component-deps testdata/dogfood-architecture` exits non-zero and
  prints one line per finding in `{file}:{line}: {message}` form when a violation exists.
  - *Given* the Phase 7 dogfood fixture (domain importing infra, a deliberate violation),
    *When* `kibitzer check architecture component-deps testdata/dogfood-architecture` runs,
    *Then* it prints a line containing `domain/domain.go:` and `[component-deps]`, and exits with
    code `1`.
- An unknown checker name exits non-zero with a clear message.
  - *Given* `kibitzer check architecture does-not-exist .`, *When* run, *Then* it prints
    `no architecture checker named 'does-not-exist' registered` to stderr and exits non-zero.
**Files**: `src/main.rs`

##### Task 1.2.2a: Add `Architecture { name: String, dir: PathBuf }` variant to `CheckCommand` (~2 min)
- Files: `src/main.rs`

##### Task 1.2.2b: Implement the match arm (~5 min)
- Walk `dir` for files (reuse whatever file-walking helper `run::run_batch` already uses — check
  `src/run.rs` for the existing walker before writing a new one), call
  `lookup_any_architecture_checker`/the Phase 1.2.1 dispatch path directly (not through a `Check`/
  config — build one synthetic `Check` in-memory, mirroring `mcp.rs`'s `SYNTAX_RULES_CHECKERS`
  synthetic-`Check` pattern at `src/mcp.rs:196`), print findings, return `ExitCode::from(1)` if any.
- Files: `src/main.rs`

##### Task 1.2.2c: Integration test via `assert_cmd`-style subprocess or direct function call (~4 min)
- Match whatever test pattern `src/main.rs` already uses for `CheckCommand::Native` (check existing
  tests first — likely none exist directly in `main.rs`; if so, add a `#[cfg(test)]` test calling
  the extracted logic function directly rather than spawning a subprocess).
- Files: `src/main.rs`

---

## Phase 2: Gap 2 — Content Rules (+ Declaration Extraction Infra)

### Epic 2.1: `DeclarationGraph` Infrastructure (Go + JS/TS)
**Goal**: The new third parsing pass (per ADR-001) — whole-repo declaration enumeration, Go and
JS/TS first (matching `import_graph.rs`'s own initial-language scope before Python/Java/Kotlin were
added in Phase 4/5).

#### Story 2.1.1: `DeclKind`/`Declaration`/`DeclarationGraph` types + Go extraction
**As a** kibitzer maintainer, **I want** a `DeclarationGraph` populated from Go source, **so that**
`ContentChecker`/`NamingChecker` (Story 2.2.1, Phase 3) have real per-declaration data to check.
**Acceptance Criteria**:
- Building over a Go file with one struct and one function produces two `Declaration`s with correct
  `kind`/`name`/`line`.
  - *Given* the file `domain/domain.go` containing:
    ```go
    package domain

    type Order struct {
        ID string
    }

    func Validate(o Order) error { return nil }
    ```
    *When* `declarations::build(repo_root, &[domain_go_path], &components)` runs,
    *Then* the returned `DeclarationGraph.declarations` contains
    `Declaration{name: "Order", kind: DeclKind::Struct, file: domain_go_path.clone(), line: 3, component: Some("domain")}`
    and `Declaration{name: "Validate", kind: DeclKind::Function, file: domain_go_path.clone(), line: 8, component: Some("domain")}`
    (given `components = [Component{name:"domain", paths: vec!["**/domain".into(), "**/domain/**".into()]}]`
    and `domain_go_path`'s repo-relative form is `domain/domain.go`, matched via the same
    `matches_scope` component resolution as Phase 1 — but here matched against the **file path**,
    not a package-identity string; see Pattern Decisions' Java/Kotlin normalization entry for why
    this is a different string space than `ImportGraph` nodes).
- A Go interface declaration is classified `DeclKind::Interface`.
  - *Given* `type Repository interface { Save(Order) error }` in the same file,
    *When* built, *Then* a `Declaration{name: "Repository", kind: DeclKind::Interface, ...}` is present.
**Files**: `src/declarations.rs` (new)

##### Task 2.1.1a: `DeclKind` enum (~2 min)
- `#[derive(Debug, Clone, Copy, PartialEq, Eq)] pub enum DeclKind { Struct, Class, Interface, Enum, Function }`.
- Files: `src/declarations.rs`

##### Task 2.1.1b: `Declaration` and `DeclarationGraph` structs (~3 min)
- `pub struct Declaration { pub name: String, pub kind: DeclKind, pub file: PathBuf, pub line: usize, pub component: Option<String> }`
  and `pub struct DeclarationGraph { pub declarations: Vec<Declaration> }`.
- Files: `src/declarations.rs`

##### Task 2.1.1c: `build()` entry point + Go dispatch (~4 min)
- `pub fn build(repo_root: &Path, files: &[PathBuf], components: &[Component]) -> Result<DeclarationGraph>`,
  dispatching `.go` files to `build_go_declarations`, mirroring `import_graph.rs::build()`'s
  extension-filter pattern.
- Files: `src/declarations.rs`

##### Task 2.1.1d: `build_go_declarations` — walk `type_declaration`/`function_declaration`/`method_declaration` (~5 min)
- Recursive `Node` walk (matching `rules.rs`'s established hand-rolled pattern, not `tree_sitter::Query`
  per stack.md's explicit "no Query usage" finding): `type_spec` with a `struct_type`/`interface_type`
  child → `Struct`/`Interface`; `function_declaration`/`method_declaration` → `Function`. Uses a
  fresh `tree_sitter::Parser` per file (matching `import_graph.rs::build_go`'s existing pattern —
  not `GrammarCache`, since `declarations.rs` is its own pass with its own cache, per architecture.md
  §4's point 3; using `checker::GrammarCache` directly is a valid alternative if a shared-cache
  instance is threaded through `build()` — pick whichever keeps the diff smaller, document the choice
  in a doc comment).
- Files: `src/declarations.rs`

##### Task 2.1.1e: `resolve_component` helper reusing `config::component_of` against the file's repo-relative path (~3 min)
- `fn resolve_component(repo_root: &Path, file: &Path, components: &[Component]) -> Option<String>`.
- Files: `src/declarations.rs`

##### Task 2.1.1f: Unit tests (disk-fixture style, matching `import_graph.rs`'s test convention) (~5 min)
- `go_declarations_finds_struct_and_function`, `go_declarations_finds_interface`.
- Files: `src/declarations.rs`

#### Story 2.1.2: JS/TS declaration extraction
**As a** kibitzer maintainer, **I want** `DeclarationGraph` coverage for JS/TS, **so that**
content/naming rules work on Tyler's TypeScript projects too, matching `ImportGraph`'s existing
Go+JS/TS baseline.
**Acceptance Criteria**:
- A TS file with a `class` and an `interface` produces `Class`/`Interface` declarations.
  - *Given* `web/src/domain/Order.ts` containing:
    ```typescript
    export interface Repository { save(o: Order): Promise<void>; }
    export class Order { id: string; }
    ```
    *When* built, *Then* the graph contains `Declaration{name: "Repository", kind: DeclKind::Interface, line: 1, ...}`
    and `Declaration{name: "Order", kind: DeclKind::Class, line: 2, ...}`.
**Files**: `src/declarations.rs`

##### Task 2.1.2a: `build_js_ts_declarations` — walk `class_declaration`/`interface_declaration`/`function_declaration` (~5 min)
- Reuses `import_graph.rs::js_ts_language()` for language selection by extension (call it directly —
  it's already `pub`-visible within the crate, or make it `pub(crate)` if currently private; check
  visibility before assuming, adjust with a one-line `pub(crate)` change if needed).
- Files: `src/declarations.rs`, possibly `src/import_graph.rs` (visibility only)

##### Task 2.1.2b: Wire `.ts`/`.tsx`/`.js`/`.jsx` dispatch into `build()` (~2 min)
- Files: `src/declarations.rs`

##### Task 2.1.2c: Unit test (~3 min)
- `ts_declarations_finds_class_and_interface`.
- Files: `src/declarations.rs`

#### Story 2.1.3: `DeclarationChecker` trait + registry scaffold (ADR-001's third trait, realized)
**As a** kibitzer maintainer, **I want** the `DeclarationChecker` trait and its registry defined,
**so that** `ContentChecker` (Story 2.2.1) and `NamingChecker` (Phase 3) have a home, and Phase
1.2.1's placeholder `declaration_checks::lookup` becomes real.
**Acceptance Criteria**:
- `declaration_checks::registry()` exists and, once `ContentChecker` is added in Story 2.2.1,
  `lookup("content-rules")` returns `Some`.
  - *Given* `declaration_checks::lookup("content-rules")` called after Story 2.2.1 lands,
    *When* invoked, *Then* it returns `Some(_)`.
  - *Given* `declaration_checks::lookup("does-not-exist")`, *When* invoked, *Then* it returns `None`.
**Files**: `src/declaration_checks.rs` (new)

##### Task 2.1.3a: `DeclarationChecker` trait (~3 min)
- `pub trait DeclarationChecker { fn name(&self) -> &str; fn check(&self, graph: &DeclarationGraph, config: &ArchitectureConfig) -> Vec<ArchFinding>; }`,
  reusing `crate::architecture_checks::ArchFinding` unmodified (per architecture.md §4).
- Files: `src/declaration_checks.rs`

##### Task 2.1.3b: `registry()`/`lookup()` (~3 min)
- `pub fn registry() -> Vec<Box<dyn DeclarationChecker>> { vec![] }` (empty until Story 2.2.1 adds
  `ContentChecker`) and `pub fn lookup(name: &str) -> Option<Box<dyn DeclarationChecker>>`. Also:
  replace Phase 1's placeholder stub (Task 1.2.1a's note) with the real module now that it exists.
- Files: `src/declaration_checks.rs`, `src/check.rs` (remove placeholder), `src/main.rs` (add `mod declaration_checks;`, `mod declarations;`)

##### Task 2.1.3c: Unit test for empty-registry `lookup` (~2 min)
- `lookup_returns_none_before_any_checker_registered` (will need updating once Story 2.2.1 adds one —
  acceptable churn, matches `checker.rs`'s own registry test pattern).
- Files: `src/declaration_checks.rs`

### Epic 2.2: `ContentChecker`

#### Story 2.2.1: `ContentRule` evaluation
**As a** kibitzer user, **I want** "component X may only contain declaration kind Y" enforcement,
**so that** I get `arch-go`'s `contentsRules`-equivalent natively.
**Acceptance Criteria**:
- A declaration whose `kind` isn't in its component's `allowed_kinds` is flagged.
  - *Given* `content_rules = [ContentRule{component: "domain".into(), allowed_kinds: vec!["struct".into()]}]`
    and the `DeclarationGraph` from Story 2.1.1's example (`Order` = Struct, `Validate` = Function,
    both `component: Some("domain")`), *When* `ContentChecker.check(&graph, &config)` runs,
    *Then* it returns exactly one `ArchFinding` with `file: Some("domain/domain.go".into())`,
    `line: Some(8)`, `message == "[content] domain/domain.go:8: 'Validate' (function) is not allowed in component 'domain' — allowed kinds: struct"`.
- A declaration with no resolved `component` (file matches no declared `Component`) is skipped.
  - *Given* a `Declaration{component: None, ...}`, *When* checked, *Then* it produces no finding
    regardless of any `content_rules` entry.
- A component with no `ContentRule` entry allows every kind (no rule = no constraint — distinct
  from `DependencyRule`'s deny-by-default, since "no content rule" has no sensible default kind list
  to deny against; this asymmetry is deliberate, documented in the doc comment).
- A declared `Component` whose `paths` match zero `DeclarationGraph` declarations produces the same
  `[component]` zero-match advisory Story 1.1.3 introduced for `ComponentDependencyChecker` —
  reusing that story's `zero_match_advisory<T>` helper, this time matched against
  `graph.declarations`' resolved `component` field instead of `ImportGraph.nodes`. This closes
  architecture-review.md's "`ContentChecker` gets no equivalent audit at all" finding and
  design/ux.md gap #2 (a `content-rules`-only config, with no `component-deps` check configured,
  previously never saw a typo'd component glob flagged).
  - *Given* `components = [Component{name:"domain",paths:vec!["**/domain".into()]}]` and a
    `DeclarationGraph` whose declarations all resolve to `component: Some("infra")` (none to
    `"domain"`), *When* `ContentChecker.check(&graph, &config)` runs, *Then* the returned findings
    include one with `file: None`, `line: None`, `severity_override: Some(Severity::Advisory)`, and
    `message == "[component] component 'domain' (glob '**/domain') matched 0 declarations — rules referencing it will never fire"`.
**Files**: `src/declaration_checks.rs`

##### Task 2.2.1a: `ContentChecker` struct + `check()` (~5 min)
- Iterate `graph.declarations`, skip `component: None`, look up matching `ContentRule` by
  `declaration.component`, skip if none, else check `decl.kind`'s string form (`kind_name(kind) -> &str`,
  a small helper mapping `DeclKind` to `"struct"`/`"class"`/`"interface"`/`"enum"`/`"function"`) against
  `allowed_kinds`; format the message per the acceptance criterion.
- Files: `src/declaration_checks.rs`

##### Task 2.2.1b: Register in `declaration_checks::registry()` (~1 min)
- Files: `src/declaration_checks.rs`

##### Task 2.2.1c: Unit tests for the first 3 acceptance criteria (~5 min)
- `content_checker_flags_disallowed_kind`, `content_checker_skips_unmapped_declarations`,
  `content_checker_allows_everything_with_no_rule_for_component`.
- Files: `src/declaration_checks.rs`

##### Task 2.2.1d: Zero-match-component advisory in `ContentChecker::check()`, reusing Story 1.1.3's `zero_match_advisory<T>` against `graph.declarations` (~4 min)
- Files: `src/declaration_checks.rs`

##### Task 2.2.1e: Unit test (~2 min)
- `content_checker_flags_zero_match_component_as_advisory`.
- Files: `src/declaration_checks.rs`

#### Story 2.2.2: Wire `content-rules` end to end (validate, batch, MCP)
**As a** kibitzer user, **I want** `"content-rules"` usable exactly like `"import-cycles"` in
`.claude/inspect.json`, **so that** it's a first-class checker, not a special case.
**Acceptance Criteria**:
- `{"checks": [{"name": "n", "architecture_checker": "content-rules", "severity": "advisory", "triggers": ["batch"]}]}`
  parses successfully (this is the exact input from Story 1.2.1's acceptance criterion, now
  expected to *succeed* instead of erroring, since `declaration_checks::lookup` now finds it).
  - *Given* that JSON, *When* `find_config`/`validate` runs, *Then* it returns `Ok`.
- `architecture_assessment` MCP tool includes `content-rules` findings in its output when configured.
  - *Given* the Phase 7 dogfood config (Phase 7 below) with a `content-rules` check configured and
    a real violation present, *When* `architecture_assessment` runs against it, *Then* the output
    contains a line with `[content]`.
**Files**: `src/config.rs`, `src/mcp.rs` (verification only — Story 1.2.1's dispatch already routes
declaration checkers through `run_architecture_check`, which `mcp.rs::architecture_assessment`
already calls generically per check)

##### Task 2.2.2a: Re-run Story 1.2.1's now-real acceptance criterion as a passing test (~2 min)
- Update/add `accepts_content_rules_architecture_checker` in `src/config.rs`, replacing the
  "expected to fail" note from Task 1.2.1a with an "expected to succeed" assertion.
- Files: `src/config.rs`

##### Task 2.2.2b: MCP integration test with a real content-rules violation (~5 min)
- Extend `src/mcp.rs`'s existing `architecture_assessment_reports_cycle_and_layering_findings`-style
  test (`src/mcp.rs:390`) with a `content-rules`-configured fixture repo, asserting the output
  contains `[content]`.
- Files: `src/mcp.rs`

#### Story 2.2.3: Wire declaration checkers into the git-HEAD-baseline downgrade path
**As a** kibitzer user with a blocking-severity `content-rules`/`naming-rules` check, **I want** a
violation that already existed at the git HEAD commit to downgrade to advisory — exactly like
`import-cycles`/`layering`/`coupling`/`component-deps` already do — **so that** I'm not blocked on
pre-existing debt my current edit didn't introduce.

Fixes adversarial-review.md's sole BLOCKER: `check_native_against_git_head_repo`
(`src/check.rs:867-931`) is called unconditionally from `run_architecture_check` whenever
`!passed && severity == Blocking` (`src/check.rs:842-852`), but its own lookup
(`src/check.rs:872`, `crate::architecture_checks::lookup(arch_name)?`) only ever finds checkers in
the `ArchitectureChecker` registry. For a Declaration-kind checker (`content-rules`, `naming-rules`)
that lookup returns `None`, so the function short-circuits to `None` and the
`if let Some(false) = baseline` downgrade branch in `run_architecture_check` never fires — a
blocking `content-rules`/`naming-rules` check always blocks on any violation, including ones that
predate the current edit. Verified directly against `src/check.rs:867-931` before writing this
story (not assumed from the finding text alone). **Scope check, confirmed**: `component-deps`
(Phase 1, Import-kind) is *not* affected — it's registered in the same pre-existing
`architecture_checks::registry()` (Task 1.1.1b) that `crate::architecture_checks::lookup` already
searches, so it already gets the correct downgrade today. Only the two Declaration-kind checkers
need this fix, and neither exists before Phase 2's `declaration_checks::registry()` (Story 2.1.3)
lands — hence this story sits here, not in Phase 1.

**Acceptance Criteria**:
- `check_native_against_git_head_repo` dispatches through both registries the same way
  `run_architecture_check` now does (Story 1.2.1's `lookup_any_architecture_checker`), branching on
  `AnyArchitectureChecker::Import`/`Declaration` and building the matching graph type
  (`ImportGraph` or `DeclarationGraph`) against the git-HEAD snapshot before calling `.check()`.
  - *Given* a `content-rules` check configured `severity: "blocking"`, a `ContentRule` making a
    `Validate` function (in component `domain`, `allowed_kinds: ["struct"]`) a violation, and that
    exact violation present unchanged in both the working tree and the git HEAD commit (i.e. it
    predates the current edit), *When* `run_architecture_check` runs, *Then*
    `check_native_against_git_head_repo("content-rules", repo_root, arch_config)` builds a
    `DeclarationGraph` from the HEAD snapshot (via `crate::declarations::build(&snapshot_dir, &files, &arch_config.effective_components())`),
    calls `ContentChecker.check(&graph, arch_config)` against it, finds the same non-empty
    violation, and returns `Some(false)` — which downgrades the outer `CheckResult.severity` to
    `Severity::Advisory` with the existing `"(downgraded: this violation predates your edits — \
    already present at the git HEAD commit)"` message suffix, exactly matching the behavior already
    proven for `layering`/`import-cycles`/`coupling`/`component-deps`.
  - *Given* the same setup but the violation is new (absent from the git HEAD snapshot — e.g. the
    `Validate` function was added in the current, uncommitted edit), *When* the same dispatch runs,
    *Then* `check_native_against_git_head_repo` returns `Some(true)` (the HEAD-built
    `DeclarationGraph` has no such violation) and the outer `CheckResult.severity` stays
    `Severity::Blocking` — unchanged.
  - *Given* `naming-rules` configured the same way, with a pre-existing `NamingChecker` violation
    present at HEAD, *When* the same dispatch runs, *Then* it downgrades identically — proving the
    fix is generic across both Declaration-kind checkers, not `content-rules`-specific.
- Regression: `component-deps`, `import-cycles`, `layering`, and `coupling` still downgrade
  correctly through the same (now dual-registry) dispatch — no behavior change for the four
  checkers that already worked. **Note**: `check_native_against_git_head_repo` has no test coverage
  at all today (verified — `src/check.rs`'s existing test module has no test naming `layering`,
  `import-cycles`, or `coupling`, and no Phase 0-3 story before this one adds one), so this
  criterion is proven by a *new* test added in this story (Task 2.2.3d), not a rerun of a
  pre-existing one.
  - *Given* an `import-cycles` (or `layering`) check configured `severity: "blocking"` with a
    violation present unchanged at both the working tree and git HEAD, *When*
    `check_native_against_git_head_repo("import-cycles", ...)` runs through the rewritten
    dual-registry dispatch, *Then* it still returns `Some(false)` and the outer `CheckResult`
    downgrades to `Severity::Advisory` — identical to its behavior before Task 2.2.3a/b's rewrite.
**Files**: `src/check.rs`

##### Task 2.2.3a: Change `check_native_against_git_head_repo`'s lookup from `crate::architecture_checks::lookup(arch_name)?` to `lookup_any_architecture_checker(arch_name)?` (~2 min)
- Returns `AnyArchitectureChecker` instead of a bare `Box<dyn ArchitectureChecker>`.
- Files: `src/check.rs`

##### Task 2.2.3b: Branch on the enum when building the HEAD-snapshot graph and calling `.check()` (~5 min)
- `Import(checker)`: unchanged existing behavior — `crate::import_graph::build(&snapshot_dir, &files)`
  then `checker.check(&graph, arch_config)`.
- `Declaration(checker)`: new — `crate::declarations::build(&snapshot_dir, &files, &arch_config.effective_components())`
  then `checker.check(&graph, arch_config)`. Mirrors `Import`'s existing `Result`-to-`Option` handling
  (`.ok().map(...)`) so a `declarations::build()` failure against the snapshot degrades to `None`
  (no downgrade — safe default, matches the existing `Import` path's own error handling), not a panic.
- Files: `src/check.rs`

##### Task 2.2.3c: Unit/integration test proving a pre-existing `content-rules` violation downgrades (~5 min)
- `content_rules_blocking_violation_downgrades_when_it_predates_head`,
  `content_rules_blocking_violation_stays_blocking_when_new_since_head` — build a real git repo
  fixture using the existing `TempRepo` test helper (`src/check.rs`'s test module, already used by
  `baseline_fails_when_violation_predates_the_edit` and similar — reuse it rather than writing a new
  fixture helper), with a `content_rules`-configured `ArchitectureConfig` and a `DeclarationGraph`-backed
  violation.
- Files: `src/check.rs`

##### Task 2.2.3d: Same test for `naming-rules`, plus a new regression test for an Import-kind checker (~5 min)
- `naming_rules_blocking_violation_downgrades_when_it_predates_head`; also add
  `import_kind_checker_head_baseline_downgrade_still_works_through_dual_registry_dispatch` (using
  `import-cycles` or `layering`) — since `check_native_against_git_head_repo` has no prior test
  coverage (see the AC note above), this is the regression guard proving Task 2.2.3a/b's rewrite
  didn't change behavior for the checkers that already worked correctly.
- Files: `src/check.rs`

---

## Phase 3: Gap 3 — Naming Rules (v1: component/glob-conditioned only)

### Epic 3.1: `NamingChecker`
**Goal**: "Declarations of kind K in component X must match pattern Y" — deliberately *not*
"types implementing interface I must match pattern Y" in v1 (see Pattern Decisions).

#### Story 3.1.1: `NamingRule` evaluation
**As a** kibitzer user, **I want** naming-convention enforcement per component/kind, **so that** I
get a scoped-but-real slice of `arch-go`'s `namingRules`.
**Acceptance Criteria**:
- A struct name not matching its component's naming pattern is flagged.
  - *Given* `naming_rules = [NamingRule{component: "infra".into(), kind: "struct".into(), pattern: ".*Repository$|.*Client$".into()}]`
    and a `Declaration{name: "OrderStore", kind: DeclKind::Struct, file: "infra/infra.go".into(), line: 4, component: Some("infra".into())}`,
    *When* `NamingChecker.check(&graph, &config)` runs, *Then* it returns one `ArchFinding` with
    `file: Some("infra/infra.go".into())`, `line: Some(4)`,
    `message == "[naming] infra/infra.go:4: struct 'OrderStore' in component 'infra' does not match required pattern '.*Repository$|.*Client$'"`.
- A matching name produces no finding.
  - *Given* `Declaration{name: "OrderRepository", kind: DeclKind::Struct, component: Some("infra".into()), ...}`
    and the same rule, *When* checked, *Then* no finding is produced.
- A rule's `kind` only applies to declarations of that exact kind — a `Function` named
  `OrderStore` in the same component is unaffected by a `kind: "struct"` rule.
  - *Given* `Declaration{name: "OrderStore", kind: DeclKind::Function, component: Some("infra".into()), ...}`
    and the same `struct`-scoped rule, *When* checked, *Then* no finding is produced.
- An invalid regex in a `NamingRule.pattern` is a config-load-time error, not a runtime panic.
  - *Given* `naming_rules = [NamingRule{component: "infra".into(), kind: "struct".into(), pattern: "(unclosed".into()}]`,
    *When* `find_config`/`validate` runs, *Then* it returns an `Err` containing `"invalid naming rule pattern"`.
**Files**: `src/declaration_checks.rs`, `src/config.rs` (regex validation)

##### Task 3.1.1a: `NamingChecker` struct + `check()` using the `regex` crate (already a dependency per stack.md) (~5 min)
- For each `Declaration`, resolve its `NamingRule` by `(component, kind_name)`, skip if none,
  `Regex::new(&rule.pattern).is_match(&decl.name)` (compiling once per rule per call — small enough
  repo sizes that per-call compilation, matching `glob.rs::glob_to_regex`'s existing non-cached
  precedent, is acceptable; note as a documented non-issue, not a TODO).
- Files: `src/declaration_checks.rs`

##### Task 3.1.1b: Register in `declaration_checks::registry()` (~1 min)
- Files: `src/declaration_checks.rs`

##### Task 3.1.1c: Regex-validity check in `config.rs::validate()` (~4 min)
- For each `naming_rules` entry, `Regex::new(&rule.pattern)` and bail with the message shape above
  on `Err`.
- Files: `src/config.rs`

##### Task 3.1.1d: Unit tests for the 4 acceptance criteria (~5 min)
- `naming_checker_flags_non_matching_name`, `naming_checker_allows_matching_name`,
  `naming_checker_scoped_to_declared_kind_only`, `rejects_invalid_naming_rule_regex`.
- Files: `src/declaration_checks.rs`, `src/config.rs`

#### Story 3.1.2: Zero-match naming pattern / zero-match component → advisory findings
**As a** kibitzer user, **I want** a naming rule matching zero declarations, or a naming-rule-
referenced component matching zero declarations, surfaced as advisories that never masquerade as
blocking, **so that** I notice a rule that's silently dead (component renamed, kind typo, etc.)
without a `"severity": "blocking"` `naming-rules` check ever failing a run over it.
**Acceptance Criteria**:
- A `NamingRule` whose `(component, kind)` matches zero declarations in the current
  `DeclarationGraph` produces one advisory finding per batch run, and that finding's
  `severity_override` is `Some(Severity::Advisory)` (Story 1.1.3's plumbing) so it renders
  `[advisory]` even under a blocking `naming-rules` check.
  - *Given* `naming_rules = [NamingRule{component: "handlers".into(), kind: "interface".into(), pattern: ".*Handler$".into()}]`
    and a `DeclarationGraph` with zero `Interface`-kind declarations mapped to component `"handlers"`,
    *When* `NamingChecker.check()` runs, *Then* the findings include one with `file: None`, `line: None`,
    `severity_override: Some(Severity::Advisory)`, and
    `message == "[naming] naming rule for component 'handlers' kind 'interface' matched 0 declarations — pattern '.*Handler$' will never fire"`.
- A declared `Component` referenced by a `naming_rules` entry but matching zero `DeclarationGraph`
  declarations produces the same shared `[component]` zero-match advisory Story 1.1.3/2.2.1
  introduced — reusing `zero_match_advisory<T>` against `graph.declarations`, same as
  `ContentChecker`. Closes the same architecture-review.md/design-ux.md gap for `naming-rules`-only
  configs that Story 2.2.1's addition closed for `content-rules`-only configs.
  - *Given* `components = [Component{name:"handlers",paths:vec!["**/handlers".into()]}]` and a
    `DeclarationGraph` with zero declarations resolving to `component: Some("handlers")`, *When*
    `NamingChecker.check()` runs, *Then* the findings include one with `severity_override: Some(Severity::Advisory)`
    and `message == "[component] component 'handlers' (glob '**/handlers') matched 0 declarations — rules referencing it will never fire"`.
**Files**: `src/declaration_checks.rs`

##### Task 3.1.2a: Zero-match detection in `NamingChecker::check()` for `(component, kind)` (~4 min)
- After per-declaration checks, for each `naming_rules` entry, if zero `graph.declarations` entries
  matched its `(component, kind)`, push the advisory finding with `severity_override: Some(Severity::Advisory)`.
- Files: `src/declaration_checks.rs`

##### Task 3.1.2b: Unit test (~2 min)
- `naming_checker_flags_zero_match_rule_as_advisory`.
- Files: `src/declaration_checks.rs`

##### Task 3.1.2c: Zero-match-component advisory in `NamingChecker::check()`, reusing Story 1.1.3's `zero_match_advisory<T>` against `graph.declarations` (~3 min)
- Same helper call as Task 2.2.1d; `naming_rules`-referenced components are the input set. If both
  `content-rules` and `naming-rules` are configured for the same repo, both independently emit the
  same `[component]` advisory for a shared typo'd component — accepted as known, documented
  duplication, consistent with the plan's existing `layering`/`component-deps` overlap precedent
  (Unresolved Questions #4).
- Files: `src/declaration_checks.rs`

##### Task 3.1.2d: Unit test (~2 min)
- `naming_checker_flags_zero_match_component_as_advisory`.
- Files: `src/declaration_checks.rs`

---

## Phase 4: Gap 4a — Java + Kotlin Import & Declaration Extraction

### Epic 4.1: Grammar Verification Fixtures (do first — de-risks the rest of the phase)
**Goal**: Real `to_sexp()` output for Java/Kotlin import and declaration node shapes, pinned in a
doc comment, before any extraction code is written — per pitfalls.md's "Net verification task."

#### Story 4.1.1: Java `to_sexp()` fixture
**As a** kibitzer maintainer, **I want** real parse-tree output for Java imports/declarations
pinned in code, **so that** the extraction walker (Story 4.2.2) is built against verified node
shapes, not assumption.
**Acceptance Criteria**:
- A test parses a fixture Java file and asserts on literal `to_sexp()` substrings for each import
  form.
  - *Given* the fixture source:
    ```java
    package com.example.domain;

    import com.example.infra.DbClient;
    import com.example.infra.*;
    import static com.example.infra.Constants.MAX;

    public class Order {
        public String id;
    }
    ```
    *When* parsed with `tree_sitter_java::LANGUAGE` and `.to_sexp()` is called on the root node,
    *Then* the output contains `(import_declaration (scoped_identifier` (for the qualified import)
    and `(import_declaration (asterisk)` (for the wildcard import) — exact strings pinned from the
    real run, pasted into the extraction module's doc comment (per `rules.rs`'s established
    discipline), not typed from memory.
- **(pre-mortem.md P1 #3(ii) / adversarial-review.md's malformed-source Concern)** A deliberately
  malformed/incomplete Java source fixture is parsed and its real error-recovery `to_sexp()` shape is
  captured and pinned — grounding Story 4.2.2/4.3.1's extraction-level malformed-source tests in a
  verified tree shape, not a guess.
  - *Given* a truncated fragment (e.g. `import com.example.infra.DbClient;\n\npublic class Order {\n    public String id\n`
    — missing semicolon after `id`, unclosed `class` body), *When* parsed with
    `tree_sitter_java::LANGUAGE` and `.to_sexp()` is called, *Then* the output is captured and
    documented (does it contain an `ERROR` node, a `MISSING` node, or both — pinned verbatim from the
    real run per adversarial-review.md's concern that "`parser.parse()` essentially never returns
    `None` for a syntax error" and produces an error-recovery tree instead).
**Files**: `src/import_graph.rs` (doc comment + test, ahead of the real extraction code landing in Story 4.2.2)

##### Task 4.1.1a: Write the fixture source + `to_sexp()`-dumping test (~5 min)
- `#[test] fn java_import_to_sexp_fixture()` — parses the fixture above, `println!("{}", tree.root_node().to_sexp())`
  during development (removed or kept as a `#[ignore]`d dump helper — match whatever convention
  `rules.rs`'s own verification tests used, check `rules.rs` for a precedent before deciding), then
  asserts on the real captured substrings.
- Files: `src/import_graph.rs`

##### Task 4.1.1b: Confirm `import static` distinguishability (or lack thereof) and document (~3 min)
- From the real `to_sexp()` output, determine whether `import static` is distinguishable from a
  regular scoped import at the node-kind level; write the finding into the doc comment either way
  (pitfalls.md flagged this must be "confirmed via live `to_sexp()`," not assumed).
- Files: `src/import_graph.rs`

##### Task 4.1.1c: Malformed Java source `to_sexp()` dump + doc comment (~4 min)
- Parse the truncated fragment from the acceptance criterion above, dump and pin the real
  `to_sexp()` output (ERROR/MISSING node shape) into a doc comment above the future malformed-source
  extraction tests' location (Story 4.2.2/4.3.1).
- Files: `src/import_graph.rs`

#### Story 4.1.2: Kotlin `to_sexp()` fixture (highest-risk sub-task in this plan — no reference queries exist anywhere)
**As a** kibitzer maintainer, **I want** real parse-tree output for Kotlin imports/declarations
pinned in code, **so that** kibitzer's extraction query — the first one ever written for this
grammar's import/declaration shapes — is grounded in real output, not the wrong sibling grammar's
node names (`import_header`, confirmed wrong per build-vs-buy.md).
**Acceptance Criteria**:
- A test parses a fixture Kotlin file and asserts on the real `to_sexp()` substrings for a plain
  import, a wildcard import, and an aliased import.
  - *Given* the fixture source:
    ```kotlin
    package com.example.domain

    import com.example.infra.DbClient
    import com.example.infra.*
    import com.example.infra.Legacy as LegacyClient

    class Order(val id: String)
    ```
    *When* parsed with `tree_sitter_kotlin_ng::LANGUAGE` and `.to_sexp()` is called,
    *Then* the output contains `(import (identifier)` or `(import (qualified_identifier)` (whichever
    the real dump shows — pinned verbatim, not guessed) for the plain import, and the *actual*
    representation of the aliased form (`as LegacyClient`) is captured and documented — this is the
    one node shape build-vs-buy.md explicitly flagged as "not confirmed from static schema data."
- **(pre-mortem.md P1 #3(ii) / adversarial-review.md's malformed-source Concern)** A deliberately
  malformed/incomplete Kotlin source fixture is parsed and its real error-recovery `to_sexp()` shape
  is captured and pinned, same purpose as Story 4.1.1's Java equivalent.
  - *Given* a truncated fragment (e.g. `import com.example.infra.DbClient\n\nclass Order(val id: String\n`
    — unclosed parameter list, missing closing paren), *When* parsed with
    `tree_sitter_kotlin_ng::LANGUAGE` and `.to_sexp()` is called, *Then* the output is captured and
    documented (ERROR/MISSING node shape, pinned verbatim from the real run).
**Files**: `src/import_graph.rs`

##### Task 4.1.2a: Write the fixture source + `to_sexp()`-dumping test (~5 min)
- Same pattern as Task 4.1.1a, Kotlin grammar.
- Files: `src/import_graph.rs`

##### Task 4.1.2b: Document the real aliased-import and wildcard-import node shapes (~4 min)
- Paste verbatim `to_sexp()` output for both forms into a doc comment above the (not-yet-written)
  Kotlin extraction function's future location — if this reveals aliased imports need special-case
  handling beyond the generic positional walk, note it explicitly rather than silently generalizing.
- Files: `src/import_graph.rs`

##### Task 4.1.2c: Document `class_declaration`'s interface-vs-class-vs-object distinguishing child (~4 min)
- Real `to_sexp()` dump of `class Order`, `interface Repository`, `object Singleton` — confirm (or
  refute) whether one shared `class_declaration` node kind with a distinguishing keyword child is
  correct (this plan's earlier assumption in the Domain Glossary), before Story 4.3's declaration
  extraction relies on it.
- Files: `src/import_graph.rs` (or `src/declarations.rs` if that module already exists by this point in real implementation order)

##### Task 4.1.2d: Malformed Kotlin source `to_sexp()` dump + doc comment (~4 min)
- Parse the truncated fragment from the acceptance criterion above, dump and pin the real
  `to_sexp()` output into a doc comment above the future malformed-source extraction tests' location
  (Story 4.2.2/4.3.1).
- Files: `src/import_graph.rs`

### Epic 4.2: Shared Qualified-Name Import Family (Go refactor + Java + Kotlin)
**Goal**: One generic resolver (architecture.md §5's `QualifiedImportLangConfig`) driving Go
(refactored), Java, and Kotlin import extraction — 3 small tables, 1 shared implementation.

#### Story 4.2.1: Refactor `build_go` into `build_qualified_name_language` + a Go table
**As a** kibitzer maintainer, **I want** `build_go`'s logic generalized behind a per-language table,
**so that** Java/Kotlin extraction (Story 4.2.2) reuses it instead of duplicating the loop.
**Acceptance Criteria**:
- All 3 existing Go tests in `import_graph.rs` (`go_import_graph_finds_a_two_package_cycle`,
  `go_import_of_stdlib_package_is_ignored`) pass unmodified after the refactor — same input, same
  assertions, zero test-code changes.
  - *Given* the existing `go_import_graph_finds_a_two_package_cycle` fixture (unchanged),
    *When* `build()` is called (now internally routing through `build_qualified_name_language` with
    a Go `QualifiedImportLangConfig`), *Then* the assertions in that test (unmodified) still pass.
- **(pre-mortem.md P1 #3(i))** `build_qualified_name_language` preserves `build_go`'s pre-refactor
  edge-construction invariant: an edge is only ever added when its target string is already present
  in `graph.nodes` (i.e. resolved to a package built from a walked repo-local file) — an import of an
  external/third-party package never becomes a graph edge, for any language driven by this shared
  function. This is the generic-resolver-level guarantee that Story 1.1.1's new graph-membership
  guard defends against ever silently breaking for Java/Kotlin (Story 4.2.2).
  - *Given* a Go file importing both a local package (present in `graph.nodes`) and an external
    package (e.g. `"github.com/some-vendor/infra-client"`, never inserted into `graph.nodes`), *When*
    `build_qualified_name_language` runs, *Then* `graph.edges` contains an edge to the local package
    only — no edge is created for the external import — exactly matching `build_go`'s pre-refactor
    behavior (verified against `src/import_graph.rs:134`, `graph.nodes.contains(&import_path)`).
**Files**: `src/import_graph.rs`

##### Task 4.2.1a: `QualifiedImportLangConfig` struct (~4 min)
- Fields per architecture.md §5: `package_decl_kind: &'static str`, `import_stmt_kind: &'static str`,
  plus function pointers for extracting the package identity string and the imported path string
  from their respective nodes (mirroring `rules.rs::LangRuleConfig`'s `body_finder`/`params_finder`
  function-pointer pattern).
- Files: `src/import_graph.rs`

##### Task 4.2.1b: `build_qualified_name_language(repo_root, files, graph, cfg)` generic function (~5 min)
- Extracted from `build_go`'s existing body, parameterized by `cfg: &QualifiedImportLangConfig`
  instead of hardcoded Go node kinds/field names.
- Files: `src/import_graph.rs`

##### Task 4.2.1c: Go's `QualifiedImportLangConfig` table + `build_go` becomes a thin wrapper (~4 min)
- `fn go_lang_config() -> QualifiedImportLangConfig` capturing today's `go_module_path`/
  `go_package_import_path`/`collect_go_imports` behavior; `build_go` (or its call site in `build()`)
  now calls `build_qualified_name_language(repo_root, files, graph, &go_lang_config())`.
- Files: `src/import_graph.rs`

##### Task 4.2.1d: Run existing Go test suite, confirm zero regressions (~3 min)
- `cargo test import_graph::` — verification task, no new code if green.
- Files: `src/import_graph.rs` (verification only)

##### Task 4.2.1e: Regression test proving the graph-membership invariant survives the refactor (~3 min)
- `build_qualified_name_language_never_creates_edges_to_non_graph_nodes` (Go-driven fixture) per the
  acceptance criterion above.
- Files: `src/import_graph.rs`

#### Story 4.2.2: Java + Kotlin import extraction via the shared family
**As a** kibitzer user with a Java or Kotlin project, **I want** import-graph edges extracted,
**so that** `component-deps`/`import-cycles`/`layering` work on my project.
**Acceptance Criteria**:
- A Java project with two packages importing each other produces a 2-node cycle, node identities
  normalized to `/`-separated (per Pattern Decisions' `normalize_package_identity` entry).
  - *Given* `src/main/java/com/example/domain/Order.java` (`package com.example.domain;` +
    `import com.example.infra.DbClient;`) and `src/main/java/com/example/infra/DbClient.java`
    (`package com.example.infra;` + `import com.example.domain.Order;`),
    *When* `import_graph::build(repo_root, &[order_path, dbclient_path])` runs,
    *Then* `graph.nodes` contains `"com/example/domain"` and `"com/example/infra"` (dot-to-slash
    normalized, **not** `"com.example.domain"`), and `graph.edges` contains both directions,
    forming a cycle `find_cycles` would detect.
- A Kotlin project's plain and wildcard imports both produce edges; an aliased import's target
  package is still correctly extracted (using Story 4.1.2's verified node shape, not a guess).
  - *Given* a two-file Kotlin fixture analogous to the Go/Java one above, using the exact import
    forms verified in Story 4.1.2 (plain, wildcard, aliased), *When* built, *Then* the graph
    contains the corresponding edges for all three import forms.
- `Component.paths` globs written with `/`-segment syntax (e.g. `"**/domain"`) match the normalized
  Java/Kotlin node identities.
  - *Given* `Component{name: "domain", paths: vec!["**/domain".into()]}` and the Java graph above,
    *When* `component_of("com/example/domain", &[that component])` is called, *Then* it returns
    `Some("domain")` — proving the normalization decision from Pattern Decisions actually closes
    the gap it was written to close.
- **(pre-mortem.md P1 #3(i))** The graph-membership invariant (Story 4.2.1) holds for Java/Kotlin
  too: an import of an external/third-party package never becomes a `graph.nodes` or `graph.edges`
  entry, even when a locally-declared `Component`'s glob would textually match a segment of its
  normalized path.
  - *Given* a Java file in package `com.example.domain` importing both `com.example.infra.DbClient`
    (local) and `org.springframework.stereotype.Component` (external — note the deliberately
    matching final segment `"Component"`/`"component"`-shaped name, chosen to stress-test that
    string-similarity to a `Component` name proves nothing), *When* `import_graph::build()` runs,
    *Then* `graph.nodes` contains `"com/example/domain"` and `"com/example/infra"` but **not**
    `"org/springframework/stereotype"`, and `graph.edges` contains no edge targeting it.
- **(pre-mortem.md P1 #3(ii))** Every import statement in a multi-import file is extracted — not
  just the first or last.
  - *Given* a Java file with 4 import statements (2 resolving to local packages present in
    `graph.nodes`, 2 external/unresolved), *When* `import_graph::build()` runs, *Then* `graph.edges`
    contains exactly 2 edges — one per local import — each with the correct target and line number
    (not duplicated, not misattributed to the wrong import's line).
- **(pre-mortem.md P1 #3(ii))** Malformed/incomplete Java and Kotlin source never causes a *wrong*
  edge to be silently accepted as correct — only the exact expected edges, or zero edges.
  - *Given* a Java file containing one well-formed local import followed by a syntactically broken
    second import (e.g. a missing semicolon, reusing Story 4.1.1's malformed-source `to_sexp()`
    fixture), *When* `import_graph::build()` runs, *Then* the returned `graph.edges` either (a)
    contains exactly the edge for the well-formed import and nothing for the broken one, or (b) is
    empty — never an edge with a truncated/mismatched target string or a line number attributed to
    the wrong import. Document which of (a)/(b) the real tree-sitter error-recovery behavior actually
    produces, verified by running it (not assumed).
  - Same test, Kotlin, reusing Story 4.1.2's malformed-source fixture.
**Files**: `src/import_graph.rs`

##### Task 4.2.2a: `normalize_package_identity(dotted: &str) -> String` helper (~2 min)
- `dotted.replace('.', "/")`, with the doc comment from Pattern Decisions explaining why.
- Files: `src/import_graph.rs`

##### Task 4.2.2b: Java `QualifiedImportLangConfig` table (~5 min)
- `package_decl_kind: "package_declaration"`, positional (not field-based, per Story 4.1.1's
  verified finding) extraction of the package's dotted identifier, normalized via Task 4.2.2a;
  positional extraction of `import_declaration`'s `scoped_identifier`/`identifier`/`asterisk` children.
- Files: `src/import_graph.rs`

##### Task 4.2.2c: Kotlin `QualifiedImportLangConfig` table (~5 min)
- `package_decl_kind` per Story 4.1.2's verified node kind, `import_stmt_kind: "import"` (not
  `"import_header"`), positional extraction per the verified aliased/wildcard shapes from Task 4.1.2b.
- Files: `src/import_graph.rs`

##### Task 4.2.2d: Wire `.java`/`.kt`/`.kts` extension dispatch into `build()` (~3 min)
- Files: `src/import_graph.rs`

##### Task 4.2.2e: Unit tests for the 3 acceptance criteria (~5 min, split if needed)
- `java_import_graph_finds_a_two_package_cycle_with_normalized_identity`,
  `kotlin_import_graph_handles_plain_wildcard_and_aliased_imports`,
  `normalized_java_identity_matches_slash_globs`.
- Files: `src/import_graph.rs`

##### Task 4.2.2f: Unit test for external-import exclusion in Java (~3 min)
- `java_import_graph_excludes_external_third_party_imports_from_graph_nodes` per the
  graph-membership acceptance criterion above.
- Files: `src/import_graph.rs`

##### Task 4.2.2g: Unit test for multi-import-block extraction correctness (Java) (~4 min)
- `java_import_graph_extracts_every_import_in_a_multi_import_file` per the acceptance criterion
  above.
- Files: `src/import_graph.rs`

##### Task 4.2.2h: Unit tests for malformed-source resilience, Java and Kotlin (~5 min)
- `java_import_graph_malformed_source_never_extracts_a_wrong_edge`,
  `kotlin_import_graph_malformed_source_never_extracts_a_wrong_edge` — each asserts the exact
  expected-edges-or-empty outcome per the acceptance criterion above, not merely "doesn't panic."
- Files: `src/import_graph.rs`

### Epic 4.3: Java + Kotlin Declaration Extraction
**Goal**: Content/naming rules (Phases 2–3) work on Java/Kotlin too, not just Go/JS.

#### Story 4.3.1: `build_java_declarations` / `build_kotlin_declarations`
**As a** kibitzer user with a Java or Kotlin project, **I want** `content-rules`/`naming-rules`
enforcement, **so that** the "Import-graph language coverage" requirement extends past just
dependency rules to content/naming too.
**Acceptance Criteria**:
- A Java file with a class and an interface produces `Class`/`Interface` declarations.
  - *Given* `com/example/infra/DbClient.java` containing `public interface Repository {}` and
    `public class DbClient implements Repository {}`, *When* `declarations::build()` runs,
    *Then* the graph contains `Declaration{name: "Repository", kind: DeclKind::Interface, ...}` and
    `Declaration{name: "DbClient", kind: DeclKind::Class, ...}`. (Method-level `Function` kind is
    **not** extracted for Java — Java has no top-level functions outside a class, so `DeclKind::Function`
    is documented as inapplicable to Java content/naming rules in this module's doc comment, not
    silently half-supported.)
- A Kotlin file's `class`/`interface`/`object` are correctly distinguished per Story 4.1.2c's
  verified node shape.
  - *Given* the Kotlin fixture from Story 4.1.2c, *When* built, *Then* `Order` is `Class`,
    `Repository` is `Interface`.
- **(pre-mortem.md P1 #3(ii))** An annotated declaration is still correctly classified by kind — the
  annotation must not shift the positional child walk's classification, given Java/Kotlin's
  `kind()`-filtered positional walk (no field names, per Pattern Decisions) is exactly the shape most
  exposed to this risk.
  - *Given* `com/example/domain/Order.java` containing `@Deprecated\npublic class Order {}` and
    `@FunctionalInterface\npublic interface Validator { boolean validate(Order o); }`, *When*
    `declarations::build()` runs, *Then* the graph contains `Declaration{name: "Order", kind: DeclKind::Class, ...}`
    and `Declaration{name: "Validator", kind: DeclKind::Interface, ...}` — neither is misclassified as
    the other kind, nor silently dropped, because of the leading annotation.
  - Same test, Kotlin, using an `@Suppress(...)`-annotated `class`/`interface` pair.
- **(pre-mortem.md P1 #3(ii))** Malformed/incomplete source never causes a declaration to be silently
  misclassified — only the exact expected declarations, or zero.
  - *Given* a Java file containing one well-formed declaration (`public class Order {}`) followed by a
    syntactically broken second declaration (reusing Story 4.1.1's malformed-source `to_sexp()`
    fixture, e.g. `public class Broken {` with no closing brace), *When* `declarations::build()` runs,
    *Then* the returned declarations either (a) contain exactly `Declaration{name: "Order", kind: DeclKind::Class, ...}`
    and nothing for the broken one, or (b) are empty — never a `Declaration` with a wrong `kind` or a
    wrong `name` silently accepted as correct. Document which of (a)/(b) actually happens, verified by
    running it.
  - Same test, Kotlin, reusing Story 4.1.2's malformed-source fixture.
**Files**: `src/declarations.rs`

##### Task 4.3.1a: Java declaration walk (~5 min)
- Files: `src/declarations.rs`

##### Task 4.3.1b: Kotlin declaration walk, using Story 4.1.2c's verified distinguishing child (~5 min)
- Files: `src/declarations.rs`

##### Task 4.3.1c: Wire into `declarations::build()`'s extension dispatch (~2 min)
- Files: `src/declarations.rs`

##### Task 4.3.1d: Unit tests (~4 min)
- `java_declarations_distinguishes_class_and_interface`, `kotlin_declarations_distinguishes_class_interface_object`.
- Files: `src/declarations.rs`

##### Task 4.3.1e: Unit tests for annotated-declaration classification, Java and Kotlin (~5 min)
- `java_declarations_annotation_does_not_shift_positional_classification`,
  `kotlin_declarations_annotation_does_not_shift_positional_classification` per the acceptance
  criterion above.
- Files: `src/declarations.rs`

##### Task 4.3.1f: Unit tests for malformed-source declaration extraction, Java and Kotlin (~5 min)
- `java_declarations_malformed_source_never_misclassifies`,
  `kotlin_declarations_malformed_source_never_misclassifies` — each asserts the exact
  expected-declarations-or-empty outcome per the acceptance criterion above, not merely "doesn't
  panic."
- Files: `src/declarations.rs`

---

## Phase 5: Gap 4b — Python Import & Declaration Extraction

### Epic 5.1: Grammar Verification Fixture

#### Story 5.1.1: Python `to_sexp()` fixture
**As a** kibitzer maintainer, **I want** real parse-tree output for Python's 6 import node kinds,
**so that** `build_python` (Story 5.2.1) handles `import`, `from...import`, relative, wildcard, and
`__future__` forms correctly — not just the two most obvious ones.
**Acceptance Criteria**:
- A test asserts on real `to_sexp()` output covering all forms named in build-vs-buy.md §4.
  - *Given* the fixture source:
    ```python
    import os
    from app.infra import db_client
    from app.infra import db_client as db
    from . import sibling
    from ..domain import Order
    from app.infra import *
    from __future__ import annotations
    ```
    *When* parsed with `tree_sitter_python::LANGUAGE` and `.to_sexp()` is called,
    *Then* the output is captured and asserted to contain distinct node kinds for
    `import_statement`, `import_from_statement`, `aliased_import`, `relative_import`
    (with `import_prefix`), and `future_import_statement` — each pinned verbatim from the real run.
**Files**: `src/import_graph.rs`

##### Task 5.1.1a: Write the fixture source + `to_sexp()`-dumping test (~5 min)
- Files: `src/import_graph.rs`

##### Task 5.1.1b: Document real node shapes for each of the 6 forms in a doc comment (~4 min)
- Files: `src/import_graph.rs`

### Epic 5.2: Bespoke Python Import Extraction

#### Story 5.2.1: `build_python` — relative + heuristic absolute resolution
**As a** kibitzer user with a Python project, **I want** import-graph edges extracted for both
relative and absolute imports, **so that** `component-deps`/`import-cycles`/`layering` work on
Python.
**Acceptance Criteria**:
- A relative import (`from . import sibling`) resolves to the sibling module in the same package.
  - *Given* `app/domain/__init__.py`, `app/domain/order.py` (empty), and
    `app/domain/validator.py` containing `from . import order`, *When* `import_graph::build()` runs
    with these 3 files, *Then* `graph.edges` contains an edge from the `app/domain` node to itself...
    — **no**, same-directory relative imports produce no cross-node edge (matching `build_js`'s
    "skip same-dir" convention at `src/import_graph.rs:247`, `to_dir != from_dir`); the concrete,
    non-trivial case: *Given* `app/domain/validator.py` containing `from ..infra import db_client`
    (one level up, into a sibling package) and `app/infra/__init__.py` present,
    *When* built, *Then* `graph.edges` contains an edge from node `"app/domain"` to node `"app/infra"`.
- An absolute import resolves via the `__init__.py`-rooted heuristic, normalized dot-to-slash
  (reusing Task 4.2.2a's `normalize_package_identity`, consistent with Java/Kotlin).
  - *Given* `app/__init__.py`, `app/domain/__init__.py`, `app/domain/order.py` containing
    `from app.infra import db_client`, and `app/infra/__init__.py`, *When* built,
    *Then* `graph.edges` contains an edge from `"app/domain"` to `"app/infra"` (the dotted
    `app.infra` module path normalized to match the same `/`-separated convention every other
    language uses, and to make `Component.paths`' `/`-segment globs work uniformly).
- A `from __future__ import annotations` line produces zero graph edges (stdlib-equivalent, not a
  local project import) — regression-guards Story 5.1.1's finding that a naive walk matching only
  `import_from_statement` would miss this node kind and mishandle it as an unresolvable local import
  rather than correctly ignoring it.
  - *Given* a file containing only `from __future__ import annotations`, *When* built,
    *Then* `graph.edges` is empty and no error occurs.
**Files**: `src/import_graph.rs`

##### Task 5.2.1a: `python_source_root` heuristic — walk from `repo_root`, treat `__init__.py`-containing dirs as packages (~5 min)
- `fn python_package_of(repo_root: &Path, file: &Path) -> Option<String>` — dotted path from the
  nearest ancestor that is *not* itself inside an `__init__.py` chain (i.e. the first non-package
  ancestor), normalized via `normalize_package_identity`.
- Files: `src/import_graph.rs`

##### Task 5.2.1b: Relative-import resolution — count leading dots (~5 min)
- `fn resolve_python_relative_import(from_file: &Path, dots: usize, dotted_suffix: &str, known: &...) -> Option<String>`.
- Files: `src/import_graph.rs`

##### Task 5.2.1c: Absolute-import resolution against the heuristic package tree (~4 min)
- Files: `src/import_graph.rs`

##### Task 5.2.1d: `collect_python_imports` walk covering all 6 node kinds from Story 5.1.1, explicitly skipping `future_import_statement` (~5 min)
- Files: `src/import_graph.rs`

##### Task 5.2.1e: `build_python` + wire `.py` extension dispatch into `build()` (~3 min)
- Files: `src/import_graph.rs`

##### Task 5.2.1f: Unit tests for the 3 acceptance criteria (~5 min, split if needed)
- `python_relative_import_resolves_to_sibling_package`, `python_absolute_import_resolves_via_init_py_heuristic`,
  `python_future_import_produces_no_edge`.
- Files: `src/import_graph.rs`

### Epic 5.3: Python Declaration Extraction

#### Story 5.3.1: `build_python_declarations`
**As a** kibitzer user with a Python project, **I want** `content-rules`/`naming-rules` coverage,
**so that** Python gets the same content/naming enforcement Go/JS/Java/Kotlin get.
**Acceptance Criteria**:
- A Python file's `class_definition` and `function_definition` produce `Class`/`Function` declarations
  (no `Struct`/`Interface`/`Enum` concept in Python — documented as inapplicable, matching Java's
  `Function`-inapplicable note from Story 4.3.1).
  - *Given* `app/domain/order.py` containing:
    ```python
    class Order:
        def __init__(self, id: str):
            self.id = id

    def validate(order: Order) -> bool:
        return bool(order.id)
    ```
    *When* `declarations::build()` runs, *Then* the graph contains `Declaration{name: "Order", kind: DeclKind::Class, line: 1, ...}`
    and `Declaration{name: "validate", kind: DeclKind::Function, line: 5, ...}` (the nested `__init__`
    method is **not** extracted as a separate top-level declaration — matching this plan's
    type-level/module-level declaration scope, same as Java methods being excluded per Story 4.3.1).
**Files**: `src/declarations.rs`

##### Task 5.3.1a: `build_python_declarations` walk (~5 min)
- Files: `src/declarations.rs`

##### Task 5.3.1b: Wire `.py` dispatch into `declarations::build()` (~2 min)
- Files: `src/declarations.rs`

##### Task 5.3.1c: Unit test (~3 min)
- `python_declarations_finds_class_and_module_level_function_only`.
- Files: `src/declarations.rs`

---

## Phase 6: UX Polish

### Epic 6.1: Uniform `[category]` Bracket Prefix + Per-Category Count Breakdown
**Goal**: UX research's "at 5 categories, implicit/inconsistent self-tagging stops scaling" fix.

#### Story 6.1.1: Bracket-prefix every `ArchFinding` message consistently
**As a** kibitzer user reading `architecture_assessment` output, **I want** every finding
consistently prefixed with its category, **so that** `grep '\[naming\]'` works uniformly across all
5 categories (2 already exist un-prefixed: `import-cycle`, `layering`).
**Acceptance Criteria**:
- `ImportCycleChecker` and `LayeringChecker` messages gain `[import-cycle]`/`[layering]` prefixes
  (currently absent — `CouplingChecker` already has `[coupling]`; `component-deps`/`content`/`naming`
  already got their prefixes in Phases 1–3's tasks above).
  - *Given* the existing `detects_two_node_cycle` test fixture, *When* `ImportCycleChecker.check()`
    runs, *Then* the returned finding's `message` now starts with `"[import-cycle] import cycle: a -> b -> a"`
    (was `"import cycle: a -> b -> a"` before this task — every existing test asserting via
    `.contains("import cycle")` still passes since the prefix is additive, but any test asserting
    an exact `==` match on the old un-prefixed string needs updating — check `src/architecture_checks.rs`'s
    existing tests for `==` assertions on `.message` before making this change, per this plan's own
    "no completion claim without proof" standard).
**Files**: `src/architecture_checks.rs`

##### Task 6.1.1a: Add `[import-cycle]` prefix to `ImportCycleChecker`'s message format (~2 min)
- Files: `src/architecture_checks.rs`

##### Task 6.1.1b: Add `[layering]` prefix to `LayeringChecker`'s message format (~2 min)
- Files: `src/architecture_checks.rs`

##### Task 6.1.1c: Audit and fix any existing test asserting an exact (not `.contains()`) match on the old un-prefixed message (~4 min)
- Files: `src/architecture_checks.rs`

#### Story 6.1.2: Per-category count breakdown line in `architecture_assessment` output
**As a** kibitzer user, **I want** a one-line category breakdown under the aggregate finding count,
**so that** I can see at a glance which rule categories are noisy without reading every line.
**Acceptance Criteria**:
- Output includes a breakdown line derived by parsing each finding's `[category]` prefix.
  - *Given* an assessment producing 1 `[import-cycle]`, 3 `[layering]`, 2 `[coupling]`, 4 `[content]`,
    2 `[naming]` findings, *When* `architecture_assessment` runs, *Then* its output contains the
    line `"  import-cycle: 1, layering: 3, coupling: 2, content: 4, naming: 2"` immediately after
    the existing `"architecture assessment: 12 finding(s) across N file(s)"` line.
**Files**: `src/mcp.rs`

##### Task 6.1.2a: Category-breakdown helper parsing `[category]` prefixes from `lines` (~4 min)
- `fn category_breakdown(lines: &[String]) -> BTreeMap<String, usize>` — regex or simple
  `strip_prefix('[')`/`split(']')` parse (no new dependency), matching declaration order
  `import-cycle, layering, coupling, component-deps, content, naming, component` where present. The
  7th category, `component`, is the shared zero-match-component advisory tag Story 1.1.3 introduced
  (distinct from `component-deps`, since `content-rules`/`naming-rules` checks emit it too — see
  Pattern Decisions' "Zero-match advisory severity" row).
- Files: `src/mcp.rs`

##### Task 6.1.2b: Insert the breakdown line into `architecture_assessment`'s output (~3 min)
- Files: `src/mcp.rs`

##### Task 6.1.2c: Unit test (~3 min)
- Extend the existing `architecture_assessment_reports_cycle_and_layering_findings` test with a
  breakdown-line assertion.
- Files: `src/mcp.rs`

### Epic 6.2: `recommendation_for()` + Mermaid Component Subgraphs

#### Story 6.2.1: Canned recommendations for the 3 new checkers
**As a** kibitzer user, **I want** `component-deps`/`content-rules`/`naming-rules` findings to come
with a canned next-step recommendation (matching `import-cycles`/`layering`'s existing precedent —
optional, since `coupling` has none, per `mcp.rs`'s own comment at line 76-77).
**Acceptance Criteria**:
- `recommendation_for("component-deps")` returns `Some(_)`.
  - *Given* `recommendation_for("component-deps")`, *When* called, *Then* it returns
    `Some("component-deps: either relocate the offending code into a component already allowed \
    to hold that dependency, or add it to the target component's may_depend_on list if the \
    dependency is actually intended.")` (exact wording is an implementation judgment call; the
    *presence* of a non-`None` return is the testable criterion).
**Files**: `src/mcp.rs`

##### Task 6.2.1a: Add 3 match arms to `recommendation_for()` (~4 min)
- Files: `src/mcp.rs`

##### Task 6.2.1b: Unit tests (~2 min)
- `recommendation_for_covers_all_five_native_architecture_checkers` (parametrized or 3 separate assertions).
- Files: `src/mcp.rs`

#### Story 6.2.2: Mermaid diagram groups nodes into per-component `subgraph` blocks
**As a** kibitzer user reading the dependency diagram, **I want** nodes visually grouped by
resolved component, **so that** component boundaries (and `component-deps` violations) are visible
in the diagram, not just the text findings.
**Acceptance Criteria**:
- Nodes resolving to a component are grouped under a `subgraph {component} ... end` block; nodes
  matching no component render outside any subgraph, exactly as today.
  - *Given* `graph.nodes = ["dogfood.example/app/domain", "dogfood.example/app/infra"]` and
    `components = [Component{name:"domain",...}, Component{name:"infra",...}]` both matching,
    *When* `mermaid::render_dependency_graph(&graph, &components)` is called (signature gains a
    `components: &[Component]` parameter — every existing call site updated),
    *Then* the output contains `"subgraph domain"` and `"subgraph infra"`, each containing the
    slugified id of its respective node, with the existing edge/cycle-highlighting lines unchanged.
**Files**: `src/mermaid.rs`, `src/mcp.rs` (call-site update)

##### Task 6.2.2a: `render_dependency_graph` gains a `components: &[Component]` parameter (~4 min)
- Files: `src/mermaid.rs`

##### Task 6.2.2b: Group node-id emission into `subgraph {name} ... end` blocks by resolved component (~5 min)
- Reuses `config::component_of`; nodes with `None` render exactly as today (outside any subgraph).
- Files: `src/mermaid.rs`

##### Task 6.2.2c: Update the one call site in `mcp.rs::architecture_assessment` (~2 min)
- Pass `config.architecture.effective_components()`.
- Files: `src/mcp.rs`

##### Task 6.2.2d: Unit test (~4 min)
- `mermaid_diagram_groups_nodes_by_component`.
- Files: `src/mermaid.rs`

---

## Phase 7: Dogfooding

### Epic 7.1: `testdata/dogfood-architecture/` Fixture + `.claude/inspect.json`
**Goal**: Prove the feature end-to-end on a real (if small, purpose-built) repo before merge, per
requirements.md's Phase 7 scope note — see Unresolved Questions #2 for why this targets a checked-in
Go fixture rather than kibitzer's own (100% Rust, out-of-scope-for-extraction) source.

#### Story 7.1.1: Go fixture project exercising all 3 new rule categories
**As** kibitzer's own repo, **I want** a small, realistic Go fixture with deliberate violations,
**so that** `.claude/inspect.json`'s new checks have something real to flag.
**Acceptance Criteria**:
- The fixture parses as a valid Go module and contains exactly one deliberate violation per new
  rule category.
  - *Given* `testdata/dogfood-architecture/go.mod` (`module dogfood.example/app`),
    `testdata/dogfood-architecture/handlers/handlers.go` (`package handlers`, imports
    `dogfood.example/app/domain` — allowed), `testdata/dogfood-architecture/domain/domain.go`
    (`package domain`, containing `type Order struct { ID string }` and
    `func Validate(o Order) error { return nil }`, and importing `dogfood.example/app/infra` —
    **the deliberate dependency-rule violation**), `testdata/dogfood-architecture/infra/infra.go`
    (`package infra`, containing `type OrderStore struct{}` — **the deliberate naming-rule
    violation**, no internal imports),
    *When* `kibitzer check architecture component-deps testdata/dogfood-architecture` runs (Story
    1.2.2's new CLI verb), *Then* it flags `domain/domain.go` importing `infra` and exits non-zero.
    *When* `kibitzer check architecture content-rules testdata/dogfood-architecture` runs,
    *Then* it flags `domain/domain.go`'s `Validate` function (domain's content rule allows only
    `struct`) and exits non-zero.
    *When* `kibitzer check architecture naming-rules testdata/dogfood-architecture` runs,
    *Then* it flags `infra/infra.go`'s `OrderStore` (infra's naming rule requires a
    `Repository`/`Client` suffix) and exits non-zero.
**Files**: `testdata/dogfood-architecture/go.mod`, `testdata/dogfood-architecture/handlers/handlers.go`,
`testdata/dogfood-architecture/domain/domain.go`, `testdata/dogfood-architecture/infra/infra.go`

##### Task 7.1.1a: Write `go.mod` and `handlers/handlers.go` (~3 min)
- Files: `testdata/dogfood-architecture/go.mod`, `testdata/dogfood-architecture/handlers/handlers.go`

##### Task 7.1.1b: Write `domain/domain.go` with the deliberate dependency + content violations (~3 min)
- Files: `testdata/dogfood-architecture/domain/domain.go`

##### Task 7.1.1c: Write `infra/infra.go` with the deliberate naming violation (~3 min)
- Files: `testdata/dogfood-architecture/infra/infra.go`

##### Task 7.1.1d: Run all three new CLI checks against the fixture, capture and verify real (not predicted) output matches the acceptance criteria (~5 min)
- This is a "run it, don't read it" verification task per this plan's own Evidence-and-Claims
  standard — actually execute `kibitzer check architecture component-deps|content-rules|naming-rules testdata/dogfood-architecture`
  three times and confirm the real exit codes/output, adjusting the fixture if reality diverges
  from the concrete GWTs above (e.g. if a glob pattern doesn't match the way this plan predicted).
- Files: none (verification)

#### Story 7.1.2: `.claude/inspect.json` wiring
**As** kibitzer's own repo, **I want** the new checks registered in `.claude/inspect.json`,
**so that** they run as part of kibitzer's own `batch` trigger and CI, proving integration (not
just standalone CLI invocation).
**Acceptance Criteria**:
- `.claude/inspect.json` (repo root) gains 3 new `Check` entries, each `scope`-restricted to the
  fixture directory, `triggers: ["batch"]`, `severity: "advisory"` (advisory, not blocking — these
  are deliberately-violating example fixtures, not real production code; blocking would break every
  future `kibitzer run --trigger batch` invocation against kibitzer's own repo).
  - *Given* the updated `.claude/inspect.json` containing
    `{"name": "dogfood-component-deps", "architecture_checker": "component-deps", "severity": "advisory", "scope": ["testdata/dogfood-architecture/**"], "triggers": ["batch"]}`
    (and analogous entries for `content-rules`/`naming-rules`), plus the corresponding
    `"architecture": {"components": [...], "dependency_rules": [...], "content_rules": [...], "naming_rules": [...]}`
    block from Story 7.1.1, *When* `kibitzer run . --trigger batch` runs at the kibitzer repo root,
    *Then* its output includes advisory-level findings for all 3 deliberate violations, and the
    overall batch run still exits `0` (advisory findings don't fail a batch run, matching existing
    `Severity::Advisory` semantics elsewhere in the config).
**Files**: `.claude/inspect.json`

##### Task 7.1.2a: Add the `architecture` config block (components/dependency_rules/content_rules/naming_rules) to `.claude/inspect.json` (~4 min)
- Files: `.claude/inspect.json`

##### Task 7.1.2b: Add the 3 new `Check` entries, `scope`-restricted and `advisory` (~3 min)
- Files: `.claude/inspect.json`

##### Task 7.1.2c: Run `kibitzer run . --trigger batch` at the repo root, verify real output (~4 min)
- Same "run it, don't read it" discipline as Task 7.1.1d.
- Files: none (verification)

### Epic 7.2: Real-Repo Adoption Proof — `stapler-squad` `depguard` Migration
**Goal**: Answer the triad review's Product-lens gap ("nothing confirms you'll actually replace
depguard/go-arch-lint/arch-go anywhere real") with a real target, per the resolved Post-Ship
Follow-up above. `tstapler/stapler-squad`'s real `.golangci.yml` has a `depguard` section with 3
rules (fetched via `gh api repos/tstapler/stapler-squad/contents/.golangci.yml`); this epic migrates
the two rules that fit kibitzer's `Component`/`DependencyRule` schema and explicitly documents why
the third does not. This is **verification only** — it proves kibitzer's `component-deps` checker
agrees with `depguard`'s current verdict on real code. It does **not** edit `stapler-squad`'s
`.golangci.yml`, remove `depguard` from its CI, or commit any kibitzer config into that repo — cutting
`stapler-squad`'s actual CI over to kibitzer is a separate, later decision for Tyler once he's seen
the checker agree with reality, not part of this feature.

#### Story 7.2.1: Migrate `no_server_in_core` to `component-deps`
**As** Tyler, **I want** kibitzer's `component-deps` checker to express the same rule as
`stapler-squad`'s `depguard.rules.no_server_in_core`, **so that** I have real evidence — not just a
synthetic fixture — that `component-deps` can replace a `depguard` rule I actually run today.
**Acceptance Criteria**:
- A local (uncommitted — not part of `stapler-squad`'s own checked-in config) `.claude/inspect.json`
  architecture block, run against a real read-only clone of `tstapler/stapler-squad` at HEAD,
  expresses `no_server_in_core` as:
  `Component{name: "core", paths: vec!["session/**".into(), "config/**".into(), "log/**".into()]}`,
  `Component{name: "server", paths: vec!["server/**".into()]}`,
  `DependencyRule{component: "core".into(), may_depend_on: None, deny_depend_on: vec!["server".into()]}`.
  - *Given* that config and a real clone of `tstapler/stapler-squad` at HEAD, *When*
    `golangci-lint run --enable-only depguard ./...` runs inside the clone (establishing
    `depguard`'s actual current verdict for `no_server_in_core` — not assumed clean), *and*
    `kibitzer check architecture component-deps <stapler-squad-clone-dir>` runs (Story 1.2.2's CLI
    verb) against the same HEAD, *Then* both report the same verdict: if `depguard` currently
    reports zero violations for this rule, `component-deps` must also report zero; any divergence is
    a finding to investigate and document in the task's real output, not silently dismissed as
    passing.
- `depguard`'s two `deny` entries (`.../server` and `.../server/**`) collapse into one
  `deny_depend_on: ["server"]` entry, since kibitzer's `component_of()` already matches any file
  under a component's `paths` glob (including subpackages) — no separate rule needed for the
  `server/**` case `depguard` had to spell out explicitly.
**Files**: none in kibitzer's own repo (verification against an external clone) — the local
`.claude/inspect.json` snippet used for the run is scratch, not committed.

##### Task 7.2.1a: Clone `tstapler/stapler-squad` read-only to a scratch directory and run `golangci-lint run --enable-only depguard ./...` to capture the real, current baseline verdict for `no_server_in_core` (~10 min)
- Files: none (external clone, read-only)

##### Task 7.2.1b: Write the scratch `.claude/inspect.json` architecture block above and run `kibitzer check architecture component-deps <clone-dir>` for real, capture output (~10 min)
- "Run it, don't read it" per this plan's Evidence-and-Claims standard — actually execute against
  the real clone, don't predict the output.
- Files: none (scratch config in the external clone, not committed)

##### Task 7.2.1c: Compare the two real outputs, document agreement or divergence (~10 min)
- Files: none (verification)

#### Story 7.2.2: Migrate `no_ent_in_services` to `component-deps` — and document the negation-glob gap
**As** Tyler, **I want** kibitzer's `component-deps` checker to express
`stapler-squad`'s `depguard.rules.no_ent_in_services`, **so that** I have a second real migration
example — while being honest about where kibitzer's schema is weaker than `depguard`'s.
**Acceptance Criteria**:
- `no_ent_in_services` expresses as `Component{name: "services", paths:
  vec!["server/services/**".into()]}`, `Component{name: "ent", paths:
  vec!["session/ent/**".into()]}`, `DependencyRule{component: "services".into(), may_depend_on: None,
  deny_depend_on: vec!["ent".into()]}`.
- **Confirmed limitation, not silently worked around**: `depguard`'s real `no_ent_in_services.files`
  list grandfather-excludes 10 specific files via negation globs (`!**/server/services/error_registry.go`,
  `analytics_escape_service.go`, `analytics_escape_service_test.go`, `workflow_service_test.go`,
  `workflow_service.go`, `backlog_service.go`, `backlog_service_query.go`,
  `backlog_service_lifecycle.go`, `backlog_service_triage.go`, `session_service.go` — comment:
  "removed-in: refactor/storage-interface-cleanup (P6)"). Verified by reading
  `src/glob.rs::matches_scope`/`glob_to_regex` (kibitzer's existing glob engine, already used by
  `Check.scope` today): neither function special-cases a leading `!` — a pattern like
  `"!**/foo.go"` is compiled as a literal glob whose regex requires the path to *start with the
  character `!`*, so it can never match a real repo-relative path. **`Component.paths` has no
  exclusion mechanism today.** This means kibitzer's `services` component, as defined above, includes
  all 10 grandfathered files, and `component-deps` will flag every one of them that actually imports
  `session/ent` as a *new* violation `depguard` currently treats as excluded.
  - *Given* the config above and the real `stapler-squad` clone, *When*
    `kibitzer check architecture component-deps <clone-dir>` runs, *Then* the task records exactly
    how many of the 10 grandfathered files it flags (real count from real output, not predicted) and
    states this divergence explicitly in the story's outcome — it is **evidence of a real schema gap
    this migration surfaced**, not a bug in the migration and not something silently patched over by
    inventing negation-glob support in `Component.paths` as an undocumented side quest. Adding
    exclusion-glob support to `Component.paths` is a legitimate future scope item but is explicitly
    **not** added by this story.
**Files**: none in kibitzer's own repo (verification against an external clone); scratch
`.claude/inspect.json` snippet, not committed.

##### Task 7.2.2a: Write the scratch `.claude/inspect.json` architecture block above and run `kibitzer check architecture component-deps <clone-dir>` for real (~10 min)
- Files: none (scratch config, not committed)

##### Task 7.2.2b: Diff against `golangci-lint run --enable-only depguard ./...`'s real current verdict for `no_ent_in_services`; count and list which of the 10 grandfathered files kibitzer newly flags (~15 min)
- Files: none (verification)

#### Story 7.2.3: Document why `no_ioutil` is not migrated
**As** Tyler, **I want** an explicit record of why `depguard.rules.no_ioutil` stays on `depguard`
even after adopting `component-deps` for the other two rules, **so that** the plan doesn't overclaim
`component-deps` as a 100% `depguard` replacement.
**Acceptance Criteria**:
- `no_ioutil` (`files: ["$all"]`, denying `io/ioutil` repo-wide) is a **global, unscoped** ban — it
  has no source component, unlike `no_server_in_core`/`no_ent_in_services`. Every `DependencyRule` in
  kibitzer's schema is keyed by a source `component`; there is no "applies everywhere regardless of
  component" concept in the plan as written. This rule is explicitly **not** migrated to
  `component-deps`.
- One sentence is added to Epic 7.2's summary (already present above) noting `no_ioutil` stays on
  `depguard`/`golangci-lint` in `stapler-squad` even after the other two rules migrate, and that this
  is evidence kibitzer's dependency-rule model, as currently scoped, is not a full `depguard`
  replacement — a global/unscoped deny-rule category is a plausible future scope item but is
  explicitly **not** added to this plan now, to avoid scope-creeping the schema over one example.
**Files**: none (documentation-only, satisfied by this plan.md section itself).
