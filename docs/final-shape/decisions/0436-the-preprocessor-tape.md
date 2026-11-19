# 0436: The preprocessor tape: one parse a file, an evaluator over tapes

Status: decided (user, 2026-11-18: "Would it make more sense to define a preprocessor tape format
and make a lean and bitlex/metal parser for it? Then we would have two phases, the parser and the
evaluator, and the evaluator would operate over tapes. Might be much simpler"; "go ahead").
Replaces 0430 (deleted at the user's direction: no setup in Rust, no array-program format).
Numbered past the highest number in any worktree (0435).

## Intro

A preprocessor does two different kinds of work.

- **Reading a file**: where its logical lines are, which lines are directives, each directive's
  kind and operands, how its conditionals nest, and every token of its text. None of this depends
  on the configuration. It is the same for every translation unit that includes the file.
- **Evaluating a translation unit**: which groups are taken, which files are entered, what each
  name means where it stands, and what each macro call becomes. All of this depends on the
  configuration and on the files before.

So the work is split into two phases with a tape between them:

1. **The parser** reads a file's bytes once and writes its **preprocessor tape**. The tape is
   kept by the file's content, so a header included by a thousand units, or a thousand times in
   one unit, is parsed once. There are two parsers that write the same tape: a lean one (Rust,
   the reference) and a Bitlex grammar compiled to the transducer's tiers (the interpreter,
   Cranelift, Metal).
2. **The evaluator** reads tapes, never raw text, and writes the unit's output. It runs on
   Metal as parallel passes in plain MSL (the user's direction, 2026-11-18):
   - includes, as rounds;
   - the definitions' lookups;
   - the conditionals;
   - the macro calls, expanded in rounds;
   - output offsets by a prefix sum;
   - the output by a scatter.

   The CPU only loads files and their tapes between include rounds.

This replaces two tapes, the directive tape (`06-raw.bitlex`, 0327 mode (a)) and the token tape
(`tokens.bitlex`, 0348 §1), with one tape.

## The tape

The tape is a v9np document (`rerail_tape_format`, format 15). There is no new format and no
side column, so every tape reader works on it. Its root holds the file's items in order. An item
is a directive, a conditional group, or a text line.

```
pp
├─ line                      a logical line of text: its tokens
│   ├─ identifier            foo
│   ├─ punct                 (
│   └─ …
├─ define                    #define F(a, ...) a + 1
│   ├─ name: identifier      F
│   ├─ params: params
│   │   ├─ param: identifier a
│   │   └─ variadic: ellipsis
│   └─ body: body            the replacement list's tokens
│       ├─ identifier        a
│       ├─ punct             +
│       └─ number            1
├─ group                     #if … #endif, nested
│   ├─ if                    expr: expr (its tokens)
│   ├─ line …                the first branch's items
│   ├─ elif / else …         each further branch, then its items
│   └─ endif
└─ include                   spelled: string | header | macro_header
```

- **Tokens** are leaves of the tokens grammar's kinds, in every part of the file:
  `identifier`, `number`, `literal`, `punct`, `digraph`, `hash`, `hashhash`, `other`. A
  punctuator is read by longest match in a directive too, as the lexer reads it. A `#define`'s
  body is the tokens its expansion copies.
- **A token's flags** are the lean lexer's: `BOL` is the first token of its line node, and `SPACE`
  is a blank byte before it, read from the source byte before the token.
- **Directives** are named by their word, as `06-raw` names them (`null`, `line_marker`,
  `unknown` otherwise). Their operands are fields: `name`, `params`, `param`, `variadic`, `body`,
  `spelled`, `expr`, `text`, `first`, `second`, `n`, `file`, `rest`. A directive's word and an
  implied `#` are not written: the kind implies them (format 14).
- **Groups** are nodes from the `#if`, `#ifdef` or `#ifndef` to its `#endif`. Their chain is the
  group's own directive children (`c_cond_next`), and the directives are those children. An
  `#elif`, `#else` or `#endif` with no open group is a directive at the level where it stands.
- **A text line** is a logical line (with `\`-newlines spliced) that is not a directive, holding
  its tokens. Comments are blanks, never nodes.
- **C and C++** differ in a few tokens (C++'s `::`, `.*`, `->*`, `<=>`). A tape is kept by the
  file's content and its language.

Mode (a)'s rows (`c_directive`, `c_define`, `c_include`, `c_cond`, …) are read from this tape
in place, as they are now read from the directive tape.

### Names: keys made by the evaluator

The evaluator must compare names across files: a use in one file against a definition in
another. The key is a 64-bit hash of an identifier's bytes. It is made by the evaluator's first
pass, one thread per identifier leaf, with word loads of the source bytes. Where two keys are
equal, the bytes are compared, so a collision never merges two names. A key column in the tape
was considered: it would add a column to the format, and it can be made from the tape and the
bytes on the device in one pass.

## The two parsers

- **The lean parser** (`rerail_c_pp::tape`) writes the tape with the tape writer
  (`rerail_tape_format::Writer`). It is built from the lean scan (`directives`) and the lean pp
  lexer (`rerail_c_lean::pp::PpLexer`), which are already held to `clang -E` through the serial
  pass (0327). It is the reference.
- **The Bitlex parser** (`rerail_c_directives`, `bitlex_pp/pp.bitlex`) merges `06-raw`'s
  directive tree with `tokens.bitlex`'s token rules. It refuses what it does not read yet, and a
  refused file goes to the lean parser, so a tape is never wrong. It runs on the interpreter,
  on Cranelift, and on Metal's tier 1, with tier 2 compiled on the CPU over the device's
  buffers (`tier2_on`). Tier 2 on the device is a later step. Since tapes are kept by content,
  the parser is not on each unit's critical path.

## The evaluator (later sections of this decision, written as each is built)

It is plain MSL in `rerail-metal`. It reads tapes and source bytes from device buffers that the
CPU filled with mapped files and kept tapes. The host encodes dispatches. Sizes come from
indirect arguments that kernels write. Rounds are bounded loops over a device work queue. The
host never reads a length or a verdict to decide the next step. Its passes, in order:

1. **Names:** a key per identifier leaf.
2. **Includes:** each `#include` matched to the unit's file table, with instances laid out by a
   scan per depth. Files not yet loaded go back to the CPU, which maps them, gets their tapes
   (kept, or parsed now) and starts the next round.
3. **Definitions:** sorted by (key, position), looked up by search, with `push_macro` and
   `pop_macro` as rows.
4. **Conditionals:** each test evaluated by its own thread; a chain's taken arm and each group's
   activity by segmented scans.
5. **Calls:** names joined to the definitions at their position; parentheses matched by a depth
   scan; the outermost calls by a max scan.
6. **Expansion:** rounds, one level each, every call at once, each round's count, scan and
   scatter; it stops when nothing expands.
7. **Output:** count, prefix sum, scatter.

The 0421 spike (`rerail_c_pp::stream::parallel`, host Rust) stays as a cross-check of the
passes' arrays (user, 2026-11-18). The serial pass is the reference for the output.

## Requirements, each held by a test

| | Requirement | Test |
|---|---|---|
| R1 | The tape is a v9np image every reader reads; no new format version. | `the_pp_tape_is_a_v9np_image` |
| R2 | The tape's tokens, their bytes and their flags are the lean lexer's, in every part of the file. | `the_pp_tape_has_the_lexers_tokens` |
| R3 | The tape's directives, operands and groups give mode (a)'s rows, equal to the lean scan's. | `the_pp_tape_gives_mode_a_rows` |
| R4 | The Bitlex parser's tape equals the lean parser's, or the file is refused; on the corpus, 0 differ (interpreter, Cranelift, Metal tier 1). | `the_corpus_pp_tapes` (ignored) |
| R5 | The evaluator reads tapes and source bytes only: no host step between passes except loading files. | a guard test over the evaluator's encoder |
| R6 | The evaluator's output equals the serial pass's on the corpus. | `the_corpus_on_the_evaluator` (ignored) |

## Steps

1. This decision, and its README row.
2. The lean parser, with R1–R3.
3. The Bitlex grammar, with R4.
4. The evaluator, pass by pass, each pass checked against the serial pass and the spike's
   arrays; R5.
5. R6 on the corpus, then timing through `quiet-run`, warm, with the GPU and the host apart.
