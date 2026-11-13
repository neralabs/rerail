# Tier 2 in clusters of states

## Intro

Tier 2 is one generated function. For `.rr` that function is 122,469 instructions over 227
states. Cranelift allocates registers over the whole of it at once, so a value live in one hot
path competes with every other path's values. The function also compiles as one unit, and its
compile time grows faster than its size.

This page looks at another cut. The states fall into **clusters**: states that pass control
mostly among themselves, like the states of a string or of an argument list. Each cluster
becomes one function. Inside a cluster, the code is today's: direct jumps from state to state.
Between clusters, control passes by a call.

The questions are:

- whether `.rr`'s states really fall into dense clusters;
- how often control would cross a cluster's edge;
- what decides the cut.

It measures them on the `.rr` corpus (1,362 files, 1.55M tokens) and sketches the shape.

## What was measured

`rr_state_clusters` (in `rr_parser`, run by hand) works on tier 2's words.

- **A state** is a `TOKEN` word. Its code is the words it reaches before the next `TOKEN`.
- **The traffic** is how often a token's state follows the one before. It comes from the
  interpreter over the corpus (`stream::run_seen`).
- **The clusters** are made by greedy merging: the two clusters with the most traffic between
  them merge first, while the merged code stays under a cap, counted in words. Tier 2 is about
  12 instructions a word.

The states:

- 227 states; a state's code is 30 words at the median, 152 at 90%, 311 at most;
- 10,061 distinct words, 14,933 counted per state (words shared by several states);
- 0.93 changes of state a token.

Cut by the corpus's traffic:

| Cap (words) | Clusters | Largest | Words, all clusters | Crossings a token |
|---|---|---|---|---|
| 300 | 130 | 311 | 13,725 | 0.343 |
| 600 | 120 | 600 | 14,165 | 0.231 |
| 1,200 | 110 | 1,198 | 13,485 | 0.064 |
| 2,500 | 116 | 2,496 | 13,378 | 0.005 |

At a cap of 1,200 words (about 14,000 instructions, 56 KB of code), one token in 16 leaves its
cluster. At 2,500 words it is one in 200.

The many clusters at each cap are mostly cold states: the corpus never passes between them,
so nothing merges them. They could be packed into a few cold functions with no cost to the
hot path.

Cut by the program alone, the crossings were then counted on the corpus:

| Weights | Cap 600 | Cap 1,200 | Cap 2,500 |
|---|---|---|---|
| The static graph (edges between states) | 0.570 | 0.769 | 0.668 |
| A walk taking each way equally often | 0.626 | 0.573 | 0.598 |

Both are ten times worse than the traffic's cut. The edges say which states meet, not how
often.

Compiling the grammar takes 2.4 s of CPU in a debug build. Cranelift takes 0.69 s on tier 2's
one function and 0.69 s on tier 1's.

Cranelift's own timings for tier 2 (`cranelift_codegen::timing`, its passes over tier 2's
function and its outlined helpers):

| pass | s |
|---|---|
| register allocation | 0.580 |
| emission | 0.123 |
| verifying the IR | 0.092 |
| lowering, CFG, dominators, the rest | 0.104 |
| all | 0.899 |

Register allocation is two thirds of the compile. It is the part that grows faster than the
function: its live ranges span the whole function, and a value live across the hot loop
competes with every state's values. A cluster's function has only its own.

## What it says

- **`.rr`'s states do fall into dense clusters.** About 14,000 instructions a cluster keeps all
  but one token in 16 inside it.
- **A call at each crossing is cheap at this rate.** 0.064 crossings a token, at a few
  nanoseconds each, is well under a nanosecond a token. Tier 2 takes about 34 ns a token
  today.
- **The cut needs a profile.** Neither static weighting comes close. The traffic of a sample
  corpus has to be measured when the grammar compiles: the ladder's documents, or the
  grammar's own test files.
- **Copies are the cost of the cut.** Words shared by states in different clusters are copied
  into each, a third more words at a cap of 1,200. A shared run could be its own small function
  instead (calls, not inlining).

## The shape

- **A cluster is a generated function.** It takes `ctx` and the place of the state it enters,
  and starts with a switch over its entry states. From there on it is today's code, with the
  run's state in memory, which costs nothing (doc 27).
- **A crossing is a tail call.** An arm's target state is known when the grammar compiles, so
  an arm that leaves its cluster tail-calls the target cluster (`return_call`, Cranelift's
  `tail` calling convention). The call is direct: no dispatch, no stack growth.
