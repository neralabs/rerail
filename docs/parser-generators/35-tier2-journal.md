# Tier 2's code shape: a journal

## Intro

Tier 2 is the `.rr` parser compiled from the Bitlex grammar,
a function a grammar rule, lowered through Cranelift.
The aim is the code the lean Rust parser (`rr_syntax/src/lean.rs`) compiles to:
the same structure, within a margin.
Each step is judged by the machine code against rustc's,
not by the benchmark, which is reported alongside.

This page keeps three things:

- **The best to date**: the structure tier 2 has now, and its numbers.
- **Tried and dropped**: each approach measured and not kept,
  with what the code showed, and what could make it worth trying again.
- **The journal**: each step, in order, with its numbers and its commit.

Pages 33 and 34 explain each step at length.

How the numbers are taken:

- Machine instructions, functions, stack reloads and compares
  come from tier 2's code for `.rr`, disassembled.
- A stack reload is a load from `[sp]` that is not an epilogue's restore.
- The benchmark is tier 2 and the lean parser over the 1,360 corpus files:
  instructions and cycles, on a shared machine (its load is noted).

## The best to date

Tier 2 at `2b46b2e8b`: the best in shape and in the benchmark.

| | Tier 2 | Lean parser |
|---|---|---|
| Machine instructions | 26,575 | 10,008 |
| Functions for rules | 56 | 45 |
| Words | 6,441 | |
| A rule call | `mov`, `bl`, `cbnz` | `mov`, `bl`, a load |
| Stores | 737 | |
| Stack reloads | 10 | 0 |
| Compares | 3,972 | 1,198 |
| Benchmark, instructions | 331.1M | 356.4M |
| Benchmark, cycles | 64.0M | 68.3M |

The biggest gap left is the switches: 36% of the machine code, 28 instructions each.

Its structure:

- **A function a grammar rule**, and a call a `PUSH`, as the lean parser's methods.
- **`ctx` in a register of its own** (Cranelift's pinned register, x21) for the whole run,
  as rustc keeps `self`. No call passes it.
- **The state in `ctx`**, read and written where used, as `self`'s fields.
  Nothing is held in a register across a call.
  Within a function, a pass forwards fields along straight-line code.
- **Words as calls**: `TOKEN`, `LEAF` and the node words are functions
  every place calls, one body each. Only the main loop is inlined.
- **Words that behave the same are one word**, across rules too.
  A rule goes into an earlier rule's code by a call whose result it passes on,
  as the lean parser's `return self.helper()`.
  The verifier refuses a jump into a later rule.
- **One exit a function**, one epilogue, as rustc's.
- **A rule's result**: 0 for every return, the run's end with its top bit flipped:
  one `cbnz` a call.
  A callee takes its caller's token at `RETURN m=0` (every call returns to one `TOKEN`, alike),
  so every return lands on the caller's switch.
  Where that does not hold, a call of a place with one way tests once, else twice.
- **The calls open are the machine stack's**, as the lean parser's.
  No `RETURN` runs with no call open, and no `ACCEPT` within one (proved, not checked).
  The depth is one stack check where each rule function is entered.
  `TOKEN`'s room for the nodes returns write is bounded by the stack's depth
  over the smallest compiled frame.
  Where the proofs fail, the calls are counted in `ctx` as before.
- **The word and flags are callee-saved**: a function whose own words change them
  gives back the values it was entered with at its one exit.
- **The stream's end is a token kind of its own**, so a switch tests it in its compare tree.
- **Switches are compare trees**, as rustc compiles `match`: no jump table.

## Tried and dropped

Each line gives what the machine code showed, and what could change the verdict.

| Approach | What it showed | Worth trying again when |
|---|---|---|
| A jump table on the token's kind | Tier 2 from 146M to 258M cycles: the indirect branch mispredicts | Never as a table; rustc does not make one either |
| Jumps resolved in the lowering, before Cranelift | Larger code (594,132 bytes, from 590,312), 500M instructions from 484M | Cranelift stops folding jump-only blocks itself |
| The state as SSA values through each function | 691M to 908M instructions: each call stores and reloads them, joins spill them | Functions are small enough that joins do not spill |
| Ten state fields held in registers through a function | 706 KB of code, 525M instructions: stack and a return area at calls | Fewer fields live at once |
| Four fields passed to calls and given back | 21,725 stack accesses: they spill between blocks of large functions | Functions are small, or the allocator has no cliff |
| A callee taking its caller's token at `RETURN m=0` | `ctx` kept on the stack in many functions: 1,780 stack accesses, 49,440 instructions | Retried with `ctx` pinned: no spill, kept (`2b46b2e8b`) |
| Shared code as tail calls, `ctx` an argument | Nothing saved: an epilogue (six `ldp`s and an `add`) at each of 587 places | Epilogues get cheaper (fewer callee-saved registers) |
| Shared code as calls, `ctx` an argument | `ctx` reloaded at each use in the largest functions: 5,700 stack loads | Solved by pinning `ctx` |
| The calls open as an argument, with shared code | One value given to every call: split twice, then reloaded at each use (1,148 loads) | The count is needed by fewer calls, or not at all |
| The calls open in the entry index's high bits | The optimizer rebuilt it from the argument, which spilled instead (1,674 loads) | As above |
| The calls open as a passed field, given back with the result | Wrong tapes (no file matched), more stack stores (929) | Never debugged: may be worth fixing |
| A helper function for each set of rules sharing code | 84 functions, not 56, and 1,700 more instructions; three loop values spilled | Helpers are small and called from few places |
| Tail calls into shared code, `ctx` pinned | An epilogue at each of 198 places: 785 restores, against 170 | Epilogues get cheaper |
| One tail-call block a function gone to, the index its argument | The index constants folded into values live through the function, and spilled | Each place's own block is built; no reason to go back |

## The journal

Numbers are tier 2's for `.rr`. Each step's commit is listed.

| Step | Commit | Machine instructions | Benchmark instructions (cycles) |
|---|---|---|---|
| A function a grammar rule (page 33) | | 148,533 | 484.2M |
| Keywords in the switch | | (530,772 bytes) | 481.3M |
| The code laid out as the lean parser's | `d7f414abb` | 135,201 | 497.5M (105M) |
| Calls as the lean parser's: `ctx` as self, results 0 or 1 | `ca98b7188` | 159,283 | 441.0M (87.6M) |
| The tape and the state as the lean parser's | `bbbb03696` | 100,134 | 351.7M (71.2M) |
| The state as locals | `0d43c099e` | 91,391 | 347.8M (76.5M) |
| A token taken for good | `cc1e0660e` | 87,152 | 339.0M (74.3M) |
| Words as calls | `fded73721` | 43,654 | 354.2M (72.3M) |
| A rule that returns one way gives back 0 | `1a876e2a3` | 42,786 | |
| The calls open as an argument; the end a kind | `43e3f975d` | 41,084 | |
| Callees shared across rules | `8fea9d7b8` | 38,750 | 335.1M (63.9M) |
| `ctx` in a register of its own | `1fa44aff2` | 36,258 | |
| Code shared from the middle of rules; the count back in `ctx` | `de6f313fb` | 31,702 | 361.0M (76.5M) |
| The machine's stack counts the calls | `2b46b2e8b` | 28,962 | 350.4M (74.7M) |
| The word and flags callee-saved | `2b46b2e8b` | 27,433 | |
| One way by the place a call enters | `2b46b2e8b` | 27,314 | |
| The callee takes its caller's token: one `cbnz` a call | `2b46b2e8b` | 26,575 | 331.1M (64.0M) |
