#!/usr/bin/env python3
"""Tracks true/false-positive triage decisions for kibitzer findings on the
real-world backtest corpus (docs/backtest-repos.md), across repeated runs.

Each repo gets one JSONL file under docs/backtest-triage/<repo-slug>.jsonl —
one line per triaged finding, keyed by (file, line, rule). `run` re-executes a
checker over the repo and reports which findings are already triaged (with
their verdict) versus new/unreviewed versus stale (triaged before, not
produced this run — the code or the checker changed). `mark` records or
updates one verdict. See docs/backtest-triage/README.md for the workflow.

Stdlib only, deliberately — this is a triage-bookkeeping script, not a new
kibitzer feature; it doesn't need a dependency for what dict + json already do.
"""
from __future__ import annotations

import argparse
import json
import os
import re
import subprocess
import sys
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

TRIAGE_DIR = Path(__file__).resolve().parent.parent / "docs" / "backtest-triage"
FINDING_RE = re.compile(r"^(?P<file>.+):(?P<line>\d+): \[(?P<rule>[\w-]+)\] (?P<message>.*)$")
VERDICTS = ("true_positive", "false_positive", "needs_discussion")


def store_path(repo_slug: str) -> Path:
    return TRIAGE_DIR / f"{repo_slug}.jsonl"


def load_store(repo_slug: str) -> dict[tuple[str, int, str], dict]:
    path = store_path(repo_slug)
    entries: dict[tuple[str, int, str], dict] = {}
    if not path.exists():
        return entries
    for line in path.read_text().splitlines():
        line = line.strip()
        if not line:
            continue
        rec = json.loads(line)
        entries[(rec["file"], rec["line"], rec["rule"])] = rec
    return entries


def save_store(repo_slug: str, entries: dict[tuple[str, int, str], dict]) -> None:
    TRIAGE_DIR.mkdir(parents=True, exist_ok=True)
    path = store_path(repo_slug)
    ordered = sorted(entries.values(), key=lambda r: (r["file"], r["line"], r["rule"]))
    path.write_text(
        "".join(json.dumps(rec, sort_keys=True, ensure_ascii=False) + "\n" for rec in ordered)
    )


def run_checker(repo_dir: Path, checker: str, glob: str, kibitzer_bin: str) -> list[dict]:
    files = sorted(str(p.relative_to(repo_dir)) for p in repo_dir.rglob(glob) if p.is_file())

    def check_one(rel_path: str) -> str:
        result = subprocess.run(
            [kibitzer_bin, "check", "native", checker, rel_path],
            cwd=repo_dir,
            capture_output=True,
            text=True,
        )
        return result.stdout

    findings = []
    with ThreadPoolExecutor(max_workers=os.cpu_count() or 4) as pool:
        for stdout in pool.map(check_one, files):
            for line in stdout.splitlines():
                m = FINDING_RE.match(line)
                if not m:
                    continue
                findings.append(
                    {
                        "file": m.group("file"),
                        "line": int(m.group("line")),
                        "rule": m.group("rule"),
                        "message": m.group("message"),
                    }
                )
    return findings


def cmd_run(args: argparse.Namespace) -> None:
    repo_dir = Path(args.repo_dir).expanduser().resolve()
    findings = run_checker(repo_dir, args.checker, args.glob, args.kibitzer_bin)
    if args.rule:
        findings = [f for f in findings if f["rule"] == args.rule]

    store = load_store(args.repo_slug)
    seen_keys = set()
    by_verdict: dict[str, list[dict]] = {v: [] for v in VERDICTS}
    new: list[dict] = []

    for f in findings:
        key = (f["file"], f["line"], f["rule"])
        seen_keys.add(key)
        rec = store.get(key)
        if rec is None:
            new.append(f)
        else:
            by_verdict.setdefault(rec["verdict"], []).append({**f, **rec})

    stale = [rec for key, rec in store.items() if key not in seen_keys]

    print(f"=== {args.repo_slug} / {args.checker}" + (f" [{args.rule}]" if args.rule else "") + " ===")
    print(f"{len(findings)} findings this run, {len(store)} previously triaged")
    for verdict in VERDICTS:
        items = by_verdict.get(verdict, [])
        print(f"  {verdict}: {len(items)}")
        if args.show_known:
            for f in items:
                print(f"    {f['file']}:{f['line']}: [{f['rule']}] {f['message']}")
    print(f"  NEW (untriaged): {len(new)}")
    for f in new:
        print(f"    {f['file']}:{f['line']}: [{f['rule']}] {f['message']}")
    if stale:
        print(f"  STALE (triaged before, not seen this run): {len(stale)}")
        for rec in stale:
            print(f"    {rec['file']}:{rec['line']}: [{rec['rule']}] verdict={rec['verdict']}")

    if new:
        sys.exit(1)


def cmd_mark(args: argparse.Namespace) -> None:
    if args.verdict not in VERDICTS:
        sys.exit(f"verdict must be one of {VERDICTS}, got {args.verdict!r}")
    store = load_store(args.repo_slug)
    key = (args.file, args.line, args.rule)
    store[key] = {
        "file": args.file,
        "line": args.line,
        "rule": args.rule,
        "verdict": args.verdict,
        "note": args.note or "",
    }
    save_store(args.repo_slug, store)
    print(f"marked {args.file}:{args.line} [{args.rule}] = {args.verdict}")


def cmd_list(args: argparse.Namespace) -> None:
    store = load_store(args.repo_slug)
    for rec in sorted(store.values(), key=lambda r: (r["file"], r["line"], r["rule"])):
        if args.verdict and rec["verdict"] != args.verdict:
            continue
        note = f" — {rec['note']}" if rec.get("note") else ""
        print(f"{rec['file']}:{rec['line']}: [{rec['rule']}] {rec['verdict']}{note}")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)

    p_run = sub.add_parser("run", help="run a checker against a repo and diff against triaged findings")
    p_run.add_argument("--repo-slug", required=True)
    p_run.add_argument("--repo-dir", required=True)
    p_run.add_argument("--checker", required=True)
    p_run.add_argument("--glob", required=True, help="e.g. '*.go'")
    p_run.add_argument("--rule", help="filter to one rule id, e.g. flag-argument")
    p_run.add_argument("--show-known", action="store_true", help="also print already-triaged findings")
    p_run.add_argument("--kibitzer-bin", default=os.environ.get("KIBITZER_BIN", "kibitzer"))
    p_run.set_defaults(func=cmd_run)

    p_mark = sub.add_parser("mark", help="record or update one finding's verdict")
    p_mark.add_argument("--repo-slug", required=True)
    p_mark.add_argument("--file", required=True, help="path relative to the repo root")
    p_mark.add_argument("--line", required=True, type=int)
    p_mark.add_argument("--rule", required=True)
    p_mark.add_argument("--verdict", required=True, choices=VERDICTS)
    p_mark.add_argument("--note")
    p_mark.set_defaults(func=cmd_mark)

    p_list = sub.add_parser("list", help="list triaged findings for a repo")
    p_list.add_argument("--repo-slug", required=True)
    p_list.add_argument("--verdict", choices=VERDICTS)
    p_list.set_defaults(func=cmd_list)

    args = parser.parse_args()
    args.func(args)


if __name__ == "__main__":
    main()
