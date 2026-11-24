# 0650: `.rerail`'s tier 2 recovers a statement, as the lean parser does

Status: done (steps 1 to 5). The item is the user's (2026-11-23): lean's statement-level error
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
- **Tier 1's refusals.** These were "a byte no token takes" and a region open at the file's end. Lean's lexer makes
  each a `Bad` token, which fails its statement. Tier 1 refused the whole file instead. Tier 1 now writes such a
  token (step 5), and its statement recovers. Tier 1's escapes stay limits (a `\u` escape, a number of 19
  digits, four backticks; a float the file ends with was one until 0655 step 9, which reads it).

## The design

**A grammar statement, `recover`.** In `rr.bitlex`'s tree:

```
recover source_file at hash_word | "#include", at first "." identifier, past "."
```

It makes the repetition of a `whole` node lean's statement loop: each repetition is a statement, marked before it is
read. Each clause is a token pattern, as a token mark's (`tokens …`), tested at a record from the failed statement's
second token on: `at` stops the scan there, `past` stops it past the record. `first` asks that the record be first
on its line, as lean's `line_start` asks: only blanks before it back to a line terminator or the file's start. Tier 1
leaves only blanks and line ends between records, so a record is first on its line where no record comes before it,
or where a terminator lies among the bytes since the record before (the token before, or a comment after it). The
scan reads those bytes a word at a time, and only in recovery, as lean reads them.

Not a mark of tier 1's. A `line_dot` mark was tried (stash `pg-0650-line-dot-mark-tried-2026-11-23`). As a plain mask
a mark's bit is lifted over the gap to the next token, so a `.` at a line's head inside a comment or a string marks
the token after it; excluding the regions makes the CPU's tier 1 walk them, which decision 0177 removed. As a mark of
one word (the `.`'s starts and the line-head mask) it is exact and walks nothing, but the CPU's tier 1 then makes the
line-head mask over every block: a mask no token's window can read, made for every file, where lean asks it only of a
failed statement.

**Which refusals recover.** A refusal is syntax or a limit. The syntax ones recover:

- a token no state takes, or one a rule needs and does not find;
- a `refuse M in L` mark, once each is exact (the gaps above);
- a decision lean makes on two tokens. Where tier 2 had a mark too wide for it, the mark is now lean's test: a suppress
  line's `,` goes on only before a string (`comma_rule`), and `.input` or `.output` with no relation is refused
  before every token but a `.` or a name (`io_bare`).
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
- **The device loop (`tier2_loop.rs`).** `run` is the calls form's statement loop. The loop's places are one
  statement's (`statement`), from its first decision to its end, which returns. A refusal anywhere in them returns
  to `run`, which cuts the loop's stack back (at a statement's start it is empty) and recovers as the calls form
  does. The loop holds no mark of its own, and the recovery is the calls form's code.
