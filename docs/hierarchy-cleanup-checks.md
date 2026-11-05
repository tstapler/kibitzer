# Hierarchy cleanup checks

Three advisory, batch-only, whole-repo checks over `ArchModel.type_edges` (resolved `Extends`
edges only; unresolved targets are ignored). Suppress with `kibitzer:ignore <rule> -- <why>`;
see `suppressing-checks.md`.

| Check | Fowler | Fires when | Languages |
|---|---|---|---|
| `remove-subclass` | Remove Subclass | Leaf subtype; ≥1 method, every one `SymbolNode::is_constant_return`; no field of the subtype is read by any method | Go, TS/JS (Java/Kotlin never fire: `is_constant_return` is always false) |
| `extract-superclass` | Extract Superclass | Two same-package types, no edge between them, ≥ `MIN_METHODS_FOR_CLUSTERING` (4) methods with field accesses each, Jaccard of accessed field *names* ≥ 0.5, ≥ 2 shared names; top 3 per package | Go only (field-access data) |
| `collapse-hierarchy` | Collapse Hierarchy | Resolved `Extends` edge; subtype has ≥ 3 methods and < 20% of them are absent by name from its direct supertype | Go, TS, Java, Kotlin |

Guards added after backtesting (not in the original issue): `extract-superclass` skips test
files, same-named types (build-tag variants) and single-shared-field pairs; `collapse-hierarchy`
skips same-named pairs and subtypes with < 3 methods.

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
- **collapse-hierarchy**: **mostly false positives.** All 7 kubernetes hits read are
  intentional (strategy embedding, versioned-compat wrappers, `MicroTime`/`Time`). Of 10
  sampled Cassandra hits, nearly all are subclasses implementing an abstract parent's methods
  (`BooleanType`, `FloatSerializer`: the template-method pattern). The model cannot see
  `abstract`, so "override everything" is indistinguishable from "adds nothing". Treat as
  ≈ 85-100% FP on Java; keep advisory, and disable per repo if noisy
  (`disabled: ["collapse-hierarchy"]`). A real fix needs an `is_abstract` bit on `SymbolNode`.
