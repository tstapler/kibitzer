# ADR-004: Literature Review and a Module-Boundary Candidate Signal

**Date**: 2026-09-28
**Status**: Proposed — analytically promising, not yet empirically validated, no owner
**Related**: ADR-001 (name allowlist, killed), ADR-002 (root-cause kill), ADR-003 (return-type +
different-package, killed by its own gate before any code was written)

## What the literature says

Searched for how the industry and academic literature handle exactly this problem —
[PMD's own Law of Demeter false-positive tracker](https://sourceforge.net/p/pmd/bugs/1245/),
[DevIQ's LoD writeup](https://deviq.com/laws/law-of-demeter/), a
[formal false-positive study using JHotDraw](https://arxiv.org/pdf/2002.06191), and the Feature
Envy detection literature
([JDeodorant](https://www.researchgate.net/publication/4283960_JDeodorant_Identification_and_Removal_of_Feature_Envy_Bad_Smells),
JMove, [DesigniteJava 2.0](https://tusharma.in/preprints/MSR2024_DesigniteJava2.0.pdf)).

Three findings that reframe this problem:

1. **This is not a kibitzer-specific gap — it's PMD's, too.** PMD ships a Law of Demeter rule and
   its own issue tracker records the identical complaint three failed attempts here have
   independently rediscovered: "this is also a problem using the Builder pattern... it's sad that
   such a good rule as LOD triggers so many false positives." The workarounds PMD users propose
   (name-check for `build()`, exempt same-return-type chains) are the same shapes as ADR-001 and
   PR #106.
2. **The formal definition is provably too strict for real code.** The rigorous Java adaptation of
   Demeter's original law (used in the JHotDraw false-positive study) permits calling a method only
   on `this`, a field of the enclosing class, a parameter of the enclosing method, or an object
   locally instantiated in the method. By that definition, *any* chained call on a method's return
   value is a violation — which is why every practical tool relaxes it. The standard relaxation
   (DevIQ, Baeldung): a fluent chain is exempt when every hop returns the same type as the previous
   receiver ("locally instantiated, same-type chaining"), and a real violation is one where the
   chain crosses into an *unrelated* object's type.
3. **Nobody has published a fix for the generated-SDK-accessor case.** Searched specifically for
   how Designite or similar tools handle generated-code/SDK accessor trees (the client-go/informer
   shape that's 36%+ of our corpus) and found nothing — not a documented exemption rule, not an
   academic treatment. This specific failure mode appears to be either unaddressed in the
   literature or addressed only via project-specific manual allowlists, which is the same
   unbounded-vocabulary conclusion ADR-002 already reached independently.

**Consequence for design**: the "same return type as previous hop" relaxation (call it Design B)
and the strict formal locality rule (Design A) are both analytically dead on arrival for this
corpus — I didn't need to prototype either to know this, because their failure mode is exactly the
one the literature already documents. `informerFactory.Certificates().V1().CertificateSigningRequests()`
never repeats a type hop-to-hop and never touches a field/param/local, so both designs flag it,
identically to every heuristic tried so far.

## Design C: module-boundary crossing (new, not previously tried)

ADR-003's "different package" signal failed because Go/Rust packages are too fine-grained a unit —
a single library (`client-go`) is dozens of packages, and hopping between its own sub-packages
(`kubernetes` → `typed/core/v1` → the resource interface) isn't a Demeter violation, it's just how
the library is organized. But package path also isn't the right *coarser* unit either — I initially
assumed "is the type's source even in this git repo" would work, then verified that's wrong:
`kubernetes/kubernetes`'s `staging/src/k8s.io/{client-go,apimachinery,apiserver}` all live in the
same git tree but each carries its **own `go.mod`** (`module k8s.io/client-go`, etc.) — verified by
reading those three files directly in the `/tmp/k8s-fp-check` worktree. They're physically
co-located but architecturally separate, independently-versioned modules, consumed by the rest of
the tree the same way an external dependency would be.

**The refined signal**: a chain hop is a candidate violation only if its resolved return type's
enclosing **build-unit** (nearest `go.mod`/`Cargo.toml` workspace-member/`package.json`) differs
from the *previous hop's* build-unit — not from the file's own build-unit, and not from a bare
package-path comparison. Flag only if the chain crosses into **2 or more distinct build-units**
across its hops (not just one).

### Checked against the corpus, by hand, module-by-module

Verified against `/tmp/k8s-fp-check` and `/tmp/ripgrep-fp-check`'s actual `go.mod`/`Cargo.toml`
layout, not just the note text:

| Category (n) | Root type's module | All hops' modules | Distinct foreign modules touched | Excluded? |
|---|---|---|---|---|
| `client-go-typed-clientset` (210) | root module | all stay in `k8s.io/client-go` | 1 | **Yes** |
| `informer-factory-accessor-chain` (62) | root module | all stay in `k8s.io/client-go` | 1 | **Yes** |
| `restclient-builder-dsl` (46) | root module | all stay in `k8s.io/client-go/rest` | 1 | **Yes** |
| `dynamic-client-resource-namespace-chain` (6) | root module | all stay in `k8s.io/client-go/dynamic` | 1 | **Yes** |
| `lister-get-list-chain` (9) | root module | all stay in `k8s.io/client-go/listers` | 1 | **Yes** |
| prometheus metric chain (sampled) | root module | all stay in `prometheus/client_golang` | 1 | **Yes** |
| `acc.details.CPUsInNUMANodes(numa).Size()` | root module (first-party) | `.CPUsInNUMANodes()` foreign into `k8s.io/utils`, `.Size()` stays there | 1 foreign module | **Yes** |
| `self.dent.path().strip_prefix(...)` / `self.wtr.borrow().supports_color()` | root module | both stay in Rust `std` | 1 | **Yes** |
| `kubeschedulerscheme.Codecs.UniversalDecoder().Decode(...)` | root module | `Codecs`→`k8s.io/apiserver`, `UniversalDecoder()`→`k8s.io/apimachinery` | **2 distinct foreign modules** | **No — residual false positive** |

This clears essentially every dominant category in the corpus (the top 5 alone are 333/786, 42%),
including the field-navigation, dynamic-client, and Rust-stdlib shapes I re-checked at the source
level, not just from category labels. It has one confirmed, narrower residual failure mode: chains
that hop between two *separate but co-released* modules in the same ecosystem family (here,
`k8s.io/apiserver` → `k8s.io/apimachinery` — sibling staging modules that are versioned and
released together as part of one Kubernetes release, but are still formally distinct Go modules).
The same shape would recur for e.g. `aws-sdk-go-v2`'s per-service modules, or a JS monorepo's
separately-versioned workspace packages that are nonetheless developed as one unit.

**This is a real, named gap, not a hand-wave.** I have not run this against the full 786-record
corpus or the ripgrep half in equal depth — the table above is a hand-check of the dominant
categories plus every record I'd already read source for in ADR-003's spot-check, not an
exhaustive pass. Before writing detection code, the same mandatory gate ADR-003 specified applies
here: build only the module-boundary classification, run it against the full corpus, and confirm
the exclusion rate holds up outside the categories checked here.

## What each design would actually require to build

None of the three designs need a new tree-sitter grammar or a genuinely new parser — the missing
pieces are ordinary extraction-function engineering on grammars kibitzer already depends on, plus
one new *non-source* parsing task:

- **Return-type resolution** (needed by A, B, and C): new per-language functions reading the
  existing `func f() *Foo` / `Foo f()` / `fun f(): Foo` / `fn f() -> Foo` return-type syntax already
  exposed by `tree-sitter-go`/`-rust`/`-typescript`/`-java`/`-kotlin-ng` — the same *kind* of code as
  `chain_unwrap` in the now-reverted `rules.rs`, not a grammar change. Python/JS still have no
  static ceiling here, same caveat as ADR-003.
- **Field/param/local-instantiation classification** (needed only by Design A, which literature
  already rules out): reuses node kinds (`parameter_list`, `field_declaration`, local var decls)
  that `binding_finder`-style functions in `rules.rs` already walk today. No new grammar.
- **Build-unit boundary map** (needed only by Design C, and only by C): a new, one-time,
  non-source-code parsing task — read `go.mod`'s `module` line (plain text, no parser needed),
  `Cargo.toml`'s `[package]`/`[workspace]` (the `toml` crate, already a mature off-the-shelf
  dependency, not something to hand-roll), or `package.json`'s `name`/workspaces field (`serde_json`,
  already a kibitzer dependency) — and build a `source path → build-unit id` map alongside
  `ArchModel.packages`. This is unrelated to tree-sitter entirely; it's manifest-file parsing, the
  same category of work as `import_graph.rs`'s existing alias resolution, not AST work.

**Direct answer to "would we need tree-sitter extensions or custom parsing?"**: No grammar
extensions for any candidate. "Custom parsing code" is required, but it's the ordinary kind kibitzer
already writes for every other native check (new extraction functions over existing parse trees),
plus one new manifest-file reader — not a new language front-end or a fork of an existing grammar.

## Decision

Not started. Design C is the first candidate in four attempts (ADR-001 through this one) that
survives contact with the actual corpus at the categories checked — a real, if partial, result, not
just another elegant-looking heuristic. It is **not validated** to the standard ADR-003 set: no
exhaustive pass against all 786 records, no corpus check for whether it produces new false
positives methods B/C never triggered on that the old heuristics didn't. Recommended next step, if
anyone picks this up: prototype the build-unit boundary map alone (cheapest part, no return-type
work needed yet) against both corpora, and hand-check whether "distinct foreign build-units
touched >= 2" tracks the confirmed-false-positive/hypothetical-true-positive line before investing
in the return-type resolver at all — same discipline as ADR-003's gate, applied one layer earlier
since the boundary map is cheaper to build first and might independently disqualify itself.

## Evidence

- ADR-001, ADR-002, ADR-003 (this project's own prior decisions).
- [PMD Law of Demeter false-positive tracker](https://sourceforge.net/p/pmd/bugs/1245/)
- [DevIQ: The Law of Demeter](https://deviq.com/laws/law-of-demeter/)
- [Did JHotDraw Respect the Law of Good Style? (arXiv:2002.06191)](https://arxiv.org/pdf/2002.06191)
- [JDeodorant: Identification and Removal of Feature Envy Bad Smells](https://www.researchgate.net/publication/4283960_JDeodorant_Identification_and_Removal_of_Feature_Envy_Bad_Smells)
- [Multi-faceted Code Smell Detection at Scale using DesigniteJava 2.0](https://tusharma.in/preprints/MSR2024_DesigniteJava2.0.pdf)
- `/tmp/k8s-fp-check/staging/src/k8s.io/{client-go,apimachinery,apiserver}/go.mod` — verified
  distinct `module` declarations directly.
- `docs/backtest-triage/{burntsushi-ripgrep,kubernetes-kubernetes}/hide-delegate.jsonl` — the
  786-record corpus this ADR's table checks against (partially, not exhaustively).
