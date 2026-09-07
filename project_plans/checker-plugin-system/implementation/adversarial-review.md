# Adversarial Review: checker-plugin-system

**Date**: 2026-09-06
**Verdict**: CONCERNS (re-checked 2026-09-06, iteration 1: the sole BLOCKER is resolved; 5 pre-existing Concerns and 3 Minors below are untouched from the original review)

## Resolved (iteration 1)

- [x] **`dist-workspace.toml`'s `installers`/`tap` workspace-wide settings would have published a second public Homebrew formula for the throwaway stub plugin.** Verified resolved: plan.md Story 1.2.1 (lines 266–338) now has a concrete task (1.2.1a) that adds `[package.metadata.dist]\ninstallers = ["shell"]` to `crates/kibitzer-stub-plugin/Cargo.toml`, scoping installers to shell-only for the stub while `kibitzer` keeps `["shell", "homebrew"]` unchanged. Story 1.2.1's second acceptance criterion (lines 279–297) is a Given-When-Then that explicitly demonstrates both binaries' resulting installer sets: `kibitzer` → shell + Homebrew formula (published to `tstapler/homebrew-tap`), `kibitzer-stub-plugin` → shell only, zero Homebrew formulas anywhere in `dist plan` output. Task 1.2.1b requires actually running `dist plan` and recording the real output against both criteria (not just trusting the doc citation), with an explicit escalation path (Task 1.2.1c) if the override doesn't behave as documented. ADR-001 was appended (not rewritten) with a dated "Update (2026-09-06, adversarial-review.md BLOCKER resolution)" section recording this same resolution and its rationale. The technical claim underpinning the fix — `installers` is package-local (overridable per-package) while `tap` and `publish-jobs` are global-only — is grounded, not merely asserted: both the plan (lines 284–286, 308–312) and ADR-001's Update section cite `book/src/reference/config.md` at GitHub tag `v0.32.0` in `axodotdev/cargo-dist`. I independently fetched that exact file at that exact tag (`gh api "repos/axodotdev/cargo-dist/contents/book/src/reference/config.md?ref=v0.32.0"`) and confirmed: `installers` is annotated "since 0.0.3 · [package-local]" (config.md:719–723), `tap` is "since 0.2.0 · [global-only]" (config.md:947–949), and `publish-jobs` is "since 0.2.0 · [global-only]" (config.md:1581–1583) — the citation is accurate at the pinned version, not just plausible-sounding. Combined with Task 1.2.1b's requirement to empirically confirm behavior via a real `dist plan` run (rather than resting on doc-reading alone), this closes the original blocker's core concern about an unaddressed, undecided packaging side effect. Checked for downstream contradictions: no other task in the plan (e.g. the CARGO_BIN_EXE dev-dependency wiring at Task 5.1.1a, or the `--source <manifest-url>` install-flow example near line 911) assumes the stub gets a Homebrew formula or brew-based install path — all consistent with shell/archive-only distribution for the stub.

## Resolved (iteration 2)

