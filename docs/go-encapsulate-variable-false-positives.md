# `go-encapsulate-variable` check — known false positives

Tracks noise from `go-encapsulate-variable` (`src/encapsulate_variable.rs`). Check new
occurrences here before re-investigating.

## Mechanism

A write `x.Field = v` / `x.Field++` is flagged when `Field` is uppercase-first, the declaring
package differs from the writer's, and the declaring type's name does not end in a
`DTO_SUFFIXES` entry. `x` must be syntactically typed `pkg.T` / `*pkg.T` (parameter, `var`,
`:=` composite literal). Test files are skipped. `pkg.Var.Field = v` and writes through call
results or interface values are not seen. Suppress one site with
`// kibitzer:ignore encapsulate-variable -- <reason>`.

## Backtest log

### kubernetes/kubernetes (`kibitzer check architecture encapsulate-variable`, debug build, ~3m50s)

| Run | Allowlist | Findings |
|-----|-----------|----------|
| 1 | the 9 required suffixes (`Config`, `Options`, `Opts`, `Params`, `Args`, `Request`, `Response`, `Settings`, `Result`) | 2161 |
| 2 | + `Configuration`, `Status`, `Spec`, `Stats`, `Condition`, `Info`, `Record`, `Row` | 997 |

Run 1 was dominated by API/data-shaped types (322 `*Configuration`, 237 `*Status`, 128
`*Condition`, 126 `*Spec`, 109 `*Stats`, 101 `*Row`), all public-and-mutable by design, hence
the added suffixes. Run 2's top types are still data structs (`Pod`, `Table`, `Event`,
`APIResource`, `Device`, ...). 408 of the 997 are under `staging/src`, 227 under `pkg/apis`
(conversion/defaulting/fuzzer code that fills API objects).

**Triage of run 2** — sample of 13 findings (every 83rd line of 997), judged from the
finding text and package paths only; the code was not opened, so this is an estimate (n=13):

- Likely false positive (API object filled by conversion/defaulting/fuzz/test-util code): 11
  (`SetDefaults_MutatingWebhook`, `copyHPAScalingRules`, `Convert_*`, `fuzzDNS`,
  `v1FuzzerFuncs`, `FakeNodeHandler.UpdateStatus`, `ConvertToTable`, `downloadLegacy`,
  `registerResourceHandlers`, `buildLeafTemplate`, `securityContextFromPodSecurityContext`).
- Plausible true positive: 2 (`GracefulTerminationManager.GracefulDeleteRS` writing
  `ipvs/util.RealServer.Weight`; `checkRegistry.EvaluatePod` writing
  `api.LevelVersion.Version`).

Not established: a full-corpus true-positive rate. Remaining noise is "filling a type from
another API package"; a per-package scope (e.g. skip declaring packages named `*/api/*`,
`*/apis/*`) is a candidate follow-up, not done here.

### Second corpus repos (same build, run-2 allowlist)

| Repo | Findings | Reading |
|------|----------|---------|
| caddyserver/caddy | 80 | 25 shown (every other line) all in `caddyconfig/httpcaddyfile`: the Caddyfile adapter filling `caddytls.*`/`caddyhttp.*` JSON-config structs (`ACMEIssuer.Challenges`, `Server.AutoHTTPS`, `TLS.Automation`, ...) — config-builder pattern, judged false positives from the finding text. Lines 26+ not read. |
| casbin/casbin | 10 | All 10 in `util_log.go`: `Enforcer` filling `log.LogEntry` fields (`Subject`, `Object`, `RuleCount`, `Error`, ...) — a plain log record, judged false positive. |

On these two repos no finding looked like a real encapsulation gap; that is a reading of the
text, not of the code, so treat "0 true positives" as unverified. Candidate follow-up: a
suffix for record types (`Entry`) and/or a package-scope exemption for builder packages.
