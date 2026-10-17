# Backtest validation: remove-middle-man

Command: `kibitzer check architecture remove-middle-man <repo>` (release build of this branch).
Triage below is my own judgment from names and shapes, not a reviewer's verdict (INFERRED).

## Transcript backtest

`kibitzer check backtest remove-middle-man` scanned 3298 transcripts, checked 2422 snapshots
(1853 edits unreconstructable) and reported no findings. `docs/backtesting.md` says a model
checker named explicitly is replayed against a model built from each reconstructed file alone,
so this only shows the check is silent on single-file edits. A type with 3+ forwarding methods
rarely appears in one edit; it says little about real-world noise. The corpus run below is the
real signal.

## Corpus run

| Repo | Raw findings | After carve-outs | Notes |
|---|---|---|---|
| kubernetes/kubernetes (Go) | 15 | 6 | 8 `*Adapter`/`*Wrapper`, 1 test file removed |
| microsoft/vscode (TS) | 34 | 31 | 3 test-file hits removed |
| tstapler/stapler-squad (Go, TS) | 7 | 5 | 2 test-file hits removed |
| BurntSushi/ripgrep (Rust) | 0 | 0 | expected: Rust has no `delegation_field` |
| denoland/deno, servo/servo | not run | not run | `check architecture` aborts on a non-UTF-8 file (`tests/registry/npm/.../index.js`, `tests/wpt/.../script-iso-8559-1.js`) while building the import graph; pre-existing, unrelated to this check |

## False-positive rate

Carve-outs added because of this run (evidence: kubernetes names):
- Skip methods in test files (`checkers::is_test_file`, applied to the repo-relative path).
- Skip types named `*Adapter`, `*Wrapper`, `*Proxy`, `*Decorator`.

Remaining Go findings (11 across kubernetes and stapler-squad), my judgment (INFERRED):
- Plausible true positives (4): `TymuxBackend` 35/36 and `TmuxBackend` 32/34 through `mgr`;
  kubernetes `REST` 13/15 and `StatusREST` 5/6 through `store`.
- Intentional interface adapters (7): `defaultFile` over `os.File`, `grpcTransport`,
  `shellPanePTY`, `atomicLogger`, `FakeBasicVolumePlugin`, `EventSinkImpl`,
  `resourceManager`.

That is about 64% (7 of 11) false positives on Go. vscode (31 remaining) was spot-read by type
name only, not triaged one by one: most are IPC channel clients (`*ChannelClient`), log-service
decorators (`LogService`, `*LogService`) and public-API facades (`ApiRepository` 60/63) —
intentional by Fowler's own Adapter/Decorator/Proxy exceptions. Expect a similar or higher rate.

Drivers, by category:
1. **Interface-implementing forwarders** (Go `var _ I = (*T)(nil)`, TS `implements`). Checked
   directly: `TymuxBackend` declares `var _ ProcessManager = (*TymuxBackend)(nil)` at
   `session/backend_tymux.go:140`, so even the "plausible" Tymux/Tmux hits are a deliberate
   adapter. Go structural satisfaction is not in `type_edges`, so the checker cannot see this.
2. **Facades/proxies not named as such** (IPC clients, log decorators, public-API wrappers).
3. **Test fakes outside test files** (`FakeBasicVolumePlugin` in `testing.go`).

Tightenings considered and rejected:
- Require the method name to match the callee name: `TymuxBackend` forwards every method under
  the same name (`b.mgr.Start` → `Start`), so this removes none of category 1.
- Exclude forwarding through a field the type constructs: needs constructor-body analysis not
  in `ArchModel`; not attempted.

## Conclusion

The smell is inherently judgment-heavy. The finding inherits the enclosing check's severity
(`severity_override: None`, like `god_class` and `single-call-site-delegation`); it is not in
`default_checks()`, so it only runs when a repo wires it in. Not raising
the 0.8/3 thresholds: they came from the backlog item, and raising them would not remove the
interface-forwarder hits, which dominate. If this is ever promoted to a default check, the next
carve-out to evaluate is "type implements an interface" (needs Go `var _ I = (*T)(nil)` or
structural-satisfaction edges, see `single_call_site_delegation.rs` header).
