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

## Not measured yet

- **The cut by grammar levels.** Each nested rule as its own cluster: static, and it may come
  close to the traffic's cut.
- **Register pressure per cluster.** `native_cpu_pressure`, run on each cluster's code once it
  is a function.
- **The time, the spills and the compile time** of a lowering that emits clusters.
- **A grammar much larger than `.rr`.**
