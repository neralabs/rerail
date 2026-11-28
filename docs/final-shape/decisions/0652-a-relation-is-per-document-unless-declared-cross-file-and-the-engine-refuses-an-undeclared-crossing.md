# 0652. A relation is per-document unless declared `cross_file`, and the engine refuses an undeclared crossing

Status: decided (2026-11-23), by the user: "these errors should all be found by the engine, not things we need to
discover. Please require some kind of "cross-file" annotation or come up with a sane default so that we don't
accidentally mess up the split. ... The vast majority of things should not need to be cross-file." It amends the
split (0051, 0281, `typing::file_local`), 0290 (`locate`'s moves) and 0649 (the position report). Maintenance
(0328, 0364) is unchanged.

## Intro

A program runs in two parts (0051): the per-file part, derived again for an edited file only, and the cross-file
part, maintained over all files from its inputs' deltas. The split places each relation. Until now it placed a
relation in the cross-file part whenever it could not prove the relation's rules are about one document, and said
nothing.

That silence has cost four rounds of finding slow edits by measuring them:

| Decision | What the split could not see | Found by |
|---|---|---|
| 0267 | a climb reads the tape's parent column, a location | 100 s cold over 50 files |
| 0290 | a relation whose rows name no document turned the memo off | an edit re-deriving every file |
| 0432 | a phase's export, read in a file-local rule | an edit adding a function: 5.3 s |
| 0649 | an inlinable view, and a column filled by a node or `unset` | edits re-deriving 2.6 M rows |

The sweep after 0649 (every pack and data-layer policy, prepared, with 0649's position report) found 85 more
rules in the cross-file part reading positions, in 16 of 66 programs. Traced to the first rule each chain left the
per-file part by:

- 35 are the split's own gaps: views computed over every file's values with several rules (17); closures the
  engine runs natively, counted cross-file whatever their edges (6); min/max over one document's nodes (4); a
  negated conjunction's set of asked values, pooled over documents (4); relations whose only node columns can be
  `unset` (2); `env`, read as a per-document input (2).
- 4 are rule text that joins documents by accident (`pha_latest` matches a write by name and index in any file;
  `pe_ord` keys by node ids as strings).
- About 34 are cross-file by their meaning: import resolution, class hierarchies, symbol closures, facts pooled
  over all files.
- About 12 were not traced further.

A gap and a real crossing look the same to the split ("joins 2 documents", "reads R with no location"). Without
the author saying which crossings are meant, the only safe default is the silent one. With it, every other
crossing is an error.

## Decision

1. **Every relation is per-document by default.** A relation is cross-file when:
   - its `.decl` carries `cross_file`: `.decl resolve_export(q: symbol, r: symbol) cross_file`;
   - or the engine knows it is: it reads an input that names no document (a phase's export, 0432; the files
     beyond the corpus, t21), or it is a natively made relation whose kind spans files;
   - or one of its rules reads a cross-file relation. Readers inherit it and need no annotation.

   The annotation decides where a relation is derived, never its rows: 0083 §0 holds (file boundaries are never
   semantics). It is the same qualifier `locate` sets (0290).
2. **An undeclared crossing is an error.** After preparation's passes, the run's split (`programs_of` on the run's
   path) is checked: each relation in the cross-file part whose cause is its own, not "reads cross-file X", must be
   declared or engine-known. Otherwise preparation fails, naming every such relation at once, its cause, and, for
   a relation the lowering made (`rr_…`), the source relation that reads it. The message says how to fix it: join
   through a location, or declare the relation `cross_file`.
3. **`locate` moves only what may move.** A relation whose rows name no document (0290) is moved when it is
   declared, or when it only places findings (`@place`); otherwise it is the same error.
4. **A test prepares every pack and data-layer policy** as `run-policies` does, without running them, and fails on
   any error of 2. A gap in the split shows up there, not in a measurement.
5. **Positions in the cross-file part become an error**, replacing `RERAIL_STRICT_POSITIONS`, once the engine
   pushes the position comparisons of cross-file rules into the per-file part (0051 §3 extended). Until then 0649's
   report stays.

## What is a crossing

The split decides it, with its own placement (`one_document`), and records each in `FileLocal::crossings`. A rule of
a relation in the cross-file part, not declared, is a crossing unless:

- it reads a cross-file relation: it follows that relation, and so do its readers;
- it reads no document's rows: only the run's inputs and facts (`env`, `p0_file`, `loaded`, a phase's export);
- it reads one document read, plain and positive: each row is one file's, a union (the cross part reading the
  chunks' findings back, `violation(c…) :- violation_at(_, c…)`);
- it places one document once each input naming no document is taken as placeless (`kind(n, …), value(n, v),
  value_type(v, …)`: cross-file for `value_type`, an input of values, not because documents meet).

Documents meet only where a rule joins two reads of them, or aggregates one. That is where the error points: for a
relation of values gathered from every file (`zero_number(t)`), at its reader that joins them with its own document's
rows, not at the union itself.

A program that reads no located input at all (phase 0's loader, over paths) has its path-keyed relations declared:
`p0_from`, `p0_file_t_shape`, `p0_phase_named` (`bootstrap.rerail`). Python's module resolution declares `py_want`
and `py_workspace` (`python/modules.rerail`).

`RERAIL_BATCH_PACK=strict` now does what it says: a failed pack's error is each member's, and none runs alone.

## What step 2 found, and what changed

The pack test went red on 59 crossings, and the engine's own tests on about 40 more. Read one by one, they were:

**The split's gaps, fixed in the split.**

- *Natives* were cross-file by kind (`cross_native`). They are placed by their rules now, as any relation. A
  `match_all` and a relation that chooses an evaluation, not rows (`STRATEGY`, `strategy`: a deferred join's hot
  set, a factorized cache), are *functional*: their rules are checked only for reads of cross-file relations, and
  a file-local reader reads them as it reads a placeless input.
- *A negated read of a nullable column* (0649 §3) took the column's class as a location of its own. Under
  negation it is its anchor's: `!r(n, a)` where `a` is `n`'s nullable column is about `n`'s document.
- *`sub_tape(n, _, t)`* splices a tape value into its node's document: `t` is `n`'s class.
- *`@doc(A) = @doc(B)`* (the JSON translator's same-document joins) is a same-document join (`Lit::Same`), as a
  shared variable is.
- *A lowering helper shares its source's declaration*: a deferred join's factors and keys (`fz…`, `fzk…`), a pushed
  join (`fl…`), an export (`xd_…`), and a guest's copy of a relation (`rr_in_<label>_<rel>`, its rows over a guest's
  documents). A `@place` finding's relation (`rr_place_…`) is meant when it only places findings.
- *A qualifier did not survive printing.* `print_version` wrote no qualifiers, so a program printed and read again
  (the engine's tests, the store's texts) lost `cross_file` with `inline` and `magic`. The printer writes the source's
  qualifiers (`QUALIFIERS`); the words a pass adds (`strategy`, a part's) are not source and stay out.
- *A min or max over nodes* (`OrderNodes`, 0094 §2) read its node back by key, `node_order(v, kv)`. A key names one
  node (0121's keys hold the document), but the split cannot see a document in a number. The helper now carries the
  node beside its key, and the rule reads it back from the helper at the key, with the group's variables bound
  outside the aggregate (TypeScript's `last_decorator`, `decorated`). A min or max with no group (the corpus's
  first `x`) is now seen for what it is, one answer over every file, and is declared.
- *A negation's context* (`rr_ctx…`) was grown through tests: `SECOND != FIRST` tied every comparison of the corpus
  to every node (SIM109, PIE810). A test ties nothing; it is kept where the other literals reach all its variables.
- *The JSON translator's gap to a capture* compared two nodes' offsets with no same-document join. It adds
  `@doc(v) = @doc(c)`, and the generated RUF018 and ISC003 have it.
- *A view its readers cover* (below) is file-local though a rule of it joins documents.
- *A demand that drops a read keeps its tie.* A deferred join's request (0488, `rr_deferred_pairs_x`) is a reader's
  body less its read of `x`. Where `x`'s rows each hold one document's nodes, the request was free to pair any two
  files' (UP008: `super_first(a, f)` asked for by `a` alone, with `f`'s class path: every name met every class path of
  the corpus). The request keeps `@doc(a) = @doc(f)` between the read's location variables its body binds: still a
  superset of what the reader joins, and on one document. The split lists the relations whose rows are each one
  document's (`row_documents`), file-local or not: `super_first` is cross-file while `sym_at` is, yet each row is one
  call's.
- *A context's copy of a relation* (`rr_pass_<context>_<rel>`, 0597) shares its source's declaration, as a guest's
  copy does.

**Views covered by their readers.** A view computed over every file's rows, read only through a join with one
file's row, is evaluated per file. Most of the crossings of the first sweep were this shape, and their rule text was
right:

```text
modattr(m, o) :- node(_, o, "identifier", _, _), text(o, x), imp(m, x).       // joins any two files by a name
bare(e) :- node(d, e, "attribute", _, _), child(e, o, _), node(d, m, "module", _, _), !modattr(m, o).
```

`bare` holds `m` and `o` in its own document, so the only rows of `modattr` it reads are its document's. The split
calls a view *covered* (`covers`) when each reader places the view's location columns from its own rows (or, for a
view of values alone, reads it as a test), and each rule of the view finds its atoms among the reader's (`maps`, a
homomorphism onto the reader's positive atoms, the head onto the read). Per chunk every atom of a rule's instance is
then the chunk's, so the view derived per chunk holds at the reader's values exactly where it holds over every file.
A reader's own head can place the columns where the head is a located view covered in turn
(`fast003_param_eq(n, k) :- fast003_alt_2(n, k)`, placed by its readers). A covered view is in `FileLocal::covered`,
with what placed it (`cover_deps`): a reader's rules are not in the view's digests, so a maintained split asks a
covered view again when one of those relations changed or went (the `retain` pass dropping `param_eq`'s last reader
uncovered `alt_2`).
A covered view of values is read as a test, and listed in `filters`, but it is no string filter: a maintained split
takes an earlier split's string filters only, and reads every covered view of values as a test, in its cone and out
of it, as the whole split does (React's `jsx-no-target-blank`, whose `noreferrer` a maintained split read as a string
filter and the whole split did not).

**Printed qualifiers showed a linked library's loss.** `print_version` now prints qualifiers, and the python library
linked from its modules lacked `no_inline` on covers the library lowered together has. A module is seeded with its
includes' fields and keeps its own by name; its includes' helpers are numbered by their own walks, so an include's
`rr_cover4` named another helper than the module's. Declarations and inputs left helpers out already; qualifiers
now do too.

**Rules the engine accepted with a head variable unbound.** A rule whose head variable no body literal binds made
rows of every value, and the split read it as a join of everything. Preparation now refuses such a rule
(`head_bound`, beside `bound`): a variable is bound by a positive atom, a pattern, a nearest, an aggregate's
result, or a comparison that defines it from bound terms, either way round (`$n = $r`).

**Text that joined documents by accident**, written per document (each one's findings unchanged on its examples):
`purity-calls` (`pc_modattr`, `pc_shadowed` through `docof`), `post-op-side-effects` (`pos_runtime_sig`),
`django-practice-bulk-not-loop` (`bnl_model_is` within its call; the numeric field names apart, declared),
`eslint-no-useless-constructor` (`passes_on`, `same_arg` within their method), `ruff-try301` (`caught_here` within its
`try`), `eslint-plugin-vue-valid-v-for` (`child_for_uses` within its element), `ruff-fast003` (`@doc` joins),
`ruff-ruf016` (its fallback span is the node's own), `phpcs-…-size-functions-in-loops` (`this_changes` within its
loop: every loop of the corpus with every `$this->x` before), `eslint-plugin-vue-require-valid-default-prop` (`prop`'s
"no call" is `unset`, a nullable location, not `0`), React's `jsx-no-constructed-context-values` (a `createContext`
at any file's program root) and `no-deprecated` (`replaced` by the file's own pragma), TypeScript's
`no-useless-constructor` (as JavaScript's), `no-misused-new` (`returns_own` within its type), `array-type`
(`simple_arg`, `shown_arg` keyed by the form, and "no argument" is `unset`, not 0), `explicit-module-boundary-types`
(`followed` in the module's own file), `parameter-properties` (`same_type` within one class), and Python's F402 (a
`__future__` import of the loop's module).

**Crossings that are meant, declared** with a comment saying why: the data layer's `cpl_msg`, `cpl_missing`,
`er_facade_site`, `stale_entry`, `la_eager_delegate`, `annotations`, `static_tests`, `pos_bypass`,
`pos_allow_broken`, `ma_branch_aware`, `csa_owners` (CONN301's count across the surface); `python/references`'
`rf_imported` and `rf_own_fn`; the typing programs' `wantx` (Python), `modchild` and `res` (Rust), `fn_ref` and
`type_of` (the workload: a call no file defines), in their `.rdl` sources, which now carry `cross_file` to `.rerail`; and the engine tests' programs whose subject is a cross-file relation (`uses`, `dep`, `pair`, `shared`, …).

Each side of the merge of 2026-11-25 wrote this part apart. The engine keeps both: the split's gaps closed here (covered
views, a request's tie, `row_documents`) and the policies' text written per document above. Where both rewrote one
policy (CONN301, `django-practice-bulk-not-loop`, RUF016, the `python/references` comments), the text above is the
one kept.

A crossing inside a pack can hide behind another: run with its pack, CONN301's `csa_owners` followed a cross-file read
of another policy's and was not its own cause; run alone it was refused. The pack test shards python-main, which
groups its policies differently, but a crossing that only one grouping hides is still found only by running alone.

The data layer's `data-layer-internal-dead-code` (pulled pack data) still reads `tape_line`, which 0612 removed; its
fixed text waits in the tool-pack's `publish/`, which CI lays over the pulled archive.

**Packs that did not run at all**, each for its own reason, found by the test:

- TypeScript's `triple-slash-reference` bound a regex capture over a node as a symbol; the captures are spans,
  read as text.
- The node and react packs read `tsconfig_for`, which core declares and nothing made (O80). The repository reader
  makes `tsconfig`, `tsconfig_for` and `tsconfig_path` now (JSONC, `extends` chains; one reader with the import
  resolver, decision 0670). A library's own `.input` that core does not make and no fact gives (`npm_ignored`, "until
  core has it") is bound empty, under its declared columns, where a cross-file rule reads it.
- Next's `default_component` ran `match_all` over a field's node, which the span binder did not know: a field's node
  binds spans as a child's does.
- JavaScript's messages with a literal brace (`{{`, as clippy's).
- JavaScript's `one-var` used `ordered by earlier` (`crowded` reads `!crowded` at earlier declarations), which the
  engine never lowered (O73). The claims a scope's declarations make only grow, over three classes, so the first
  claim of each class is found in three rounds of plain rules, and `crowded` reads them.
- Python's `perf403` and Vue's `no-child-content` ran `match` over a node the span binder could not place: the first
  reads the key's text, the second names its element's kinds.

## Amended (2026-11-24, user, at the hub's green pass): demand keeps a left-out atom's document

Merged with the hub (roll-up 125), the check refused a crossing no source wrote. python/compare.rerail's `same_pair`
carries a comparison's asks down to children; `unparen`'s demand from its recursive rule must leave
`same_pair(x, y)` out to stay stratified (it negates `same_leaf`, which reads `value`), and what was left,
`magic_unparen_bf(c) :- same_child(x, i, c), same_child(y, i, d)`, joined any two nodes of the corpus
(`an_answer_the_programs_demand_puts_across_files_is_eager_alone_as_in_a_pack`, #932). Asked whether demand may make
such a crossing, be exempt from the check, or keep the document, the user chose the last:

- **A positive atom demand leaves out keeps its document.** A relation not declared `cross_file` holds rows of one
  document each, so its atom's node variables lie in one. The magic rule that leaves it out keeps
  `@doc(x) = @doc(y)` for each pair of those variables its kept literals bind (`magic::same_documents`). It is still
  wider than the atom, so the demand stays sound, and it is a file's pairs, not the corpus's.
- An atom of a `cross_file` relation says nothing of documents, and leaves no join.

Tests: `a_left_out_per_document_atom_keeps_its_nodes_in_one_document` (the shape, both ways); #932's test, whose own
`twin` (a crossing by `d != e`) is now declared `cross_file`.

The pack-wide test reads each pack from the repository's root: the Django pack moved to `packs/django` (main's #930)
the day this record's test listed it under `docs/rule-syntax/packs`.

## Order of work

1. The qualifier and the check (2, 3), and the test (4). It goes red on the gaps.
2. Each gap fixed in the split, not annotated, until the test is red only on crossings that are meant. Those are
   declared in the libraries and policies, and the two accidental joins are written per document.
3. Pushing positions down, then 5.

## Tests

- A relation joining two documents by a value is refused, naming it and its variables; declared `cross_file`, it
  is accepted and its readers follow it.
- A lowering helper's error names its source relation.
- `a_text_test_over_op_passes_the_batchs_operators`: a text test over `op`'s strings passes the documents' operators.
  `op` is a view (its strings its tags' kind-name atoms, interned by no document's walk), so the batch's vocabulary
  gives them too; `@equals("==")` had passed none, and the rerail pack's NATIVE_MESSAGE_EQUALS was missed. A kept
  count of a numbered relation (`strtest_<k>`, `matchall_<k>`, an `ast` matcher's) is keyed by its definition as
  well as its name: two programs' `strtest_0` are two tests.
- The pack-wide test of 4, one test per pack, in the default tier. Python-main's 537 policies, 142 of which ask for
  guests of their own and run alone, take 84 s of CPU: its test is twelve shards by policy, Python's two and
  JavaScript's (121) three, each under 20 s with the pack tests run together on one host.
- `a_view_its_readers_cover_is_file_local`: a view of values read as a test, and a located view whose reader places
  both columns, are covered; a reader leaving a column open is refused.
- `a_covered_view_is_asked_again_when_a_reader_that_placed_it_goes`: the maintained split is the whole split's.
- `a_covered_view_of_values_outside_the_cone_is_read_as_the_whole_split_reads_it`: an edit outside a covered view
  of values and its reader; the maintained split is the whole split's.
- `library_modules_link_to_the_lowering_of_them_together`: the linked library's qualifiers are those lowered
  together.
- `a_relation_whose_rows_are_each_one_documents_is_listed_cross_file_or_not` (`row_documents`), and
  `a_request_keeps_the_document_its_dropped_read_ties`: the request carries `@doc(a) = @doc(f)`, is file-local, and
  derives the same rows.
- The translator's (`json_rules_pack_is_current`): the generated pack is its output.
