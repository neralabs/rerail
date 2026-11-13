# regret.py <tsv>...: per skew log, how often the skewed order differs and what it costs by the true statistics.
import sys, math
for p in sys.argv[1:]:
    rows = [l.rstrip('\n').split('\t', 4) for l in open(p)]
    r = [float(x[0]) for x in rows]; same = sum(int(x[1]) for x in rows)
    work_s = sum(float(x[2]) for x in rows); work_t = sum(float(x[3]) for x in rows)
    worse = sorted(r)
    q = lambda f: worse[min(len(worse) - 1, int(f * len(worse)))]
    print(f"{p.split('/')[-1]}: {len(r)} searches, same order {same/len(r):.1%}, regret median {q(.5):.3f} p95 {q(.95):.3f} p99 {q(.99):.3f} max {max(r):.2f}; total planned work x{work_s/work_t:.3f}; >1.5x: {sum(x>1.5 for x in r)}, >2x: {sum(x>2 for x in r)}")
    for x in sorted(rows, key=lambda x: -float(x[0]))[:3]:
        print(f"   x{float(x[0]):.2f} work {float(x[2]):.0f} vs {float(x[3]):.0f}: {x[4][:110]}")