- **A `RETURN` goes to one of several places**, known only at run time. It tail-calls through a
  table of the clusters, or returns the place to a short driver loop. Returns are about 0.76
  words a token, but most land in the caller's own cluster.
- **Calls in the grammar stay as they are.** `PUSH` and `RETURN` keep their slots in the
  returns column, bounded by the tape's room, because a grammar's nesting depth is the input's.
  The native stack holds a cluster's own frame: its spills, and the outlined helpers' frames.
- **Register pressure is per cluster.** Each function is allocated alone, over the values its
  own states use.
- **Compiling is per cluster.** The clusters are independent functions, so they compile in
  parallel, each small enough to stay off Cranelift's superlinear curve.
- **Metal gets the same cut.** A cluster is a function of a smaller kernel, or a kernel of its
  own.

## Built and measured

Tier 2 can now be lowered in clusters (`native_cpu_clusters`, `native_cpu_stream::compile_cut`).
`rr_tier2_clusters` (in `rr_parser`, run by hand) measures it on `.rr`.

### The shape as built

- **The cut is of words, not states.** Each word is in one cluster, so no word is copied. A
  word shared by states of two clusters is a crossing only where control passes it.
- **A cluster is a function** in Cranelift's `tail` calling convention, taking `ctx` and the
  index of the place it enters at. Its first block picks that place (a jump table).
- **A jump to another cluster's word is a tail call** (`return_call`) with the word's index.
  No stack grows.
- **The state stays in memory**, in `tempo_stream`'s stack frame. Its address is in the pinned
  register (x21), which no cluster allocates. Passed as an argument, the allocator spilled it,
  and every access of the state first reloaded the address: 16K more loads.
- **One cluster is the old function**, unchanged: its state in its own frame, no driver.
- **The shape check still holds.** A cluster may tail-call only the clusters. Loops across
  clusters are checked as a function's are: from each place a cluster is entered at, the calls
  it reaches before a loop head; over all clusters, no cycle of those.
- **The cut** (`by_traffic`) merges the pairs of words with the most traffic first, under a
  cap in words. Words the profile never ran join a neighbour; the clusters left are packed
  into as few as hold them.

### The numbers

A profile of 41 files (every 34th: 0.26 s CPU in a debug build), measured over all 1,362.
Tier 2's compile alone; tier 2's time over the corpus with tier 1's buffers made before, each
form's best of five rounds, the forms in turn. Every file's table equals the one function's.

| | functions | crossings, of the corpus's steps | register allocation | instructions | tier 2 |
|---|---|---|---|---|---|
| one function | 1 | 0 | 0.147 s | 122,469 | 40.8 ms |
| cap 600 | 14 | 0.47% | 0.079 s | 139,740 | 44.5 ms |
| cap 1,200 | 7 | 0.05% | 0.085 s | 140,404 | 40.5 ms |
| cap 2,500 | 4 | 0.03% | 0.102 s | 139,012 | 37.6 ms |
| by the program alone, cap 1,200 | 7 | 17% | 0.079 s | 133,651 | 87.6 ms |
| by the program alone, cap 2,500 | 4 | 6.4% | 0.089 s | 133,158 | 55.9 ms |

- **Register allocation halves.** A cluster's live ranges are its own.
- **The whole compile does not shrink.** It was 0.39 s, and stays 0.37 to 0.45 s. IR building,
  the verifier and the shape checks are the rest, and do not depend on the cut.
- **The run is no slower, and at 2,500 words a little faster.** That held in four runs: 37 to
  38 ms against 40 to 41.
- **Spills are the same.** Stack loads and stores other than the state's: 15,600 in the one
  function, 15,400 to 16,700 in clusters.
- **The code is 14% larger.** Each crossing site is a tail call; each cluster reloads `ctx`'s
  fields at its entry and reads the pinned register once a block.
- **A crossing costs about 10 ns**: the callee-saved registers restored and saved again, and
  `ctx`'s fields loaded at the next cluster's entry. So the cut must come from a profile. The
  program's own edges cross 6 to 25% of the steps, and the run is 1.4 to 2.7 times slower.
- **A small profile is enough.** 11 files (90 ms of CPU) gave a cut at 2,500 words no slower
  than the one function.

### Compiling in parallel

