# `go-encapsulate-collection` check — known false positives

Tracks false-positive firings of `go-encapsulate-collection`
(`src/checkers/go_encapsulate_collection.rs`, `check_method`). Check new occurrences against
this list before re-investigating from scratch.

## Mechanism (confirmed by reading the source)

`check_method` checks the body shape (`return recv.field`) and that the field's type is a slice
or map, and skips exported fields and `_test.go` files. It still looks at nothing else: not
whether the method satisfies an interface, whether the field is ever mutated after construction,
or whether a doc comment says the sharing is intentional.

## Log

### 2026-10-12 — stapler-squad/kubernetes corpus backtest — getter fixed by an external interface

- **Repo**: stapler-squad `session/git/diff.go` (`func (p *diffFilePatch) Chunks() []fdiff.Chunk { return p.chunks }`,
  unexported type implementing go-git's `fdiff.Patch`); kubernetes `SupportedMediaTypes()`
  implementing `runtime.NegotiatedSerializer`.
- **Why it's a false positive**: the interface fixes the return type and the receiver type never
  leaves the package.
- **Mechanism**: no receiver-visibility or interface check in `check_method`. Counts (30/218
  stapler-squad real+worktree, about 10 kubernetes) are agent estimates.

## Fixed

### 2026-10-12 — kubernetes/stapler-squad corpus backtest — exported fields and test fakes

- **Symptom**: getters returning an already-exported field (`NodeInfo.GetPods` returning `n.Pods`,
  `MultiError.Unwrap` returning `m.Errors`) and test fakes (`fakeMux.ListedPaths`,
  `fakePeerLister.LiveSnapshot`) fired.
- **Mechanism**: `check_method` never checked the field name's case, and `check()` skipped only
  generated files.
- **Fixed by**: skipping fields whose name starts uppercase and `_test.go` files. Re-run with the
  shipped binary: kubernetes 183 → 55 (the harness fix also dropped the 82 `vendor/` findings;
  none of the cited examples fire), stapler-squad 16 real-tree → 3.

## Not logged here

Harness/corpus artifacts, not checker bugs (since fixed in `d3d6ffe`): `kubernetes/vendor/`
(82 of 183 findings) and gitignored `stapler-squad/.claude/worktrees/` copies (202 of 218).

`Server.GetOrigins` (`stapler-squad` `server/server.go`, HEAD `d8c6b3d`) is the nearest real hit;
it is a needs_discussion, not a false positive.
