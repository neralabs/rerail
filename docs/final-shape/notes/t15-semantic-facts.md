# T15 notes: semantic fact producers (O24)

Workstream A, branch `jeremybarnes/final-rerail-shape-semantic` from `35c010235`. Two
producers, `resolves` and `binds`, emit per-document rows. A clause's form of the predicate is a
filter over those rows, and the result is its node label. Both are exact against W's matcher on
every fixture of the Python pack. Runs are under `~/rerail-bench/final-shape/semantic/runs/`, and
each one records the commit and the load before and after it.

## Where they live, and why there

Decision 0011 and O24 direct that "a semantic leaf is a node label produced by its fact producer
(`transducer::semantic_facts`, rows before SQL composition)". The producers are submodules of
that module, beside its slot constants:

| File | What |
|---|---|
| `transducer/semantic_facts/scopes.rs` | A document's `ScopeTable` and usage index under one reading (`reading(python, stub)` keys it as W's `file_table` does), built from the tape once per document, reading and module path |
| `transducer/semantic_facts/resolves.rs` | `rows` (reads, imported names, `__all__` entries, module imports); `admits`; `label` |
| `transducer/semantic_facts/binds.rs` | `rows` (names, bindings, shadows, chains); `admits`; `label` (three-valued, with sub-rule labels joined at the binding's nodes) |
| `…/harness.rs`, `…/gate.rs`, `…/coverage.rs` | Tests: the pack, W as the oracle, the gates, and the coverage count |

**Rows do not depend on the clause.** One pass per document emits every row any form reads. A
form's parameters only filter the rows: the listed names, `prefix`, the typing members, the target
version's builtins, `form`/`scope`/`latest`/`used`, `sameScope`/`sameImport`. That is the
`WHERE` a wave's SQL would hold. 161 `resolves` forms and 33 `binds` leaves read two row sets per
document.

**How these rows relate to N's.** `semantic_facts::{Facts, slot, BindsRequest}` and
`matcher::producer_facts` hold N's rows. N's rows answer one leaf at one node in one capture
environment, with the sub-rules' target nodes in slots (`SITE`, `VALUE`, `ANNOTATION`,
`SHADOWED_SITE`). `binds::BindingFacts` holds the same targets, for every binding, once per
document. Neither replaces the other yet: N's catalog backend still reads `producer_facts`.

**What the rows are read from.** The tape (ordinals, kinds and spans; nodes for fields), plus the
scope table and usage index that `cr/scope` builds from it. Core facts (`facts::core`) are not
read. They have no resolution, and the candidates are walked from the tape's kinds directly.
`Facts::of_kind` postings would serve as well.

**Why there is no registration.** `facts/families.rs` derives four families per path (`tape`,
`core`, `prefilter`, `kinds`) from a path's names alone. A semantic derivation also reads the
reading (the target Python version, and the stub flag from the host path) and the module path,
which no derivation input carries today. Registering it is more than a one-line hook, so it is
O44, not a PROPOSAL commit.

## Forms

The forms were taken from the pack by walking every Python clause's matcher, utilities and
semantic sub-rules included (`harness::walk`). The census: 286 clauses, 199 refused by S1, and
the same first-refusal counts as `match_tree`.

**`resolves`**, by clauses using each shape (several shapes may appear in one clause):

| Shape | Clauses |
|---|---|
| `import` | 115 |
| `global` + `import` | 29 |
| `global` | 11 |
| `import` + `prefix` | 4 |
| `imported` + `relative: text` | 3 |
| `imported` + `seenBefore` | 2 |
| `unresolved: true` (with `languageOptions.python`) | 2 |
| `unresolved: star` (with `languageOptions.python`) | 1 |
| `beforeLocal` | 1 |
| `import: true` | 1 |
| `imported` + `prefix` + `relative: text` | 1 |

All 161 distinct forms (distinct parameters) are covered. Refused by name, none of them in the
pack: a non-Python form, `this`, the `eslintDirective` item, and `languageOptions`, `directives`
or `scriptDeclarations` (TypeScript, PHP).

