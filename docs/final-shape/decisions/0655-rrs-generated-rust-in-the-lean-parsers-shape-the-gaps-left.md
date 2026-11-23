# 0655: `.rerail`'s generated Rust in the lean parser's shape: the gaps left after 0623

Status: in progress (the user, 2026-11-23: "please implement all of these, evaluating ONLY by how much closer they
make the tier 1 and 2 code to the lean Rust code"). Follows 0623 and 0650.

## Intro

Decision 0623 put the generated Rust (`rr_aot`'s `tier1.rs` and `tier2.rs`) beside the lean parser
(`rr_syntax/src/lean.rs`) piece by piece. Read whole after 0650, the two still differ in six places. These are
structural differences, not tuning: each is a shape the lean parser has and the generated code lacks. Each step
below is judged only by that: is the generated code now closer to lean's? Timing decides nothing.

The tier 2 loop form (`tier2_loop.rs`, decision 0618) is not part of this decision.

## The gaps

Tier 2:

1. **A choice of tokens is one arm per token.** `any`, `any except` and the punctuation choices became a `match
   self.word()` with one arm, and one take, per token. Lean takes such a class with one test: `raw_line` is a loop on
   `kind() != BAD`, and `is_p` tests a punctuation code.
2. **The grammar's near-copies.** `unsigned` and `lattice_unsigned`, `literal` and `aggregate_literal`, `negated` and
   `aggregate_negated`, `punctuation` and `setting_punctuation` are each two rules that differ in a token or a call.
   Each is generated twice, where lean has one function with a parameter.

Tier 1:

3. **A checker in the product file.** Under `test-hooks`, `tier1.rs` holds a whole-file mask pass (`stage1`), a walk
   of the regions, and an assert at each region's opener. Lean's lexer holds no checker: its tests live apart.
4. **The token loop.** Lean makes a block's starts at once (not space, not inside a word, not a pair's second byte).
   It takes the common tokens, words and operators, before the next rare byte in a run: one `TOKEN` load and the
   word's run. Only the rare bytes reach its `match`. The generated loop starts at each byte past the last token, and
   every token goes through the rows' `match`.
5. **The block's masks.** Lean's `Window` has four masks: `space`, `word`, `rare`, `pair`. Its arms test the byte
   after a `/` or a backtick where they read it. The generated loop makes eleven classes and a dozen masks a block,
   whether or not a token reads them.
6. **The marks.** Tier 1 works out each token's marks (`hits`, `pending`, `tested`, and `float_after`, `line`,
   `newline`, `json_bad`). Lean's lexer has none: its parser asks where it reads one (`ends_a_line`).

## Step 1: a choice of tokens is one arm per leaf (done)

The tier 2 writer (`bitlex_rust`) has a pass over the rules' code, `one_arm_a_take`, after the takes get their tests
(`takes`). In a `match self.word()`:

- An arm whose take tests the very token its pattern matched takes it untested. `w::COMMA => word_leaf(w::COMMA, K)`
  becomes `tk(true, K, 0)`. A kind's guard, `_ if self.at_string() => tk(self.at_string(), K, 0)`, becomes the same.
  `merge` and `skip` take the same way.
- The word arms whose code is then the same are one arm, `w::A | w::B | …`, at the first one's place: the words are
  distinct, so their order is free. Consecutive guards of the same code are one guard, `_ if a || b`.

So `raw_line` has three arms where it had one per token: the words, the two words a mark may begin (whose take asks
`open_mark`, as `takes` decides), and the token kinds. The decision is the lean parser's class test, as a `match`.
`io_parameters`, `io_inner`, `punctuation` and the rest shrink the same way. JSON's and Kotlin's tier 2 too, written
by the same pass.

All tests pass: rr, JSON, Kotlin, clippy.

## Step 2, first part: the lattice's value and the punctuation are one rule each (done)

- **A lattice's value is lean's `lattice_operation`.** The engine reads `@` and a name with no `(` after it as a
  functor named, not called, and any other value as a term. The grammar's `lattice_term`, `lattice_product` and
  `lattice_unsigned` were a second copy of the term for its first primary. Now a token mark, `bare_functor` (`tokens "@"
  identifier ~"("`, asked by tier 2 as the engine peeks), picks `lattice_functor` (`@` and the name), else the value is
  `term`. This also reads as the engine where the copy did not: `@f * 2` was a product there, and the engine refuses it
  (its statement recovered now).
- **`punctuation` is one rule.** A setting's `setting_punctuation` was `punctuation` without `{`, which opens its JSON
  object. Now `punctuation` has no `{`, and the two declaration tails that take one say so: `(modifier: word |
  punctuation | '{')*`, as lean's loops test for the `{` where they read it.

