# Backtest finding triage

Tracks a true/false-positive verdict per finding on the real-world backtest
corpus (`docs/backtest-repos.md`), across repeated runs — so re-running a
checker after a change shows only what's actually new, instead of re-reading
the same thousands of already-judged findings every time.

One `<repo-slug>.jsonl` file per repo, one line per triaged finding, keyed by
`(file, line, rule)` — `file` is relative to the repo root so the store stays
valid regardless of where the repo happens to be cloned locally. Each record:

```json
{"file": "cmd/kubeadm/app/phases/upgrade/postupgrade.go", "line": 99, "rule": "flag-argument", "verdict": "true_positive", "note": "..."}
```

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

# Record a verdict for one of the findings it lists as NEW:
scripts/backtest-triage.py mark \
  --repo-slug kubernetes-kubernetes \
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

- **Line numbers assume the clone doesn't move on.** These repos are
  shallow-cloned once by `scripts/clone-backtest-repos.sh` and never
  `git pull`ed — if you delete and re-clone one at a newer commit, its stored
  line numbers can go stale (a previously-triaged line may now hold different
  code, or the finding may have moved). Re-triage from scratch after a re-clone
  rather than trusting old verdicts against new code.
- **A `false_positive` verdict here doesn't change checker behavior.** This is
  a bookkeeping tool for the triage *loop*, not a suppression mechanism the
  checker itself reads. A `false_positive` finding worth encoding permanently
  still belongs in `docs/<checker-name>-false-positives.md` (see
  `.claude/skills/kibitzer-sample-review`) once its root cause is confirmed —
  that's the artifact that actually informs a future fix.
