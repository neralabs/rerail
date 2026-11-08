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

## 5. The CPU (done)

**Built:** `native_cpu_tier2::compile(program, fallbacks)`: one Cranelift function per block
program, generated from its rows. It reads tier 1's masks, word-major, and writes the nodes in
preorder. Its columns are packed by the tape's one packer (`TapeImage::from_written`).

The function walks the events in byte order: every bit of a row's start or end mask. At each:

- **the token** that starts there picks an arm of one `br_table`: an arm per token row, and one
  for a byte where no token starts. In its arm, the rows' starts are tests in rank order. A
  test the arm decides is folded: a token row's own start, another token row's (false), a
  kind known from a row without keywords. Keywords are inlined as compares of 8-byte words.
- **the ends** close the stack's top while its row ends there, by a `br_table` on the top's
  row. A node that ends `with` its child's field closes with it, by another.
- **the checks** send the file to tree-sitter: a fallback bit, two tokens at a byte, bytes in
  no token, a stray or too-deep close, a validator, a second root.

The CPU plan's rule is `also`: the pair opens from the left. Its `before ':'` is checked at
the token after its first child (the string). A file where the two rules differ goes to
tree-sitter.

**Changed from the plan above:**

- **The dispatch is per token row, not per kind.** JSON's punctuation is one row whose
  keywords give six kinds: the kind is read in the arm, by compares, as simdjson's stage 2
  switches on the byte.
- **`check_nesting` is gone.** The stack makes nodes nest; the second-root check is generated.
- **The validator is a call,** the only one: `json_number`, a built-in over the token's text.
  Its exponent takes `-` only, as tree-sitter-json's.
- **Open:** the masks are copied word-major and the source padded by 8 bytes before the call.
  Tier 1's buffer is already word-major; reading it in place is phase 7's.
- **Open:** two valid files of the 900 go to tree-sitter (as with the deleted writer).

**Tests** (`bitlex_compile_tests`): `json_tapes_equal_tree_sitter_byte_for_byte` (on the
interpreter's masks and Cranelift's), `json_tier2_writes_doc_16s_nodes` (doc 16 §8's 19
nodes), `json_tier2_has_one_case_per_token_row`, `json_tier2_reads_no_rows` (arguments only;
no global, data or stack slot), `json_tier2_calls_nothing_per_token_but_its_validator`, and
`json_rows_equal_the_evaluator_on_every_executor` again asserting that a file the evaluator
rejects falls back. The corpus, once: 900 files, 897 tapes equal, 3 to tree-sitter, 0 differ.

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
