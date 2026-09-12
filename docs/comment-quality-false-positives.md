# `comment-quality-<lang>` — known false positives

Tracks confirmed false-positive firings of the `comment-quality-<lang>` checkers
(`src/comment_quality.rs`) — `[verbose-comment]`, `[commented-out-code]`, and
`[over-commented]`. Check new occurrences against this list, and against the entries
below already fixed, before re-investigating a firing from scratch. See
`docs/reporting-false-positives.md` for how to file a new one.

The `[over-commented]` proportionality check (comment lines >= 4 and >= 2.0x the
declaration's body code-line count) has no external benchmark behind its threshold —
unlike `[commented-out-code]`, which mirrors SonarQube's long-shipped S125 rule, a
literature/tooling survey found no other tool or published research validating a
specific comment-to-code ratio in either direction (every ratio-based tool found
flags too *few* comments, never too many). Treat `2.0x`/`4 lines` as a starting
hypothesis to tune against real firings, not a validated constant — see the "Open"
section below for the first such real-world signal.

## Fixed

### Semicolon-terminated bullet-list doc comment misread as commented-out code

- **Symptom**: a doc comment written as a semicolon-separated bullet list (a common
  technical-writing style — "- validates input;", "- normalizes casing;") fired
  `[commented-out-code]` on every line, even though none of it was code.
- **Mechanism**: `looks_like_code`'s `text.ends_with(';')` branch treated any
  semicolon-terminated clause as code-shaped, with no check that it also carried any
  code-only punctuation — the same documented gap as SonarQube's S125
  (community reports of prose semicolons being misflagged).
- **Fixed by**: requiring the semicolon-ending branch to also see a code-only
  punctuation character (`(){}[]=<>+*/&|!` — deliberately excluding `.`/`,`, both
  common in ordinary prose). Regression-guarded by
  `comment_quality::tests::does_not_flag_a_semicolon_terminated_bullet_list`.
- **Known trade-off**: a punctuation-free single-word statement like a bare `return;`
  or `break;` is no longer flagged as commented-out code. Accepted per this checker's
  stated bias toward missing real dead code over flagging prose.

### 2026-09-06 — Kubernetes/Cassandra/Servo backtest — six findings, all fixed

A real backtest (three cloned OSS repos: `pkg/kubelet` from kubernetes/kubernetes,
`src/java/org/apache/cassandra/db` from apache/cassandra, `components/script` from
servo/servo — see kibitzer's `CLAUDE.md`) surfaced six concrete false positives, fixed
in the same session:

- **License-header/parenthetical-prose false positive** — `[commented-out-code]`
  fired on 703/755 hits (93%) in the Kubernetes sample, every one an Apache-2.0
  license header line ("Licensed under the Apache License, Version 2.0 (the
  \"License\");"). Cassandra's `ClusteringComparator.java`/`Trie.java` hit the same
  mechanism via ordinary parenthetical asides ending a sentence in `;`, and Servo's
  `basecommand.rs` hit it via a spec-quoting `>` blockquote marker. **Fixed** by
  narrowing `looks_like_code`'s code-only punctuation set from `(){}[]=<>+*/&|!` to
  `{}[]=+*/&|!` — `(`/`)`/`<`/`>` all turned out to be common in ordinary prose, not
  unambiguous code signals. A real call/assignment expression is still caught by
  `is_call_expression`/`is_assignment` regardless.
- **`is_assignment`'s unchecked RHS** — a narrative computation like
  `FQDN=15 + 1(dot) + 55 = 71 chars` or `OOMScoreAdj = 1000 - (...) = 869` has a
  valid-looking `lhs`, so it matched as a real assignment statement. **Fixed** by
  rejecting when `rhs` itself contains another bare `=` — no single real assignment
  statement re-derives a value through a second `=`.
- **Short banned phrase with no word boundary** — `"this pr"` matched inside ordinary
  words with no boundary check (`"this **pr**events"`, `"this **pr**operly"`, `"this
  **pr**ocess"`), confirmed independently in both the Kubernetes and Cassandra
  samples. **Fixed** by `contains_whole_phrase`, a whole-word/-phrase match requiring
  non-alphanumeric characters on both sides of the match.
- **Fenced doc-comment code examples** — Rust doc comments routinely embed real,
  intentional usage examples in ` ``` `-fenced blocks (`components/script/dom/bindings/
  like.rs`, `function.rs`), which `looks_like_code` correctly (for its actual purpose)
  recognizes as code-shaped. **Fixed** by tracking fence state across a declaration's
  comment nodes in `check_commented_out_code` and skipping lines inside a fence.
- **Adjacent unrelated commented-out code folded into the next declaration's ratio** —
  `leading_comment_rows` walked backward through *any* contiguous comment siblings, so
  a run of leftover commented-out function stubs immediately before a real, unrelated
  function (`components/script/dom/testing/testbinding.rs`) got misattributed as that
  function's own leading doc comment, inflating its `[over-commented]` ratio. **Fixed**
  by stopping the backward walk (without including the block) at a comment whose own
  text looks like commented-out code rather than documentation.
- **Rust `# Safety` doc sections** — the idiomatic convention for justifying an
  `unsafe fn`'s invariants naturally produces a short function with a long
  justification (`components/script/layout_dom/servo_layout_node.rs`), which the ratio
  read as restatement when it's the opposite. **Fixed** by exempting a leading comment
  block containing a `# Safety` heading from the ratio check entirely.

### 2026-09-06 — `[over-commented]` too aggressive on thin, well-documented wrapper methods

Both the Kubernetes and Cassandra backtest samples independently converged on the same
pattern: a public function/method with a short, delegating body but a thorough
doc comment explaining real contract details the signature doesn't expose (parameter
semantics, caveats, when/why to call it) got flagged as "restating the code," when
the comment was actually carrying information the one-line body can't. Examples read
directly from source (not just the summary counts): `ioutils.go`'s `LimitWriter`
(`return &LimitedWriter{w, n}`, kubernetes/kubernetes), `ColumnFamilyStore.java`'s
`sstablesRewrite` (`return CompactionManager.instance.performSSTableRewrite(...)`,
apache/cassandra), and `servo_layout_node.rs`'s `dangerous_first_child`
(`self.node.first_child_ref().map(Into::into)`, servo/servo) — every one a body that's
a single statement delegating to something else, whose real behavior lives in the
callee/constructed type or an invariant the signature can't express, not in the one
line visible here. The Kubernetes sample's `[over-commented]` hits had zero clear true
positives at n=16.

**Fixed** two ways, both evidence-driven from the examples above:
- **Delegating-single-statement-body exemption** (`is_delegating_single_statement_body`):
  a body that's exactly one statement (AST-based — named-child count, after unwrapping
  Go's `block`/`statement_list` and Kotlin's `function_body`/`block` wrapper nesting,
  not line-count-based, since a line-count check would only catch a body crammed onto
  one source line and miss the far more common "brace on its own line" style every
  real example above is written in) that itself looks like a delegation (a call, a
  method chain, or a struct/object construction) is exempt from the ratio entirely,
  regardless of comment length. A real precondition-plus-delegation body (Cassandra's
  `addSSTable` — see the "stale comment" entry above) is *not* exempt: it's two
  statements, not one, and correctly keeps failing the gate.
