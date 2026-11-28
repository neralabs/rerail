# 0729. A relation names its document, and placement is declared, never inferred

Status: decided (2026-11-28), by the user, on the pages "Crossing Rules After 0727" and "Explicit Documents": "I
would prefer that we require a document type on the rules and make them simple than have an ever-expanding list of
requirements. Explicit is better than implicit", then "that makes a lot of sense. I want this to be encoded as hard
rules, if we need to fix our rules, we need to fix our rules. I agree we need to lock it down with .rerail rules that
we can run over the file". Being built. It amends 0652 (placement), and retires the inference of 0052/0290/0680 §3–4
(located forms, moved unions, relays and read-backs), 0676's keyed-family proofs and 0686 (flags applied last), as
each step below removes them.

## Intro

The split decides from each rule's body whether its relation belongs to one file, and repairs what it cannot prove:
a located form built six ways (`Doc::{Var, Unnamed, Through, ByPath, Wild, Twin}`), relays, read-backs, unions moved
to the cross part. Every new shape of rule has added a case. After 0727 two cases contradicted it outright:

- `asked_at`'s located form was built in the cross part by a `doc` join on the path (`ByPath`), so the cross part read
  a base relation, where `imp`'s own form named the document already.
- `pairs(v, t) :- value_type(v, t)`: a union of a base relation whose rows name no document, which 0680 §3 moves to the
  cross part and 0727 §2 forbids it to read.

The sibling lines met the same gap: 0710 replaced the split's lists of relations by `.decl` qualifiers; 0725's
callables require a column that names the file; RR303 guesses a document key from a column's name (`d`, `doc`,
`document`). The type was missing.

## Decision

### 1. What a document is

- **Its identity is its path** (for a guest, its host's path and the guest's node path, 0722). The document key is
  a function of that alone. It never changes on an edit; a rename is a removal and an addition.
- **Its version is its content's hash**, with the program and the settings it read: the per-file memo's key (0146,
  0063, 0665). It decides whether a file's rules run again, and is held in no row.
- **Its names** (a Python module, a resolved import) are facts its language derives, as relations of the language's
  library. The core names no language (0710).

### 2. The rules (hard: a program that breaks one is refused at load, naming the rule and the rewrite)

1. **`doc` is a column type.** `.type doc <: number`. A `doc` column holds document keys: a variable in one is bound
   by a `doc` column (core's `doc`, `docof`, a relation's declared `doc` column) or by `@doc`, and stands in no
   column of another type but `number`.
2. **A per-file rule is evaluated over one document.** A relation not declared `cross_file` is per file: each of its
   rules joins one document's rows, by construction (the engine keys a chunk's joins by the document, or runs a
   document at a time). Nothing is inferred and nothing is moved. `e(x, y) :- s(x), s(y)` is a pair in one file; a
   pair across files is a `cross_file` relation.
3. **A relation spans files only if declared `cross_file`.** No relation becomes cross-file by reading one. A
   `cross_file` relation's rules read `cross_file` relations, per-file relations that declare a `doc` column, run
   constants and earlier phases' exports. Never a base relation (0727 §2), never a per-file relation without a `doc`
   column, and no `cross_file` relation declares a `node`, `line`, `match`, `token` or `place` column (0677, 0727 §1).
4. **A per-file rule reads cross-file rows only through a declared route.** A `.keyed` setting, an earlier phase's
   `.export`, or a `cross_file` relation declared `staged` (0709, which inferred staging from column types, becomes
   a qualifier). Any other read of a `cross_file` relation by a per-file rule is refused.

### 3. Checked by rules over the rule files

`packs/rr/rr-document-columns.rerail` finds each break in a `.rerail` file without running it (RR320–RR324), as
RR301 does for 0727. Tests run it over its own bad examples, every rule file of the repository and every program in
the tests' sources; each must find nothing once the rewrite is done. The engine's load check is the same rules, and
it refuses.

### 4. What it removes

Location inference for placement, keyed-family proofs (0676), the six located forms, relays and read-backs, moved
unions, the output exception, a rank's located reads, covered-view relays (0708), flags applied last (0686: a staged
flag is read per file), RR303's guess by name. Pushing one-document joins into chunks stays, keyed by the `doc`
column; staging and probes stay; the node and base-relation checks stay, as checks of declared types.

## Steps

1. The `doc` type and the rr rules, reporting; the tests' bad examples. The rule files' and the tests' findings
   counted: the size of the rewrite.
2. The rule files and the tests' programs rewritten until the rules find nothing; the tests then require nothing.
3. The engine refuses at load what the rules find (rules 1, 3, 4), and evaluates per-file rules over one document
   (rule 2).
4. The split reads placement from the declarations; the inference and the repairs of §4 are deleted.
5. The document key made a function of the path alone (today `memo::doc_keys` probes on a collision, so a key
   depends on the run's other paths), and a guest's key of its host's path and node path.