The clusters are compiled on threads, one function each, largest first
(`native_cpu_codegen::define_functions`), then defined in the module in order. At cap 2,500
(4 functions), tier 2's compile is 0.40 to 0.44 s of CPU in 0.29 to 0.32 s of wall time,
against the one function's 0.39 to 0.41 s in the same wall time. The largest cluster bounds
the wall time; smaller caps spread better (cap 600: 0.51 s CPU in 0.31 s).

## Spills and the run

The clusters were meant to cut spills, and so to run faster. Three ways were tried, alone and
together, at caps from 150 to 2,500 words.

- **The cut's size.** Caps of 150, 400, 600, 1,200 and 2,500 words.
- **The state in registers** (`Cut::held`). The run's state fields become SSA variables in
  each function: loaded where it is entered, stored before each tail call. Three sets: the
  token's 8 fields, those and the tree's (14), all 23.
- **`ctx` at each word** (`Cut::reload`). Each word loads the `ctx` fields it reads where it
  runs, so none lives from the function's entry. Before, every cluster loaded about 30
  fields at entry, all live everywhere, and the allocator spilled them.

In clusters the pinned register holds `ctx`, and the state is inside it (`Ctx::state`): one
register for both.

Stack loads and stores (all functions), and tier 2's time over the corpus (best of 3 to 5,
load 15 to 30):

| | in memory | `ctx` at each word | the token's state held | all held |
|---|---|---|---|---|
| one function | 15.6K + state, 40.7 ms | 14.8K + state, 44.0 ms | 25.8K + state, 39.5 ms | 104K, 45.4 ms |
| cap 150 (53 functions) | 13.8K, 52.1 ms | 2.3K, 52.9 ms | 26.7K, 57.2 ms | 74K, 54.5 ms |
| cap 1,200 (7) | 15.8K, 40.1 ms | 4.4K, 41.2 ms | 32.0K, 39.8 ms | 88K, 45.3 ms |
| cap 2,500 (4) | 16.7K, 37.5 ms | 3.9K, 38.7 ms | 39.6K, 37.0 ms | 106K, 42.8 ms |

"+ state": the one function also keeps the state in its frame, 27,142 stack accesses that
are not spills.

- **`ctx` at each word removes three quarters of the spills** in clusters (16.7K to 3.9K at
  cap 2,500, 13.8K to 2.3K at cap 150), and the run is no faster.
- **The state held in registers spills more**, at every cap. In the dumps the variables live
  in stack slots around the 855 calls of outlined helpers and across joins. Within 150 words
  as within 2,500.
- **Small clusters are slower**: at cap 150, 2.6% of the steps cross, at about 10 ns each.

### Why fewer spills did not run faster

A spilled `ctx` field is a value that never changes. Its spill is one store at the entry and
one L1 load at each use: the same load as reading it from `ctx` at the word. Removing it
moves the load, not the cost.

What the run waits on, by the CPU's counters (Instruments' CPU Counters, over each form's
tier 2 alone, 80 rounds of the corpus): the share of the cycles the core spent in each state.

| | useful | delivery | discarded | processing |
|---|---|---|---|---|
| one function | 0.37 | 0.08 | 0.19 | 0.36 |
| cap 2,500 | 0.42 | 0.08 | 0.16 | 0.34 |
| cap 2,500, `ctx` at each word | 0.42 | 0.09 | 0.14 | 0.35 |

- **"Processing" (the back end waiting) is a third of the cycles**, with spills or without.
  It is the state's own round trips: a word stores a field and the next word loads it back,
  a dependency through memory each time.
- **"Discarded" (mispredicted work) is a sixth.** It is what the clusters improved: 0.19 to
  0.16, the 8% the cut gains. A token runs 11.7 words: 1.9 jumps, 1.7 marks, 1.6 switches on
  the token's kind, 1.0 spans, 0.9 leaves, 0.7 calls and 0.7 returns of the grammar.
- **A token is about 100 cycles**, 8.6 a word.

So the spills a split can remove are not on the run's path. What is on it is the mutable
state between words, and the branches.

### What would pay

- **The state's hot fields in registers, without spills.** This needs the functions to hold
  no calls on the hot path and few values at once. The outlined helpers (the span, the
  table) take arguments in caller-saved registers, so every value live across them goes to
  the stack. A split that leaves each hot loop free of calls, with its state as block
  parameters, is the shape. That is a different cut from the traffic's: around the loops of
  the token, not between them.
- **Fewer mispredicted branches.** The switches on a token's kind are the grammar's own
  choices. A cut that keeps each switch's hot arms together already helped; ordering the arms
  by the profile would help more.

## The product's cut

