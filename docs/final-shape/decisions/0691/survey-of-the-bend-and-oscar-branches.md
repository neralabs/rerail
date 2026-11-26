# File-local / cross-file invariants: survey of the bend and oscar branches

Nov 25, 2026

Both branches enforce two invariants that main does not have yet: every relation is per-file unless declared `cross_file` (an undeclared crossing is a preparation error), and no node id enters the cross-file part. Oscar holds the full node-id check and the symbol rewrites; bend holds a different, engine-side route for the same reports. Main's split is the baseline both started from.

## At a glance

Six invariants came out of this work. Two are hard errors in preparation (I2, I3 once its list is empty); the rest keep the split able to prove that a rule is per-file, so the errors fire only on real crossings.

| # | Invariant | Decision | Enforced by | Bend | Oscar | Main |
| --- | --- | --- | --- | --- | --- | --- |
| I1 | No cross-file rule reads a position (`visit_index`, `node_order`, `line`, `col`, `end_line`, `ast_depth`), except to place a finding | 0649 | Report on every split; `RERAIL_STRICT_POSITIONS=1` refuses | Report; 31 rules still flagged after 0686 | Report (same code) | Absent |
| I2 | Every relation is per-document unless declared `cross_file`, engine-known, or reading a cross-file relation; any other crossing is refused, naming every relation, its cause and the fix | 0652 | `crossings_checked` after preparation's passes, on the run's split | Built, error | Built, error | Absent; no `cross_file` qualifier |
| I3 | No relation of the cross-file part holds a node id, or a value computed from one; cross-file facts are keyed by symbols, paths, document keys and values | 0677 | `node_crossings` on every split; a list that only shrinks; `RERAIL_STRICT_NODES=1` refuses | Absent (0675 says re-keying is "a decision of its own") | Built; list 3,140 to 288 since fix 1 | Absent |
| I4 | A rule binds every head variable | 0652 | `head_bound` in preparation | Built | Built | Absent |
| I5 | Qualifiers (`cross_file`, `inline`, `magic`, `no_inline`) survive printing and linking a program | 0652 | `print_version` writes `QUALIFIERS` | Built | Built | Absent |
| I6 | A crossing is checked per policy alone as well as per pack, because a crossing can hide behind another policy's | 0652, 0665 | `pack_split_tests`, including `..._policy_by_policy` | Built | Built, plus the node list | Absent |

The user's words that set them: "these errors should all be found by the engine, not things we need to discover ... require some kind of "cross-file" annotation or come up with a sane default" (0652), and "rules that don't allow node ids to progress into the cross-file part; facts need to be keyed on something else inside the per-file part" (0677).

## Lineage

Bend is `investigate-bend-vs-datalog` (worktree `Bend-language-investigation`, tip 0bfd6ce1d6). Oscar is `jeremybarnes/sync-integration-hub-tape` (worktree `oscar`, tip 45b397aaa8, with uncommitted work on the tail's next batch). Both last merged main at 92fbff1ccf (2026-11-22); bend is 8,435 commits ahead of it, oscar 8,992, and main 57.

The invariant work is one shared trunk of 2026-11-23/24, then two routes. Newest first:

| Date | Commit | Branch | What it did |
| --- | --- | --- | --- |
| 11-25 | 45b397aaa8 | oscar | 0677 tail 1: the TSX libraries' text helpers keyed by node |
| 11-25 | eb37a998db | oscar | 0677 fix 3g: a `/* global */` comment's items split in its own file |
| 11-25 | 0bfd6ce1d6 | bend | 0686: a flag over every file that gates per-file work is applied last |
| 11-25 | 1d2efb9bd2 | oscar | 0677 fix 3f: the data layer's purity call graph by key, and three engine fixes |
| 11-25 | 01a0f8aa17 | bend | 0675 (proposed): a cross-file relation of names a per-file rule reads is staged before the program |
| 11-25 | b754ec6c93 | oscar | 0677 fix 3e: points-to as the phase `points_to`, the call graph by symbol |
| 11-25 | fcc1ef18c3 | oscar | note t55: points-to by symbol, the design for fix 3e |
| 11-25 | 91ce1bf9c1 | oscar | 0677 fix 3d: a model's `save()`/`delete()` and signal receivers by symbol |
| 11-25 | a2cd36b350 | oscar | 0677 fix 3c: a model's Meta options and managers by symbol |
| 11-24 | ff0924b589 | oscar | 0677 fix 3b: a Django model read by its symbol at a node of the reading file |
| 11-24 | cbf1a4bdf4 | oscar | 0677 fix 3: class ancestry and ancestors' members by symbol; two store sweeps fixed |
| 11-24 | fc57da4d1a | oscar | 0677 fix 2: Python's names through the phase `py_names` |
| 11-24 | d5141c21f2 | oscar | 0677 fix 2's design |
| 11-24 | 8139b88d1c | oscar | 0677 fix 1: `value_type_of`, a value's type is the node's own |
| 11-24 | a8a5f3c101 | oscar | 0677: the check, and a list that only shrinks |
| 11-24 | a19ed32a1c | oscar | 0677 decided |
| 11-24 | 1a9517d64a | oscar | 0676: a scope `nearest` finds is a location; a keyed family of values is per-file |
| 11-24 | 91051b452a | oscar | 0353 row 7: a text read left as rows says why its node is not known |
| 11-24 | bd14a6ac64 | bend | 0672: a view of values its reader holds is per-file; a chunk's guard is its own |
| 11-24 | 94e9aafd78 | shared | 0665 §6: module resolution is the phase `tsx_names`, read as a setting per file |
| 11-24 | fcf40323f9 ... a10802a3f3 | shared | 0665: a file's path places its document; settings read per file (9 commits) |
| 11-23 | 872f3a300b | shared | 0652 follow-up: the split learns the shared patterns; rule text kept as written |
| 11-23 | 7e817aab91 | shared | 0660 (bend) / 0670 (oscar): a project configuration is read once |
| 11-23 | 968c2f4b34 | shared | 0652: per-document unless declared `cross_file`; undeclared crossing refused |
| 11-23 | bb7d504ba8 | shared | the split reports a cross part's position reads; `RERAIL_STRICT_POSITIONS` |
| 11-23 | 4420df69e9 | shared | the split keeps a document's rules in the chunks: computed views inline, `unset` locations |
| 11-23 | 6104a6d562 | shared | decision 0649 |
| 11-23 | ac0e5f8dea | oscar | 0640 corrected: node ids do cross today (35 of 141 cross relations, 11.5 M of 19.8 M rows) |

**Where they part.** Bend makes the engine prove more rules per-file and stage name relations automatically (0672, 0675, 0686), and declares the remaining Django crossings `cross_file` though they carry nodes. Oscar adds the node-id invariant and rewrites the crossings to symbols by hand, through explicit phases (`py_names`, `points_to`), as 0665 §6 did for `tsx_names`. The user confirmed oscar's shape: of 0665 §6, "yes that is it ... for now please record it as decided" (0677's status).

