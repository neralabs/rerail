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
   (`plp_anycand`, 3f): **done (2026-11-25)**, § Fix 3f: built. `item_parts` remains.
4. **The tail:** 146 crossings of 1–14 relations each, read one by one, as 0652's were.

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

## Open

- **A sharded pack's stale entries.** Each shard runs in a process of its own and sees its own policies, so no shard
  can say a listed relation no longer crosses. Fix 3's were found by comparing the shards' logs with the list by hand,
  and fix 3f's from the union of one run's `RERAIL_SHOW_CROSSINGS` lines. The same holds for a run of each policy
  alone (`data-layer:alone`).
  The gate needs a test that unions the shards' logs and refuses a listed relation none of them names.

- **Where a finding that pairs two files' nodes is written.** `call_target(call, def)` is an example. An output
  keeps its node (§ Built), but once its family is rewritten it names the second file's node by that node's symbol,
  or by the definition's place read in its own file. This is settled family by family, as each is rewritten.
