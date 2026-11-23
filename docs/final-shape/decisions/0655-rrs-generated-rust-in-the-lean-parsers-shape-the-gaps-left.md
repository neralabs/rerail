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
