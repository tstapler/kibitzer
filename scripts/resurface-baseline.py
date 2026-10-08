#!/usr/bin/env python3
"""Baseline for the inline-ignore go/no-go gate: how often does a finding re-surface?

Input is the output of `kibitzer check backtest all` (WITHOUT --only-new, so
pre-existing findings are present and tagged), one line per finding:

    <transcript>#<seq> <file>:<line>: [<checker>] [(pre-existing) ]<message>

A tuple is (transcript, rule, file). A tuple "re-surfaces" when the same
(rule, file, message) is reported at an earlier seq, then again at a later seq
tagged (pre-existing): the agent edited the file again and the same finding
persisted. See project_plans/inline-ignore-syntax/implementation/plan.md, Task 0.1.1.

Usage: resurface-baseline.py replay.txt [--sample N] [--seed S]
Stdlib only.
"""
from __future__ import annotations

import argparse
import random
import re
import statistics
from collections import defaultdict

LINE_RE = re.compile(
    r"^(?P<transcript>\S+\.jsonl)#(?P<seq>\d+) (?P<file>.+?):(?P<line>\d+): "
    r"\[(?P<rule>[\w-]+)\] (?P<pre>\(pre-existing\) )?(?P<message>.*)$"
)


def parse(path: str):
    rows = []
    with open(path, encoding="utf-8", errors="replace") as fh:
        for raw in fh:
            m = LINE_RE.match(raw.rstrip("\n"))
            if m:
                d = m.groupdict()
                d["seq"] = int(d["seq"])
                d["pre"] = bool(d["pre"])
                rows.append(d)
    return rows


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("replay")
    ap.add_argument("--sample", type=int, default=20)
    ap.add_argument("--seed", type=int, default=7)
    args = ap.parse_args()

    rows = parse(args.replay)
    tuples = defaultdict(list)
    for r in rows:
        tuples[(r["transcript"], r["rule"], r["file"])].append(r)

    # events: (transcript, seq) with at least one new (non-pre-existing) finding
    events = {(r["transcript"], r["seq"]) for r in rows if not r["pre"]}

    resurfaced = {}  # tuple -> list of re-surfaced (message, first_seq, later_seq)
    for key, rs in tuples.items():
        by_msg = defaultdict(list)
        for r in rs:
            by_msg[r["message"]].append(r)
        hits = []
        for msg, group in by_msg.items():
            group.sort(key=lambda r: r["seq"])
            first = group[0]
            for later in group[1:]:
                if later["seq"] > first["seq"] and later["pre"]:
                    hits.append((msg, first["seq"], later["seq"], first["pre"]))
        if hits:
            resurfaced[key] = hits

    F = len(tuples)
    Rn = len(resurfaced)
    print(f"findings parsed: {len(rows)}")
    print(f"F (tuples with a finding): {F}")
    print(f"re-surfaced tuples: {Rn}  R = {Rn / F:.4%}" if F else "no findings")
    strict = {k for k, h in resurfaced.items() if any(not x[3] for x in h)}
    print(f"  strict (first sighting was agent-introduced, not pre-existing): {len(strict)} = {len(strict) / F:.4%}")
    per_rule_f = defaultdict(int)
    per_rule_r = defaultdict(int)
    for k in tuples:
        per_rule_f[k[1]] += 1
    for k in resurfaced:
        per_rule_r[k[1]] += 1
    print("per-rule (re-surfaced / tuples):")
    for rule, n in sorted(per_rule_r.items(), key=lambda x: -x[1]):
        print(f"  {rule}: {n}/{per_rule_f[rule]}")
    counts = [len(h) for h in resurfaced.values()]
    if counts:
        print(f"median re-reports per re-surfaced tuple (distinct message re-surfacings): {statistics.median(counts)}")
    total_instances = 0
    for key, hits in resurfaced.items():
        by_msg = defaultdict(int)
        for msg, *_ in hits:
            by_msg[msg] += 1
        total_instances += len(by_msg)
    print(f"E (hook-visible failing edit events): {len(events)}")
    print(f"distinct re-surfaced findings (rule,file,message): {total_instances}")
    reports = sum(1 for r in rows if r["pre"] and (r["transcript"], r["rule"], r["file"]) in resurfaced)
    print(f"total re-report events (pre-existing sightings in re-surfaced tuples): {reports}")

    rng = random.Random(args.seed)
    keys = sorted(strict) if strict else sorted(resurfaced)
    pool = sorted(strict)
    sample = rng.sample(pool, min(args.sample, len(pool)))
    print(f"\n--- sample of {len(sample)} strict re-surfaced tuples ---")
    for i, key in enumerate(sample, 1):
        t, rule, f = key
        msg, s1, s2, _ = next(h for h in resurfaced[key] if not h[3])
        print(f"{i}. {t}\n   rule={rule} file={f}\n   first_seq={s1} later_seq={s2} re-reports={len(resurfaced[key])}\n   msg={msg[:200]}")


if __name__ == "__main__":
    main()
