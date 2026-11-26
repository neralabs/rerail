# 0631: Format 16 is the engine's one tape: format 15 and everything that made or read it are removed

Status: built (2026-11-22). The user asked: "please merge the hub / then remove all format 15 infrastructure"
(2026-11-22). On how a language's numbering is chosen, they said: "I think it has to be grammar. We don't have
"exhaustive" corpora, so how would specializing to a corpus possibly be correct?" So every table is made from
its grammar.

The commits of this work (b4a1562740 to 5b16cfb50b) name it 0626. That number was taken in another worktree
(`0626-a-semicolon-after-a-line-break.md`), so the record is 0631, and the code cites 0631.

## Intro

A tape is a document's syntax tree as bytes: each node's code, its span and its subtree, read where it lies
(0293, 0353). Until this record the engine had two layouts:

- **Format 15** was columns at byte widths. It had a header and its own code table per tape, which held only
  the codes that tape used.
- **Format 16** (0156 §7, 0434) has fixed blocks and checkpoints. Its codes are numbers in the language's one
  table, and it has no table of its own.

Format 16 had become the stored and resident format, but format 15 was still written, converted and read in
many places:

- the lean parsers' writer;
- the compiler's tier 2 mode;
- the engine's conversions;
- the CLIF and device reads;
- the preprocessor, the rule files' parser and simdjson's index writer;
- the lab.

Two layouts meant two readers for every question, conversions between them, and a second code numbering.
Format 16 alone is written, held and read now.

## Every language's table is its grammar's

A format 16 code is a number in the language's table (`tables/<tag>.tsv`, shipped by tape_format). The lab
writes each table from the grammar, not from a corpus (`tape16 table`). The table holds every kind, field and
supertype the grammar can give, and the names the lean parsers number their keys by (`f16::Names`). No corpus
orders or bounds it, so a file of any shape is written. Every table's revision is 2. Revision 1 was ordered by
captures.

## What was built

**The engine** (b4a1562740):

- `Format` is F16 and F16Z alone, and `RERAIL_TAPE=15` is refused.
- Gone:
  - the conversions (`in_format`, `converted`, `keeps_15`, `DocTape::tape_in`);
  - `Columns15`, `Held::F15` and the packed records;
  - every format 15 arm.
- A stored image that is not format 16 or 16z is refused.
- A slab's plan is a format 16 tape, and its table is required (`Resident.table`, `TableAt`).
- The label, join and wave kernels are told format 16.
- The test language TestHtml (tag 250) has its own table.

**The lean parsers** (385288cca1, 3af3cc05a8):

- Each of the 17 crates writes format 16 through its own sink (`Named`). It numbers its keys in the
  language's table by their names (`Lang::number_named`).
- The engine's adapters number them the same way (`formats::lean_named`), one mapping a language. The
  adapters' run-time grammar lookups (`ids()`, `code_of`) are gone.

**The rule files, the preprocessor, the directives and Kotlin** (f0deb08368, 5b16cfb50b):

- rr_syntax, c_pp, c_directives' grammars, rr_parser's and kotlin_parser's tier 2 write format 16 alone.
- Their tapes are read in place.

**The lean writer** (`tape_format::Writer`):

- It writes format 16 only, through `f16::Writer`'s ops.
- Its columns are only the edit window since the last commit (the code column is `u16`).
- Its target is `Result<Sixteen, String>`: an Err is the reason no tape is written.
- Gone:
  - format 15's header (`MAGIC`, `FORMAT_VERSION`, `HEADER_BYTES`, `COLUMN_ALIGN`, `CHECKPOINT_STRIDE`,
    `TAIL_FROM_START`);
  - its image (`preorder_image`, `image_bytes`, `finish`, `Columns`, `place`);
  - its decoder (`decode`, `View`, `Children`);
  - its widths' offset column and its code classes.

**The tape API** (`tape_format::tape`):

- `F15`, `F15Columns`, `f15_view`, `f15_columns` and `AnyTape::F15` are gone.
- `reads::f15` is gone, and `Tape::parent_within` too: only format 15 used the hint. The engine's
  `parent_within` and the granules' `holding` go with it. The syntax view's parent is `facts.parent`.
- f16's `write_from`, `write_from_with_presence` and `numbers` are gone. They wrote a format 15 tree as
  format 16. Tests and the reference writer take nodes (`nodes_of`).

**The CLIF and device reads:**

- The view reads dispatch to 16z where the view says so, and otherwise to format 16 (`ukl.rs`; `view.h`
  regenerated).
- `tape_f15.h` and the format 15 constants in `tape_common.h` are removed.

**simdjson:**

- The index writer (the DOM route, on a CPU no kernel runs on) gives format 16 events, as a kernel's walk does
  (`events_from_index`).
- The test now compares the two word for word, so the index writer writes an empty string, object or array as
  one leaf, as the kernels have always written them.

**The lab** (`native/lab/tape16`):

- The engine's `capture-tapes` writes format 16, and the lab reads its captures in place by the tape's own
  reads (`captured::read`, replacing `v15`).
- The in-place baseline every candidate is set against is the captured tape (`E16`, replacing `F15`).
- Format 15's writer is gone from E6 (`writer`) and E28 (`f16w`). E6's events writer takes the captured tree's
  nodes, and its lean parse is compared to the capture's tape without the presence after it.
- zlevel reads the captured tape and writes nodes through `f16::write`.
- The lab's workspace patches rayon-core as the repository does, so `--features lean` builds again.

**The tests:**

- Each format's tape is made by a run of it, never converted.
- The admission tests write broken trees through format 16's writer or edit the stored bytes.
- The writer's tests are the format 16 writer's:
  - `leaves_inserted_at_once_are_an_insert_each`;
  - `a_leaf_inserted_later_is_the_tape_in_order`;
  - `a_parsers_writer_finishes_as_its_edits_make_the_tree`;
  - `a_format_16_writers_columns_are_the_window`.
- An inserted leaf's start never goes back in preorder (format 16's bounds).

## Records this changes

- 0293: the one API over the tape has one format below it, 16 (and 16z).
- 0340: there is no format 15 to fall back to.
- 0434: format 16's writer is the only writer.
