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
| 5 | the CPU: a generated Cranelift tier 2 | tapes equal tree-sitter's; shape guards |
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
- **Threadgroup memory is a budget, well under 32 KB,** for occupancy: about 9 KB for JSON.
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

## 4. The plan

**Builds:** `block_plan::Plan`: steps (`pass bytes`, `scan`, `pass tape`, `barrier`), the
entry layout as arrays, the threadgroup budget; `rir` prints it; `verify` checks barriers,
uniform loops and the budget. The CPU plan is one pass.

**Tests:**

- `json_gpu_plan_prints_as_designed`: the whole printed plan, as text.
- `json_cpu_plan_is_one_pass`.
- `plan_verifier_refuses_a_missing_barrier`: for each barrier of JSON's plan, removed.
- `plan_verifier_refuses_simd_in_a_ragged_loop`.
- `json_entry_is_13_arrays_of_words`: 52 bytes, candidate-major.
- `json_threadgroup_memory_within_budget`: at most 9 KB.
- `match_scan_has_no_mid_pass_barrier`.

## 5. The CPU

**Builds:** `native_cpu_tier2`: a Cranelift function generated from the plan and the rows,
over the masks tier 1 writes, as simdjson's stage 2 reads stage 1's indexes. One case per
token kind. Its columns are packed by the tape's one packer (`TapeImage::from_written`), the
format's encoder, not a rule.

**Tests:**

- `json_tapes_equal_tree_sitter_byte_for_byte`, un-ignored.
- `json_corpus_tapes_equal_tree_sitter`, un-ignored, run once at the phase's end.
- `json_tier2_has_one_case_per_token_kind`: the CLIF's `br_table` has as many arms as kinds.
- `json_tier2_reads_no_rows`: the function's arguments are masks, bytes and columns; nothing
  of the program's tables is reachable from it.
- `json_tier2_calls_nothing_per_token` except the built-in validator.

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
