# Parser generators: the TDD plan for tier 2

Status: plan, being built (2026-11-07). Rewritten for decision 0051: the tree section is a
grammar (doc 18), and tier 2 is its state machine. Each phase writes its tests first and sees
them fail. It builds until they pass, commits, and only then starts the next phase.

## Intro

Tier 2 is simdjson's stage 2: a state machine over tier 1's token stream. The Bitlex file
states the grammar; the compiler makes the state machine from it, as data in the block
program; and the compiler lowers that data to the transducer's IR, which runs on the IR's
interpreter, Cranelift and Metal. Nothing else runs it.

| phase | builds | pinned by |
|---|---|---|
| 1 | the tree section as a grammar, and its reference parser | tree-sitter, over the corpus |
| 2 | the state machine: a compiler pass, held in the block program | its `rir` text, pinned whole; a guard |
| 3 | the monoids, kept for Metal | laws and sequential references |
| 4 | the Metal plan's stores, steps and verifier, kept for Metal | hand-built plans and refusals |
| 5 | the CPU: tier 2 lowered to the IR, run on the interpreter and Cranelift | IR shape; tapes; cost |
| 6 | Metal, after the CPU: depth carry, nearest smaller value | tapes equal the CPU's; shape guards |
| 7 | timing | CPU time next to simdjson's stages |

The oracle for tapes is tree-sitter. Rust references are oracles for the parts (the
reference parser, the monoids' walks). None of them is an executor.

## The rule

The states and arms are the compiler's data. Two things read them: the pass that makes them
(`grammar`) and the pass that lowers them to the transducer's words (`bitlex_tier2`, decision
0053). No Rust function walks them to parse a file, and generated code calls no Rust. The guard
`only_the_compiler_reads_tier2s_states` fails if any other file reads them.

## 1. The tree section as a grammar (done)

**Built:** doc 18's syntax in `tools/bitlex` (`node K [nested] = E`, `choice N [supertype S]`,
`f: X`), and the reference parser over the token stream. JSON's tree section is
tree-sitter-json's `grammar.js`, restated.

**Found:** the root spans the file to its end, trailing whitespace included, as tree-sitter's.
A number's syntax went to tier-1 masks and errors (no validator call).

**Tests:** `tree_grammar_parses`; the corpus: 28,467 of 28,470 files equal tree-sitter's
nodes. One file has no tokens and falls back; the two that tree-sitter errs on are refused.

## 2. The state machine (done)

**Built:** `grammar::build`, a pass of the Bitlex compiler:

- one level for the root and one per `nested` node, the others inlined;
- Glushkov's positions, with the wrappers (nodes, fields, supertypes) each edge leaves and
  enters;
- a state per position, an arm per token: closes, opens, the leaf, the next state; a call
  opens the nested node and pushes its return state;
- refusals: not LL(1), an empty node, a self-reference without `nested`;
- Moore's refinement merges states that act alike.

The block program held token rows, keywords, `skip`, states and arms; since decision 0053
they are the compiler's only, and the words carry them. The old rows (`after`, `before`,
`also`, `parent`, code facts, the `json_number` validator) are deleted, with the Rust tape
writer's leftovers.

**Tests:** `json_tier2_states_print_as_designed` (19 states, pinned whole),
`json_number_check_is_straight_line`, `json_number_check_equals_the_reference`,
`a_check_that_needs_a_byte_loop_is_refused`, `check_matches_a_tokens_bytes`,
`rir::tests::block_tree_rows_round_trip`, `json_rows_equal_the_evaluator_on_every_executor`
(the token rows), `only_the_compiler_reads_tier2s_states`.

## 3. The monoids (done, for Metal)

`block_monoid`: `StateFn`, `Last`, `First`, `Dyck`. Metal composes each level's transitions as
state functions and matches closes by depth (phase 6).

**Tests** (`block_plan_tests`): `state_fn_laws`, `state_fn_composes_like_a_walk`, `dyck_laws`,
`dyck_equals_a_stack`, `last_and_first_equal_a_walk`.

## 4. The Metal plan's verifier (done, for Metal)

`block_plan`: stores (threadgroup or device, single or double), steps (uniform or ragged
loops, work, barriers), and the verifier: the budget of 10,240 bytes, SIMD operations and
barriers only in uniform loops, a barrier between every write and read of a shared store by
different steps, double buffers as a parity. The plans themselves are made again in phase 6,
from the state machine.

**Tests:** `plan_verifier_accepts_a_whole_plan`, `plan_verifier_refuses_a_missing_barrier`
(each of 5), `plan_verifier_models_double_buffers`,
`plan_verifier_refuses_simd_in_a_ragged_loop`,
`plan_verifier_refuses_threadgroup_memory_over_the_budget`.

