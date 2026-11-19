# 0431. No guest documents: strings the rules parse are tape values; programs the grammar holds are the file's

Status: decided (2026-11-18), by the user: "there are no guest documents. The CPU and GPU run on exactly the same
data"; "parsed strings are only for annotations and maybe format strings, not all strings; the rules decide them";
"if the rules say 'here is an embedded string, here is its sha and here is its tape', then why can't that be kept in
the facts that the program produces and not changed next time it's run?". On a guest's node ids: (a), a node of a
parsed string is named by its tape ("do a"); and the split below ("okay, I agree with that"). Supersedes 0055's guest
documents (§4, "What was built" 3–4) and 0391 (guest keys). Keeps 0055's model of a tape as content, 0058 (hiding,
normalized strings, `@parse`), 0104 (no SQL in the waves), 0353 (blocks never copied), 0426 (format 16, unchanged).
Work queue Q261.

## Intro

A file can hold code in another grammar, in two ways:

- **A string a rule decides is code.** A Python string annotation (`x: "List[int]"`), a format string, an eslint
  directive in a comment, a regex or JSON value in a `.rr` file. The rules decide which strings; what is inside is
  tested and reported at the string or a host node.
- **A program the grammar holds.** An HTML `<script>` or `<style>`, a Vue component's script blocks, a Blade view's
  compiled PHP, Markdown's inline text, a `.rr` pattern. Whole rule sets (the TypeScript, PHP, CSS packs) run over
  them and report findings at their nodes, at the file's lines.

Until now both were *guest documents*: a document of its own, with a key in its own range, its own node ids and a place
in the chunk's batch. In every chunk the rules that ask for guests were cut out and run first (the "embed slice"); the
guests were parsed; the whole batch was built again with them; and the slice ran again, up to four rounds. On PostHog
(cold, M3, 2026-11-18, 1ca497070) that took 32.1 s of CPU on the CPU binding and 34.4 s on Metal, 18.4 s and 13.7 s of
it building the batch again. The device read a batch the host had rebuilt. A guest's tape was never stored (Q30), so a
file read from the memo had no guests, and a cross-file rule could not see them.

What is fixed, whatever rules run: the host's nodes, a text's tape (a function of the text alone), and each node's
position in its tape. A rule only decides whether a fact like `sub_tape(s, g, T)` exists. So nothing about embedded
code may be named by which rules asked for it, or in what order.

```
// x: "List[int]" = []               a.py; the same line in b.py, at line 4
//
// sub_tape(s, "annotation", T)      s = a.py's string node; T = the tape of "(List[int])", keyed by its sha
// parsed(s, "annotation", r)        r = (T, 1): the tape's node, the same r for b.py's string
// guest_of(n, s)                    n is a node of the tape parsed at s: both files' strings, for each n
// kind(n, …), child(…), text(…)     read from T where it lies
// a finding at s, or at s by n      placed through s: a node of T has no line of its own
//
// <script>let a = f(1);</script>   a.html: the script is JavaScript by the grammar, parsed with the file
// node(d, c, "call_expression")     c is a.html's node, numbered after its own: docof(c, a.html), line(c, 1)
```

## Decision

1. **There are no guest documents.** No document key range for guests, no guest key registry (0391), no embed slice,
   no rounds outside the engine, no batch built again.
