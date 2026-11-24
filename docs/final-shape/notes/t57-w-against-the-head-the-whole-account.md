# t57: W against the head, the whole account of CPU and memory

## Intro

W (6970a3e539, 2026-11-19: the engine with the dead-code pack's reference graph written in Rust) and the head
(`10dfc35d09`, the same pack's analysis as rules, decision 0483's translation) find the same rows on `all`. The head
takes about three times W's CPU and twice its memory. This note accounts for both, top down (by the engine's spans)
and bottom up (by `sample`'s own frames), from one run each on m3, back to back at load 3–5, profiled
(`tmp/refs/three/acct_cpu.sh`: `RERAIL_PROFILE`, `sample` over the run, RSS every 0.5 s; `bottomup.py` for the
frames). It then lists what other worktrees have built since W that bears on it.

The conclusion first: **the cost of a join's work is the same in both; the head does 8.5 times as much of it.** The
per-probe work of t56 (steps 3–7) cannot close that, and it stops here.

## 1. The runs

| | CPU | wall | instructions | peak footprint | max RSS |
|---|---|---|---|---|---|
| W | 50.9 s | 15.3 s | 500 G | 1,076 MB | 1,688 MB |
| head | 155.0 s | 46.4 s | 1,643 G | 2,433 MB | 4,102 MB |

Unprofiled (`wvh.sh`, load 5): W 50.2 s and 482 G, the head 132.0 s and 1,571 G.

## 2. CPU, top down: the spans' own CPU by kind of work

| Kind of work | W s | head s | head − W |
|---|---|---|---|
| per-file joins (`joins: wait`) | 3.41 | 52.25 | +48.8 |
| W's reference graph (Rust, off the tape) | 6.13 | — | −6.1 |
| cross-file joins | 6.76 | 30.19 | +23.4 |
| the per-file batch (W `other: local`; head `other: wait`: matcher wave 5.8, exports measured 2.0, …) | 15.65 | 27.02 | +11.4 |
| measuring sources | 0.14 | 8.36 | +8.2 |
| live statistics, bookkeeping | 0.06 | 3.40 | +3.3 |
| parsing | 2.45 | 2.95 | +0.5 |
| moving rows to the cross-file part | 0.50 | 1.74 | +1.2 |
| the rest | 6.2 | 9.7 | +3.5 |
| **all spans** | **41.3** | **135.6** | **+94.3** |

### The joins: same cost a unit, 8.5 times the units

| | probe CPU | planned work (units) | ns a unit |
|---|---|---|---|
| W | 5.38 s | 42.5 M | 127 |
| head | 50.14 s | 361.9 M | 139 |

W's largest per-file head is `inventory` (1.07 M units); its largest cross-file head is `reach_edges` (2.6 M). The
head's per-file heads, which replace W's 6.1 s of Rust: `wstart` 15.8 M, `rf_in` 16.6 M, `bind_only` 20.3 M,
`pre_311` 7.9 M, `pre_308` 6.5 M, `rf_named` 6.3 M, `rf_expr` 6.3 M, `cls_late` 5.0 M, `cls_early` 4.8 M, `rf_bind`
4.5 M, `rf_annot_head` 4.5 M, `rf_qn` 4.2 M, `param` 3.8 M, … Its cross-file part: `rf_named` 4.1 M, `rf_method`
2.2 M, `rf_super_method` 2.0 M, …

So the gap is the work the rules ask for, not the speed of a probe. The question for each of those heads is what W's
Rust computed in its place, and why the rules need so many more rows to get the same answer.

## 3. CPU, bottom up: own frames (`sample`, every thread)

Both runs are 81% parked threads. Of the rest, a "kernel" bucket (mostly the `trap` frame of waiting threads) is
20–26% in both. Without it, W has about 32,000 working samples and the head about 101,000, the same 3.2 times.

The head's working frames, by what they do:
- **the join loop**, about a quarter: `cands::candidates` 5,448, `run_join_in_each::enter` 3,747, `RowRef::at`
  3,639, `IntCol::get` 2,857, `runs::cmp_key` 1,651, `Cands::any` 1,258, `Rows::each` 1,400 or so;
- **an id's place and the views' reads**, about 9,000: `Projected::local_of` 1,464, `Projected::node` 1,286,
  `Ranks::select` 1,146, `Numbering::granule` 1,002, `tape reads::parent` 808, `Tape::code` 972;
- **statistics and bookkeeping**, about 6,000, which W barely has: `Sketch::add` 939, `Sketch::grow` 796,
  `plan::statkeep` 1,094, `RelStats::count` 572, `plan::corpus::retain ids` 851, hash maps in blocks (insert 924,
  rehash 588);
- the matcher wave's set reader (`Reader::made` 1,265) and atoms (`AtomRead::find` 1,585), about as in W;
- the allocator: `memmove` 3,328, jemalloc about 2,500 (`arena_stats_merge` 715 is the profile's memory sampler).

## 4. Memory

### At the peak (both at the run's end, writing the outputs)

| Holder | W MB | head MB |
|---|---|---|
| footprint | 1,072 | 2,396 |
| heap live (jemalloc) | 847 | 1,169 |
| heap resident beyond live | 233 | 403 |
| host blocks | 93 | 1,555 (829 of them free, on the blocks' lists) |
| the device pool (a CPU run) | 0 | 190 (leases 128, idle 62) |

The head's extra 1,300 MB is mostly blocks: 829 MB freed and kept on the lists, about 700 MB more live, and 190 MB
of a Metal pool a CPU run never uses. The heap's live bytes are 320 MB more.

### Churn: what the spans allocated

W's spans allocate hundreds of MB each. The head's chunks allocate 37.5 GB over the run:
- the matcher wave run: 6.2 GB (25 calls);
- the base relations counted for the cross part: `node` 2.2 GB, `text` 1.8 GB, `docof` 1.6 GB, `node_order`
  1.6 GB (36 calls each);
- the matcher's captures 1.5 GB; `fl1925301037_rf_named` 1.4 GB; `rr_long_text_node`'s layered rows 1.1 GB;
  `look/walk` 1.1 GB; node rows 0.8 GB; string annotations 0.75 GB.

933 MB of blocks were zeroed again as they were handed out.

### Copies (decision 0353)

W 499 MB and 1.78 s; the head 1,887 MB and 2.58 s: head rows copied from `Fresh` (`push_fresh`, 1,295 MB), staged
and merged across workers (`Fresh::absorb`, 400 MB), and the per-node rows (144 MB, 2.0 s).

### Not yet accounted

Memory by allocation stack (`system-allocator` builds under `malloc_history`) is not taken: W's checkout is at
`tmp/w-src`. The gap is in blocks, which `malloc_history` does not see. What is needed there is the blocks' live bytes
at the peak by owner (relation, index, batch, kept facts), which the profile does not yet print.

## 5. What other worktrees have built since W (not in this branch)

1,059 commits since 2026-11-19 are on other branches and not here. Those that bear on this account:

| Decision / work | Branches | What it removes here |
|---|---|---|
| 0656, the tape's auxiliary index: names, vocabulary, places, counts, read in place (Rust and UKL); `text` is a view, no row made; the places column removed; a child's place read in the index | django-bugs (steps 1–4 part 3), final-rerail-shape-phased-facts (step 4 parts 4–5, `text` done), serverplan-quality, hub | the batch's node rows, string annotations and `text` rows; the per-chunk counts of `node`/`text`/`docof`; the id-to-place reads (`Projected`, `Ranks`, `granule`) |
| 0353 step 3 on the CPU: the views read through the batch's directory in Rust; the host structures go into the batch's block; a node's text read in place | sync-integration-hub-tape (2026-11-21, 22) | the views' host structures and per-node rows |
| 0652, a relation is per-document unless declared `cross_file` (and 0649, positions stay in the per-file part) | investigate-bend-vs-datalog, Native-and-GPU-strata | rules the split sent to the cross-file part: part of its 30 s |
| 0671, a hashed index marks each place with its key's tag | uglykl-CPU-SIMD-parallelization | candidates' key reads in the block indexes |
| One run room; a run's anonymous room in shards; tape values admitted as one batch | django-bugs, Typing-support-final-shape | small allocations into blocks, without a lock |
| 0659, an unchanged run answered by its frames | hub, django-bugs, … | (warm runs only) |
| t50 step 9, a refused seed's counts read their inputs' indexes | Native-and-GPU-strata | t50's own work, on another branch |

This branch last merged on 2026-11-22. The hub's 0353 step 3 work and 0656 are the largest pieces of the batch's cost
and memory above, and they are built.

## 6. What follows

1. Merge the hub and the 0656 work (django-bugs, phased-facts) into this branch, and measure `all` again against W.
   That takes the batch's rows, its counts and the id-to-place reads with it.
2. For each head of section 2's list, take what W's Rust computed in its place and why the rules make so many more
   rows to get it: the 8.5 times is there, head by head.
3. The blocks at the peak: free blocks given back, the device pool not opened by a CPU run, and the live blocks
   accounted by owner.
4. The cross-file part with 0652's split.