**Numbering.** The configuration decision is 0660 on bend and 0670 on oscar; 0652's text cites each its own. Oscar has its own 0660 (Metal at 512 MB). A port renumbers past the highest number in any worktree.

## I1: positions stay per-file (0649)

A cross-file rule that reads a node's position re-reads most of an edited file's rows, because an edit moves the position of every node after it. On the data layer's 334-file history, a 3–4 file edit re-derived 2.6 M rows through `visit_index` (`rr_agg46`, `ca_alias_at`); after the fix no stratum was re-derived and edits fell from 0.61–1.53 s to 0.43–1.01 s CPU.

**The rule.** `POSITIONS = [visit_index, node_order, line, col, end_line, ast_depth]`. Each read of one by a cross-file rule whose head is not an output is reported with its relation; `RERAIL_STRICT_POSITIONS=1` refuses the program. It is a preparing switch: a program prepared without it is not reused under it. 0652 §5 makes it an error once position comparisons of cross-file rules are pushed into the per-file part; until then it stays a report.

**Node ids were not limited by 0649**: under granule ids an edit changes only its own granules' ids. 0677 later limited them for a different reason (moves and every importer's rows).

**What 0649 changed in the split** (commit 4420df69e9), because the rule that read positions was in the cross part only through split gaps:

1. `InlineFilters` inlines a computed view whose columns are defined by comparisons over its domain (one rule, positive atoms of other relations, comparisons that test or define head variables; a read may leave a column `_`), where the reader reads its domain already. Example: `ca_split(r, h, t) :- ca_dotted(_, r), match("(?<h>[^.]*)(?<t>.*)", r)`.
2. A column filled in every rule by a location or by `unset` (-1) is a **nullable location** (`FileLocal::nullable`). It joins its row's document only where a location column of the same rule places its variable; two rows joined by nullable columns alone stay cross-file. It is kept apart from `locations` and `same_doc`, because a row's document is read off those and -1 has none.
3. The position report itself (bb7d504ba8).

**Ruled out, and why** (useful when someone proposes them again): a third maintenance path "derive again for changed documents" (breaks R9); `earlier(x, y)` as an in-place comparison with no rows (unsound under maintain mode: a flipped order is never removed); order keys an edit leaves alone (every edit here adds or removes a granule); tape-order node ids (`child` delta 1,359 → 62,845 rows, same CPU).

Tests: `a_view_computed_by_comparisons_is_inlined_where_its_domain_is_read`, `a_location_or_unset_column_keeps_a_rule_through_it_on_one_document`, `a_cross_file_rule_reading_a_position_is_reported`.

## I2: per-document by default, crossings declared (0652)

The split used to put a relation in the cross-file part whenever it could not prove its rules were about one document, and said nothing. That silence cost four rounds of finding slow edits by measuring them (0267, 0290, 0432, 0649). 0652 inverts the default: the author says which crossings are meant, and every other one is an error.

**The declaration.** A qualifier on `.decl`, the same one `locate` sets (0290):

```
.decl resolve_export(q: symbol, r: symbol) cross_file   // why it spans files, in a comment
```

It decides where a relation is derived, never its rows: 0083 §0 holds (file boundaries are never semantics).