- **Parameter-count-scaled ratio**: `COMMENT_TO_CODE_RATIO` grows by
  `PARAM_COUNT_RATIO_BONUS` (0.25, itself unvalidated the same way the base ratio is —
  no backtest example isolated this specific increment) per parameter beyond
  `PARAM_COUNT_RATIO_BASELINE` (2), for a multi-statement body with several
  parameters whose semantics the type system can't carry.

Both regression-guarded in `comment_quality.rs`'s test module, built directly from the
real examples above rather than only synthetic cases. A genuinely-restating synthetic
case (`Add(a, b) { return a + b }` — a self-contained computation, correctly *not* a
delegation) still fires unchanged.

### 2026-09-09 — stelekit — `/`-separated prose list with a trailing `;` misread as commented-out code

- **Symptom**: a prose comment ending a clause in `;` with a bare `/` list separator
  (`tags/auto_labels/ocr_text`, from `kmp/src/jvmTest/kotlin/.../QueryPlanAuditTest.kt:45`
  in `tstapler/stelekit`) fired `[commented-out-code]`, even though nothing on the line
  was code.
- **Mechanism**: `looks_like_code`'s semicolon-ending branch (`src/comment_quality.rs:403`)
  still included a bare `/` in its code-only punctuation set after the 2026-09-06 backtest
  narrowed it from `(){}[]=<>+*/&|!` to `{}[]=+*/&|!` — `/` alone, with no other code-only
  punctuation nearby, reads as a list separator far more often than as division.
