#!/usr/bin/env python3
"""How good the derived relations' size estimates are (Q184), from a run's `RERAIL_SIZES_LOG`.

Estimates are measured directly here, not through a run's time. A run's time moves about 3% between two runs of
the same build, and an estimate's error shows in it only through the orders chosen from it.

A log holds two kinds of tab-separated lines (`plan::prepared::sizes_log`):

  estimate REL PER_NODE RECURSIVE    what preparation sized REL at, rows per node
  seen REL ASSUMED HELD NODES        a prepared plan taken: what it assumed of REL, and what the run held then

Usage:
  derived_sizes.py LOG                 the estimates against what the run held
  derived_sizes.py LOG --against LOG2  what one run held against what another held, relation by relation: how well
                                       sizes a run saw would size another corpus's (Q184's "size them from what
                                       runs saw")
"""
import argparse
import math
import statistics
import sys
from collections import defaultdict


def read(path):
    est = defaultdict(list)
    rec = {}
    seen = defaultdict(list)
    for line in open(path, errors="replace"):
        f = line.rstrip("\n").split("\t")
        try:
            if f[0] == "estimate" and len(f) >= 4:
                est[f[1]].append(float(f[2]))
                rec[f[1]] = f[3] == "1"
            elif f[0] == "seen" and len(f) >= 5:
                seen[f[1]].append((float(f[2]), float(f[3]), float(f[4])))
        except ValueError:
            continue
    return est, rec, seen


def deciles(xs):
    xs = sorted(xs)
    return [xs[min(len(xs) - 1, int(len(xs) * i / 10))] for i in range(1, 10)] if xs else []


def within(rs, k=2.0):
    return sum(1 for r in rs if 1 / k <= r <= k) / len(rs) if rs else float("nan")


def show(title, ratios, weights=None):
    """Ratios held/assumed (or B/A): the share within 2x, plans-weighted and by relation, and their deciles."""
    if not ratios:
        print(f"{title}: none")
        return
    rs = list(ratios.values())
    line = f"{title}: {len(rs)} relations, {within(rs):.0%} within 2x, {within(rs, 10):.0%} within 10x"
    if weights:
        w = sum(weights[r] for r in ratios)
        ok = sum(weights[r] for r, x in ratios.items() if 0.5 <= x <= 2)
        line += f"; weighted by plans taken ({w}): {ok / w:.0%} within 2x"
    print(line)
    print("  ratio deciles: " + " ".join(f"{x:.3g}" for x in deciles(rs)))
    over = sum(1 for r in rs if r > 2)
    under = sum(1 for r in rs if r < 0.5)
    print(f"  held over 2x the estimate: {over}; under half of it: {under}")


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("log")
    ap.add_argument("--against", help="another run's log: its held sizes against this one's")
    ap.add_argument("--worst", type=int, default=15)
    a = ap.parse_args()
    est, rec, seen = read(a.log)
    held = {r: statistics.median(h / 1 for _, h, _ in xs) for r, xs in seen.items()}
    assumed = {r: statistics.median(x for x, _, _ in xs) for r, xs in seen.items()}
    plans = {r: len(xs) for r, xs in seen.items()}
    print(f"{a.log}: {len(est)} relations estimated, {len(seen)} met by a taken plan, {sum(plans.values())} plans taken")
    ratio = {r: max(held[r], 1e-12) / max(assumed[r], 1e-12) for r in seen}
    show("held / assumed", ratio, plans)
    for flag, name in ((True, "recursions"), (False, "the rest")):
        show(f"  {name}", {r: x for r, x in ratio.items() if rec.get(r) is flag}, plans)
    worst = sorted(ratio, key=lambda r: -abs(math.log(ratio[r])))[: a.worst]
    if worst:
        print(f"worst {len(worst)} (relation, assumed, held per node, plans):")
        for r in worst:
            print(f"  {ratio[r]:10.3g}  {r}  {assumed[r]:.3g}  {held[r]:.3g}  {plans[r]}")
    if a.against:
        _, _, seen2 = read(a.against)
        held2 = {r: statistics.median(h for _, h, _ in xs) for r, xs in seen2.items()}
        both = sorted(set(held) & set(held2))
        print(f"{a.against}: {len(seen2)} relations met; {len(both)} in both")
        show("held there / held here", {r: max(held2[r], 1e-12) / max(held[r], 1e-12) for r in both}, plans)
        show("held there / assumed", {r: max(held2[r], 1e-12) / max(assumed[r], 1e-12) for r in both}, plans)
    return 0


if __name__ == "__main__":
    sys.exit(main())
