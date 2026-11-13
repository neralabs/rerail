# 0155: A stored program is its `.rr` text, read straight back into a program

Status: accepted (2026-11-13). Supersedes decision 0074 §1–3 and §4's facts entry.

## Intro

A run keeps what it compiled: the prepared program (its `front` and `lowered` programs and what preparing found),
the plan choices it made, and the combined program of a batch. Decision 0074 stored each of them as rows of a fixed
schema, one row per tree node: a rule, each body literal, each atom, each argument, each term. The rows were printed
as a `.rr` document of facts, and kept again in a compact binary form for reading back.

That was the form decision 0026 forbids ("nothing may store one row per node"). It was measured as such:

| A warm dl1 batch keeps | 0074's rows |
|---|---|
| prepared programs | 46.7 MB of rows (4.1 MB after zstd) |
| plan choices | 17.4 MB of rows |
| the printed rows, as documents | 36.5 MB of text |

Reading the rows back is a join per program over a million rows. It was about 12% of the warm batch's main thread,
and about 18% with the plan choices and the store opens around them.

A rule is already text with a parser. This record stores each program as the `.rr` text of its rules, and reads that
text back with the rr parser straight into a `front::Program`. Nothing is lowered again, and nothing is joined.

One rule shows the difference. This is a rule of the data-layer pack as lowered, printed:

```
// One body literal per atom, comparison or test, as the lowered program holds them.
bch_allowed(d, l) :- src_line(d, l, sp), text(sp, rr_text24), strtest_6(rr_text24),
    d < 1610612736, sp < 6917529027641081856.
```

That is about 120 bytes. Decision 0074 stored it as 35 rows, printed as 1,184 bytes:

```
// 0074: the same rule as rows (a few of them). `p` names the program, numbers are ids.
rule("lowered", 812, 4301).
body("lowered", 812, 0, 9120).
lit_atom("lowered", 9120, 4302).
atom("lowered", 4302, "src_line", 0, 0).
arg("lowered", 4302, 0, 20811).
term("lowered", 20811, "var", 0, "d").
…
```

## Decision

### 1. A program's text is the `.rr` text of its own parts

`logic::program_text` prints a `front::Program` as `.rr` text (`print`) and reads that text back as the same
program (`read`). It is not `rr::print`: that prints a program as source, to be lowered again, so a test reads back
as `match(…)` and a native relation as the construct that made it. This text is the program as it is:

```
// Illustrative: the shape of a stored program's text.
// Declarations, inputs and outputs, as written for any rule file.
.decl src_line(d: int, l: int, sp: int)
.decl bch_allowed(d: int, l: int)
.input src_line
.output bch_allowed
// The program's facts, in their order.
strtest_6_pattern("bootstrap-cache-health: allow").
// Then its rules, in their order: an atom per atom, `!` a negation, `@f(…)` a call,
// `v = count : { a(…) }` an aggregate, `nearest(n, a, set)`, `ast(d, n, "matcher", NAME: v)`.
.rules
bch_allowed(d, l) :- src_line(d, l, sp), text(sp, rr_text24), strtest_6(rr_text24), d < 1610612736.
// What has no syntax of its own, as facts: which negations are gated, the native relations, the
// gates, the kind sets, the magic-sets report, the settings and policies, the regexes and patterns.
.about
gated(14, 2).
native_reach("reach_dom", "dom_seed", "dom_edge").
```

`read` parses the text with the rr parser, without a tree (`syntax::Parser::new(…).statements()`), and maps each
statement to the program's own part: a declaration to `decls`, a clause before `.rules` to `facts`, one after it to
`rules`, a fact after `.about` to the part it names. That is one walk over the statements. No relation of rows is
built, sorted or joined.

Two kinds of names are written apart, so the text reads back exactly:

- A variable named `top`, `count`, `min`, `max`, `sum`, `product`, `mean`, `join`, `by` or `sep` (the parser reads
  those as words) gets a trailing `_`. So does one that already ends in `_` after one of those names, so the escape
  can be undone.
- A relation named `nearest` or `ast` is escaped the same way: those names are literals of their own in this text.

`read(print(p))` is `p`, field for field. A program `print` cannot write exactly (one with maintenance's `again`,
`together` or `shapes`) is refused, not written in part, and the caller keeps it unstored.

### 2. What is stored

Every stored program is a **source** of the store, by its sha256, like any rule file. The same program stored twice
(`front` equal to `lowered`, one plan choice equal to another) is held once.

- **Prepared** (decision 0073): the pointer entry, under the program text's source and the run's plan, holds small
  facts: `prepared_program(name, digest)` for `front` and `lowered`, and the `prepared_*` rows of what preparing
  found (domains, operator atoms, interned atoms, the split's relations, the text tests). No program row is in it.
- **Plan choices**: each choice's entry holds its program's digest, beside its own facts (inlined, demanded, the
  scale).
- **Combined**: the entry holds the program's digest and the files it read.

Reading one back reads the entry, then each named source, and `program_text::read`s it. A source that is gone makes
the lookup miss, and the run prepares again, as before.

### 3. The tape is made only when a rule reads the syntax

The text is a `.rr` document, so the store can tape it the way it tapes any rule file (decision 0061). Reading a
program back does not need the tape: the parser's statements are the program. So no tape is made when a program is
stored or read. When a rule asks for a stored program's syntax, the tape is made as it is for any rule file, and the
syntax relations (`node`, `field`, `text`) are served from it (`facts::vtab::syntax`).

### 4. 0074's relations are a view, made only if a rule needs them

The relations of 0074 §1 (`rule`, `body`, `atom`, `arg`, `term`, …) are no longer stored. No rule reads them today.
Only the test `a_rule_reads_a_program_as_facts` does. `program_facts::write` stays as the function that makes those
rows from a `front::Program`, and that test makes them from the program it reads back. If a rule comes to need them,
they are served as virtual relations over the stored program. They are computed when a rule names them, as the
syntax relations are, and kept nowhere.

The reading half of `program_facts` (rows back to a program, its in-place `decode_view` and the part documents of
`PART_ROWS`) goes. Its readers of the metadata rows stay, for `.about`.

## Measured

To be filled in when built: bytes kept per warm dl1 batch, the lookup's share of the main thread (inclusive samples,
`incl.py`), instructions and cycles of the warm batch with the load, and the findings (48 on dl1 and dl10).

## What this does not do

- **Lowering is still Rust.** The lowered program is written by the engine, as before (decision 0062's open item).
- **The `prepared_*` facts are still rows.** They are small: per relation or per intern call, not per node.

## Prior work consulted

- **Decision 0026:** nothing stores one row per node. 0074 did, and this record undoes it.
- **Decisions 0061, 0062:** a rule file's tape, and its statements read off it. A stored program is a rule file's text
  and is read by the same parser.
- **Decision 0074:** what it kept stays: a program in the store as data, readable by a rule. Only the form changes.
- **Lessons ledger 1:** the compact source is the source of truth, and the rest is derived from it on demand. The text
  is kept, and the rows and the tape are made from it when asked for.
- **ast-index-spike design index S112 (the compiled artifact):** a compiled query is an artifact of its own. Here it
  is a `.rr` text, which a person can read too.
