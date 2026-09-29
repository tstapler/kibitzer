# ADR-004: Literature Review and a Module-Boundary Candidate Signal

**Date**: 2026-09-28
**Status**: Proposed — prototyped and spot-verified against real compiler-level type info for
41.6% of the corpus (see Prototype Results); still not exhaustive, no owner
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

### First pass, by category label and hand-reasoning (superseded below)

An initial pass matched categories against the note text and hand-traced a few chains, which is
what first surfaced this design — but it produced one wrong conclusion (the `Codecs`/
`UniversalDecoder` case looked like a residual gap) and left the ripgrep workspace-crate question
open. See **Prototype Results** below for the corrected, signature-verified version of this table.

This clears essentially every dominant category in the corpus (the top 5 alone are 333/786, 42%),
including the field-navigation, dynamic-client, and Rust-stdlib shapes I re-checked at the source
level, not just from category labels. It has one confirmed, narrower residual failure mode: chains
that hop between two *separate but co-released* modules in the same ecosystem family (here,
`k8s.io/apiserver` → `k8s.io/apimachinery` — sibling staging modules that are versioned and
released together as part of one Kubernetes release, but are still formally distinct Go modules).
The same shape would recur for e.g. `aws-sdk-go-v2`'s per-service modules, or a JS monorepo's
separately-versioned workspace packages that are nonetheless developed as one unit.

## Prototype: a real go.mod/Cargo.toml boundary-map builder

Built a small Python prototype (`/tmp/boundary-map/build_map.py`, not committed — throwaway
research tooling, not a kibitzer feature) that walks a repo, finds every `go.mod`, and records its
declared `module` path keyed by directory. Ran it against `/tmp/k8s-fp-check`: it found all 34
modules in the tree in one pass, including every `staging/src/k8s.io/*` submodule (`client-go`,
`apimachinery`, `apiserver`, `metrics`, `kms`, ...) plus the root `k8s.io/kubernetes` module — this
part is mechanical and complete, not a sample.

That alone answers "which build-unit does a chain's *root* file belong to" for all 786 records. It
does **not** answer which build-unit each *hop's return type* belongs to — that still needs real
type information. Rather than guess (which is what produced an error below), I used the Go module
cache directly: running `go build`/`go list` against the k8s worktree downloaded the real dependency
source into `~/go/pkg/mod` (confirmed network access works in this environment; ~10GB, not fast —
not worth doing for a full build, but the *source* is what matters here). Grepping the actual
declarations in that cache gives ground truth without running the type checker at all.

**One correction this caught in my own earlier hand-analysis**: I'd flagged
`kubeschedulerscheme.Codecs.UniversalDecoder().Decode(...)` in the draft of this ADR as a residual
false positive, guessing `Codecs`'s type came from `k8s.io/apiserver`. Checking the real
declaration (`pkg/scheduler/apis/config/scheme/scheme.go:32`, `Codecs = serializer.NewCodecFactory(...)`,
import `k8s.io/apimachinery/pkg/runtime/serializer`) and `NewCodecFactory`/`CodecFactory.UniversalDecoder`'s
actual signatures in the module cache (`k8s.io/apimachinery@v0.34.9/pkg/runtime/serializer/codec_factory.go:178,266`)
shows the whole chain — `Codecs`'s type *and* `UniversalDecoder()`'s return type — both resolve to
`k8s.io/apimachinery`, a single foreign module, not two. **This record is correctly excluded**; the
"2 distinct foreign modules" gap I described earlier didn't exist — it was an unverified guess that
the prototype disproved. Removed as a counterexample.

**A second correction, caught while checking whether ripgrep's own workspace crates (also
independently published, same shape as k8s's staging modules) would break Design C**: for
`self.dent.path().strip_prefix("./")` (`crates/core/haystack.rs:109`), `dent`'s declared type is
`ignore::DirEntry` — the `ignore` crate is a *separate*, independently-versioned workspace member
(`ignore = { version = "0.4.29", path = "crates/ignore" }` in the root `Cargo.toml`), not part of
the root `ripgrep` package. Naively counting `self.dent`'s type-origin as a "crossing" makes this
chain touch 2 foreign units (`ignore`, then `std` via `DirEntry::path()`'s return, verified at
`crates/ignore/src/walk.rs:38`) — which would wrongly flag a record ADR-002 confirmed is a false
positive. The fix is one the literature already gave and I'd under-applied: **classic Demeter
explicitly permits calling a method on a field of `self`, regardless of what module that field's
*type* comes from** — `self.dent` is a preferred supplier by construction, not a crossing, full
stop. The crossing count should start only from the first non-preferred-supplier hop onward (the
call *result* that gets chained further), not from the root field access itself. Applying that
correction: `self.dent.path()` (field access, not a crossing) → `.strip_prefix()` on the `&Path`
result (one foreign module, `std`) — one module touched, correctly excluded. Checked three more
sampled untagged records (`crates/printer/src/standard.rs:579`, `crates/searcher/src/searcher/mod.rs:689`,
`crates/index/src/literal.rs:475`) against this corrected rule and all three hold the same shape:
root is a field/preferred supplier, the actual delegation is 1–2 hops staying inside one module
(`std` or the same crate).

