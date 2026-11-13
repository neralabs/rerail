# 0069: Compiled languages keep simdjson's shape, locked by tests

Status: accepted (user, 2026-11-07: "We should be able to get approximately SIMDJSON performance
on the CPU, since we should generate nearly identical code … ensure that the correct shape is
locked in from now on"; the plan, "yes, please go ahead with it, and make sure that you lock in
the improvements with tdd")

## Intro

A Bitlex file compiles to a block program: bit operations per 64-byte block, with a carry where
a value depends on earlier blocks. simdjson's stage 1 is the model: a few carries, every value in
a register, one bitmask per byte class. Hand-written stage 1 already compiles to code within
1.3× of simdjson (31.3 against 24.0 ns per block). The whole JSON lexer did not: 579 operations,
146 of them carried, 195 ns per block against simdjson's full parse at 25.6. Most of the carries
came from the compiler, not from JSON.

This record fixes what a compiled program may contain, and the tests that hold it there.

```
before: next(x) -> every value late -> a prev to align each operand, each output, each error
after:  each value at its own delay; outputs and fallback masks never aligned;
        byte classes folded; within at a delay a leaf
```

## Prior work consulted

- **ast-index-spike indexes:** the spike ran no block programs. Its rule of measuring against
  the real reference holds: simdjson's stage 1 and full parse, timed on this machine, on the same
  files (`fixtures/simdjson-stage1/stage1_indexes.cpp`).
- **Lessons ledger #28** (proxies are not the cost; only end-to-end CPU time counts). **Agrees,
  with a caveat:** the guards are structural (operations, carried words, violations), because
  timings on a loaded machine cannot gate a build. Each benchmark reports CPU time next to
  simdjson's.
- **Lessons ledger #10** (measure the code that runs). The guards read the compiled program and
  Cranelift's machine code, not a model of them.
- **Doc 12 §2.3–§2.4** ("the mask passes are loop-free, branch-free bit operations … fused per
  64-byte block"; "the cost per block, next to simdjson's"). **Agrees**, and extends the check
  from stage 1 to every compiled language.

## Decision

1. **No alignment carries.**
   - Each output keeps its own delay (`output %v delay d`). The host shifts each output by its
     own delay. The program's `delay` is only how far past the stream the executors run.
   - The fallback is one output per delay its masks have (`Compiled::fallbacks`). A mask is
     never brought to another delay, because the fallback asks only whether any bit is set.
2. **`within` at a delay is a leaf** (`within d`: bits `d <= i < len + d`), computed from the
   position on every executor. It is never a `prev` of `within`.
3. **Byte classes fold.** A value computed from bytes alone keeps its byte set, and combining
   two such values gives one class: one lookup per table, one bitmask.
4. **`prev` folds.** `prev(prev(x, a), b)` is `prev(x, a + b)`, on `x`'s carried word.
5. **The guards** (`json_compiles_to_simdjsons_shape`, now in `rerail_parser_tests::compile_tests`):
   - **No violations, for every compiled program:**
     - no output is a `prev`;
     - no `prev` of `within`;
     - no `prev` of a `prev`;
     - no `eq` combined with a class;
     - no two classes of one table combined.
   - **JSON's budgets are ratchets.** Operations and carried words (one per value a `prev`
     reads, plus one per other carried operation) may only go down. A change that lowers them
     lowers the constants in the same commit.
6. **Cranelift's loop keeps simdjson's machine shape**
   (`block_tests::stage1_loop_keeps_values_in_registers`,
   `json_loop_keeps_values_in_registers`, the same file):
   - **One carried word per `prev` base**, shared by every distance, as simdjson's one
     `prev_in_string` word. The body's block parameters are exactly the carried words plus one
     count per extract.
   - **A class test is a scalar `not`.** Each lane is compared with zero, then the 64-bit
     bitmask is inverted. No vector `mvn` is in the loop.
   - **Constants are made in the body**, where they are used: `movi` for a splat, a
     literal-pool load for a table. Hoisted out of the loop, more of them than the vector
     registers hold would be spilled. For the same reason the mid-end stays off, because its
     code motion hoists them.
   - **Outputs are stored through one pointer.** Block `i`'s words sit together
     (`out[i * outputs + k]`), each store at an immediate offset.
   - **The first block's zero carries are made per file**, not once for the batch, so they are
     not kept on the stack across files.
   - **Stage 1 has no stack loads or stores in its loop.** JSON's loop instructions and stack
     operations are ratchets.

## Results so far

| | before | after steps 1–4 |
|---|---:|---:|
| operations | 579 | 527 |
| `prev`s | 114 | 62 |
| carried words | 129 | 90 |

Cranelift's loop, from the machine code (aarch64):

| loop | before | after step 6 |
|---|---:|---:|
| stage 1: instructions | 359 | 351 |
| stage 1: stack loads and stores | 12 | 0 |
| JSON: instructions | 2314 | 2163 |
| JSON: stack loads and stores | 692 | 510 |
| vector `mvn` (both) | many | 0 |

JSON's remaining stack traffic is its 90 carried words and their live values, more than the 31
registers hold. That is the language's logic, the next item.

Every executor's outputs still equal the evaluator's. The data-layer corpus still gives 771 of
774 tapes equal to tree-sitter's.

## Next, under the same guards

- Fewer carried words in the language's own logic, until they fit the registers.