## 5. The CPU: tier 2 lowered to the IR

**The shape.** Tier 1 is stage 1: bit operations per 64-byte block, ending in one extraction
of the token stream. Tier 2 is stage 2: one loop over the stream. At each entry it reads the
token's kind, branches on it in its state, does straight-line work, and jumps to the next
state. Branches are fine where they are on something concrete and go straight through. There
is no search for a character, no loop over bytes, no counting.

**5a. The stream.** Two extracts, as simdjson's structural indexes: every token's first byte,
and the last byte of each token whose end its row does not give. A one-byte token ends after
its byte; a keyword-only row's word gives its length. JSON's second extract holds string
contents' and words' last bytes.

**5b. The words (decision 0053).** Tier 2 is the transducer's own instruction words, with new
ones for the stream, lowered by its own pipeline: the words' interpreter, `native_ir`, and
Cranelift's lowering of `native_ir`. There is no second IR and no new code generator. Per
state:

- `TOKEN` takes the next token, and `SWITCH` jumps by its first byte;
- the arm: `SPAN` (its end), `WORD` or `CHECK` where needed, its node writes (`CLOSE`, `OPEN`,
  `LEAF`, `NODE_FIELD`, `NODE_SUPER`), a `PUSH` for a call, and `GOTO` the next state;
- a nested level that may end closes its node and `RETURN`s; the root `ACCEPT`s.

The verifier refuses any cycle without a `TOKEN`: no loop over bytes, ever (decision 0052). A
`CHECK` runs its block program over the token as tier 1 runs over a file, 64 bytes at a time,
its carries passed from word to word; tree-sitter is never the way out of a case simdjson
handles.

**5c. The kinds and checks.** A token's kind is its first byte, as simdjson's
`switch (*value)`: the compiler finds each row's possible first bytes from its start mask's
operations, and where two rows share a byte, tier 1 guards the wider. A keyword is a switch
per byte, and a `WORD` only where the switches have not read all of it or its length is not
its row's. A token with a check runs it in its arm, as simdjson's `parse_number`: JSON's number
is 100 operations, a `fill` per digit run.

**5d. The stack.** simdjson's: one frame per nested open, in memory: the open's ord and the
return state. A close writes the size into the open's slot. Its depth is the file's.

**5e. The tape, with widths speculated.** Tier 2 writes the v9np columns directly, at widths
guessed before the file (from its length and its stream's length), and ORs each column's
values as it goes. If the ORs fit the guess, the tape is done. If not, the columns are written
again at the known widths: at most one second pass.

**Tests, first:**

- `json_stream_equals_the_evaluator_on_every_executor` (done): the two extracts are the
  evaluator's tokens, on the interpreter, Cranelift and Metal.
- `a_loop_not_over_the_stream_is_refused` (done): the verifier refuses a cycle without a
  `TOKEN`; `json_tier2_is_transducer_words` (done): no word of the tree walker's.
- `json_tier2_nodes_equal_the_reference` (done, the words' interpreter): the reference
  parser's nodes on cases, nesting, every kind, long numbers and strings.
- `json_tapes_equal_tree_sitter_byte_for_byte` and the corpus, on the interpreter and Cranelift.
- `json_widths_speculated_once`: a second pass only where the guess was wrong.
- `json_cost_ratchet`: both tiers next to simdjson's full parse, and each next to its stage, in
  CPU time.

**Tier 1's cost (decision 0052).** The number masks took tier 1 to 95 operations and 450 loop
instructions; with the check in tier 2 and one word token it is 37 and 174, below the old
ratchets, which are lowered. Handing tier 2 its stream (decision 0053) took it to 40 and 182.

## 6. Metal

**Builds:** tier 2 from the state machine: the carry is the depth and its low point; closes
find their opens by nearest smaller value (doc 01 §3, `metal_nearest`); within a level, the
transitions compose as state functions, with each transition's node count. A plan for it,
checked by phase 4's verifier.

**Tests:**

- the MSL compiles; its barriers are its plan's; its loops with a SIMD call or a barrier are
  uniform; its threadgroup arrays are within the budget;
- `monoids_on_metal_equal_rust`: the generated helpers, run on random values;
- `json_metal_tapes_equal_the_cpus`: test cases and the corpus.

## 7. Timing

CPU time per 64 bytes for tier 2 on the CPU and on Metal, next to simdjson's stage 2, with
the machine's load. Paired runs.

## Process

- Targeted test runs only, each under 2 minutes. The corpus once per phase, in the background.
- A commit per phase, after its tests pass. Pushed.
- A conformance table (plan item → `file:line`) at each report.