- [x] **ADR-002's redirect-allowlist promise wasn't implemented by any task.** Verified
  resolved: Task 3.2.1a now checks `is_allowed_host` against the response's final,
  post-redirect URI host too (via `ureq`'s `ResponseExt::get_uri()`, confirmed present on
  `ureq = "3"`'s `Response` type), rejecting with `"not an allowed host"` if the
  post-redirect host fails the check — exactly like the pre-request check. Task 3.2.1c
  reuses the same `is_allowed_host` function for `fetch_target_bytes`'s pre- and
  post-redirect checks. Addressed in plan.md at Tasks 3.2.1a and 3.2.1c, and reflected in
  Story 3.2.1's own acceptance criteria ("A redirect that lands off-allowlist is rejected
  too").
- [x] **The "zero new hard dependencies" success metric was never reconciled with adding
  `ureq`/`sha2` to the root `Cargo.toml`.** Verified resolved: `requirements.md`'s Success
  Metrics now reads "The core `kibitzer` binary's `Cargo.toml` gains no new ML/heavy
  dependencies... the plugin mechanism itself may add small, lightweight dependencies (an
  HTTP client, a checksum crate) needed to implement install/verify, but nothing
  ML-related." Plan.md's Epic 3.2 goal restates this explicitly: "`requirements.md`'s
  Success Metrics scopes 'no new dependencies' to mean no ML/heavy runtime (`ort`, model
  weights), not literally zero." Addressed in `requirements.md`'s Success Metrics section
  and plan.md's Epic 3.2 goal text.
- [x] **Daemon-cache-staleness disposition ("Extend as-is"/document-only) was under-fixing a
  cheap, research-confirmed gap.** Verified resolved: the Tech Debt Disposition table's
  disposition for this row is now "**Fix: fold `registry.json` into `config_stamp`**," not
  documentation-only, implemented as Story 4.1.2 (`CacheEntry` gains a `registry_stamp`
  field; `Cache::get`/`put` compare/store it alongside `config_stamp`). Addressed in
  plan.md's Tech Debt Disposition table and Story 4.1.2 (Tasks 4.1.2a-c).
- [x] **Manifest-name vs. CLI-arg-name reconciliation was unspecified.** Verified resolved:
  Story 3.2.2 now carries an explicit acceptance criterion ("A manifest whose own `name`
  field disagrees with the CLI-supplied name is refused") and Task 3.2.2a implements the
  check as the first gate in `install_plugin`, bailing with `"manifest name does not
  match"` before any version-compat check, download, or `Registry` write. Addressed in
  plan.md's Story 3.2.2 acceptance criteria and Task 3.2.2a.

## Concerns

- [ ] **Pitfalls.md's explicitly prioritized risk #2 ("the registered command/binary path must be portable across machines by construction... a hard requirement, not a nice-to-have") has no corresponding disposition anywhere in the plan.** The other three research-flagged risks (command timeout, daemon-cache staleness, missing-binary signal) each get an explicit Tech Debt Disposition row; this one — despite research calling it out with equal or higher priority given this specific user's real multi-machine dotfiles-synced setup — is absent from the table, the ADRs, and Unresolved Questions. In practice the chosen design (registry.json lives under `$XDG_DATA_HOME`, is read fresh on every `find_effective_config` call, and is never written into `.claude/inspect.json`, which is the file that actually syncs via `cfgcaddy`) likely sidesteps the concern by construction — but the plan never states this reasoning or confirms the assumption that `$XDG_DATA_HOME/kibitzer/plugins/` isn't itself dotfiles-synced. — **Recommendation**: add one explicit disposition row/sentence stating why per-machine registry files avoid the cross-arch-path risk research called a hard requirement, rather than leaving it unaddressed.

## Minors

- `Registry::save` is instructed to mirror `Cache::save` "exactly" (Task 2.1.2a), which uses a plain `fs::write` with no temp-file+rename — two concurrent `plugin install`/`remove` invocations racing on `registry.json` could lose one writer's update. Consistent with this project's own single-user/low-risk posture and an existing precedent (`Cache::save`), but never explicitly acknowledged as accepted.
- Task 1.2.1b's `dist plan` verification confirms the archive/App shape but doesn't check whether adding the `kibitzer-stub-plugin` workspace member changed any *resolved* dependency version/feature set for the existing `kibitzer` package in `Cargo.lock` (the exact class of risk `pitfalls.md` §2 cites via axodotdev/cargo-dist#1740). Low actual risk since the stub's only dependency (`serde_json`) is already used by `kibitzer`, but not explicitly verified.
- The `sh -c` dispatch's documented ~64KB pipe-buffer limitation (Task 4.2.1a) is accepted as-is for today's checks; worth a one-line mention in `docs/plugins.md` since a future embedding-checker's output could plausibly be larger, even though it's out of scope to fix now.
