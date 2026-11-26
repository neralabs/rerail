# 0420: A mask leaves its vector where it is read

Status: built (audited 2026-11-23). Decided (the user, 2026-11-18: "okay please try that. I would like the compiler to
do it automatically if possible", on relieving integer register pressure with vector or floating point registers).
Follows 0069, 0417 and 0418.

## Intro

**The problem.** JSONC's hot loop is short of integer registers. Its peak is 40 live integer
values against about 28 registers, and it had 8 stack accesses after 0418.

The measurement (the pressure profile and the loop's IR):

- **The function's 8 parameters** are live through the loop but read nowhere in it. The
  allocator keeps them on the stack at no cost inside the loop, so they are not the problem.
- **Masks taken out of their vectors early.** A mask is made in a vector register (`addp`) and
  taken out to an integer one (`extractlane`, a `fmov`) where it is made. Of JSONC's 31, 5 were
  taken out 46 to 505 IR instructions before their first reader, held in integer registers
  meanwhile.
- **Loop values idle through most of the body.** The block index, the input and output
  pointers and two running counts are first read 600 to 1,700 instructions into the loop.

Cranelift moves neither on its own: its mid-end is off (0069).

## The decision

**Each mask is taken out of its vector where it is read** (`sink_extracts`). After the
function is built, every `extractlane` to a word in the hot loop (the blocks the hot body
dominates) moves to just before its first reader in its block, or to the block's end when its
readers are in later blocks. This happens in every grammar, with no setting. It costs no
instruction: the vector holds the mask meanwhile, and it is live there anyway.

**Rejected: parking idle loop values in vector registers as `f64`.** It was built as an
automatic pass. It parked the idlest of the hot body's integer parameters, as many as the
vector registers left free at the loop's warm peak.

- The budget came out as none: the UTF-8 constants held since 0418 fill the vector registers.
- Forced, each parked value cost 1 to 3 instructions of moves, about what the stack spill it
  replaced costs.

| Parked | `14-json` | `15-jsonc` |
|---:|---|---|
| 0 | 475 / 1 | 645 / 7 |
| 1 | 476 / 1 | 648 / 7 |
| 2 | 480 / 3 | 651 / 8 |
| 3 | 482 / 3 | 650 / 5 |
| 4 | 484 / 3 | 652 / 5 |

A pass whose budget is always zero is dead code, so it is not kept. It would pay where values
live across calls: the low halves of `v8`–`v15` survive a call. That is the token loop's case,
which these steps are not.

## Requirements, a test each

- R1. In every ladder step's hot loop, each `extractlane` to a word comes just before its
  first reader (after any other taken out for the same reader), or ends its block
  (`a_mask_leaves_its_vector_where_it_is_read`). Turning the pass off fails it at `01-vrai`.
- R2. Tier 1 equals the interpreter:
  - the compiler suite;
  - the parser_tests guards;
  - the rr_parser suite (48);
  - the rr JSON set: 789 files, every token the same, and `the_loops_errors_are_the_masks`.

## What changed

| | Hot loop | Stack accesses |
|---|---:|---:|
| `14-json` after 0418 | 475 | 1 |
| `14-json` now | **475** | **1** |
| `15-jsonc` after 0418 | 647 | 8 |
| `15-jsonc` now | **644–645** | **6–7** |

**JSONC's count varies by one between runs.** The IR is the same each run except for the
heap addresses of tier 2's tables, which a call takes as constants. How many instructions
build an address depends on its bits, and that shifts the register allocation by about one
instruction. This was already so; the new order makes JSONC's allocation sensitive to it.
