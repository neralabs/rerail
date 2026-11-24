# 0668: `.rerail`'s tier 2 decides a literal after its term, as the lean parser

Status: in progress (the user, 2026-11-24: "address these differences, with the success criteria being structural
similarity not execution speed or efficiency"). Follows 0655, which brought tier 1 into the lean lexer's shape.

## Intro

Tier 2 (`rr_aot/src/tier2.rs`, written by `bitlex_rust` from `rr_parser/rr.bitlex`) was set beside the lean parser
(`rr_syntax/src/lean.rs`) after 0655. The statements and clauses read as lean's: a function a rule, `open` / `field`
/ `close` where lean has them. The expressions do not. Tier 2 is about 3.4 times lean's parser in lines, and most of
the extra is in `literal` (225 lines against lean's 63), `aggregate_literal` (305), `unsigned` (144) and the near-copies
they call. Each step below is judged only by one question: is the generated code closer to lean's shape? Speed
decides nothing.

`tier2_loop.rs`, tier 2's device form (decision 0618), is not compared with lean. It is still written from the same
grammar and must read what the calls form reads.

## The gaps

1. **A literal is decided before its term.** Lean's `literal` parses one `term`, then the token after it decides: a
   `~` makes a pattern match, a comparison operator a comparison. Otherwise a call alone is an atom (`retag_atom`)
   and anything else fails. Tier 2's grammar decides at the first token and at the token after an identifier. So
   `literal` repeats `term`'s alternatives, and `aggregate_literal` repeats `unsigned`'s, with `relabel`, `wrap` and
   `retype_moved` on each path.
2. **A choice tests the word twice.** Where two tests lead to one arm, `decide` numbers the arms (`let arm`,
   `match arm`), and inside an arm the alternative's own `match self.word()` tests the token again. Lean tests a token
   once.
3. **`!` is a flag carried down.** Lean parses the negated literal, then fails where it was a comparison
   (`inner == Lit::Cmp`). Tier 2 sets `negating` for the call so that the operand never takes a comparison. Every
   literal's call in a conjunction then saves and restores the flags and the level's word.
4. **Small differences of form.**
   - A `match` with one arm and an empty `_`, where lean writes `if self.is_p(X)`.
   - `loop { match … _ => break }`, where lean writes `while`.
   - `self.marked = false` at the end of every rule, where no decision of `.rerail`'s reads `marked`.

## What tier 2 can and cannot take from lean

Lean goes back where its first try fails. A `(` at a literal's start is tried as a group, and read again as a term
where that fails or an operator follows. An aggregate's word is tried as an aggregate, and read again as a
variable. Tier 2's grammar is also the JIT's and the interpreter's (`grammar.rs`'s state machine, the stream words).
Those read a file in one pass and never go back. So tier 2 keeps its cover form for these two cases: a group read
again as a `parenthesized_expression` (`reread parenthesized`), and an aggregate's call read again inside its
aggregate (`reread aggregate_value`). Each is one pass with the same tree, and the same refusal where lean's
statement fails: lean's `recover` cuts back to the statement's first token, wherever inside it the failure was.

The decision after the term needs no going back. Lean's `term` returns whether it was a call alone, and `literal`
asks that after the term. Tier 2 has the node it made last at its level, and can ask its kind. That is the one thing
the grammar language lacks.

## Plan

1. The writer's forms (gap 4): `if` and `while` where a decision has one arm, and no `marked` where no decision reads
   it.
2. The kind of the node made last, asked where a level ends (`ending`): a statement of the grammar, a test word of
   the machine's (`MADE_IS`), and the writers' code. `literal` becomes `negation | group [group_term] | bare: term
   [literal_tail]`, ending with "a call is an atom". `aggregate_literal` and the copies it called go (gap 1).
3. `negation`'s operand is any literal, and its level, where it ends, refuses a comparison made last (gap 3). The
   `negating` flag goes.
4. What is left of gap 2 once 2 and 3 are done.

## Step 1: one arm is an `if`, a loop of one a `while`; no `marked` unread (done)

The calls form's writer (`bitlex_rust`'s `assemble`, not the loop form's) has a pass after the takes get their tests
(`one_arm_a_take`), so a take inside keeps its untested form:

- `ifs_and_whiles`: a `match self.word()` with one arm and an empty `_` is `if COND { … }`. COND is
  `self.word() == w::X`, `matches!(self.word(), w::A | w::B)`, or a kind's guard, with a flag's guard joined by `&&`.
  A `loop` whose one decision is such a match, its `_` a `break`, is `while COND { … }`. `conjunction` now reads as
  lean's: `while self.word() == w::COMMA_12 { … }`.
- `no_marked`: `marked` tells a decision that a mark was taken last (`after_mark`). Where no decision reads it
  (`!self.marked` nowhere), its writes, its field and its place in a statement's `Mark` go. That is `.rerail`'s case:
  every rule's `self.marked = false` is gone. Kotlin's decisions read it, so Kotlin keeps it.

rr's `tier2.rs` loses about 400 lines; Kotlin's is written by the same pass. All tests pass: rr 58, Kotlin 70,
JSON 26, clippy.
