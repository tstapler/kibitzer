# Backtest validation: remove-middle-man

Command: `kibitzer check architecture remove-middle-man <repo>` (release build of this branch).
Triage below is my own judgment from names and shapes, not a reviewer's verdict (INFERRED).

## Transcript backtest

`kibitzer check backtest remove-middle-man` scanned 3298 transcripts, checked 2422 snapshots,
and reported 0 findings. That is not evidence of low noise: `backtest` replays per-file
checkers from `checker::registry()`, and this whole-repo `ArchModelChecker` is not in it
(same as `single-call-site-delegation`). The corpus run is the real signal.

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
- Skip methods in test files (`duplicate_code_fuzzy::is_test_file`).
- Skip types named `*Adapter`, `*Wrapper`, `*Proxy`, `*Decorator`.

Remaining Go findings (11 across kubernetes and stapler-squad): most are interface-implementing
forwarders (`defaultFile` over `os.File`, `grpcTransport`, `shellPanePTY`, `atomicLogger`,
`FakeBasicVolumePlugin`, `EventSinkImpl`). Plausible true positives: `TymuxBackend` 35/36 and
`TmuxBackend` 32/34 through `mgr`, and the kubernetes `REST`/`StatusREST` 13/15 and 5/6 through
`store`. That is roughly 4 of 11 plausible and 7 of 11 intentional (about 64% false positive).

vscode (31 remaining) was spot-read by type name only, not triaged one by one: most are IPC
channel clients (`*ChannelClient`), log-service decorators (`LogService`, `*LogService`), and
public-API facades (`ApiRepository` 60/63) — intentional by Fowler's own Adapter/Decorator/Proxy
exceptions. Expect a similar or higher false-positive rate there.

## Conclusion

The smell is inherently judgment-heavy, so the finding stays advisory (no severity override
needed; it is not in `default_checks()`, same as `single-call-site-delegation`). Not raising
the 0.8/3 thresholds: they came from the backlog item, and raising them would not remove the
interface-forwarder hits, which dominate. If this is ever promoted to a default check, the next
carve-out to evaluate is "type implements an interface" (no Go structural-satisfaction edges
exist yet, see `single_call_site_delegation.rs` header).
