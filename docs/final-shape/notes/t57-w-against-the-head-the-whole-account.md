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

## 7. Steps 1–3, done (2026-11-24)

### Step 1: the merge, and `all` again

The hub (roll-up 116, e41938af26) is merged, and 0649 and 0652 are ported onto its tree (9123d21c82). 0652's
follow-up (872f3a300b) and the strata branch's view work are not: they meet the hub's own view work in the same
structures (the merge question, published as a page). With 0652 the cross-file part's share of the join work fell
from 18% to under 5%, and no per-file relation joins there.

The merge first made the head slower, profiled on m3: 1,643 G → 2,246 G instructions, for fewer join rows (444 M
→ 384 M). The stack samples put it in two places, both fixed (9b6d39a4c1):
- the code table, moved into a block by the hub, read every word through a non-inlined `AsRef` with the region
  guard's check, about eight a node the matcher reads: now a slice taken once;
- every freed chunk gave its pages to the OS at once, and the next allocation faulted them back; the hub's blocks
  for short-lived vectors made that six times as frequent. A free now gives pages back once its merged free chunk
  reaches 256 KB (`GIVE_MIN`).

A whole-run sample after the fix: the code table's calls about 5,000 samples → 17, give-back and reuse 1,970 →
208. Unprofiled, `all` on m1max: W 477 G, the head 1,927 G (on m3 before the fixes: 582 G and 2,151 G). The hosts
were loaded; the difference is four times, which the load does not move.

### Step 3: blocks given back, no device pool for a CPU run

- A run bound to the CPU leased the residency's 64 MB slab of the device pool (190 MB on `all`) and copied every
  tape into a room there. `rerail_blocks::alloc::run_device` asks the run's binding; `tape::placed::device` is
  `None` for a CPU run, and the store's writer takes the parsed image where it lies (a5bae2b36a). s10: bound to
  the CPU, 0 MB leased; with `--metal`, 1,122 tapes placed and 64 MB leased; the same rows.
- `Extent::given` and `host_free_resident`: the free lists' resident bytes, which `freed_blocks_are_given_back`
  bounds.
- Still owed: the live blocks at the peak by owner.

### Step 2: each heavy head against W's Rust

W's reference graph is `calls::summarize`, run in the parse workers on each Python file's tape:
1. `ScopeTable::build_in` walks the tree once and gives each identifier its binding (local, parameter, class
   member, import, global). `resolve_python_index` then resolves each load: 3.46 M loads on `all` in 0.23 s, 70 ns
   a load.
2. `Extractor::walk` walks the tree once more with a stack of contexts (the owner definition, what `self` means,
   the class) and records each definition, call and reference with its `via`.
3. The serial part links the summaries by name (`Graph::link_references`): 8% of the graph's CPU.

Each of the head's per-file heads does one of these jobs, by joins:

| Head (units on `all`) | W's job | Why the rules make more rows |
|---|---|---|
| `wstart` 15.8 M, `walk` | where a load's lookup starts: the scope on the walk's stack | six steps a name: the name, `!def_outside`, `nearest`, an id's range, `kind`, `scope` (E3, E7) |
| `bind_only` 20.3 M | a binding target, marked as the walk passes the statement that binds | a union of ten rules read only negated; demand probes each rule for every identifier (E4) |
| `rf_in` 16.6 M | the owner definition on the walk's stack | the nearest `def` found, then range and kind tests as steps (E3) |
| `cls_early` 4.8 M, `cls_late` 5.0 M | a class's members, collected once (`members`) | every name of the body against every binding before it (E2); six negations a name (E10) |
| `param` 3.8 M | the parameter list's children, read at the definition | the list joined to its children and its kinds as steps (E3) |
| `rf_qn` 4.2 M | the qualified name, the stack's names joined | the recursive rule binds the scope and lists every node under it to keep the few definitions (E1) |
| `rf_named`, `rf_expr`, `rf_bind`, `rf_annot_head` 4.5–6.3 M | a reference's `via`, decided as the walk reads it | each `via` a relation of its own over every name read, each materialized (E7) |
| `rf_builtin` | not a reference: a builtin's name is never interned | every read name's text joined, then tested against 150 names (E6) |
| `rf_fcond`, `rf_crossing` | the context on the stack | the nearest condition or function found as a join, its kind tested after (E6) |
| `rf_same_qn` | the interner: one id a qualified name | every pair of definitions in a document through two `docof`s and two `node_order`s (E9) |

So the rules do W's two walks as relations that a name passes through one after another, each materialized, each
found by joins where W reads the node or the stack. The rules are right to say it this way; the engine is what makes
it cost 8.5 times. §8 lists what it needs.

