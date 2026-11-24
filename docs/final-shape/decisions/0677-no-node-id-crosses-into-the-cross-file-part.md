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
2. **Python name resolution: the phase `py_names`.** This is family 1. It covers 2,536 relations, 84% of the list.
   The design is below (§ Fix 2).
3. **The library crossings, one by one:** points-to (`pt_*`), Django's models (`dj_model_q`), class hierarchy
   (`subclass`, family 3), the TSX scope tables (`item_parts`). Each is rewritten to symbols under tests written first.
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
files, first, to its fixed point, and holds no node in its cross part. Its own per-file rules may use nodes (`qn`,
`def_name`, `qualname`), as `tsx_names`'s do (`import_spec`). It hands on:
- `resolved_at(p, q0, r)`: in the file at path `p`, the qualified name `q0` (a `qn` of that file) is `r`. Re-exports
  and star imports are followed (`resolve_export`). Where `q0`'s module exports no such name, `r = q0`.
- `reads_at(p, m)`: the file at `p` reads module `m`. That is its own module, the module of each `r` it resolves to,
  and every package above them.
- `class(m, c)`, `defined(m, q)` and `public(m, x)`: module `m` defines the class `c`, and the qualname `q` (its
  defs, classes and their members, nested). `x` is part of `m`'s public interface (`public_name`).

**The reader** (`names.rerail`, `upgrade.rerail`):
- `sym_of(n, q) :- qn(n, q0), <n's file at p>, py_names__resolved_at(p, q0, q)`.
- `sym_of(n, q) :- attr_of(n, o, a), sym_of(o, c), <n's file at p>, py_names__reads_at(p, m),
  py_names__class(m, c), q = @dot(c, a), py_names__defined(m, q)`.

  A member is defined in its class's module, so the class's module anchors both reads.
- `site_exported` reads `py_names__public(m, x)` for its own module, which is anchored by `reads_at`.
- `upgrade.rerail`'s `sym_at` reads `resolved_at` as `sym_of` does.
- `resolve_export`, `export_query`, `exports` and `exported` leave the per-file programs. They stay in
  `imports.rerail` for the phase, and for the rules that ask about modules themselves (`reaches`, the data layer's
  `public-api-triples`), which hold no node.

`file_local::SETTINGS_BY_PATH` gains `("py_names__resolved_at", none)` and `("py_names__reads_at", 1)`, and
`SETTINGS_BY_FOLDER` gains `py_names__class`, `py_names__defined` and `py_names__public`.

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

## Open

- **Where a finding that pairs two files' nodes is written.** `call_target(call, def)` is an example. An output
  keeps its node (§ Built), but once its family is rewritten it names the second file's node by that node's symbol,
  or by the definition's place read in its own file. This is settled family by family, as each is rewritten.