- **Fixed by**: dropping `/` from the set, leaving `{}[]=+*&|!`. Regression-guarded by
  `slash_separated_prose_list_ending_in_semicolon_is_not_flagged_as_commented_out_code`
  (built from the real stelekit example) and
  `division_assignment_expression_is_still_flagged_as_commented_out_code` (confirms a
  genuine `total = width / height;` is still caught via `is_assignment`, independent of
  the punctuation set).
- **Backtested**: pre/post-fix release binaries diffed against `servo/servo`,
  `BurntSushi/ripgrep`, `denoland/deno`, and `kubernetes/kubernetes` — 51 findings dropped
  across the four repos, zero new findings, and every dropped finding manually confirmed
  to be prose (URLs, paths, "and/or" lists) ending in `;` with a bare `/`. A pre-existing
  true positive (`main.go:56`'s real commented-out `delete(...)` call) is still flagged in
  both binaries.

### 2026-09-09 — stelekit — brace-closing `} // ConstructName(args)` annotation comments misread as commented-out code

- **Symptom**: a closing brace annotated with which construct it closes
  (`} // CompositionLocalProvider(LocalWindowSizeClass)`, a common idiom in deeply-nested
  Compose UI code — `kmp/src/commonMain/kotlin/.../App.kt` in `tstapler/stelekit`) fired
  `[commented-out-code]`, even though the named call is live code several hundred lines
  above, not something commented out.
- **Mechanism**: `is_call_expression` (`src/comment_quality.rs:409`) matches purely on
  text shape (identifier, `(`, matching `)`, optional trailing `;`/`,`) with no awareness
  of the comment's position relative to code on the same physical line — a brace-closing
  annotation naming its construct in call syntax is syntactically indistinguishable from a
  real commented-out call.
- **Fixed by**: `is_brace_closing_annotation` (`src/comment_quality.rs:345`), which checks
  whether the comment's own physical row has only a bare `}` (optionally with trailing
  `;`/`,`) before it, and skips commented-out-code detection on that comment's first line
  when true. Regression-guarded by
  `brace_closing_annotation_comment_is_not_flagged_as_commented_out_code` (built from the
  real stelekit example) and
  `trailing_call_comment_after_real_code_is_still_flagged_as_commented_out_code` (confirms
  a trailing `// doSomething(x, y)` after real, non-brace-closing code is still caught).
- **Backtested**: this repo's own `src/*.rs` (`comment-quality-rust`) showed no new noise.
  The brace-closing idiom doesn't occur in the locally-cloned Go/Rust corpus repos, so the
  Kotlin regression test above is this fix's primary evidence.

### 2026-09-12 — kubernetes/kubernetes + stapler-squad corpus backtest — `"used by"` banned-phrase matched ordinary Godoc consumer documentation