`.rr`'s parser is now compiled with tier 2 in clusters by default. The cut is at 2,500 words,
from the profile of 12 sample documents the parser ships with (`rr_parser/samples`, 84 KB):
copies of the repository's `.rr` files, from all its folders. They are named `.rr.sample`, so
nothing that walks the repository for `.rr` files counts them twice.

## A cut around the token's loops

The section above ends with a shape that should pay: the state's hot fields in registers along
each token's loop, with no calls on that loop, and the switches' arms ordered by the profile.
These are built (as `Cut` options) and measured here.

### What was built

- **A function a state** (`by_states`). Each `TOKEN` word and the words it reaches before the
  next `TOKEN` is one function: 227 functions, crossing at each change of state.
- **Fields as arguments** (`Cut::pass`). The tail calls pass four of the state's fields (next,
  start, end, need) as arguments. Cranelift's `tail` convention has six registers for
  arguments (x2 to x7); `ctx` and the entry's index take two.
- **No call on the token's path** (`Cut::inline_span`). `SPAN` found the token's end by calling
  a helper, about once a token. Its common case, a bit left in the mask's current word, is now
  one `ctz` in line. The helper searches past that word only.
- **The arms by the profile** (`Cut::pick`). A switch on a byte was a balanced search over its
  runs of bytes. Two other ways:
  - **weighted**: a run with half the profile's weight is tested first, with one compare;
    otherwise the split is at the weighted middle;
  - **a table**: one indirect branch over the 256 bytes.
- **Cranelift's optimizer** (`Cut::speed`, `opt_level = speed`). Every JIT in the compiler runs
  Cranelift at its default, `opt_level = none`: no dead code removed, no values numbered, no
  redundant loads removed. A load is pure to the optimizer only when it is read-only and
  `can_move`. `ctx`'s fields are now loaded so, and the unused ones go.

### The numbers

Over the corpus, the best of 5 to 7 rounds, load 10 to 50 (the one function moves from 40.6
to 43.7 ms with the load):

