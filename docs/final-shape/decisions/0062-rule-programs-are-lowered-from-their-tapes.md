# 0062: A rule file's program is lowered from its tape

Status: built (2026-11-07)

## Intro

Decision 0061 gave `.rr` files a tape. Until this record, the program a rule file runs still came from a second
output of the same parse: the parser built its statements beside the tree, and lowering read the statements.
Nothing ran from the tape, and nothing kept a rule file's parse between runs. Decision 0060 kept it within a
process only.

This record makes the tape the one thing lowering reads. A rule file is a source in the content store, like any
corpus file. Its tape is stored by content and grammar version, and its statements are read off that tape. A run
then reads a rule file's tape from the store instead of parsing it, and rules about rules (decision 0061) read the
same tape the program was lowered from.

```
rule.rr ──► content store (source, by sha256)
                │
                ▼
          tape (rr, tag 13) ──► statements (from_tape) ──► lower ──► Program
                │
                └──► facts (node, field, text, …) ──► rules about rules
```

## Decision

### 1. Statements come off the tape

`logic::rr::from_tape::statements(src, nodes)` reads decision 0061's tree back into the statements lowering takes
(`syntax::Item`, `Lit` and `Term`). Each statement, literal and term kind maps to one form. Texts come from the
source by the nodes' spans:
- a string is unescaped as the lexer does;
- a backtick literal loses its fences;
- a number's text is read with its sign;
- a setting's text runs from its first argument to its end, as before.

A tape with an ERROR gives the error running the file gave before: the parser's message for its first bad statement
(`rr::syntax_error`).

### 2. Every read goes through it

- **`rr::read`**, the read of each file a program includes, puts the file in the content store and takes its tape
  from there (`facts::core::build_cached`, label `rr`). A tape stored by an earlier run is decoded, not made again.
  Without a store (the browser build, or `RERAIL_TAPE_CACHE=off`), the tape is made in memory and read the same way.
- **`rr::parse`**, a program given as text, makes its tape in memory and reads it.
- **Decision 0060's in-process caches stay in front:** statements by path and digest, then the combined and prepared
  programs. The store's tape is what a new process starts from.

### 3. The parser's own statements are the oracle

The parser still builds statements as it builds the tree, because its control flow reads them: whether a term
becomes an atom, and whether a negated literal is a comparison. Nothing outside tests uses them now.
`statements_from_the_tape_are_the_parsers` checks every `.rr` file in the repository (1,130): from the tape and from
the parser, the statements are equal, and so are the errors. The parser keeping statements is O107.

## Measured

The converted-rules replay gives its recorded findings (total 3988 303 298 0) in 10.27 s user + 2.18 s sys at load
64. Before this record it took 8.19 s + 1.87 s at load 84. Some of the difference is the store: each rule file's tape
is read from the content store and decoded, where the parser's statements were used directly. The two runs are one
each, on a loaded machine, so the cost is not yet attributed. Storing the lowered program (below) is what removes the
per-process reading.

## What this does not do yet

- **The lowered program is not stored.** Lowering, prune, demand and the split still run per process (decision
  0060's caches). Storing the lowered program as facts derived from the tape, memoised per source, is the next step.
  It needs the lowered program's atoms to be the store's, which decision 0060's prepared cache already checks.
  Decision 0073 takes that step: the prepared program is a facts-memo entry under its text's source.
- **Lowering itself is not a rule program.** `lower` is Rust over statements. Making it rules over the tape's facts
  would let a rule be checked against what it compiles to. That is not needed for the cache, and it is not done.

## Prior work consulted

- **Lessons ledger 1:** the compact tape is the source of truth, and everything else derives from it. Here a rule
  file's program derives from its tape.
- **Lessons ledger 25:** restoring a stored derivation beats redoing it (20 to 26x for tapes). A rule file's tape is
  restored like any other.
- **ast-index-spike history index L919-920:** key by what a value is a function of, and make the language part of
  the key. A rule file's tape is keyed by content, the `rr` tag and the grammar version, and the grammar version
  hashes the parser (decision 0061).
- **ast-index-spike history index L417 (DH L2185-2189):** "storing data in memory... is dishonest". Decision 0060's
  caches were per process; the store now holds the tape across runs.
- **Decision 0061 §7:** names this step and leaves the stored lowered program to a later one.
