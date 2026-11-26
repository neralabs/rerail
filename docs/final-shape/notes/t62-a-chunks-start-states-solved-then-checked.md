# t62: A chunk's start states solved, then checked: tier 1 on the device without a carried state

Status: design, for review (2026-11-25). Nothing in it is built. Follows doc 23 (parser generators:
carrying forward only), t55 (the regions as one machine), 0654 (`speculate`), 0540 (bounded launches).

## Intro

**What the user asked** (2026-11-25): "generating GPU kernels from the Bitlex parser [...] a shape for the
lexer where we first infer token boundaries, in parallel, on the GPU by solving for possible start and end
states from a chunk in the middle without knowing yet what the state of the preceding chunk is. By proving
states impossible from the syntax, this should allow us to solve very many structures in parallel and check
rather than propagate the state."

**What tier 1 does on the device today** (doc 23, `metal_programs/src/block_serial.rs`). It runs a file in
segments, and no segment waits on another. Each part of the carried state is handled its own way:

- **The `prev`s and the escape carry** forget within a few bytes. A segment starts a few blocks early (its
  warm-up) and has them right.
- **The 1-bit scans** (toggles, fills) are made exact by levels. Each segment's 2-entry table is made, then
  a prefix runs over the file's segments.
- **A region group** (`rounds until settled`) never forgets. rr's group is taken as one carry of 7 states.
  Each segment runs once from each state, and a prefix composes the 8-entry maps (doc 23 §3.1).
- **What is left is guessed and checked.** A segment whose start differs from its predecessor's end runs
  again, and the run again moves on while its end differs.

Two things keep this from scaling to every grammar:

- **A run per state.** Kotlin's regions nest: braces 4 deep, templates that resume a string, block comments
  3 deep. t55 counts a few thousand reachable states. Running every segment from each is a thousand times
  the work.
- **The run again is serial.** On rr before §3.1 it was 16,011 segments over 237 launches. A launch count
  set by the data breaks 0540's bound.

**The idea.** Most states are impossible at most places. A segment that opens with `*/` and then a newline
cannot start in code. One that holds a newline before its first `"` cannot start inside a one-line string.
The grammar says which, so the device can drop those states before it runs anything. With the impossible
states dropped, almost every chunk has one possible start, and so one run. Joining the chunks is then a
check that each one's start is its predecessor's end, not a carry passed from one to the next.

## 1. What a chunk's state is

A chunk is the bytes of one SIMD group, a block (64 bytes) a lane: 1 KB at 16 lanes, 2 KB at 32. Its state at
its first byte has four parts. Only one of them is guessed:

| Part | How it is known | Guessed? |
|---|---|---|
| `prev`s, escape carry | they forget within a few bytes; a warm-up block has them right (doc 23 §2, §3.2) | no |
| 1-bit scans outside the region group | a 2-entry table a chunk, composed by a prefix (doc 23 §3) | no: exact, a table |
| **the region machine's state** (t55): the open region, the nest stack, a nesting region's depth | **solved**: §2, §3 | the possible states |
| scans that read the region group | follow from the region state | no |

So the guessed state is a state of t55's `RegionMachine`. That is one definition, from the grammar's
`Region` list, shared by the prover, tier 1's walk and these kernels. Its reachable set `R` comes with a
shortest file to each state (t55 step 2).

## 2. A state impossible from the syntax

A state `s` is **impossible before bytes `w`** when every file that is in `s` and then reads `w` is refused.
Dropping such a state changes no accepted file's output:

- if the file's true state there is another one, that state's run gives the answer;
- if its true state is `s`, the file is refused whatever the device does. The check (§4) then finds no run
  for that start, and refuses the file too, as the serial run would.

This is the soundness argument, and it needs no rerun.

Two sources of the facts, both made once a compile and kept with the grammar's answers (0414):

1. **Tier 1's own refusals, by running the machine.** From state `s` the bytes `w` are decided: the machine
   is deterministic, and its other inputs are local. So "`w` from `s` reaches a check that escapes"
   (`CheckThen::Escape`) is computed exactly, by running it. This needs no prover. For every byte-class
   n-gram `w` (n up to 3, over the block program's classes) it gives a bitset over `R`: **`allowed(w)`**, the
   states from which `w` is not refused.
2. **The grammar's syntax past tier 1** (tier 2's refusals: token pairs, 0646). "No accepted stream has `s`
   and then `w`" is a question for the prover. It is stated as `speculate` (0654) where it is not proved: a
   false one costs a refused file and the fallback, never a wrong tape. These are added only if step 1's
   measurement shows that tier 1's refusals leave too many states.

`allowed` also gives the **synchronizing n-grams**: those after which every allowed state reaches one state.
A lane whose block holds one knows its state from there on, whatever came before.

