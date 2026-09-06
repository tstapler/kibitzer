# UX Research: architecture-export

Two consumer populations: AI agents (MCP tool calls / CLI-exported JSON) and human
developers (CLI output, rendered diagram, LSP-integrated editor). Findings below are
grounded in kibitzer's own house style (`src/mcp.rs`, `src/install.rs`, `src/mermaid.rs`,
`src/main.rs`) plus general dev-tool conventions.

## 1. Comparable UX patterns

### MCP tool schema — house style already in `src/mcp.rs`

The existing three tools (`list_checks`, `run_checks`, `architecture_assessment`) show a
consistent, guessable convention worth preserving exactly:

- **Parameter naming**: every tool that takes a filesystem location calls the field
  `path` (`ListChecksRequest.path`, `ArchitectureAssessmentRequest.path`) or `file_path`
  when it specifically means a single file (`RunChecksRequest.file_path`). There is no
  `repo_path`, `root`, `directory`, or `target` anywhere. This is the exact naming
  guessability lesson from this session's earlier `architecture_assessment` friction
  (expecting `repo_path`, getting `path`) — the new tool(s) must use `path` for a
  repo-root/subdirectory argument and `file_path` only when scoping to one file, not
  invent a third synonym.
- **Every field has a doc comment** used as the JSON-schema description (`schemars`
  derives from `///` comments), e.g. `ArchitectureAssessmentRequest.scope`: "Optional
  glob (relative to the repo root, `**` supported) restricting which files are in
  scope... Defaults to the whole repo." Each comment states default behavior explicitly
  — an agent should never have to guess what omitting an optional field does.
  `include_diagram`'s comment goes further and states the size-based fallback behavior
  up front ("Repos over 150 nodes fall back to a text note instead — pass a narrower
  `scope`..."), pre-empting a follow-up query.
  → the new tools' request structs need the same: doc-comment every field, state
  defaults, and state fallback/degradation behavior inline rather than leaving the
  agent to discover it from output text.
- **Tool-level `description` states scope explicitly**: `architecture_assessment`'s
  description says "Run a **whole-repo** architecture assessment" — the word
  "whole-repo" is the exact signal an agent needs to distinguish it from a scoped query.
  A new `architecture_query`-style tool should symmetrically say "scoped" or "targeted"
  in its own description so the two tools read as opposites at a glance, not as two
  overlapping ways to do the same thing.
- **`get_info()`'s `instructions` field is a one-paragraph tool index** (`src/mcp.rs`
  lines 304-315): "Use list_checks to discover..., run_checks to inspect..., and
  architecture_assessment for a whole-repo...". This is the single place that
  disambiguates *when* to reach for which tool — new architecture-export/query tools
  must get a clause added here too, or an agent picking between `architecture_assessment`
  and the new query tool has no session-level guidance to go on.
- **Return type is a flat `String`**, not structured JSON, across all three existing
  tools — human/agent-readable prose with `[level] message` lines and `##` section
  headers (`## Recommendations`, `## Dependency graph`). A new *query* tool that's meant
  to return a scoped slice of the architecture model should decide deliberately whether
  to break this convention (return real JSON so an agent can parse fields without
  string-splitting) or stay consistent (flat text). Given the model is inherently
  structured (tree-shaped, package/symbol level), returning JSON is likely the right
  call for a query tool even though it breaks precedent — but that break should be
  explicit and justified in the tool description ("returns JSON", not prose), since an
  agent conditioned on the other two tools' text output will otherwise try to
  string-match a JSON blob.

### CLI JSON export — cross-tool conventions