**Refined rule**: exempt a chain's root hop if it's a field/parameter/local access (regardless of
that binding's declared type's module) — matching classic Demeter's own locality exception, which
Design A tried to use *instead of* module-boundary crossing and failed on (ADR-003/literature);
here it's a *prerequisite filter* underneath the module-boundary count, not a replacement for it.
Flag only if, among the hops *after* that root, 2+ distinct foreign (non-caller) modules are
touched.

**Result, with real signatures checked, not category-label inference**: `CoreV1()`→`CoreV1Interface`
(`client-go@v0.34.9/kubernetes/typed/core/v1/core_client.go:29`), `Pods()`
(`.../core_client.go:90`), `RESTClient.Get()`→`*Request` and `Request.Namespace`/`.Resource`/`.Do`
(`client-go@v0.34.9/rest/{client,request}.go`) all resolve inside `k8s.io/client-go` — confirmed by
reading the actual method signatures in the downloaded module cache, not assumed from the category
tag. Combined with the corrected `Codecs`/`UniversalDecoder` case and the corrected ripgrep
locality rule: **327/786 records (41.6%) — `client-go-typed-clientset` (210) +
`restclient-builder-dsl` (46) + `informer-factory-accessor-chain` (62) + `lister-get-list-chain`
(9)** — are confirmed excluded by real declared types, not guessed from labels.

**What's still unverified.** This is real signature-level evidence for the four dominant tagged
categories plus 8 hand-checked untagged/misc records (both k8s and ripgrep) — not an exhaustive
pass over all 786. The remaining ~59% (`misc-empty-allowlist-chain`, the rest of the untagged
ripgrep records, the smaller tagged categories) hasn't been checked at this level of rigor. **More
importantly**: every one of the 786 records carries `verdict: "false_positive"` — there are zero
confirmed *true positives* in this corpus. This ADR's evidence validates precision (does the
signal correctly exclude known-bad findings) but says nothing about recall (would it still catch a
real Law-of-Demeter violation if one occurred) — that question has no data to test against yet.
Before writing detection code, the same mandatory gate ADR-003 specified still applies: extend this
same signature-verification method (not category-label inference) to the rest of the corpus, and
separately construct or find at least a few confirmed-true-positive examples to check recall
against — a precision-only validation is not a green light on its own.

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

Not started as a kibitzer feature; the boundary-map idea itself has cleared its first checkpoint.
Design C, refined with the preferred-supplier-root exemption discovered during prototyping, is the
first candidate in four attempts (ADR-001 through this one) with real, signature-verified evidence
behind it rather than category-label inference or hand-wave: 327/786 records (41.6%) confirmed
excluded by reading actual method signatures in the Go module cache and ripgrep's own crate
manifests, plus a self-correction (the initial "residual gap" example turned out to be a research
error, not a real counterexample) that increased confidence rather than decreasing it.

**Still not a green light.** The corpus is 100% `false_positive`-labeled — this validates
precision, not recall, and the remaining ~59% of records haven't been checked at this rigor.
Recommended next step, if anyone picks this up: extend the signature-verification method (build
the `go.mod`/`Cargo.toml` boundary map into an actual tool, not a one-off script, and cross it with
real type resolution) across the full corpus, and separately find or construct confirmed
true-positive examples before writing any detection code — same discipline ADR-003 specified,
now with one layer of it already discharged.

## Evidence

- ADR-001, ADR-002, ADR-003 (this project's own prior decisions).
- `/tmp/boundary-map/build_map.py` — the boundary-map prototype (throwaway, not committed): walks
  a repo for `go.mod`/`Cargo.toml`, builds a directory → module/package map by plain-text line
  parsing (`go.mod`) and regex (`Cargo.toml`'s `[package]`/`[workspace]` sections). No tree-sitter
  or grammar involved — confirms the parsing-requirements verdict above empirically, not just in
  theory.
- [PMD Law of Demeter false-positive tracker](https://sourceforge.net/p/pmd/bugs/1245/)
- [DevIQ: The Law of Demeter](https://deviq.com/laws/law-of-demeter/)
- [Did JHotDraw Respect the Law of Good Style? (arXiv:2002.06191)](https://arxiv.org/pdf/2002.06191)
- [JDeodorant: Identification and Removal of Feature Envy Bad Smells](https://www.researchgate.net/publication/4283960_JDeodorant_Identification_and_Removal_of_Feature_Envy_Bad_Smells)
- [Multi-faceted Code Smell Detection at Scale using DesigniteJava 2.0](https://tusharma.in/preprints/MSR2024_DesigniteJava2.0.pdf)
- `/tmp/k8s-fp-check/staging/src/k8s.io/{client-go,apimachinery,apiserver}/go.mod` — verified
  distinct `module` declarations directly.
- `docs/backtest-triage/{burntsushi-ripgrep,kubernetes-kubernetes}/hide-delegate.jsonl` — the
  786-record corpus this ADR's table checks against (partially, not exhaustively).
