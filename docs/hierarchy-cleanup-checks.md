# Hierarchy cleanup checks

Eight advisory, batch-only, whole-repo checks over `ArchModel.type_edges` (resolved `Extends`
edges only; unresolved targets are ignored). Suppress with `kibitzer:ignore <rule> -- <why>`;
see `suppressing-checks.md`.

| Check | Fowler | Fires when | Languages |
|---|---|---|---|
| `remove-subclass` | Remove Subclass | Leaf subtype; ≥1 method, every one `SymbolNode::is_constant_return`; no field of the subtype is read by any method | Go, TS/JS (Java/Kotlin never fire: `is_constant_return` is always false) |
| `extract-superclass` | Extract Superclass | Two same-package types, no edge between them, ≥ `MIN_METHODS_FOR_CLUSTERING` (4) methods with field accesses each, Jaccard of accessed field *names* ≥ 0.5, ≥ 2 shared names; top 3 per package | Go only (field-access data) |
| `collapse-hierarchy` | Collapse Hierarchy | Resolved `Extends` edge; subtype has ≥ 3 methods and < 20% of them are absent by name from its direct supertype | Go, TS/JS (Java/Kotlin subtypes skipped, see below) |

| `push-down-method` | Push Down Method | Supertype method (≥2 subtypes) whose resolved callers, via the caller's own receiver, are all in one direct subtype's subtree | Go, TS/JS (call graph) |
| `push-down-field` | Push Down Field | Unexported supertype field accessed only from one direct subtype; no access from the supertype or outside | Go only |
| `pull-up-method` | Pull Up Method | ≥2 sibling subtypes (a majority) with a same-signature method whose normalised bodies are identical; supertype lacks it | Go, TS/JS, Java, Kotlin |
| `pull-up-constructor-body` | Pull Up Constructor Body | Same as `pull-up-method`, constructors only (`constructor`; Java name == type) | TS/JS, Java, Kotlin |
| `pull-up-field` | Pull Up Field | A majority of subtypes declare a field with the same name and type text; supertype lacks it | Go only |

The last five have their own false-positive pages: `push-down-method-false-positives.md`,
`push-down-field-false-positives.md`, `pull-up-method-false-positives.md`,
`pull-up-constructor-body-false-positives.md`, `pull-up-field-false-positives.md`. Measured rates (every hit read by one reviewer; standard corpus plus ten held-out Go repos): push-down-method 0/3 (0 corpus
hits, 3 held-out); push-down-field 4/20; pull-up-method 0/21; pull-up-constructor-body 1/15 (corpus only); pull-up-field 5/34
overall, 3/15 = 20.0% on the held-out repos alone (the corpus rate is optimistic because two guards were tuned on it). Samples are
small. They need `SymbolNode::end_line`, `ArchModel::struct_field_names`/`struct_field_types`
and `promoted_field_accesses`, added with them.

Guards added after backtesting and code review (not in the original issue). All three skip test and `fixtures/`/`testdata/` files: `extract-superclass` skips test
files, same-named types (build-tag variants) and single-shared-field pairs; `collapse-hierarchy`
skips same-named pairs, subtypes with < 3 methods and `.java`/`.kt` subtypes; `remove-subclass` skips same-named pairs.

Known false-negatives: a declared-but-never-read field is invisible to `remove-subclass`;
method comparison is by name, not signature; no cross-package or transitive comparison.

## Backtest (release build, 2026-11-04)

Transcript replay (`kibitzer check backtest <name>`, 2038 snapshots): **0 findings** for all
three. Expected: these are whole-repo model checks and per-file edit snapshots rarely carry a
hierarchy; this shows no noise, not recall.

Corpus (`kibitzer check architecture <name> <repo>`), final build:

| Repo | remove-subclass | extract-superclass | collapse-hierarchy |
|---|---|---|---|
| kubernetes/kubernetes | 1 | 11 (after guards; 18 before) | 7 (41 before guards) |
| tstapler/stapler-squad | 0 | 2 | 1 |
| apache/cassandra | 0 | 0 | 80 (213 before guards) |
| microsoft/vscode | 3 | 0 | 0 |
| ripgrep, deno, servo, nowinandroid | 0 | 0 | 0 |

The 'before' numbers are from the first run (kubernetes only for extract-superclass), before the guards above.

### Triage (manual read of the findings; small n, judgement calls)

- **remove-subclass** (4 hits read): `RebaseUpstreamItem`, `NonCollapsibleObjectTreeModel`
  (vscode) and `iscsiDiskMapper` (k8s) are plausible true positives; the vscode hit in
  `fixtures/.../f2.ts` is a test fixture (false positive, not yet excluded). ≈3 TP / 1 FP.
- **extract-superclass** (13 hits read): true-ish: `FakeWatcher`/`RaceFreeFakeWatcher`,
  `DeltaFIFO`/`FIFO`/`RealFIFO`, `RemoteImage/RuntimeServiceBuilder`, the dynamiccertificates
  pair, both stapler-squad pairs. False: `GroupVersion*` value types, `AttachedVolume`/
  `VolumeToAttach` data structs. ≈ 8 TP / 5 FP.
- **collapse-hierarchy**: **mostly false positives.** All 7 kubernetes hits read were
  intentional (strategy embedding, versioned-compat wrappers, `MicroTime`/`Time`). Of 10
  sampled Cassandra hits, nearly all were subclasses implementing an abstract parent's methods
  (`BooleanType`, `FloatSerializer`: the template-method pattern). The model cannot see
  `abstract`, so "override everything" is indistinguishable from "adds nothing".
  Mitigation: JVM (`.java`/`.kt`) subtypes are now skipped, which takes Cassandra from 80
  hits to 0 and leaves Go/TS, where kubernetes still gives 6 (judged mostly intentional, so
  treat the Go rate as still high, ~85%+, n=7). The real fix is an `is_abstract` bit on
  `SymbolNode` (not done). Disable per repo with `disabled: ["collapse-hierarchy"]`.

### Re-run after code-review changes (release build)

| Repo | remove-subclass | extract-superclass | collapse-hierarchy |
|---|---|---|---|
| kubernetes/kubernetes | 1 | 11 | 6 |
| tstapler/stapler-squad | 0 | 2 | 0 |
| microsoft/vscode | 2 (fixture hit gone) | 0 | 0 |
| apache/cassandra | 0 | 0 | 0 (was 80) |
