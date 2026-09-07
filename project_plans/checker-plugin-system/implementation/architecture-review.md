# Architecture Review: checker-plugin-system
**Date**: 2026-09-06
**Verdict**: CONCERNS (re-checked after iteration 1 — see Resolved section; the blocker is closed, 4 pre-existing concerns and 3 nitpicks remain)

## Constitution Check

`docs/adr/ADR-000-architecture-constitution.md` does not exist in this repository
(`ls docs/adr/` → no such directory). No constitution to check the plan against — skipped,
not a finding.

## Grounding notes

- All plan claims about the current codebase were re-verified directly against
  `src/config.rs`, `src/check.rs`, `src/main.rs`, `src/cache.rs`, `src/install.rs`,
  `src/mcp.rs`, `src/hook.rs` at the cited line numbers — `Check`'s field shape,
  `find_effective_config`/`merge_checks`/`default_checks()`, `Cache::load`/`save`'s
  fail-open-to-empty behavior, `Command`/`DaemonAction`'s nested-subcommand shape,
  `install.rs`'s lifecycle-module precedent, and `mcp.rs`/`hook.rs`'s rendering call
  sites all match the plan's citations exactly. No drift found.
- `kibitzer run src --trigger batch` was run against the touched directory. The only
  findings are `dogfood-component-deps`/`content-rules`/`naming-rules` advisories on the
  repo's own `.claude/inspect.json` `architecture.components`, which are scoped to
  `testdata/dogfood-architecture/**` (a fixture used to dogfood the architecture-checker
  feature itself, not real `src/` code) — as the task brief anticipated, this repo's
  component-deps/layering model doesn't cover `src/*.rs`, so no mechanical
  architecture-checker findings apply to the files this plan touches. No existing
  hotspot finding surfaced for `config.rs`/`check.rs`/`main.rs`/`cache.rs`.
- `research/build-vs-buy.md`'s recommended stack (hand-rolled installer orchestration,
  `ureq` for HTTP, `sha2` for checksums, a kibitzer-specific JSON manifest schema) is
  followed exactly by Tasks 3.2.1a/3.2.2b/3.2.2e/2.1.1a — full consistency, no
  build-vs-buy divergence found.

## Blockers

None open.

## Resolved (iteration 1)

**RESOLVED** — **Story 3.1.1 / Task 3.2.2c / Task 3.3.2b — unvalidated plugin `name` flows
  directly into filesystem path construction, including a recursive delete.** Verified
  resolved: `plan.md`'s Domain Glossary (`PluginName` entry) now defines a parse-at-boundary
  smart constructor — `PluginName::parse(s: &str) -> Result<Self>`, allowlist
  `^[A-Za-z0-9_-]{1,64}$` — and it is actually wired in, not just declared: Task 3.1.1a
  types `PluginAction::Install`/`Remove`/`Status`'s `name` fields as `PluginName` via a
  clap `value_parser`, so an invalid name is rejected at argument-parsing time, before
  `main()` dispatch. Both originally-flagged filesystem call sites now consume the
  validated type: Task 3.2.2c's binary write and Task 3.3.2b's `fs::remove_dir_all` both
  join `default_plugin_dir()` against `name.as_ref()` where `name: &PluginName`, with an
  inline comment on each citing this as the BLOCKER fix. A concrete Given-When-Then
  demonstrating rejection of `"../../etc"`/`"foo/bar"` before any filesystem operation
  appears twice — Story 2.1.1's third AC and Story 3.1.1's second AC. A full-plan grep
  for residual `name: String`/`&str` typing found only `missing_binary_for(check_name:
  &str)`, which is explicitly and correctly justified in the glossary (it must also match
  non-`PluginName`-shaped hand-authored check names, and only reads an already-stored
  `binary_path` — it never re-joins a path from the raw string) — no new inconsistency
  introduced.

## Resolved (iteration 2)