Tests: `lattice_values_are_the_engines` holds each executor to the engine on lattices (a named functor, a call, a
term, a comment after the name, a named argument) and to lean where the engine refuses (`@f * 2`); the generated
Rust's tapes on the same cases.

## Step 3: tier 1 holds no checker (done)

Where tier 1's loop reads its masks from the bytes (decision 0623 step 1), `bitlex_lex_rust` no longer writes the
test build's checker into it: no stage 1 of its own (`Stage1`, `stage1`, `stage1_best`), no walk of the regions over
the file, and no assert at a region's opener or an assumption's byte (`no_test_checks`, which also turns the `if {
assert; E } {` the asserts left into `if E {`). The nibble tables stay: the loop's classify reads them. The functions
the loop no longer calls go too (`drop_uncalled_fns`, until none is left): the whole-file mask helpers and the window
readers only the checker read.

The lean lexer holds no checker either. Its tests live apart, and so do tier 1's: `rust_tests` holds the records and
the tapes of the generated Rust to the interpreter's and the JIT's on every case, and the interpreter's own tests hold
the regions' walk.

JSON's and Kotlin's tier 1 read their masks in stage 1 and are unchanged.

## Step 6: tier 2 asks the marks' token tests (done)

Tier 1's loop made a word of hits for every token (`hits`), carried a token's "after" hits to the next grammar token
(`pending`), and joined them into each record's marks (`tested`). Rows of tests for every token, where tier 2 reads
two marks (`float_after` and `json_bad`) at a few places. Lean's lexer makes no marks: its parser asks what it reads of
a token where it reads it.

Where tier 1's loop reads the bytes (0623 step 1) and a byte's kind is `TOKEN`'s, `bitlex_lex_rust` writes the token
tests into a function of their own, `token_test` (one arm a mark: each row's tests, the row known by its first byte,
its kind and whether it is an extra), and `tested`: whether a test holds at grammar token `j`, its own or one of the
token after it at the grammar token before or a comment between, as the loop carried them. `TESTED` names those marks,
and no record holds them (`UNRECORDED`). Tier 2's `bit_at` and `marks_hold` ask `tested` for them, as they ask `gap` for
the gap marks: at a number's next token where `numeric` reads `float_after`, and at a JSON level's tokens for
`json_bad`.

The loop holds no hits, no `pending`, no `tested`, and with them none of the gap's state (`from`, `before`): a record is
its kind, its word and an extra's own bits. A float the file ends with (`float_after`'s `else escape`) is asked once,
after the loop, of the last tokens. The tests are of the windows from the token's first byte (`Bytes::rewrite_windows`);
a window past the end of a source with no padding (tier 2's) is its last bytes and spaces, as the padding's.

JSON's and Kotlin's tier 1 read their masks in stage 1 and keep their hits: their `TESTED` is none.

## Steps 4 and 5: tier 1's loop and its masks in the lean lexer's shape (done)

Where tier 2 asks the marks (step 6), the loop keeps nothing between tokens. `bitlex_lex_rust` then writes the lean
lexer's loop (`lean_loop`):

- **Four masks a block** (lean's `Window`): the bytes `skip` holds, a word's (the run of the common word row), a rare
  token's first byte (`k_rare`, nibble tables of its own, `RARE_0`), a pair's first byte. Nothing else is made of the
  block.
- **The block's starts at once**: a byte `skip` does not hold, not a word's after its first, not a pair's second (the
  pairs read left to right, `left_to_right`), as lean's `starts`. After a rare token they are made again from its end
  (`starts_from`), the byte after it a start where `skip` does not hold it: a word right after a number (`12abc`) starts
  there, as lean's number reader writes it.
- **The run of common tokens** before each rare start: one `TOKEN` load for the kind and the word alone, the last byte by
  the word's run or the pair's bit, the record written. No `match`.
- **Only the rare starts reach the rows' `match`**: a string, a comment, a backtick literal, `#`, a number, a byte no
  token takes, and `/`, whose group's test picks a comment or the operator. The rows only the run reads (`identifier`)
  have no arm. Each arm reads its bytes from the token's first (the next bytes tested where it reads them, the windows
  from its first byte: `Bytes::rewrite_windows`), not the block's masks: `/` tests the byte after it, a backtick the
  bytes after it, as lean's arms do.

A common row is a run of one mask whose first byte it holds, a pair, or one byte, with no marks, no region and no test of
its own, and no byte of a word but the run's. The rest are rare. Where a grammar's rows do not fit, the loop is the one
before.

Tests: the rr suite (its records against the interpreter's and its tapes against the JIT's on every case) and both
corpus tests over the repository's 1,949 `.rerail` files (`corpus_tier1s_records_are_the_interpreters`,
`corpus_tapes_are_the_jits`). JSON's and Kotlin's tier 1 are unchanged.
