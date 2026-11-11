# Research: pull-push-hierarchy-checks

## Codebase findings

### Dependency status (re-verified against HEAD)

The issue description says these blockers must ship first:

| Dep | Claim | Actual status |
|---|---|---|
| #40 type-hierarchy graph | Blocker for all five | **Already shipped** — `ArchModel.type_edges: Vec<TypeRelationEdge>` exists; `hierarchy_checks.rs` helpers (`type_index`, `resolved_extends`, `supertype_ids`, `methods_of`) already consume it. |
| #39 field read/write tracking | Needed for Push Down Field | **Already shipped** — `ArchModel.field_accesses: Vec<FieldAccessEdge>` exists; `encapsulate_variable.rs` and `remove_setting_method.rs` already consume it. |
| #33 call-graph edges | Needed for Push Down Method | **Already shipped** — `ArchModel.call_edges: Vec<CallEdge>` exists; `feature_envy.rs`, `single_call_site_delegation.rs`, and `unreferenced_symbols.rs` already consume it. |

**Conclusion**: All stated blockers are resolved. This item can be implemented now.

### Remaining model gaps

**Gap 1 — `SymbolNode.end_line` is absent.**

`SymbolNode` (`src/arch_model.rs:67`) has `line: usize` (declaration start)
but no `end_line`. Method body-hashing for Pull Up Method / Pull Up Constructor
Body needs to slice the source between `line` and `end_line`. Without it, we
can compare method names but not bodies (a weaker signal, same as the name-
only approach `collapse-hierarchy` already uses for a different purpose).

Option A — Add `end_line: Option<usize>` to `SymbolNode` (populated by
`extract_symbols_for_file` via tree-sitter's `node.end_position().row`). Zero
cache-compat risk with `#[serde(default)]`.

Option B — Skip body-hashing; detect same-name methods on siblings and report
"consider whether these have the same body". Higher false-positive risk; defers
the body-equality gate to a future PR.

This plan adopts **Option A** because tree-sitter already knows end positions,
the change is mechanical, and it makes the Pull Up finding actionable rather
than advisory-noise.

**Gap 2 — Field declarations are not stored in `ArchModel`.**

`build_model` (`src/arch_model.rs:434`) builds
`struct_fields: HashMap<(String, String), HashSet<String>>` during the file
walk for the purpose of resolving field-access edges, then discards it. Pull
Up Field and Push Down Field need a per-type field-name set at checker time.

The fix is straightforward: expose `struct_fields` as
`ArchModel.struct_field_names: BTreeMap<String, BTreeSet<String>>` (key =
`"{pkg}::{type}"`, value = sorted field-name set). `#[serde(default)]` for
cache compat. The data is already computed; this is a retention-only change.

### How existing checkers handle similar problems

`duplicate_code.rs` (`src/checkers/duplicate_code.rs`) uses a windowed
line-hash approach: it slides a `MIN_BLOCK_LINES`-wide window, skips blank
lines, and fires when ≥3 windows hash identically. For Pull Up Method, the
"block" is the entire method body; the window covers it wholesale. The hashing
strategy (trimmed-line concatenation keyed by a rolling hash) is reusable
verbatim once `end_line` is available.

`collapse_hierarchy.rs` compares sibling method *names* only. Pull Up Method
goes one step further: same name *and* same body. The added body-equality gate
is what distinguishes "both subtypes override a method differently" (expected)
from "both subtypes have an identical method that belongs on the parent" (the
smell).

`encapsulate_variable.rs` / `remove_setting_method.rs` consume `field_accesses`
exactly the way Push Down Field will: filter `field_accesses` by `to` prefix
(`"pkg::Type.field"`) and inspect the `from` ids. The same pattern applies to
Push Down Method using `call_edges`.

### Language coverage

| Language | type_edges | call_edges | field_accesses | end_line (after gap fix) | Constructor detection |
|---|---|---|---|---|---|
| Go | ✅ (embedding) | ✅ | ✅ | ✅ | `New*` by convention, not keyword |
| TypeScript/JS | ✅ | ✅ | ❌ | ✅ | `constructor` keyword |
| Java | ✅ | ✅ | ❌ | ✅ | Same name as class |
| Kotlin | ✅ | ✅ | ❌ | ✅ | `constructor`/primary constructor |
| Rust | ❌ (no inheritance) | ✅ | ❌ | ✅ | N/A |

Field_accesses is Go-only today. Push Down Field and Pull Up Field are
therefore Go-only until #39 expands. Pull Up/Push Down Method and Pull Up
Constructor Body are broader (all OOP languages with type_edges + call_edges).

### False-positive risks and mitigations

**Pull Up Method**:
- Template method pattern: intentional override in every subtype. Mitigation:
  skip if the supertype already declares the same-named method (it's an override
  contract, not duplication).
- Covariant return / parameter specialization: same body text but different
  signature. Mitigation: include the first line of the method declaration in
  the hash (signature + body, not body alone).

**Push Down Method**:
- A method used only internally (within the supertype itself) may show zero
  external callers, but it's not a push-down candidate. Mitigation: only fire
  when at least one resolved caller exists, and that caller belongs to exactly
  one subtype.
- A supertype method called from tests only: tests are excluded by
  `is_ignorable_file`, so test-only callers won't trigger this.

**Push Down Field**:
- A field declared but never accessed shows no field_access evidence. Same
  no-evidence stance as other checkers: skip, don't flag.
- A field accessed only from one subtype *and* from the supertype's own
  constructor: the supertype constructor call site is `from=pkg::Super.Init`
  which is not a subtype method. Mitigation: exclude accesses from the
  supertype's own methods entirely (they're not evidence of single-subtype
  usage).

**Pull Up Field**:
- Same field name, different type (e.g. `count int` vs `count int64`): report
  with a "consider checking type compatibility" note. Do not suppress — the
  name match is the signal; type mismatch is a reviewer concern.

### Backtest expectations

No transcript-replay findings are expected for these (batch-only, whole-repo
checkers over hierarchy data that rarely appears in per-file snapshots).
Corpus backtest is the meaningful signal — the same pattern as the existing
hierarchy checks. Target: ≤10 findings across all five checkers on the
standard corpus, with ≤30% judged false positives after manual triage.

## Prior art and literature

- Fowler, M., *Refactoring*, 2nd ed., Ch. 12 — Pull Up Method (p. 350), Pull
  Up Field (p. 353), Pull Up Constructor Body (p. 355), Push Down Method
  (p. 359), Push Down Field (p. 361).
- Palomba, F. et al., "Textual analysis for code smell detection," IEEE TSE
  2018 — demonstrates that body-level duplicate detection (not just name
  matching) substantially reduces false negatives on the Pull Up smell.
- The `docs/refactoring-catalog-analysis.md` entry for issue #59 is the
  original design; this research validates that its stated deps are met and
  adds the two model gaps not mentioned there.
