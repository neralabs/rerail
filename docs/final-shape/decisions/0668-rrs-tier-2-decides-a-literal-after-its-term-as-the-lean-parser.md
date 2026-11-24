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

## Step 2: a literal is a term, then what follows it; a call alone an atom (`ending`) (done)

`literal` is now lean's: `negation | group [group_term] | bare: term [literal_tail]`. The term is the one rule every
term is read by; `aggregate_literal` and the first token's copies of `term`'s and `unsigned`'s alternatives are gone.
What lean asks after the term (`is_call`, then `retag_atom`) is a new statement of the grammar:

```text
ending literal call as atom inner relation
```

Where `literal` ends, if the node made last at its level is a `call`, it becomes an `atom`, its first child's field
`relation`, its own field none (the caller's, set where it returns, as `decide`'s). `ending R K refused` refuses
instead (step 3). Each consumer of the grammar reads it in its own form:

- The reference (`bitlex::tree`): where a rule's match ends, before its node closes, the node made last since the
  rule began.
- The state machine (`grammar.rs`): each level's endings, on its states (`StateRow::endings`, part of the merge key).
  Its words (`bitlex_tier2`) put them between the closes inside the level and its own close: a new test word,
  `RETAG m=4 K` (`MadeIs` in the IR), runs the next word where the node made last at the level is a `K`. A relabel
  is three tested words, the inner field, the field and the kind, in that order so the kind is tested before it
  changes. The interpreter, the JIT (`native_cpu_stream`) and the stream analyses take the word as they take
  `FLAG m=2`.
- The calls form (`bitlex_rust::rule`) writes it after the body, as lean's `literal` does:
  `if self.made_is(k::CALL) { relabel_inner; relabel_field(0); relabel_kind(k::ATOM) }`. The loop form writes the
  same at the level's end. JSON's one loop refuses an ending, since its grammar has none.

A group and a term both start with `(`. The state machine took a rule's choice as LL(1) and refused two arms on one
token. It now keeps the earlier arm where the later one enters a call, as the reference and the Rust writer's `decide`
already do. A group a term turns out to be is still read again as one (`group_term`, the cover form kept above).
Two arms on one token where neither enters a call are still refused.

rr's `tier2.rs` is 4,438 lines, down from 4,893.

Three of the rr parser_tests had been red since 0650 and 0655, and are fixed at their roots:

- The ladder's reference does not recover, but the interpreters do (`recover`, decision 0650). Where a statement
  recovered (an ERROR leaf, the reference refusing), the ladder now holds the compiled code's tree to the
  interpreter's. rr_parser's tests hold both to the lean parser's.
- The reference read a region left open at the file's end (`open bad`) as its own token's kind. Tier 1 reads it as
  the `open` kind's, lean's `Bad`. The reference now does too (`TokenRule::open_ends`).
- The engine refuses an integer past an `i64`, but the grammar reads it as written (0655 step 13). The ladder's
  engine check now reads the text as lean does, through the same helper rr_parser's tests use, moved to
  `rr_syntax::as_lean_reads`. The marks test names the grammar's `hidden` line as it is since 0655 step 9.

Tests: rr 58, the 8 corpus tests, Kotlin 70, JSON 26, parser_tests 95, Bitlex 11, clippy.
