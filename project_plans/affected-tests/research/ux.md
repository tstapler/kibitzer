# UX Research: `kibitzer affected --base <sha>`

Scope: CLI/DX only — this is a CI-consumed, non-interactive subcommand with no GUI
surface. Findings below are grounded in kibitzer's own existing output conventions
(`src/main.rs`) and the concrete reference implementation this feature replaces
(`scripts/test-affected.py` + its `build.yml` consumer, both in
`~/Programming/stapler-squad`).

## 1. kibitzer's existing batch/report output conventions

Read `run_change_coupling`, `run_root_cause_clusters`, and `run_hotspots`
(`src/main.rs:659-742`, dispatched from the `Architecture` subcommand at
`src/main.rs:510-516`). All three share one pattern:

- **Human-prose stdout, one finding per line(s), free-text formatting** — e.g.
  `"{:.0}% coupled ({}/{} shared revisions): {} <-> {}"` (`src/main.rs:669-676`),
  `"score {}: {} ({} revisions x complexity {})"` (`src/main.rs:736-739`). Percentages,
  arrows (`<->`), and parenthetical annotations are meant for a human reading a
  terminal, not a parser.
- **No `--format`/`--json` flag anywhere in the CLI.** `grep -n "OutputFormat\|--format"
  src/main.rs` returns nothing outside of `.claude/inspect.json`-related help text.
  Every existing report command is prose-only.
- **"Report, don't gate" exit-code convention**: all three always return
  `ExitCode::SUCCESS` when the analysis itself succeeds, regardless of whether findings
  were produced — an empty result prints a friendly one-line message
  (`"[kibitzer] no coupled file pairs found above threshold"`, `src/main.rs:664`) and
  still exits 0. This is explicitly documented in the doc comments above each function
  (`src/main.rs:656,683,722-725`) as intentional: these are inspect/report tools, not
  pass/fail gates.
- **Contrast**: `run_duplicates_cli` (`src/main.rs:747-755`) and `run_architecture_cli`
  (`src/main.rs:609-652`, the checker-invocation path) *do* use exit code 1 to signal
  "found something" — these are the gating-check family, and their stdout format is
  `path:line: message` (`src/main.rs:650,791-806`), one finding per line, still
  human-prose but closer to compiler/linter conventions (`file:line:` prefix).

**Implication for `affected`**: none of kibitzer's precedent is dual-purpose
(simultaneously the thing a human reads AND the thing a shell command-substitutes).
The three `Architecture` report commands are pure-human; the checker family is
pure-machine-adjacent (`file:line:` is grep/editor-jump-friendly but still not
whitespace-safe for `$(...)`). `affected` needs a genuinely new contract — there's no
existing kibitzer format to just reuse, and matching the report family's prose style
would break the CI consumer, its primary user.

## 2. Job-to-be-done: output shape that minimizes shell friction

The direct analogues stapler-squad's own `test-affected.py` already established, and
that Unix convention independently supports:

- **`git diff --name-only`**, **`go list ./...`**, and `test-affected.py` itself
  (`~/Programming/stapler-squad/scripts/test-affected.py:133-134`,
  `print(pkg)` per line) all emit **one bare token per line, no headers, no quoting,
  no trailing decoration** on success.
- The concrete consumer (`~/Programming/stapler-squad/.github/workflows/build.yml:294-323`)
  does `PKGS="$(python3 scripts/test-affected.py "$BASE_SHA")"` then later uses `$PKGS`
  **unquoted** (`# shellcheck disable=SC2086`, line 321) specifically so it word-splits
  into separate `go test` arguments. Under `$(...)` command substitution, newlines and
  spaces are equivalent as separators (both are `IFS` word-splits) — so a
  newline-per-package format is both diffable/readable when a human runs it directly
  *and* trivially consumable by the existing CI wrapper unchanged.
- **Recommendation**: match `test-affected.py`'s existing contract exactly — plain
  package identifiers, one per line, on stdout, nothing else on stdout. This directly
  answers the requirements doc's open question: matching the existing contract isn't
  just "minimizes stapler-squad's cutover diff," it's also the Unix-conventional choice
  independent of that cutover.
- **JSON should be opt-in, not default**, if kibitzer wants to serve non-shell
  consumers later (e.g. a future GitHub Actions composite action that wants structured
  output) — gate it behind an explicit `--format json` (matching the `--format`-flag
  pattern already established for `Command::Run`'s `trigger` type at `src/main.rs:82-84`,
  even though no output-format flag exists yet). Do not make JSON the default: it forces
  every existing/future shell consumer to add a parsing step for zero benefit over plain
  lines, and there is no current consumer asking for it.

## 3. Error-state UX

Precedent from `git_toplevel`/`git_log_commits*` (`src/hotspots.rs:38-52`,
`src/change_coupling.rs:76-86,205-216,238-248`): git-command failures are wrapped in
`anyhow::bail!` with the failing command, its exit status, and stderr, e.g.
`"git log exited with {}: {}"`. These propagate through `main() -> Result<ExitCode>`
uncaught, so Rust's default `Termination` impl prints `Error: <context>: <bail message>`
to stderr and exits 1. This is a good, already-established pattern for `affected`'s
hard-failure cases:

