# Suppressing a check

kibitzer runs a built-in default catalog everywhere — pylint-style, no
`.kibitzer/inspect.json` required (`config::default_checks()` in `src/config.rs`;
the full list is in `docs/syntax-rules.md` and `docs/comment-quality.md`, plus
`markdown-link-integrity`, `primitive-obsession`, `duplicate-code`,
`duplicate-code-cross-file`, `file-complexity`, `go-blank-imports`,
`go-ignored-error`, `go-error-context`). A local
`.kibitzer/inspect.json`
overlays that catalog rather than replacing it — see
`config::find_effective_config`. To dismiss one finding you judged acceptable,
write a `kibitzer:ignore` comment at the flagged spot (see
[Dismiss one finding inline](#dismiss-one-finding-inline)). The other levers in
this doc are config-based and cover whole checkers, files, or directories.

## Turn a default check off entirely

Add its `name` to `disabled` in the repo's `.kibitzer/inspect.json`:

```json
{
  "disabled": ["primitive-obsession"]
}
```

This only removes a *default*. It has no effect on a check you added
yourself — just delete it from `checks` instead.

## Change a default's severity or scope

Add a `checks` entry that reuses the default's exact `name`. It replaces the
default outright rather than running alongside it:

```json
{
  "checks": [
    {
      "name": "markdown-link-integrity",
      "checker": "markdown-link-integrity",
      "severity": "advisory",
      "scope": ["**/*.md"]
    }
  ]
}
```

## Exclude one file or directory, keep the check everywhere else

`scope` patterns prefixed with `!` are exclusions (`src/glob.rs::matches_scope`),
checked after the positive patterns — gitignore-style:

```json
{
  "checks": [
    {
      "name": "primitive-obsession",
      "checker": "primitive-obsession",
      "severity": "advisory",
      "scope": ["**/*.go", "!vendor/**", "!internal/generated/api.go"]
    }
  ]
}
```

A `checks` entry with only negative patterns (no positive ones) matches
everything except what it excludes — equivalent to starting from `**/*`.

## Dismiss one finding inline

The levers above are whole-checker or whole-file/directory. For a genuine hit at
one specific line that you've deliberately decided to keep — a real
`flag-argument` match on a CLI's standard `-v` toggle, say — disabling the whole
checker for that file would also silence every other rule it covers there. Put a
comment at the flagged spot instead:

```go
// kibitzer:ignore flag-argument -- standard -v toggle, matches the CLI contract
func run(verbose bool) {}
```

In Markdown the comment is an HTML comment:

```markdown
<!-- kibitzer:ignore markdown-link-integrity -- placeholder until the page lands -->
```

### Grammar

```text
<comment leader> kibitzer:ignore <rule>[,<rule>...] -- <reason>
```

- **One marker.** Only `kibitzer:ignore` exists. `kibitzer:false-positive`,
  `kibitzer:disable` and similar near-misses are reported as `[ignore-syntax]`
  and suppress nothing.
- **Leaders.** `//`, `#`, `/* */`, `<!-- -->` and the doc-comment forms, in a real
  comment (a string literal never counts). The directive must start the comment.
- **`<rule>`** is the finding's `[rule]` prefix, or the checker name. Several
  rules go in one comma-separated list. Findings with no `[rule]` prefix answer to
  their checker name; the hook footer lists the ids to copy. Rule ids are lowercase
  `[a-z0-9-]`, at most 64 characters, and a directive names at most 16 of them.
- **`<reason>`** is required: at least two words, at most 300 characters, and not
  just the rule id repeated. Use `--`; an em dash is rejected. An over-long rule id,
  rule list or reason is reported as `[ignore-syntax]` and suppresses nothing.
- **Native checkers only.** Directives apply to kibitzer's own per-file checkers (the
  ones `kibitzer check list` shows). A shell `command` check, a plugin, an
  architecture check and a file type with no native checker ignore them, so the hook
  and MCP footers offer the syntax only when a failing native finding could use it.
- **Scope.** A comment on its own line covers its own rows and the row directly
  below. If comment lines follow it with no blank row between (a reason continued on
  `//` lines), it covers the first row after that stack. A trailing comment after
  code covers only its own row. In Markdown, list (`-`, `1.`) and blockquote (`>`)
  markers before the comment do not make it trailing.
- **Misplaced directives.** A directive after other text in a comment is reported
  (`[ignore-syntax]`) only when a bare `TODO`/`FIXME`/`NOTE`-style label precedes it;
  prose that merely describes the syntax is left alone. An upper-case marker such as
  `KIBITZER:IGNORE` is reported as a near miss.
- **Unused ignores.** `kibitzer run` reports a directive that suppresses nothing as
  `[unused-ignore]`: "remove it" when no finding of that rule exists, or the row of
  the nearest finding when the comment sits on the wrong row. A rule that is not a
  known rule or checker name is reported as a probable typo instead. It judges
  against findings before any `accepted/` entry, so an ignore an `accepted/` entry also
  covers is not unused. A directive is not judged when its checker is disabled or
  did not run on that file (too large, excluded by trigger), nor with `--no-inline-ignores`.
- **Never suppressible:** the directive diagnostics themselves (`inline-ignore`,
  `ignore-syntax`, `unused-ignore`, `ignore-volume`, `blocking-suppressed`).
- A malformed directive never suppresses; its `[ignore-syntax]` message says the
  exact fix.

### Where the comment goes

Most checkers anchor on the flagged statement: the comment goes on the line above
(or at the end of that line). The exceptions:

| Checker | Where the ignore goes |
|---|---|
| `file-size` (also its per-language checker names such as `python-file-size`) | First 10 lines of the file, or the reported (last) line |
| `file-complexity` | Above one function to dismiss only it; in the first 10 lines to dismiss all of them |
| `duplicate-code` | On its own row directly above the last block's first line. A trailing comment becomes part of the block text and re-anchors the finding |
| `duplicate-code-cross-file` | Same, in each file you want quiet; one file's ignore does not cover the other |
| `markdown-link-integrity` | Name the checker, not the link label: `markdown-link-integrity`, because the `[label]` in its messages is data |
| `over-commented`, `commented-out-code` | Directly above the first flagged comment row; for dead code, delete it instead |

### Limits worth knowing

- **Stale ignores.** In the PostToolUse hook an unused (stale) ignore is reported
  only when the directive sits inside the lines just edited. A stale ignore
  elsewhere in a file is not reported by the hook, and `kibitzer run` does not
  audit them yet. `kibitzer run` does flag a rule name that matches no known rule
  or checker, as a probable typo.
- **Visibility.** `kibitzer run` prints how many findings were suppressed inline
  (and how many came from blocking checks); `kibitzer run --no-inline-ignores`
  shows them again. A directive that silences a blocking finding raises
  `[blocking-suppressed]` in the hook so the agent tells the user (at most 10
  advisories, then one count line). The hook keeps no state between calls: it compares
  the blocking findings the file's directives silence now with the ones they silenced in
  the file's git HEAD content, matching by rule and the text of the silenced line, and
  counting identical lines by occurrence. It reports the silenced findings that are new
  relative to HEAD: one that appeared, code that slid under a directive, a duplicate
  of an already-silenced line, or a file-scope finding (`file-size`, `file-complexity`)
  that the file only now trips. An edit that touches the directive's rows or the silenced
  row always reports. A staged rename (`git mv`) reads the renamed-from HEAD blob, and a
  HEAD blob that is not UTF-8 is decoded lossily. When git gives no baseline (no repo, no
  commits, an untracked, ignored, newly added or submodule path, a git that fails or does
  not answer in time) the baseline is "unknown", never "everything is new": a
  whole-file `Write` reports every silenced finding, and an `Edit` reports an untouched
  one only when the edit removed lines (so a finding may have slid under a directive) or the
  rule is file-scope. The hook passes that "removed lines" flag to the daemon; nothing is
  stored between calls. Every baseline git call is bounded: 2 seconds for `show`, `diff` and
  the rename probe, 10 seconds for the whole-tree `git archive` a repo-wide command check
  needs, and 5 seconds of git time in total per hook, daemon, MCP or LSP request (after
  that the remaining calls read as unavailable without running). A timed-out git runs in its
  own process group, and the whole group is killed. The baseline git calls drop `GIT_DIR`, `GIT_WORK_TREE`,
  `GIT_INDEX_FILE` and the other repo-redirecting variables, so a hook launched from inside
  another git command still reads the edited repo. The baseline is HEAD, not the previous
  edit, so until you commit, later edits to the same file report the earlier new
  suppressions again (at most 10 lines and a count); there is no per-file memory to dedupe
  that, on purpose, since a memory would hide the advisory from an agent that lost it to
  context compaction.
  The comparison costs one `git show` plus one extra checker run over the HEAD content,
  and only when the file carries the `kibitzer:ignore` marker and a blocking finding
  was silenced. The advisory shows the reason in quotes, cut to 160 characters.
- **Failing CI on a suppressed blocking finding.** `kibitzer run` exits 0 when the
  only blocking finding was suppressed inline. Add `--deny-blocking-suppression` to
  exit 1 instead; the default is unchanged.
- **Untrusted text.** Two sanitizer tiers (`src/inline_ignores/sanitize.rs`):
  - *Strict* (`echo`): directive reasons, rule text and near-miss markers that kibitzer
    composes into its own `[ignore-syntax]`, `[unused-ignore]` and `[blocking-suppressed]`
    lines. Only printable text survives (an allow-list over assigned code points: no
    control, format, unassigned, variation-selector, tag, bidi, filler or other invisible
    characters), whitespace
    collapses to one line, at most two combining marks (any script) follow a character, and
    the text is shortened; a reason is shown in quotes as data. File paths in those lines use
    `echo_path`: the same limits, but a character the allow-list would drop (and any
    newline or tab) is shown as `\n`, `\t` or `\u{..}`, and spaces are kept as written. A ZWJ
    between two emoji or between letters of an Arabic, Indic or Myanmar script, a ZWNJ between
    letters of those scripts, and a VS16 right after an emoji base are kept as written; any
    other joiner is escaped.
  - *Lenient* (`strip_unsafe`): the finding text other checkers produce, on hook stderr,
    hook `additionalContext` (PostToolUse and Stop), `run_checks` and
    `architecture_assessment` MCP output, LSP diagnostics, `kibitzer check native`,
    `kibitzer check architecture` and `kibitzer run` stdout. Also an allow-list: every
    assigned graphic character (general category L*, N*, P* or S*: letters, numbers,
    punctuation, symbols, emoji, in any script), combining marks (below), tabs, newlines and
    the spaces ASCII space, NBSP, U+2009, U+202F and U+3000 survive. Six assigned but blank
    symbols are dropped (Braille blank U+2800, U+303F, U+13441, U+13442, U+1D159, U+FFFC).
    "Assigned" comes from range tables generated from Unicode 18.0.0
    (`scripts/gen-unicode-tables.py` writes `src/inline_ignores/unicode_tables.rs`; the script
    header says how to regenerate). A code point assigned in a later Unicode version is
    dropped until the tables are regenerated, so the failure is safe: a new letter is lost,
    never an invisible kept. Everything
    else is dropped: controls (carriage return included), unassigned, private-use and
    noncharacter code points, every Default_Ignorable_Code_Point (soft hyphen, grapheme
    joiner, fillers, bidi embeddings, overrides and isolates, deprecated format characters,
    tag characters, Mongolian selectors), and every other format or space character. A
    small set of invisibles keeps legitimate text and survives only where it does real work,
    judged from its raw neighbors so a run never helps itself (a second one in the same gap
    is dropped):
    - ZWJ between two emoji (arrows such as U+2194 count), or between two letters or digits of
      one Arabic, Indic or Myanmar script; ZWNJ between two letters or digits of those scripts
      (Persian, Devanagari, Bengali and so on); either one ending a word right after a
      script's own mark (the Malayalam chillu `ന്‍` before a space or at the end).
    - ZWSP and word joiner between two letters or digits of one Thai, Lao, Khmer or Myanmar
      script (a BOM in the middle of text is dropped).
    - LRM, RLM and the Arabic letter mark beside a right-to-left letter, one per gap: any
      invisible directly before one, kept or dropped, ends the allowance.
    - A variation selector directly after a base that has variation sequences, one per base,
      and only on a base that itself survives: CJK ideographs take VS1-VS16 and the
      ideographic selectors; emoji and symbol bases take only VS15 and VS16; other bases take
      the pairs listed in Unicode's `StandardizedVariants.txt` (math operators, a few
      hieroglyphs); after ASCII only VS15/VS16 after `#`, `*` or a digit.
    - The England, Scotland and Wales flags (U+1F3F4, the tags for `gbeng`, `gbsct` or
      `gbwls`, U+E007F); no other tag run.
    - Combining marks (Mn, Mc and Me, so enclosing marks count), at most three in a row
      (Tier 1: two), on a visible base; on a Tibetan, Myanmar, Khmer or Indic base up to five,
      of which at most three are generic accents. A generic diacritic (U+0300-036F and the
      other combining blocks) may follow anything, any other mark only a base of its own
      script (Thai tone marks after Thai, the Khmer coeng after Khmer, and so on). Alphabetic
      marks such as Devanagari vowel signs count too.

    Residual covert bandwidth, not none. An attacker who controls finding text can still
    encode data in what survives, per carrier character: about 8 bits per CJK ideograph
    (257 selector states: none, U+FE00-FE0F, U+E0100-E01EF), about 1.6 bits per emoji or
    symbol that takes VS15/VS16 (3 states: none, VS15, VS16; up to 3 bits for the few bases
    with several listed standardized sequences, such as 8 states on one hieroglyph), per
    Arabic/Indic/Myanmar letter pair (none, ZWJ, ZWNJ) and per Thai/Lao/Khmer/Myanmar letter
    pair (none, ZWSP, word joiner), 2 bits per right-to-left letter (none, LRM, RLM, ALM),
    about 2.3 bits per space character (five kinds of space), and, as visible accent stacks,
    up to three generic marks per base (289 generic marks, about 8.2 bits each, about 25
    bits per base) or, on a Tibetan, Myanmar, Khmer or Indic base, three generic and two
    script marks (about 37 bits per base at most; Tibetan has 77 marks). A run of invisibles
    over an ASCII carrier carries nothing: before round 6 3,766 invisible code points
    survived at about 11.9 bits each with no limit, and before round 7 a further 508
    unassigned code points survived because whole blocks were allowed. A homoglyph or
    word-choice channel in visible text is out of scope here.
    Not kept, because the cost of keeping them is a wider channel: Mongolian free variation
    selectors, Arabic number signs and other Cf characters, non-ASCII spaces other than the
    four above, and joiners in scripts not listed.
  - A file path in lenient output has its newlines and other unsafe characters shown as
    escapes (`\n`), so a file name cannot start a forged line. Findings that name another
    file (`duplicate-code-cross-file`, architecture findings) escape that file's path where
    they print it; in addition the edited file's path and base name, and every entry in
    its directory whose name needs escaping, are escaped wherever they occur in the text.
    Not covered: an external command check that prints a hostile name from a different
    directory in a form kibitzer does not generate (it still gets the control-character
    strip, but its newlines stay).
  - Direct-print subcommands in `src/main.rs`: `kibitzer check architecture` runs finding
    messages through the lenient tier (with sibling-name escaping for the directory it was
    given); `check native`, `check duplicates` and `status` escape paths. Other diagnostics
    there were not audited again.
