# comment-quality-corpus

A hand-labeled corpus of real code comments, tagged `how` / `why` / `ambiguous`,
for training, testing, and tuning a future kibitzer checker that flags "how"
comments (bad — restate what the code does) while preserving "why" comments
(good — rationale, invariants, caveats). See `docs/comment-quality.md` for the
existing `comment-quality-<lang>` checker this corpus is meant to extend or
evaluate against.

## Provenance

210 comments pulled from three real open-source repos, each an isolated
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
`research/fetch_rs.py`) — not guessed from the local filename alone.

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
| `round` | Which research round produced the label: `3` or `4`. |
| `_round3_embedding_similarity` | Present only on some round-3 rows: a leftover score from an abandoned embedding experiment. Historical only, not authoritative — do not use for anything beyond curiosity about that experiment. |

Example row:

```json
{"id": "kubelet-001", "repo": "kubernetes/kubernetes", "path": "pkg/kubelet/stats/cadvisor_stats_provider.go", "language": "go", "comment": "newCadvisorStatsProvider returns a containerStatsProvider that provides container stats from cAdvisor.", "signature": "new cadvisor stats provider", "label": "how", "round": 3}
```

## Current size and label balance

210 examples total (60 from round 3, 150 from round 4), 70 per language.

| | how | why | ambiguous | total |
|---|---|---|---|---|
| go | 44 | 26 | 0 | 70 |
| java | 30 | 39 | 1 | 70 |
| rust | 18 | 52 | 0 | 70 |
| **all** | **92** | **117** | **1** | **210** |

### Caveat: the rust ("servo") `why` subset is skewed toward spec-URL citations

34 of the 52 rust `why` labels (about two-thirds) are comments that are
*nothing but* a bare spec-URL citation, e.g.:

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
other way round.

## Known limitations

- **Small.** ~210 examples is not enough to trust precise accuracy
  percentages — treat any number derived from this set as a rough signal, not
  a benchmark result.
- **Narrow.** Only 3 repos and 3 languages; all three are large, mature
  systems-level C-family-descended codebases with strong comment conventions,
  which may not generalize to smaller or more casually-commented projects.
- **No inter-rater agreement check yet.** Labels come from two contributors'
  blind judgment calls with no measured agreement rate — some fraction of the
  `how`/`why`/`ambiguous` boundary is inherently subjective, and that
  subjectivity hasn't been quantified here.
