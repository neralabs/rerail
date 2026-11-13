"""CPU cost on a synthetic call graph (user+sys, this process only).

    python3 bench.py NODES BACK_EDGE_FRACTION [modules|giant] [db-path]

Shape: functions 0..N-1, about 5 calls each, mostly to later functions (a DAG); a fraction of
calls go back to earlier functions (recursion, which makes cycles); 1% of functions are sinks.
Roots are every function, in order, as the scripts iterate them.
"""
import os
import random
import resource
import sys

from dfs_first import SetBased, oracle


def cpu():
    r = resource.getrusage(resource.RUSAGE_SELF)
    return r.ru_utime + r.ru_stime


def build(n, back, giant, seed=1):
    rng = random.Random(seed)
    edges = []
    for s in range(n):
        for o in range(rng.randint(1, 9)):
            if rng.random() < back and s > 0:
                # recursion: back to a function of the same module (5 functions), or, for the
                # worst case (giant), anywhere in the last 50
                lo = s - s % 5 if not giant else max(0, s - 50)
                d = rng.randrange(lo, s + 1)
            else:
                d = min(n - 1, s + 1 + int(rng.expovariate(1 / 200)))
            edges.append((s, d, o, s * 10 + o))
    sinks = {v: v * 10 for v in rng.sample(range(n), n // 100)}
    return edges, sinks, list(range(n))


def main():
    n, back = int(sys.argv[1]), float(sys.argv[2])
    giant = len(sys.argv) > 3 and sys.argv[3] == "giant"
    path = sys.argv[4] if len(sys.argv) > 4 else ":memory:"
    if path != ":memory:" and os.path.exists(path):
        os.replace(path, path + ".old")
    edges, sinks, roots = build(n, back, giant)
    print(f"{n:,} nodes, {len(edges):,} edges, {len(sinks):,} sinks, back-edge fraction {back}, {'giant cycle' if giant else 'module cycles'}")
    for share in (False, True):
        t = cpu()
        skip = not share and n > 20000          # the fresh-memo oracle is quadratic; too slow here
        want, steps = (None, 0) if skip else oracle(edges, sinks, roots, "node", share)
        t_or = cpu() - t
        t = cpu()
        sb = SetBased(edges, sinks, path if not share else ":memory:")
        t_sql = cpu() - t
        t = cpu()
        got = sb.run(roots, "node", share)
        t_run = cpu() - t
        assert skip or got == want, "disagreement"
        live = len(sb.live)
        print(f"  {'shared memo (scripts)' if share else 'fresh memo (dfs_first)':24}"
              f" oracle {t_or:6.2f}s {steps:>11,} steps |"
              f" set-based SQL {t_sql:5.2f}s + search {t_run:5.2f}s"
              f" {sb.steps:>9,} steps | live {live:,}, knot {len(sb.knot):,}, chain meets knot {len(sb.ent):,}, searches {sb.searches:,}")


if __name__ == "__main__":
    main()