- **Footers never suggest meta rules.** When the only failure is an `[ignore-syntax]`
  repair, the footer omits the ignore hint instead of suggesting a directive that
  cannot work.
- **`kibitzer check native <name> <file>`** runs the raw checker and bypasses
  inline ignores (and `accepted/`), so it shows what an ignore is hiding.
- **Fallback-only files.** The malformed-directive and unused-ignore diagnostics run
  on grammar-backed languages and Markdown. Other files (shell, YAML, and so on)
  honor whole-line `//`, `#`, `;` and `--` directives but never report a malformed one.
- **Dev builds.** The daemon's `cache.json` is keyed on the package version, so a
  rebuild at the same version can serve stale results. After rebuilding, delete
  `cache.json` (under `$XDG_CACHE_HOME/kibitzer/` or `~/.cache/kibitzer/`) or
  restart the daemon. A daemon left running across an upgrade is retired
  automatically: every reply carries its version, and a client that sees a different
  one (or none) stops using that daemon and asks it to exit. A replacement is spawned
  at most once per 10 seconds, so two kibitzer versions on one machine cannot restart
  each other on every hook. Only one daemon runs per socket: it holds an exclusive
  lock (`kibitzer-<user>.lock` beside the socket, mode 0600, holding its pid) and a
  second `daemon start` exits instead of taking the socket over. The socket and lock
  live in `$XDG_RUNTIME_DIR`, or in a `kibitzer-<uid>` directory (mode 0700) under the
  temp dir. `$XDG_RUNTIME_DIR` is used if it is a directory you own that group and others
  cannot write in (mode 0755 is fine: the socket and lock are 0600, so others can see
  their names but not connect or replace them), or a symlink (WSLg) whose target is a
  private directory you own, in which case the resolved path is used. kibitzer never
  chmods it. Otherwise (another user's directory, group- or other-writable, a symlink to
  anything else) it falls back to `<tmp>/kibitzer-<uid>` and notes that once an hour in
  the hook log (`kibitzer status` shows "Runtime directory fallback"). Only when that
  fallback is untrusted too (a planted symlink or another user's directory) is there no
  daemon: hooks run checks in-process and note "Hooks ran without the daemon" (at most
  once an hour per reason and six an hour in all); the note clears itself the next time a
  hook finds a usable directory, and `kibitzer status` shows only notes still in force.
  `kibitzer daemon start` exits 1 naming both directories and why, and `daemon status` and
  `daemon stop` print `runtime dir untrusted: <dir> (...); a daemon started earlier may
  still be running (pid N in <lock>)`, since the directory may have been loosened after a
  daemon started. `daemon status` on a stopped or hung daemon prints `daemon not responding
  (pid N)` and exits 1; `daemon stop` on a holder it cannot verify names its pid.
  A client probes the daemon with a 750 ms ping before
  each request; a daemon that holds its lock but never answers (stopped, deadlocked) is
  skipped for 10 seconds, and a new `daemon start` (or `daemon stop`) terminates it with
  SIGTERM then SIGKILL, but only after checking that the pid in the lock is this user's
  `kibitzer daemon start` (the executable path may contain spaces) and is older than the
  lock file, and after pinging the holder once more right before signalling. A refused
  connection while the lock is still held (macOS refuses once a stopped daemon's backlog
  is full) counts as a wedged daemon, not as no daemon.

## Accept one specific, correctly-flagged finding

Where a location cannot hold a comment (generated code, a file you must not edit),
see `docs/accepting-findings.md` for `.kibitzer/accepted/`, a per-finding lever
kept in a checked-in file. It requires a written reason and covers only that one
location.

## If a finding looks flat-out wrong

That's not a suppression question — see `docs/reporting-false-positives.md`
for how to report it so the underlying check gets fixed.
