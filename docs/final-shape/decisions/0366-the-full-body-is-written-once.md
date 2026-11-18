# 0366: The full body is written once

Status: decided (the user, 2026-11-17: "just fix the shape of the generated code"). Follows 0363.

## Intro

**The problem.** A full block's body in tier 1 was written twice. The plan's `ascii_branch`
(`native_cpu_lanes`) said some byte test is known on an all-ASCII block (a UTF-8 lead byte, a
continuation byte). Then `step` tested the block's bytes (`all_ascii`) and branched to one of two
whole bodies:

- an ASCII twin, with those tests folded to constants;
- the general one.

Both ways met after the body. In JSON's tier 1 read as Rust (`clif_rust`), the pass function
`f0` was 4,886 lines, and most of it was the body twice.

simdjson writes its body once. Its ASCII test guards only the UTF-8 checker:

```cpp
// simdjson's json_structural_indexer::next, arm64
if (likely(in.is_ascii())) {
  // nothing to check: the error stays as it was
} else {
  check_utf8(in);   // the lookups, the carried lead bits
}
```

## The decision

The body is one. What an all-ASCII block knows sits behind one branch inside it
(`native_cpu_block::ascii_split`):

1. **What is known.** `ascii_known` gives each value's word on such a block, where the plan knows
   it: a byte test on the vectors (`Plan::ascii`), and the bit logic of known words.
2. **The values inside.** These are the live values so known, apart from constants, values known
   in every full block, region members and fixed points' members.
3. **The exits.** An exit is a value inside that something outside reads as a word, or that is
   an output, a carry or a flag.
4. **The branch.** First, what the inside values read is made, once for both ways. Then
   `brif(all_ascii, fast, slow)`:
   - `fast` passes each exit's constant word;
   - `slow` makes the exits from a copy of the emitter.

   The two ways meet in one block whose parameters are the exits' words. The rest of the body
   follows once.

**A reader on the vectors.** A byte-local value is made from its operands' vectors in each of
its readers (`Lanes`). So a reader past the branch would make the known values again, from the
bytes, on every block. Such a reader of bit logic (`not`, `and`, `andnot`, `or`, `xor`) is made
from words instead (`Emit::as_words`), and so is any reader of bit logic that reads it. Their
other vector operands are made masks. A reader of another kind (a shift, a byte test) still makes
its known operands from the bytes: the values are the same, without the saving.

No ASCII split is made where nothing leaves it. Then `ascii_branch` is false, and the body has
no ASCII test. The twin's fold in `Lanes` (`Lanes::ascii`, `At::ascii`) is gone.

## What changed in JSON's tier 1, read as Rust

| `f0`, the pass function | Before (0363) | Now |
|---|---:|---:|
| lines | 4,886 | 3,418 |
| `vqtbl1q_u8` table lookups | 154 | 82 |

The UTF-8 checks are one block behind `if all_ascii { constants } else { checks }`, 9 words
leaving it, as simdjson's.

## Found on the way, fixed

- **The ladder's tier 2 jump tables.** The ladder counted `br_table`s in the CLIF text, taken
  before `switch_trees` (0322) turns a small one into compares. It now counts the tables left
  as compiled (`CompiledStream::tables`).
- **The ladder's tier 1 branches.** 0353's toggle has a cold branch for escaped quotes
  (`toggle`), which the ladder's count did not include. `PassShape::rare_branches` counts them:
  the toggles' cold blocks in the round's function as built. `forward` builds the pass again
  until no region's call costs the hot loop, so the list is cleared each round.
- **A pass with no block loop.** A pass like this (0177's `bodiless_body`) has its body's blocks
  cut, so its IR has no `block2`. `clif_text::hot_loop` panicked; now it gives no blocks, and the
  ladder skips that pass's body count.
- **Counts of the body's region calls.** `outline_tests`, and `json_gated_regions_equal_the_interpreter`,
  counted one per body copy. They now count one.

Not fixed: the `.rr` ladder's field ids lose 64 in the interpreters and the compiled parser
alike (work queue Q69). The fault is in the program they share, and this change does not touch it.

## Tests

- `rr_parser`:
  - the JSON equality set (`json`, `the_pairs`, `the_marks_tests`, `the_cpus_records`,
    `a_short_run`, `the_next_start`, `the_run_loop_calls_nothing`, `a_numbers_checks`,
    `rrs_records`, `the_loop_reads_no`), with `RERAIL_JSON_CORPUS` and `RERAIL_JSON_SUITE`;
  - `json_tier1_has_no_label_switch`.
- `parser_tests`:
  - `lanes_equal_the_interpreter`, a high byte's test folded behind the split, at each block
    edge;
  - `json_tier1_vectors_equal_the_interpreter`, with non-ASCII sequences around the first two
    block edges;
  - `json_gated_regions_equal_the_interpreter`;
  - the `json`, `loop_keeps_values`, `lowers_to` and `equal_the` guards.
- `compiler`: `the_general_body_is_a_call_not_a_copy`,
  `ladder_tests::every_step_parses_as_the_reference`, `token_local`.