| form | functions | stack loads and stores | tier 2 |
|---|---|---|---|
| one function | 1 | 42.7K | 40.6 ms |
| one function, optimized | 1 | 38.2K | **37.2 ms** |
| cap 2,500 (the product's) | 4 | 16.7K | 37.7 ms |
| cap 2,500, optimized | 4 | 11.9K | 38.9 ms |
| cap 2,500, the token's state held, `ctx` at each word, `SPAN` in line, optimized | 4 | 31.1K | 38.4 ms |
| a function a state, optimized, `ctx` at each word, `SPAN` in line | 227 | 1.6K | 77.0 ms |
| the same, four fields passed as arguments | 227 | 14.4K | 79.4 ms |
| one function, arms weighted | 1 | 41.6K | 40.9 ms |
| one function, arms by a table | 1 | 46.8K | 72.1 ms |

- **A function a state has almost no spills, and runs twice as long.** 17.6% of the steps
  cross, 3.2M crossings over the corpus, at about 12 ns each: about 50 cycles.
- **What a crossing costs in Cranelift.** Each state function:
  - has a frame and signs its return address;
  - saves and restores nine callee-saved registers;
  - picks its entry by an indirect branch when it has more than one (120 of the 229).

  None of this is the state's work. With no calls but tail calls, nothing needs saving: a
  chain of tail calls never returns to the function that saved the registers. Cranelift's
  aarch64 ABI saves x19 to x28 in every function's prologue, whatever the convention.
- **Inlining `SPAN`'s common case** gains 1 to 3 ms, within the load's noise.
- **Weighting the arms changes nothing.** The arms' order was not what mispredicts. **A table
  mispredicts far more**: 1.8 times slower.
- **The optimizer gains 9% on the one function** (40.6 to 37.2 ms), and as much as the cut at
  2,500 words. On the clusters it gains nothing, measured so far.

### What it says

- **The shape needs crossings of a few cycles.** At 2 crossings a token and a token's 100
  cycles, the registers save a few cycles a step. A crossing has to cost less than that.
  That means:
  - functions without a frame or callee-saved registers;
  - one entry each, so a crossing is a direct branch.

  Cranelift offers neither for tail calls. Without them, the state's registers are lost to
  the crossings.
- **Within one function, the allocator spills the held state** wherever values meet, so
  holding it gains nothing either.
- **What was left to try** is measured in the next section: the crossing's cost was the
  entry's jump table, not the convention.

## Crossings made cheap, and the optimizer

### How others make tail calls cheap

- **In LLVM**, interpreters built of tail calls (CPython's tail-calling interpreter, upb)
  use `musttail` with the `preserve_none` convention. A function then saves no register for
  its caller, so a chain of tail calls saves nothing. CPython reports 9 to 15% from it.
- **Cranelift has no such convention.** Its `tail` convention had no callee-saved registers
  at first. They were added in 2024 (issue 6759, PR 8246), because Wasm code spilled more
  without them. But a function saves only the callee-saved registers it uses. The allocator
  takes x19 to x28 only when more than 16 values are live. So a function with little pressure
  saves nothing, in the stock convention: 95 of the 231 state functions save none, 72 save
  one.

### What a crossing really cost

A crossing was three things, each now a `Cut` option:

- **an indirect call** (`return_call_ind`): every call is long-range, because the JIT may map
  functions farther apart than a branch reaches (`near`: one arena, direct branches);
- **a signed return address** (`unsigned`: off);
- **an indirect branch at the entry**, a jump table on the entry's index (with a profile,
  `pick_traffic`: compares, the hottest entry first; a single entry is a jump).

On the function a state, alone (CPU time, best of 7):

| | tier 2 |
|---|---|
| none of the three | 72.2 ms |
| direct branches | 72.5 ms |
| no signing | 72.0 ms |
| entries picked by the profile | **41.2 ms** |
| all three | 41.4 ms |

The entry's jump table was the cost: an indirect branch whose target changes with each
crossing, so it mispredicts. The other two cost nothing measurable.

### The cut's size, crossings cheap

Instructions retired and cycles (`proc_pid_rusage`, the least of 7 rounds in turns) and CPU
time; all with `ctx` at each word, `SPAN` in line and the optimizer, except the first three:

| `.rr` tier 2 | instructions | cycles | CPU |
|---|---|---|---|
| one function | 596M | 156.1M | 36.4 ms |
| one function, optimized | 580M | **140.8M** | 32.8 ms |
| cap 2,500 (the product's) | 632M | 142.5M | 33.2 ms |
| cap 800, entries picked | 611M | 144.7M | 33.7 ms |
| cap 400, entries picked | 635M | 146.9M | 34.2 ms |
| cap 100 (80 functions), entries picked | 641M | 148.0M | 34.4 ms |
| a function a state (227), jump table entries | 675M | 278.2M | 64.9 ms |
| a function a state, entries picked | 666M | 150.6M | 35.1 ms |

- **With entries picked by the profile, every cut is within 7% of the one function's cycles**,
  from 4 functions to 227.
- **None is faster than the one function optimized.** More functions retire more
  instructions: each loads the fields it needs where it is entered.
- **Holding the state in registers still gains nothing.** With crossings cheap, the token's
  fields held (or passed as arguments) spill as before, and run no faster.
- **A token costs about 375 instructions and 91 cycles** (IPC 4.1). Its time is the
  instructions retired, not spills or crossings.

### The optimizer on tier 1, and on JSON

Tier 1 runs without the optimizer by decision 0069, which keeps its constants in its loop.
Measured again, in turns in one process:

| | instructions | cycles | CPU |
|---|---|---|---|
| JSON tier 1, off | 184.1M | 27.4M | 6.37 ms |
| JSON tier 1, on | 207.4M | 29.2M | 6.78 ms |
| JSONC tier 1, off | 242.1M | 34.6M | 8.05 ms |
| JSONC tier 1, on | 278.8M | 36.3M | 8.45 ms |
| `.rr` tier 1, off | 534.5M | 106.5M | 24.8 ms |
| `.rr` tier 1, on | 657.1M | 122.5M | 28.5 ms |
| JSON tier 2, off | 422.1M | 126.7M | 29.5 ms |
| JSON tier 2, on | 424.8M | 128.8M | 30.0 ms |
| JSONC tier 2, off | 432.1M | 101.7M | 23.7 ms |
| JSONC tier 2, on | 420.2M | 99.2M | 23.1 ms |

- **On tier 1, the optimizer costs 13 to 23% more instructions** and 5 to 15% more cycles: it
  hoists the splats and tables out of the loop, as decision 0069 says, and they spill. The
  decision holds.
- **On tier 2 it gains 10% of `.rr`'s cycles** (156.1M to 140.8M), and about nothing on JSON
  (−2% to +2%).

## Not measured yet

- **Cheaper crossings.** With `ctx` at each word, an entry loads nothing but the state.
- **The cut by grammar levels**, and a grammar much larger than `.rr`.