- **Pretty-print by default, not compact.** kibitzer's own `install.rs` sets the
  precedent: `serde_json::to_string_pretty(&settings)? + "\n"` (`src/install.rs:35`) —
  a trailing newline and indented output even though it's a machine-oriented settings
  file. A `kibitzer architecture export` JSON artifact meant to be committed and diffed
  needs the same: pretty-printed with stable key ordering (kibitzer already depends on
  `serde_json`'s `preserve_order` feature per the comment at `src/install.rs:94-95`) so
  diffs are line-oriented and reviewable, and a trailing newline so the file is
  POSIX-clean. A `--format json|compact` (or similar) flag can offer compact output for
  piping to `jq` in scripts, but pretty should be the default — matching `tree -J`,
  `cargo metadata` (compact by default, arguably a foil) vs. `eslint -f json` conventions
  vary, so kibitzer should pick pretty-by-default deliberately and document it, since
  its own `install.rs` already sets that expectation.
- **Exit codes**: kibitzer's established convention (`src/main.rs`, `src/run.rs`,
  `src/hook.rs`) is `ExitCode::SUCCESS` (0) for "ran successfully, possibly with
  findings reported in output" and `ExitCode::from(1)` for "findings exist" in
  `Check Native` / `Backtest` (i.e., 1 means "there's something for you to look at," not
  "the tool crashed" — that's a `Result::Err` / anyhow bail, which clap/anyhow convert
  to a nonzero exit with a stderr message automatically). For `architecture export`,
  "wrote the file successfully" should be 0 regardless of what the architecture model
  contains (there's no pass/fail concept for an export — unlike `run`/`check`, which
  encode blocking-check status in the exit code). Reserve nonzero exit for genuine
  failure to produce the artifact (no supported language found, i/o error writing the
  file), not for "the model is empty" or "the repo is small."
- **`--format` flag**: no existing kibitzer command has one yet (all current output is
  either fixed-format text or the JSON of `settings.json`), so there's no in-repo
  precedent to preserve, but the ecosystem convention (`docker inspect --format`,
  `kubectl get -o json|yaml`, `gh pr view --json`) is a value-taking flag, not a
  boolean, and JSON is the safe default for a `tree`-shaped export since Mermaid/text
  are visual by nature and belong to a separate verb (see §2).

## 2. User mental models

- **Query vs. dump**: `architecture_assessment` is explicitly a "run everything, get a
  full report" tool (its own description says "whole-repo"), which matches an agent's
  mental model of "give me the full picture" — appropriate for a one-shot advisory scan.
  A new query tool needs a *different* mental model: agents already know `Grep`/`Glob`
  semantics (scoped, fast, returns exactly what was asked, empty/no-match is a valid
  normal result, not an error). Concretely this means: required, non-optional scoping
  parameters (a package path, a symbol name, or a C4-like level — not all-optional
  fields that default to "everything"), and a response that is *only* the matched
  subtree, not the matched subtree plus surrounding context the agent didn't ask for.
  Naming it `architecture_query` (verb-first-adjacent to `run_checks`/`list_checks`) or
  splitting into `list_architecture_symbols` / `get_architecture_node` — mirroring the
  existing `list_checks` vs `run_checks` split (discovery tool vs. action tool) — keeps
  it inside the established naming family rather than introducing a new verb pattern.
- **`export` vs `dump` vs `generate`**: "export" is the right verb for the human-facing
  CLI command because it matches the mental model of "produce a file I can commit, diff,
  feed to another tool" — this is precisely the scope statement's own wording
  ("CLI export command... writing that model to a file"). `dump` connotes a debug-only,
  possibly-unstable format (associated with `--debug-dump`, heap dumps, etc.) and would
  undersell that this is a first-class, stable artifact. `generate` is the right verb for
  *derived, presentational* output — i.e., the diagram — not the structured model itself,
  because "generate" doesn't imply round-trippability or a canonical source of truth the
  way "export" does.
