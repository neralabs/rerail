# 0337. A dialect's run reads its language's other dialects' files as text

Status: built (2026-11-24). Amends 0201 §1 and the dialect runs `run_program` makes (0104, 0168): each run still
parses and reports its own dialect's files, and now reads the others' files as `.scope text` documents.

## Intro

`.language typescript` names a language of three dialects: TypeScript (`.ts`, `.mts`, `.cts`), TSX (`.tsx`) and
JavaScript (`.js`, `.jsx`, `.mjs`, `.cjs`). `run-rr`, `test-rr`, `run-policies` and the shipped runner
(`rr_host::run`) run such a policy once per dialect with files in scope (`corpus::run_dialects`), each run lowered
for its dialect and writing its own `outputs.rro` (`out`, `out/tsx`, `out/javascript`). Each run read its own
dialect's files and nothing else. A rule of a `.tsx` file could not see a `.ts` file at all:

```
.language typescript
.violation X(at: node) "sees a.ts"
X(n) :- kind(n, "program"), doc(_, "a.ts", _).
```

over `a.ts` and `b.tsx` reported `a.ts` alone. The full-stack TypeScript rules join across exactly that line: a
`"use client"` component (`.tsx`) importing a server module (`.ts`), a component calling a hook defined in a `.ts`
file.

## Why the runs stay apart

A run is one program over documents of one grammar, and this is not incidental:

- A kind or field constant lowers to one grammar's atom: `kind_atom(tag, code)` (`lower::Constants::sym`). The
  TypeScript and TSX grammars number their kinds differently (TSX adds the JSX kinds), and JavaScript is a third
  table. `kind(n, "program")` lowered for TypeScript cannot match a TSX node.
- Every reader of a kind atom compares it in its own grammar: the `kind` column and its postings
  (`syntax::Dialect::canonical`, `codes`), the tree axes' kind sets, the prefilter, the device's code tables.
- A run's documents are parsed in its grammar (`build_doc`'s `core::Source { language: label }`), phase 1 states
  its sources by it, a load of another grammar is refused (`world::load`), and the tape cache, guests and W's
  reference graph are keyed by it.

One program over every dialect's files needs either a copy of every rule that reads a kind or field constant per
dialect (a positive constant as a disjunction of copies, a negated one as a conjunction across dialects, a kind
set as the union of its members in each grammar, a rule body with nodes of two dialects not covered), or one kind
identity for the family remapped in every reader above. Neither is a contained change. Parsing every file in every
run, each in its own grammar, needs the per-file grammar in all the places above as well, multiplies the parse,
the facts and the evaluation by the number of dialects, and still leaves an unqualified kind blind to the other
grammar's nodes (0058 §2).

## Decision

1. **Each dialect's run reads the in-scope files of its language's other dialects as text documents**
   (`corpus::lane_text_dialects`, applied in `corpus::scoped` when the run has the `dialect` option). They are read
   as a `.scope text` document is: `doc`, `path_part`, `source`, `src_line`, `src_line_lf`, `tape_line`, and a
   match's spans over them; no tree, so no node. They are in the run's policy scope as its own files are.
2. **A run reports findings in its own dialect's files, and in files of no dialect of its language**
   (`corpus::lane_reports`, in `tools::run_program` and `tools::run_pack`). A finding in another dialect's file is
   dropped: that dialect's run parses the file and reports it. A doc-level rule (`doc(d, p, _)` alone) is therefore
   reported once per file, not once per run. The lane's own `outputs.rro` still holds every row its run derived.
3. **A split run's `files` is the largest of its runs' counts**, not their sum: every run reads every file.
4. **The facts memo's space names the grammar the run parses its files in** (`memo::space_digest`, v3). Its
   space named the phase and the file-local part's relation names, and an entry is found by its file's content and
   path, so the three runs of one policy shared a space: `a.ts`, parsed by the TypeScript run, was found as text by
   the TSX run, and the reverse. The first build of this record did that, and the JavaScript run of a three-dialect
   tree derived nothing at all (a rule joining its own `c.js`'s `doc` was empty) until `RERAIL_CORPUS_MEMO=off`. A
   file read as text and parsed now has an entry in each run's space. The runs shared no entry worth keeping: each
   parsed only its own dialect's files.
5. **A text document's bytes are held from the run's one read** (decision 0300): the digest read before phase 1
   keeps them for the builders, as it keeps the run's own files', so another dialect's file is read once a run, not
   once for the memo lookup and again for its document.

So a rule of any dialect sees every file of the language: its `doc` row, its path and its text and lines. A rule
that needs another dialect's nodes still cannot have them. A negation over nodes of such a file finds none, as
for any `.scope text` document: a rule that asks "does the imported module say `"use server"`" tests the module's
text (`src_line`, a match over `source`), not its nodes.

## Measured

`run-rr`, debug build, CPU binding, a fresh output directory each run, over 546 `.ts` files and the same 546 as
`.tsx` files beside them (two runs, each now reading 1,092 documents where it read 546), with a rule joining each
`.ts` file to its `.tsx` twin by path and a call rule. The machine was under a load average of 27–35; two runs
each, interleaved:

| build | findings | wall s | user s | sys s | peak RSS MB |
|---|---|---|---|---|---|
| before | 2,098 | 9.73, 9.78 | 27.2, 27.4 | 1.6, 1.7 | 448, 452 |
| after | 2,644 (the 546 twins) | 11.2, 11.3 | 28.7, 29.0 | 2.3, 2.6 | 430, 453 |

About 5% more CPU and 1.5 s more wall for reading every file twice as many times (each as text in the run that
does not parse it), and no more memory beyond the noise. A `.ts`-only tree (one run, no text documents) took the
same before and after (14.5–14.8 s user). Earlier runs during heavier I/O contention showed a larger gap (sys 2.5 s
against 20 s), which shrank to the above once the contention passed; without item 5 the gap was 0.4 s sys more.

## Tests

- `logic::rr::tests::a_dialects_run_reads_the_others_files_as_text`: a `.test` block run as `test-rr` runs it, then
  `run-rr` over the same files. A rule joining `kind(n, "program")` with `a.ts`'s `doc` and first `src_line` is
  reported in `a.ts`, `b.tsx` and `c.js`; a doc-level rule once per file; `files` is 3.
- `logic::rr::tests::a_language_of_several_dialects_runs_over_each`: unchanged.
