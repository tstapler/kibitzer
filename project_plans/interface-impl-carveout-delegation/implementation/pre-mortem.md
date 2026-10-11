# Pre-mortem: interface-impl-carveout-delegation

**Date**: 2026-10-10
**Phase**: retroactive sdd:4-validate pre-mortem (written after implementation, HEAD `dc7f40a`)
**Change under review**: `git diff 67885ac~1..HEAD -- src` — all in `src/single_call_site_delegation.rs`
(`implements_index` L67-84, `implements_shared_interface` L89-103, filter at L134, doc L10-16, 3 new tests L290-337).

Premise: the change shipped, and six months later someone is asking why it went wrong.

## Evidence base (all re-run, not recalled)

| Fact | Source / command | Status |
|---|---|---|
| vscode corpus: 4059 findings before, 3222 after, 0 added, 837 removed (all `method`) | pre-change binary built from `67885ac` vs `target/release/kibitzer`, `kibitzer check architecture single-call-site-delegation ~/code/github.com/microsoft/vscode`; `comm` of sorted outputs | VERIFIED (reproduces `decisions/validation.md`) |
| Of the 837 removed: 313 are `private`/`protected`/`#`-marked; a further 84 are public but their name appears in no `interface` body anywhere in vscode (regex heuristic, upper-bound for interface membership); 440 have a name that matches some interface member | Python scan of the 837 `file:line` declaration lines against interface-member names collected from every `*.ts`/`*.tsx` under vscode | VERIFIED for the 313; 84 is heuristic (INFERRED) |
| kubernetes corpus (Go): 502 findings before, 501 after; the one removed is `fakeWatchApfFilter.wait` at `staging/src/k8s.io/apiserver/pkg/server/filters/priority-and-fairness_test.go:426` | same two binaries against `~/code/github.com/kubernetes/kubernetes`; struct embeds `utilflowcontrol.WatchTracker` / `MaxSeatsTracker` at L373-374, and `fakeApfFilter` (L84-85) embeds the same two | VERIFIED |
| vscode model after change: 5889 `implements` edges, 4818 resolved, 1071 unresolved; 2082 distinct resolved interfaces, 1347 with one implementer, 735 with more | `kibitzer architecture export --include-private`, JSON analysed with Python | VERIFIED |
| Of the 2610 `method` findings still emitted on vscode: 708 on a type whose resolved interfaces are all sole-implementer, 416 on a type whose only `implements` edges are unresolved, 1058 on a type with no `implements` edge, 428 with no parent | same export joined to `new.txt` by file:line | VERIFIED (counts), classification INFERRED from id-prefix join |
| New unit tests pass: 13/13 in `single_call_site_delegation` | `cargo test --bin kibitzer single_call_site` | VERIFIED |
| TS `exported` is `parent.kind() == "export_statement"`, so every class *method* is `exported == false` regardless of `private`/`public` | `src/symbol_extract.rs:85-89`, `:506` | VERIFIED (code read) |

## Findings

### P1 — would cause wrong behavior / false negatives at scale

#### P1-1. Carve-out is per *type*, not per *method*: it silences methods that cannot be on any interface

- **Failure story**: the rationale ("a second caller may appear through the interface") only holds for a
  method that is *on* the interface. The implementation suppresses every pure-delegation method of any
  class that shares an interface, including `private`/`protected`/`#` helpers (a TS interface member can
  never be private, and a caller going through the interface can never reach one). These are the
  highest-confidence Inline Function findings and they vanish. A reviewer six months on sees
  `single-call-site-delegation` go quiet on exactly the large service classes where wrapper-creep
  accumulates, and nobody notices because the counter just went down (4059 -> 3222 looks like noise reduction).
- **Magnitude**: 313 of the 837 removals (37%) are explicitly private/protected/`#`; 397 of 837 (47%) are
  provably or near-provably not interface members. Private-method findings on vscode drop 1015 -> 707 (-30%,
  same line-text heuristic on `base.txt` vs `new.txt`).
