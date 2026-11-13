# 0047a: `core.rr`'s names are the engine's

_Numbered 0047 until 2026-11-13, when 2 decisions shared the number; the letter orders them by when each was written._

Status: accepted (2026-11-06)

## Intro

`core.rr` declares every relation the engine makes: `node`, `kind`, `docof`, `text`, `file`, `col`,
`option`, and about a hundred more. A rule file reads them. Before this decision, a rule file could also
declare its own relation with one of those names, and nothing stopped it:

```prolog
// a rule file, after core.rr (which declares docof(n: loc, d: number) as an input)
.decl docof(n: node, d: number)     // a second docof
docof(n, d) :- node(d, n, _, _, _).  // derived by the program
```

The program then had two `docof`s, the engine's and its own. The planner named the columns of a relation
called `docof` as the engine's (`n, d`), but the program's table had the columns of a derived relation (`c0,
c1`). The SQL failed: "t4.d names no column in scope". T's programs did exactly this, and nine planner tests
failed after decision 0046 made `docof` an input (open item O81).

## Decision

1. **A name `core.rr` declares belongs to the engine.** No other file of a program (the rule file, what it
   includes) declares it or derives it. The reader (`logic::rr::read_file`) refuses such a file with the
   file, the line and the name:

   ```text
   rules/x.rr:12: docof is core.rr's: a program reads it and cannot declare or derive it
   ```

   This is Soufflé's own rule (a relation is declared once) applied across `core.rr`, which every program
   reads first.
2. **The planner names a column by the input's name only for an input.** A relation the program derives has
   columns `c0`, `c1`, … whatever its name (`plan::Model::col`). The lowering itself still derives some
   engine relations: `kind`, and `docof` declared over nodes, are projections of `node` (`rr::lower`); those
   rules are the engine's, and their tables are derived tables.
3. **The engine owns T's programs.** `typing/programs/*.rr` and the pinned copies in
   `logic/plan/golden/programs/*.rr` are edited here. They now read `kind` and `docof` as inputs, as every
   rule does, instead of deriving their own. They are read without `core.rr` (`rr::parse`) and still declare
   the base relations they read, in `core.rr`'s shape.

## Namespaces: where this leads

This decision closes the engine's names. The same problem waits one level down, and the data-layer rules
coming from More-ergonomic-rule-syntax will meet it:

- **Library against rule file.** `lib/python/*.rr` defines helpers; a rule file that includes one may not
  reuse a helper's name. Today only the test-time check `rr::check::declarations` finds a clash. The
  libraries avoid it by hand, with prefixes: `bb_immutable_q` in `bugbear.rr`.
- **Rule file against rule file.** A wave runs many programs over the same files (decision 0024). Two rule
  files that each define a helper `call_name` mean different things by it. Running them together needs the
  names kept apart.
- **Facts the engine adds later.** Every relation added to `core.rr` takes a name from every rule file. A
  rule written today with a helper called `scope` breaks when `core.rr` gains `scope`.

The first steps to think about, not decided here:

- **Soufflé's components.** `.comp` and `.init` are Soufflé's namespaces: `.init py = Python` makes
  relations `py.call`, … Our rules are Soufflé only (decision 0043), so a namespace should be one Soufflé
  reads. The engine already names families with a dot (`py.*`, `facts::vtab::syntax::PY`).
- **Three levels.** Engine (`core.rr`, closed by this decision), library (one namespace per library file,
  public by declaration), program (private: a rule file's helpers are its own, so the runner can rename them
  when it merges programs into one wave).
- **What the runner does.** Merging programs into a wave already has to rename or share helpers. A
  program-private namespace makes the rename mechanical, and lets identical helpers be shared.

## Evidence

- The nine `logic::plan::tests` that failed with "names no column in scope" pass again (`logic::` and
  `typing::`: 89 passed), the replay gate
  still gives 3988 295 290 0, and no library file the replayed rules include redefines a `core.rr` name
  (the reader would refuse it).
