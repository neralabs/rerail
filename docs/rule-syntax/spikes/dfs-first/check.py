"""Differential check: SetBased against the ordered_first.rs port, on random graphs.

    python3 check.py [cases]     # default 20000; prints the first disagreement, or ok
"""
import random
import sys

from dfs_first import SetBased, oracle


def graph(rng):
    n = rng.randint(1, 9)
    edges = []
    for _ in range(rng.randint(0, 3 * n)):
        s, d = rng.randrange(n), rng.randrange(n)
        edges.append((s, d, rng.randint(0, 4), rng.randint(1, 99)))   # ord ties are broken by dst
    sinks = {v: rng.randint(100, 199) for v in range(n) if rng.random() < 0.25}
    roots = list(range(n))
    rng.shuffle(roots)
    return edges, sinks, roots[: rng.randint(1, n)]


def main():
    cases = int(sys.argv[1]) if len(sys.argv) > 1 else 20000
    rng = random.Random(7)
    cyc_roots = 0
    for i in range(cases):
        edges, sinks, roots = graph(rng)
        sb = SetBased(edges, sinks)
        for mode in ("node", "edge"):
            for share in (False, True):
                want, _ = oracle(edges, sinks, roots, mode, share)
                got = sb.run(roots, mode, share)
                if got != want:
                    print("DISAGREE case", i, mode, "shared memo" if share else "fresh memo")
                    print(" edges", edges, "\n sinks", sinks, "\n roots", roots)
                    print(" want", want, "\n got ", got)
                    return 1
        cyc_roots += sum(r in sb.ent for r in roots)
    print(f"ok: {cases} graphs x 2 modes x 2 memo variants; {cyc_roots} roots had a chain meeting the knot")
    return 0


if __name__ == "__main__":
    sys.exit(main())