- **Symptom**: "PodSubnet is the subnet used by pods." (kubernetes/kubernetes) and "defaultSessionRetentionDays is
  used by RetentionDaysOrDefault..." (stapler-squad) both flagged — ordinary Godoc sentences describing a field's
  consumer, with no reference to any specific caller, PR, or fix. Across a 40-finding corpus sample this single
  mechanism accounted for roughly a third of all `comment-quality-go` false positives.
- **Mechanism**: `contains_whole_phrase` (`src/comment_quality.rs`) matched the bare substring "used by" anywhere
  in a comment — `BANNED_PHRASES`' reason ("this belongs in the commit message, not a comment that outlives its
  caller") targets a comment naming an impermanent change/caller, but the bare-substring match also caught the
  ubiquitous, encouraged Godoc idiom "X is used by Y."
- **Fixed by**: `contains_issue_number_token` — "used by" now only fires when it co-occurs with a `#\d+`-shaped
  issue/PR reference in the same comment, preserving the actual intended catch ("used by the fix in issue #456")
  while eliminating the Godoc-consumer-doc false positive. Verified directly: both cited examples produce zero
  "used by" findings, a backtest across `session/ent/` found 4 more real instances of the same false positive
  (all eliminated), and a synthetic "used by the fix in issue #456" case still flags.

### 2026-09-12 — kubernetes/kubernetes + stapler-squad corpus backtest — unfenced doc-comment code examples read as commented-out code

- **Symptom**: an indented callback-signature example (`func(ctx context.Context, data []byte)`, kubernetes/kubernetes
  `vendor/github.com/onsi/ginkgo/v2/core_dsl.go`) and an ent-generated `OnConflict` usage example ending
  `Exec(ctx)` (stapler-squad `session/ent/classificationanalytics_create.go`) both flagged — real, intentional
  documentation examples, not dead code.
- **Mechanism**: the previously-fixed "fenced doc-comment code examples" exclusion only recognized triple-backtick
  fences. Go/Godoc's much more common idiom — an indented, unfenced `//` code example with no backticks — wasn't
  covered.
- **Fixed by**: `is_indented_code_example_line` + `block_comment_baseline_indent`, recognizing Godoc's real
  convention (a line indented beyond its paragraph, via a tab/2+ spaces after `//`/`///`/`//!`, or beyond a
  computed per-comment baseline for `/* */` continuation lines — tolerating uniformly source-indented block
  comments). Verified directly: `session/ent/` alone dropped from 462 to 0 `[commented-out-code]` findings; 6
  sampled eliminations by hand were all genuine ent-generated Godoc examples, no dead code lost.

### 2026-09-12 — kubernetes/kubernetes corpus backtest — `is_call_expression` matched a math-notation formula, not a function call

- **Symptom**: `LendableCL(i) = round( NominalCL(i) * lendablePercent(i)/100.0 )` (kubernetes/kubernetes
  `staging/src/k8s.io/client-go/applyconfigurations/flowcontrol/v1beta2/exemptprioritylevelconfiguration.go:50`)
  flagged — prose describing a formula, not a leftover call statement.
- **Mechanism**: `is_call_expression` only checked that alphanumeric text preceded the *first* `(` and that the
  text ended with `)` — never validating balanced parens or that nothing followed the closing paren, so a
  multi-term formula with several `name(args)` sub-expressions read as one call.
- **Fixed by**: rewrote `is_call_expression` to require balanced parens via `matching_close_paren`, rejecting
  anything following the call's own matching close paren. Nested real calls (`Foo(Bar(1), Baz(2))`) still match.
  Verified directly: the cited line no longer flags.

### 2026-09-12 — kubernetes/kubernetes corpus backtest — quoted proto-IDL field declaration misread as commented-out Go

- **Symptom**: `// optional bool alpha_enum = 1060;` (kubernetes/kubernetes
  `vendor/github.com/container-storage-interface/spec/lib/go/csi/csi.pb.go:7644`) flagged — quoted Protocol
  Buffers `.proto` IDL syntax, not commented-out Go.