- [x] **Story 3.2.1 (Tasks 3.2.1a, 3.2.1c) — the HTTPS-vs-local-path classification and
  host-allowlist check (ADR-002) was implemented twice, independently.** Verified resolved:
  plan.md's Domain Glossary now names `is_allowed_host` as the single shared implementation
  of the allowlist rule (`PLUGIN_HOST_ALLOWLIST` membership or `.githubusercontent.com`
  suffix). Task 3.2.1a implements it once and calls it for both the pre-request host and the
  post-redirect host (via `ureq`'s `ResponseExt::get_uri()`); Task 3.2.1c's comment now
  explicitly states it "reuse[s] Task 3.2.1a's `is_allowed_host`... rather than re-deriving
  the allowlist rule here — this was the second of the two independent copies
  architecture-review.md flagged; there is now exactly one." Addressed in plan.md at the
  `is_allowed_host` Domain Glossary entry and Tasks 3.2.1a/3.2.1c.

## Concerns

- [ ] **Story 2.1.1 / Task 2.1.1b — `InstalledPlugin` flattens three distinct concerns
  into one struct** — `name`/`version`/`min_kibitzer_version`/`sha256`/`binary_path`
  (plugin identity and install provenance) sit alongside `severity`/`scope`/`triggers`/
  `output_format` (purely "what `Check` to synthesize from this plugin," consumed only
  by `registered_plugin_checks()`). Nothing is incorrect today, but the struct conflates
  "what this plugin is" with "how it registers as a check," and a future second
  Check-producing consumer (or a second plugin kind) would have no obvious place to put
  registration parameters without either duplicating them or reaching back into
  `InstalledPlugin`. **Remediation**: nest the four Check-construction fields in a small
  `CheckTemplate` value object field on `InstalledPlugin` (`InstalledPlugin { name,
  version, min_kibitzer_version, sha256, binary_path, check_template: CheckTemplate }`),
  making `registered_plugin_checks()`'s mapping read as "identity + template → Check"
  rather than eight flat fields of mixed provenance.

- [ ] **Task 4.3.1b — `CheckResult.plugin_missing: bool` compounds an already-flagged
  flat-field-accretion pattern instead of isolating from it** — `CheckResult`'s own doc
  comments on `command` and `findings` already acknowledge those fields were bolted on
  incrementally for cache-compatibility reasons rather than the struct modeling outcomes
  as a proper sum type. Adding `plugin_missing: bool` alongside the existing `passed:
  bool`/`severity: Severity` is one more boolean in that same accretion, and nothing in
  the type prevents constructing an incoherent combination (e.g. `passed: true,
  plugin_missing: true`) even though today's single early-return call site never does
  so. The Tech Debt Disposition table correctly labels this item "Isolate via seam" for
  *behavior* (the `missing_binary_for` preflight check is a clean, well-named seam), but
  the *result representation* itself isn't isolated from the pattern it's extending.
  A full outcome-sum-type refactor of `CheckResult` is disproportionate for this
  project's scope, but the minimum fix is cheap: document the invariant
  (`plugin_missing == true` implies `passed == false` and `severity == Advisory`)
  directly on the field like `command`/`findings` are documented, and have Task 4.3.1b's
  early return go through a named constructor (e.g. `CheckResult::plugin_missing(check,
  path)`) rather than a hand-built struct literal, so the invariant lives in one place
  instead of being re-asserted by whoever writes that literal.

- [ ] **Story 5.2.1 — the `remove` reference-guard's scope (Task 3.3.2a) is narrower than
  the mechanism's own "install is machine-wide" model, and that asymmetry isn't
  documented** — `plugin remove`'s guard checks only the *current directory's*
  `.claude/inspect.json` (`config::find_config`), while `plugin install` (per Step 0.5's
  chosen design) registers the plugin to run in *every* repo on the machine. A
  hand-authored check referencing the plugin by name in a different repo than the one
  `remove` is run from is silently missed by the guard. Task 5.2.1a's "Known limitations"
  section is currently scoped only to the daemon-cache-staleness item (Tech Debt
  Disposition item b); this asymmetry should get the same one-sentence disclosure
  treatment, since it's the same class of "the mechanism's own effective scope doesn't
  match a naive reading of one command's guard" gap.

## Nitpicks

- `src/plugin.rs` will end up holding domain types (`PluginManifest`/`Registry`),
  infrastructure I/O (HTTP fetch, checksum verification, filesystem placement), and
  CLI-facing rendering logic in one file — matches `src/install.rs`'s existing
  single-module-per-lifecycle precedent, so it's a reasonable "extend as-is" for v1, but
  if a second, non-stub plugin (the eventual embedding checker, per requirements.md)
  needs materially more logic, split fetch/verify into a `plugin/fetch.rs` submodule
  before the file becomes unwieldy, not after.
- The Pattern Decisions table labels `PluginAction`'s dispatch "Command pattern (GoF)."
  It's really an idiomatic `match`-per-`Subcommand`-variant — the same shape
  `DaemonAction` already uses — which the `design-patterns` skill notes is the Rust/Go
  idiom that *replaces* GoF Command, not an instance of it. Harmless — the code shape is
  correctly minimal — but worth a wording fix in the plan itself.
- `version_meets_minimum` re-parses the raw version strings from scratch at each of its
  two call sites (`install_plugin`'s gate, `plugin status`) rather than the type carrying
  proof of a prior parse. Not worth fixing given ADR-003's own explicitly narrow,
  two-call-site scope — flagged only for completeness under the type-driven-design lens.
