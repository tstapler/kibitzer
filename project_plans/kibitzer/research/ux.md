# Research: UX — architecture-linting feature (config, errors, output, JTBD)

Surface: this is a config-file + CLI/MCP-tool-output UX problem, not a GUI. No
WCAG/ARIA/keyboard-nav applicability.

## 1. Config UX: named components + dependency rules

### Current kibitzer config shape (`src/config.rs`)

`Config` is `{ checks: [...], architecture: ArchitectureConfig }`.
`ArchitectureConfig` today is just:

```rust
pub struct ArchitectureConfig {
    pub layers: Vec<String>,  // e.g. ["handlers", "domain", "infra"]
}
```

(`src/config.rs:120-130`) — layer membership is inferred by exact path-segment
match (`architecture_checks.rs:69-74`, `layer_of`), not by an explicit glob.
This is the whole precedent to build on: **kibitzer has never had a
user-declared name → set-of-paths mapping before.** `Check.scope` (glob list)
is the only existing glob-authoring surface, and it's flat ("this check
applies to files matching these globs") — not "these globs are component X."

### Comparable external tools (verified against real docs/source)

**go-arch-lint** (`.go-arch-lint.yml`, confirmed via the actual file at
[fe3dback/go-arch-lint/.go-arch-lint.yml](https://github.com/fe3dback/go-arch-lint/blob/master/.go-arch-lint.yml)):

```yaml
version: 3
workdir: internal
allow:
  depOnAnyVendor: false
excludeFiles:
  - "^.*_test\\.go$"
vendors:
  3rd-cobra: { in: github.com/spf13/cobra }
components:
  main:       { in: app }
  container:  { in: app/internal/container/** }
  operations: { in: operations/* }
  services:   { in: services/** }
  models:     { in: models/** }
commonComponents:
  - models
deps:
  main:
    mayDependOn: [container]
  container:
    anyVendorDeps: true
    mayDependOn: [operations, services]
  operations:
    mayDependOn: [services]
    canUse: [3rd-graph]
```

Two-block structure: `components:` (name → glob) is declared once, then
`deps:` is keyed *by the component doing the importing*, each with an
allow-list (`mayDependOn`). Vendor deps are a parallel, separately-named
system (`vendors:`, `canUse:`), so "this component may use this third-party
package" doesn't get confused with "this component may use this internal
component." `commonComponents`/`commonVendors` factor out the "everything may
depend on X" case so it isn't repeated in every `deps` entry.

**arch-go** (`arch-go.yml`, confirmed via GitHub README fetch):

```yaml
dependenciesRules:
  - package: "**.impl.**"
    shouldOnlyDependsOn:
      internal: ["**.foo.**", "*.bar.**"]
    shouldNotDependsOn:
      internal: ["**.model.**"]
  - package: "**.foobar.**"
    shouldOnlyDependsOn:
      external: ["gopkg.in/yaml.v3"]
```

No separate `components:` block — the glob pattern *is* inline, per-rule,
matched directly against import paths (`*.name`, `**.name`, `name.**`,
`**.name.**`). There's no named indirection: you write the pattern twice
(once as `package`, again wherever another rule needs to reference it),
because there's nothing to reference — patterns aren't given identity.

### What makes each shape easy/hard for a human to author and debug

**go-arch-lint's named-component indirection is the more usable pattern for
kibitzer to copy**, for three concrete reasons that matter for the
requirements' "arbitrary allow/deny rules between named components" scope:

1. **A name is grep-able and typo-checkable; a repeated glob is not.**
   arch-go's `"**.impl.**"` has to be retyped correctly at every rule site
   that references it. If the project reorganizes a directory, every
   inlined pattern needs a find-and-replace across the whole rules list. A
   named component is declared once (`components: { domain: { in: ... } }`)
   and referenced everywhere by name — one place to fix, and (critically for
   error-message UX, see §2) the tool can statically check that every
   *reference* resolves to a *declared* name, which is not possible when the
   "identity" of a component is an unlabeled glob string compared
   structurally.
2. **allow-list-keyed-by-source (`deps: { component: { mayDependOn: [...] }
   }`) reads like the dependency graph itself** — a reviewer scanning the
   YAML top-to-bottom sees "domain may depend on: nothing; infra may depend
   on: domain" and it maps directly onto the mental layer diagram. arch-go's
   flat `dependenciesRules` list of `{package, shouldOnlyDependsOn,
   shouldNotDependsOn}` objects requires holding more state in your head to
   answer "what can X depend on" — you have to scan the whole list for every
   rule whose `package` might match X (globs can overlap ambiguously, e.g.
   `**.impl.**` and `**.foobar.**` might both match one file).
3. **Two rule verbs (`shouldOnlyDependsOn` = allow-list,
   `shouldNotDependsOn` = deny-list) that can theoretically both apply to the
   same package** is a footgun arch-go accepts and go-arch-lint avoids by
   having exactly one relationship shape (`mayDependOn`, an allow-list) plus
   `canUse` for vendor deps. Requirements ask for "arbitrary per-component
   allow/deny dependency rules" — if kibitzer supports both allow and deny on
   the same component, precedence must be explicit and documented (does deny
   win over allow when both list the same target? does a component with an
   empty/absent rule set mean "may depend on anything" or "may depend on
   nothing"?). go-arch-lint answers this by making the *absence* of a
   `deps` entry mean "no internal deps except commonComponents" — a safe
   default (deny-by-default) that fails closed. This is the more defensible
   default for kibitzer too, given it's advisory tooling meant to catch
   drift, not silently pass by default when a component is left
   unconfigured.

**Recommendation for kibitzer's schema** (not prescribing implementation,
just the shape a human would find easiest to author/debug, consistent with
existing `layers: Vec<String>` desugaring into the same internal model per
the requirements):

```jsonc
{
  "architecture": {
    "components": {
      "handlers": { "in": ["cmd/**", "internal/handlers/**"] },
      "domain":   { "in": ["internal/domain/**"] },
      "infra":    { "in": ["internal/infra/**"] }
    },
    "rules": {
      "handlers": { "mayDependOn": ["domain"] },
      "domain":   { "mayDependOn": [] },
      "infra":    { "mayDependOn": ["domain"], "mayNotDependOn": ["handlers"] }
    }
  }
}
```

— named components (glob-mapped, matching the requirements text exactly),
rules keyed by source component with explicit allow (`mayDependOn`) and
explicit deny (`mayNotDependOn`) lists, deny-by-default for anything not
declared. `layers: ["handlers", "domain", "infra"]` continues to parse and
desugars at load time into this same `components`/`rules` shape (each layer
name becomes a component whose `in` glob is the existing exact-segment-match
behavior, and `mayDependOn` is derived from list order) — so existing configs
don't break, per the requirements' explicit backward-compat constraint.

## 2. Error-message UX: config validation states

Kibitzer already validates config strictly and fails the whole run with
`anyhow::bail!` + a scoped error (see `src/config.rs:140-206`, `validate()`)
rather than warning and continuing — e.g. "unknown checker 'x' — run
`kibitzer check list` for available checkers" (`config.rs:169-176`). That's
the established tone: **specific, names the exact bad value, and tells the
reader the next action** (a command to run, or what to change). New
architecture-config errors should match this, not introduce a softer
"warning" tier out of nowhere — but two of the three cases below are
*semantically* different from "invalid config" (they're valid config that's
probably a mistake), so they need a different mechanism, not just a copy of
the `bail!` tone:

**a. A component glob matches zero files.**
This is not necessarily invalid — a fresh project might declare a component
for a directory that doesn't exist yet. But it silently means every rule
referencing that component can never fire (`mayDependOn`/`mayNotDependOn`
checks against an empty node set produce zero findings), which is exactly
the kind of "rule looks configured but is dead" trap the requirements
question flags. This should **not** be a hard validation error (that would
break the "declare ahead of the code" use case) — it should surface as an
*advisory line in the assessment output itself*, not just at config-load
time, because whether the glob matches is a function of the current file set
(`scope`-dependent), not static config shape:

```
[advisory] architecture config: component 'domain' (glob 'internal/domain/**') matched 0 files — rules referencing 'domain' will never fire
```

Emit this once per `architecture_assessment` / batch run, not once per file,
and emit it unconditionally (even if `domain` has zero rules referencing it)
so a typo'd glob is caught before rules are even written against it.

**b. A dependency rule references an undefined component name (typo).**
This one *is* a hard config error, not a soft advisory — unlike (a), there's
no legitimate reason to reference a name that was never declared; it's
either a typo or a stale rule after a rename. This should fail exactly like
the existing `unknown checker`/`unknown architecture checker` errors
(`config.rs:166-185`) — at config *load* time (`validate()`), not deferred to
assessment run time, so `kibitzer run` and the `PostToolUse` hook path (for
non-architecture checks sharing the same config file) fail fast on a broken
config rather than silently running with a rule that can never match:

```
.claude/inspect.json: architecture rule for component 'hanlders' references undefined component 'domain' in mayDependOn — declared components are: handlers, domain, infra (did you mean 'handlers'?)
```

A same-file typo (`hanlders` vs `handlers`) is exactly the case a Levenshtein-
distance "did you mean" suggestion earns its keep, the way `cargo`/`rustc`
already do for the same audience (Tyler, in a terminal) — cheap to implement
(edit distance ≤2 against the declared-name list) and it turns a "go re-read
the config" round trip into an instant fix.

**c. A naming rule's pattern matches zero types in the codebase.**
Same failure mode as (a) — silently-dead rule — but for a different reason:
the *pattern itself* isn't wrong, the codebase just doesn't have anything
matching it (e.g. a rule requiring `I`-prefixed interface names in a
codebase with none, or a rule scoped to a `scope` glob that doesn't overlap
with where the naming convention actually applies). Same treatment as (a):
advisory line in assessment output, not a load-time error, because
"zero matches" is legitimately sometimes correct on the first run (rule
written in anticipation of code that will be added later) or on a narrowed
`--scope` invocation:

```
[advisory] architecture config: naming rule 'exported-error-types' (pattern '^Err[A-Z]') matched 0 declarations in scope — rule may be dead, or scope may be too narrow
```

**General principle that ties a/b/c together**: undefined-name references
(b) are a config-shape error → fail fast at load. Zero-match conditions on
otherwise-valid patterns (a/c) are a config-content smell that's only
knowable against the actual repo state → report as an advisory finding
*inside* the assessment output the user is already going to read, not a
separate validation pass they have to remember to run. This also means the
zero-match advisory benefits from the same recommendation-block treatment
`recommendation_for()` already gives `import-cycles`/`layering`
(`mcp.rs:78-88`) — worth a one-line canned nudge ("check the glob/pattern
against `kibitzer check list` or a `find` on the repo") rather than a bare
fact.

## 3. CLI/tool output UX: new rule categories alongside existing findings

### Current finding shape

`ArchFinding { file: Option<PathBuf>, line: Option<usize>, message: String }`
(`architecture_checks.rs:13-17`) has no severity or category field of its
own — severity comes from the *check's* configured `Severity` in
`.claude/inspect.json`, applied uniformly to every finding a checker
produces (`mcp.rs:176-183`), and category is implicit in which
`architecture_checker` name produced it. Findings render as one
`[level] {file}:{line}: {message}` line each (`mcp.rs:180-183`), collected
into a flat list, `## Recommendations` appended after (deduped, alphabetized,
one canned string per triggered check), then `## Dependency graph` after
that (`mcp.rs:232-269`). This is deliberately flat and line-oriented — it's
built to be grep-able output an agent (or Tyler) skims, not a nested report.

Existing message conventions already self-tag by category via a bracket
prefix baked into the message text, inconsistently:
- `coupling` messages start with `[coupling] ...` (`architecture_checks.rs:153,168`)
- `import-cycles` and `layering` messages don't self-tag (`"import cycle: ..."`,
  `"layering violation: ..."`) — category is only recoverable from reading
  the message prose.

### Recommendation for content-rule and naming-rule findings

Two new rule categories are being added (package-content, naming-convention)
on top of the existing three (import-cycles, layering, coupling). At five
categories, the current implicit/inconsistent self-tagging stops scaling —
a flat findings block mixing five kinds of message prose is no longer
reliably scannable by eye. Two changes, both cheap and consistent with the
existing "flat, grep-able lines" philosophy rather than introducing a nested
report format:

1. **Make the `[coupling]`-style bracket prefix mandatory and uniform across
   all five categories** — `[import-cycle]`, `[layering]`, `[coupling]`,
   `[content]`, `[naming]` — inside the message itself (before the
   `[level]` tag that already wraps the whole line), so a finding line looks
   like:
   ```
   [advisory] [naming] internal/domain/order_service.go:14: exported type 'orderRepo' should be 'OrderRepo' or match pattern '^[A-Z]'
   [advisory] [content] internal/domain/http.go: package 'domain' contains 'http.Client' usage — content rule forbids HTTP clients in this component
   [blocking] [layering] internal/infra/db.go:22: infra (layer 'infra') imports internal/domain (layer 'domain') — 'infra' is declared as a lower layer than 'domain'
   ```
   This costs one `format!` change at each `ArchFinding` construction site
   and makes `grep '\[naming\]'` / `grep '\[content\]'` work immediately for
   an agent or a human filtering a large assessment. It also gives a stable
   machine-parseable token if kibitzer ever wants per-category counts without
   a full structured-output redesign.

2. **Extend the count line and add a per-category breakdown**, not just the
   existing single aggregate. Today: `architecture assessment: N finding(s)
   across M file(s)`. With five categories active at once, "12 findings"
   is much less actionable than a one-line breakdown immediately under it:
   ```
   architecture assessment: 12 finding(s) across 84 file(s)
     import-cycle: 1, layering: 3, coupling: 2, content: 4, naming: 2
   ```
   This is a small addition (tally by the same bracket-prefix token used in
   change 1) that pays for itself the moment content/naming findings are
   numerous relative to structural ones (naming conventions in particular
   tend to produce many low-severity findings on a first run against an
   existing codebase — see JTBD section below on switching cost).

3. **Content/naming findings should follow the same `{file}:{line}:
   {message}` convention wherever a real location exists** (a naming
   violation always has one — the declaration site; a content rule violation
   sometimes won't, e.g. "component X contains disallowed import Y" is a
   component-wide fact like `coupling`'s findings, which already set
   `file: None, line: None`). Don't invent a different shape for the new
   categories — reuse `ArchFinding` as-is; the existing `Option<PathBuf>`/
   `Option<usize>` already models the "sometimes graph-wide, sometimes
   pinned to a location" split the new categories need.

4. **`recommendation_for()` (`mcp.rs:78-88`) should grow entries for the new
   categories** — right now only `import-cycles`/`layering` get a canned
   `## Recommendations` line; `coupling` findings already embed their advice
   inline in the message ("consider splitting its responsibilities"). New
   content/naming checkers should follow whichever precedent fits per-finding
   specificity: a content-rule violation ("component X imports package Y, which
   its content rule forbids") is specific enough to embed inline like
   `coupling` does; a naming-convention violation is more repetitive
   across many findings (same pattern, many offenders) and would benefit
   from ONE `recommendation_for("naming")` line rather than repeating advice
   in every finding message.

## 4. Job-to-be-done: does this actually lower switching cost?

**The stated job**: "one persona linter instead of a different tool per
language/project" — replace `go install fe3dback/go-arch-lint` (or `arch-go`,
or a hand-rolled script) per-project with kibitzer, which Tyler already runs
everywhere via `.claude/inspect.json`/CLI/hook/LSP.

**What this feature does and doesn't change about switching cost:**

- **Genuinely lowers switching cost for the *config-authoring* step**, if
  the schema recommendation in §1 is adopted: named components + keyed
  allow/deny rules is not meaningfully harder to write than
  `.go-arch-lint.yml`'s `components:`/`deps:` blocks — it's the same shape,
  same amount of typing, in JSON5-ish JSONC instead of YAML (kibitzer's
  existing `inspect.json` convention, confirmed via `README.md`'s `jsonc`
  examples throughout). A user who already knows go-arch-lint's mental model
  ports it over almost mechanically. This is a real, load-bearing win: it
  means Tyler doesn't have to relearn a rule DSL per tool.
- **Does *not* by itself remove the requirement to write component/rule
  config from scratch per project** — neither kibitzer nor go-arch-lint nor
  arch-go can infer layers/components from an existing codebase automatically
  (none of the three tools researched has an "infer starter config" mode).
  So the *first* setup cost per new project is unchanged regardless of which
  tool is used — this feature doesn't reduce it, it only avoids paying a
  *second, different* learning cost when Tyler already knows the shape from
  a prior project. This is worth stating plainly rather than oversold: the
  win is "don't relearn a DSL and don't install a second binary," not "config
  authoring becomes free."
- **The multi-language import-graph extraction (Python/Java/Kotlin) is the
  piece that actually changes behavior, not just theory** — this is the part
  that makes "one tool for everything" true rather than aspirational.
  Without it, Tyler would still need go-arch-lint (Go-only) *and* a Java/Kotlin-
  specific tool (e.g. ArchUnit, konsist) *and* something else for Python
  (e.g. import-linter) to get equivalent coverage across a polyglot set of
  projects — precisely the "different tool per language/project" problem
  the requirements name. If this research pass finds (via the architecture
  research agent, not this one) that Python/Java/Kotlin import extraction is
  scoped thin (e.g. Java/Kotlin package resolution without full classpath
  awareness, which is a known hard problem those ecosystems' own tools
  invest heavily in), the JTBD claim weakens proportionally — the value of
  "one persona linter" is gated on the import graphs for the new languages
  being trustworthy enough to build allow/deny rules against, not just
  present. **This is a flag for the architecture/tech-stack research
  dimension to confirm, not something resolvable from the UX surface alone**
  — but from a UX standpoint, if Java/Kotlin extraction is heuristic
  (e.g. best-effort import-statement parsing without full symbol
  resolution), the zero-match advisory from §2c and a documented
  "known limitations" section (kibitzer already has this pattern —
  `docs/output-formats.md`'s "Known limitation: no diff-aware scoping yet"
  section) become load-bearing UX, not optional polish: a user who writes a
  naming/content rule against a language extraction that's silently
  incomplete needs the tool to tell them their rule might not be seeing
  everything, the same way SARIF's known-limitation section already does
  for a different gap.
- **Switching-cost honesty check**: is adopting kibitzer's architecture
  linting in a *new* project meaningfully easier than `go install
  fe3dback/go-arch-lint`? For a Go-only project, **no** — go-arch-lint is a
  single `go install`, one YAML file, and a mature, Go-specific tool with
  years of exactly-Go-shaped ergonomics (`vendors:`/`canUse` modeling
  stdlib vs. third-party vs. internal separately, which kibitzer's schema
  as scoped in the requirements doesn't appear to distinguish). The genuine
  win is **cross-project, cross-language consistency and the existing
  kibitzer integration surface** (hook/MCP/LSP/CLI Tyler already has wired
  up) — not that kibitzer's Go-specific architecture linting is better than
  go-arch-lint's. Frame the value prop accurately: this feature is worth
  building because it consolidates tooling Tyler already maintains
  elsewhere, not because it's a superior architecture linter on a
  per-language basis. That's a fine and sufficient justification given the
  stated problem statement ("persona linter for everything"), but a
  plan/pitch that claims per-language superiority would be overclaiming.

## Sources

- [fe3dback/go-arch-lint — `.go-arch-lint.yml`](https://github.com/fe3dback/go-arch-lint/blob/master/.go-arch-lint.yml) (raw config fetched verbatim)
- [fe3dback/go-arch-lint — README](https://github.com/fe3dback/go-arch-lint)
- [fdaines/arch-go — README](https://github.com/fdaines/arch-go)
- Local: `src/config.rs`, `src/architecture_checks.rs`, `src/mcp.rs`, `src/mermaid.rs`, `src/import_graph.rs`, `src/glob.rs`, `README.md`, `docs/output-formats.md` (all read in full or targeted-range, `tstapler/kibitzer` repo, current working tree)
