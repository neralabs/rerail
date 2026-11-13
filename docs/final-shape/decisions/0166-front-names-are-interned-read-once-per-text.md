# 0166: A program's names are interned, and a stored text makes each distinct name once

Status: built (2026-11-13). Follows decision 0162 (the tape walk). Amends decision 0155's text reader only.

## Intro

Decision 0162 reads a stored program's `.rr` text off its tape image. What was left of the walk's cost was the
program's own form. `front::Program` held each variable, string constant, function name and column as its own
`String`. So reading a text made a string for every occurrence of a name, and every pass that copies a program
copied each string again.

Decision 0074 stored a program as facts in a compact encoding: a string table, then each value a tag and a number
(`program_facts::encode`). Decision 0155 replaced that store with the `.rr` text. The text has no string table, but
it does not need one. The walk builds the same table as it reads, one entry per distinct name. Each later
occurrence is a lookup.

```
// before: x(a, b) :- y(a, c), z(c, b).   six variable Strings, three relation Names
// now:    the walk meets "a" once, interns it, and every later "a" is a clone of that Name (a count, no allocation)
```

## Decision

### 1. Every name in a program is a `Name`

`front::Name` is the process-interned `Arc<str>` that relation names already were. These now take it as well:

- `Term::Var`, `Term::Str`, and `Term::Call`'s function;
- `Value::Str`, the string in a fact row;
- `Program::decls`, each relation's key and each column's name and type: `BTreeMap<Name, Vec<(Name, Name)>>`.

A `Name` reads as a `str` (`Deref`, `Borrow<str>`, comparison with `str`, `String` and `&str`), so lookups by
`&str` still work. It also serializes as a string. The other parts of a program (`inputs`, `outputs`, `facts`'
keys, `nullary`) stay `String`s. They are few, and many callers build them.

### 2. The walk interns from the text's spans

`program_text::from_tape::Walk` keeps a table per text, `&str` span → `Name`, for the length of one walk:

- A variable, a relation, a function, a declaration's name and its columns are each looked up by their span in the
  source text. Only a name not yet in the table goes to the process interner (one lock).
- A string constant is interned from its span when it holds no `\`. Only one with an escape is unescaped into a
  new string first.

This is decision 0074's string table, built at read time from the text rather than stored beside it.

### 3. What still allocates

The walk now makes 260k allocations over the 141 texts, down from 797k when decision 0162 was built. What
is left is the program's vectors, one for each:

| | count |
|---|---|
| argument list | 90.5k |
| declaration's column list | 52.5k |
| fact row | 26.9k |
| rule body | 22.4k |
| distinct name per text (process interner hits are free) | the rest |

Reaching "almost none" would mean an arena form of `front::Program`, or argument lists held inline (a small-vector
form). That changes every pass that builds or rewrites rules, so it is not taken here.

## What this does not do: SIMD keyword and punctuation matching

The statement lexer (`rr_syntax::syntax`) finds punctuation by its first byte and borrows its tokens' text (commit
9fbb25274). That is a scalar stand-in. SIMD matching of keywords and punctuation is left to the tree building the
Bitlex parser. Its shape is the one the lean lexer (`micro.rs`, `lean.rs`) already has:

- simdjson's nibble-table classification (`vqtbl1q_u8`);
- pair tables for the two-byte operators;
- an escape finder;
- keywords found by a length-masked 8-byte load and a perfect hash.

Two things stay open there. One is string masks by prefix XOR, done with a carry-less multiply. The other is the
cost of the per-token step: lexing is about a third of the lean image.

## Measured

Same set as decision 0162: the 141 program texts a warm dl1 run reads (6.18 MB), in a release build (opt-level 3)
of the readers alone. Counts are per pass, whole-process counts less a run that only reads the texts. Load was 44
to 55.

| | instructions | cycles | walk allocations |
|---|---|---|---|
| tape image, then the walk (0162) | 1,359 M | 335 M (load 87) | 797k |
| after sizing each vector to its children | 1,303 M | | |
| after the `Name` terms and the per-text table (this) | 1,186 M | 250 M | 260k |
| of which the walk alone | about 726 M | | |

That is 13% fewer instructions than decision 0162's walk, and 32% fewer than the statement reader (1,752 M). The
cycle counts were taken under different loads, so their change is not exact.

## Tests

- The tests of decision 0162 (`program_text::tests`), which read pack programs back through the tape.
- The front, lowering, magic sets, simplification, planner and typing tests that build programs. They now build
  `Name`s, and compare them with `str`.

## Prior work consulted

- **Decision 0074 §4:** the facts store's string table, which this rebuilds at read time.
- **Decision 0155:** a stored program is its `.rr` text.
- **Decision 0162:** the walk this changes.
- **`front::name`:** the interner already used for relation names.
