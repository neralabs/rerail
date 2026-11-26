# 0677. No node id crosses into the cross-file part: cross-file facts are keyed by symbols, nodes stay per file

Status: decided (2026-11-24), by the user: "we are working on rules that don't allow node ids to progress into the
cross-file part; facts need to be keyed on something else inside the per-file part", and, of 0665 §6 (the work in
`Bend-language-investigation`, 94e9aafd78), "yes that is it … for now please record it as decided". The user will
update 0665 with it. The check is built (§ Built, 2026-11-24): every pack's offenders are listed, and the list only
shrinks. The fixes are not done yet. It amends 0094 §2 (which kept node ids out of exports and kept
findings only), 0051's split, and 0652 (which places a relation cross-file but lets it carry nodes).

## Intro

A program runs in two parts (0051): the per-file part, derived again for an edited file alone, and the cross-file
part, maintained over all files. A node id is `document << 32 | slot` (0094 §2a). Its document half is the file, so
a fact that carries one file's node into another file's rows changes whenever the first file's ids change.

0094 §8's move gate shows the cost. `calls_cx` resolves names in the cross part. `qv(m, q, k, v: node)` binds a
qualified name to its definition's node, and `bv` and `pv` copy that node into every importer and every use site. A
definition moved to a module of its own gets new ids. Every caller's rows then change, and everything they derive:
4,062 delta rows against an edit's 720, with nothing changed in meaning.

0640 (as amended in `abstraction-design`, ac0e5f8dea) counted the same over the full corpus program: 35 of the
cross part's 141 relations have a node column, holding 11.5 M of its 19.8 M rows (`python/references.rerail`'s
`rf_*`, `dl_reference_edge`).

## Decision

1. **No relation of the cross-file part holds a node id.** That covers a declared `cross_file` relation, one the
   engine knows is cross-file, one that inherits it, a lowering's helper placed there, and a phase's export. It holds
   no column typed `node`, and no value computed from a node: its text as a number or string, arithmetic on it, a
   `cat` of it.
2. **What cross-file facts are keyed by instead:** symbols (a module, `module.qualname#k`, 0094 §2), paths, document
   keys, and values. A document key is the file's identity, not a node. `docof(n, d)`'s `d` may cross; its `n` may
   not.
3. **Nodes stay in the per-file part.** A file's rules relate its nodes to symbols (a reference to the name it
   resolves to, a definition to its own symbol). The cross-file work relates symbols to symbols. A file's rules read
   the cross-file answers back by symbol and place them on their own nodes.
4. **The shape is 0665 §6's.** Cross-file resolution is a phase that runs first, to its fixed point, over symbols
   alone. Examples are `tsx_names`'s `target_at(p, spec, t)`, `exports(m, x, q)` and `module(m)`. The main
   program's per-file rules read its exports as a setting, keyed per file by the rows that file can read.

## Detection

- **One check, in preparation, on the split as the run will use it** (`programs_of` on the run's path, where 0652's
  crossing check runs). It goes after the lowering, so a helper the lowering placed in the cross part (`rr_…`,
  `fl…`, a request, a guest's or a context's copy) is checked too. For each relation in the cross-file part and
  each phase export, it refuses:
  - a column whose declared type is `node`;
  - a column whose values flow from a node variable in any rule of the relation (`$x = @to_string($n)`,
    `$k = $n + 0`, a `cat` with a node). The type of a head column is inferred from what binds it, as `text_demand`
    infers a column's kind;
  - `docof`'s node side, or `node_order` and the other position relations (0649), read by a cross-file rule. Its
    document side may cross.
