# 0612: A file's lines have one definition, its language's ecosystem's, and every place is read off them

Status: built (2026-11-22). The user asked for it (2026-11-22), on span places taken from several line
splittings: "lines and columns have ONE definition, and we have a line index in the tape already", "of the
existing linting tools and editors, tell me what is the convention. We want to match that", and "we need to match
the convention of the ecosystem, so it needs to be language-specific". They chose one relation: `src_line` is the
language's lines, the lines `line(n, l)` numbers.

## Intro

A finding's line and column are what a user sees in an editor, a linter's report and a compiler's message. Before
this record the engine had five line splittings. The facts' `LineTable` split at `\r\n`, `\r` and `\n` for most
languages. `core_rr::Lines` split as `str.splitlines()` does. `src_line_lf` split at `\n` alone. `tape_line` and
`split_line`/`lf_line` were further relations over those. A span's place (its `line`, `end_line` and `col`)
was taken from whichever splitting its relation was made with. So one byte could be on two different lines,
depending on which relation reached it.

Now there is one definition. A file's lines are its language's (`Terminators::of`), the convention of that
language's compiler, linters and editors. Every place of a node, token, comment, match or line is computed from
its byte range by `LineTable::place`. The CPU, the JIT and the Metal kernels read those lines in place.

## What each ecosystem ends a line at

The conventions were read from each tool's own definition:

| Set (`Terminators`) | Ends a line at | Whose convention | Languages |
|---|---|---|---|
| `Newlines` | `\r\n`, `\r`, `\n` | the LSP specification, CPython's tokenizer, Ruff, pylint, editors | Python, the JVM languages, PHP, C, C++, Swift, HTML, Markdown and the rest |
| `Eslint` | those, U+2028, U+2029 | ESLint, TypeScript's scanner (ECMAScript's LineTerminator) | JavaScript, TypeScript, a file of no language |
| `Lf` | `\n` | rustc, `go/token` (a lone `\r` is inside a line; one before a `\n` is left out of the line's text) | Rust, Go |
| `Roslyn` | `\r\n`, `\r`, `\n`, U+0085, U+2028, U+2029 | Roslyn | C# |
| `Css` | `\r\n`, `\r`, `\n`, form feed | CSS Syntax Level 3 | CSS |

The two ambiguous cases the user named are settled the same way in every set where `\r` ends a line. A lone `\r`
ends a line. `\n\r` is two line ends, a `\n` and then a `\r` (as in LSP, ESLint and CPython). Older clang read it
as one; no current tool does.

## What a place is

`LineTable::place([s, e])` is `(line(s) + 1, line(e) + 1, s - start(line(s)))`. The `line` is the line holding the
first byte, and the `col` is that byte's offset in it. The `end_line` is the line holding the end offset, so a span
that ends with its terminator ends on the next line. That is core.rerail's statement, ESLint's `loc.end` and
Ruff's. The old `Loc` view read `line(e - 1)` instead. It is now `line(e)`, the same as every other reader.

`src_line(d, l, sp)` holds every line, including the empty one after a final terminator. Those are the lines
`line(n, l)` numbers. `line_count(d, k)` is what a linter counts (ESLint's `max-lines`, pylint's
`too-many-lines`). It is `src_line`'s count without that final empty line, and 0 for an empty file
(`LineTable::counted`).

## Built

- `Terminators` has five sets: `Lf`, `Roslyn` and `Css` are new. Each language's adapter names its own
  (`TERMINATORS`), and `kit.rs` checks it. A capture's line in evidence uses the file's language's set, no longer
  `Newlines` for every file.
- `core_rr::Lines` is removed. So are the relations `src_line_lf`, `tape_line`, `split_line` and `lf_line`, and
  `Extra`'s rows of them. `tape_line`'s two readers (`python/misc2-noqa.rerail` and the data-layer dead-code
  overlay) read `src_line`.
- A ported check that numbers `str.splitlines()`'s lines reads the library's `py_line` instead
  (`lib/data-layer/pub-lines.rerail`, a rule over `src_line`). These are the data-layer overlays
  `admin-async-purity`, `bootstrap-cache-health` and `executor-publication-repairs`.
  - `py_line_sep` now tests each rest of a line, not only the line's whole text. So a line holding two extra
    separators splits at both.
  - Two checks count `\n` as the upstream SQL does. They count it in the text with `@count_matches("\n", s)`
    rather than through a line relation: `plan-optimizer-v2`'s PV2-044 and `bootstrap-engine-invariants`'
    first-occurrence line.
- `Locs` (the core shape's places) and `Extra` (the T shape's) place every span with `LineTable::place`. A guest's
  places use its host's lines through its placement (`Locs::place_lines`).
- The views' reads take the lines from the `LineTable`'s words where they lie in their slab room: `[n, starts,
  ends]`. These are the CLIF `Loc` reads, written once and lowered to the JIT and to MSL (`view.h`).
  - `lines_line(at)` bisects the starts. `lines_word(i)` reads one word. Both are in the lines family, so a view
    read reaches the lines through their own base (`ll`), not through the chunk's.
  - `ViewDoc` no longer carries a copy of the lines. The chunk's `flines` serve the `line` and `Loc` views, and
    `fill_locs` makes no line starts of its own.
  - A document whose lines are not placed on the device counts `views.refused.lines_unplaced` and has its views made.
- The granules' `ExtraSpan` kinds are `Line`, `Comment` and `Match`. `MATCH_SLOTS` is 14, so `EXTRA_SLOTS` stays 16
  and node ids do not move. Line positions moved by one per file: the empty final line is a line.
  `definition_ids_are_pinned` records the new digest and why.
- The tape's line index is the language's lines. Before, it held each `\n` alone. `Terminators` is now
  `rerail_tape_format::newlines`'s, and the engine re-exports it. The index records the last byte of each
  terminator of the document's set, so a terminator's bytes belong to the line they end. A line's text end is
  that byte, less the terminator's length, which comes from the bytes before it (`length_ending_at`).
  - Every producer passes the set it writes for: the format 16 writer and its reference, `Sixteen`, the engine's
    tree ops, the JSON parser and the C preprocessor. The preprocessor's is `LINE_ENDS`, the C adapter's
    `TERMINATORS`.
  - Bitlex's tier 1 marks the bytes a terminator of its grammar's set can end with (`last_bytes`), as one class.
    The index keeps each one where it ends a terminator (`ends_at`). The set is carried on the grammar's
    `Symbols` and recorded with the output (`BlockProgram::newlines`). A parser uses those bits only for that
    set, and otherwise scans. A single class leaves the hot loop's registers as they were
    (`the_hot_loop_keeps_simdjsons_utf8_and_error_registers`).
  - A token loop's tier 1 (`.rr`'s) makes no mask of its own. Its line ends are among `records_made`, and its
    index is the scan's.
  - `newlines.rs` is part of the grammar build id, so stored tapes are made again.
