# 0381: The block body, read against simdjson's stage 1

Numbered 0353 where it was written (Parser-generators); 0353 is "blocks are never copied" on the hub.

Status: decided (the user, 2026-11-17: "go ahead with 2 then 1", item 2: get the block body on
plain bytes down toward simdjson stage 1, and `json_loop_keeps_values_in_registers` back under
its first budget, 690 instructions and 4 stack accesses). Follows 0349–0351.

## Intro

**The problem.** The JSON program's hot loop (parser_tests' guard, no token loop) was 704
instructions and 10 stack loads or stores, over its first budget of 690 and 4. Read beside
`rerail-json-lean`'s stage 1 (simdjson translated, 0343), the body did work stage 1 does not:

- each class was `and`, `cmeq #0` on four vectors, then used complemented: simdjson's
  `classify` is one `vtst` (`cmtst`) a vector;
- the escaped quotes' correction (six shift-XOR rounds) ran every block, though it is zero
  but where a quote is escaped;
- the bitmask's weights and the toggle's all-ones were built from pieces, and the multiply's
  operand moved to the general registers and back;
- a class complemented for one reader was complemented again for another (`bnot`, then
  `band_not`), one more value live;
- a total (a file's refusal flag) was held from where the body made it to the step's end, and
  spilled, to be OR-ed into the totals' running word there.

## The decision

Each of these goes, in the generator or in the vendored Cranelift's aarch64 lowering:

| Change | Where | The guard's loop |
|---|---|---|
| The escaped quotes' rounds behind a cold branch (`toggle`) | `native_cpu_block` | 704 → 699 |
| A 128-bit constant replicated from 64 or 32 bits is a `movi`/`dup`, not a pool load; `clmul x, -1` is `pmull` with an all-ones vector, the operand read in place | vendored `inst.isle`, `lower.isle` | → 698 |
| The body's constants made at its head in every loop (0350 had it where a token loop runs) | `native_cpu_block` | → 696, 6 stack |
| `icmp ne (band x m), 0` on a vector is one `cmtst` (`VecALUOp::Cmtst`); `BIT_TEST` on by default where the vendored Cranelift builds for aarch64 | vendored, `native_cpu_lanes` | → 672 |
| No complement of a complement: `band_not`/`bor_not`/`bxor_not` of a `bnot y` read `y` (`fold_nots`, JIT only, as `remat_consts`) | `native_cpu_codegen` | → 667 |
| A total is stored where the body makes it, and read back from its row at the step's end | `native_cpu_block` (`step`) | → 4 stack |

**667 instructions and 4 stack accesses**: under the first budget. The guard's constants are
now 667 and 4 (a ratchet). Stage 1's loop (`stage1_loop_keeps_values_in_registers`) went from
603 to 558 instructions, its stack budget from 9 to 3.

**Tried and dropped: totals folded into the running word in the body.** OR-ing each total into
the running word where it is made (an accumulator in `Emit`) gave 699 and 8: the running word
was then live through the body in the total's place. Storing the total and loading it back at
the end costs one load from L1 and holds nothing.

**Tried and dropped: the mid-end's remat rule.** At `opt_level = none` (0069) the ISLE
mid-end does not run; `remat_consts` (0351) is the pass that does.

## Measured

`json_tier1_by_pass`, instructions a byte (load 55–76; cycles are noise at that load):

| Input | 0351 | 0381 | simdjson |
|---|---:|---:|---:|
| corpus | 20.9 | 20.26 | 2.96 |
| white space | 4.70 | 4.30 | 2.15 |
| long ASCII strings | — | 4.36 | 2.15 |
| numbers | 43.8 | 43.07 | 4.51 |
| short strings | 50.3 | 50.50 | 4.51 |

The plain-byte inputs moved; numbers and short strings are the token loop's (item 1).

## Tests

- parser_tests: `json_loop_keeps_values_in_registers` (667, 4),
  `stage1_loop_keeps_values_in_registers` (558, 3), `*_lowers_to_*` (a branch to a cold block is
  not on the loop's path; `clmul` a bit operation), `json_state_is_bits`, the `equal_the`
  tests (interpreter, gated regions, every executor, simdjson).
- rr_parser: `jsons_cpu_records_are_the_interpreters` (789 files, 1,165,507 tokens the same),
  `jsons_tier2_split_is_tier1s`, `json_split_stores_only_what_rows_are_read_for`,
  `rrs_records_are_rows_on_the_cpu`, `the_loop_reads_no_record_back`,
  `json_loop_against_step_14` (129 the same, 216 refused by both, none differ).
- rerail-clif: its 9 tests.
- vendored: an encoding test for `cmtst` beside `cmeq`'s (`emit_tests.rs`; the vendored crate is
  outside the workspace, so it runs with the vendored crate's own tests, not here).
- `RERAIL_LOOP_PRESSURE` and `RERAIL_DUMP_LOOP_CLIF` print the guard loop's pressure and IR.