2. **A string a rule parses is a tape value.** `@parse(grammar, text)` gives a tape keyed by the sha256 of its text,
   grammar and tape format (0055's key); equal texts anywhere share one. `parsed(s, g, r)` and `sub_tape(s, g, t)` are
   derived by the program's own rules, the parse a functor inside the fixpoint: a rule that reads the tape's nodes is
   in a later stratum than the rule that makes it, and a string inside a parsed string is ordinary recursion. Nothing
   parses a string no rule asked for.
3. **A tape value's node is named by its tape:** (the tape's key, its preorder). The same node for every place, rule
   and program that reaches it. It is no file's: `doc` has no row for its tape, so a finding on it has no path. Its
   `line` and `col` are its tape's own text's, which every view holds for every node, on the CPU and the device alike
   (a node with no row would make the views partial). A file's place comes through the string:
   - `guest_of(n, s)`: n is a node of the code parsed at string s, a row per place;
   - `guest_place(n, s, l, c)`: n, parsed at s, stands at line l, column c of s's file, through s's placement (the
     escapes and parts of a normalized string, decision 0058 §3). A string inside a tape value is placed in that
     value's text, which is no file's;
   - a finding placed at a tape value's node is refused (R9).
   A tape value is never shown as code to rules that do not name a guest (0058 §1's ranges): in the run's language it is
   in the annotations' range, else hidden.
4. **A program the grammar holds is part of its file.** The host language's parse makes it, as the matcher path makes
   its derived programs (`DerivedImage`): what a file embeds depends on its bytes and its grammar, not on rules. Its
   nodes are numbered in the file's id space after the file's own, in the order the parse found them, so `docof`,
   `line` and `col` are the file's and findings land at its lines. The libraries' structural `embed` rules (`html`,
   `vue`, `blade`, `markdown`, `rr` patterns) become the languages' declarations of what they embed.
   How (step 3), under per-definition ids (0083 §3, `granules`): a program's nodes are granules of its file. A program
   granule's key is its host node's granule key, then `embedded:<grammar>#<ordinal>`, then the program's own granule
   key, and it is placed in the file's one slot table with the file's own granules (same hashing, same clash order).
   So an edit inside a script renumbers that script's granules and no other's, and an edit to the file around it moves
   none of the script's unless their keys change. The table answers which tape a slot is of, so a node id resolves to
   (file, program, node) in place. In the batch a program is a document with its file's key and its own tape and rank
   index; it has no span slots (its positions are the file's, through its map) and no `doc` row of its own.
5. **Format 16 does not change** (0426): a tape is one text, one tree, one grammar. A file's programs are each a tape of
   their own grammar; a tape value is a tape. Around them:
   - a file's record of its programs (each one's tape key, host node, grammar, first id, the map from its text to the
     host's bytes), a derived fact stored beside the file's tape and loaded with it;
   - the reads (CLIF, 0310) resolve a node id to its tape and position: a file's node through that record, a tape
     value's node through its key. Written once, for the CPU and the device.
   Languages without a format 16 table (brace and percent formats, eslint directives, HTML, CSS, Markdown) keep format
   15 for their tapes until they have one (0293 §3).
6. **The batch is built once a chunk.** The views (`node`, `child`, `field`, `text`, …) are reads of tapes (0353): of
   the files' tapes, their programs' and the tape values the run's facts name. The CPU and the device read the same
   blocks.
7. **Kept, and not made again.** A file's programs are kept with the file. A tape value is stored by its key, written
   behind the run (0306), and loaded with the facts that name it before the first wave (0104). An unchanged file under
   an unchanged program parses nothing; a file read from the memo has its programs and its parsed strings' tapes, so a
   cross-file rule reads them.

## Requirements, each with its test

| | Requirement | Test |
|---|---|---|
| R1 | A program the grammar holds is its file's: `docof` of its nodes is the file, its lines are the file's; an edit inside one program renumbers no other's nodes nor the file's | `a_program_a_file_holds_is_the_files`, `an_edit_in_one_script_renumbers_no_other_program` |
| R2 | `guest_of(n, s)` names the string a tape value's node was parsed at | `guest_of_names_the_string_a_node_was_parsed_from` |
| R3 | One text is one tape and one set of nodes at every place; a tape value's node is no file's | `one_tape_at_two_places_is_one_set_of_nodes` |
| R4 | No string is parsed unless a rule asks for it | `a_string_no_rule_asks_for_is_not_parsed` |
| R5 | The batch is built once a chunk (`Report::batch_rebuilds` is 0): a file's programs (step 3), tape values (step 4) | `a_files_programs_build_no_second_batch`, `tape_values_build_no_second_batch` |
| R6 | A text a run parsed is not parsed again: after an edit elsewhere in its file, in a new file, after a rule's edit (`Report::guest_parses`) | `an_edit_elsewhere_parses_no_text_the_last_run_parsed`, `a_new_file_with_a_text_parsed_before_parses_nothing`, `an_edited_rule_parses_no_text_the_last_run_parsed`; guard `equal_texts_are_parsed_once_a_run` |
| R7 | A file read from the memo has its programs and tape values for a cross-file rule | written with step 4 |
| R8 | CPU and Metal give the same rows, the device reading the same tapes | the Metal path's parity tests, with both kinds |
| R9 | A finding placed at a tape value's node is refused: the lowering derives `rr_unplaced(clause)`, and the run fails naming the clauses | `a_finding_at_a_tape_values_node_is_refused` |

The tests for R1–R4 are in `logic/rr/tests.rs`, R6's in `logic/plan/corpus/tests.rs`, ignored until their step lands (`#[ignore = "0431 …"]`). 0055's tests
keep their meaning, rewritten for `guest_of` and the file's programs where they read `guest(g, …)`.

## Steps

1. The record, its tests, Q261.
2. Tape values: node ids by tape key and preorder, `guest_of`, findings placed through the string (R2, R3, R9); the
   packs' `docof(n, d), guest(d, _, s, _)` over parsed strings rewritten (14 uses in 2 files; the `guest` rows left
   are the grammar's programs, step 3's).
3. Programs as the file's: made by the host's parse, numbered in the file's space, the file's program record; the views
   and the reads (CLIF) resolve ids through it; `with_matchers` runs once a chunk; guest documents go (R1, R5, R8).
4. `@parse` as a functor in the fixpoint; `parsed` and `sub_tape` derived by the program; the embed slice, `DEPTH`,
   `GUEST_ROUNDS_KEY` and 0391's registry go (R4). Tape values stored and loaded before the waves (R6, R7).
5. PostHog cold on CPU and Metal against 1ca497070's 32.1 s and 34.4 s of guests, findings unchanged (29,747,
   fa0efeaed6ff).

## Open

- A value's tape (`@parse(g, x)` of a computed string, `parse_of`): a tape value like any, placed through its asking
  node. To check against the packs' one use.
- `embed_value` (step 2): a library's value placed at a node, as `embed` is, so the second kind: its file's, placed at
  its node. Its one use in the packs is a `.rr` regex.