- There is one line index: the tape's. `LineTable` reads it and holds no lines of its own. It used to hold
  `[n, starts, ends]` words. Its reads are the tape's (`reads::f16::line`, `line_start`) over a view whose
  `lines` is the index's start.
  - A document with a format 16 or 16z tape made with its language's terminators reads the index where it lies
    in that tape (`TapeImage::lines`, `LineTable::of_f16`), the tape held.
  - Any other document has an index of its own in the same format. Where the device reads tapes, it is placed in
    a slab's room (`LineTable::placed`, `newlines::index_in`).
  - A line's text end comes from the source's bytes at its terminator (`end`, `text_of`).
  - Tree-sitter's rows are `Lf`'s lines (`Points`).
- The device reads the same index.
  - The lines' reads (`lines_line`, `lines_start`, `lines_count`, `lines_select`) are the tape's line reads'
    bodies, with the index at their base (`ll`). They are written once in CLIF, into `<ukl/reads/lines.h>`.
    `lines_word` is removed.
  - Format 15's branch of the line reads is removed, in CLIF and in the kernels' `tape_body.h`. A format 15 slab
    places its language's index beside its tape, where `\n`-only line starts were.
  - A `Loc` view places a line from its start, which its text's end shares.
  - The chunk's `flines` (`fill_lines`, the listed copy of 0353 step 4) now hold the index. `chunk_views` reads
    it with the generated reads, not a search of its own.
- The packed batch's line starts (in its listed copy, 0353 step 7) are the document's lines, read off its index. They
  were its `\n`s.
- The data-layer pack's test data follows. The overlays were edited, the converter's output was regenerated
  (`convert_the_data_layer_pack`), and the 50 changed files were staged in the lint tool-pack's `publish/`
  (`npm run fixtures:stage`).

## Tests

- `lines_end_at_each_ecosystem_s_terminators` checks each set's lines over one source holding every terminator.
  `a_place_is_read_off_the_lines` checks `place`.
- `a_terminator_is_known_by_its_last_byte` checks each set's terminators, their lengths from the last byte, and
  the candidates `ends_at` keeps. `the_newline_output_is_every_newline` checks tier 1's candidates on every grammar,
  over lone `\r`, `\r\n` and `\n\r`.
- `a_files_lines_are_its_languages` runs a program per language (Python, JavaScript, Rust) over files holding each
  ambiguous ending. It checks `src_line` and the line of a call.
- `source_lines_are_the_languages_lines` checks `src_line` against the `LineTable`, `py_line` against W's
  splitlines lines, and `line_count`.
- `stored_tapes_are_admitted_into_slabs_and_read_in_place` reads each line's start off a format 15 slab's placed
  index. `the_lines_reads_are_these` holds `lines.h` to its CLIF.
- `views_made_on_the_device_are_the_hosts_rows` (Metal, with a guest and without) checks that the `line`,
  `end_line`, `col` and `docof` views read in place on the device are the host's rows. It caught `loc_word`
  reading the lines' words from the chunk's base, which `lines_word` fixed.
- The data-layer convert tests pass over the staged data: the pack comparison with the converter's run, the
  replay sample and the reference graphs.

## Prior art

LSP 3.17 ("Position": `\n`, `\r\n` and `\r` end a line), ESLint's `SourceCode` line breaks (ECMAScript's
LineTerminator), Ruff's `LineIndex` and CPython's tokenizer, rustc's `SourceFile` and `go/token.File` (`\n`),
Roslyn's `SyntaxFacts.IsNewLine`, and CSS Syntax Level 3 §3.3 (a form feed is a newline). This agrees with them;
it departs from the old engine, which reported one byte on different lines by relation.
