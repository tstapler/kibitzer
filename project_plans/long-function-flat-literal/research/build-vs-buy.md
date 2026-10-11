# Build vs. Buy: long-function-flat-literal

## Decision: Build (extend existing)

This is a precision fix to an in-house static analysis rule backed by tree-sitter grammars
kibitzer already vendors. There is nothing to "buy" — no external library provides
"per-language Go/TS/Python composite-literal body exemption for a configurable
line-length check." The decision space is:

1. **Build the predicate ourselves** (recommended): ~25 lines of Rust, one new
   `LangRuleConfig` field, uses tree-sitter node traversal the codebase already does
   extensively.
2. **Workaround via existing suppression mechanisms**: users add a `scope` glob to
   `.kibitzer/inspect.json` or a per-finding entry to `.kibitzer/accepted/`. This
   silences the entire `syntax-rules-*` checker for a path or requires per-finding
   maintenance. Documented in `docs/syntax-rules.md:160–167` as the current escape
   hatch for `replace-magic-literal` table-test noise — the same escape hatch that the
   filed item correctly identifies as disproportionate for this case.
3. **Do nothing**: leave users with workaround option 2.

Option 2/3 impose ongoing user friction for a structural property the tool can detect
directly. Build is the right call.

## Reuse opportunities inside the codebase

- `cfg.statement_container`, `cfg.return_kind`, `cfg.nesting_kinds`, `cfg.call_kinds`,
  `cfg.if_kind`, `cfg.function_kinds` — all already present, no new fields needed
  beyond `collection_literal_kinds` (or the derived-from-`density_boundary_kinds` variant
  per `research/architecture.md` Option A).
- The node-walk pattern (recurse, check kind membership) is identical to what
  `max_nesting_depth` already does — no new traversal infrastructure.
- The test pattern (inline source string, `check_source(&src)`, assert on `message`) is
  the repo standard; no new test infrastructure.

## External tools checked and ruled out

- **Tree-sitter playground** (for verifying node kind names): useful for grammar
  verification during implementation, not a build dependency.
- **SonarQube / ESLint / staticcheck**: no applicable rule or plugin to import; their
  line-counting approaches differ from kibitzer's (see `research/features.md`).

**Verdict**: Pure in-house build, reusing existing infrastructure throughout.
