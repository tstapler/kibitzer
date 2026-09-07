# comment-quality-corpus

A hand-labeled corpus of real code comments, tagged `how` / `why` / `ambiguous`,
for training, testing, and tuning a future kibitzer checker that flags "how"
comments (bad — restate what the code does) while preserving "why" comments
(good — rationale, invariants, caveats). See `docs/comment-quality.md` for the
existing `comment-quality-<lang>` checker this corpus is meant to extend or
evaluate against.

## Provenance

330 comments pulled from three real open-source repos, each an isolated
excerpt (one comment + its enclosing function/method's identifier context —
not the surrounding file), attributed via each entry's `repo`/`path` fields.
This is the same practice most linters use to build test/eval fixtures from
real-world code.

| Repo | License |
|---|---|
| [kubernetes/kubernetes](https://github.com/kubernetes/kubernetes) (kubelet package) | Apache-2.0 |
| [apache/cassandra](https://github.com/apache/cassandra) | Apache-2.0 |
| [servo/servo](https://github.com/servo/servo) | MPL-2.0 |

Every entry's `path` was confirmed by matching a distinctive line from the
downloaded source against the upstream repo via `gh api search/code` (or, for
the round-4 `servo2_*` files, against the exact paths used to fetch them in
`research/fetch_rs.py`) — not guessed from the local filename alone. The
round-5 rows (`round: 5`) were confirmed the same way in spirit but via a
different mechanism: each file was fetched directly at the path returned by
`gh api repos/<org>/<repo>/contents/<dir>` (the GitHub Contents API listing
for that directory on the repo's default branch), so the path is the one the
API itself reports exists — not a guess later verified by search. The
round-6 rows (`round: 6`) were confirmed the same way as round 5: each file
was fetched via `gh api repos/<org>/<repo>/contents/<path>` at a path chosen
from the same three repos' current directory structure (verified via
`gh api search/code` when a guessed path 404'd, e.g. Cassandra's `Memtable`
class moved to `db/memtable/AbstractMemtable.java` and Servo's `element.rs`/
`document.rs` moved under `dom/element/` and `dom/document/` since round 4/5).

## Format

JSON Lines, one object per example, sorted by `id`. Fields:

| Field | Description |
|---|---|
| `id` | Stable id, unique across the corpus: `<repo-short>-<seq>` (`kubelet-`, `cassandra-`, `servo-`). |
| `repo` | Full `org/repo` of the source. |
| `path` | Upstream file path within `repo`, confirmed as described above. |
| `language` | `go` / `java` / `rust`. |
| `comment` | Raw comment text, verbatim from source. |
| `signature` | The function/method identifier context for the comment. |
| `label` | `how` / `why` / `ambiguous`, from blind hand-labeling (see below). Copied verbatim from the source research data — never re-derived. |
| `round` | Which research round produced the label: `3`, `4`, `5`, or `6`. |
| `_round3_embedding_similarity` | Present only on some round-3 rows: a leftover score from an abandoned embedding experiment. Historical only, not authoritative — do not use for anything beyond curiosity about that experiment. |

Example row:

```json
{"id": "kubelet-001", "repo": "kubernetes/kubernetes", "path": "pkg/kubelet/stats/cadvisor_stats_provider.go", "language": "go", "comment": "newCadvisorStatsProvider returns a containerStatsProvider that provides container stats from cAdvisor.", "signature": "new cadvisor stats provider", "label": "how", "round": 3}
```

## Current size and label balance

330 examples total (60 from round 3, 150 from round 4, 60 from round 5, 60
from round 6), 110 per language.

| | how | why | ambiguous | total |
|---|---|---|---|---|
| go | 61 | 49 | 0 | 110 |
| java | 46 | 62 | 2 | 110 |
| rust | 36 | 72 | 2 | 110 |
| **all** | **143** | **183** | **4** | **330** |

The round-5 fresh-60 batch (`round: 5`, 20 per language, gathered from files
not previously sampled in the same three repos) was deliberately curated for
more balance than rounds 3-4's rust sample: 9 `why` / 9 `how` / 2 `ambiguous`,
versus the heavily `why`-skewed round 3/4 rust rows. The round-6 fresh-60
batch (`round: 6`, 20 per language, again from files not previously sampled)
came out 11 `how` / 9 `why` for go, 10 `how` / 9 `why` / 1 `ambiguous` for
java, and 9 `how` / 11 `why` for rust — close to balanced for all three
languages, including rust, without deliberately excluding bare-URL rust
comments (round 6 happened to sample few of them; see the caveat below,
which still describes the corpus as a whole).

### Caveat: the rust ("servo") `why` subset is skewed toward spec-URL citations

41 of the 61 rust `why` labels from rounds 3-5 (about two-thirds of that
subset) are comments that are *nothing but* a bare spec-URL citation, e.g.:

```
<https://html.spec.whatwg.org/multipage/#category-listed>
```

These get labeled `why` (they point at a spec rationale rather than restating
the code) but they're trivially separable by a `contains a URL` feature —
they don't exercise the subtler discourse-level signals (rationale phrasing,
contrastive clauses, invariant/caveat language) that a real classifier needs
to get right. **Any future accuracy evaluation should report Go+Java and Rust
as separate numbers, not blend them into one aggregate** — a blended number
that looks good may just mean the classifier learned "has a URL" on the rust
third of the set.

## How to extend it

The extraction pipeline lives in the research scratchpad this corpus was
consolidated from (not checked into this repo): `extract.py` does a heuristic
scan of downloaded upstream source for comment-block-then-signature patterns
in Go/Java/Rust, `curate.py` pulls a chosen subset of `file:line` keys out of
those candidates into a sample file, and each sample was then hand-labeled
**blind** — reading only the comment and its signature, before running any
classifier — to keep it valid as a test set. Reuse that same scan-then-curate
approach to pull more candidates from these or additional repos, and keep the
blind-labeling discipline: label first, run the classifier second, never the
other way round. Rounds 5 and 6 followed this same discipline; their
scratchpads (`build_fresh60.py`, `gen_corpus_fields.py`, `fresh60_labels.py`,
`discourse_classifier_v3.py`/`v4.py`, plus round 6's `eval_step1.py`/
`eval_step2.py`/`append_corpus.py`) are not checked into this repo either.

## Known limitations

- **Small.** ~330 examples is not enough to trust precise accuracy
  percentages — treat any number derived from this set as a rough signal, not
  a benchmark result.
- **Narrow.** Only 3 repos and 3 languages; all three are large, mature
  systems-level C-family-descended codebases with strong comment conventions,
  which may not generalize to smaller or more casually-commented projects.
- **No inter-rater agreement check yet.** Labels come from two contributors'
  blind judgment calls with no measured agreement rate — some fraction of the
  `how`/`why`/`ambiguous` boundary is inherently subjective, and that
  subjectivity hasn't been quantified here.
