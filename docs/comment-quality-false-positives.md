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

## Open

### `[over-commented]`'s ratio appears too aggressive on thin, well-documented wrapper methods

Both the Kubernetes and Cassandra backtest samples independently converged on the same
pattern: a public function/method with a short, delegating body but a thorough
doc comment explaining real contract details the signature doesn't expose (parameter
semantics, caveats, when/why to call it) gets flagged as "restating the code," when
the comment is actually carrying information the one-line body can't. Examples:
`ColumnFamilyStore.java`'s `sstablesRewrite`/`addSSTable`
(apache/cassandra, `src/java/org/apache/cassandra/db`), and — most starkly — the
Kubernetes sample's `[over-commented]` hits had **zero clear true positives found at
n=16** (`pkg/kubelet/util/ioutils/ioutils.go`, `pkg/kubelet/certificate/transport.go`,
`pkg/kubelet/cm/cpumanager/cpu_assignment.go`, `pkg/kubelet/kubelet_node_status.go`,
`pkg/kubelet/cadvisor/cadvisor_linux.go` — every one a well-documented public function
over a short/delegating body).

This is a threshold/design question, not a mechanical bug — the ratio (`2.0x`, 4-line
floor) was already flagged as unvalidated when it shipped (see this file's intro), and
this backtest is the first real data point, pointing toward "too aggressive." Not
changed in this pass; needs a decision on how much to loosen it (or whether to add a
"public API with a short/delegating body" exemption instead of a blanket threshold
change) before the next tuning pass.

## Log
