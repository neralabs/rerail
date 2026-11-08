# Parser generators: the TDD plan for tier 2

Status: plan, being built (2026-11-07). It builds docs 14 to 16, with the corrections from
review (§0). Each phase writes its tests first and sees them fail. It builds until they pass,
commits, and only then starts the next phase.

## Intro

Tier 2 is built in seven phases, from the Bitlex file down to Metal. Each phase ends in code
that the next one reads, and its tests pin the shape of that code, not only its answers:

| phase | builds | pinned by |
|---|---|---|
| 1 | `before`, `after`, `also when` in Bitlex and the block program | the reference, every executor, `rir` |
| 2 | the monoids: combines, summaries, references | laws and sequential references |
| 3 | the analysis: classes, depths, candidates, fixes | JSON's table, as doc 16 §4.1 |
| 4 | the plan: steps, entry layout, budget, verifier | the printed plan, and refusals |
| 5 | the CPU: tier 2 lowered by `native_ir`, fused into tier 1's function | IR text; no call; tapes; cost |
| 6 | Metal: the generated plan kernel | tapes equal the CPU's; shape guards |
| 7 | timing | CPU time next to simdjson's stages |

The oracle for tapes is tree-sitter, as before. Rust references are oracles for the parts:
the monoids' sequential references and the carries'. None of them is an executor.

## 0. Corrections from review, built in

- **The scan phase has one barrier.** Every SIMD group scans the 32 SIMD totals itself, and
  the SIMD totals carry four candidates. So all depths are exchanged at once.
- **Entries go to device memory,** as structure of arrays: one array per 4-byte word, in
  candidate-major order. Pass A stages each tile's 32 entries in threadgroup memory, then
  writes each array's 32 words as one 128-byte segment.
- **Threadgroup memory is a budget, well under 32 KB,** for occupancy: 10,240 bytes, of which
  JSON uses 10,112.
- **No chunk limit.** A file of any size is one pass A, one scan, one pass B.

## 1. Bitlex and the block program (done)

**Built:** `before X` and `also when` in Bitlex. A node statement may run over several lines,
with `also` and `end` starting their own. Rows carry `before` and `also`, and `rir` prints them.
`json.bitlex`'s pair is found by `before ':'`, with the rule from the left as its `also`.

**Changed from doc 16 §3:** the block program keeps rules. `before` and `after` are not tier-1
ops. The analysis lowers the rows' conditions to values (phase 3). The one-byte kinds are not
tier-1 outputs either: as outputs, they took JSON's loop from 178 instructions to 274. Tier 2
reads them from the token's byte, as simdjson's stage 2 does. The punctuation row's keywords
say which byte is which kind.

**Tests:** `bitlex::syntax::before_and_also_parse`,
`rir::tests::block_tree_rows_before_and_also_round_trip`,
`bitlex_compile_tests::json_rows_read_kinds_as_designed`, and the tier-1 ratchets unchanged.

## 2. The monoids (done)

**Built:** `block_monoid`: `StateFn` (S1's 16-bit table, with an identity that keeps the
halos coming in), `Last` and `First` (forward and backward `last`), and `Dyck`. `add` and `or`
are integer operations.

**Changed from doc 14 §3:** the stack effect has a **peak**, 4 bits: the most levels above the
piece's start. `nested max 13` bounds the absolute depth, and a nesting inside a piece that
closes again before its end shows in no final count. A file fails when, from its start, the
effect has a mismatch, a close with nothing open, or a peak past 13. The effect is 41 bits.

**Tests** (`block_plan_tests`): `state_fn_laws`, `state_fn_composes_like_a_walk`, `dyck_laws`,
`dyck_equals_a_stack`, `last_and_first_equal_a_walk`. Breaking the peak's combine fails the two
Dyck tests.

**Moved to phase 3:** S1's summaries against tier 1's carries need the block program's carried
slice, which the analysis finds.

## 3. The analysis (done)

**Built:** `block_plan::analyse(program, target)`: every value with its class, monoid and depth,
the scans by depth, the candidates, the fixes and the passes over the bytes. Metal takes the
rule with fewer passes, and the CPU the one that runs forward. `Dataflow::text` prints it.

**Found by the analysis, against doc 16 §4.1:**

- **The stack is in S3, not S2.** The pair bit's event is the key, and the key reads the
  backward carry at the block's right edge. So the stack is one deeper than the key: S3, with
  the node counts. The chain is still three scans, and the scan phase still one exchange.
- **The key is two `last 1` backward values:** the token after the string's end is `:`, and
  that bit carried back to the string's start. Doc 16 used one, over tokens outside strings;
  the analysis has no notion of "outside strings", only rows.
- **Four nodes may start at one byte, so three planes.** `document`, `pair`, `string` and `"`
  can all start at the first token by the rows alone. Such a file fails the `also` check, but
  the planes must hold the count.
- **The counts are 14 bits.**

**Tests:** `json_values_are_classed_as_designed` (the whole text),
`json_rule_from_the_left_needs_a_third_pass` (S1, S2, S3 the stack, S4 the counts after a third
pass), `json_cpu_dataflow_is_forward`. S1's summaries against tier 1's carries move to phase 6,
where the generated code makes them.

## 4. The plan (done)