## 8. The expansions, and the shapes the engine lacks

The cost cases (`crates/boot/.../convert/tests/cost_shape_tests.rs`, 31 of them) run the rules over generated
modules and bound each head's join work a name read. 9 pass after steps 1–3. Each failing case names its expansion:

| | Expansion | Cases (now) | The missing shape |
|---|---|---|---|
| E1 | a recursive `nearest` called with its far end bound lists every node under it | `rf_qn`: a step makes 4,528 rows for 96 names | `nearest` evaluated from the near end only: a bound scope binds nothing to walk down from |
| E2 | `f < n` between nodes becomes two joins with `node_order` | 4 plans join `node_order`; `cls_early` | a comparison of nodes is a read of their tape order (their ids are in tape order), a test in the step that binds the second |
| E3 | a step that only filters bound variables (an id's range, a constant kind, a comparison) costs a unit a row | 287 plans, 34.2 units a name; `c_node` 3 units a row; `wstart`, `walk`, `rf_in`, `param` | filters fused into the step that binds their last variable |
| E4 | a union read only negated is probed rule by rule for each demanded key | `bind_only`: 9,975 units | the union made once, as a set, then tested |
| E5 | per-file relations run in the cross-file part | passes after 0652 | — |
| E6 | a test of a value is made after a join that reads every candidate | `rf_builtin` 3,441 units over 332; `rf_fcond`; `rf_crossing` | the test in the step that reads the value (a text against a constant set, a kind) |
| E7 | each name passes a dozen per-node relations, each materialized | 137 units a name read against a bound of 40 | node-keyed columns: a per-node fact is a column read by node id, not a relation joined |
| E9 | a group by key found as pairs | `rf_same_qn` | a group: one read a key |
| E10 | a chain of negations tested a name each | `cls_late` | negations of the same key read in one step |

And the allocation cases: the joins 21.6 KB a name read, closing an engine 3,113 bytes, the live statistics 4,096,
the matcher wave 1,651, string annotations 403.

The next work builds E2, E3, E1, E4 and E7, in that order: E2 and E3 are the lowering's and the join's own, and
the others read through them.

### What was built, and what the step CPU says (2026-11-24)

Units of work count a step's rows in, so a step that does little a row reads as much as one that probes. The cost
dump now also takes the profiled CPU of every step, by head, and ranks the heads by it. At 8 definitions a file, the
joins' steps took 154 ms before the work below and 116 ms after.

| | Outcome | Commit |
|---|---|---|
| E2 | A comparison of nodes reads each node's preorder key where the node lies (`@rr_node_order`). No plan joins `node_order`. | 410c2312c5 |
| E3 | No cost. The filter steps take 0.1–0.6 ms of the steps' 116–154 ms. Most are fused into the binding atom's row program already, and the rest run as comparisons. Writing hiding's bounds before the body moved the units a name from 34.2 to 31.1 and the CPU not at all, so that change was not kept. The case now bounds their CPU at 1% of the steps'. | aab93aa6c9 |
| — | Not in the catalogue, the largest step: `text($e, $x)` read by its string walked every text row of every document for each probe (`rf_target`, 36.6 ms of 154). `Projected::text_rows_of` finds a document's rows of a text where the text occurs in its source, by a binary search on the tape's node starts. `rf_target` went from 52.9 to 12.8 ms, with the same outputs. The planner prices a step by the rows it passes on, so a read's own cost never enters a plan's price; this one was found only by timing. | 37f8fd8c9e |
| E1 | `NearEnds`, run as a program is prepared (not in the lowering, which stays T's front end's). In a recursive rule whose recursion binds a `nearest`'s far end, the `nearest` and the atoms that bind its near end become a helper, made once and read by the far end. `rf_qn` reads 32–160 helper rows a wave, against 4,528 nodes. | e91529c5dd |
| E4 | Built (2026-11-25). The demand split keeps a candidate whole where its readers ask about at least as many keys as it holds. Both sizes are the size scan's. It now bounds:<br>- a derived atom by its relation's own rows;<br>- an `ast` literal's node by its matcher's root kind's nodes (`TreeStats::freq`);<br>- a variable a comparison computes (`$x = @rr_node_text_any($n)`) by its inputs' rows;<br>- a relation another part derives by its measured rows.<br>On the cost case, `bind_only`'s readers ask about 3,602 keys of its 2,662 rows. It is made once, by its own rules: 3,687 units for 516 rows, against a bound of 1,437. The rest is the parameter rules' steps, a unit a row each (E3's count, not CPU). The cost cases pass 15 of 31, and the whole graph is 122 units a name read (137 before). On `all`, the head's instructions went from 1,604–1,612 G to 1,562 G. | — |
| E7 | A spike beside the engine (2026-11-25): `rerail_blocks::node_keyed`. A relation keyed by one node of one document is a set (a bit a node), a value a node (`u32`; a second value breaks the key, and the writer is told), or a node's values grouped by offsets. All of it lies in the block, read in place. Over the same rows as a frozen relation probed by its index on the node (Relax, one thread, 2 M tests of 200 k nodes): a set's test 24.7 ns against 3.2 ns, in 35 times fewer bytes; a value's read 58.9 ns against 10.7 ns, in 4.5 times fewer bytes. Not yet wired into the engine. | — |

The cost cases pass 13 of 31 (with E1). The remaining heads by step CPU: `rf_target` 12.8 ms (its `text` read and
`rf_named`), `wstart` 8.1 (E7: `nearest` per name read, then the scope's kind), `rf_in` 6.0, `bind_only` 2.6–5.2
(E4), `target_form` 3.9, `rf_local` 3.6.

## 9. The merge's cost, function by function (2026-11-24)

Before the merge, `all` cost the head 132 s and 1,571 G instructions. After it, 152.7 s and 1,896 G (§7: 2,246 G before
9b6d39a4c1's two fixes). Both builds ran profiled and sampled on m3 over the whole run, the same corpus, back to back:
the pre-merge commit 83fe3f5449 with its own library and programs, and HEAD.

**What did not change.** The joins' own step CPU is the same: 51.7 s before, 50.9 s after. `text` got cheaper (7.1
s → 2.4 s, read by its string, 37f8fd8c9e). The cross-file part got cheaper (31.1 s → 15.9 s, 0652), and the
per-file part took that work. So the plans are not the regression.

**What did.** The non-wait samples went from 110.9 k to 141.9 k. Most of the 31 k extra are in code the merge
brought, outside the steps:

| Where | Samples before → after | What it is |
|---|---|---|
| `Guard::cpu_read`/`cpu_write` | 0 → 2,819 | every checked access of a run-room vector called its block's ledger through `dyn Guard` (decision 0487 R4), for a check that is one relaxed load |
| `Numbering::select` from `Viewed::read_value` | 990 (`Ranks::select`) → 3,470 | a view's row read through its document's directory row selects the row's tape node once per column, not once per row |
| `Docs::read`, `doc_of_row` | ~0 → 2,456 | a view's value read alone (`Source::value`) finds its document and remakes its read for each value |
| `TapeDoc::transducer_kind` → `entry` | → 1,117 | the AST matcher reads a node's kind through a whole `EntryRef` (alias string, classes) |
| `rows::placed`, `open_blocks`, `InChunk`, `Ledger::take_in`, BTreeMap removes, `memmove` | ~1,400 → ~6,300 | each small vector of a run (a clone, a capture's rows, a `collect`) takes a region in a block: three shared `Arc`s cloned (the chunk's, the run's, the run's `Open`), a lane's lock, the ledger's lock, a region with two `Arc`s and a box. Before the merge, one jemalloc call. |
| `Fresh::settle` | 647 → 2,561 | a wave's rows moved from a `Vec<i64>` into a relation's wave room: each comparison of the sort looks up the span, the region's `OnceLock`, the part's column and its chunk |

`literal_str`'s 8.4 s (0.3 s before) is the AST matcher's work charged there (its rows are made on first read);
the matcher's own span fell by as much.

**Fixed:**
- the guard (359dc17781): the region keeps a pointer to the ledger's queued-use count and reads it inline. 1,896 G →
  1,797 G, 152.7 s → 140.8 s;
- a row's tape node is selected once for all its columns (`view_reads::Doc` remembers the node it last selected);
- a node's kind and field are read from its code's record alone (`CodeTable::kind_field`). These two: 1,797 G →
  1,759 G on m3 (load 19, so its CPU of 147.7 s says nothing).

**Still owed:** the run room's small allocations, and the wave room's reads in `settle`. Both come from the hub's
run room: every vector in the run's blocks (8f4fdb7293, the user's call). That stays. What has to change is the
room's own cost: a small region taken without three shared `Arc`s and two locks (a thread's own open stretch,
bumped), and a wave's rows read a chunk at a time in the sort, not one span-and-column lookup a value. The largest
takers: `ast_wave::captured_rows` (a capture's one-node `Vector`, about 2,100 samples) and `Within::rows` (a
vector a probe, about 970).