**The error.** After preparation's passes, the run's split (`programs_of` on the run's path) is checked by `crossings_checked`. Each relation in the cross-file part whose cause is its own (not "reads cross-file X") must be declared or engine-known. Otherwise preparation fails, naming every such relation at once with its cause and rule; a lowering helper (`rr_…`, `fz…`, `fl…`, `xd_…`) also names its source relation. The message:

> the split: N relation(s) are cross-file without a declaration (decision 0652). Join each rule's reads through a location (a node, `docof`, `earlier`), or declare the relation `cross_file` if it is meant to span files

**`locate`'s moves.** A relation whose rows name no document (0290) is moved to the cross part only when declared, or when it only places findings (`@place`); otherwise the same error ("its rows name no document (moved by `locate`), and are read on"). `MOVED_CROSS_FILE` marks a move, so a move is never mistaken for a declaration.

**What a first sweep found** (every pack and data-layer policy prepared, after 0649): 85 cross-file rules reading positions, in 16 of 66 programs.

| Share | Kind | Fix |
| --- | --- | --- |
| 35 | The split's own gaps: multi-rule value views (17), natively run closures (6), min/max over one document's nodes (4), a negated conjunction's pooled ask (4), only-`unset` node columns (2), `env` as a per-document input (2) | In the split |
| 4 | Rule text joining documents by accident (`pha_latest`, `pe_ord` keying by node ids as strings) | Rewrite per document |
| \~34 | Cross-file by meaning: import resolution, class hierarchies, symbol closures, facts pooled over all files | Declare `cross_file` |
| \~12 | Not traced further |  |

The gate's first red run was 59 crossings in the packs and about 40 in the engine's tests; each was read one by one (see Special cases and the Rule fixes catalogue).

**Order of work** that 0652 set and that held: (1) the qualifier, the check and the pack-wide test, which goes red; (2) each gap fixed in the split, not annotated, until red only on meant crossings, which are then declared, and accidental joins rewritten; (3) push positions down, then make I1 an error.

Also in 0652: `RERAIL_BATCH_PACK=strict` makes a failed pack's error each member's, and none runs alone.

## Propagation rules

Cross-file status has three sources and spreads one way, to readers. Only a relation that is cross-file for a reason of its own needs a declaration; everything above it follows with no annotation.

**A relation is cross-file when** (0652 §1):

1. its `.decl` carries `cross_file`; or
2. the engine knows it is: it reads an input that names no document (a phase's export before 0665/0675; the files beyond the corpus, t21), or it is a native relation whose kind spans files; or
3. one of its rules reads a cross-file relation. Readers inherit it. Their cause reads "reads cross-file X", and the checks follow that chain down to the root (`crossing_under`).

**Inherited declarations.** A helper the lowering or a pass makes shares its source's declaration, so declaring the source covers it:

- a deferred join's factors and keys (`fz…`, `fzk…`), a pushed join (`fl…`), an export (`xd_…`);
- a pooled negation's set (`rr_not…`);
- a guest's copy of a relation (`rr_in_<label>_<rel>`) and a context's copy (`rr_pass_<context>_<rel>`, 0597), copies of copies too;
- a `@place` finding's relation (`rr_place_…`) is meant when it only places findings.

`lowering_source_in` finds the source: by name for the hashed and copied helpers, else the first non-made relation among the readers, reader by reader, the least by name in its round (so the answer does not depend on read order).

**What is not a crossing** (`FileLocal::crossings` skips these, 0652):

- a rule that reads a cross-file relation: it follows that relation;
- a rule that reads no document's rows: only the run's inputs and facts (`env`, `p0_file`, `loaded`, settings, a phase's export);
- a rule with one plain, positive document read: each row is one file's, a union (the cross part reading the chunks' findings back, `violation(c…) :- violation_at(_, c…)`);
- a rule that places one document once each input naming no document is taken as placeless (`kind(n, …), value(n, v), value_type(v, …)`).

Documents meet only where a rule joins two reads of them, or aggregates one. The error points there. For a relation of values gathered from every file (`zero_number(t)`), it points at the reader that joins it with its own document's rows, not at the union.

**Functional relations** are read as placeless inputs by a file-local reader, and their rules are checked only for reads of cross-file relations: `match_all`, and a relation that chooses an evaluation, not rows (qualifier `strategy`: a deferred join's hot set, a pair cache's guard, a factorized cache).

**Placeless, not cross-file** (0665): the settings (`package`, `package_field`, `package_export`, `package_for`, `tsconfig`, `tsconfig_for`, `tsconfig_path`, `env`), the run's options once stated as facts (`option`, `option_item`), and a phase's exports read as settings (`tsx_names__*`, later `py_names__*`, `points_to__*`). A per-file rule joining them stays per-file. `file` is not a setting.

**Held cross-file by purpose, without a declaration:** the cross part's read-back `r(…) :- r_at(_, …)` (qualifier `READ_LOCATED`, 0665).

**For I3 the chain is read the same way.** A node crossing is named where the node enters (the root of the chain), not at its followers: `air302_alt_1(n) :- sym_of(n, "…"), kind(n, …)` follows `sym_of` and goes per-file with it. The pack test groups each offender under the crossing it is under, which is the fix to make.

## I3: no node id crosses (0677, oscar)

A node id is `document << 32 | slot` (0094 §2a), so a fact carrying one file's node into another file's rows changes whenever the first file's ids change, with nothing changed in meaning. 0094 §8's move gate: moving a definition to its own module gave 4,062 delta rows against the edit's 720, through `calls_cx`'s `qv`/`bv`/`pv`. Over the full corpus program, 35 of the cross part's 141 relations had a node column, holding 11.5 M of its 19.8 M rows (0640 corrected, ac0e5f8dea).

**The rule** (0677 §1–3):

1. No relation of the cross-file part holds a node id: declared `cross_file`, engine-known, inherited, a lowering helper placed there, or a phase's export. No column typed `node` (or `loc`), and no value computed from a node: its id as text or number, arithmetic on it, a `cat` of it.
2. Cross-file facts are keyed by **symbols** (a module, `module.qualname#k`), **paths**, **document keys**, and **values**. `docof(n, d)`'s `d` may cross; its `n` may not.
3. Nodes stay per-file. A file's rules relate its nodes to symbols; the cross-file work relates symbols to symbols; a file reads the answers back by symbol and places them on its own nodes.
4. The shape is 0665 §6's: cross-file resolution is a phase that runs first, to its fixed point, over symbols alone, read by the main program as a setting keyed per file.

**Which columns hold a node** (`NodeColumns::of`). Seeded from declared `node` and `loc` columns, then a fixpoint over all rules adds a head column filled from a node: a variable bound at such a column, by `nearest`, or by an `ast` pattern; a function of one; a `min`, `max` or `sum` of one (not `count`). Equalities carry it within a rule, either way round. **Not a node:** a function whose answer is what the node says: `@doc` (a document key), the text and value reads (`NODE_READS`), `@tree_digest`, `@unparse`, `@py_unparse`, `@ast_dump`, `@place`.

**Where a node enters** (`node_crossings`). Named:

- a relation derived in the cross part whose rule binds a node per file: by a per-file relation's node column (an export's, a base relation's), a pattern, or `nearest`, and by no node column of another cross-file relation;
- a relation made natively in the cross part with a node column, read there;
- any rule reading a node column of a setting or an earlier phase's export (`run_constant`), which every file reads.

Not named: a **follower** (reads its node from another cross-file relation; it goes per-file with it); a **file-local relation the cross part derives again** for its readers (`kind`); an **output** (its node is the finding's place, written once). A relation only outputs read *is* named, unlike 0652's `placement_only`: a set of other files' nodes (`callee(d)`) is maintained over every file and changes with their ids.

**Report first, then refuse.** Preparation reports offenders once per program version; `RERAIL_STRICT_NODES=1` refuses, naming each relation, its column, its rule, a helper's source, and the crossing it is under. `RERAIL_NODE_CROSSINGS=FILE` appends `<relation> <under>` lines for the pack test. Once the list is empty, the report becomes the error and the switch goes.

**The list that only shrinks.** `native/crates/boot/src/code_rule_dsl/node_crossings.txt` holds `<pack> <relation>` for every pack and the data layer (`data-layer:alone` for its policies split one by one). The pack tests fail on any relation not listed, and a whole-pack run also fails on a listed relation no longer found, so a fix removes its lines in the same commit. At oscar's tip it holds 288 lines: python 72, data-layer:alone 59, data-layer 51, django 33, node 15, vue 11, typescript 11, python-main 9, javascript 9, php 5, sql 3, rust 3, react 3, blade 3, next 1.

**Cost of the check:** 5 ms for a 2,523-rule program (a python-main shard). A first version spent 110 ms resolving helpers' sources; it now reads an index of each relation's readers, made once.

**Not covered:** a prepared program read back from the store keeps no causes and is not reported again (it was when prepared).

## Special cases: the split's gaps fixed in the engine

Most first-sweep crossings were gaps in the split, not wrong rules. The rule was: a gap is fixed in the split, never by annotating the rule. Shared means both branches have it; the rest are one branch's.

| Gap | Example | Fix in the split | Decision | Where |
| --- | --- | --- | --- | --- |
| A computed view defined by comparisons was not inlined | `ca_split(r, h, t) :- ca_dotted(_, r), match(…, r)` | `InlineFilters` inlines it where the reader reads its domain; a domain atom must share a variable with the head (0665) | 0649 §2 | shared |
| A column filled by a node or `unset` was not same-document | `ca_param(f, x, ann)` | Nullable location (`FileLocal::nullable`), joins only where a location column places the variable | 0649 §3 | shared |
| A negated read of a nullable column took it as its own location | `!r(n, a)` | Under negation it is its anchor's document | 0652 | shared |
| Natives were cross-file by kind | `cross_native` | Placed by their rules; `match_all` and `strategy` relations are functional | 0652 | shared |
| `sub_tape(n, _, t)` | a guest tape value | `t` is `n`'s class | 0652 | shared |
| `@doc(A) = @doc(B)` was not a join | JSON translator's same-document joins | `Lit::Same`, as a shared variable | 0652 | shared |
| A min/max over nodes read its node back by key | `OrderNodes`: `node_order(v, kv)` | The helper carries the node beside its key; a min/max with no group is declared | 0652 | shared |
| A negation's context grew through tests | `SECOND != FIRST` tied every comparison to every node (SIM109, PIE810) | A test ties nothing; kept where other literals reach its variables | 0652 | shared |
| A view read only through a join with one file's row | `modattr(m, o)` read by `bare` | **Covered views** (`covers`, `maps`): file-local though a rule joins documents; `cover_deps` re-asks when a placing reader changes | 0652 | shared |
| A demand that drops a read lost its tie | UP008 `super_first(a, f)` paired every name with every class path | The request keeps `@doc(a) = @doc(f)`; `row_documents` lists relations whose rows are each one document's | 0652 | shared |
| A head variable no literal binds | rows of every value; read as a join of everything | Refused: `head_bound` | 0652 | shared |
| Qualifiers lost when a program was printed and read | `cross_file`, `inline`, `magic` | `print_version` writes `QUALIFIERS`; linked library helpers excluded by name | 0652 | shared |
| A join by path was "two documents" | `bcgp_lead` joining `bcgp_line` rows by `p` | `doc`'s path is a **path key**; `placing` = locations + path keys; `locate` relays by path (`Doc::ByPath`) | 0665 §1–2 | shared |
| Settings had no location | `package_for`, `tsconfig_for`, `env` | Placeless; memo keyed by the settings a file can read (`settings_scopes`) | 0665 §3 | shared |
| A constant's join was demand-cut by files' requests | `deps_on` | A constant read by a non-constant rule is left as written | 0665 §4 | shared |
| Options were run-time inputs | `option_item("pydocstyle.ignore-decorators", …)` | Stated as facts before the run; `DecidedFactTests` drops what cannot hold | 0665 §5 | shared |
| A symbol built from a file's path placed nothing | `er_doc(d, m)`, `cat(m, ".", q)` | A variable built by `rx_group`, `replace_all` or `concat` led by a placed one places what it places | 0665 | shared |
| The scope `nearest` finds was no location | `rdefn(s, x, f) :- …, nearest(f, s, scope)` | `nearest`'s variables named and placed (`placed_classes_over`) | 0676 (oscar), 0675 §1a (bend) | both, separately |
| A value view from a set every file adds to | `canon_number(t, r) :- num_source(t), …` | **Keyed family**: a root and members covered where readers hold the root's row at the key | 0676 | oscar |
| A view's branches each required | `negate(a, b) :- (number(_, a, _) ; number(_, _, a))` | Branches cover each other where they test alike (`same_rest`) | 0676 | oscar |
| A rule that can never hold | `kind(-1, …)`, `-1 != -1` after `unset` substitution | Dropped (`dead`); a read at `UNSET` of a location reads no document; `one_document` and `why_not_local` agree | 0677 fix 2, 0675 | both |
| A gated negation's ask read an unbound variable | `$rr_kd is unbound in SQL` | The ask reads later positive atoms and comparisons too | 0677 fix 2 | oscar |
| The factorizer moved nodes into the cross part | `base_name(q, q0) :- base_expr(q, e), qn(e, q0)` | No factoring on a node key unless the head holds a node | 0677 fix 3 | oscar |
| A row located by a built name found no document | `plp_find_k` keyed by `cat(p, "::", q)` | `FileLocal::paths`: relay by the path as it is; a relation placed by a built name alone moves | 0677 fix 3f | oscar |
| A later `locate` round blocked an earlier relay | 0652 refusal | The moved reader reads the existing relay | 0677 fix 3f | oscar |
| An id-range bound pushed nodes out | `$f < 6917529027641081856` on every pushed join | The bound stays with the join; took 43 relations off the list | 0677 fix 3f | oscar |
| A view of values through other views, natives, siblings | `item_parts`, `zero_number`, the format family | Deeper `covers` (`through`, `Held`, `native_inputs`, `same_tests`), depth bound 64 | 0672 | bend |
| A guard read across files | `rr_pair_cache_…_small` | Each part derives its own guards | 0672 §2 | bend |
| A nullable location tested set | `bad(d) :- param(f, d), !unset(d), …` | Places its head (`tested_set`) | 0672 §1 | bend |
| An existence read | `class_def(_, c)` | Read from `rr_any_<rel>_<cols>`, a relation of names | 0675 §1a | bend |
| A flag over every file gating per-file work | `plp_docin(d)` | Two copies of the cone, chosen in the cross part (`flags_last`) | 0686 | bend |
| An input file missing or empty | swallowed-exceptions' ignores file | Stated empty: a constant | 0686 §2 | bend |

Tests for each are named in the decision records; most are in `typing::file_local::tests` (`native/crates/device_metal/src/code_rule_dsl/typing/file_local/tests.rs` on both branches).

## Playbook: fixing a flagged rule

This is the procedure both branches followed, read off 0652, 0665 and 0677's fixes. Each crossing is read one by one; nothing is declared to make a test green.

1. **Get every offender at once.** Run the pack's split test (or the policy alone, `RERAIL_BATCH_PACK=off`; a crossing can hide behind another policy's). The 0652 error and `RERAIL_STRICT_NODES=1` name each relation, its rule, a helper's source, and the root crossing it is under. `RERAIL_SHOW_CROSSINGS=1 … --no-capture` prints `<pack> <relation> under <crossing>` for every one.
2. **Fix the root, not the followers.** Group offenders by `under`. Fixing a root takes its followers per-file with it: fix 2 (`resolve_export`) cleared 2,536 entries.
3. **Classify the root.** Read its rule and ask, in this order:
   1. **Is it a gap in the split?** The rule text is right and about one file, but the split cannot prove it (it reads through a view, a nullable column, a native, a path, a setting, a helper). Fix it in the split with a test that must stay cross-file beside it. Re-run: the root moves to the next cause, often one step along.
   2. **Does it join documents by accident?** A per-file rule joins another file's rows by a name, a value or an index. The usual mistake: a table keyed by a value drops the node it came from (`comment(_, w)`, `fn_words(_, w)`, `item_parts(i, x, v) :- gitem(_, i, _)`), so it holds every file's values and its reader lands in the cross part. **Fix:** key the table by the node, or test the value in the reader's own rule (`capital(x)` → `capital_id(n)`; `cap_name` → `!match("_*[a-z].*", x)` where it is read).
   3. **Is it meant to span files, and does it hold no node?** Declare it `cross_file` with a comment saying why. Examples: `csa_owner_key` (CONN301 counts a name's owners over every file), `stale_entry`, `rf_imported`.
   4. **Is it meant to span files, and does it carry a node?** Rewrite it to symbols (next list). Under I3, a declaration is not enough.
4. **Rewrite a real crossing to symbols** (oscar's shape, 0665 §6 / 0677 §3–4):
   1. Find where the node enters: the one or two rules that bind a per-file node into a cross-file head (for Python, `sym_of`'s two rules).
   2. Per file, map nodes to keys: a definition to its qualified name, a module to its name, a def to `cat(path, "::", qualname)`, an anonymous node to its scope-relative place (`m.C.run@14+5^0`, points-to's terms), a site to `"path:line"` text.
   3. Move the cross-file closure into a phase over those keys alone (`.phase py_names`, `points_to`, `tsx_names`), run first to its fixed point. It reads definitions and imports, never a name's use, so a body edit changes none of its rows.
   4. Hand on by **module**, anchored at the reading file's **path**: `reads_at(p, m)` (the modules file `p` can name), then `exports(m, x, q)`, `class(m, c)`, `ancestor(m, q, b)`, … Register them in `SETTINGS_BY_PATH` / `SETTINGS_BY_FOLDER` so each file's memo entry is keyed by the rows it can read. Keying by a symbol computed from the file's content would miss every file on any change.
   5. Read back per file at the file's own nodes: `export_at(n, q0, q) :- qn(n, q0), split_dot(q0, m, x), <n's file at p>, py_names__reads_at(p, m), py_names__exports(m, x, q)`.
   6. Where an answer pairs two files (a model's fields, Meta, hooks), join it **in the phase** under the reader's module; the reader's file need not import the module the ancestor is defined in.
5. **Prove it the same.** Write the tests first (one per requirement):
   - the new route's answers equal the old cross-file rules', run beside them in one program;
   - every pack's findings unchanged; for an accidental join, a two-file case where the old rule flags the wrong file;
   - the edit tests: a body edit misses its own file only; an interface change misses the module's file and its readers only;
   - for a large rewrite, old engine + old rules against new engine + new rules on a real corpus, findings equal, instructions compared.
6. **Shrink the list in the same commit.** Remove the fixed lines from `node_crossings.txt`; the whole-pack test fails on a stale entry. Sharded packs (python-main, python, javascript) and `:alone` runs cannot see a fix, so union their `RERAIL_SHOW_CROSSINGS` lines and remove stale ones by hand.
7. **Expect latent faults.** Moving a relation per-file exposes rules the split never checked while they were cross-file. Each found was fixed at its root (see Engine faults).

**Pitfalls seen:**

- A phase's export read by a key no setting scope covers (`doc(d, _, m)` instead of `reads_at(p, m)`) made every file miss on every change (0 of 5 kept).
- A rewrite can be slower: sfu's first version paired every right-hand-side node with every later read (22 M rows, +40% instructions); start from the anchoring calls.
- `0` written for "no node" is a value, not `unset`; use `unset` (TypeScript `array-type`, Vue `require-valid-default-prop`).
- A `min` over symbols is refused because it would order their ids; pick the least by symbol text.
- Message text that names a node id (`'\204539970558885890'`) reads its span's text instead.

## Rule fixes catalogue

Three kinds of rule change were made: accidental joins rewritten per document, meant crossings declared, and node-carrying crossings rewritten to symbols. Rule text whose only problem was a split gap was kept as written (`purity-calls`, `ruff-try301`, `ruff-up008`, JS/TS `no-useless-constructor`, TS `no-misused-new`, `parameter-properties`, Vue `no-duplicate-attributes`, `valid-v-for`, and the libraries `python/references`, `python/modules`, `bootstrap`).

### Accidental joins, rewritten per document

Each was run against its original over one file and over two: the same findings on one, and on two the original flags a file it does not concern.

| Policy | The original | On two files | Where |
| --- | --- | --- | --- |
| Python F402 | a `__future__` import before the loop by node order, in any file | flags `b.py`'s loop for `a.py`'s import | 0652 |
| React `jsx-no-constructed-context-values` | a `createContext` at any file's program root | flags `d.jsx`'s `Ctx`, a plain `1` | 0652 |
| TS `explicit-module-boundary-types` | `followed(x, d)` by the name alone | flags `b.ts`'s private `g` for `a.ts`'s export | 0652 |
| React `no-deprecated` | `replaced` by any file's pragma | same on these files | 0652 |
| `phpcs-…-size-functions-in-loops` | every loop with every `$this->x` | refused before the split | 0652 |
| `post-op-side-effects` | allowlist read in any file | same with one allowlist | 0652 |
| React `cap_name` | names of every file's identifiers | a file's components depended on other files' names | 0665 |
| ESLint `no-useless-computed-key` | `touching(t, k)`, an old signature | joined any node whose preceding code equalled the key's id | 0675 (bend) |
| `value_type` (python-main, 249 relations) | `value(n, x), value_type(x, T)` keyed by the text, `"12"` = `12` | a false finding at `"12"` because some int `12` existed anywhere | 0677 fix 1 |
| TSX `capital(x)`, `unescape`, `upper_first_ask`, capitalized-comments | tables keyed by text | a per-file ESLint rule depended on other files | 0677 tail 1 |
| TSX `item_parts` (`/* global */` items) | split through a table of every file's items | `global_item` and all above it cross-file | 0677 fix 3g |
| TS `array-type`, Vue `require-valid-default-prop` | `0` for "no node" | same findings; `unset` now | 0652 |

### Meant crossings, declared `cross_file`

Shared by both branches (no node in their rows): the data layer's `cpl_msg`, `cpl_missing`, `er_facade_site`, `stale_entry`, `la_eager_delegate`, `annotations`, `static_tests`, `pos_bypass`, `pos_allow_broken`, `ma_branch_aware`, `csa_owner_key`; `rf_own_fn` and `rf_imported` (`lib/python/references.rerail`); Django's `bnl_number`; the generated FAST003's `fast003_alt_2`, `fast003_pyd_field`; the typing programs' `wantx`, `modchild`, `res`, `fn_ref`, `type_of` (in their `.rdl` sources); and engine-test programs' subjects (`uses`, `dep`, `pair`, `shared`, …). Path-keyed bootstrap relations `p0_from`, `p0_file_t_shape`, `p0_phase_named`, and Python's `py_want`, `py_workspace`.

Bend only (19: 17 Django relations, python/names' \`mro\_method\`, and the late test program's \`called\`; each pairs a reference with a definition in another file, so the Django ones carry nodes): `dj_task_waits_for_commit`, `dar_non_atomic_wrapped`, `dj_model_super`, `goc_meta_from`, `goc_meta_opt`, `bwh_method`, `bwh_in_hook`, `dj_qs_model`, `dj_instance_value`, `rmw_class`, `ghw_model_ref`, `goc_call`, `sfu_atomic_def`, `tam_task_call`, `tam_instance_value`, `tam_instance`, `smi_doc`, `mro_method`, `called`. Oscar rewrote these to symbols instead, and most are gone. Under I3 they would be offenders.

Oscar only: test programs whose subject is a crossing (`edge`, `in_doc`, `in_node_doc`, `keyed`, `old_anc`, `old_lib`, `old_sym`, `resolved`, `target`), all in engine tests: the split's tests, the corpus tests' old-versus-phase comparisons, and the points-to oracle tests' call edges by path and line.

The JSON translator declares what it generates: a helper reading a guest capture by text and binding it to nothing else (`fast003_alt_2`).

### Node-carrying crossings, rewritten to symbols (oscar)

| Fix | Root crossing | What changed | Entries left | Tests |
| --- | --- | --- | --- | --- |
| 1 | `value_type` | Translator writes `value_type_of(n, T)`, the node's own type; a value followed through a binding keeps `value_type`. 41 python-main files regenerated | 3,190 → 3,023 packs; 3,140 with the alone runs | `json_rules_value_type_is_the_nodes_own` |
| 2 | `resolve_export` (2,536 relations) | Phase `py_names`: `reads_at`, `exports`, `class`, `defined`, `public`. `names.rerail` = `scopes.rerail` (per-file) + reads of the phase. `export_query` gathers from-imports only | 3,140 → 592 | `python_names_through_the_phase_are_the_cross_file_rules_answers`, `a_python_files_entry_is_kept_under_its_own_names_resolution` |
| 3 (3a, 3b) | `subclass`, `dj_model_q`, `dj_model_super` | `ancestor`, `library_ancestor`; ancestors' members `anc_call`, `anc_kw`, `anc_ref`, `anc_method`; a model read by symbol (`dj_model_ref`, `dj_model_named`, `dj_model_field`, …) | 592 → 549 | `python_class_ancestry_through_the_phase_is_the_cross_file_closures`, `a_django_model_is_read_by_its_symbol_in_another_file` |
| 3c | `goc_call` | Phase's Django section: `dj_group`, `dj_cond_group`, `dj_opaque`, `dj_unmanaged`, `dj_abstract`, `dj_manager_method` | 549 → 541 | `a_django_models_meta_and_managers_are_read_by_their_symbol_in_another_file` |
| 3d | `bwh_method`, `bwh_hook` | `dj_hook(m, q, k, h)`, `dj_receiver_fn`; helpers moved to `python/syntax`, `python/django-shared` | 541 → 538 | `a_django_models_hooks_are_read_by_their_symbol_in_another_file` |
| 3e | `pt_*` (points-to) | Phase `points_to` over terms; `points-to-file` (per file) + `points-to-solve` (phase); `call_target(c, f)` gives the callee's symbol; four Django call-graph policies over symbols | 538 → 440 | the 30 oracle tests, every rule file's tests |
| 3f | `plp_anycand`, `pc_*` | A def is `cat(p, "::", q)`; requests per file, answers over keys; three engine fixes | 440 → 328 | `a_row_is_located_by_its_path_not_a_name_built_from_it`, `an_id_range_bound_goes_with_the_pushed_join` |
| 3g | `item_parts` | `global_item` splits an item in the comment's own rule | 328 → 312 | `global_comment_items_are_read_in_their_file` |
| tail 1 | `capital`, `unescape`, … | Keyed by the identifier node; `match_all` over the node | 312 → 288 | ESLint pack data, 3,671 files compared |

Measured where it mattered: Django's four call-graph policies on 143 posthog files, 8% fewer instructions; the purity policy, 14% fewer (0.574 → 0.494 × 10¹²) and 2.83 → 2.69 GB; ESLint pack 1.091 → 1.032 × 10¹²; findings equal in each.

## How the node list shrank

*(A chart was here: Decision 0677's list counts per fix, packs plus the data layer's policies alone; oscar, 2026-11-24 to 11-25. The per-fix counts are in the table above and in decision 0677.)*

One root crossing, Python's `resolve_export`, held 84% of the list; moving name resolution into a phase cleared it. Each later fix is one library family. The list held 3,190 pack relations when the check was built; the series starts after fix 1, the first count that includes the data layer's policies run alone.

## Bend's engine route (0672, 0675, 0686)

After 0665, bend worked the position report (I1) down with engine changes and no rule rewrites, in the user's plan steps A–D: 56 flagged rules → 41 (0672) → 32 (0675) → 31 (0686). It never adds the node-id check.

**0672, decided: a view of values its reader holds is per-file.** `covers` goes past one step: through other covered views, through natives at their inputs' values (`match_all`, counted ranks), siblings by kind (`same_tests`), head values a call makes, tests the reader holds, id-range bounds read past. A nullable location tested set places its head. A pair cache's guard read across files is each part's own. A covered view a cross-file rule also reads is derived by both parts (`FileLocal::views`). 15 rules left the report (UP032's format family, `item_parts`, `zero_number`, PYI011/014, …).

**0675, proposed: name relations are staged automatically.** A derived relation is staged (`FileLocal::staging`) when its rows name no document, hold names (no node), are the program's own, are not answered by a load, read none of their readers, and a file-local rule reads them. The staged cone becomes a phase `staged_<k>`, run first; the program reads `staged_<k>__s`. A file's memo entry is keyed by **probes** (`rr_probe_…`) at the values its rules fix, which replaces `SETTINGS_BY_PATH`/`SETTINGS_BY_FOLDER` for `tsx_names`. `RERAIL_STAGE=off` disables it. Also: existence reads become `rr_any_<rel>_<cols>`; `nearest` places both nodes. Staging uncovered 22 refused relations, fixed by split gaps (reads at `unset`, constant comparisons) or by declaring Django's node-carrying joins `cross_file`.

**0686, decided: a flag over every file is applied last.** A flag (`plp_anycand()`, `plp_docin(d)`) gating per-file work: its cone is derived twice per file, `rr_flag1_r` and `rr_flag0_r`, and the cross part picks by the flag's value. `RERAIL_FLAGS_LAST=off`. A missing or empty input file is stated empty, so it is a constant.

**How it differs from oscar.**

| Question | Bend | Oscar |
| --- | --- | --- |
| Who moves name resolution out of the program | The engine, automatically (staging, 0675, proposed) | The library author, by a `.phase` (`py_names`, `points_to`), as 0665 §6 |
| A file's memo key for a phase's rows | Probes at the values its rules fix | `reads_at(p, m)` anchored by path, then by module (`SETTINGS_BY_PATH`/`FOLDER`) |
| Node ids in the cross part | Allowed; "re-keying is a decision of its own" | Refused (0677), list shrinks to zero |
| Django's cross-file joins | Declared `cross_file` (nodes cross) | Rewritten to symbols in `py_names`' Django section |
| Value views and family coverage | 0672's deep `covers` | 0676's `nearest` placing and keyed families |
| Purity's flag `plp_docin` | Applied last (0686) | Rewritten by key (fix 3f); `plp_` off the list |

The two engine extensions overlap: 0672 and 0676 both widen `covers`, and 0675 §1a and 0676 both place `nearest`. They were written separately on the same base and have not been reconciled.

## The checkers

Three checks run on the split the run will use, in preparation, after the lowering, so every helper the lowering placed is checked too. Paths are oscar's; bend's are the same for I1 and I2.

| Check | Code | Called from | Switches | Output |
| --- | --- | --- | --- | --- |
| I1 positions | `file_local.rs`: `POSITIONS`, `position_reads`, `positions_checked` | `programs_of` / `assemble` (each split, once per version) | `RERAIL_STRICT_POSITIONS=1` refuses | stderr line: `the cross-file part reads positions … (decision 0649): <head> reads <pos>, …` |
| I2 crossings | `file_local.rs`: `crossings_checked`, `placement_only`, `crossing_shown`, `lowering_source(_in)`; qualifiers `CROSS_FILE`, `MOVED_CROSS_FILE`, `READ_LOCATED`, `STRATEGY`; the split's `FileLocal::crossings` in `file_local/on_version.rs` | `logic/plan/corpus.rs` (the run) and `corpus/prepared.rs` (a prepared program) | none: always an error | `Error::Plan` naming every relation, its cause and the fix |
| I3 node ids | `file_local/node_crossings.rs` (379 lines): `NodeColumns`, `node_crossings`, `crossings_under`, `nodes_checked` | `programs_of` | `RERAIL_STRICT_NODES=1` refuses; `RERAIL_NODE_CROSSINGS=FILE` appends lines; both classified as preparing switches | report, or the refusal with each relation's rule and crossing |
| I4 head bound | `head_bound` beside `bound` in preparation | preparation | none | refusal |

Oscar's crate paths: `native/crates/typing/src/code_rule_dsl/typing/file_local.rs`, `…/file_local/on_version.rs`, `…/file_local/node_crossings.rs`. Grammar and printing of the qualifier: `rr_syntax/…/front/ast.rs`, `rr/…/rr/print.rs` (`QUALIFIERS`), `rr/…/rr/lower.rs`, `rr/…/rr/translate.rs`.

**The pack-wide test** (`native/crates/boot/src/code_rule_dsl/tools.rs`, `mod pack_split_tests`, default tier):

- Copies each pack's `.rerail` files to a temp tree, drops `.scope include/exclude/dirs` lines so every policy runs over a nine-file corpus (`a.py`, `a.js`, `a.ts`, `a.tsx`, `a.php`, `a.blade.php`, `a.rs`, `a.sql`, `a.vue`), and calls `run_policies` with `RERAIL_BATCH_PACK=strict`.
- Fails on any run error (I2 refusals and anything else preparation refuses).
- Reads the `RERAIL_NODE_CROSSINGS` file and fails on a relation not in `node_crossings.txt`, printing `<pack> <relation> under <crossing>`; a whole-pack, non-sharded run also fails on a stale listed relation.
- One test per pack: data-layer, blade, complexity, django, next, node, php, react, rerail, rr, rust, sql, typescript, vue; `…_policy_by_policy` for the data layer alone (`RERAIL_BATCH_PACK=off`, slow tier, 23–38 s); python-main in 12 shards, python in 2, javascript in 3, each under 20 s.

**Other tests tied to the checks:** `typing::file_local::tests` (the split's own: 36 at 0665, 45–50 on bend, more on oscar), `json_rules_pack_is_current` (generated python-main is the translator's output byte for byte), `every_switch_lowering_reads_is_classified`, `a_program_linked_from_its_modules_is_its_whole_lowering`.

**Cost.** python-main's 537 policies take 84 s of CPU to prepare at once, hence the shards. The node check is 5 ms on a 2,523-rule program.

## Engine faults found on the way

Making relations per-file exposed faults that had been hidden while everything above them was cross-file. The column "For a port" says whether a port of the invariants meets the same fault, judged by whether main has the code involved.

| Fault | Found by | Fix | For a port |
| --- | --- | --- | --- |
| `print_version` dropped qualifiers, so a printed program lost `cross_file`, `inline`, `magic` | 0652 | Print `QUALIFIERS`; linked library helpers excluded by name | Needed: main has no `print_version`; check how main prints and links programs |
| A head variable no literal binds made rows of every value | 0652 | `head_bound` | Needed |
| A text test over `op`'s strings passed no operators | 0652 follow-up | The batch's vocabulary gives `op`'s strings; kept counts keyed by definition too | Check |
| A line's text twinned as a node's | 0665 | `span_text_demand` | Check |
| An output or request decided empty lost its table | 0665 | Keep rules as written / an empty fact list | Needed with 0665 §5 |
| `def_unit`'s lookup by unit missed document roots (-1) | 0665 | Each document's root | Check |
| Two runs of one store in one process: one context's sweep removed another's staged file | 0677 fix 2 | `staging.lock` shared until commit; sweep under it exclusive | Only if phases run beside programs in one store |
| A warm run after a cold one with tape values wrote a new manifest | 0677 fix 2, 0675 | A memo entry keeps each embedded string's text (sha256) | Only with phases and guests |
| A pass's `.from` reads and a program's demand printed in link order | 0677 fix 2, 3c | Printed sorted | Check |
| A collection's sweep after its commit removed a later commit's file | 0677 fix 3 | `sweep_committed` under the write lock | Store; check main |
| A space's second write in one transaction swept files the committed chain still named | 0677 fix 3 | `first_write_of_space` | Store; check main |
| `py_names`' `base` missed `Named.Meta` (a class read as a class's attribute) | 0677 fix 3c | Resolve the object, take the attribute's qualname | With `py_names` |
| `match_all` over a node only a derived relation binds scans nothing, silently | 0677 fix 3g, tail | Open; one rule per kind meanwhile | Open |
| A stage's and its program's documents' tables collided | 0675 (bend) | Keyed by the plan too | Only with staging |
| `covers` recursion overflowed a test thread's 2 MB stack | 0675 (bend) | Depth bound 64 | Only with 0672's `covers` |

## Survey of main

Main (d54013c841) has the split both branches started from, and none of the invariants. Its split code has not changed since the branches' last merge (92fbff1ccf), but the branches carry about 8,400 commits of other engine work main lacks, so the checkers cannot be cherry-picked.

**Main's split.** `native/src/code_rule_dsl/typing/file_local.rs` (2,169 lines) and `file_local/on_version.rs` (914 lines; the branches' were already 1,072 just before 0649). `FileLocal` has `local`, `constants`, `locations`, `filters`, `causes`, `same_doc`, `no_document`. Oscar's adds `nullable`, `crossings`, `covered`, `row_documents`, `path_keys`, `paths`. Main has `programs_of`, `cross_causes`, `locate`, `why_not_local`, `one_document`, `push_local_joins`, `factorize_exports`, the program image (`Version`, `Fields`, `manager::analysis`), `Lit::Nearest`, `Lit::Ast`, `NODE_READS` and `QUALIFIERS`.

**What main lacks, by invariant.**

| Needed for | Missing on main |
| --- | --- |
| I1 | `POSITIONS`, `position_reads`, `positions_checked`, `RERAIL_STRICT_POSITIONS`; 0649's inlining of comparison views and nullable locations |
| I2 | the `cross_file` qualifier (no `.decl … cross_file` anywhere), `CROSS_FILE` / `MOVED_CROSS_FILE` / `READ_LOCATED`, `FileLocal::crossings`, `crossings_checked`, `lowering_source`, `covers`, `row_documents`, `head_bound`, qualifier printing |
| I2, so the error fires only on real crossings | 0665's path keys, placeless settings (`setting_input`, `SETTINGS_BY_PATH`), `settings_scopes` memo keys, options stated before the run; `locate_all`, `Doc::ByPath` |
| I3 | `node_crossings.rs`, `node_crossings.txt`, `RERAIL_STRICT_NODES` |
| I3, so a crossing can be fixed | the phase pattern: no `tsx_names`, `py_names` or `points_to`; main's typing programs are `calls_cx`, `calls_p3`, `struct`, `types`, `values`, `sources`, `workload*` |
| I6 | `pack_split_tests` (main has `run_policies` and `RERAIL_BATCH_PACK`) |

Several lowering helpers the checks name are not on main at all (`rr_deferred_pairs`, `rr_pass_`, `rr_pair_cache`, `OrderNodes`): the port names only the helpers main's lowering makes.

**Where main's rules live.** Main moved the Django pack out of `docs/rule-syntax/packs/django` into `packs/django/rules` (27 policies, with `cases/`), and `lib/python/django.rerail` with it (ae5e077366, #930, 2026-11-23). Oscar's Django fixes target the old paths. Other packs are where the branches have them:

| Pack | Policies on main |
| --- | --- |
| python-main | 537 |
| python | 537 |
| javascript | 121 |
| typescript | 65 |
| data-layer | 49 |
| rerail | 44 |
| react | 40 |
| node | 37 |
| rust | 33 |
| django (`packs/django/rules`) | 27 |
| vue | 23 |
| next, sql | 21 each |
| php | 15 |
| blade | 10 |
| complexity | 7 |
| rr | 1 |

Of the 142 rule files oscar's invariant commits touched, 29 do not exist on main (new libraries such as `lib/python/scopes.rerail`, `syntax.rerail`, `points-to-*.rerail`, `django-shared.rerail`, `lib/tsx/exports.rerail`, and the phases), 15 moved with the Django pack, and 1 changed on main since the merge (`lib/bootstrap.rerail`). The rest are unchanged on main since 92fbff1ccf, so their pre-fix text on main is the text the branches started from.

**What main would report.** Not measured: main has no checker to run. The branch numbers are an upper bound on the rules side only: oscar's first node list was 3,190 relations, taken after the 0649/0652/0665/0676 split fixes. Main, without those, would place more relations cross-file, and the I2 gate would refuse the split's gaps along with the real crossings, as oscar's first red run did (59 in the packs, about 40 in the engine's tests).

## Open items on the branches

**Oscar (0677):**

- The tail: 288 entries, about 110 distinct crossings of 1–11 relations each, mostly value tables that dropped their node. Batch 2 is uncommitted in the oscar worktree (python libraries: bugbear, fastapi, pyflakes, ruff-classes, upgrade, and five python policies).
- A sharded pack's stale entries: no shard can say a listed relation no longer crosses. A test that unions the shards' logs and refuses a stale entry is wanted.
- `match_all` over a node that only a derived relation binds scans nothing, and nothing says so (`kind_bound` in `lower.rs`).
- Where a finding that pairs two files' nodes is written (`call_target(call, def)`): settled family by family.
- I3 becomes an error, and `RERAIL_STRICT_NODES` goes, once the list is empty.
- `maintain_tests::a_moved_definition_costs_about_itself` to be measured again and its bar set from what the rewrite derives.

**Bend (0672, 0675, 0686):**

- 0675 is proposed, not decided.
- 31 rules still in the position report: `pc_tgt` (purity-calls joining a call to another file's target, undeclared), PLP-007's `plp_xq`, `pha_latest`, VMI002, LAO003, data-layer-module demands, FAST003's `rr_or142`, a demand flag over every file, three data-layer `violation` rules reading a finding with no location; Django rules below `mro_method` reading positions across files.
- A relation whose rules are all dead is not yet taken as placeless.
- The cost of a flag's cone derived for every file is not measured.
- Two device-crate GPU tests fail on a Linux host (not the split).

**Both (0652 §5):** I1 becomes an error once the engine pushes cross-file rules' position comparisons into the per-file part. Not started on either branch.

**Across the branches:** bend's 19 Django-side declarations contradict I3; bend's 0672 and oscar's 0676 extend `covers` separately; 0675's probes and oscar's `reads_at` anchoring are two answers to the same memo-key question.

## Sources

Read first-hand from the two branches with `git show`, not from either worktree's checkout.

- Decision records, `docs/final-shape/decisions/`: 0649 and 0652 (both branches; 0652 differs only in citing 0660 vs 0670), 0665 (both), 0672, 0675, 0686 (bend), 0676, 0677 (oscar).
- Note t55, points-to by symbol (oscar).
- Code (oscar): `native/crates/typing/src/code_rule_dsl/typing/file_local.rs`, `…/file_local/node_crossings.rs`, `native/crates/boot/src/code_rule_dsl/tools.rs` (`pack_split_tests`), `native/crates/boot/src/code_rule_dsl/node_crossings.txt`.
- Main: `native/src/code_rule_dsl/typing/file_local.rs`, `…/file_local/on_version.rs`; commit ae5e077366 (Django pack moved).
- Commits as listed under Lineage; the `.decl … cross_file` lists were taken with `git grep` over each branch's `docs/rule-syntax` and `native`.
