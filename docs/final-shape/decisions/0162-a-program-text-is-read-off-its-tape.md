# 0162: A stored program's text is read off its tape, in one walk

Status: built (2026-11-13). Amends decision 0155 §1 (how `read` reads the text).

## Intro

Decision 0155 keeps each program a run compiled (prepared programs, plan choices, the combined program) as `.rr`
text, and reads it back into a `front::Program`. It read the text with the statement parser: lex the text into
tokens, parse the tokens into `syntax::Statement`s (each name an owned `String`), then map each statement to the
program's part, copying each name again.

The lean parser (`rerail_rr_syntax::tape`, docs/parser-generators/30) already reads the same grammar into the tape
image a rule file is stored as, with no statements made. This record reads a stored program off that image directly.
The image is walked in place, and each node becomes the program's own part as it is reached. Nothing sits between:
no `Vec<Node>` decoded from the image, no tree indexed over the nodes, no statements.

```
// 0155: text -> tokens -> statements (owned strings) -> program (strings copied again)
// this: text -> tape image (lean parser) -> program (each name copied once, into its part)
```

## Decision

### 1. `program_text::read` walks the lean parser's image

`read(text)` makes the image with `rerail_rr_syntax::tape(text)` and walks it with `program_text::from_tape::parts`:

- **The image is read in place.** `rerail_tape_format::View` reads a node's key, subtree size and span from its
  columns, and `View::children` steps from a child to its next sibling by its size. Nothing is decoded ahead of the
  walk. `decode` makes every node at once and stays for the readers that want that.
- **One node, one visit.** A clause's head, an atom's relation and arguments, and an aggregate's operator, value,
  keys, separator and body are found in one pass over the node's children, by field.
- **A name is copied once**, into the part that holds it. A relation's name is interned straight from the source
  text (`Name::from(&str)`), with no `String` made first. A string is unescaped only when it holds a `\`.
- **A fact row lands without a search.** `print` writes a relation's rows together and the relations in order, so a
  row is most often the last relation's (`BTreeMap::last_entry`).

What the walk accepts and refuses is what `read` over the statements accepted and refused. It accepts what `print`
writes. It refuses a disjunction, a pattern match, a float, a `mean`, a modal head, an edit, a setting other than
`.rules` and `.about`, and a statement that does not parse.

### 2. The statement reader goes

`read` has one path. The statement parser stays for rule files, which lowering reads.

## Measured

The 141 program texts a warm dl1 run reads (6.18 MB), in a release build (opt-level 3) of the readers alone. Both
paths make the same parts (declarations, inputs, outputs, facts, rules, the `.about` rows). Instructions and cycles
are per pass over all 141 texts, whole-process counts less a run that only reads the texts. Load was 87.

| | instructions | cycles |
|---|---|---|
| statements, then parts (0155) | 1,752 M | 462 M |
| tape image, then the walk (this) | 1,359 M | 335 M |
| of which the lean image | 462 M | |
| of which the walk | about 900 M | |

That is 22% fewer instructions and 27% fewer cycles. The walk at first took 957 M. Two causes were found in its
profile and fixed:

- a B-tree search, and a key `String` made, for every fact row;
- a `String` made for every relation name before it was interned.

What is left of the walk is mostly the program's own form. In the walk's profile, `malloc` and `free` are 592 of 1,635
samples, and dropping the result is more. The walk's own steps (finding children, reading spans) are about a quarter.
`front::Program` holds every variable as its own `String` and every argument list as its own `Vec`. Both paths pay
that, and the old path paid it twice. A program form that borrows or interns its variable names would be the next
step, and is not taken here.

Of the lean image's 462 M, lexing is about a third (1,163 of 3,341 samples). The lexer is already simdjson's shape:
nibble-table classification into 64-byte masks, pair tables for the two-byte operators, an escape finder, and
keywords found by a length-masked 8-byte load and a perfect hash. The recursive-descent parser and the tape writer
are the rest.

End to end, dl1 (the 44 data-layer policies) on a fresh store, debug build, CPU, strict SQL, at load 60 to 83:

| | instructions | cycles |
|---|---|---|
| first run | 191.4 G | 62.4 G |
| warm runs | 85.1–85.2 G | 29.0–29.1 G |

All three runs gave the same 48 findings. These numbers are from a debug build and decide nothing here; the release
numbers above do.

## Tests

- `program_text::tests::pack_programs_read_back_from_their_text`: every 7th pack rule's program is read back
  exactly, through the tape.
- `program_text::tests::the_tape_reader_refuses_what_print_never_writes`.
- `lean::tests::a_view_reads_each_node_as_decode_does`: every `.rr` file of the repository, and an image with
  4-byte columns.
- Before the statement reader was removed, both readers gave the same program on all 141 texts.

## Prior work consulted

- **Decision 0155:** what is stored, and the format of the text. This changes only how the text is read.
- **Decisions 0061, 0140:** the tape image, and its writer as the lean parser writes it.
- **docs/parser-generators/30 and 31:** the lean parser and its micro-lexers.
- **Lessons ledger 1:** derive on demand from the compact source. Here the walk goes from the text's image straight
  to the program, with no form kept between.