- **Mechanism**: matched `looks_like_code`'s `text.ends_with(';') && text.chars().any(|c| "{}[]=+*&|!".contains(c))`
  branch — the proto declaration ends in `;` and contains `=`, which the code-only punctuation heuristic treated
  as unambiguous code.
- **Fixed by**: `is_proto_field_declaration`, narrowly scoped to the exact shape `<optional|repeated|required>
  <type> <field> = <digits>;` — cheap and safe since a real assignment (e.g. `optionalFlag =
  computeDefault();`) doesn't match this shape and stays flagged. Verified directly: the cited line no longer
  flags, and a lookalike real assignment was confirmed still caught.

### 2026-09-12 — stapler-squad corpus backtest — `is_assignment` still under-constrained beyond the documented second-`=` guard

- **Symptom**: `// cmd.WaitDelay = 2 * time.Second. This analyzer enforces that rule` (stapler-squad
  `tools/lint/norawexec/analyzer.go:13`, a code fragment quoted mid-sentence followed by unrelated prose) and
  `// true = no matching stapler-squad session` (stapler-squad `gen/proto/go/session/v1/insights.pb.go:41`, doc
  shorthand explaining a boolean field) both flagged as assignments.
- **Mechanism**: the already-fixed "unchecked RHS" gap only excluded a RHS containing a second bare `=`. Neither
  case has one: `is_assignment` never checked that the RHS was the whole remaining comment text (missing a
  sentence-boundary check), and accepted a bare boolean literal (`true`) as a valid "LHS identifier" since it
  only checked character class, not that it's an actual variable/field reference.
- **Fixed by**: (a) reject a RHS containing `". "` (a sentence boundary — a real RHS never has a space after a
  dot); (b) reject `true`/`false`/`nil` as LHS identifiers, since a boolean/nil literal can never legitimately be
  assigned to in real Go code. Verified directly: both cited lines no longer flag.

## Log

### 2026-09-11 — stapler-squad — mathematical interval notation `[a, b)` in prose misread as commented-out code

- **Repo**: `tstapler/stapler-squad`, file
  `testutil/tmuxreap/tmuxreap.go` (line number shifted repeatedly across a session's
  edits — reported at `:216`, then `:222`, then `:232` as unrelated lines were added
  above it; content unchanged throughout).
- **What changed**: nothing at this line — pre-existing doc comment, first surfaced
  by an unscoped re-check after unrelated edits elsewhere in the same function. The
  line reads (after stripping `//`): `"test-isolated-1234"). PID range on this
  system is [2, 4194304);` — part of `extractTestSocketPID`'s doc comment
  explaining the valid PID range using half-open interval notation.
- **Why it's a false positive**: `[2, 4194304)` is standard mathematical notation for
  a half-open range (inclusive lower bound, exclusive upper bound), not an array/slice
  literal or any other code construct. The whole line is prose describing a numeric
  range and citing a quoted example socket name.
- **Mechanism**: `comment_quality.rs::looks_like_code` (line ~403, unchanged by the
  fixes above): `text.ends_with(';') && text.chars().any(|c| "{}[]=+*&|!".contains(c))`.
  The line ends in `;` (a normal sentence-terminating semicolon, consistent with this
  file's own comment style elsewhere) and contains `[`/`]` from the interval notation,
  which is in the code-only punctuation set — so the combined check fires even though
  neither the brackets nor the semicolon are code here. This is the same class of gap
  already fixed above for `(`/`)`/`<`/`>` and `/` (Apache license headers, blockquote
  markers, slash-separated prose lists): `[`/`]` is not an unambiguous code signal
  either when used for mathematical interval notation in prose. Not independently
  verified against a broader corpus — flagging as a hypothesis with one concrete
  real-world instance, per this file's stated approach to unvalidated constants.