- **Minimal repro** (VERIFIED, old vs new release binaries): `class Sq implements Shape { area(){return this.thin()} private thin(){return this.compute()} compute(){...} }`
  plus a second `class Circ implements Shape`: old binary reports `thin` (1 finding), new binary reports 0.
- **Example**: `extensions/copilot/.../chatSessionMetadataStoreImpl.ts:472` `private async readSessionMetadataFile`
  (class at L39 `extends Disposable implements IChatSessionMetadataStore`, interface has other implementers).
- **Does the implementation prevent it?** NO. `implements_shared_interface` (`src/single_call_site_delegation.rs:89-103`)
  looks only at the owning type id; nothing consults the method's own visibility or the interface's member
  set. The new test at `:292-304` actually encodes the behaviour: its `thin` is declared `private thin()` and
  is asserted suppressed. `symbol_extract.rs` does not record TS accessibility modifiers at all (`:85-89`),
  so the checker cannot distinguish them today.
- **Spec note**: `requirements.md` Success Metrics literally say "A private TypeScript/TSX/JS method whose owning
  type has at least one `Implements` edge is **not** flagged". The spec encodes the flaw, so a spec-conformance
  review will pass it. The spec also forbids touching `symbol_extract.rs`, which is where the fix has to start.
- **Prevention**: (a) record a `visibility`/`is_private` bit (TS `accessibility_modifier` private|protected, `#name`)
  on `SymbolNode` and skip the carve-out for those; (b) better, intersect the method name with the resolved
  interface's declared member names (`isp_fat_interface.rs::interface_declared_methods` already extracts them) so
  only methods actually on the interface are exempted; (c) add a test asserting a `private` helper on a
  shared-interface implementer IS flagged. Relax the requirement text accordingly.
- **Status: OPEN.**

### P2 — noticeable wrong behavior, bounded blast radius, or a stated requirement not met

#### P2-1. Go leaks into the carve-out via embedded interfaces; "Go unchanged" requirement is not met

- **Failure story**: Go struct embedding resolves to `kind_hint: None` and then takes its kind from the resolved
  target's `SymbolKind`; an embedded in-repo interface becomes an `Implements` edge
  (`symbol_extract.rs:1606`, `arch_model.rs:789-791`, `:761-762`). Once two structs embed the same interface,
  every unexported delegating method of both is suppressed. Go is the primary `is_pure_delegation` language,
  so this applies to the main audience, and `requirements.md` says Go behaviour must not change.
- **Magnitude**: small on k8s (1 of 502, `priority-and-fairness_test.go:426`, an unexported test helper that is
  on no interface), but it is the same wrong-direction logic as P1-1 and grows with embedding-heavy Go repos.
- **Prevented?** NO. The Go test `go_method_without_implements_edges_is_still_flagged` (`:321-336`) uses a
  struct with *no* embed, so it passes while the embed case regresses. The module doc (`:14-16`) says "Only
  explicit `implements` syntax is covered", which is false for Go embeds. Plan Story 7's k8s no-regression run
  was not recorded in `decisions/validation.md` (only vscode); I ran it and it shows 502 -> 501.
- **Prevention**: gate the index on the owning symbol's language (TS/TSX only until Go structural satisfaction
  lands), or keep `kind_hint` on the edge so the checker can require "keyword-derived Implements". Add a Go
  test with `type T struct{ I }` plus a second embedder.
- **Status: OPEN.**

#### P2-2. Unresolved `implements` targets are treated as "no interface" (inverse of the requirement's own rule)

- **Failure story**: `implements_index` keeps only `e.resolved` (`:68-73`). A class implementing an external or
  ambiguous-name interface (`IDisposable`, `vscode.Disposable`, `vscode.TreeDataProvider`, `Partial`) gets no
  carve-out although such an interface certainly has other implementers.