**Built:** `block_plan::plan(program, target)` and `Plan`: the steps (loops over tiles or
rounds, work, barriers), the entry's fields, the threadgroup stores and the budget.
`Plan::text` prints it; `verify` checks the budget, uniform loops, and a barrier between every
write and read of a shared store by different steps. The CPU plan is one pass.

How the plan is made:

- **The entry's fields** are the later scans' values, in the dataflow's order, then the fixes,
  the error and, matching by a scan, the open ords. A field wider than a word starts at one;
  no narrower field crosses one. JSON's candidate is 3 words, its entry 13.
- **Threadgroup stores:** staging and the exchange have two buffers, the opens one. While the
  stores are over the budget, the largest double store loses a buffer.
- **Staging** holds at most 16 words of each entry. A wider entry is staged and written out in
  slices, which take the two buffers in turn.
- **The exchange** is one step if all of an entry fits in 4 KB, both buffers. Otherwise it is
  two: S1 first, then the candidate S1 picked.
- **Barriers:** one where the verifier needs it. A loop gets a barrier at its body's end only if
  the next iteration would rewrite a store still being read.

**Changed from the plan above:**

- **Double buffers are a parity.** Each write to a double store takes the other buffer, and a
  read reads the one last written. The verifier tracks this, and so must the generated code.
  It is what lets `match scan`'s eight staging slices share one 4 KB store.
- **`match scan`'s exchange is single-buffered:** 121 words per entry is over the budget with
  two. It needs no extra barrier: the next round's S1 barrier comes before its rewrite.
- **The budget is 10,240 bytes,** not 9 KB. JSON uses 10,112.
- **`rir` does not print plans yet.** The plan is derived from the program each time.
- **The value names:** the backward key value prints as `r1 before ':'`.

**Tests** (`block_plan_tests`): `json_gpu_plan_prints_as_designed` (the whole text, 7
barriers), `json_cpu_plan_is_one_pass`, `plan_verifier_refuses_a_missing_barrier` (each of the
7 removed), `plan_verifier_refuses_simd_in_a_ragged_loop`, `json_entry_is_13_arrays_of_words`
(with the budget), `match_scan_has_no_mid_pass_barrier` (121 words, 2 scan barriers, none in
pass B).

## 5. The CPU: tier 2 through the compiler

Tier 2 is part of the transducer's block side program (doc 12 §3), so it goes where tier 1
goes: lowered by `native_ir`, defined by `native_cpu_block`, and on Metal by the same packing
and kernels. Nothing on the path is Rust: no call from generated code, no host step between
the tiers, no hand-built emitter. (A first build did all three, and is deleted.)

**5a. Tier 2's native IR.** `native_ir` lowers the rows, through the analysis (phase 3), to
tier-2 instructions in `BlockNative`:

- **states:** the rows' `after` and `parent` conditions take values from small sets. Each
  reachable pair (previous token's kind class, innermost open kind) is a state, as simdjson's
  `object_field` and `array_value` are labels;
- **per state and token kind,** straight-line actions: open a node (kind, field, supertypes
  as constants), push, close, check, go to a state;
- **the token's kind** from the byte, for one-byte keyword rows; from masks otherwise;
  multi-byte keywords as word compares in the IR;
- **validators are tier-1 masks:** a number's syntax is bit operations, a fallback bit, like
  every other refusal. No call.

**5b. Passes over that IR:** fold what a state decides; merge equal states; fixed-shape tokens
(a string's quote, content and quote) emitted from one event, their ends found in the masks.

**5c. One function, fused:** `native_cpu_block` defines tier 1 and tier 2 in one loop per
block. The masks stay in registers; the events are the token starts.

**5d. The tape:** the extract pass writes the v9np columns. `TapeImage::from_written` stays
only as the test's encoder of tree-sitter's side.

**Tests, first:**

- `json_tier2_ir_prints_as_designed`: JSON's tier-2 IR, as `rir` text, pinned whole.
- `json_tier2_states_are_simdjsons`: the states equal stage 2's labels.
- `json_block_function_has_no_call`: the compiled CLIF has no call, and one loop per block.
- `json_tapes_equal_tree_sitter_byte_for_byte` and the corpus, on the fused function.
- `json_tier2_cost_ratchet`: ns per block next to simdjson's stage 2, in CPU time.

## 6. Metal

**Builds:** `metal_tier2`: MSL for the plan, generated as one kernel: tier 1 per lane for a
carry-in, pass A with candidates and staging, the scan, pass B (B1, B2 spread, B3 with its
barrier), and the columns.

**Tests:**

- `json_tier2_msl_compiles`.
- `json_tier2_msl_has_the_plans_barriers`: the barriers in the MSL, in order, are the plan's.
- `json_tier2_msl_loops_are_uniform`: every loop with a SIMD call or a barrier has a uniform
  bound.
- `json_tier2_msl_threadgroup_bytes`: the declared threadgroup arrays equal the plan's budget.
- `monoids_on_metal_equal_rust`: the generated helpers, run on random values.
- `json_metal_tapes_equal_the_cpus`: test cases and the corpus.

## 7. Timing

CPU time per 64 bytes for tier 2 on the CPU and on Metal, next to simdjson's stage 2, with
the machine's load. Paired runs.

## Process

- Targeted test runs only, each under 2 minutes. The corpus once per phase, in the background.
- A commit per phase, after its tests pass. Pushed.
- A conformance table (plan item → `file:line`) at each report.
