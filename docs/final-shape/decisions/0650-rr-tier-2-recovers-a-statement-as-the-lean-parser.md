# 0650: `.rerail`'s tier 2 recovers a statement, as the lean parser does

Status: designed; the steps below are under way. The item is the user's (2026-11-23): lean's statement-level error
recovery in every executor of `rr.bitlex`'s tier 2, with A20 kept (no trial reads inside an expression; `reread` and
the lookahead patterns stay).

## What the lean parser does

`lean::tape_at` reads a file as statements. Before each it takes a mark (`Parser::mark`: the token index, the
writer's length, the open nodes, the children, the comment cursor). When the statement fails, `recover` cuts back to
the mark (`reset`) and writes one ERROR leaf over the statement's first token through the token before the resume
point `k`, found by a scan from the first token on:

- a `#` token stops it, `k` there;
- a `.` first on its line (only blanks before it since the line's terminator, `\n` or `\r`) before a name stops it,
  `k` there;
- any other `.` ends it, `k` past the `.`.

Then it goes on at `k`. A comment inside the leaf's text is no node (`push_near_comment` skips it). Only the file's
statements recover: a `.test` block is one statement, and one that fails is cut back whole, its body read again as
the file's statements. The resume point depends only on the statement's first token, never on where the statement
failed.

## What the grammar refuses, against lean's ERROR leaves

`where_the_grammar_refuses_against_leans_errors` (`rr_parser`'s tests, run by name) runs the interpreter over the
corpus and lean's edited copies of it (the edits and the seed of `lean_tape_is_the_engines_on_edited_files`). It
tallies each refusal by its word's reason and by where lean's first ERROR leaf lies. No text the grammar takes has
an ERROR leaf in lean's tape. Of the refusals:

- **In lean's failed statement.** "No state takes the token" lies in it nearly always. So does a refusal past the
  leaf's last token but before the statement lean reads next. One kind is the token after a broken `n` with comment
  lines between. Another is a `{` that opens a JSON object or a braced column and reads to the file's end. Both are
  the same statement, and the resume point is the same.
- **A bare term held to the file's end.** A literal that is a bare term (`kw_n,one(…)`, `bl.acklisted(…)`,
  `fie*ld(…)`) holds the provisional field `bare` until a comparison relabels it. Lean fails the statement there.
  The grammar ends the statement, reads on, and refuses only at `ACCEPT` ("a provisional field at the file's end").
  The check belongs at the statement's end.
- **The grammar refuses what lean reads.** Each is a gap in the grammar, to be closed before its refusal can
  recover:
  - a backtick literal then `at "kind"` in a pattern (`n ~ `…` at "except_clause"`);
  - a `#` word among a setting's arguments (`revi#sion 1`);
  - an `@` word among them (`.des@cription`: the `fn_uncalled` mark refuses it);
  - a `#` in a declaration's tail (`.decl r(p: node)#`);
  - an aggregate whose word is any name (`cont : { … }`, `k = product w : { … }`).
- **Limits.** These are not syntax, and lean reads past them:
  - a `\u` escape the engine checks ("a local check holds");
  - the depth;
  - a reading that reaches a dropped node;
  - the tape's widths.
  They refuse the file as before, and the engine's parser (lean's tape) writes it.
- **Tier 1's refusals.** These are "a byte no token takes" and a region open at the file's end. Lean's lexer makes
  each a `Bad` token, which fails its statement. Tier 1 refuses the whole file instead. Until tier 1 writes such a
  token (step 5), they stay whole-file refusals.

## The design

**A grammar statement, `recover`.** In `rr.bitlex`'s tree:

```
recover source_file before hash_word | line_dot identifier, after "."
```

It makes the repetition of a `whole` node lean's statement loop: each repetition is a statement, marked before it is
read. `line_dot` is a token mark tier 1 sets on a `.` first on its line, from the bytes, as lean's `line_start`
reads them. The resume rule is lean's, read off records: a record's kind, its mark bit, and the next record's kind.

**Which refusals recover.** A refusal is syntax or a limit. The syntax ones recover:

- a token no state takes, or one a rule needs and does not find;
- a `refuse M in L` mark, once each is exact (the gaps above);
- a bare term held at the statement's end.

The limits keep the whole-file refusal. In the calls form this is lean's shape: `refuse()` fails the statement, and
`limit()` fails the file.

**The mark.** It holds everything a statement changes, in each executor:

- the record index and the comment cursors (`pend`, `xi`, `next_x`);
- the writer's length;
- the levels (open nodes, their saves);
- the dropped nodes (`dead`);
- the strings open over a template.

Reset truncates each and sets the flags and the word back to the file's level. As in lean, it is taken only at the
statement loop's head, where no call is open: the returns are empty, and the levels are the root's.

**Linear work.** The scan passes the failed statement's tokens once more. As lean's, a failed statement's parse may
read past its resume point (a `{` read to the file's end), and those tokens are read again from `k`. The bound is
lean's.

**Each executor.**

- **The calls form (`tier2.rs`, `bitlex_rust`).** `source_file`'s loop takes lean's `mark`, and on a syntax
  refusal calls `recover`.
- **The device loop (`tier2_loop.rs`).** Its statement loop head saves the mark in its state. A syntax refusal goes
  to one recovery block, which resets, scans, writes the leaf, and goes back to the head.
- **The interpreter (`stream.rs`) and the JIT (`native_cpu_stream.rs`).** The same, as words: a `MARK`-like word at
  the loop head, and a refusal kind that jumps to the recovery instead of returning.

**Tests.** Lean's edit fuzz per executor: wherever an executor takes or recovers an edited text, its tape is
lean's, byte for byte. The probe stays, as the tally of what still refuses.

## Steps

1. The grammar's gaps where it refuses what lean reads, closed in `rr.bitlex`. Done:
   - `at_on` and a pattern's `selector`;
   - a `#` word in a line's punctuation (`hash_word as identifier`): `hash_word`'s keywords are no longer
     `required`, so a `#` word but `#include` is a token kind of its own, and `io_bare` names both;
   - a named argument at a literal's start (`unless no_named`, as lean's `primary`);
   - `product` among the aggregates' words, an identifier as lean's `anon` writes it. A `remember` choice's
     token taken `as` a kind remembers that kind (`grammar.rs`'s `Pos::Tok`, `bitlex_rust`'s `token_choice`);
   - `fn_uncalled` removed: in a term `functor_call`'s arguments refuse what it did, and in a line it was wrong.

   The probe now finds no refusal of a text lean reads with no error, but for the limits and tier 1's.
   `the_grammar_reads_what_the_engine_reads` holds each case.
2. `recover` in the grammar language, the `line_dot` mark, the calls form's recovery, the provisional check at the
   statement's end, and the fuzz over the calls form.
3. The device loop.
4. The interpreter and the JIT.
5. Tier 1's errors as lean's `Bad` tokens, so a byte no token takes and a region open at the end recover too.
