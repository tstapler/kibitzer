# `pull-up-method` check — known false positives and tuning

`pull-up-method` (`src/pull_up_method.rs`, engine in `src/pull_up_common.rs`) is Fowler's *Pull Up
Method*: ≥2 direct sibling subtypes of one supertype declare a same-named, same-signature method with
an identical body, and the supertype does not declare it. Advisory, batch-only, on by default.

Bodies are sliced from source using `SymbolNode::line..=end_line` and compared after dropping
whitespace and comment-only lines. A method with no `end_line`, an unreadable file, or fewer than 2 body
lines is skipped. Works for every language that has methods in the model (Go, TS/JS, Java, Kotlin).

## What it requires

| Gate | Why |
|---|---|
| Body opener is the first line ending in `{` | A `{` inside a Go return type (`interface{}`) is not the body. |
| Signature text equal (Java/Kotlin: parameter-list text; Go/TS: `params`) | `params` is empty outside Go/TS/JS; without this, same-text methods with different parameter types matched. |
| Go: every use of the receiver is `recv.member` with `member` declared on the supertype | A Go base cannot see its embedder. Same text over `s.SummaryOpts` vs `c.CounterOpts` binds to different types (4/4 Go samples in the first run). |
| Skip bodies using an `UPPER_SNAKE` token | The constant may differ per class. |
| Skip members nested inside another function | They belong to an anonymous/local class but are attributed to the enclosing class. |
| Same-named siblings are skipped | They are copies of one class; supertype resolution by name is unreliable there (vscode `ClassBlueprintSearch`). |
| More than half of the supertype's direct subtypes share the method | Two of hundreds under a base like `Disposable` is not a missing common method. |
| Go subtypes must be in the supertype's package; tests/fixtures/unresolved edges ignored | Embedding a foreign type is composition. |

## Known false-positive patterns

| Pattern | Status |
|---|---|
| Identical text that resolves to different static members/types per sibling (other than the `UPPER_SNAKE` and Go receiver cases) | Not filtered; the model has no symbol resolution inside bodies. |
| Intentional per-type duplication (generated code, protocol/IDL boilerplate) | Not filtered; use `kibitzer:ignore`. |
| Private methods that would need to become `protected` | Reported; the refactor is still valid. |

## Backtest (release build, final; `kibitzer check architecture pull-up-method <repo>`)

Every hit was read against the source by one reviewer.

**Standard corpus**: cassandra 16, kubernetes 2, vscode 2; nowinandroid, servo, ripgrep, deno, stapler-squad 0. All 20 are
plausible pull-up candidates (duplicated shard loops in the two memtables, `ListType`/`SetType`, `Direct`/`Standard` readers,
TS `capabilities` getters, Go `SignPod`): **0/20**.

**Held-out Go repos** (not used to design any guard): argo-cd, caddy, trivy, containerd, etcd, docker/compose, syft, pack, cli, coredns (ten Go repos named before any of this check's output on them was seen). 1 hit (containerd `Commit` on the two Windows
snapshotters, identical bodies over the embedded `windowsBaseSnapshotter`), plausible: **0/1**. No held-out Java/TS repo
was available.

Combined **0/21**. Iterations on the way: the first build gave 92 hits with 5 of 14 sampled wrong (36%); the Go receiver
guard, then the guards in the table above, got to the current figures. Transcript replay: 0 over 1818 snapshots.

## Suppression

Add `kibitzer:ignore pull-up-method -- <why>` above the flagged line, or disable the check per repo
(`disabled: ["pull-up-method"]`); see `docs/suppressing-checks.md`.
