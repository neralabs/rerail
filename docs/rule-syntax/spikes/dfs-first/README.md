# Spike: `dfs_first`, set-based until the end

The design is data-layer/DESIGN.md, addition 7. This spike asks two things: is the set-based plan exactly
the scripts' depth-first search, and what does it cost.

## Files

- `dfs_first.py`
  - `oracle`: a line-for-line port of the Data-layer branch's `Graph::first`
    (native/src/code_rule_dsl/relational/ordered_first.rs), the reproduction of the Python scripts'
    memoised search.
  - `SetBased`: SQLite computes `live`, `next`, `first_of` and the chain entry points; Tarjan finds the
    knot; Python searches only where a chain enters the knot.
- `check.py`: differential test on random graphs (self-loops, duplicate edges, tied orders), in both
  match modes, with a fresh memo and a shared one.
- `bench.py`: CPU time on synthetic call graphs.

## Run

```
python3 check.py 20000                                # ok: 20000 graphs x 2 modes x 2 memo variants
python3 bench.py 5000 0.05 modules                    # recursion inside modules of 5 functions
python3 bench.py 5000 0.2 giant                       # one cycle through almost everything
python3 bench.py 100000 0.05 modules /path/to/db      # SQLite on disk; skips the quadratic oracle
```

## Results (2026-11-05)

Correctness:

- 20,000 graphs of 1–9 nodes and 5,000 of 10–40 nodes agree with the oracle exactly: the hit and the
  first edge's location, for every root.
- A planted bug is caught: a fresh memo where the scripts share one disagrees on graph 97 of the small
  set and graph 0 of the large one.
- The producer's `match: "node"` mode equals `match: "edge"` over the non-target roots, with each
  target root its own hit (20,000 graphs). So one built-in covers both modes.

Cost: see the table in data-layer/DESIGN.md. In short, the fresh-memo semantics drops from a search per
root (millions of steps at 5,000 functions) to set passes plus tens of search steps. The scripts'
shared-memo search is linear and already cheap. A cycle through most of the graph keeps the search
large, as Reif's P-completeness result says it must.

Stage costs at 100,000 functions and 500,000 calls, CPU seconds:

| Stage | Time |
|---|---:|
| load and index | 0.44 |
| `live` | 0.34 |
| `next` | 0.42 |
| `first_of` | 0.14 |
| knot edges | 0.39 |
| Tarjan | 0.31 |
| entry points | 0.04 |

## What it found on the way

- Peeling the graph from both ends (Kahn's algorithm) is the wrong knot: it keeps every node between two
  cycles. That was 3,385 of 4,586 live nodes, against 30 for real components.
- Without the knot test, "reaches a cycle" covers almost every root in a connected call graph. The
  search region has to be where a root's `next` chain *enters* a cycle, not anything it can reach.
- A call to itself is skipped by the search. Excluding self-loops from the knot is only exact if `next`
  skips them too; the fuzzer found the case where `next(v) = v`.

## Not modelled

- The producer's supplied `memo` and `visiting` inputs, from its supplied-fact API.
- Roots ordered by a symbol: the spike uses integers.