What the label reads per shape:
- `global` and `import` read a read's row: its head name, whether a binding resolves it and
  shadows, and the qualified name of the import or definition joined with the chain.
- `import: true` on an unbound head reads the builtins.
- `unresolved` reads the five facts of `ScopeRules::unresolved` (bare annotation, deletion, the
  `except` unbinding, implicit names, star visibility), and the `__all__` rows for strings.
- `beforeLocal` reads the usage index's F823 flag.
- `imported` reads the import statement's dotted name, strict and as text.
- `seenBefore` reads the module imports at Ruff's visiting depth and offset, against every node's
  visiting depth. That depth is computed in node order from the tape.

**`binds`**: 33 leaves of 14 shapes in 7 clauses (one clause each of F401, F402, F811 with 22
leaves, F841 with 4, RET504 with 3, F842, SIM113). The shapes are `form`, `scope`, `used`,
`exported`, `latest`, `outer`, `nestedUse` and `bindings`, `site` sub-rules, and `redefines` with
`used`/`exported`/`sameScope`/`sameImport` and its own `site`. All 33 are covered. Refused by
name, none in the pack: `occurrence`, `redefines.every`, and `languageOptions`/`builtin`/
`directives` (TypeScript).

## Gates (W's matcher as the oracle)

The oracle for each form is W's leaf alone, as a clause cloned from one that reads it. It is
three-valued: true where the leaf matches, false where its negation matches, unknown where
neither does. It runs inside a file evaluation (`FileScopeSlot`). Each fixture is read twice:
under no module path (as S1's gate reads it) and inside a package (`pkg.sub.fixture`), which
qualifies definitions and relative imports. For `binds`, the sub-rules' labels at the binding's
nodes are W's verdicts there, standing in for their own lowered labels. Both the true set and the
unknown set must equal W's.

| Run | Fixtures | (form, document) pairs | Matched nodes | Unknown | Differences |
|---|---|---|---|---|---|
| `resolves`, sample (every 8th) | 228 × 2 | 73,416 | 21,310 | 0 | **0** |
| `resolves`, every fixture | 1,817 × 2 | 585,074 | 187,412 | 0 | **0** |
| `binds`, sample | 228 × 2 | 15,048 | 974 | 0 | **0** |
| `binds`, every fixture | 1,817 × 2 | 119,922 | 8,516 | 36 | **0** |

Run: `runs/20261102T160113-gates-every-fixture-at-tip`, at `b1f606c1d`, whose code is this
series' (load 29.6 before, 10.4 after). An earlier run of the same logic, before formatting and
lint fixes (`runs/20261102T152758-gates-all-fixtures`), gave the same counts.

**Mutation checks.** Rows that forget shadowing fail the `resolves` gate: 8 pairs on every 32nd
fixture, for example E721, B006 and B008 on `NPY001.py`, and ASYNC250 on `RUF007.py` (23 extra
nodes). Rows that read every binding as its name's latest fail the `binds` gate: 4 pairs (RET504
on `UP028_1.py`, F401 on `ASYNC212.py`).

What the gates do not exercise:
- `bindings.within` reads a capture's range, and the gate's environment binds no capture, so
  SIM113 is checked with the count read as 0 (O43).
