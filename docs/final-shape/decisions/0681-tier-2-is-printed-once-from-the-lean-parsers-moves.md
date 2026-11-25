# 0681: Tier 2 is printed once from the lean parser's moves

Status: done but the names (step 2) (the user, 2026-11-24: "what are the BIG FUCKING ROCKS?", then "so do 1"). Follows 0673. The bar is
0673's: is the generated code the lean parser's (`rr_syntax/src/lean.rs`) shape?

## Intro

0668 and 0673 brought tier 2's code closer to the lean parser one form at a time, and each form was a rewrite of the
code's text. The generator (`bitlex_rust.rs`) writes each rule's Rust as strings: `code` gives a grammar element's
lines, `decide` a decision's. Then eleven passes rewrite those lines: `takes`, `leaf_fields`, `one_arm_a_take`,
`no_retests`, `ifs_and_whiles`, `no_empty_elses`, `no_code_after_return`, `no_taken_retests`, `shared_test_as_match`,
`no_outer_parens`, `loops_as_whiles`, `no_retest_after_while`. Each pass finds a shape by matching lines and
indentation, and `decide` itself reads its own conditions back out of strings (`holds`, `untested`, `word_arms`).

That is why each round changed little. A pass can rename a call or join two `if`s. It cannot see what the code does,
so it cannot change the code's structure, and each new form needs another pass that matches the last pass's output.
The lean parser is written in a few moves, and the generator has no model of them.

## Decision

The generator builds each rule's code as the lean parser's moves, rewrites the moves, and prints Rust from them once.

### The moves (`bitlex_rust/moves.rs`)

A rule's code is a block of moves:

- **A take** of the token: its test (none where the code before it tested it), and its leaf (none, the lean parser's
  `p` and `implied`; a kind and a field, `p_stored`, `id`, `bump_as`; or the word's own kind, `anon`). A pass and a
  merge are takes too: into a `leaf`'s text, into the leaf made last.
- **A call** of a rule, with the state it saves around it.
- **The tree's moves**: open, close, wrap, retag, a field set on the node made last, relabel.
- **A decision**: an `if` chain of tests and blocks, or a `match` on the word at a place (the token, or one after it),
  each arm its words, an optional guard and a block, and a default.
- **A repetition**: `while` a test, or a `loop` its blocks `break` out of.
- **A refusal** (`return self.refuse()`), and a few statements of their own (the flags saved and set, a scan's `let`).

A test is structured too: the word at a place is one of some words, the token is of a kind, a mark holds (a peek), a
flag is clear; and `!`, `&&` and `||` of tests. A test the moves do not model yet is its text, so the moves can be
built a part at a time.

### The rewrites are of moves

Each of the eleven text passes becomes a rewrite of the moves, and the text pass goes:

- a take whose test the decision around it tested takes the token with no test (`no_retests`, `no_taken_retests`,
  `one_arm_a_take`'s untested take, `untested`);
- a field set on the leaf just taken is the take's field (`leaf_fields`);
- arms of one block are one arm, their words joined (`merged_arms`);
- a `match` of one arm is an `if`; a `loop` whose decision breaks where one test holds is a `while` (`ifs_and_whiles`,
  `loops_as_whiles`); a `while` whose refusal follows it refuses at its `break` (`no_retest_after_while`);
- an `if` chain whose tests share their first test and then ask the next word is that test and a `match` on the word
  (`shared_test_as_match`);
- no empty `else`, no move after a refusal (`no_empty_elses`, `no_code_after_return`);
- the take's tests (`takes`' `M` and `F`) are the take's own fields, and the printer writes lean's names (`lean_names`
  for the rules' code).

`decide` decides on tests, not strings: where a test may hold (`holds`) is read from its structure.

### Both forms

The loop form (`one_loop/records.rs`) builds its places from the same moves (`G::code`, `G::decide`), a jump to a
place one more move, and the same rewrites run on each place.

## Steps

1. The moves, and a printer that writes the text the generator writes now. `code`, `decide` and the loop form build
   moves. The generated files do not change. (Done: `moves.rs`; `G::code`, `decide`, `decide_tests`, `asking`,
   `token_choice`, `take_token`, `ending_code` and the loop form's `walk`, `level` and places build moves, and
   `records::no_refusing` on them is a field of each take. rr's, Kotlin's and JSON's generated files are byte for
   byte the same.)
2. Each text pass on the rules' code becomes a rewrite of moves, and is removed. The generated files do not change,
   or each change is named. (Done for all but the names: `moves::rewrite` runs `leaf_fields`, `takes`,
   `one_arm_a_take`, `no_retests`, `ifs_and_whiles`, `marked`'s writes, `no_empty_elses`, `no_code_after_return`,
   `no_taken_retests`, `shared_test_as_match`, `no_outer_parens`, `loops_as_whiles` and `no_retest_after_while` on
   each rule's and each place's moves, and their text forms are gone. A take's `M` is a field of the take, from
   `G::open_mark_words`. `G::rule` gives moves, and each form prints its rules once all of them are made, as whether
   any decision reads `marked` is known only then (`G::marked_read`). The changes: Kotlin's two `_ => {}` arms lose
   the empty line the text `one_arm_a_take` wrote in them; and a bracket's scan (`scan_fn`) writes its nest's tests
   without outer parentheses itself, which the text `no_outer_parens` had taken off. Not yet: `lean_names` still
   renames the takes in the whole text, the runtime's definitions with them, and `no_marked` still takes `marked`
   out of the runtime and the methods' ends.)
3. `decide` reads tests, not strings. (Done: a test is a `moves::Cond`: the word at a place one of some words
   (`Word`), the token's kind (`Kind`), a mark holding with its peek's tests (`Mark`, from `G::peek`), a bracket's scan
   (`Scan`), a clear flag (`Flag`), and `!`, `&&`, `||` of tests; a `match` arm's pattern is its words and a guard
   (`Pat`); a field set on the node made last is a move (`Field`). `decide_tests` builds its runs of tests and reads
   where each may hold of the test (`G::holds`); its scans are asked once by finding the `Scan` tests; and the
   rewrites compare tests, not their text. A part of a test the moves do not model, an element of a mark's run that
   is no word, stays its text. The changes: a peek's choice of words is one `matches!` (`matches!(self.word(), w::MUST
   | w::MAY)`, not `(… || …)`); a take whose arm's guard is a kind with hard words (`self.kind() == t::HASH_WORD &&
   self.word() != w::…`) takes its token untested, so the arms of one take are one arm (`_ if a || b && c || d`),
   their jumps in the loop form one; a negated call is `!self.m_x()`, and a negated group has one pair of
   parentheses.)

The passes over the runtime's text (`one_take`, `no_field`, `no_dead_methods`, `plain_writer`, the runtime's
replacements in `assemble`) are not rule code and are not part of this decision.