- **Magnitude**: 416 of the 2610 remaining method findings on vscode sit on types whose only `implements` edges
  are unresolved (top unresolved targets: `IDisposable` 179, `Command` 31, `vscode.Disposable` 26).
- **Prevented?** NO (by design: `requirements.md` "Rabbit Holes" chose resolved-only to "avoid false
  carve-outs"). Note the requirements' own "Unknown != no interface" non-functional rule argues the other way.
- **Prevention**: treat an unresolved `Implements` edge whose target is not a repo symbol as "shared"
  (external interface => other implementers exist); keep ambiguous-name-in-repo as the conservative case.
  Backtest-diff the vscode set before enabling.
- **Status: OPEN** (accepted trade-off in the plan, but the cost is now measured).

#### P2-3. "Sole implementer => interface is effectively private" is unsound in TS

- **Failure story**: TS typing is structural and interfaces are frequently exported. A type can be the only
  *declared* implementer while object literals, `satisfies`, mocks using `as`, or out-of-repo consumers also
  implement it. Only explicit `implements` clauses are counted (`:68-73`), so the exemption fires and the
  wrapper is reported as inlinable.
- **Magnitude**: 708 of the remaining 2610 method findings sit on sole-implementer types; e.g.
  `GitHubOrgChatResourcesService.sanitizeFilename` on exported `IGitHubOrgChatResourcesService`.
  Findings here are no worse than the pre-change baseline, so this is a missed *improvement*, not a regression.
- **Prevented?** PARTIALLY. The exemption is explicit, tested (`:307-319`) and documented (`:10-13`); it does
  not consider the interface's `exported` flag.
- **Prevention**: do not exempt when the interface symbol is `exported` (the `SymbolNode` is already in the
  model; needs an id -> exported lookup). Optionally make "private interface only" the exemption.
- **Status: OPEN, accepted by plan Finding 3.**

### P3 — low probability, low impact, or forward-looking

#### P3-1. TS `abstract class` is invisible to both symbol and type-edge extraction

- `ts_class_heritage_relations` fires only for `class_declaration` (`symbol_extract.rs:1236-1239`); methods of an
  `abstract class` get `parent: None` and ids `{pkg}::{name}` that can collide with free functions. Repro in
  scratchpad: the same `private thin()` is flagged inside `abstract class Base implements Shape` yet suppressed in
  a concrete class (old/new binary behave identically for the abstract case).
- Size: ~189 of 2613 remaining vscode method findings are in abstract classes (regex estimate, INFERRED).
- Prevented? NO (pre-existing extractor gap, residual false positive, not a regression).
- Prevention: handle `abstract_class_declaration` in both walkers.

#### P3-2. `extends`-only polymorphism is ignored

- Only `Implements` edges count. A subclass overriding a base-class method or a mixin pattern gives no carve-out;
  `Extends` edges are filtered out (`:70`). Residual false positive, same as baseline.
- Prevented? NO. Prevention: out of scope unless P1-1(b) lands (then include base-class member sets).

#### P3-3. Id collisions skew counts

- `type_edges.from` is `{pkg}::{Class}` where `pkg` is the directory. Two same-named classes in one directory
  merge: `HashSet<(from,to)>` dedupes them into one implementer (undercount => flagged, baseline FP) and a
  non-implementing twin inherits the other's suppression (overcount => silent FN).
- Prevented? NO (known v1 id-collision limit, `symbol_extract.rs:316-318`). Rare. Prevention: id includes
  file stem for TS, as the module doc already contemplates for Java overloads.

#### P3-4. Cross-file action-at-a-distance in diff-aware mode

- Whether a method is flagged now depends on a second implementer in another file. Adding/removing that
  implementer flips findings in files the author did not touch (same property `caller_counts` already has).
  INFERRED from `check.rs:1193-1250` (whole-repo model built from all files; snapshot comparison at `:1355`);
  I did not exercise the diff-aware path.
- Prevented? NO; consistent with the existing checker's model. Prevention: none needed beyond documentation.

#### P3-5. Test coverage thinner than the logic

- Missing: a type implementing one sole + one shared interface (the `any` branch), duplicate-edge dedupe,
  an unresolved edge, TSX/JS input, Go embed (see P2-1), `private` helper (see P1-1). The three tests added
  (`:292-336`) only pin the happy/sole/Go-no-embed paths.
- Process gap: AC6 (`kibitzer check backtest ...`) is not runnable for whole-repo `ArchModelChecker`s
  (`decisions/validation.md` L107-109), so there is no transcript backtest; the "12/12 sampled removals are in
  classes with an explicit `implements` clause" check validates the *type* predicate, which is the wrong unit
  (see P1-1).
- Prevention: add the tests above; replace the sampling criterion with "removed method is a member of the
  interface".

#### P3-6. Forward risk when Java/Kotlin gain call-graph support

- `implements_index` is language-agnostic. Java/Kotlin `Implements` edges exist now but `is_pure_delegation` is
  `false` for them (`symbol_extract.rs:524-528`, `call_graph_supports` at `:678-683`), so it is a no-op today.
  If those languages are added, P1-1's type-level semantics apply there, and Kotlin bare-`type` edges infer
  `Implements` from the resolved target kind.
- Prevented? Not applicable now. Prevention: land P1-1 before extending `call_graph_supports`.

#### P3-7. Coupling to the id string format

- `implements_shared_interface` derives the owner id with `strip_suffix(name)` + `strip_suffix('.')`
  (`:91-97`). If `build_id` changes (e.g. overload disambiguation), the lookup returns `None` and the carve-out
  silently turns off (fails open to baseline).
- Prevented? YES: the suppress test (`:292-304`) would fail on any id-shape change; the comment at `:90`
  names the contract.

#### P3-8. Performance

- One pass over `type_edges` (13746 on vscode) building two maps, then O(interfaces-per-type) per candidate
  (`:67-103`, `:128`). Negligible against model build (~60 s whole-run on vscode, dominated by parsing).
- Prevented? YES (no quadratic path; iteration-order nondeterminism of the `HashSet` is harmless because only
  `any(..)` is evaluated and findings are sorted at `:138`).

## Summary table

| ID | Sev | Failure mode | Already prevented? |
|---|---|---|---|
| P1-1 | P1 | Type-level suppression hides private/non-interface wrappers (-313 private findings on vscode) | NO |
| P2-1 | P2 | Go struct embeds produce `Implements`; Go no longer "unchanged" (k8s 502 -> 501) | NO |
| P2-2 | P2 | Unresolved/external interfaces get no carve-out (416 remaining findings) | NO (spec choice) |
| P2-3 | P2 | Sole-implementer exemption ignores exported interfaces / structural typing (708) | Partial |
| P3-1 | P3 | `abstract class` invisible to extraction | NO |
| P3-2 | P3 | `extends`-only polymorphism ignored | NO |
| P3-3 | P3 | Same-name class id collisions | NO |
| P3-4 | P3 | Cross-file flips in diff-aware mode | NO (inherent) |
| P3-5 | P3 | Test and backtest gaps | NO |
| P3-6 | P3 | Future Java/Kotlin enablement | n/a today |
| P3-7 | P3 | Id-format coupling | YES |
| P3-8 | P3 | Performance | YES |

Counts: **P1 = 1 (open: P1-1)**, **P2 = 3**, **P3 = 8**.

## Recommendation

Do not treat the 4059 -> 3222 drop as the success signal. Before the next release, fix P1-1 (method-level
carve-out, or at minimum skip `private`/`protected`/`#` methods) and P2-1 (language gate), and re-run both corpus
diffs; the vscode "removed" set should shrink toward the 440 methods whose names match an interface member.
