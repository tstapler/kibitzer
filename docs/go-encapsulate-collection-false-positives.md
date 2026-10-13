# `go-encapsulate-collection` check — known false positives

Tracks false-positive firings of `go-encapsulate-collection`
(`src/checkers/go_encapsulate_collection.rs`, `check_method`). Check new occurrences against
this list before re-investigating from scratch.

## Mechanism (confirmed by reading the source)

`check_method` only checks the body shape (`return recv.field`) and that the field's type is a
slice or map. It looks at nothing else: not whether the field is exported, whether the receiver
is a test fake, whether the method satisfies an interface, or whether a doc comment says the
sharing is intentional.

## Log

### 2026-10-12 — kubernetes/stapler-squad corpus backtest — getter returns an already-exported field

- **Repo**: `kubernetes/kubernetes` `staging/src/k8s.io/apiserver/pkg/authentication/user/user.go:61`
  and `pkg/scheduler/framework/types.go:220`
  (`func (n *NodeInfo) GetPods() []fwk.PodInfo { return n.Pods }`); stapler-squad
  `pkg/warren/errors.go:25` (`Unwrap() []error { return m.Errors }`).
- **Why it's a false positive**: callers can already reach and mutate the exported field, so a
  copying getter protects nothing.
- **Mechanism**: `check_method` calls `struct_field_type` and never checks the field name's
  case. kubernetes: 28 of 183 return an exported field (agent script, not re-derived).

### 2026-10-12 — kubernetes/stapler-squad corpus backtest — test fakes and stubs

- **Repo**: `kubernetes/kubernetes` `cmd/kube-proxy/app/server_test.go:96` (`fakeMux.ListedPaths`);
  stapler-squad `server/services/claim_checker_test.go:22`
  (`func (f fakePeerLister) LiveSnapshot() []session.RegistryEntry { return f.entries }`).
- **Why it's a false positive**: the fake exists to hand back the slice the test seeded.
- **Mechanism**: `check()` skips only generated files; `file_globs` is `**/*.go`. kubernetes: 24 of
  183 findings are in `_test.go` files (re-run grep), plus 26 more matching fake/mock/stub names
  (agent count).

### 2026-10-12 — stapler-squad/kubernetes corpus backtest — getter fixed by an external interface

- **Repo**: stapler-squad `session/git/diff.go` (`func (p *diffFilePatch) Chunks() []fdiff.Chunk { return p.chunks }`,
  unexported type implementing go-git's `fdiff.Patch`); kubernetes `SupportedMediaTypes()`
  implementing `runtime.NegotiatedSerializer`.
- **Why it's a false positive**: the interface fixes the return type and the receiver type never
  leaves the package.
- **Mechanism**: no receiver-visibility or interface check in `check_method`. Counts (30/218
  stapler-squad real+worktree, about 10 kubernetes) are agent estimates.

## Not logged here

Harness/corpus artifacts, not checker bugs: `kubernetes/vendor/` (82 of 183 findings) and
gitignored `stapler-squad/.claude/worktrees/` copies (202 of 218; the 16 real-tree findings are
the same 12 receiver/method pairs). Re-run counts, 2026-10-12.

`Server.GetOrigins` (`stapler-squad` `server/server.go`, HEAD `d8c6b3d`) is the nearest real hit;
it is a needs_discussion, not a false positive.