## 3. The kernels, generated from the grammar's block program

Four launches a batch, whatever its files (0540). Each kernel is UKL, generated where `block_program.rs`
generates today's (`emit`), so it runs on CUDA, Metal and UKL's CPU backend.

**K1, solve: a lane's map over its possible starts.** Each lane holds one block.

1. Its candidates are `R ∩ allowed(w)` for its first n-grams. They are narrowed by each n-gram the block holds
   where the machine's state is fixed (a synchronizing one). This is a table lookup a block, with no run.
2. The lane runs its block once from each candidate. It uses today's body with the region group's carry set
   at the first byte, as doc 23 §3.1 does. A run that refuses drops its candidate. Runs that reach the same
   state merge.
3. It writes its map, `start → end`, at most `M` entries (`M = 8`), and the block's output words for each
   surviving start.

   - A lane with one survivor writes its outputs where they go.
   - A lane with more keeps up to `M` sets of output words. Each set is a derived fact in the batch's block
     (0353), not a copy.
   - A lane past `M` is marked **wide**. It is run in K4.

**K2, compose within a group.** The 16 or 32 lanes' maps are composed by a SIMD-group prefix, and the
composition of maps is associative. This gives each lane its map from the chunk's start. Where every lane
had one survivor, this is a compare of each lane's start with the lane before's end.

**K3, compose the chunks.** A file's chunks' maps are composed the same way, a group a run of up to 1,024
chunks (and one level more past that). Each chunk now has its exact start state. A chunk whose start has no
entry in its map refuses the file (§2).

**K4, select and write.** Each lane takes its exact start:

- one survivor: its outputs are written already;
- several: it writes the survivor's set;
- wide: it runs its block once from its exact start.

Then it makes the extracts' positions and totals (doc 23 §2 step 3). This is one run of one block, not a run
again that moves on: K3 made every start exact first.

| | doc 23 today | this design |
|---|---|---|
| runs of a block | 1 + one per region state (§3.1) + the runs again | the block's surviving starts (§2), typically 1 |
| launches | levels + checks, set by the data (59–237 on rr) | 4 |
| a wrong guess | a serial run again, file by file | none is made: every start is checked exact by K3 |
| grammars | those whose region group has few states | any whose `allowed` keeps lanes under `M` |

## 4. Check, not propagate

The state goes nowhere. Each lane solves its own block for every start the syntax allows. K2 and K3 only
check that each start is its predecessor's end and choose, by a prefix over small maps. Nothing is run again
in order.

## 5. What it needs first

- **t55 steps 1–2**: `RegionMachine` from the `Region` list, equal to tier 1's walk on the corpora, and its
  reachable set. No branch has them.
- **The machine's state as a number** the kernels carry: an index into `R`, with the machine's transition as
  a table of `R` × the block program's classes. For rr that is 7 × 30. For Kotlin, a few thousand states:
  the table's size is step 1's measurement.

## Steps, each with its tests

1. **Measure before building** (CPU, Rust, from tier 1's walk). On the Kotlin, rr and JSON corpora, at
   every 64-byte block and every 1 KB chunk, record:
   - `|R|`;
   - the candidates left by `allowed` for n = 1, 2, 3;
   - the survivors after the block's own refusals;
   - the bytes to convergence.

   **Bar:** median survivors 1, the 99th percentile ≤ 4, wide lanes under 1 in 1,000. If tier 1's
   refusals alone miss it, measure again with tier 2's token-pair facts (§2.2). If both miss it, the design
   does not hold for that grammar, and it is reported so.
2. **t55 steps 1–2**, with their tests (`RegionMachine` equals the walk; each shortest file ends in its
   state).
3. **`allowed` and the synchronizing n-grams**, made once a compile. Tests:
   - every dropped `(s, w)`, run as `s`'s shortest file then `w`, is refused by tier 1;
   - every kept one is not refused within `w`.
4. **K1–K4 in UKL**, generated from the block program. Tests: on every corpus file, the outputs, positions
   and totals equal tier 1 on Cranelift, with 4 launches in each batch, on CUDA and UKL's CPU backend (Metal
   once its runs are allowed again). Two more tests:
   - a file tier 1 refuses is refused;
   - a false fact injected into `allowed` refuses its file and never makes a wrong tape.
5. **Then doc 23 §3.1's run per state and the run-again loop are removed** for every grammar K1–K4 runs.
   They are not kept beside it.

## Open questions

- **Is `M = 8` enough on Kotlin's nests?** A block deep inside a template inside a string, with no
  synchronizing n-gram, may keep the nest stack's depth open. Step 1 answers it.
- **Stale scan values** (doc 23 §2: "inside a number" through kilobytes of strings). Two starts that differ
  only in a value no output reads should merge. Merging by what is read, not by the whole state, is the rule
  this needs.