- **The interpreter (`stream.rs`) and the JIT (`native_cpu_stream.rs`).** The same, as words: `STATEMENT` at the
  loop's heads, and a refusal by syntax that recovers instead of returning (step 4).

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
2. `recover` in the grammar language, the calls form's recovery, the provisional check at the statement's end, and
   the fuzz over the calls form. Done:
   - `recover` in `bitlex`'s syntax (`Stmt::Recover`, `RecoverClause`). The interpreter of `tools/bitlex` and
     the masks' compile pass it by: the other executors take it in steps 3 and 4.
   - `bitlex_rust` writes the root's loop as lean's: `mark()` before each statement, then `statement()` and
     `settled(save.w)`; when either fails, `recover(save)`. `Mark` holds every field a statement changes that the
     parser has: the record index, the comment cursors, the tape's length, the levels, the level's state, the
     word, and the strings open over a template. `reset` truncates the tape and the levels and drops the dropped
     nodes past the mark. `recover` scans by the clauses and writes the ERROR leaf (the tape's `CODE_ERROR`). The
     comments inside it are its text. Then it goes on.
   - A limit is no syntax: past the depth, `limited` is set, and `recover` refuses the file. Tier 1's errors,
     escapes and local checks refuse before tier 2 runs, as before.
   - The provisional field `bare` is checked at each statement's end (`settled`), not at the root's close.
   - Only a grammar that declares `recover` gets it: Kotlin's and JSON's generated Rust is unchanged.
   - `comma_rule`: a suppress line's `,` goes on only before a string, as lean's. The code's 16 mark bits are full,
     so it is declared after `io_bare`, the last mark that holds one. A token mark there holds no bit, and tier 2
     tests its tokens alone, as lean peeks (`bitlex_masks`'s `code_marks`).
   - `io_bare` names every token lean refuses after `.input` or `.output`.

   Tests: `a_statement_that_fails_is_one_error_leaf` (a case for each resume rule, comments in and after the leaf,
   a `.test` block, a bare term, and the suppress line's `,`; past the depth, refused).
   `corpus_edited_tapes_are_the_lean_tapes` (run by name) parses the corpus and lean's edited copies of it: each
   text the calls form takes, whole or with statements recovered, has lean's tape, node for node. Each other
   text is refused for a limit or for tier 1. `deep_nesting_ends_in_a_refusal` covers the limits.
3. The device loop. Done, as above: `statement_loop` writes both forms' `run`. The tests above hold the one loop's
   result to the calls form's on each text, taken, recovered or refused.
4. The interpreter (4a) and the JIT (4b). Done.

   **The words.** `recover` lowers to `STATEMENT` and the `recover at|past [first]` rows (`Words::recover`, a token
   mark's elements each). The root's words are not one loop state: `include` is read at the root's own level, and a
   statement's node is still open at the state after it, where the next token decides to close it. So the heads are
   the root's states where it may end, each with `STATEMENT` right after its `TOKEN`, where both returns land. At a
   head the levels are the root's and the statement's before, whole.

   **The interpreter (4a).** A refusal is syntax (`refuse!`, a break out of the word) or a limit (`limit!`: the
   depth, a count, a token's or a local check, tier 1's), which refuses the file.
   `STATEMENT` first fails the statement before where it holds a provisional field, then marks: the record, the
   tape's length, the open levels' entries (a few at a head), the pending extras and their `keep`, the last end, the
   word, flags, counts, strings, the moved node and the dropped flag. A refusal by syntax with a mark cuts back to it:
   the tape truncated, the wraps' chains back to their heads before it, the levels as they were; the statement before
   closed at its last token, as its `CLOSE` does; then the scan from the statement's second record, the extras
   skipped (`recover_holds`, a token mark's test; `first` reads tier 1's line ends between the record before and it,
   a word at a time), one ERROR leaf (`ERROR_LEAF`, the tape's `CODE_ERROR`) from the first token to the last before
   the resume point, and on at the root's first head's `TOKEN` from the record after it, which takes the extras
   between. A token mark that refuses is held where the grammar recovers: a level that reads the token ahead, to end,
   may be the statement's before, and the statement that takes the token is the one that fails. So the held refusal
   comes where the token is taken, at the next `TOKEN` that takes one, or at the next `STATEMENT` after its mark,
   whichever is first; a recovery clears it.

   **The JIT (4b).** The mark is `Ctx::mark`: set, held, the state's 32 fields (`Mem::ALL`) and the open levels'
   entries, at most 4 (more: no mark; a head has 2). `STATEMENT` checks the provisional fields of the rows since the
   mark, then marks where the stream goes on, then refuses a held token. A refusal's value says whether it is a limit:
   a bit a word's place (`Recovery::limits`: `PUSH`'s depth, `COUNT`, `CHECK` and the checks a switch's edge makes at
   the check's place, `EMPTY`, `ACCEPT`; the machine stack's depth; out of room past the bits). `recover_statement`
   restores the fields but the tape's own (its codes, last block's base and wide flags), takes the codes back from the
   mark's node (`uncode`) and makes the last block's base again (`cut_to`), puts the open entries back, closes the
   levels above the root with `CLOSE`'s word, scans the records in CLIF (each pattern a token mark's test,
   `token_pattern`, with `Next` at the record; `first` a word loop over the line ends, tier 1's stored mask or the
   mask from the bytes), and writes the leaf with `LEAF`'s word, its row's kind `ERROR_ROW` (0x3ff, which no grammar's
   kind takes), whose code the tape's code table writes with no kind and `CODE_ERROR`. In one function the refusal
   block recovers and jumps to the root's head; in clusters every refusal returns to the driver, the state in `ctx`,
   and the driver recovers and calls the head's cluster again, at an entry of its own: a loop, a turn a statement.
   The wraps' chains need no cut-back: the root's heads make no wrap, so a wrap in a statement wraps the statement's
   own nodes. Not compiled where tier 2 makes the records or splits a region, or for format 16 (none of them is
   rr's): `compile_with` refuses those.

   Tests: `corpus_edited_tapes_are_the_lean_tapes` holds the interpreter's tape to the calls form's on each edited
   text, and each JIT's to the interpreter's (a function a rule, one function, one machine): taken, recovered or
   refused alike, over all 9,691 texts (6,035 taken whole, 2,971 with a statement recovered, 685 refused). The rr tests
   that once asked a refusal of a statement's syntax ask lean's tape with an ERROR leaf of each executor
   (`reads_as_lean`), or the same refusal of every executor for a limit.
5. Tier 1's errors as lean's `Bad` tokens, so a byte no token takes and a region open at the end recover too. Done.

   **The grammar.** `rr.bitlex` declares lean's `Bad` as a token, `bad`: the bytes no other token begins with,
   outside the regions (`class other`), a run of them one token (`run other`; lean's are one a character, and the
   ERROR leaf over them is the same). No state takes it. A region kind takes `open K` (bitlex's `Region::open`): an
   opener with no closer after it anywhere in the file is a token of kind `K` to the file's last byte, as lean's
   lexer makes the rest of the file one `Bad`. The string, the block comment and the literals of one and of three
   backticks are `open bad`; the line comment ends at the file's end already, and two backticks with no closer are
   the empty literal. The three `error … by tokens` lines for a region open at the end, and the one for a byte no
   token takes, are gone. `K` is the lexer's error token: `any` and `balanced` leave it out, as lean fails a
   `.type`'s line, an `.input`'s parameters or braces at a `Bad`.

   **A quote after a backslash.** Lean reads a backslash outside a string as a `Bad` and the quote after it as a
   string's opener. The regions read escapes before they know where the strings are, so a string's opener was
   `q`, a quote not escaped, and such a quote opened none. It is `quote` now: any quote outside the regions opens a
   string, and the escape is read only for the closer (`str = quote .. q`), as lean's string reader reads it. Every
   text tier 1 took before reads the same: a backslash outside a string was refused.

   **Tier 1.** The reader's arm with `open` reads `ReadNone::Bad(k)`, `k` the `bad` token's record kind. Where it
   finds no closer it ends at the file's last byte, and the token is `k`'s: no word, none of the reader's marks,
   none of its kind's tests (the `\u` check, `json_bad`'s hits). The mask form's last byte of such a region is its
   fill at the last byte (`TokenRule::last`), so Metal's lasts hold it.
   - The interpreter: `read_end` ends it, and the record pass gives it its kind (`Reader::open_kind`: the arm whose
     opener holds at the start, and no closer's first byte where the last says, a bit or two read again).
     `check_hits` gives it none.
   - The JIT's tier 1: `read_none` jumps to the token's end with the kind as its tag, as a reader's retag.
   - The generated Rust (`bitlex_lex_rust`): the reader's `None` arm sets `open_bad` and ends at `n - 1`; the record
     is `bad`'s kind, no word, no bits, no tests. In one pass (decision 0619) it waits for every block, as `else`
     does.
   - Metal: `record_bad`, the same test from the opener's and closer's bits, gives the record pass its kind and the
     marks' tests none.
   The `bad` record has the punctuation's kind, whose switches go by word: with no word, every switch refuses it.

   **The loop's main run.** `bad`'s run has more first bytes than the identifier's, and the token loop is shaped
   about the widest run (`main_run`). The main run is now the widest one a word may begin, as a lexer's identifier
   arm: `bad` is rare, as a lexer's error arm.

   Tests: `tier1_reads_leans_bad_tokens` holds, on each case of `bad_token_cases` (a run of bad bytes, each region
   open at the end, in its first block and blocks past it, a backslash before a quote, an escape in a string open at
   the end), the `bad` record, the CPU's records to the interpreter's, Metal's to the CPU's, and each executor's
   tape to lean's with its ERROR leaf. The generated Rust's records and tapes are held to the JIT's on the same
   cases. Over the 9,691 edited texts: 6,035 taken whole, 3,640 with a statement recovered (2,971 before), 16
   refused (685 before), each at a limit: 11 at tier 2's `\u` check, 5 at tier 1's escapes.