- **Base ref doesn't exist / not a git repo**: these are exactly the shape
  `git_toplevel`/`git rev-parse` already handles — reuse the same "shell out, check
  `output.status.success()`, `bail!` with status+stderr" pattern, exit 1, message on
  stderr. Do not invent a different error contract for this one subcommand.
- **Zero changed files → distinguish from "something went wrong"**: `test-affected.py`
  already draws this line correctly and kibitzer's own `run_change_coupling`-style
  "empty result, exit 0" precedent agrees — nothing changed vs. base is not an error.
  Exit 0, empty stdout, optionally a stderr note (not stdout — stdout must stay pure
  package-list output for the `$(...)` capture) like `"No packages affected vs
  <base>"`. `test-affected.py`'s own CI wrapper already does this same split: it treats
  empty `$PKGS` as "nothing to test, exit 0" (`build.yml:295-298`), never as an error.
- **Bail-out sentinel shape**: `test-affected.py`'s `__ALL__` (a single literal line on
  stdout) is exactly what the CI wrapper string-matches (`if [ "$PKGS" = "__ALL__" ]`,
  `build.yml:299`). Keep this exact string and keep it on **stdout, exit 0** — not a
  distinct exit code. Rationale: a distinct exit code would conflate "the tool is
  telling you something" with "the tool failed," and would force every consumer
  (existing and future) to branch on exit code *and* parse stdout instead of just the
  latter. A magic string a shell script can `[ "$X" = "__ALL__" ]` against is simpler
  to integrate than out-of-band exit-code semantics, and is proven in production today.
  Matching this exact string (not inventing kibitzer's own spelling) is the literal
  answer to the requirements doc's "should it match test-affected.py's contract
  exactly" question — yes, for this specific bit of the contract, because any consumer
  string-matching `__ALL__` (this CI wrapper, and potentially other adopting repos
  copying the same pattern) breaks silently on a rename.
- Conservatism over cleverness: per the JTBD framing in the task prompt, a false
  "nothing affected" is a silent regression risk, while a spurious `__ALL__` only costs
  CI time. For a diff that *was* successfully computed, an ambiguous case (e.g. parse
  failure on any file that might gate blast radius) should resolve toward `__ALL__`
  rather than toward a hard error — an error here is at least loud (CI red), but a
  silently-too-narrow package list is a regression that ships. This does **not**
  extend to a `--base` ref that fails to resolve at all (typo'd ref, not fetched
  locally) or a missing `git` binary: with no diff computed in the first place,
  neither `__ALL__` nor a package list is a meaningful answer, so these fail loud as
  a distinct hard error (see the table below) rather than reusing
  `test-affected.py`'s blanket `except subprocess.CalledProcessError: print("__ALL__")`
  posture (`scripts/test-affected.py:78-83`) — a deliberate divergence so a CI
  wrapper can tell "something is broken" apart from "everything is affected."

## 4. Accessibility / i18n

Not applicable. `affected` has no interactive prompts, no TUI, no color-dependent
output, and its consumers are a CI wrapper script and a developer piping stdout to
another command — there is no rendering surface for accessibility or localization
concerns to attach to.

## 5. Job-to-be-done summary

- **Functional job**: "run only the tests this diff can affect, fast" — served by a
  plain, newline-separated package list that slots into `go test $(kibitzer affected
  --base main)` with zero glue code, matching `go list`/`git diff --name-only`
  conventions the target audience (a Go developer/CI script) already knows.
- **Emotional job**: trust that narrowing is safe. This is why the bail-out sentinel
  must err conservative (see §3) and why kibitzer should not get clever with, e.g.,
  heuristic package pruning beyond what the import graph proves — cleverness here
  trades a rare CI-time win for a rare, hard-to-notice under-testing risk, a bad trade
  for a test-selection tool specifically.
- **Social/team job**: a CI check the whole team (or, for a solo-maintainer tool,
  future-you and any other kibitzer-adopting repo) can trust without reading
  `affected`'s source each time. Reusing `test-affected.py`'s already-proven contract
  (plain list, `__ALL__` sentinel, exit-0-on-empty) rather than inventing kibitzer's own
  vocabulary is itself a trust-building UX choice: it's a contract stapler-squad's CI
  has already run in production, not a novel one nobody has stress-tested.

## Recommendation summary

| Question | Recommendation |
|---|---|
| Default output format | Plain package identifiers, one per line, stdout only — no header/footer/decoration |
| JSON output | Opt-in only, behind a future `--format json`, not the default |
| Nothing changed / nothing affected | Exit 0, empty stdout, optional stderr note |
| Base ref invalid / not a git repo | Exit 1, `anyhow::bail!`-style message on stderr, matching `git_toplevel`'s existing pattern (`src/hotspots.rs:38-52`) |
| Bail-out sentinel | Literal `__ALL__` string on stdout, exit 0 (not a distinct exit code) — match `test-affected.py` exactly |
| Ambiguous/uncertain cases | Resolve toward `__ALL__`, never toward a narrowed-but-possibly-wrong list |