- **Does "C4-like diagram" need its own verb, separate from `export`?** Yes — and the
  requirements doc itself flags this ("interview explicitly rejected claiming full C4
  conformance and that distinction needs to be visible to a user choosing between them").
  Two reasons beyond the conformance-labeling concern: (a) the artifacts have different
  consumption stories — the JSON export is meant to be grepped/jq'd/versioned, the
  diagram is meant to be *viewed* (pasted into a PR description, opened in a Mermaid
  renderer) — conflating them behind one verb with a `--format mermaid` flag would bury
  a fundamentally different use case as a format variant of another command; (b) it
  matches the precedent `architecture_assessment`'s own `include_diagram` flag already
  set: diagram generation is treated as an optional *addition* to a report, not the
  report itself, and CLI users choosing between "give me data" and "give me a picture"
  benefit from that being two distinct verbs (`kibitzer architecture export` vs.
  `kibitzer architecture diagram`) they can list via `kibitzer architecture --help`
  rather than one command with a flag whose default they have to look up.
  `kibitzer architecture diagram` should explicitly note in its own `--help`/description
  that it's a Component/Code-level *visual notation inspired by C4*, not standards-
  conformant C4, given the interview's explicit rejection of that claim — the
  disclaimer belongs in the tool's own text, not just this research doc, so a user
  doesn't infer conformance from the name alone.

## 3. Accessibility

- kibitzer already has a text-fallback precedent worth reusing verbatim: `src/mermaid.rs`
  (`MAX_NODES = 150`) falls back to a plain-text note — not a diagram — once
  `graph.nodes.len() > MAX_NODES`, with actionable guidance ("pass a narrower `scope`
  to render a subgraph instead"). A C4-like diagram command should apply the same rule
  (likely reusing or extending `render_dependency_graph`), and more generally: **a
  text-tree representation of the same subtree should always exist alongside the
  diagram**, not only as an overflow fallback. Rationale: Mermaid diagrams have no
  reliable accessible-text equivalent when rendered (screen readers see either raw
  Mermaid source, which is not prose, or nothing if the renderer draws to canvas/SVG
  without ARIA labeling — this is a known Mermaid limitation, not something kibitzer
  controls). Since kibitzer doesn't control the rendering surface (Claude Artifact,
  GitHub markdown preview, VS Code extension, etc.), the only reliable accessible path
  is to *also* emit a structured/text form the consumer can read regardless of whether
  their renderer supports Mermaid accessibility features. This aligns with `export`
  already being the JSON/text source of truth and `diagram` being a strictly optional,
  visual-only derivative — never the only way to get the information.
  So concretely: `kibitzer architecture diagram` output should include (or default to
  including) a text-tree section above or alongside the Mermaid code fence, the same
  way `architecture_assessment` always emits its text findings first and treats the
  diagram as an appended, optional section (`## Dependency graph`, gated by
  `include_diagram`).
- CLI/MCP/LSP have no GUI of their own, so standard visual-a11y concerns (contrast,
  focus order) don't apply directly to kibitzer; the actual accessibility surface is
  entirely about *not requiring* the visual channel to get the information — text
  output, JSON export, and LSP diagnostics/symbols should each independently be
  sufficient for a screen-reader or non-visual workflow to use the feature.

## 4. Error states / edge cases

- **Querying a nonexistent package/symbol**: should behave like `Grep`/`Glob` on no
  matches — a normal, successful, empty result (not an error, not a nonzero exit /
  MCP error response). This matches the "genuinely easy to use on first try" bar from
  §1: an agent probing for a symbol that may or may not exist shouldn't have to
  special-case an error path just to test existence. Message text should say plainly
  "no match found for <query>" the way `list_checks`/`architecture_assessment` already
  say "no .claude/inspect.json found above this path" rather than silently returning an
  empty JSON array with no explanation — the existing tools always emit an explicit
  sentence for the "found nothing" case rather than empty output, and the new tools
  should match that.
- **`export` on a repo with no supported languages**: `architecture_assessment` and
  `list_checks` both have an established pattern for "nothing to work with" — a plain
  string message, not a panic or silent empty file (`"no .claude/inspect.json found
  above this path"`). `architecture export` should follow suit: detect zero
  parseable/supported files up front and print/return an explicit message ("no
  supported languages found under <path>; nothing to export") rather than writing an
  empty or near-empty JSON file that looks like a bug.
- **LSP "still indexing" state for a huge repo**: `src/lsp.rs`'s current model
  (`check_and_publish`) is per-file, synchronous, and re-runs on every `did_open` /
  `did_change` / `did_save` — there is no existing "whole-repo index" concept in
  `lsp.rs` today, so architecture-export's LSP integration (workspace/document symbols)
  is new surface, not an extension of an existing indexing flow. If workspace-symbol
  lookups require the shared architecture model to be built first and that build is
  slow on a huge repo, the LSP integration should report the standard LSP mechanism for
  this — a `window/workDoneProgress` notification (or, more simply, an empty/partial
  symbol response rather than blocking the editor) — rather than hanging the request.
  Given the existing `lsp.rs` code has no async background-build precedent to reuse,
  this is a case where the *simplest* correct behavior (return what's indexed so far,
  or empty, never block) may be preferable to building new progress-reporting
  machinery, unless the underlying model-build genuinely needs to be a long-running
  background job.
- **Export file already exists**: `install.rs` is kibitzer's one existing "writes a
  file that might already exist" precedent, and its convention is instructive but not
  directly transferable — `merge_hook`'s `settings.json` handling *merges* into the
  existing file rather than overwriting or refusing (because settings.json holds
  unrelated user content that must be preserved), and separately offers `--dry-run` to
  preview the write without committing it (`src/install.rs:14`, `36-39`). An
  architecture-export JSON file has no analogous "unrelated content to merge with" —
  it's a wholesale generated artifact — so the applicable half of that precedent is
  **`--dry-run`, not merge**: `kibitzer architecture export` should support `--dry-run`
  (print what would be written) matching `install`'s flag name and behavior exactly for
  consistency. For the overwrite question itself, kibitzer has no existing "refuse to
  clobber" precedent to point to (install's default *is* to overwrite `settings.json`
  once merged), so the choice is open, but leans toward silent overwrite as the default
  (an export command's whole point is to regenerate a fresh artifact — like
  `cargo metadata > out.json`, `terraform show -json`, or `go doc -json`, none of which
  guard against overwriting), with `--force` reserved only if a future safety concern
  emerges (e.g. warning when overwriting a file that has uncommitted diffs, git-status
  permitting) — not required for v1.

## 5. Jobs-to-be-done

- **Functional job**: get an accurate structural answer fast, without kibitzer
  re-parsing the whole repo per query. This is the direct rationale for persisting a
  model (export) separately from a query surface (MCP tool / LSP) that reads it —
  mirrors the daemon's existing "cache check results across invocations" job
  (`src/main.rs:53`, `src/daemon.rs`) conceptually, though it's a separate mechanism.
- **Emotional job**: confidence the answer is complete, not silently truncated. This is
  exactly the concern `mermaid.rs`'s `MAX_NODES` fallback already addresses for the
  dependency graph (explicit text note instead of a silently cut-off diagram) and
  `ArchitectureAssessmentRequest.include_diagram`'s doc comment already states
  proactively. Any pruning/minimization applied to the exported tree (the requirements
  doc's "Rabbit Holes" section — unexported symbols, single-method interfaces,
  generated code) must be stated explicitly in the artifact itself (a `pruned: true` /
  `omitted_reason` field, or a header line in text output) rather than silently
  dropped, so a consumer never mistakes "we pruned this" for "this doesn't exist."
- **Social job**: a diagram/artifact worth sharing with a human teammate or pasting into
  a PR description — this is the reason `diagram` deserves distinct CLI/MCP surface
  from `export` (§2): a JSON export is not shareable/legible to a human reviewer, but a
  Mermaid diagram pasted into a PR body is. The C4-like disclaimer (visual notation
  *inspired by* C4, not conformant) protects this social job too — a diagram that
  overclaims standards conformance risks a teammate's (or the interview's own,
  explicitly rejected) pushback, undermining the "worth sharing" trust the feature is
  meant to build.

## Key files referenced
- `/home/tstapler/.stapler-squad/repos/github.com/tstapler/kibitzer/src/mcp.rs` — MCP tool naming/description/schema conventions (`ListChecksRequest`, `ArchitectureAssessmentRequest`, `RunChecksRequest`, `get_info()`)
- `/home/tstapler/.stapler-squad/repos/github.com/tstapler/kibitzer/src/install.rs` — file-write conventions: pretty-print + trailing newline (`run_install`, line 35), `--dry-run` flag, existing-file merge behavior (`merge_hook`)
- `/home/tstapler/.stapler-squad/repos/github.com/tstapler/kibitzer/src/mermaid.rs` — `MAX_NODES` text-fallback precedent (`render_dependency_graph`)
- `/home/tstapler/.stapler-squad/repos/github.com/tstapler/kibitzer/src/lsp.rs` — current per-file, synchronous diagnostic model; no existing whole-repo/background-index precedent
- `/home/tstapler/.stapler-squad/repos/github.com/tstapler/kibitzer/src/main.rs` — CLI subcommand/flag naming conventions and exit-code conventions (`ExitCode::SUCCESS` vs `ExitCode::from(1)`)
