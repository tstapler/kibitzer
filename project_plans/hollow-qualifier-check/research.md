# Research: hollow-qualifier-check

## Codebase orientation

### Existing prose checkers (reference implementations)

All prose checkers share the same shape:

1. A struct implementing `Checker` in `src/checkers/<name>.rs`
2. `inventory::submit! { crate::checker::CheckerFactory(|| vec![Box::new(MyChecker)]) }`
   — the registration macro that makes the checker callable via `kibitzer check native <name>`
3. A `check` method that calls `crate::markdown_text::check_paragraphs(ctx.source, |line, text| …)`
4. Unit tests in the same file, using a `check_source(body: &str) -> Vec<Finding>` helper

Key files:
- `src/checkers/ai_vocabulary_density.rs` — closest analogue: configurable word list,
  opt-in, paragraph-scoped, `configure()` override pattern
- `src/checkers/repetitive_sentences.rs` — simplest complete example of the checker shape
- `src/checkers/formulaic_ai_openers.rs` — first-sentence-of-paragraph scoping (relevant)
- `src/markdown_text.rs` — shared `for_each_paragraph`, `check_paragraphs`, `split_sentences`
- `src/config.rs:784` — `prose_checks()` function lists opt-in vs. default; new opt-in
  checkers are registered via `inventory::submit!` only, not added to `prose_checks()`

### Heading extraction gap

`markdown_text::for_each_paragraph` only walks `Tag::Paragraph` events. Headings arrive
as `Tag::Heading` events. To check headings, the new checker needs to either:

**Option A:** Add a `for_each_heading` helper to `markdown_text.rs` (consistent with the
shared-helper pattern the module doc comment describes — see its comment about
`check_paragraphs` extracting a 7th copy of the same pattern).

**Option B:** Inline the `pulldown_cmark` walk inside the new checker (acceptable but
less reusable).

Option A is preferred: a second heading-aware prose check is likely (headings are a common
AI writing anti-pattern site), and the shared helper pays for itself immediately.

### Configurable option pattern

`ai_vocabulary_density.rs` shows the standard pattern for `Check.options` override:
- Define a `#[derive(serde::Deserialize)] #[serde(deny_unknown_fields)] struct Options { … }`
- Implement `Checker::configure(&self, options: &serde_json::Value) -> Result<Option<Box<dyn Checker>>>`
- Validate at config-load time (unknown fields → error)

For this check, the configurable field is `adverbs: Option<Vec<String>>` (extra adverbs
to add to the default list, or a full replacement — TBD based on whether full replacement
or additive is more useful).

### Backtesting requirement

Per `CLAUDE.md` and `docs/backtesting.md`:
- Run `kibitzer check backtest hollow-qualifier-phrasing` against Claude Code session
  transcripts to see FP rate on real AI-generated markdown
- Run against the standard corpus (`docs/backtest-repos.md`): kubernetes/kubernetes,
  apache/cassandra, servo/servo, BurntSushi/ripgrep, denoland/deno, microsoft/vscode,
  plus stapler-squad
- Use `scripts/backtest-triage.py` to track true/false-positive triage across runs
- A checker not run against this corpus "isn't done"

The backtest is required before the check can be graduated to `default_checks()`.
Given only 1 observed occurrence, the check will start opt-in; backtest data may either
confirm it's safe for default or validate the opt-in decision.

### Relevant adverb list

Sourced from the description and expanded by analogy with similar style-guide lists
(Pinker's *The Sense of Style* on hollow qualifiers; Google Developer Documentation Style
Guide on empty modifiers):

Core list (high confidence these are problematic in heading-terminal position):
- `generically`
- `fundamentally`
- `essentially`
- `basically`
- `broadly`
- `generally`
- `abstractly`
- `conceptually`
- `theoretically`
- `ultimately`
- `nominally`

Candidate additions (lower confidence — may appear legitimately in technical docs):
- `philosophically` — technical philosophy sections may use legitimately
- `architecturally` — common in real system-design docs; risk of FP
- `technically` — very common in legitimate technical writing; probably too noisy

Recommendation: start with the core 11 and tune after backtesting.

### False-positive surface

Known FP risks for this specific pattern:
1. **Intentional category labels in reference docs** — "The method, generically: `foo(x)`"
   style, where the comma+adverb is used as a deliberate annotation style.
   Mitigation: advisory (never blocking); opt-in only.
2. **Adverb mid-sentence in compound headings** — "Broadly speaking, this approach …"
   does NOT match because the adverb is at the start, not after a comma. The
   `<noun>, <adverb>` at-end pattern is specific enough to avoid this.
3. **Technical adverbs that are domain-specific** — "abstractly" in a CS theory doc is
   legitimate. Mitigation: configurable list; users can remove entries.

### Setext headings

Setext (`===`/`---` underline style) headings are rare in modern markdown but present in
some older docs. pulldown-cmark parses them identically to ATX headings as
`Tag::Heading` events — the implementation covers both styles for free once heading
events are walked.

## Summary of findings

The check is straightforward to implement following the existing prose checker pattern.
The main new code is:
1. `for_each_heading` helper in `markdown_text.rs` (reusable)
2. `src/checkers/hollow_qualifier_phrasing.rs` (new file, ~120 lines with tests)
3. `docs/prose-checks.md` update (opt-in table entry)

No architecture model, no tree-sitter grammar, no config file needed. The check is
below the 2-occurrence threshold for immediate implementation but is well-scoped for
a future sprint once a second occurrence is confirmed or corpus mining finds additional
evidence.
