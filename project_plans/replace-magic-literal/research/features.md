# Research: Feature landscape for `replace-magic-literal`

Agent 2 (Features) — SDD Phase 2, project `replace-magic-literal`.

## 1. How ESLint, SonarQube, and PMD actually scope this

The three tools cited in the issue (`docs/refactoring-catalog-analysis.md:252`) do **not**
all implement the same rule shape. There are two distinct families, and kibitzer's proposed
rule (repeat-count ≥2, numbers *and* strings, one shared allow-list) is a hybrid of both —
not a copy of any single tool's default.

### Family A: per-occurrence magic-number check (no repetition threshold)

- **ESLint `no-magic-numbers`** ([docs](https://eslint.org/docs/latest/rules/no-magic-numbers)):
  flags *every* numeric literal not explicitly allowed — there is no "repeated ≥N times"
  gate at all; a single occurrence is enough. Numbers only, no strings.
  - `ignore: number[]` — explicit allow-list, **empty by default**. Confirmed via
    [eslint/eslint#8052](https://github.com/eslint/eslint/issues/8052) and
    [eslint/eslint#4193](https://github.com/eslint/eslint/issues/4193): ESLint used to
    default `ignore` to `[0, 1, 2]` and deliberately changed it to `[]` — **0 and 1 are
    flagged by default in current ESLint**, unlike PMD/SonarQube below.
  - `ignoreArrayIndexes` (default false) — permits literal array indices 0..4294967294.
  - `ignoreDefaultValues` (default false) — permits numbers in default parameter values /
    destructuring defaults (`function map(concurrency = 3)`).
  - `ignoreClassFieldInitialValues` (default false) — permits numeric class field initializers.
  - `enforceConst` (default false) — when a magic number is pulled into a constant, requires
    `const` (not `let`/`var`).
  - `detectObjects` (default false) — off by default; object property values (`{ tax: 0.25 }`)
    are otherwise *not* flagged at all.
  - `typescript-eslint`'s fork adds `ignoreEnums`, `ignoreNumericLiteralTypes`,
    `ignoreReadonlyClassProperties`, `ignoreTypeIndexes` — all TS-type-system edge cases with
    no tree-sitter/syntax-only equivalent.

- **SonarQube `S109` (Java/C#/TS "Magic numbers should not be used")**: also per-occurrence,
  no repetition threshold. Confirmed against the real fixture
  (`sonar-java/java-checks/src/test/files/checks/MagicNumberCheck.java` on
  [SonarSource/sonar-java](https://github.com/SonarSource/sonar-java/blob/master/java-checks/src/test/files/checks/MagicNumberCheck.java)):
  - `-1`, `0`, `1` exempt by default (all numeric types: int/long/float/double).
  - Exempt when the literal is the initializer of a `final`/`static final` field, a local
    `final` variable, an enum constant argument, or an interface constant.
  - Exempt inside array/collection initializers and inside a constructor-call argument list
    (`new ArrayList<>(42)`, `new Long(3L)`, `ByteBuffer.allocateDirect(8)`) — i.e. "literal as
    an API argument" is broadly exempt, not just a fixed allow-list of values.
  - Exempt when returned directly from a `hashCode()` override (a documented, narrow
    carve-out for a specific idiom, not a general "any literal in any override" rule).
  - Community threads ([SonarSource/sonar-dotnet#1356](https://github.com/SonarSource/sonar-dotnet/issues/1356))
    show recurring false-positive reports on enum-ordinal-style code even with the
    documented exemption — a signal that "is this literal legitimately enum-like" is a
    perennial soft spot for this family of rule, independent of implementation.

### Family B: repeated-literal / duplicate-literal check (repetition threshold, closer to the issue's actual shape)

- **SonarQube `S1192` ("String literals should not be duplicated")** is the tool in this
  landscape structurally closest to what the issue actually asks for — repetition-gated,
  not per-occurrence:
  - Default threshold: flags a string literal duplicated **≥3 times** in a file (not 2).
  - Excludes literals **shorter than 5 characters** (its own effective "trivial value"
    allow-list, separate from a fixed value list).
  - Excludes a literal that matches a parameter name, and (per Sonar Community reports)
    excludes literals used in annotations/attributes.
  - Numbers are explicitly **out of scope for S1192** — duplicate numeric literals are
    S109's job (per-occurrence), not S1192's (repetition-gated). No SonarQube rule combines
    "numeric or string" with "repetition-gated" the way this issue's design does.

- **PMD `AvoidDuplicateLiterals`** (Java, `errorprone` category —
  [source](https://github.com/pmd/pmd/blob/main/pmd-java/src/main/java/net/sourceforge/pmd/lang/java/rule/errorprone/AvoidDuplicateLiteralsRule.java)):
  string-only, repetition-gated, but far more permissive than this issue's proposal:
  - `maxDuplicateLiterals` default **4** — i.e. a literal must appear **5+ times** before
    it's flagged, not 2.
  - `minimumLength` default 3 characters.
  - `skipAnnotations` (default false) — historically buggy/ignored per
    [yegor256/qulice#785](https://github.com/yegor256/qulice/issues/785), a specific known
    footgun (annotation string arguments like `@SuppressWarnings("unchecked")` double-counting).
  - `exceptionList` — free-form list of literal values to exempt, the general form of a
    fixed allow-list.
  - PMD's numeric equivalent, `AvoidLiteralsInIfCondition`, is **scoped only to `if`
    conditions** (not the whole file) and exempts `-1`/`0` by default via
    `ignoreMagicNumbers` (configurable), with a separate `ignoreExpressions` (default
    `true`) controlling whether a literal inside a larger expression like `pos + 5` counts.

**Implication for this issue**: kibitzer's proposed design (repeat ≥2, numbers *and*
strings together, one allow-list, no length/annotation/parameter-name carve-outs) is
stricter on the threshold (2, vs. 3 for S1192 and 5 for PMD) but simpler in scope than every
tool surveyed — none of them merge numbers and strings under one repetition-gated rule with
a single fixed allow-list. The 2× threshold is the most aggressive of any reviewed tool,
which directly supports the requirements doc's own call-out that false-positive rate at
scale is "the honest unknown" (`requirements.md`'s Appetite section) — there's no existing
tool's default to point to as calibration evidence for "2 is right," only "everyone shipping
a repetition-gated version chose 3+ or higher."

## 2. Precedent already in kibitzer's own codebase

- `src/rules.rs`'s `CATALOG` (`src/rules.rs:34-65`) currently has zero literal-related node
  kinds anywhere in the file — confirmed via `grep -n "number_literal\|string_literal\|integer_literal\|float_literal" src/rules.rs` returning nothing. This is genuinely new surface in `LangRuleConfig`
  (`src/rules.rs:72-144`), not an extension of an existing field.
- All five existing `CATALOG` rules are `Severity::Advisory` (`src/rules.rs:34-65`), and
  every other native default check in `config::default_checks()` that could plausibly be
  a peer (`primitive-obsession`, `duplicate-code`, `file-complexity`) is also
  `Severity::Advisory` (`src/config.rs:613-618`) — no default native check is
  `Severity::Blocking` except `markdown-link-integrity` (`src/config.rs:601-604`).
  `replace-magic-literal` should be Advisory to match every structurally-similar peer.
- **Message-wording precedent, directly on point**: `duplicate-code`
  (`src/duplicate_code.rs:100-122`) is kibitzer's existing "same thing repeated ≥N times in
  one file" check (`MIN_OCCURRENCES = 3`, `src/duplicate_code.rs:22`, for 6-line blocks,
  `MIN_BLOCK_LINES`, `src/duplicate_code.rs:13`) and its finding message is:
  ```
  "{MIN_BLOCK_LINES}-line block repeated {n} times (lines {a, b, c}) — consider extracting a shared function"
  ```
  This embeds **every occurrence's line number** in the message text itself, joined by
  commas — because `Finding` (`src/checker.rs:12-15`) only carries one `line: usize` field,
  there's no structured multi-location output, so the convention for "here's where the
  other copies are" is to inline it into the message string. `replace-magic-literal` should
  follow this exact convention (e.g. `"literal 42 repeated 3 times (lines 12, 45, 90) —
  consider a named constant"`), not just report the last occurrence with no cross-reference.
  The `unreachable-code` rule (`src/rules.rs:788-792`) does the analogous single-line-back-reference
  (`"...an unconditional `return` on line {term_line} ends this block first"`), reinforcing
  that inlining a second line number into the message is the established pattern whenever a
  finding logically involves more than one location.
- **Actionable-suggestion precedent**: every existing `CATALOG` rule's message ends in a
  concrete, generic remediation clause, not just a smell name:
  `long-function` → "consider splitting it up"; `deep-nesting` → "consider extracting a
  function or inverting a condition"; `long-parameter-list` → "consider a config struct";
  `flag-argument` → "split into two named functions or replace with a small enum"
  (`src/rules.rs:824-869`). None of them say *where exactly* to put the fix (no file/line
  suggestion for the new function, no proposed struct name) — the guidance is generic
  ("a config struct," "a shared function"), not location-specific. This directly bounds
  question 4 below.
- `docs/suppressing-checks.md:6-7` lists the exact set of names in the default catalog that
  a repo can `disabled` or override — `replace-magic-literal` needs to be added there per
  AC 9, alongside the `docs/syntax-rules.md` per-rule table (already lists the five existing
  rules with Rule ID / Category / Default severity / Threshold / Description columns —
  `docs/syntax-rules.md:26-32` — the new row should match that exact table shape).
- `docs/accepting-findings.md:46-47` distinguishes checkers whose messages **self-prefix**
  their rule id (`[long-function]`, `[flag-argument]`, etc. — see the literal `format!`
  strings above, all of which start with `[rule-id]`) from ones that don't
  (`primitive-obsession`, `duplicate-code`, `file-complexity`, the `go-*` checks).
  `replace-magic-literal` is a `SyntaxRulesChecker` rule living alongside the bracketed
  ones, so its message must start with `[replace-magic-literal]` to match its siblings —
  `duplicate-code`'s un-bracketed convention above is a wording-shape precedent, not a
  prefix-convention one.
- No existing kibitzer check does file-scoped (not per-declaration) literal-value
  bookkeeping, so there's no shared helper to reuse for "collect all literal text values
  across the whole tree, group, count." The nearest cousin is `duplicate_code.rs`'s
  windowed-line grouping — same overall shape (`HashMap` bucket by normalized value →
  `Vec<line>`, filter buckets by count) but over source lines instead of AST literal nodes.

## 3. Edge cases this check will hit in real code

Per the issue's own stated allow-list (`0`, `1`, `-1`, `""`, empty collection literals) and
the "referenced-elsewhere-by-name const/let" exclusion, mapped against what the landscape
tools above found necessary in practice:

**In scope, and the landscape confirms these need explicit handling:**
- **HTTP status codes** (`200`, `404`, `500`) repeated across a file — not covered by the
  0/1/-1 allow-list, will fire. This is exactly the kind of case Sonar's community threads
  flag repeatedly for S109/enum-ordinal-style values; kibitzer has no enum/status-code
  carve-out planned, so this is a known, accepted trade-off, not an oversight — worth
  stating explicitly in the plan doc so it isn't "discovered" during backtest triage.
- **Table-driven test fixtures**: arrays of test cases with repeated field values (counts,
  expected results, repeated small integers) are the single highest-volume source of
  legitimate repetition in real Go/Rust/Java codebases (kibitzer's own `#95`, "native Go
  check for table-driven-test candidates," and the `go_table_driven_test.rs` fixture file
  in this repo's own `src/` are evidence this pattern is common and already a checker
  concern here). The requirements doc's own "Suppression convention correction" section
  (`requirements.md:46-56`) already resolves this: **not** an in-check allow-list expansion,
  but a `scope` exclusion glob in `.claude/inspect.json` or `.kibitzer/accepted/` for test
  fixture files. This needs to be called out prominently in the plan/docs, or every
  table-driven-test-heavy repo in the backtest corpus (`kubernetes/kubernetes`,
  `apache/cassandra` — both famously table-test-heavy) will show a wall of findings at
  first run.
- **Array indices** (`arr[0]`, `arr[1]`) — `0`/`1` are already in the allow-list, so the
  single most common index values are covered "for free." Larger fixed indices (`arr[2]`,
  `arr[3]`) repeated across a file are not covered and will fire — ESLint's
  `ignoreArrayIndexes` exists precisely because this is a known false-positive source, and
  kibitzer's proposed design has no equivalent knob. Worth flagging as a likely backtest
  finding rather than pre-building the carve-out speculatively (per the Appetite section's
  own philosophy: "derisk via backtest, not speculative pre-analysis").
- **Negative numbers**: `-1` is allow-listed; other negative literals (`-100`) are not.
  Implementation note (not a design gap): whether tree-sitter represents `-1` as a single
  literal node or a `unary_expression` wrapping a positive literal is **grammar-specific**
  and needs verification per language the same way every other `LangRuleConfig` field in
  this file already is (`src/rules.rs:67-70`'s own stated verification discipline) — get
  this wrong and `-1` silently fails to match the allow-list on some subset of the 8
  languages.
- **Floating-point literals used in comparisons**: no floating-point-specific allow-list is
  proposed (unlike PMD's `AvoidLiteralsInIfCondition`, which has a Sonar-Community-reported
  open question about float exceptions — see
  [Sonar Community: "C/S109: Magic Number Exceptions for Floats"](https://community.sonarsource.com/t/c-s109-magic-number-exceptions-for-floats/187689)).
  Treating float literals as ordinary literal text (not doing float-equality-aware bucketing
  like `1.0` vs `1.00`) is a reasonable simplification for v1 given this check is textual/AST,
  not semantic — but it means `1.0` and `1` are different buckets even if a language treats
  them as the same value, which is fine (undercounting, not overcounting — a false negative,
  not a false positive) and safely deferrable.

**Out of scope per the issue's own stated design, and correctly so:**
- **String concatenation / literal composition** (Python f-strings, JS template literals,
  Kotlin string templates): these are typically **not** the same tree-sitter node kind as a
  plain string literal (template literals are usually a distinct node kind wrapping
  substitution expressions), so they naturally fall outside a plain
  `string_literal`/`number_literal` node-kind walk without any special-casing needed — this
  is a "correctly excluded by construction" case, not a gap to patch. Confirm in
  implementation by checking each grammar's node kind for template/f-string nodes are
  distinct from plain string literal kinds (expected, based on this file's own documented
  verification discipline for every other node-kind field).
- **Version strings** (`"1.2.3"`) repeated in a file: in scope for the literal allow-list
  and repetition logic exactly like any other string — the issue names this as an explicit
  SonarQube-style exception in some rule sets, but kibitzer's proposed design has **no
  version-string carve-out**, and the acceptance criteria don't ask for one. This is a
  deliberate scope-narrowing already made in `requirements.md`, not an open question for
  this research to resolve.
- **Format-string literals** (`"%s: %d"`, `"{}: {}"`) repeated across a file: same as any
  other string literal — no special handling proposed or needed; a repeated format string
  really is a case for a named constant, so this is arguably a case the check should
  legitimately catch, not an edge case to exempt.
- **Enum-like repeated string tags** (`"pending"`, `"active"` used as ad hoc string-typed
  status values): explicitly the type of thing `primitive-obsession`
  (`docs/refactoring-catalog-analysis.md:221`, "Replace Primitive with Object," shipped for
  Go) already targets from a different angle (type-based, not repetition-based). Overlap
  between the two checks on this category of string is expected and acceptable — they
  approach the same underlying smell from different signals, similar to how `long-function`
  and `deep-nesting` can both fire on the same bloated function.

## 4. Unstated maintainer needs beyond the literal requirements text

- **Where to put the constant**: kibitzer's own precedent (section 2 above) is uniformly
  generic ("consider a named constant" / "consider a shared function"), never
  location-specific. Given every sibling `CATALOG` rule follows this pattern, the
  maintainer's actual expectation is almost certainly "flag it, suggest a constant,
  don't try to pick a location" — proposing a specific insertion point (top of file? same
  scope as first occurrence?) would be scope creep relative to every existing rule in this
  file, not filling a real gap.
- **Cite every occurrence's line, not just one**: `Finding` has one `line` field
  (`src/checker.rs:12-15`), and `duplicate-code`'s established convention
  (`src/duplicate_code.rs:113-119`) is to embed every occurrence's line number into the
  message text. A `replace-magic-literal` finding that reports only the *last* occurrence
  with no back-reference to the others would be a regression in usability relative to the
  existing bar — a maintainer reading one finding line should not have to grep the file for
  the literal's other appearances. This should be treated as an implicit acceptance
  criterion even though the requirements doc's AC list doesn't spell it out.
- **A path to "I looked at this and it's fine" without inventing new suppression syntax**:
  already resolved correctly in `requirements.md`'s "Suppression convention correction"
  section — worth reinforcing here only insofar as backtest-triage tooling
  (`scripts/backtest-triage.py`) is the maintainer's actual mechanism for tracking "this
  fired, I decided X" at corpus scale, not a new inline comment. No new work implied beyond
  what AC 10 already states.
- **Distinguishing "correctly factored" bindings across scopes**: the requirements text says
  "referenced elsewhere by name" for the const/let exclusion, but doesn't specify whether
  "elsewhere" means anywhere in the file (simple, matches this check's whole-file scope) or
  specifically outside the binding's own declaring scope (more correct, more work). Given
  this checker's own stated file-scoped design (`requirements.md:33-36`, unlike the
  per-declaration `long-function`/`deep-nesting`/`long-parameter-list`), the simplest
  faithful reading is "referenced by name anywhere else in the file's AST, no scope
  resolution" — consistent with `collect_condition_identifiers`'s own documented choice
  (`src/rules.rs:869-876`) to deliberately skip scope resolution ("this check is a
  mechanical, low-false-positive heuristic, not a scope-resolving analysis") for
  `flag-argument`. The planning phase should make this an explicit, stated simplification
  rather than leave it ambiguous, since getting it wrong either direction changes the
  false-negative/false-positive balance.

## Sources

- ESLint `no-magic-numbers`: https://eslint.org/docs/latest/rules/no-magic-numbers
- ESLint default-`ignore`-value history:
  [eslint/eslint#8052](https://github.com/eslint/eslint/issues/8052),
  [eslint/eslint#4193](https://github.com/eslint/eslint/issues/4193)
- typescript-eslint `no-magic-numbers`: https://typescript-eslint.io/rules/no-magic-numbers/
- SonarQube S109 fixture (ground truth for compliant/noncompliant cases):
  https://github.com/SonarSource/sonar-java/blob/master/java-checks/src/test/files/checks/MagicNumberCheck.java
- SonarQube S109 enum false-positive report:
  [SonarSource/sonar-dotnet#1356](https://github.com/SonarSource/sonar-dotnet/issues/1356)
- SonarQube S109 float exceptions discussion:
  https://community.sonarsource.com/t/c-s109-magic-number-exceptions-for-floats/187689
- SonarQube S1192 threshold/exceptions: community discussion at
  https://community.sonarsource.com/t/improve-filtering-on-s1192-duplicate-strings/176824
- PMD `AvoidDuplicateLiteralsRule` source:
  https://github.com/pmd/pmd/blob/main/pmd-java/src/main/java/net/sourceforge/pmd/lang/java/rule/errorprone/AvoidDuplicateLiteralsRule.java
- PMD `AvoidDuplicateLiterals` `skipAnnotations` bug report:
  [yegor256/qulice#785](https://github.com/yegor256/qulice/issues/785)
- PMD `AvoidLiteralsInIfCondition` (`ignoreMagicNumbers`, `ignoreExpressions`) reports:
  [pmd/pmd#2140](https://github.com/pmd/pmd/issues/2140),
  [pmd/pmd#388](https://github.com/pmd/pmd/issues/388)
- In-repo: `project_plans/replace-magic-literal/requirements.md`, `src/rules.rs`,
  `src/config.rs`, `src/duplicate_code.rs`, `src/checker.rs`,
  `docs/refactoring-catalog-analysis.md`, `docs/syntax-rules.md`,
  `docs/suppressing-checks.md`, `docs/accepting-findings.md`