- No fixture is a `.pyi` stub, so the stub reading is checked by construction only (`reading`
  copies `file_table`'s key).

## Coverage of S1's refused clauses

`coverage::semantic_labels_coverage_of_s1s_refused_clauses`: the clause and its utility tables
are rewritten, with each leaf a producer answers replaced by a stand-in label (`Never`). The
leaf's sub-rules sit beside the stand-in, so they still lower as labels of their own. `match_tree`
then reads the rewrite. The stand-in binds nothing, so captures bound by `arguments` and `parse`
are not counted as shared bindings. Those are capture joins (O24's direction).

- 199 of 286 Python AST clauses are refused. 177 read a semantic predicate; 22 read none.
- **86 of the 177 have every semantic input from the two producers**: 81 read `resolves` only, 4
  `binds` only, 1 both. The other 91 also read a predicate with no producer (leaves by clause:
  `value` 35, `arguments` 32, `resolves.binding` 20, `parse` 17, `capture` 9, `formatArgs` 6,
  `layout` 3).
- **With `resolves` and `binds` as labels, 60 more clauses lower** (87 + 60 = 147 of 286).
  The rest are still refused for: shared binding 32, `arguments` 30, `parse` 17,
  `resolves.binding` 16, `value` 14, constraints 13, compare 6, `formatArgs` 6, utility call 3,
  `layout` 2. Shared binding rises from 18, because clauses that stopped at a semantic leaf now
  reach the joins behind it.
- With every semantic leaf a label, 111 more lower (198 of 286). That is the bound on what
  producers alone can add. The rest: shared binding 36, constraints 35, compare 13, utility
  call 3, nthChild 1.

## Cost per document

| Producer | Rows | Rows per document | Time per document |
|---|---|---|---|
| `resolves` | 100,434 on 3,634 documents | 27.6 | 0.20 ms [unverified: load 10–30] |
| `binds` | 150,816 on 3,634 documents | 41.5 | 0.16 ms [unverified: load 10–30] |

Times are from the run above, in a debug build. They include building the table and usage index,
which each producer's gate builds for itself. No run was made at a 1-minute load under 3: the load
stayed above 10 while four workstreams shared the machine. Labels took 1.3 s (`resolves`, 585,074
pairs) and 0.8 s (`binds`, 119,922 pairs). W took 148 s and 25 s for the same verdicts. Both are
from the same build and load, so the comparison is a ratio, not a timing.

## Findings

1. **`binds` is three-valued, and its labels must be too.** 36 nodes on the fixtures are unknown
   (a usage fact the filter reads is unknown). The packed labels are sets, and `not` takes their
   complement, which would read an unknown binding as false. A semantic label needs its unknown
   set as a second label, with `not` complementing both (O42). `resolves` was never unknown on
   the fixtures, and Python's `holds` has no unknown path.
2. **W's `binds` sub-rules are sequential, not Kleene** (code reading, `match_binding_nodes`).
   The first sub-rule that does not hold decides: an unknown `site` before a false `value` is
   unknown, where Kleene's AND would give false. The label keeps W's order. A SQL join that
   ANDs the sub-rule labels must keep it too, or W changes.
3. **`seenBefore` is a label over every node** after the first listed import in Ruff's order. It
   is cheap as a comparison against one per-document minimum, and large as a row set.
4. **`bindings.within` is a join**: the count reads the range of a capture another leaf binds
   (SIM113's `LOOP`). The label takes the capture's range as an input (`Region`). In the wave it
   is a join on the capture's column (O43).

## Where `match_tree` would read a label (sketch, not committed)

No PROPOSAL commit was made. A compiling sketch needs `MatchTree` and `LoweredMatcher` to change
together, in two hot zones. The change would be:
- In `matcher/lower.rs`, `Lowering::tree`'s `AstMatcher::Semantic` arm, a form a producer admits
  (`resolves::admits`, `binds::admits`) becomes `MatchTree::Label(semantic)` instead of
  `Unsupported`. For `binds`, its sub-rules are lowered first as trees of their own, and the
  label's pass joins them at the binding's nodes.
- `transducer/packed/compile/matcher.rs` gains a label source, "a producer's rows", beside
  decision 0011's pattern, roots, walk, complement and empty set. The pass's node set is
  `label(form, rows, …)` on the tape's document.
- Its kinds (`MatchTree::kinds`) are the form's: `reference_kinds ∪ dotted_kinds ∪ string_kinds`
  for `resolves`, `name_kinds` for `binds`. `seenBefore` has none.

## Open items

- **O41.** Producer labels in the lowering (the sketch above).
- **O42.** Three-valued semantic labels.
- **O43.** Capture-dependent filters as joins.
- **O44.** Semantic rows as derivations.
- **O45.** Producers for the remaining predicates.
