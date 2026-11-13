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
- `realgraph.py`: LEG-083's call graph from the real data-layer repository, and the answers of four versions
  of the script's search (as it is; one memo per root; lazy terminals only; both), for `native real`. Reads
  blobs with `git cat-file`; parses with `ast`; runs nothing from the repository.
- `vs_cache.py`: checks realgraph.py's copy of the script against the real linter's cached output.
- `plugin_scope.py`: LEG-083's findings as each of the linter's two entry points (lint-all, `--check`) sees
  them.
- `native/`: the same plan in Rust, std only, with no SQL and no materialised tables. Every set pass is a
  linear scan of a CSR edge array; Tarjan reads the array directly. It carries its own port of
  `Graph::first`, including the producer's supplied visiting set and initial memo, and checks that those
  are a rewrite of the edge and target relations.
  - `src/par.rs`: the set passes and root answering on threads; `src/gpu.rs` and `gpu/dfs_gpu.mm`: steps 1-3
    as Metal waves, compiled by `build.rs` with the system clang.

## Run

```
python3 check.py 20000                                # ok: 20000 graphs x 2 modes x 2 memo variants
python3 bench.py 5000 0.05 modules                    # recursion inside modules of 5 functions
python3 bench.py 5000 0.2 giant                       # one cycle through almost everything
python3 bench.py 100000 0.05 modules /path/to/db      # SQLite on disk; skips the quadratic oracle

cd native
cargo run --release -- check 20000                    # 20,000 small + 20,000 large graphs
cargo run --release -- state 20000                    # supplied visiting set and memo = an edge rewrite
cargo run --release -- bench 1000000 0.05             # 1,000,000 functions, 5,000,000 calls
cargo run --release -- bench 5000 0.2 giant
cargo run --release -- par 5000                       # threads and GPU arrays = sequential (THREADS=k)

python3 -I ../realgraph.py DATA_LAYER_REPO COMMIT leg.graph leg   # LEG-083's files
python3 -I ../realgraph.py DATA_LAYER_REPO COMMIT all.graph all   # every .py file
cargo run --release -- real leg.graph                             # four versions x set-based, threads, GPU
python3 -I ../vs_cache.py DATA_LAYER_REPO COMMIT leg.graph        # copy = real linter, where cached
python3 -I ../plugin_scope.py SCRIPT leg.graph DATA_LAYER_REPO COMMIT
```

## Results (2026-11-05)

Correctness:

- 20,000 graphs of 1–9 nodes and 5,000 of 10–40 nodes agree with the oracle exactly: the hit and the
  first edge's location, for every root.
- A planted bug is caught: a fresh memo where the scripts share one disagrees on graph 97 of the small
  set and graph 0 of the large one.
- The producer's `match: "node"` mode equals `match: "edge"` over the non-target roots, with each
  target root its own hit (20,000 graphs). So one built-in covers both modes.

Native, at 1,000,000 functions and 5,000,000 calls: 0.20 s CPU of set passes, 0.06 s to answer every
root with a fresh memo, 0.03 s with the shared one (the scripts' own search: 0.32 s); 435 MB peak. Both
planted bugs (a fresh memo for the shared variant; a rewrite that keeps a known node as a target) are
caught.

Real repository (LEG-083):

- The copy of the script equals the real linter on all 355 files its fact cache covers (10 findings).
- The set-based plan equals the copy on every root, for all four versions:
  - 61,403 roots in LEG-083's files at 2b1504;
  - 44,811 at 41ab36f;
  - 152,753 over every .py file.
- Set passes take 1–4 ms.
- The script's eager test is the problem, not the search. At 2b1504 its 18 findings are all pathlib
  `resolve()` or `resolve(arg)` calls, and the corrected test leaves 0. At 41ab36f it is 56 against 40.
- The shared memo loses 34 functions repo-wide, all behind a pathlib call. With the corrected test it loses
  none.
- See data-layer/upstream/LEG-083.md and its patch.

Threads and GPU: on 8 threads and on Metal (steps 1-3), every array equals the sequential pass on 10,000
random graphs, the bench graphs and both real graphs. At a million functions, steps 1-3 take 0.004 s of GPU
time against 0.07 s on a core.

Cost: see the tables in data-layer/DESIGN.md. In short, the fresh-memo semantics drops from a search per
root (millions of steps at 5,000 functions) to set passes plus tens of search steps. The scripts'
shared-memo search is linear and cheap: the SQLite spike costs more than it, the native one less. A cycle through most of the graph keeps the search
large, as Reif's P-completeness result says it must.

SQLite spike stage costs at 100,000 functions and 500,000 calls, CPU seconds:

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

## Supplied state and symbol order

- The producer's supplied `visiting` set and initial `memo` are modelled in the native oracle and are a
  rewrite of the arguments (DESIGN.md, addition 7), checked on 20,000 graphs. A seed must name a target
  that is itself neither blocked nor seeded, as the memo entries a search writes always do.
- Roots ordered by a symbol: the spikes use integers. Soufflé's `<` on symbols compares the strings, so
  text order is Soufflé's own.
