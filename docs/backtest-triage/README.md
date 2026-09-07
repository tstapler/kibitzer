# Backtest finding triage

Tracks a true/false-positive verdict per finding on the real-world backtest
corpus (`docs/backtest-repos.md`), across repeated runs — so re-running a
checker after a change shows only what's actually new, instead of re-reading
the same thousands of already-judged findings every time.

Each repo gets its own directory, `docs/backtest-triage/<repo-slug>/`, holding
one `<rule-id>.jsonl` shard per rule — this keeps any single file small even
as a repo's triage history grows, and keeps a git diff scoped to the rule
that changed. One line per triaged finding, keyed by `(file, line, rule)` —
`file` is relative to the repo root so the store stays valid regardless of
where the repo happens to be cloned locally. Each record:

```json
{
  "file": "cmd/kubeadm/app/phases/upgrade/postupgrade.go",
  "line": 99,
  "rule": "flag-argument",
  "verdict": "true_positive",
  "note": "why",
  "commit": "0ba07e0866bb1e3d67464b2d3e01f6bde6fe9963",
  "permalink": "https://github.com/kubernetes/kubernetes/blob/0ba07e0866bb1e3d67464b2d3e01f6bde6fe9963/cmd/kubeadm/app/phases/upgrade/postupgrade.go#L99"
}
```

`commit`/`permalink` are stamped automatically by `mark --repo-dir DIR` from
that clone's current `HEAD` and `git remote get-url origin` — pinning the
verdict to the exact code it was made against, one click away, without
anyone re-deriving the SHA or URL by hand. `end_line` (optional, via
`mark --end-line N`) extends the permalink to a range (`#L99-L110`) for a
finding that isn't a single line.

`verdict` is one of `true_positive`, `false_positive`, `needs_discussion` (a
real hit whose value as a lint is genuinely debatable — e.g. an idiomatic
Rust builder setter that mechanically matches a rule but reads as a defensible
style choice to some).

## Workflow

```sh
# Re-run a checker and diff against what's already triaged:
scripts/backtest-triage.py run \
  --repo-slug kubernetes-kubernetes \
  --repo-dir ~/code/github.com/kubernetes/kubernetes \
  --checker syntax-rules --glob '*.go' --rule flag-argument

# Record a verdict for one of the findings it lists as NEW — pass --repo-dir
# so the commit + GitHub permalink get stamped in automatically:
scripts/backtest-triage.py mark \
  --repo-slug kubernetes-kubernetes --repo-dir ~/code/github.com/kubernetes/kubernetes \
  --file cmd/kubeadm/app/phases/upgrade/postupgrade.go --line 99 --rule flag-argument \
  --verdict true_positive --note "why"

# List what's been triaged so far (optionally filtered):
scripts/backtest-triage.py list --repo-slug kubernetes-kubernetes --verdict false_positive
```

`run` exits 1 if there are any NEW (untriaged) findings — usable as a
"nothing new to review" gate once a repo's backlog is fully triaged, though
with 12k+ findings across the current corpus that's a long way off; for now
just treat NEW as the reading list for the next triage session. `--show-known`
also prints the already-triaged findings inline (useful for reviewing a
`needs_discussion` bucket together, say).

`--repo-slug` is `<owner>-<repo>` (matching `docs/backtest-repos.md`'s table),
lowercased, e.g. `kubernetes-kubernetes`, `apache-cassandra`,
`burntsushi-ripgrep`, `denoland-deno`, `microsoft-vscode`,
`tstapler-stapler-squad`.

## Caveats

- **Each record's `commit` is the ground truth for what was actually
  reviewed** — its `permalink` always resolves to the exact code a verdict
  was made against, even if the local clone moves on afterward. But a `run`
  diffs against the *current* file at its *current* line, keyed only by
  `(file, line, rule)` — if the repo advances and a triaged line shifts or
  its content changes, `run` has no way to know that and will still report it
  as already-triaged. `tstapler-stapler-squad` is the one corpus repo that's
  actually live (not a frozen `--depth 1` clone like the rest) — before
  trusting its triage results, check whether the triaged files changed
  between the record's `commit` and current `HEAD`
  (`git diff --name-only <old-commit> HEAD -- <file>`); if they did, re-open
  the permalink and re-verify rather than trusting the stale verdict.
- **A re-clone at a new commit invalidates the affected entries.** The other
  six repos are shallow-cloned once by `scripts/clone-backtest-repos.sh` and
  never `git pull`ed specifically to avoid this — if you ever delete and
  re-clone one, diff its old and new commits the same way before trusting any
  existing verdicts against the new code.
- **A `false_positive` verdict here doesn't change checker behavior.** This is
  a bookkeeping tool for the triage *loop*, not a suppression mechanism the
  checker itself reads. A `false_positive` finding worth encoding permanently
  still belongs in `docs/<checker-name>-false-positives.md` (see
  `.claude/skills/kibitzer-sample-review`) once its root cause is confirmed —
  that's the artifact that actually informs a future fix.