- **The error names everything at once:** the relation, the column, the rule that fills it, the source relation for
  a helper, and the first relation down its chain that brought the node across (0652's chain of causes). Each comes
  with the fix: key it by a symbol, or keep the rule per file.
- **Report first, then refuse.** Until the existing offenders are fixed, preparation reports them, as 0649's
  position report does, and `RERAIL_STRICT_NODES=1` refuses. A test prepares every pack, data-layer policy, library
  and typing program (0652 §4's pack test) and holds the list of offenders. The list may only shrink, as 0353's
  copies may: a new rule that carries a node across fails the test. Once the list is empty, the report becomes the
  error and the switch goes.

## Fixing the rules that carry nodes now

The report gives the list. The families known now, largest first:

1. **Python's name resolution** (`calls_cx`'s `qv`, `bv`, `pv`, `bknown`, `xknown`, `run_import`, `ext_import`,
   `open_sync`; `python/modules.rerail`; `python/references.rerail`'s `rf_*`). This becomes a phase `py_names` as
   `tsx_names` is. It hands on `module(m)`, `target_at(p, spec, m)`, and `exports(m, x, q)` following re-exports and
   star imports, all symbols. A file's rules resolve each name node to its qualified name per file.
   `rf_target(t, d)` becomes `rf_target(t, sym)`, and the definition's node is found by its symbol in its own file.
2. **Cross-file types** (`calls_cx`'s `tv`, `tsub`, `type_of`, a call's return across files). Each function's
   summary is per file and keyed by symbols: its return in terms of the symbols it reads (0065, 0083's dependency
   summaries). The fixpoint over symbols is the cross-file phase, and a file reads the solved types back by symbol.
   This is the largest rewrite.
3. **Class hierarchies and attribute lookups across files** (`anc`, `basex`, `cls_q` read across files): bases and
   members by qualified name, `bases(q, qb)`, `member(q, x, q2)`.
4. **The policies the report lists beyond these.** As in 0652, each is read one by one. Text that joins documents by
   accident is written per document. A real crossing is rewritten to symbols.

Each family is rewritten under tests written first, as 0665's were. The same findings as a fresh run, cold and after
an edit. The family's relations leave the report. An edit of a definition's body misses its own file. A change to a
module's exports misses that module's file and its importers alone.

**The move gate under this.** Once `calls_cx` resolves through a phase, a move changes the export rows of the two
modules it touches. It derives those two files again, and the callers whose resolved qualified name changed
(`pkg.helpers.parse` → `pkg._moved_0.parse`), per file. No cross-file row changes for a node id.
`maintain_tests::a_moved_definition_costs_about_itself` is measured again then, and its bar set from what the
rewrite derives.

## Built: the check (2026-11-24)

`typing::file_local::node_crossings` (`native/crates/typing/src/code_rule_dsl/typing/file_local/node_crossings.rs`)
runs on every split, after the lowering. `programs_of` reports what it finds once per program version, as 0649's
position report does. `RERAIL_STRICT_NODES=1` refuses the program instead, naming each relation with its rule.

- **Which columns hold a node.** Columns typed `node` or `loc` (`node | span`) are seeded from the declarations. A
  fixpoint over the program's rules then adds each column a rule fills from a node: a variable bound at such a
  column, by `nearest` or an `ast` pattern; a function of one (`d + 1`, `@cat`); a `min`, `max` or `sum` of one. A
  function whose answer is what the node says is not one: `@doc` (a document key, which may cross by §2), the text
  and value reads (`NODE_READS`), `@tree_digest`, `@unparse`, `@place`.
- **Where a node enters the cross part.** A relation derived in the cross part is named when one of its rules binds
  a node per file: by a per-file relation's node column (an export's, a base relation's), a pattern or `nearest`,
  and by no node column of another cross-file relation. A relation made natively in the cross part with a node
  column is named too. So is any rule that reads a node column of a setting or an earlier phase's export, which
  every file reads.
- **What is not named, and why:**
  - **A follower.** It reads its node from another cross-file relation: `air302_alt_1(n) :- sym_of(n, "…"),
    kind(n, …)` holds `sym_of`'s node and goes per file with it. 0652 names a crossing, not its readers, in the same
    way.
  - **A file-local relation the cross part derives again for its readers** (`kind`). Each of its rows is one
    file's. The node enters at the cross-file rule that reads it, and that rule is named.
  - **An output.** Its node is the finding's place, written once. This settles the earlier open point: a finding
    keeps its node, and every relation of the cross part that feeds it does not. A relation that only outputs read
    is named (unlike 0652's `placement_only`). A set of other files' nodes (`callee(d)`) is maintained over every
    file and changes with their ids for every finding that reads it. Keyed by a symbol instead, the finding is
    placed in its file's own rules (§3).
- **The crossing each is under.** It is read down the split's causes (0652's chain): through each "reads cross-file
  X" to the first relation that is cross-file for a reason of its own, or the input it joins with no location. That
  relation's fix takes its followers per file with it.
- **The list.** `native/crates/boot/src/code_rule_dsl/node_crossings.txt` holds `<pack> <relation>` for every pack
  and the data layer. `<pack>:alone` covers the data layer's policies split one by one. 0652's pack tests
  (`tools::pack_split_tests`) prepare every policy, and fail on any relation that is not in the list. A whole pack's
  run also fails on any listed relation that is no longer found, so the list only shrinks and stays current. A
  python-main shard sees a twelfth of its programs, so it checks only for new relations; the shards can't tell that a listed one was fixed, and a python-main relation leaves the list by hand. Its failure prints each new
  relation with its crossing (`<pack> <relation>  under <crossing>`).
- **Cost.** The program with 2,523 rules takes 5 ms (spark2, a python-main shard, measured during a test run).
  Resolving the lowering's helpers to their source relations was 110 ms of a first version. It now reads an index
  of each relation's readers, made once. A helper's source is the least by name among the source relations of its
  first round of readers, so it is the same whatever order the readers are read in.
- **Not covered.** A prepared program read back from the store (`prepared.rs`, `assemble` with its kept parts)
  keeps no causes, and is not reported again. It was reported when it was prepared.

### What the list holds now

3,023 relations over the packs and the data layer, by the crossing they are under (and 117 more for the data layer's
policies split alone). It held 3,190 when the check was built; fix 1 took out 182, and 15 others are new (below).

| Crossing | Where | Relations | Packs |
|---|---|---:|---|
| `resolve_export` | `lib/python/imports.rerail`: Python name resolution (family 1) | 2,536 | python-main 2,362, python 109, django 52, data-layer 13 |
| `pt_wres` | `lib/python/points-to.rerail` | 39 | django |
| `plp_anycand` | `data-layer/pure-lazy-runtime-purity.rerail` | 25 | data-layer |
| (its own) | a relation that is cross-file for itself | 25 | |
| `dj_model_q` | `lib/python/django.rerail` | 19 | django |
| `subclass` | `lib/python/names.rerail`: class hierarchy (family 3) | 18 | django 14, python 4 |
| `item_parts` | `lib/tsx/eslint-b-scope.rerail` | 18 | javascript 12, typescript 6 |
| the rest | 146 crossings of 1–14 relations each | 343 | |

### The fixes, in order

1. **`value_type`: done (2026-11-24).** 251 relations were under it, 249 of them in python-main. Main's translator
   wrote a value's `type` test (and `numeric`, `truthy`, `bitsAny`) as `value(n, x), value_type(x, T)`.
   `value_type(v, t)` is keyed by the value's text, which `"12"` and `12` share. So the join held at the string
   `"12"` because some int `12` existed, in whatever file. It was the cross-file part's join of every file's types,
   and a false finding besides. Placing `value_type` per file in the split would not have fixed it: a batch makes it
   from its own nodes, so the answer would have depended on which files share a chunk. Both relations are made from
   the same type names per node (`syntax.rs`), so the translator now writes `value_type_of(n, T)`, the node's own
   type, for a node's own value. A value followed through a binding (`follow`, `static_value`) is no node's own, and
   keeps `value_type`. The 41 python-main files are regenerated. `json_rules_value_type_is_the_nodes_own` runs the
   rule over `a = "12"` / `b = 12`: the int alone is found, and the old join finds both.
   - **Where they went.** 182 relations left the list: 177 of the 251, and 5 others whose chain ran through them.
     Of the 251, 74 are still listed. 71 read a Python name too, and are under `resolve_export` now. Two are under
     crossings of their own (`D412`, `value`), and two are the JavaScript policy's (below). 15 relations are new to
     the list, all under `resolve_export`: each read its node from a relation that the fix made per file, so the
     node now enters at them.
   - **One hand-written use remains.** A JavaScript policy's two relations are under `value_type`, and are listed
     with the tail.
2. **Python name resolution, the phase `py_names`: done (2026-11-24).** This is family 1, 2,536 relations, 84% of
   the list. The design is below (§ Fix 2), and what it took (§ Fix 2: built).
3. **The library crossings, one by one:** points-to (`pt_*`), Django's models (`dj_model_q`), class hierarchy
   (`subclass`, family 3), the TSX scope tables (`item_parts`). Each is rewritten to symbols under tests written first.
   Class hierarchy (3a), class members and a Django model read by its symbol (3b): **done (2026-11-24)**, § Fix 3:
   built. A model's `Meta` options and managers (3c), and its hooks (3d): **done (2026-11-25)**, § Fix 3c: built,
   § Fix 3d: built. Points-to (3e), the phase `points_to`, and Django's four call-graph policies over symbols:
   **done (2026-11-25)**, § Fix 3e: built (note t55 is its design). The data layer's purity call graph
   (`plp_anycand`, 3f): **done (2026-11-25)**, § Fix 3f: built. The TSX `/* global */` items (`item_parts`, 3g):
   **done (2026-11-25)**, § Fix 3g: built.
4. **The tail:** 146 crossings of 1–14 relations each, read one by one, as 0652's were. In progress, § The tail.

### Fix 2: the design of `py_names`

**Where the node enters.** Python's resolution is already keyed by symbols: `exports(m, x, q)`,
`exported(m, x)` and `resolve_export(q, r)` hold no node. The node enters in `names.rerail`'s `sym_of(n, q)`, in
two rules:
- `sym_of(n, q) :- qn(n, q0), resolve_export(q0, q)`. It is cross-file only because `resolve_export` is, and
  `export_query(q) :- qn(_, q)` gathers every file's queried names into it.
- `sym_of(n, q) :- attr_of(n, o, a), sym_of(o, c), class_def(_, c), q = @dot(c, a), qualname(_, q)`. This is a
  class's member, found by its qualname in any file.

`qn` (a name through the file's own bindings and imports, and its attribute chain) is per file, and so are the other
two rules of `sym_of`.

**What the memo can key by.** A file's memo entry is keyed by the setting rows it can read (`settings_scopes`,
0665). Two kinds of read are keyed by the file's own rows:
- a setting read at the file's own path (`tsx_names__target_at(p, s, t)`);
- a setting read by a key that such a row names (`tsx_names__exports(t, …)`).

A read by any other key is of every file's rows. That includes a symbol computed from the file's content, such as
`resolve_export(q0, …)` asked by `qn`'s `q0`. Every change to any module's exports would then miss every file, which
is 0083 §2.3's every-file miss again. So the phase hands on what each file reads by its path, and each module's
facts by the module, anchored at the file's path.

**The phase** (`native/src/code_rule_dsl/typing/programs/py_names.rerail`, `.phase py_names`). It runs over the same
files, first, to its fixed point, and holds no node in its cross part. Its own per-file rules may use nodes
(`def_name`, `qualname`, `import_alias`), as `tsx_names`'s do (`import_spec`). It reads the modules' definitions
and import statements, never a name's use, so an edit that changes neither changes none of its rows. It hands on:
- `reads_at(p, m)`: the file at path `p` can name something of workspace module `m`. That is its own module; each
  module its imports name, and each module above or below one in the package tree (a name through `import pkg` is
  resolved in the module its last dot ends); and the module of each definition one of those exports.
- `exports(m, x, q)`: module `m` exposes `x`, which denotes `q`, re-exports and star imports followed
  (`imports.rerail`'s `exports`).
- `class(m, c)`, `defined(m, q)` and `public(m, x)`: module `m` defines the class `c`, and the qualname `q` (its
  defs, classes and their members, nested). `x` is part of `m`'s public interface (`public_name`).

**The reader** (`names.rerail`, `upgrade.rerail`). `names.rerail` is what a rule includes: `scopes.rerail` (the old
`names.rerail`'s per-file half: who binds a name, what a reference reads) and the reads of the phase.
- `export_at(n, q0, q) :- qn(n, q0), split_dot(q0, m, x), <n's file at p>, py_names__reads_at(p, m),
  py_names__exports(m, x, q)`.
- `sym_of(n, q) :- export_at(n, _, q)`, and `sym_of(n, q0) :- qn(n, q0), !export_at(n, q0, _)`: a name whose
  module exports no such name (an external module's, a class's member read through its dotted name) is itself.
- `sym_of(n, q) :- attr_of(n, o, a), sym_of(o, c), <n's file at p>, py_names__reads_at(p, m),
  py_names__class(m, c), q = @dot(c, a), py_names__defined(m, q)`.

  A member is defined in its class's module, so the class's module anchors both reads.
- `site_exported` reads `py_names__public(m, x)` for its own module, which is anchored by `reads_at`.
- `upgrade.rerail`'s `sym_at` reads `exports` as `export_at` does (`up_export`).
- `export_query` no longer gathers every file's `qn` names: only a from-import's, the chains `exports` follows.
  `resolve_export`, `exports` and `exported` stay in `imports.rerail` for the phase, and for the rules that ask about
  modules themselves (`reaches`, the data layer's `public-api-triples`), which hold no node.

Each of these reads is by the file's own path, or by a module its own `reads_at` row names, so a file's entry is
keyed by those rows alone (`settings_scopes`, 0665). `file_local::SETTINGS_BY_PATH` gains `("py_names__reads_at", 1)`,
and `SETTINGS_BY_FOLDER` gains `py_names__exports`, `py_names__class`, `py_names__defined` and `py_names__public`.

**Why by module and not by path.** The first version handed on `resolved_at(p, q0, r)`: each file's own names,
resolved, by its path. The phase then had to read every name's use (`queried(p, q) :- qn(…)`), so every body edit
changed its rows, and its table held a row per name per file. Keyed by module, the phase's rows are a function of
the modules' interfaces, and the resolution of a name is its own file's join, as `tsx_names` does it (0665).

**Tests first**, as 0665's were:
1. The phase's exports on a small workspace: a package's `__init__` re-exporting from a submodule, a chain of two
   re-exports, a star import with and without `__all__`, a class with a method and a nested class, and an import
   of a module outside the workspace.
2. `sym_of` before and after is the same on the same workspace and on the python pack's tests. Every pack's findings
   are unchanged (the pack tests, and the data layer's 142 checks).
3. The split: `sym_of`, and the policies' relations that read it, are file-local. The node list loses the relations
   under `resolve_export`, and the test's list shrinks with them.
4. The edits: a body edit misses its own file alone. A change to a module's exports misses that module's file and
   the files whose `resolved_at` rows changed. `maintain_tests::a_moved_definition_costs_about_itself` is measured
   again.

### Fix 2: built (2026-11-24)

**The list.** 592 entries, from 3,140 (the packs and the data layer 484, the data layer's policies alone 108):

| Pack | Before | After |
|---|---:|---:|
| python-main | 2,390 | 11 |
| python | 195 | 85 |
| django | 220 | 180 |
| data-layer | 111 | 102 |
| data-layer:alone | 117 | 108 |
| javascript | 38 | 37 (a duplicate line) |
| the others | 69 | 69 |

No reader of Python names reads `resolve_export` now. python-main's 11 are its shards' sightings, which a shard can
only add to (§ Built). A Django relation the old `sym_of` hid is now its own crossing:
`goc_meta_from(c, b)` joins two classes' nodes across files, and is declared `cross_file` until fix 3 rewrites it,
as `mcs_base` and `cfr_form_base` (their bases' symbols) are.

**Tests.** `python_names_through_the_phase_are_the_cross_file_rules_answers` (tests 1 and 2): a package's
`__init__` re-exporting a submodule's class, a chain of re-exports, star imports with and without `__all__`, a
class's method and nested class, and a name of a module outside the workspace. Each use's symbol, and each public
binding, are the old cross-file rules' (run beside them in one program). `a_python_files_entry_is_kept_under_its_own_names_resolution`
(test 4): a body edit misses its own file alone, and an added method misses the four files that read its module;
each answer is a fresh run's. The pack split tests (test 3) take the list above.

**Latent faults the split exposed.** While `sym_of` was cross-file, every relation that read it was too, and the
split never checked them. Each is fixed at its root:
- *A rule that can never hold.* `unset(x)` (the empty value, `rr::lower::UNSET`) substituted into later literals
  leaves rules with `kind(-1, …)`, `xd_…(-1)` or `-1 != -1`. The split now drops a rule whose comparison of two
  constants fails, or whose positive literal holds `UNSET` at a location column, and reads a negated or aggregated
  literal with `UNSET` there as holding no location. Its numbered check (`Form::one_document`) and its text check
  (`why_not_local`) agree on them (`on_version.rs`).
- *A gated negation's ask.* The ask rule of a negation (`ask_<rel>_<adornment>`, 0065 §2a) was built from the
  literals before it alone. Where a key variable is bound only after it, the ask read an unbound variable (`$rr_kd is
  unbound in SQL`). It now reads the later positive atoms and comparisons too (`answers.rs`).
- *Two runs of one store, in one process.* The rule-file tests now run `py_names` beside their own programs, in the
  same kept space. One context's sweep removed a file another had staged before its commit named it, and every
  reader then found a chain naming a missing file ("corrupt content store"). The staged files' guard was a set in
  the run's shared object (0442), which another context does not see. A staged file now holds its space's
  `staging.lock` shared until its commit, and a sweep removes one only under that lock taken exclusive: a file lock,
  seen by every context and process, and released by a crashed one
  (`a_staged_file_outlives_another_contexts_sweep_until_its_commit`). A reader inside a read transaction whose files
  a later commit swept reads the last commit's chain (`a_reader_inside_a_transaction_whose_files_a_commit_swept_reads_the_last_commits`).
- *A warm run's manifest.* C408's files are all file-local now, so its warm run is all hits and no corpus batch
  remakes their hosts. The cold run had named its tape values' sources (it parsed them), and the warm one named none:
  a new manifest, a write in a warm run. A memo entry's `VALUES` row now keeps each embedded string's text, as a
  parsed value's row did, and a hit names its value's tape by that text's SHA-256 among the root's last runs' tapes,
  with nothing parsed or read (`guests::Parses::name_kept`). An entry kept before has no text and names nothing, as
  before. `the_run_after_a_cold_one_with_tape_values_writes_nothing` holds again. `…_of_a_names_policy_…` (no string)
  and `…_through_a_phase_…` (`tsx_names`) are its neighbours.
- *A pass's reads, in order.* The library's link (0639) orders modules of one rank by name, and the whole lowering
  by their files' includes. `python/scopes` (the old `names`' half) and `python/points-to` both add `.from` reads to
  the `annotation` pass, and now link in the other order, so the printed `.pass` differed from the whole lowering's
  (`a_program_linked_from_its_modules_is_its_whole_lowering`). Its reads are a set, as its exports are, and are
  printed sorted (`contexts::write`).
- *The switches.* `RERAIL_STRICT_NODES` and `RERAIL_NODE_CROSSINGS` are classified as preparing switches
  (`every_switch_lowering_reads_is_classified`).

### Fix 3: built (2026-11-24): class hierarchy and class members

**The list.** 549 entries, from 592:

| Pack | Before | After |
|---|---:|---:|
| python-main | 11 | 10 |
| python | 85 | 77 |
| django | 180 | 147 |
| data-layer | 102 | 101 |
| data-layer:alone | 108 | 108 |
| the others | 106 | 106 |

**3a: a class's ancestors, by symbol.** `py_names` now hands on `ancestor(m, q, b)`: class q of module m inherits from b,
at any depth. Each base is resolved as python/scopes' `class_base` resolves it, through the same three rules `base`
reads: the file's own binding, a name through an import by the modules' exports, else the name itself. It also hands on
`library_ancestor(m, q, b)`, the ancestors no module of the run defines. python/names reads them as
`class_anc(c, b)` and `class_lib_anc(c, b)`, through `reads_at` (below). Eight policies' `subclass(q, b)` and Django's
`dj_model_q` closure read them now. `dj_model_q` is gone. `mcs_base` and `cfr_form_base`, the bases outside the run
that every file's classes named, are `class_lib_anc` and no longer `cross_file`.

**3b, first half: a class's ancestors' members, by symbol.** Django's `dj_model_super(c, b)` paired a model with its
bases' class nodes in their own files, for their fields (`dj_has_field`), managers (`goc_mgr_def`) and methods
(`srl_defines`). `py_names` now hands on what each class's ancestors of the run hold, under the class's own module:
- `anc_call(m, q, x, f)`: an ancestor's body assigns `x = f(...)`.
- `anc_kw(m, q, x, k, v)`: that call's keyword argument k, of text v.
- `anc_ref(m, q, x, s)`: an ancestor's `x = …` names s at any depth.
- `anc_method(m, q, x)`: an ancestor defines method x.

They are joined in the phase, not by the reader, because a file need not read the module an ancestor is defined in.
use.py's `Leaf(Mid)` reads app's module, and Mid's base is in models_base, which use.py never imports. Django's field
relations drop the call node: `dj_has_field(c, x, t)`, `dj_attname(c, a, x)` and `dj_field_kw(c, x, k, v)`, where
their users read the keyword arguments (`unique`, `primary_key`, `to_field`). `dj_model_super` is gone.

**3b, second half: a model read by its symbol.** `dj_qs_model(q, c)`, `rmw_class(e, mc)` and `tam_instance_value`
paired a use with the model's class node in its own file (`sym_of(r, mq), class_def(c, mq), dj_model(c)`), and their
users read the model's fields through it. `py_names` also hands on each class's own members (`member_call`,
`member_kw`, `member_method`), and python/django reads a model at a node of the reading file:
- `dj_model_ref(e, mq, m)`: e names class mq of module m (sym_of), or is that class, and e's file reads m.
- `dj_model_named(e, mq)`: that class is a model, by dj_model's tests on the phase's rows: a known base itself or
  among its ancestors, or a field in its own body that is no query expression's `output_field`.
- `dj_model_field(e, x, t)`, `dj_model_field_kw(e, x, k, v)`, `dj_model_has_method(e, x)`: the model's own and
  inherited fields, their keyword arguments, and its methods.

`dj_qs_model(q, r)` now pairs a queryset with its root `r`, which names the model in q's own file, and its users read
the model through `r`. A user that holds the model's own class node (`self`'s class) reads through that node. tam's
typed values ask the same of a proven type (`tam_model_type`), in tam's own relation: the type program's facts cross
files (points-to), and asking through python/django's key took that key into the cross part.
`rmw_class` is gone, and 10 more Django entries are out of the list (549 now).

**What remains of Django's crossings.** `goc_call(c, m, mc)` still pairs a call with another file's model class: its
users read the model's `Meta` (`unique_together`, constraints, `managed`), whose shape is no member call. bwh's hooks
read each model's `save()`/`delete()` bodies and the signal receivers of every file. Both stay listed, declared
`cross_file` where they pair nodes, until each is summarised by symbol.

**Tests.**
- `python_class_ancestry_through_the_phase_is_the_cross_file_closures`: every class's ancestors and library ancestors,
  re-exports followed, equal to the old cross-file rules' answers, run beside them in one program.
- `python_class_members_reach_each_subclass_through_the_phase`: an ancestor's field call, keyword arguments, manager
  and method reach a subclass two modules away, and a class's own members are not among them.
- `a_django_model_is_read_by_its_symbol_in_another_file`: a queryset in views.py reads its model's own fields, those it
  inherits from a base in a module views.py never imports, their keyword arguments and the model's method, each at
  the queryset's root.
- `a_python_files_entry_is_kept_under_its_own_classes_ancestry`: a body edit misses its own file alone, and a changed
  base misses every file but the one that reaches no module whose ancestry changed.
- The pack split tests take the list above.

**Faults found on the way, each fixed at its root.**
- *A phase's export read by a module no key covers.* `class_anc` first read `doc(d, _, m)`: the module from the
  document's own column. The memo's settings scopes (0665) key a file's rows by its path and the modules its
  `reads_at` rows name, so this read was of every module's ancestry, and a changed base missed every file (0 of 5).
  Every reader of the phase reads through `py_names__reads_at(p, m)`, its own module among them.
- *The factorizer moved nodes into the cross part.* An export the factorizer (0099, 0488) handed on as parts joined in
  readers was keyed by its factor's join variable. Where that variable was a node and the head held none
  (`base_name(q, q0) :- base_expr(q, e), qn(e, q0)`), the parts paired a node with its symbols in the cross part.
  The factorizer no longer factors on a node key unless the head holds a node (`factorize::choose`). That is the
  root of fix 4's `exported` crossing too.
- *A collection's sweep after its commit.* The collection (0401 F4) swept a space by the rows it had written,
  after its commit and outside a transaction. Another context's commit could land in between, and its new file was
  removed. It now sweeps by the chain as it is then, under the write lock (`sweep_committed`,
  `a_sweep_after_a_commit_keeps_a_later_commits_file`).
- *A second write of a space in one transaction.* The writer's batch runs its jobs as savepoints of one transaction.
  A write sweeps the files its chain's rows do not name, by the rows as its transaction reads them. In a space's
  second write of the batch those were the first write's rows, uncommitted. The sweep removed the files the first
  write had merged away, while the committed chain still named them, and every reader on another connection found a
  chain naming a missing file ("corrupt content store"). It failed now and then in `rule_files_pass_their_own_tests`:
  a log of each removal showed `c653` removed by a sweep naming `c655`, while the reader's last commit named `c653`.
  A space is now swept only in its first write of a transaction (`connection::first_write_of_space`). That write's
  rows are the last commit's, because a write holds the write lock from its `BEGIN`. The next transaction's first
  write sweeps what the batch replaced (`a_second_write_in_one_batch_keeps_the_committed_chains_files`). Three runs of
  the rule-file test in a row had no such error.
- *A shard cannot see an entry it fixed.* A pack split in shards fails on a new crossing only, so a fixed relation of
  a sharded pack stays listed. The Python shards' logs, compared with the list, held 8 such (`exports` among them,
  a leftover of fix 2), and python-main's 1. They are out. A check that runs all of a pack's shards and refuses a
  stale entry is open (below).

### Fix 3c: built (2026-11-25): a model's Meta options and managers

**The list.** 541 entries, from 549: Django's 147 are 139. goc's `goc_call(c, m, mc)` paired a call with the model's
class node in another file, and its readers took the model's `Meta` through it: `goc_meta_opt`, `goc_meta_from`,
`goc_group`, `goc_cond_group`, `goc_opaque`, `goc_mgr_class` and `goc_unknown`, the 8 entries now out.

**Where the Meta is read.** No other rule reads a model's `Meta`, so it needs no phase of Django's own. Such a phase
would need the class line (`base`, `anc`) and the names' resolution (`sym_of`, `qn`, the modules' exports) that
`py_names` holds, and it would run python/scopes over every file a third time. `py_names` has a Django section
instead. Each file's rules turn its Meta classes' bodies into symbols (`dj_own_group`, `dj_own_opaque`,
`dj_own_unmanaged`, `dj_own_abstract`). Django's inheritance of Meta options is joined by symbol: a model takes the
options of its own Meta, of an abstract base whose Meta it takes (`dj_meta_from`: it declares no Meta, or its Meta
subclasses the base's), and of a concrete model base's, at any depth. The phase hands these on under the model's
module:
- `dj_group(m, q, g, x)`: field x is in group g. The group is unique_together, or a UniqueConstraint's `fields=` with
  no condition. g names the group by the Meta's class, the option and the element's index:
  `models_base.Coded#constraints.0`.
- `dj_cond_group(m, q, g, x)`: the same, with a condition.
- `dj_opaque(m, q)`: a uniqueness option that is not read (not a literal list, an expression constraint, `fields=` not
  a literal list).
- `dj_unmanaged(m, q)` and `dj_abstract(m, q)`.
- `dj_manager_method(m, q, a, x)`: a manager attribute a of the model, or of an ancestor, names a class of the run.
  That class, or the class its `get_queryset()` calls, defines or inherits x (`get_or_create`, `update_or_create`).

python/django reads them at a node of the reading file that names the model (`dj_model_group`, …, through
`dj_model_ref`). goc's call now holds that node, `goc_call(c, m, r)`, as `dj_qs_model(q, r)` does. For `cls` in a
classmethod, the node is the model's own class node. The list of model bases (`dj_model_base`) is one file,
included by both python/django and the phase (python/django-shared since fix 3d).

**Found on the way.**
- *`py_names`' `base` missed a class read as an attribute of a class.* `class Meta(Named.Meta)` names
  `Named.Meta` (python/names' `sym_of` rule 3), and `base` resolved only a name and an import. It now resolves the
  object, through the file's own binding or an import, and takes the attribute's qualname where a module defines it.
  The ancestry of fix 3a gains those bases.
- *`unparen` moved to python/scopes,* so the phase reads it, since python/ast includes python/names. A data-layer
  policy, swallowed-exceptions, declared its own narrower `unparen` and includes python/names. Its relation is
  `se_unparen` now.
- *Demand is a set.* With `unparen`'s `magic-transform` pragma in python/scopes, the program linked from its modules
  stated `unparen, reaches` and the whole lowering `reaches, unparen`
  (`a_program_linked_from_its_modules_is_its_whole_lowering`). Demand is stored sorted where every lowering and link
  writes it (`program_facts`), as fix 2 made a pass's reads. T's goldens are compared with their demand line sorted.
- *rerail-tests-lang did not compile.* A test passed `&Arc<Atoms>` where `&dyn AtomTable` is taken. The call
  dereferences it now.
- *Fix 3's `WRITTEN_SPACES` was in no list of decision 0104* (`every_kept_map_is_state_or_listed`, rerail-logic, a
  crate fix 3's runs left out). It holds the spaces the open write transaction has swept, and is cleared when its scope
  closes, so it is listed as state.

**Semantics kept, and one narrowed.** goc's cases pass unchanged. A concrete base's Meta options count when the base
is a model by its line (a known model base among its ancestors). Before, they also counted when the base was a model
only by a field in its own body. Then the base's chain leaves the run at a class the list does not know. Such a model
then has an unrecognised base and a Meta with unique options; none is in the cases.

**Tests.** `a_django_models_meta_and_managers_are_read_by_their_symbol_in_another_file`. views.py imports shop.models
alone, and reads from it:
- Plank's options from an abstract base whose Meta Plank's Meta subclasses. That base is in a module views.py never
  imports.
- None of them for Board, which declares a Meta of its own (unmanaged).
- Part's options from its concrete base's constraints, one of them with a condition.
- Tag's expression constraint, which is opaque.
- Part's manager, whose `get_queryset()` builds a queryset class that defines `get_or_create`.

### Fix 3d: built (2026-11-25): a model's hooks

**The list.** 538 entries, from 541: Django's 139 are 136. bwh (django-practice-bulk-write-skips-hooks) paired a bulk
write with the model's `save()`/`delete()` defs in the model's and its bases' files (`bwh_method`), and with the
receivers connected for it in any file (`bwh_hook`, `bwh_in_hook`).

**By symbol.** `py_names`' Django section hands on two more relations:
- `dj_hook(m, q, k, h)`: a hook of kind k (`save`, `delete`) that a bulk write on model q skips, with h its
  description. It is a `save()`/`delete()` that does more than call the base class's, the model's own or an
  ancestor's of the run (`Stamped.save() (models_base.py:8)`). Or it is a `pre_save`/`post_save` receiver for sender
  q, connected by `@receiver` or `.connect` in any file (`` post_save/pre_save receiver `book_saved`
  (shop/signals.py:7) ``).
- `dj_receiver_fn(m, f, q)`: function f of module m is such a receiver for q.

Each file's rules turn its own sites into symbols keyed by place ("path:line"). The phase resolves a receiver's sender
and function by the three rules a base is resolved by, and joins them by symbol. The place is a value of the file's
own rule, like a keyword argument's text, and no node crosses. python/django reads `dj_model_hook(e, k, h)` at the
queryset's root, and `dj_receiver_def(f, mq)` at a def of the reading file. bwh's in-hook test reads a save/delete
def of the reading file, its class matched to the model or its ancestors (`py_names__ancestor`). `bwh_method` and
bwh's receiver relations are gone.

**python/syntax.** The phase needed `call_arg`, `posarg`, `body_stmt`, `body_len`, `docstring` and their string
forms. Those lived in python/calls and python/ast, which include python/names, which reads the phase. They and
`unparen` are moved, unchanged, into python/syntax: the syntax helpers that read the tree alone. python/scopes
includes it, so every rule that had them still has them. `dj_test_doc` moved into python/django-shared (formerly
python/django-bases) for the phase too. The executor-coverage ledger (`gate/executor-coverage/packs.tsv`) counts two
device gaps where they now are, in the phase (`engine`): the receivers' string join and the bare-super regex. Both
were bwh's.

**Tests.** `a_django_models_hooks_are_read_by_their_symbol_in_another_file`. views.py imports shop.models alone,
and reads Book's hooks:
- the `save()` it inherits from a base in a module views.py never imports;
- a receiver connected by `@receiver`, for both signals;
- a receiver connected by `.connect`.

Both receivers are in a signals module nothing imports. A bare `super().save()` with a docstring is no hook. The
signals module's own defs are known as receivers.

### Fix 3e: built (2026-11-25): points-to as the phase `points_to`, the call graph by symbol

**The list.** 440 entries, from 538: Django's 136 are 38. They left:
- points-to's 74 `pt_` relations;
- `call_target`, `call_open` and `type_open`;
- the call-graph policies' 20: ghw's `ghw_handler`, `ghw_own_method` and `ghw_self_bound`, sora's 7 and sfu's 10;
- `tam_instance`, whose node came through `proven_type`.

The data layer's lists are unchanged.

**The phase.** Points-to's per-file rules (`lib/python/points-to-file.rerail`) state each file's flows at its nodes,
as before. Its cross-file rules (`points-to-solve.rerail`) are the old ones with every node column made a term: a
rule's per-file part became a helper that reads its nodes and gives their terms (`<rel>_t`, `<head>_l_<i>`). A
one-off script lifted them, rule for rule, and the comments were kept. `native/src/code_rule_dsl/typing/programs/
points_to.rerail` is the phase. It exports seven relations by path and term (`call_target_at(p, t, f)`,
`type_of_at`, `proven_type_at`, …). They are settings scoped by path (`SETTINGS_BY_PATH`, 0665), with no folder hop:
a file reads its own rows, so a change misses only the files whose answers changed. `lib/python/points-to.rerail` is
the interface. It reads them back at the file's own nodes (`call_target(c, f) :- pt_tm(c, t),
points_to__call_target_at(p, t, f)`), so `call_target`'s second column is the callee's symbol.

**Terms** (`pt_tm(n, t)`, in the per-file part):
- A module: its name, or its path when it has none.
- A live definition: its scope's term and its name (`m.C.run`), and `#k` for the k-th live def of that name in
  that scope.
- A dead definition, and any other node: its nearest scope's term, its start relative to that scope's, its length,
  and how many of its ancestors span exactly it (`m.C.run@14+5^0`). The count tells a node from a parent of the same
  span: a `type` over its identifier, an expression statement over its call.
- A guest's node (a quoted annotation's code): its own document, which every string of that text shares
  (decision 0431), and its place there (`~1825239168@0+4^1`). The annotation context names it (`pt_gtm`), and the
  host reads that, since only the context reads a guest's nodes.

So an edit changes the terms of its own scope only. A node's kind is not in the term: the planner keeps a kind
symbol and a name symbol apart (`$k is a Name and a Kind`), and the count does the kind's job.
`pt_def(f, q)` gives the defs' terms alone. A rule finds a definition's node in its own file through it, never
through `pt_tm` with the node free.

note t55 planned terms by expression shape (`T(o).a`). Positions in the scope are what was built: they come from
the tree alone, need no rule per expression form, and change with the same edits, those inside the scope.

**The call graph by symbol.** python/calls' `calls`, `calls_open` and `reach` are over symbols. It adds `method_sym`
and `mro_method_sym`, python/scopes' `method` and `mro_method` by the def's symbol. The four Django policies that
walk the graph keep one shape: each file states its own facts by symbol, the walk is a cross rule over symbols alone,
and the finding reads its node in its own file.
- **get-handler-writes (ghw), sync-ORM-from-async (sora).** A handler, or an async def, is its class's `class_def`
  name and its def's symbol. A call in it is its term. Each def's first write, or first query, is stated in its own
  file (`ghw_wrote`, `sora_ran`), and a finding names it by path and line (`{p}:{l}`, no longer a node of another
  file). When several defs reached write, the finding names the least by symbol: a `min` over symbols is refused,
  since it would order their ids.
- **select-for-update (sfu).** Whether a node runs in a transaction is stated per file: for reasons of its own file
  (`sfu_loc`), or if a context manager or a decorator of the run opens one (`sfu_via(n, "cm:" + symbol)`,
  `"wrap:" + name`). Which of those open one is a fixpoint over symbols (`sfu_atomic`): a context manager whose
  `yield`, or a wrapper whose call of its argument, runs in a transaction for either reason. It was a recursion over
  nodes of every file.
- **dispatch-in-atomic (dar).** `dar_view` and `dar_live` are over symbols. Each file states its own calls
  (`dar_call`).

The rule left on each policy's list is a relation that reads another family's crossing (`dj_view_class`,
`ghw_instance_class`), under that crossing's own name.

**Tests.**
- The oracle tests (`typing::oracle_tests`, 30): the old analysis's answers, mapped to terms, equal the phase's.
  The Jedi comparison reads the same spans (`P3_SPANS` by term), and `rules_read_the_python_program_through_lib_calls`
  reads both by term.
- Every rule file's own tests (`rule_files_pass_their_own_tests`): python/calls' cases, and Django's policies' cases
  with every expected line. python/calls' test answers pair two files' lines, and are declared `cross_file`.
- The pack split tests: Django's, the data layer's, python's.
- The corpus tests of fixes 2–3d and the prepared program's identity.
- The executor-coverage ledger counts one more `lib` file for the bound-pattern match (741 programs): points-to's
  rules are two files now, and no rule gained a gap.

**Cost.** The four call-graph policies over 143 files of posthog (`rerail-shared/bench/posthog-1644`, its `api` and
top-level modules), the CLI at O0 with `test-hooks` on this Mac, one run each:

| | Wall | CPU | Instructions | Peak RSS | Findings |
|---|---|---|---|---|---|
| Before (fcc1ef18c3), load 19 | 22.6 s | 35.2 s | 0.725 × 10¹² | 1.18 GB | 0 |
| After, load 4 | 18.5 s | 35.0 s | 0.668 × 10¹² | 1.40 GB | 0 |

The loads differ, so the instructions are the comparison: 8% fewer. A first version of sfu took 40% more: its
`sfu_built_read` paired every node of an assignment's right side with every later read of the name (22 million
rows). The old rule's `sfu_in_atomic(i)` had bound it. It now starts from the lock calls. The phase's own largest
costs are `pt_tm` over every node, about 1.6 s of CPU between the two parts.

### Fix 3f: built (2026-11-25): the data layer's purity call graph by key, and three engine fixes

**The list.** 328 entries, from 440:
- 69 left with the rewrite below: every `pc_` and `plp_` relation of `data-layer` and `data-layer:alone`.
- 43 more left with the push fix (below), across packs: the data layer's 16 and 14 alone (`app_semantic`, `er_site`,
  `la_find`, `pos_reachable`, …), Django's 5 (`dar_view`, `ghw_model_call`, `sfu_atomic_view`, …), python's 5,
  typescript's 2 and python-main's 1 (`exported`).

`rr_found_through` stays on the list. It is the tail's.

**The rewrite.** `lib/data-layer/purity-calls.rerail` built the call graph between defs by node (`pc_edge(f, g)`), and
`pure-lazy-runtime-purity.rerail` walked it in the cross part. Now a def is its key: its file's path and its
qualified name, `cat(p, "::", q)` (`pc_key(f, k)`).
- Each file states what its records call as requests: a name or a dotted call (`pc_ask_s`), and a method of the
  record's own class through `self` or `cls` (`pc_ask_m`).
- The cross part answers them over keys (`pc_res`, `pc_mans`), so `pc_edge(f, g)` is over keys.
- The purity analysis (`plp_kterminal`, `plp_owner`, `plp_colored`, …) runs over keys too, and decides which defs
  count (`plp_kan`).
- The findings are each file's own, at its own nodes (`plp_at`). They are kept by key (`plp_kept`), and PLP-007
  reads its lazy targets by key (`plp_klazy`).

eager-resolve's same-file walk (`er_dirty` over `er_edge`) carries the file's path. Its skip list is tested in the file
(`er_part`), so the walk reads no def.

**Three engine fixes.** The rewrite found them. The first two dropped or refused rows without saying why.
- **A row is located by its path, not by a name built from it.** A built name (`cat(p, "::", q)`) places a row in
  its file, and 0665 counts it among a relation's path keys. But `doc` holds no such name. `locate` relayed
  `plp_find_k` by its key column, every row's lookup in `doc` found no document, and the cross part read no row: the
  first run of the rewrite reported nothing. `FileLocal::paths` holds the columns that are a path as it is (a
  derived relation's, by the fixpoint that finds path keys, without the name builders). The relay and `ByPath` find
  a row's document by one of them. A relation placed by a built name alone is not relayed. It moves to the cross
  part. Before, eager-resolve's `er_edge` and `er_skip` were relayed by their symbol column in the same way.
- **A later round reads a relay an earlier round made.** `locate` repeats until nothing is blocked. A target relayed
  in one round, whose reader a later round moves to the cross part, was blocked in the later round, and refused by
  0652. The later round now rewrites the moved reader to read the existing relay.
- **A bound of an id's range goes with a pushed join.** The push pass (`PushLocalJoins`) hands a join's bounded
  variables on to the reader, so that a narrow copy and its original push one join (0575). Lowering writes a bound on
  every node and document (`$f < 6917529027641081856`, `$d < 1610612736`), so every pushed join handed its nodes and
  documents on to the cross part, though no reader read them. Such a bound now stays with the join. It is the same
  in every copy, so the copies still share their join. This fix took the 43 relations off the list.

**Tests.**
- `a_row_is_located_by_its_path_not_a_name_built_from_it` covers:
  - the path columns of a relation keyed by a built name and a path;
  - the relay by the path;
  - a relation keyed by a built name alone, which moves to the cross part;
  - its rule, moved in the next round, reading the relay made in the first.
- `an_id_range_bound_goes_with_the_pushed_join`: the join hands on the name alone, and keeps both bounds.
- Every pack's split test, every rule file's own tests, the corpus tests, the plan tests and the file-local tests.
- boot's quick tier, apart from two Metal tests that a Linux host cannot run (no device).

The data layer's tests reach no finding of these two policies. So the rewrite was compared on a 224-file sample of
the data layer's repository: the old engine with the old rules against the new engine with the new rules, in each
of the purity policy's three modes. The policy's `input_error` rule was removed from both copies, because the CLI
refuses a policy that reads it, and nothing derives it.

**Cost.** The CLI at O0 with `test-hooks` on this Mac, one run each. CPU is user time.

| Policy, mode | Findings, old and new | Instructions, old → new | Peak RSS, old → new |
|---|---|---|---|
| purity, plugin (the default) | the same | 0.574 → 0.494 × 10¹² (CPU 27.4 → 24.7 s) | 2.83 → 2.69 GB |
| purity, `scan_paths` | the same, 8 | 0.574 → 0.493 × 10¹² | 2.85 → 2.68 GB |
| purity, `scan_source` | the same, 8 | 0.205 → 0.202 × 10¹² | 0.80 → 0.65 GB |
| eager-resolve | the same (the allowlist's one diagnostic) | 0.304 → 0.309 × 10¹² | 1.39 → 1.36 GB |

The purity analysis runs once per key, not once per def node, and its findings join in their own file. That is the
14% fewer instructions.

**For the list's work.** With `RERAIL_SHOW_CROSSINGS=1`, a pack split test prints each relation it finds and the
crossing it is under (run with `--no-capture`). This fix's stale entries, those of the `:alone` and sharded runs
included, came from the union of one run's printed lines (§ Open).

### Fix 3g: built (2026-11-25): a `/* global */` comment's items in its own file

**The list.** 312 entries, from 328: javascript's 11 and typescript's 5, every relation under `item_parts`.
javascript's `rr_found_through` stays, under its other crossings (`capital`, `unescape`).

`lib/tsx/eslint-b-scope.rerail` split a `/* global */` comment's items (`"b:readonly"` is `b`, `"readonly"`) through a
table keyed by the item's text, `item_parts(i, x, v) :- gitem(_, i, _), …`. Leaving out the comment made it a table
of every file's items, with no location. So `global_item(c, …)`, its one reader, joined it in the cross part, with
the comment's node. Everything built on `global_item` followed it there: `gval`, `global_named`, `global_call`,
typescript's `global_hit`. An item's parts are a function of its text alone, so `global_item` now splits it in the
comment's own rule, and `item_parts` is gone.

No test read a `/* global */` comment before. typescript-eslint-no-unused-vars now has a case of two files
(`global_comment_items_are_read_in_their_file`), with a setting, an `off` item and an item after the comment's
line break. Its findings, and eslint-no-redeclare's, are the same as the old engine's with the old rules. The pack split
tests and every rule file's own tests pass. javascript's split test runs in 3 shards, so its stale entries came from
the union of their `RERAIL_SHOW_CROSSINGS` lines (§ Open).

### The tail (2026-11-25, in progress)

After fix 3g, 110 distinct crossings were left, of 1–11 relations each. Most are one mistake. A table is keyed by a
value (a name, a comment's text) and its rule leaves out the node the value came from (`comment(_, w)`,
`fn_words(_, w)`). So the table holds every file's values. Its readers join it at their own nodes, and they all land
in the cross part. Some of these are wrong as well as misplaced. A per-file ESLint rule then depends on other files:
`capital(x)` held only for texts some identifier in the run had. The fix is the same each time: key the table by the
node, or test the value in the reader's own rule.

**Batch 1: the TSX libraries' text helpers.** The list went from 312 to 288: javascript 17, typescript 7.
- `capital(x)` is now `capital_id(n)`, a test of the identifier node. `bound_this` and no-extend-native's `proto`
  read it at their own node: a function's name, and a reference whose name is the global's.
- `unescape` and its pieces are keyed by the identifier. A piece is a span of it (`match_all` over the node), ordered
  by its column. The `match_all` over the text was a native table of texts. The lowering's helper for `match_all`
  over a symbol is keyed by the text alone, so it is cross-file wherever its text comes from.
- `upper_first_ask(n, s)`: the asker names its node. capitalized-comments keys `bare`, `url` and `upper` by the
  comment.
- no-octal-escape's message named a span's id, not the escape (`'\204539970558885890'`). On a node, a `match`
  group is a span. The message now reads its text, `'\01'` as ESLint's.

Compared on 3,671 files: up to 40 cases of each of the 121 rules in ESLint's pack test data. Both runs are the CLI
at O0 with `test-hooks` on this Mac: the old engine with the old rules (fcc1ef18c3), and the new engine with the new
rules.

| Pack | Findings, old and new | Instructions, old → new | Peak RSS, old → new |
|---|---|---|---|
| javascript (121 policies) | 9,922, the same, apart from no-octal-escape's 34 messages | 1.091 → 1.032 × 10¹² | 1.94 → 1.71 GB |
| typescript (65 policies) | 4,365, the same | 0.450 → 0.384 × 10¹² | 1.29 → 1.31 GB |

**Batch 2: the Python libraries' value tables.** The list went from 288 to 256: python 32. Each was a table of a
value, filled from every file's nodes, and read at a node:
- `fast_strip` held the empty value as a row of its own (`fast_strip(e, e) :- unset(e)`). That row names no
  document, so the relation had no location. `fast_param` now tests the empty annotation and default in its own rule,
  `(unset(a0), unset(a) ; fast_strip(a0, a))`, and fast_strip covers nodes alone. Written `a = a0` instead of
  `unset(a)`, the column is not seen as nullable, and FAST002's `route_param` is refused as joining two documents.
- `canon_number(t, r)` and `negate(a, b)`, F601's numbers, were keyed by the number's text. They are keyed by the
  literal now, `canon_number(e, t, r)` and `negate(e, a, b)`.
- `typing_name(q, r)` took its names from `ruff_qual(_, q)`. It is `typing_q(n, q)`, keyed by the node that
  `ruff_qual` reads. RUF009 tests NewType's three names itself.
- `identifier(x)` took its strings from every str literal and every name. It is `identifier_str(s)`, a test of the
  literal, and `bb_mangled(s)` with it. B009, B010, PIE804, UP013 and UP014 read it at their literal.

Compared on 473 files (Ruff's pack test data, and two files of FastAPI, NewType and TypedDict cases), all 546
policies of the python pack, in 8 runs of 70, the same CLI builds as batch 1:

| Pack | Findings, old and new | Instructions, old → new | Peak RSS, old → new |
|---|---|---|---|
| python (546 policies) | 6,708, the same; F601 50, B009 27, PIE804 20, B010 13 of them | 1.968 → 1.937 × 10¹² | 0.86 → 0.79 GB |
| FAST001/002/003, RUF009, UP013 cases | 7, the same | | |

**Batch 3: the Python causes, and three engine-made crossings.** The list went from 256 to 214: python 40 (none
left), node 1, react 1. Run alone in chunks of 70 under `RERAIL_STRICT_NODES=1`, the 546 policies of the python pack now leave nothing in the cross part
holding a node. Before, 26 causes did.

The rules: each table that was keyed by a value is now keyed by the node the value came from.
- **Format strings.** The `%` and `.format` parsers (`pct_*`, `brace_*`, `spec_rest`) were keyed by the string's
  value. They are now keyed by the format expression: `b` of `s % r`, and the call `c` of `.format`. The parse
  rules are unchanged, and only the seed rows and the key's type moved. UP030, UP031 and UP032 key their own
  relations the same way (`indexed`, `by_hand`, `mixed`), as F524 keys `open_call`.
- **Names.** `naming_word(n, x)` and the tests over it (`lower_name`, `camel_name`, `dunder`, …) take the node with
  the name. An import alias's two names are each that alias's (`acronym_of(a, x, l)`). The same goes for SIM300's
  `upper_name(m)`, for the element keys of `__all__` and `__slots__` (`sort_key(e, …)`, `nat_key`, `member_group`),
  and for `arg_dummy(nm)` and `zero_number(n)`.
- **Airflow.** `air_kind(n, m, s)` and `air_dag(c)` are tests of the call or instance. Before, they were tables of
  every file's qualified names. AIR301's hits and its callees are per call (`air301_hit`, `air301_callee`).
- **A module's privacy** (D1xx's `doc_public`) reads `doc`'s module, which ingestion writes per file. Before, it
  walked `file` for the package's directories, which is cross-file by design (0665). An unpackaged file is tested
  by its own name. `!(…)` of two literals (N811, D412) lowers to a helper keyed by its values, so it is written as
  a disjunction.
- **A nullable column read with `!unset(x)` alone places nothing.** `typing_root`'s bound and default, PYI011/014's
  default, and FBT001's `hint_part` read `kind(x, _)` instead, which both places x and leaves out the empty value.
- B005, RUF043, UP015 and ERA001 key their text tests by the literal, the mode argument or the comment.
  UP015's count over the mode's letters (`count : { flag(re), @count_matches(re, v) > 1 } = 0`) lowered to a
  helper keyed by the mode's text. It is one regex now.

The engine. Three crossings were the lowering's or the planner's, not the rules':
- **Magic sets dropped the literal that tied two nodes.** A magic rule keeps the literals before the demanded atom.
  It leaves out a derived one that would close a negative cycle. RUF034's `same_pair(x, y)` was left out, so
  `magic_unparen_bf(c) :- same_child(x, i, c), same_child(y, i, d)` joined y's file to c's by the index i. Run
  alone, RUF034 was refused (0652) and found nothing. Now a magic rule also leaves out a literal that the dropped
  literals had tied to its key and that now reaches it by a value alone. Its places are its `node`, `loc` and
  `span` columns, and the document columns. A join by a value that the program writes itself
  (`magic_lookup_fbb(x, m)` beside a scope) stays. Leaving a literal out only widens the demand. T's five programs
  transform as before. The same fix cleared `call_q`'s seven crossings.
- **`match_all` over a text read every file's texts.** The context that a `match_all` over a symbol scans was keyed
  by the text alone (batch 1). It now also holds the nearest node its literals reach, such as `strip_arg(c, v)`'s c,
  or era_line's comment for a capture of its line. The native still reads column 0, each text once. Placed this
  way, the context and the matches (a function of its texts) are derived in each file. A printed program reads back
  as itself.
- **A lookup in a constant was driven as a product.** `async_sleep_call(c, x) :- async_call(c, _, q),
  async_sleep_arg(q, _, x)` is a join of a file's calls with a table of facts. The deferred joins (0488) drove it by
  its readers' requests. Its keys became a table of every file's names, and its factors and pair cache held the
  calls. A join of a file's nodes, one of whose two parts is a constant, is now left as written and made in each
  file. A join with no node (`deps_on(dir, r)`) is driven as before.

The two entries outside python went with them: react's `sandbox_token`, a `match_all` over an attribute's value,
and node's `module_use`, whose rules join a file's uses with the traced modules' tables.

Compared on the 473 files of batch 2, all 546 policies in 8 runs of 70, against the old engine with the old rules
(fcc1ef18c3). Both are the CLI at O0 with `test-hooks`, on this Mac:

| Pack | Findings, old and new | Instructions, old → new | Peak RSS, old → new |
|---|---|---|---|
| python (546 policies) | 6,708, the same: UP031 93, F507 47, SIM300 46, N801 36, F523 32, UP032 29, ERA001 18, B005 13, RUF034 1 of them | 1.968 → 1.827 × 10¹² | 0.86 → 0.78 GB |
| AIR policies, on Airflow cases | 68, the same | | |
| N8xx and D1xx, on packaged and unpackaged modules | 153, the same | | |

**Batch 4: the other packs' value tables, and three engine fixes.** The list went from 214 to 170. Every pack but
the data layer, Django and python-main: javascript, typescript, vue, node, php, blade, sql, rust, react and next.

The rules: each table keyed by a value is keyed by the node it came from.
- **Libraries keyed by an asked string.** PHP's format strings (`printf_needs`, `scanf_assigns`, …) are keyed by the
  format argument, as Python's were in batch 3. natural-compare's key (`natural_key(n, s, k)`, eslint-b-objects) is
  keyed by the asker: sort-keys asks at a property, member-ordering at a member.
- **Tests of a node's own text**, keyed by the node: PHP's `php_falsy(n)` and `php_key_of(x, k)`, Rust's digit
  groups (by literal and part) and hex digits (`hex_upper(n, h)`), sort-imports' `key(m, k)`, default-case's
  `note(c)`, no-warning-comments' `shown_text(c, s)`, ban-ts-comment's `trimmed_length(c, n)`, React's prop names
  (`normal(a, n)`, `standard(a, std)`), next's `unwanted(o, f)`, Vue's `vns_step(e, c, n)`, `empty_alias(a, w)`,
  `says(f, w, m)` and the hyphenation and no-unused-vars `ignored`/`wrong` tests. typescript no-unused-vars tests
  its ignore patterns in each rule.
- **Tables of a fixed set, drawn from every file's rows.** no-duplicate-imports' `apart` and explicit-member-
  accessibility's `level` enumerated the shapes and member kinds some file had. They are facts now. PHPDoc's stop
  set (`doc_code_kind`, every kind any file had) is the file's non-comment nodes, `precedes_stop`'s node set.
- **Two lookups by a type's name** (`type_decl` in consistent-indexed-object-style, `vprops_type` in Vue's
  `defineProps<Props>()`) found a declaration of that name in any file. ESLint reads the file's own scopes. They
  are keyed by document. This was wrong as well as misplaced.
- SQL AL05's `use(u)` read `column_ref`'s empty qualifier (`unset`), which places nothing: `kind(u, _)`.
- no-useless-computed-key's `!(is_static(h), v = "prototype")` is a disjunction (the batch 3 pattern).

The engine:
- **`static_value_type_of(n, t)`**, beside `value_type_of`: the type of the followed value (`static_value`'s),
  per node. no-implied-eval's `code_string` read `static_value(a, v), value_type(v, "string")`. `value_type` is
  every file's (value, type) pairs, cross-file by design. W's followed reading gave each node's types already, and
  they were dropped. The literal path (documents W reads no values of) fills it from the literal's type, as
  `static_value` and `value_type` are filled there.
- **The factorization (0099) leaves a lookup in a constant whole.** `export_all(e, m) :- star_export(e, s),
  node_module(s, m)` was handed on as parts. The constant's part (`node_module`) and the key (the specifiers) were
  tables of every file's values. Their readers joined them at the file's nodes and went to the cross part (node's
  `use`). An export of a file's nodes, one of whose parts reads only constants, is left as written. The deferred
  joins have done the same since batch 3. Test: `an_export_that_looks_a_files_nodes_up_in_a_constant_is_left_whole`.
  Each node policy run alone now holds no crossing. The whole pack still holds 13, `export_all` among them, under
  `node_module`: the pack's programs together are another split, read next.
- **A Blade view's kinds are PHP's.** PHPDoc's new stop set (`doc_code(s) :- kind(s, k), k != "comment"`), read
  over a Blade template's compiled view, never finished planning. Hiding (0058 §2) names a kind compared over a
  guest's node in the guest's grammar (`php:comment`), by `kinds_label`. That knew fragments and on-demand
  embeddings, not one built at the file's load: `compiled-view` named its own kinds, `compiled-view:comment` is no
  dialect's (`qualified` refuses it), and each round qualified it again. `kinds_label` now takes any embedding of one
  dialect, and hiding qualifies a kind only in a grammar whose label is a dialect's, so a round cannot repeat an
  edit. Test: `a_kind_compared_over_a_blade_view_is_phps_and_ends`. `blade_code` itself holds no row in either
  engine: it reads `parsed(document, "compiled-view", program)`, and the engine gives no `parsed` row for a compiled
  view yet (NOTES-blade, request 3). So the three policies over the view (bladestan's two, duplicate-condition-expressions) find
  nothing, before and after, and the Blade comparison below says nothing about them.

Compared on the ESLint sample of batch 1 (121 rules' cases): 53 policies, the 14 changed javascript, typescript, react
and next ones and every node policy. The old engine with the old rules (fcc1ef18c3) against the new, both the CLI at
O0 with `test-hooks`, on this Mac:

| Policies | Findings, old and new | Instructions, old → new | Peak RSS, old → new |
|---|---|---|---|
| 53, on 3,671 files | 2,934, the same | 726 → 686 × 10⁹ | 1.79 → 1.59 GB |

Each other changed pack's policies on its fixtures, the same way (each engine reads its own rules and libraries):

| Pack | Policies, files | Findings, old and new | Instructions, old → new (× 10⁹) |
|---|---|---|---|
| php | 15, 116 | 241, the same | 45.5 → 42.7 |
| blade | 10, 55 | 2, the same | 24.8 → 22.0 |
| vue | 23, 21 | 2, the same | 78.1 → 72.7 |
| sql | 21, 20 | 118, the same | 20.8 → 20.3 |
| rust | 33, 60 | 286, the same | 38.2 → 37.2 |

**Batch 5: the data layer, and one engine fix.** The list went from 170 to 65. The data layer's 51 entries (its
policies batched) are 4, and its 59 (each policy alone) are 3. Django's bulk-not-loop took 2 more. What the data layer
still holds is the lowering's: `rr_found_through` and the negation helpers (`exists_N`) of a located output (§ Open),
and bounded-composed-graph-paths' `violation`. `exists_55` is the listed `exists_53` (plan-optimizer-v2's
`pv2_doc_text`), renumbered by the lowering.

The rules. The patterns of batches 1–4 again, and four more:
- **Tables of every file's values, read at a node.** The fix is to key them by the node or the entry, or to test the
  value in the reader's rule.
  - cached-pure-lazy-allowlist's policy entries (`pm_str`, `pm_num`, `pm_list`, … keyed by the entry; `pc_str` and
    `pc_num`, keyed by the call, for the call's own reader).
  - purity-ast's `py_repr` table, which is now core's `@py_repr` functor, called in the reader's rule.
  - Name cuts and prefixes, each now tested inline in its one reader: lazy-adapter-ownership's (`lao_base`,
    `lao_cut`), connection-service-architecture's schema name, plan-optimizer-v2's module prefix (`pv2_under`),
    pub-lines' line separator (`py_line_sep`), and bootstrap-engine-invariants' lowerer prefix.
- **A fact of a whole document, written nullary or keyed by a value.** A nullary relation of a file's rows names no
  document. A relation with a document column is placed by `docof`. Now keyed by the document:
  - post-op-side-effects: `pos_has(d, what)`, `pos_allow_fault(d)`, `pos_has_verifier(d)`;
  - conditional-architecture: `ca_role_found(d, r)`, `ca_has_compile(d)`;
  - authentication-plan-purity: `app_semantic(d, x)`;
  - schema-introspection-data-paths: `sidp_marked(d)`;
  - lazy-adapter-ownership: `lao_lookup(d, …)`;
  - bootstrap-engine-invariants: `bei_installed`.
- **A test of every file's rows, made at a node.** swallowed-exceptions joined each `except` with every file's ignore
  entries (`ignored_at`). Its findings now carry their path and line (`warn_at`, `critical_at`), and the output tests
  `!ignored(p, l)`, a test of values. lazy-architecture's first deferral (`la_defer`, a `min` over the file's own) and
  versioned-merged-identity's backing test (`vmi_unbacked`) are per file the same way.
- **Roles taken from the run's paths.** connection-structural-boundaries' `csb_path` read `file(p)`, which has no
  location, so every role and every reader of a role went to the cross part. A document's role now comes from its own
  path, the options and the globs: a document's path names a file that exists. Whether the lifecycle file exists is
  tested in the finding (`CSB030`), over a per-file candidate (`csb_lifecycle(f, x, r)`).
- **An aggregate over two documents' nodes.** cached-pure-lazy-allowlist's `item_last` took a `max` over pairs of
  nodes from any two files. It is per document now (`docof(x, d), docof(y, d)`), and its per-file items (`it_last`,
  `it_err`, `it_hashes`, …) are joined to the policy entries by key.
- **Django's bulk-not-loop.** `bnl_number(x, n)` was declared `cross_file`: the product of every field name with every
  number literal. The step's test is per node now (`bnl_number_literal(n)`). The field's test is per document
  (`bnl_number_field(d, x)`): a model that the counter's file can name declares x a number field. It reads the rows
  the phase `py_names` hands on for the modules the document reads (`member_call`, `anc_call`), as `dj_model_field`
  does. Before, any model of the run counted. A rule reading those rows with the module left free would leave a
  model's edit out of the file's memo key (0665), so it is narrowed. A literal step (`+= 1`) is still found in every
  file.

The engine. **A symbol demand on a relation of a file's nodes is installed only when constants alone ask for it**
(`asked_by_constants`, product_demand's `upstream.rs`). A demand fed by a file's rows is a table of every file's
values. The demanded relation's rules read it at their nodes and went to the cross part: the data layer's
`vmi_callee(f, x)`, asked for by each file's aliases. A demand asked for by facts alone, which no rule with a body
derives, is a fixed set and is installed as before. Test: `a_symbol_demand_on_a_files_nodes_is_of_constants_alone`.
With the demand gone, one rule lost the location the demand had given it by accident. VMI001's
`!vmi_caller(_, s, _)` asks of the whole run whether some file has caller s, and its place rule then read every file's
callers with no location. It now reads a name set that each file relays (`vmi_seen(s)`). The batched data layer
refused the run until it did.

Compared, the old engine with the old rules (HEAD before the batch, 6c5dda1725) against the new. Both are the CLI at
O0 with `test-hooks`, on this Mac, and each engine reads its own rules and libraries.

| Run | Policies | Findings, old and new |
|---|---|---|
| The data layer's 224-file sample | the 17 changed ones | 52, the same (swallowed-exceptions 26, VMI001 12, lazy-architecture 7, post-op 3, lazy-adapter 2, eager-resolve 1, plan-optimizer-v2 1) |
| The port's conformance probes, each with its recorded options | lazy-architecture 197, eager-resolve 77, plan-helpers-allowlist 77, cached-pure-lazy-allowlist 54, swallowed-exceptions 34, post-op-side-effects 29, versioned-merged-identity 12, lazy-adapter-ownership 9, authentication-plan-purity 7, public-api-triples 6, executor-publication-repairs 5 | the same, probe by probe |
| bulk-not-loop's cases (`test-rr`) | 1 | pass |

The other packs' split tests (blade, complexity, django, javascript, next, node, php, python, python-main, react,
rerail, rr, rust, sql, typescript, vue) find what they found before the demand change, so the change made no new
crossing there.

**Batch 6: Django, by the phase `py_names`, and one engine fix.** Django's rules walked the run's classes and defs by
`class_def` and `qualname` in any file: a view's line (`dj_view_q`), a custom field's (`dco_field_q`), a task class's
methods (`mro_method`), a form base's calls (`subclass`), a task's def. Each such walk joined the reading file's
nodes with another file's, so the reader went to the cross part. Fixes 3a–3d moved the same walks into the phase,
and this batch does it for the rest. Each file's rules turn their own defs, decorators and calls into symbols, the
phase joins them by symbol, and hands them on under a module. A file reads them through `reads_at`, as it reads
`ancestor`. A name is read as the file binds it or, if imported, by its qualified name (`sym_of ; qn`): in the
phase, `sym_of` alone resolves no library name (`@shared_task`, `celery.Celery`, `transaction.on_commit`).

The phase's new exports (`SETTINGS_BY_FOLDER` lists them):
- `plain_def(m, q)`: a def with no decorator. `dj_task_ref` reads it per document (`dj_plain_at(d, q)`).
- `dj_task_def(m, f)`: a def decorated as a Celery task (task-arg-model-instance).
- `dj_deferred_task(m, f)` and `dj_app_defers(m)`: a task whose class's delay/apply_async calls
  `transaction.on_commit`, and an app that makes such a class every task's (`dj_task_waits_for_commit`).
  `dj_app_defers` is a fact of the whole run. It is handed on under every module, so a file's memo entry, keyed by
  the rows of the modules it reads, is keyed by it too.
- `dj_anc_clean_call(m, q, x)`: a form base of the run calls `self.clean_<x>()` (clean-field-returns-value).
- `dj_view_model(m, q, s)`: a view or its base says `model = S` or `queryset = S.objects…` (get-handler-writes).
  The queryset helpers (`dj_manager`, `dj_qs`, `dj_qs_root`) moved to python/django-shared for it, unchanged.

The readers:
- A view is a class with a Django or DRF view among its library ancestors (`class_lib_anc`, `dj_lib_view`).
  `dj_view_q`, `dj_view_lib` and `dj_view_base` are gone, and so is function-decorator-on-method's `fdm_drf_q`.
- A custom field class is read through `library_ancestor` (default-called-once).
- A model by class name is `dj_model_at(d, q)`, keyed by the document. `dj_model_named` reads it, so the model test
  has one definition. get-handler-writes asks it of a proven type and of a view's model.
- settings-module-import resolves modules by name and document. Each file relays its imports, its
  DJANGO_SETTINGS_MODULE values, its star imports and its INSTALLED_APPS. The cross part finds the settings
  documents (`smi_doc`, `smi_named`), and the finding joins the file's import to that answer.

The engine. **A coarser demand keeps no literal that only the demanded atom tied to its key** (`magic.rs`,
`drop_other_documents`). `unparen(v, c)` with both columns bound is answered through `unparen_bf`. The magic rule
kept the prefix that had bound c, and that prefix reached v only through a value:
`magic_unparen_bf(v) :- field(c, "function", e), kind(c, "call"), sym_of(e, q), dco_w161_whole(t, q), dco_field(f, t),
call_arg(f, -1, "default", v)`. With `dco_field` per file, the split refused it as a relation of one file joining two.
Batch 3's rule now measures the key's reach over the whole rule, the demanded atom included. It also leaves out a
needed literal that reads no key variable, as long as the positive literals kept still bind the key. Test:
`a_coarser_demand_keeps_no_literal_only_the_demanded_atom_tied`. T's own `calls` program had the same rule:
`magic_home_ffb(t) :- magic_ifrom_bbff(t, x), rifrom(s, x, m, y)`, where `home_bbb` is answered through `home_ffb`
and `rifrom`'s scope s reaches t by the value x alone. The engine now leaves `rifrom` out, and
`t_programs_transform_like_t` names that one line where it differs from T's golden. The golden file stays as T
wrote it.

Django's entries go from 31 to 2 (65 to 36 in all): `dar_non_atomic_wrapped`, and `rr_found_through` under
`smi_named`. The second is the located-output item that stays open below. All 27 case tests pass.

**Batch 7: Node, two engine fixes and one run fix.** Every node entry but one hung off `node_module` or `node_global`.
Their rules read node-trace's `module_global(g, m, x)`, an `extensible` table whose rows the policies that name
globals state. A program that names none (n-no-deprecated-api, run alone) states no row. In the pack's batch, each
stating policy's rows go to its own copy (`link::Names`, `q<k>_module_global`), and the shared table keeps none. Never
stated, it is a base relation of no location to the split, so `trace_module`, `node_module`, `node_global` and every
reader of a value through them went to the cross part: `val`, `api_use`, `loader`, `module_load`, `env_global`, and
n-no-path-concat's and n-no-deprecated-api's rules.

- **The engine: an `extensible` table nothing states is stated empty, where the whole program is known.** The split
  must not decide it: a library module planned alone (decision 0639) does not see its includer's rows. t50 tried the
  structural rule ("declared and not derived is a constant") and found the engine's world facts are such relations
  too. Decision 0263 says an `extensible` table's rows come only from the files that include its own. So the
  lowering keeps `extensible` as a word of the relation's qualifiers (not core's: `env`, `option` and `load` are the
  engine's). The link states each one that no part states and that has only value columns empty: a batch's
  (`Merged::finish`) and a program's linked alone (`combine_linked_as`). Stated, it is a run constant, as stated rows
  are (t50's `.references` setting precedent). Test: `a_programs_facts_of_a_library_relation_ask_of_the_library` (the
  library's `lit_repr_threshold`, each part's rows in its own copy, is stated empty).
- **The engine: locate leaves a request where it is asked.** C's configuration tables (`c_predefine`, `c_feature`,
  …) are `extensible` too. Stated empty, a configuration became a run constant, `c_pp_tu_req(tu, spec)` a file's by
  its unit's path, and locate gave it a located form that joins `doc`. The cross part's cone of requests asks it, and
  reads no base relation (decision 0065 §2a), so the run was refused. A program stating every table was refused the
  same way before this batch (checked on batch 5's engine). A request (`WORLD`'s) is now left by locate and moved to
  the cross part with its readers, as a relation whose rows name no document is. Test:
  `c_includes_resolve_over_a_configuration_of_stated_tables`.
- **The run: a program's own empty input is bound for the per-file part too.** A library's input that core does not
  make yet (`spec_resolved`, node-resolve's "PROPOSED core input") was bound empty for the cross part's reads only.
  Once its reader moved per file, that part's planner had no statistics for it. `corpus.rs` binds the per-file
  part's own inputs empty in the same place.
- **node-resolve's specifier tables keyed by the target.** `spec_kind(name, k)`, `spec_package(name, m)` and
  `spec_allowed(name)` enumerated every file's specifiers (`node_target(_, x, _)`) and were read at a target, so the
  target node crossed. They are now `spec_kind(n, k)`, `spec_package(n, m)` and `spec_allowed(n)`, tested at the
  target from its own name, and `spec_unbang` folds into `node_target`. The readers (no-missing-require and -import,
  no-extraneous-require) changed with them.
- **checkPublish in two halves.** `spec_unpublished(n, style, msg)` asked of the target's file whether npm leaves it
  out (`npm_ignored`, `pkg_outside`), and of the package whether it is only a devDependency. Both are facts of other
  files. A file now names its targets under its document: `spec_publish_file(n, st, d, f)` and
  `spec_publish_dep(n, st, d, m)`. The cross part answers by document and path or package:
  `pkg_unpublished_file(d, f)`, which takes `pkg_outside` in, and `pkg_dev_only(d, m)`. The violation joins them,
  with the message from `spec_unpublished_says`.
- **n-handle-callback-err** tests the option's name at the function (`err_first(f, p)`), not as a table of every
  file's first-parameter names (`err_name`).

The node pack, run whole over its translation corpus (this repository's TypeScript at its pin, 602 files), gives the
same 5,767 findings with batch 6's rules as with batch 7's, line for line: 3,276 of no-missing-import and 368 of
no-unpublished-import among them, in 2% more instructions. The data layer's 18 policies give the same findings on
batch 5's engine and this one. Node's entries go from 13 to 2 (36 to 25 in all): `exists_9` under `unpublished_bin`,
and `rr_found_through` under `pkg_workspace_root`. Both are the located-output item below.

The translation suite (`scripts/ship/translations.ts`) made its stores with the default mode, and the engine refuses a
store folder others may read (decision 0574). Each translation failed before running. It now makes them mode 700.

**Batch 8: the unplaced check in each file, FAST003 by a capture's text, and two engine fixes.** Every entry left by
the lowering was its check of findings no file places (0431 R9): `rr_found_through` in seven packs, `rr_found_node` in
PHP, and the negation helpers `exists_N` of a located output in five.

- **The lowering: the negation is made in each file, and each finding's check is a positive join with it.** Before,
  each finding wrote its node by its clause (`rr_found_node(clause, n)`, or `rr_found_through` where findings are
  placed through a string), and the program's one rule refused those of no file's document in the cross part. A
  finding at a location read back from the relay (0665, `no_redeclare`'s `@place(f, l)`) wrote its own negation,
  `exists_N(d) :- V(v), docof(v, d)`. Both held a finding's node in the cross part. Now a program makes, once, in each
  file's part, the nodes no file places: `rr_unplaced_node(n) :- node(d, n, _, _, _), !doc(d, _, _)`. Where findings
  are placed through a string, a tape value's nodes are placed together (`guest_place` puts each of its nodes at each
  string that holds it), so the test is of the values: `rr_unplaced_value(d) :- tape_of(d, _), !doc(d, _, _),
  !rr_placed_value(d)`, and `rr_unplaced_through(n) :- rr_unplaced_value(d), node(d, n, _, _, _)`. Before, it was of
  each node (`!rr_placed_at(n)`, a row for every node of every value). Both are kept relations (`no_inline`): inlined
  into a finding's rule, the node and its document became a part that meets the rest at the document alone, which
  `ProjectExistentials` moved into an `exists_N(d) :- V(v), node(d, v, …)` reading the finding's node, in the cross
  part for a finding made there. A finding at a location reads `rr_unplaced_doc(d)`, the documents no file's. Each finding's rule is
  `rr_unplaced(clause) :- V(…), rr_unplaced_node(v0)` (or `docof(v0, d), rr_unplaced_doc(d)`), positive, and an output.
  A program still makes one negation, not one a finding (the reason R9 gathered them: three strata a finding). Only a
  guest-reading program reads `tape_of`, a guest relation, as before. Hiding takes a finding's placed column from the
  new atoms' first column. Tests: `a_program_refuses_its_unplaced_findings_in_one_rule` (one rule of the nodes no file
  places holds the negation; each finding's rule joins it, positive), `a_finding_no_file_holds_is_refused_naming_its_clause`,
  `a_packs_programs_share_their_unplaced_findings_relations`, and `a_session_passes_on_a_stratum_its_changed_kinds_do_not_reach`:
  an edit of a placed file no longer reaches `rr_unplaced` at all. The nodes no file places did not change, so it is
  neither an input that changed nor a stratum reached that saw no row.
- **FAST003 (python-main) passes the path parameter's text, not its node.** The route's `{k}` is a node of the
  brace-format tape, a value every file holding the text shares (0431). Its helpers (`param_eq`, `fn_has`, `init_has`,
  `call_has`, `pyd_field`, `alt_2`, `alt_10`, `alt_13`, and the demands of the last two) took that node and compared
  texts. `alt_2` and `pyd_field` were declared `cross_file` (0652): they joined any file's node of the text. Each
  now takes the text (`KT`, bound once as `text(K, KT)`) and compares its own node's text with it in its file. The
  alias branch compares the alias's value with `KT` too.
- **The engine: an aggregate's context keeps the text it is asked for.** `placed_texts` left out a `text(v, t)` whose
  node no atom of the context binds. FAST003's `K = c3, text(K, KT)` binds `K` through an equality, so the context's
  head held `KT` with nothing binding it ("$KT is unbound in SQL"). A node equal to a bound one is bound, and a text
  the context is asked for stays. Test: `a_context_keeps_the_text_it_is_asked_for`.
- **The engine: a guest language's kind is no Python matcher's.** The pass that runs a rule's tree-shaped parts as
  matchers (`Absorb`, 0106 §4) took `kind(c, "braceFormat:identifier")` with its parent edges as a Python matcher,
  which did not lower ("Kind `braceFormat:identifier` is invalid"). A kind or field of another language stays a join.
  Test: `a_guest_languages_kind_is_no_python_matcher`.
- **The engine: a pushed location lookup is narrowed to the finding's nodes.** A cross-file finding's place is read by
  `doc(d, p, _), node(d, v, …), line(v, l), col(v, c)`, a lookup of every node. The push pass (0281) left that lookup
  in the cross part while the cross part read base relations anyway, and pushed it into the chunks otherwise, so that
  the per-file memo works. With the crossings above gone, the data layer's cross part reads none, and the lookup was
  pushed whole: 1.5 M rows over 277 files for `swallowed-exceptions`, whose peak went from 442 MB to 717 MB. Now where
  the rule reads each of the lookup's nodes from a relation whose every rule binds it in the chunks
  (`WARNING(x, s) :- warn_at(x, s, p, l), !ignored(p, l)`), the pushed join also reads that relation's domain, a
  per-file helper of those reads (`fl…_WARNING(x) :- warn_at(x, _, _, _)`). The rule's own read implies the domain's,
  so the join keeps every row the rule uses. `swallowed-exceptions` is back to 448 MB. Test:
  `a_pushed_location_lookup_is_narrowed_to_the_findings_nodes`.
- **The engine: hiding's copy of a relation keeps its qualifiers.** Hiding (0058 §2) copies a derived relation read
  with a guest's node as `rr_in_<grammar>_<column>_<relation>`, with that column seeded. The copy had no qualifiers.
  `rr_unplaced_through`, read over an ESLint directive's node in the JavaScript and Node packs, was copied without its
  `no_inline` and inlined, which made the `exists_N` above. A copy now has its relation's qualifiers. Test:
  `a_copy_keeps_its_relations_qualifiers`.

Results, on the checkout's engine against batch 7's, each run with its own stores:
- The data layer's 18 policies of batch 5, over 277 files of its repository, one at a time: the same findings in each,
  and no crossing (batch 7: `rr_found_through` in five, `exists_4` or `exists_8` in two). Instructions and peak memory
  are within the runs' spread of batch 7's (`swallowed-exceptions`: 255 M against 268 M, 447 MB against 447 MB).
- The Node pack over its 20-finding sample: the same findings, no crossing (batch 7: `exists_17`,
  `rr_found_through`), 305 M instructions against 311 M, 322 MB against 326 MB.
- ESLint's no-loss-of-precision over its 40 cases and 16 more that do and do not lose precision: the same 12 findings.
- FAST003 finds nothing in the pack's sample on either engine: `guest_of` has no rows in its program (§ Open). Its
  rewrite was checked with `guest_of` given by a stub relation: the same findings as the rule before it.

**Batch 9: the last three, and BCGP's decorators linear.** The list is empty.
- **The data layer's `violation` under `bcgp_field`.** BCGP009's second rule tested `graph_type`'s line text in the
  cross part: `bcgp_field(p, x, "graph_type", l), bcgp_line(p, l, sp), match(…, sp)`, a span read where `bcgp_field`
  (a `min` per path and class) is. The line test is each file's now, `bcgp_optional_type(p, l)`, and the rule joins on
  the line number. Both forms of an optional `graph_type` (`: … | None`, `= None`) give the same two findings as
  before on a copy of `intent_graph.py` made to have them.
- **BCGP's decorated methods, linear.** `bcgp_decorated(p, a, k, m)` paired each decorator line with each later def
  and counted the lines between: cubic in a file's length, so the policy passed 6 GB on the sample's 20,000-line file,
  on batch 7's engine as on this one. A decorator's run of `@` lines is walked instead (`bcgp_deco_run`), and the def
  is the first line after it. The 12,000-line prefix: the same 30 findings, 1.2 GB to 236 MB, 425 M instructions to
  78 M. The 277 files: 30 findings, 551 MB, where no engine finished before. A two-decorator method, and one whose
  decorator a blank line parts from its def, are found and refused as before.
- **Django's `dar_non_atomic_wrapped`.** The wrapped views were a table of call nodes, and `dar_wrapped(f)` joined a
  def's node with them by name. The wrapped names are symbols now, and a view is refused by its own name where the
  names meet: `dar_view_fn(fq, q)`, each file's (q `""` for a def with no name), then `dar_view(fq) :- dar_view_fn(fq,
  q), !dar_non_atomic_wrapped(q)`. A project with a plain view, a urls-wrapped one, a decorated one, and one of each
  calling a helper gives the same two findings as before.
- **JavaScript's `significant_bits`.** `radix(n, …)` was keyed by a literal's text (`radix__key_slice0`, a table of
  every file's texts). It is keyed by the literal now, `radix_digits(n, digits, zeros, b)` over `raw_number(n, r)`. The
  same 12 findings.

## Open

- **FAST003's `guest_of` has no rows.** A violation that reads a recursive relation over one that reads `guest_of(c, h)`
  gets no `guest_of` rows in its program, on batch 5's engine as on this one: FAST003 (python-main) finds nothing in
  the pack's sample. The smallest program that shows it is a violation reading `ra`, recursive over `dm`, where `dm`
  reads `guest_of(c2, h4)`.

- **A sharded pack's stale entries.** Each shard runs in a process of its own and sees its own policies, so no shard
  can say a listed relation no longer crosses. Fix 3's were found by comparing the shards' logs with the list by hand,
  and fix 3f's from the union of one run's `RERAIL_SHOW_CROSSINGS` lines. The same holds for a run of each policy
  alone (`data-layer:alone`).
  The gate needs a test that unions the shards' logs and refuses a listed relation none of them names.

- **A `match_all` over a node that only a derived relation binds scans nothing.** `match_all("re", n, m)` goes to
  `rx_span`, which scans nodes of the kinds the span analysis gives, only when a `kind(n, "K")` atom of the same rule
  binds n (`lower.rs`, `kind_bound`). Otherwise it goes to the batch's `matchall_` relations. Those scan `source`,
  `src_line` and comment spans, so a node of another kind gets no row, and nothing says so. `pk(n) :-
  kind(n, "property_identifier")` and `pk(n), match_all("[a-z]+", n, m)` give nothing. eslint-b-objects'
  `escape_piece` writes one rule per kind until the dispatch reads a node column's kinds from the span analysis.
  The span analysis's error for an unbounded subject then applies.

- **An aggregate over a text's own rows is keyed by the text.** `count : { flag(re), @count_matches(re, v) > 1 } = 0`
  lowers to a helper `rr_h_agg_…(v)` over a context of the text alone: a table of every file's texts, in the cross
  part (UP015's, batch 3). UP015 writes its test as one regex. The aggregate's context can hold a node, as
  `match_all`'s does now (batch 3), with its readers joining on both.

- **Where a finding that pairs two files' nodes is written.** `call_target(call, def)` is an example. An output
  keeps its node (§ Built), but once its family is rewritten it names the second file's node by that node's symbol,
  or by the definition's place read in its own file. This is settled family by family, as each is rewritten.
