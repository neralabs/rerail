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

## 1. Bitlex and the block program

**Builds:** `before X` and `after X` in tree conditions, and `also when` for the other rule.
`BlockOp::After` (forward, a 1-bit carry, every tier-1 executor). `BlockOp::Before` (backward,
tier 2 only: no tier-1 output may depend on one). Rows that read masks: one-byte kinds become
masks, and the pair's start becomes `opening & before(U, ':')`.

**Tests, red first:**

- `bitlex::syntax`: `when before ':'` and `also when after ('{' | ',') and parent object`
  parse.
- `block_tests::after_and_before_equal_their_references`: random words and carries, against
  a byte-at-a-time definition.
- `rir_tests`: `after` and `before` round-trip.
- `block_tests::after_runs_on_every_executor`: interpreter, Cranelift and Metal tier 1 agree.
- `block_tests::an_output_may_not_depend_on_before`: `verify` refuses it.
- `bitlex_compile_tests::json_rows_read_masks`: the pair, object and array rows start at
  masks; the pair keeps the rule from the left as its `also`.

## 2. The monoids

**Builds:** `block_plan::monoid`: S1's 16-bit function, `last` (forward and backward),
`dyck` (38 bits), `add`, `or`. Each with an identity, a combine, a step, a block summary and a
sequential reference.

**Tests:**

- `monoid_laws`: identity and associativity on random values, for each monoid.
- `s1_summaries_compose_to_the_carries`: blocks of random JSON-like bytes; S1 summaries
  combined in any grouping give the carries and halos that tier 1 has at each block.
- `dyck_equals_a_stack`: random bracket strings; the combined effects give the stack, and
  the error, that a stack walk gives.
- `last_and_before_equal_a_walk`.

## 3. The analysis

**Builds:** `block_plan::analyse(program) -> Dataflow`: every value with its class, monoid
and depth; the scans by depth; candidates; fixes; dead values dropped.

**Tests:**

- `json_values_are_classed_as_designed`: the table of doc 16 §4.1, as printed text.
- `json_scans_are_s1_s2_s3`: S1 `fn` 16; S2 four forward and one backward; S3 `add`.
- `json_candidates_are_s1s_state`: four, from 2 bits; halos as fixes.
- `json_rule_from_the_left_needs_a_third_pass`: the pair without `before`: three byte passes.
- `json_token_count_is_dead`.

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
