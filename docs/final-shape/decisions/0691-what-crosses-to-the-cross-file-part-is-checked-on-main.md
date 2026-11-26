# 0691: What crosses to the cross-file part is checked on main, against lists that only shrink

Status: building

## Context

A program runs as a file-local part, re-derived for each edited file, and a cross-file part, maintained from deltas
(0051). Whatever reaches the cross-file part is paid for at every edit of every file it came from. The integration
branches stated what may cross, and checked it (records of those branches, cited here by number):

- **0649.** No cross-file rule reads a node's position (`visit_index`, `node_order`, `line`, `col`, `end_line`,
  `ast_depth`), except one whose head is an output: every edit moves those for each node after it.
- **0652.** Every relation is per-document unless it is declared `.decl r(...) cross_file`. A relation the split
  sends to the cross-file part for a reason of its own is a crossing, and an undeclared one is refused. The reasons:
  one of its rules joins two documents' rows, reads a document's rows with no location, or the relation is made
  natively. These are not crossings: a relation that reads a cross-file relation (it follows that relation); a
  relation whose rules read no document's rows; a single plain positive read of one document's rows, which is a union
  of each file's rows. A helper the lowering made (`rr_…`, `fz`/`fzk`/`fl` with a hash, `xd_`, `rr_in_<label>_`)
  shares its source relation's declaration. A `@place` finding's relation (`rr_place_…`) that only outputs read is
  meant.
- **0677.** No node id, and no value computed from one, enters the cross-file part. A node id is its document's and
  its slot's: a cross-file relation holding one changes for every reader whenever the file's ids change, though
  nothing changed in meaning. Cross-file facts are keyed by symbols, paths, document keys and values, and a file's own
  rules relate its nodes to them. The node columns are inferred to a fixpoint, seeded from the declared `node` and
  `loc` columns. A node's document, text, digest and source are values, not ids. Not named: a relation that follows
  another cross-file relation's node, a file-local relation the cross part derives again, and an output (a finding's
  place).

The branches found main-like rule sets far from these (oscar: 3,140 node crossings at the start), and fixed them
over many commits, together with gaps in the split itself. Main had none of the checks, and the user asked for them
on main, without a merge, so that main's rules are fixed and no new rule breaks them.

## Decision

1. **The checks, as the branches have them, measuring only.** Nothing is placed differently.
   - `cross_file` is a `.decl` qualifier (`front::ast::CROSS_FILE`), and the printer writes a relation's source
     qualifiers back.
   - `locate`'s moves also carry `MOVED_CROSS_FILE`, so a move is told apart from a declaration.
   - The version split records its undeclared crossings (`FileLocal::crossings`).
   - `node_crossings` names the relations a node enters, each under the crossing it follows (the split's causes).
   - `position_reads` finds 0649's reads.
2. **Reports by default, refusals on request.** Preparation checks each split it makes. `RERAIL_STRICT_CROSSINGS`,
   `RERAIL_STRICT_NODES` and `RERAIL_STRICT_POSITIONS` refuse a program. `RERAIL_SHOW_CROSSINGS` prints what was
   found, and `RERAIL_CROSSINGS=FILE` and `RERAIL_NODE_CROSSINGS=FILE` append the names. A prepared program read back
   from the store keeps no crossings: it was checked when it was prepared.
3. **Lists that only shrink.** `tools::pack_split_tests` runs every pack and the data layer's policies as
   `run-policies` does, over a file of each language with the `.scope` paths dropped, in a store of the test's own.
   It compares what it finds with three lists in `native/src/code_rule_dsl/`:
   - `split_crossings.txt` (0652);
   - `node_crossings.txt` (0677);
   - `policies_not_run.txt`, the policies that fail on the test's files for an error of their own, whose programs go
     unchecked.

   A relation new to a list fails its pack's test. A relation fixed must leave its list in the same commit.
4. **Then the fixes, by the branches' playbook.** Fix the root crossing, not its followers. A gap in the split is
   fixed in the split. An accidental join is rewritten per document. A meant crossing with no node is declared
   `cross_file`. A meant crossing that carries a node is rewritten to symbols. Once a list is empty, its check becomes
   a refusal by default.

## Measured on main (d54013c841)

| List | Entries | Notes |
|---|---|---|
| `split_crossings.txt` | 129 | data layer 79 (39 batched, 40 alone), Python 14, TypeScript 11, JavaScript 7, python-main 6, Django and Vue 4 each, React, SQL, PHP and Rust 1 each |
| `node_crossings.txt` | 3,524 | python-main 2,473, Python 333, Django 250, data layer 281 (batched and alone), the rest 187; by root, 2,387 follow Python's `resolve_export`, 248 `value_type`, 109 `local_def_chain`, and 114 are roots of their own |
| `policies_not_run.txt` | 25 | React 9, Node 8, JavaScript 4, Next, Python, TypeScript and Vue 1 each |

Complexity, rr and rerail have none.

Found on the way and fixed:
- A `nearest` kind set's guest copy kept its label's `-` (`rr_in_compiled-view_ks0`), which SQLite refuses as a
  table name. The blade pack could not run as one program.
- clippy-redundant-field-names' message wrote `{ x }`, which was read as a column.

## Fixes ported

1. **A value's type is the node's own** (the branch's 0677 fix 1). The JSON translator wrote a value's `type` test (and
   `numeric`, `truthy`, `bitsAny`) as `value(n, x), value_type(x, T)`. `value_type` is keyed by the text, which `"12"`
   and `12` share: a false finding, and a join of every file's types. It now writes `value_type_of(n, T)`; a value
   followed through a binding (`static_value`) keeps `value_type`. python-main is regenerated (41 files).
   - Node list, python-main: 2,473 → 2,303. None is left under `value_type`. 7 are new, all under `resolve_export`: their
     node now enters through a relation that is per-file.
   - Split list, python-main: SIM113 is added. It was cross-file before, as a follower of `value_type`. Its own
     crossing is a demand of an aggregate that drops the aggregate's `@doc(c) = @doc(N)` tie (`rr_demand_agg`). The
     branch's split keeps that tie (`row_documents`, `same_document_ties`); that is one of the split gaps under Open.
   - Test: `json_rules_value_type_is_the_nodes_own`: over `a = "12"` / `b = 12`, the int alone; the old join, both.
2. **0649's nullable columns and computed views, and 0652's split gaps** (the branch's 0649 steps 2–3 and 0652's
   split commit). The version split places:
   - a functional native (`match_all`) with its input's row, and other natives by their rules;
   - `@doc(a) = @doc(b)` and `d = @doc(n)` as one document (`Lit::Same`), and a `sub_tape` splice with its tape;
   - a negated read of a nullable location column with its row's anchor;
   - a view computed by comparisons inlined where its domain is read (`simplify::computed`).

   The lowering ties nothing by a test (`!=`, `<`) in a context, its span binder reads `field` as a child, and min/max
   over nodes reads its node back from the ordering helper (`front::order_nodes`). Preparation refuses a head
   variable no body literal binds (`head_bound`). The repository makes `tsconfig`, `tsconfig_for` and
   `tsconfig_path`, and a library input no producer binds is bound empty. `.rdl` reads `cross_file`.
   Main's debug check that the tree's split and passes match the version's goes (decision 0276: the version is the
   only path), as the branch's did: the version's split now has fixes the tree's lacks.
   - Split list: 130 → 38 (data layer 32, Django 2, Python 2, JavaScript and python-main 1 each).
   - Node list: 3,354 → 3,025. Python's ruff_qual root no longer crosses; 7 relations are new to the list, whose node
     now enters directly where it followed that root (`air301_callee`, `air_needs_guard`, `fast_dep_name`,
     `fast_pydantic`, `in_collection_call`, `violation_at`; the data layer's `ca_conditional`). The branch rewrote
     these rules in its later fixes.
   - Policies not run: 25 → 0. React's and Node's failures were the missing `tsconfig` rows and unbound library
     inputs. `spec_resolved` and `npm_ignored` were bound empty, so n-no-missing-import and n-no-missing-require
     found every import missing. Fixes 10 and 11 give them producers.
3. **Covered views and each relation's one-document rows** (the branch's 0652 follow-up). A view of values that every
   reader covers is file-local (`covers`): each of its rules finds its rows in the reader's own. The split lists the
   relations whose rows are each one document's (`row_documents`), and a maintained split asks a covered view again
   when a reader that placed it changes (`cover_deps`). The maintained split's cone now takes in the views of values
   its rules read, as the whole split reads them. The JSON translator declares a helper that reads a guest capture
   by text alone `cross_file`, and a `$$$X` capture binds nothing.
   - Not ported, as main lacks what they extend: the reader-demand ties (oscar's `product_demand`), the `op`
     vocabulary (`op` is not a view on main), `Given::of`'s qualifiers, check-rr's `.from`.
   - Rule texts stay main's. The branch put rules back "as written" (TRY301, UP008, no-useless-constructor ×2,
     no-misused-new, parameter-properties, Vue's no-duplicate-attributes and valid-v-for, python/modules, Django's
     bnl_number). Those rely on the reader-demand pass to stay per document. On main they cross, while main's
     rewrites are per document.
   - Split list: 38 → 34. SIM113 is gone; Vue's `vprop_name` is new (a join through a guest's `xd_` copy).
   - Node list: 3,025 → 3,121. 123 relations of React (55), TypeScript (32), Node (20) and Vue (16) are new. They are
     not new crossings: main's maintained split had kept them file-local where a whole split does not. `fn_component`
     reads `cap_name(x)` by value, which places no document, and the old cone left `cap_name` out. The branch's
     rewrites (`fn_component` inlines the test) come with its later fixes.
4. **Rule rewrites that need no engine change** (the branch's 0665 `cap_name`, 0677 fix 3g, tails 1 and 2):
   - React's `cap_name` is a test of the name's own text where `fn_component` reads it;
   - a `/* global */` comment's items are split in its own file;
   - the TSX libraries' text helpers and the Python libraries' value tables are keyed by node.
   - Node list: 3,121 → 3,083 (JavaScript 45 → 31, Python 189 → 171, TypeScript 32 → 26). 8 relations are new,
     each now entering directly under a root that is still open: Python's `resolve_export` (6), TypeScript's
     `package_for` and `no_redeclare`. React's 55 now root at module resolution (`symbol`, `jsx_source_runtime`,
     `package_for`, `resolve_base`, `tsconfig_for`), as Node's do at `fixed_ext`: the branch's 0665 settings and its
     `tsx_names` phase.
5. **A file's path places its document; the project's settings are read per file** (the branch's 0665 core).
   - `doc`'s path is a path key. The split places by locations and path keys (`FileLocal::path_keys`, `placing`),
     and publishes locations alone.
   - `locate` finds a path's document (`Doc::ByPath`). It relays a path-keyed relation that per-file rules also read,
     and qualifies the cross part's read-back `READ_LOCATED`, which holds it cross-file.
   - The settings (`package*`, `tsconfig*`, `env`) are placeless to the split, in the version's split and in main's
     tree split alike.
   - A file's memo entry is kept under the settings rows its relations can read (`memo::settings_scopes`,
     `SettingsDigests`): its own package's, or every file's.
   - Found on the way: a relation relayed in one `Locate` round was left by the next. A batch runs a second round
     when another relation is left, so bounded-composed-graph-paths' `bcgp_block` was moved cross-file in the data
     layer's batch, not alone. A relayed relation is now located.
   - The branch's `pair_path` case in the explainer's test is per-file now: one path is one document.
   - The JSON translator change of fix 3 stays (declaring a helper that reads a guest capture by text, and `$$$X`
     binding nothing). Its four regenerated python-main rules replace hand edits that the translator did not
     write, so `json_rules_pack_is_current` had failed since fix 2.
   - Not ported: 0660's shared configuration reader (main's repository reads `tsconfig` with its own), the deferred
     joins' constants (oscar's `product_demand`), parts' path-key words.
   - Lists: split 34 → 29, node 3,083 → 3,031 (JavaScript 31 → 8, TypeScript 26 → 14, Node 20 → 11, React 55 → 47;
     python-main +4, fast003's helpers, now declared and holding a node).
6. **Module resolution is the phase `tsx_names`; the split places built names, keyed families and scopes** (the
   branch's 94e9aafd78, 6be2b4b88f's split, 1a9517d64a (0676), and 1d2efb9bd2's three engine fixes).
   - TypeScript's and JavaScript's import resolution runs first as the phase `tsx_names`
     (`typing/programs/tsx_names.rerail`) and hands on symbols alone: `target_at` by the importing file's path,
     `exports`, `module`. `tsx/names.rerail` reads them as settings, so a file's memo entry is keyed by its own
     imports' rows (`SETTINGS_BY_PATH` carries the target column). A phase naming no language runs in its reader's.
   - Found on the way: the phase's own cross-file part read every file's import nodes (`target` held one under
     `resolve_base`). Its resolution now reads each file's statements as rows of a document and symbols
     (`spec_in`, `export_from_in`, `star_export_in`, `own_export`), so no node crosses with it.
   - A variable built from a placed one that leads its string (`rx_group`, `replace_all`, `concat`) is placed by it,
     a key with no location (6be2b4b88f). A scope `nearest` finds is a location, and a keyed family of values read
     where its root holds is per-file (0676).
   - `FileLocal::paths`: the path keys that hold a path as it is. `locate` finds a row's document only through one of
     those; a name built from a path is a key the split places by, but `doc` holds no such name, so a row relayed by
     one was found in no document and dropped. A relay an earlier `locate_all` round made is read by the cross part's
     readers a later round moved (the branch's fix of what fix 5 found). The preparation's locate loop is
     `file_local::locate_all`.
   - Rule texts (not the data layer): React's file gate joins its JSX library table in the file's own rule
     (`jsx_source_runtime` read every file's `@jsxImportSource`), and Vue's namespace step is keyed by the element
     (`vns_step` read every file's element names).
   - `gate/executor-coverage/packs.tsv`: `@glob_sub`, `@package_of`, `@qname` and `@join` now count under the
     engine's phase program. Written from the static census alone, with no executor run.
   - Lists: split 29 → 32 (Vue 1 → 0; the data layer +2, alone and batched), node 3,031 → 2,939 (React 47 → 2, Vue 16 → 4, TypeScript 14 → 8, JavaScript 8 → 7, the
     data layer 103 → 89 and 104 → 90).
   - Added, each a consequence named here, not a new rule:
     - The data layer's `er_edge` and `er_skip` (eager-resolve). The paths fix moves them cross-file, where before
       their rows were relayed by a built name and dropped. The branch's matching rewrite of `eager-resolve.rerail`
       is data-layer text, not ported.
     - React's `sandbox_token`, under `sandbox_value`'s `match_all` table (`rr_ctx`/`rr_mall`, a lowering's value
       view, not covered here). It was hidden while `iframe` crossed under `jsx_source_runtime`.
   - Not ported, as not split work: the branch's run options decided before the run (9dccfd0b30, on constant-folding
     passes main lacks), the configuration files' kept rows (12b6ba7e77, 4e11f91949), embedders' places read in place
     (362ad19b9f), and the kept-facts staging lock in fc57da4d1a.
7. **Python's names are the phase `py_names`** (the branch's fc57da4d1a, 0677 fix 2).
   - The phase (`typing/programs/py_names.rerail`) resolves modules across the workspace from their definitions and
     import statements, and hands on symbols alone: `reads_at(p, m)` (the modules the file at `p` can name),
     `exports`, `class`, `defined`, `public`. They are read as settings, keyed by the reading file's path
     (`SETTINGS_BY_PATH`) and by module (`SETTINGS_BY_FOLDER`).
   - `python/names.rerail` is now `python/scopes.rerail` (who binds a name, file by file) and a short
     `python/names.rerail` that adds `sym_of`'s re-export and class-member rules and `site_exported` through the
     phase. The phase includes `scopes`, so it never reads its own exports. Main's own `names.rerail` was split this
     way, not the branch's copy, which holds other branches' work.
   - `pyupgrade`'s `sym_at` and Django's practices read the phase too.
   - The split: a rule that never holds (a false comparison of constants, `unset`'s empty value in a placing column)
     places nothing (`never_holds`).
   - Tests: the phase's names are the old cross-file rules' answers (`python_names_through_the_phase_are_the_cross_file_rules_answers`);
     a Python file's memo entry is kept under its own names' resolution.
   - Not ported: the store's naming of a hit's tapes in a warm run (`Parses::name_kept`, on the branch's value store,
     which main lacks), with the two warm-run tests that check it; `GateRules`' late-bound keys (main's gate has no
     binding analysis, and no test needs it).
   - Lists: split 32 → 35, node 2,939 → 456 (python-main 2,300 → 7, Python 171 → 40, Django 247 → 206, the data layer
     89 → 80 and 90 → 81).
   - Added, as on the branch after this commit: holders the `sym_of` crossing hid, now under their own roots. In
     Django, `bwh_in_hook`, `dar_non_atomic_wrapped` and `tam_task_call` (split), and `fdm_drf`, `fdm_drf_q`,
     `ghw_drf_q`, `ghw_model_ref`, `rmw_class`, `s_dj_deferring_task_class` (node). Python's `D412`, the data
     layer's `pub_member` and `true_lazy`. The branch's fix 3 commits fix the Django ones.
8. **Class ancestry and ancestors' members by symbol, through `py_names`** (the branch's cbf1a4bdf4, 0677 fix 3a).
   - The phase hands on each class's ancestors by symbol, a linear closure keyed by the class's module, and each
     ancestor's declarations by symbol. `python/names`' `class_anc` and `class_anc_call` read them by the reading
     file's own rows.
   - Django's library (`packs/django/rules/lib/django.rerail` on main) reads them. A model is a class with a known base
     among its ancestors, and `dj_has_field(c, x, t)` and `dj_attname(c, a, x)` name fields by symbol, not by a call
     node in another file. `dj_field_kw` carries a field's keyword arguments. Its readers follow.
   - `bulk-not-loop` keeps main's split form, with only the field's symbol cross-file. The branch's `bnl_number`
     held the step's node cross-file.
   - `factorize`: a relation whose factors meet at a node key, and whose head holds no node, is handed on whole: its
     parts would carry the node into the cross part.
   - Tests: the class ancestry through the phase is the cross-file closures' answer; class members reach each
     subclass; a Python file's entry is kept under its own classes' ancestry. The Django pack's 27 rule tests pass
     (`test-rr`).
   - Not ported: the store's two sweep fixes in the same commit (`connection.rs`, `kept_facts.rs`).
   - Lists: split 35 → 40, node 456 → 418 (Django 206 → 179, Python 40 → 33). Added, as on the branch after this
     commit: Django's `bwh_method`, `dj_instance_value`, `ghw_model_instance`, `ghw_model_ref`,
     `sora_model_instance` (split), and `dj_fetch_call`, `dj_instance_class`, `dj_instance_value`, `dj_qs_model`
     (node), which pair two files' class nodes. Fix 3b keys them by symbol.
9. **Django's models, Meta options, managers, hooks and receivers by symbol** (the branch's ff0924b589, a2cd36b350
   and 91ce1bf9c1: 0677 fixes 3b, 3c and 3d).
   - The phase `py_names` gains a Django section. It turns each Meta class's body into symbols and joins Meta
     inheritance by symbol (the model's own, an abstract base's, a concrete base's). It also hands on managers,
     `save()`/`delete()` overrides and signal receivers, all under the model's module.
   - Django's library reads these at a node of the reading file. `dj_model_base` and `dj_test_doc` move to
     `python/django-shared.rerail`, one list for the library and the phase.
   - Python's syntax-only helpers (`unparen`, `call_arg`, `body_stmt`, `docstring`, ...) move to
     `python/syntax.rerail`, which `python/scopes` includes, so the phase reads them.
   - The touched rules follow: `bulk-write-skips-hooks`, `fk-id-not-relation`, `get-or-create-needs-unique`,
     `read-modify-write`, `select-related-in-loop`, `task-arg-model-instance`.
   - The one data-layer edit: `swallowed-exceptions`' own `unparen` is `se_unparen`. The library's `unparen` now
     reaches it through `python/scopes`, and two definitions of one relation do not load.
   - Tests: a Django model, its Meta and managers, and its hooks are each read by symbol in another file. On main the
     library is the pack's (`packs/django/rules/lib`), so the tests' programs include it as the pack's rules do,
     copied beside the program. The Django pack's 27 rule tests pass (`test-rr`).
   - Not ported: demand stored as a sorted set (`program_facts`), the tests that compare with it, and the branch's
     test compile fix. Main has neither the linked program nor that crate.
   - Found on the way, on main:
     - The inliner (`simplify::inline_filters`) numbered its fresh variables across the whole program. Dropping
       an unreached clause renumbered every later rule, so `lazy-architecture`'s program changed when only
       unreached clauses were dropped. The numbers are now per rule, each copy carrying its count, and a number
       whose names the rule already holds is skipped.
     - `python/scopes`' `in_first_iter` went through `first_iter`, now one definition in `scopes`, taken from
       `pyflakes`. It binds the comprehension kinds itself, so it is no filter the inliner folds back. Before this,
       the phase's demand split cut `first_for` off and started from `field(fc, "right", e)`, walking every
       assignment's right side: 20,385 rows on the split test's 32 files, against 1,441 now.
     - `a_demand_split_stays_linear_in_the_modules` measured every split against the references times the
       modules. Since fix 7 the phase does work per identifier (`nameref`, 1,441 rows), linear, which that bound
       took for growth; the test failed from fix 7 on. It now runs 16 and 32 modules, and each part's largest
       split may grow at most 2.5 times, plus a fixed slack. The quadratic split it was written for grows 4 times.
   - Lists: split 40 → 35, node 418 → 394 (Django 179 → 155). Added: Django's `sora_async_call` and `sora_hit`. The
     rule's text is the branch's: the crossing `sora_query` and `sora_model_instance` named is now named where its
     node enters the cross part. On the branch, the points-to phase (b754ec6c93) moves it.
10. **`npm_ignored` is made** (new on main; the branch binds it empty).
   - The repository makes `npm_ignored(path)` for each file of the tree under a package.json that npm leaves out of
     its nearest package. The reader is `package_manifest`'s (`files` and `main`, else `.npmignore`, else
     `.gitignore`), the one `ImportTarget::published` reads. The facts crate lies below it, so the engine registers it
     at load (`install_npm_publication`); without it, no rows.
   - A setting by path (`SETTINGS_BY_PATH`, keyed by column 0): a file reads its own row, or a target file's.
   - `n-no-unpublished-bin`'s finding states that it lies in its document (`docof(at, d)`). With `npm_ignored` empty,
     the rule had been simplified away; with rows, the split saw two documents, `d` and `at`'s.
   - Tests: the repository asks each file's nearest package (a stand-in publication, in the facts crate), and a
     program reads what npm leaves out (`files`, `.npmignore`, no package), with the real reader.
11. **`spec_resolved` comes from the phase `node_resolve`** (new on main; the branch binds it empty).
   - Core has a fourth file-system request, `module_target(k, f)`: `k` is the importing file's path, the specifier
     and its style (`require`, `import`, `type`), joined by `\n`. `rr::WORLD` names it with `rr_module_req`, so it is
     asked as `dir_entry` is: by a rule that binds `k`. (Not `module_file`: P1, `struct.rerail`, includes core and
     derives a relation of that name.)
   - The engine answers it with `import_target`, eslint-plugin-n's resolution at default options
     (`import_target::resolver`): resolved where the target has no error, as the plugin reads `resolveError`. The
     answer is workspace-relative, or absolute outside the root. Corpus lies below the ecma crate, so the engine
     registers the resolver at load (`install_module_resolver`); without it, nothing resolves.
   - `World` keeps each answer. A cross memo checks it by resolving again (the kind `module`), and the cone's rounds
     never keep a module request as unchanged by the journal: its key names no path, and a resolution reads files
     it does not name (`package.json`, `tsconfig.json`, `node_modules`).
   - The phase (`typing/programs/node_resolve.rerail`) states, per file and by symbol, each string a specifier may
     be: a call's first constant string argument as a `require`, an import or export source as an `import` or
     `type`, and an `import()`'s argument as an `import`, each cut at its `!`. That is a superset of
     `node_target`'s, which follows the global require with the policy's tables (`node_global`, `node_module`), and a
     phase has none of them. It exports `file_at(p, x, st, f)`. `node-resolve.rerail` derives `spec_resolved` from
     `node_resolve__file_at` at the node's own path, which is a setting by path (`SETTINGS_BY_PATH`, column 0).
   - Found on the way, on main: the cone of requests bound the per-file relations it reads only as the answers of
     a load. A cone that asks only the file system and keys its requests by a file's own rows (here `ask_at`)
     loads nothing. Its reads were bound to nothing, and planning failed for want of their statistics. The cone now
     binds every export it reads as an input as well. A cone that loads binds the same relations as before.
   - Tests: a workspace with a relative file, a missing one, a package in `node_modules` (its `main`), a builtin
     and a dynamic import. Every specifier but the missing one resolves, and the missing one is `spec_failed` until
     its file is added, on the next run. The Node pack's lists are unchanged: its 11 node entries come from
     `node_module` and `node_global`, not from resolution.
12. **Python's points-to is the phase `points_to`** (b754ec6c93, the branch's 0677 fix 3e).
   - Main's P3 (`calls_p3.rerail`, 1,625 lines, calls_cx's lineage) joined nodes of different files. It is replaced by
     the branch's split: per-file rules (`python/points-to-file`), and the phase `points_to` over terms and symbols
     (`python/points-to-solve`, `typing/programs/points_to.rerail`). `calls_p3` keeps only what it hands on. The four
     Django call-graph policies take the branch's symbol-only walk. `corpus/bridge.rs` goes with the old P3.
   - Main has neither t32 stage 8 nor 0597's annotation context (`python/annotation`), which the branch's rules read
     a quoted annotation's code through. Hiding (decision 0058) lets a guest's nodes reach a relation only along a
     variable a guest relation binds, so solve's lifts saw no guest. `points-to-file` states the guests' nodes
     (`pt_gnode`), and `points-to-solve` lifts their names from there.
   - Forced edit in the data layer: `python/calls`' call graph is now by symbol, so `leg-083-typed.rerail` walks it
     by symbol (`eager(q)`, a function's file as `def_doc`) and reports the function whose term is `q`.
   - Found on the way, on main:
     - The constants' run (decision 0094 §6) required a run's settings (`env`) to have rows before it folded what
       reads them. A run with no settings adds none (`added::Added`), so `py_version` did not fold, and per-file
       `pt_` relations went to the cross part "under py_version" (data layer +68, Django +73). `env` is now given
       whole, rows or none, as decision 0665 §5 states.
     - With it folded, phase 1's cone became load-only, and two latent gaps showed in the warm tests
       (`the_run_after_a_cold_one_writes_nothing`):
       - The cone-rounds record was asked inside the waves, where a store's records are not read (decision 0104),
         so a warm run never found it and wrote it again. It is now asked before the first wave, and a record whose
         journal named no change, with the same roots and world, is not written again.
       - A plan taken on budget (decision 0122) reads no distributions (decision 0123), yet remembered the sizes it
         read: a lazily bound relation's distinct values as one. The next run, priced with them measured, kept the
         plan again. A plan taken on budget now remembers its rows, and the distributions it was last priced at.
     - Fix 11's resolver (`world::MODULE_RESOLVER`) is listed with 0104's state (`facts_only`).
   - Lists: split 35 → 33, node 394 → 273 (Django 155 → 34). Nothing added. The executors' coverage
     (`gate/executor-coverage/packs.tsv`) moves rules from `engine` to `lib` with the library.
   - Tests: Django's 27 rule tests; the typing oracle tests; `quoted_annotations_are_read_as_their_code`;
     `leg_083_typed_counts_resolve_by_the_receivers_type`; every pack's split test.
13. **The Python causes keyed by node, and two engine-made crossings** (the branch's 6d59c0b734, 0677 tail 3).
   - Before this, the branch's fixes 3f, 3g and tails 1 and 2 were checked against main again: fixes 4 and 6 ported
     them. 3f's third fix (an id range's bound stays with a pushed join) changes a step main lacks: main's push pass
     never hands a bound to the reader (0575's narrow copies), so its bounds stay with the join already.
   - Rules: the branch's 56 Python rule and library files. Each table that was keyed by a value is keyed by the
     node the value came from (format strings, names, Airflow's calls, `sort_key`, `zero_number`, ...).
   - A module's privacy (D1xx's `doc_public`) walked `file` for the package's folders, which reads every file. The
     branch reads an ingested `unpackaged(d)`, which main lacks. Here the repository makes `py_unpackaged(path)`,
     a Python file of the root or of a folder with no `__init__.py`, as it makes `npm_ignored` (fix 10). It is a setting
     by path (`SETTINGS_BY_PATH`), so a file reads its own row.
   - PERF401's `copy_only` joined a loop with any call's argument, tied only by PERF401's reads of it. On the branch
     the reader-demand pass ties them; main lacks that pass (fix 3), so the rule states it: the argument is a call's
     in the loop's body, as PERF401 reads it.
   - The engine, in main's own form (main's magic sets and lowering are over `front::Program`):
     - A magic rule (`magic::demand_bound_columns`) leaves out a literal the left-out literals had tied to its key and
       that now joins it by a value alone. Its places are its `node`, `loc` and `span` columns and the document
       columns, anywhere in the rule. RUF034's `same_pair(x, y)` was left out (it would close a negative cycle), so
       `magic_unparen_bf(c) :- same_child(x, i, c), same_child(y, i, d)` joined y's file to c's by the index i.
     - `match_all` over a text: its context also holds the nearest node its literals reach (`strip_arg(c, v)`'s c), so
       it and the matches are derived in each file. The native reads the texts, column 0, once each, and a printed
       program reads back as itself.
     - Not ported: the deferred joins' fix (a lookup in a constant driven as a product). It is oscar's
       `product_demand`, which main lacks.
   - Tests: `a_demand_joins_no_other_documents_nodes_by_a_value` (it fails without the fix),
     `a_match_all_over_a_derived_text_is_placed_by_its_node`, `a_python_file_outside_every_regular_package_is_unpackaged`;
     every pack's split test; the rule files' own tests and `check_rr`; the rr, magic, file-local and plan tests.
   - Lists: split 33 → 31, node 273 → 239 (Python 33 → 0, React 2 → 1: `sandbox_token`, as on the branch). The
     executors' coverage loses the old privacy rule's `@replace_all` and `@contains`.
14. **The other packs' value tables keyed by node, and three engine fixes** (the branch's 6c5dda1725, 0677 tail 4).
   - Rules: the branch's 32 rule and library files of JavaScript, TypeScript, Vue, PHP, Rust, SQL, React and Next.
     Main's Vue has no contexts (0597): the branch's `async_computed_in` is main's `async_computed`, and main's
     `vns_step` is keyed by the element already (fix 6). Vue's `vplain_stmt(k)` was a table of every file's kinds,
     read at a statement. On the branch the script's context holds it. Here it is keyed by the statement.
   - The engine, in main's form:
     - `static_value_type_of(n, t)`: the followed value's type (`static_value`'s), per node. no-implied-eval read
       `static_value(a, v), value_type(v, "string")`, and `value_type` is every file's (value, type) pairs. W gives it
       from the followed reading, and the literal path from the literal's type.
     - Factorizing the exports (0099) leaves a join of a file's nodes with a constant whole: a lookup made in the
       file. On main the split's check found the constant's part not file-local and left it whole already, in the
       branch's test, so the check states up front what was its effect.
     - Hiding (0058 §2) names a kind compared over a Blade view (`compiled-view`) in PHP, the one dialect the
       embedding gives (`language::embedding_dialect`), and qualifies a kind only in a grammar a dialect's label names.
       Before, `compiled-view:comment` was no dialect's, so each round qualified it again and hiding never ended
       (PHPDoc's new stop set over a Blade view).
     - Not ported: the branch's located output's unplaced check, which is not in the batch's code, and tail 5's
       symbol-demand fix, which is in `product_demand` (main lacks it).
   - Tests: `a_kind_compared_over_a_blade_view_is_phps_and_ends` (it does not end without the fix),
     `an_export_that_looks_a_files_nodes_up_in_a_constant_is_left_whole`; the hiding, file-local and value-type tests;
     every pack's split test; the rule files' own tests and `check_rr`; the executors' coverage (unchanged).
   - Lists: split 31 → 30, node 239 → 210. JavaScript 7 → 1 (`significant_bits`, under the engine's
     `radix__key_slice0`, as on the branch), TypeScript 8, PHP 4, Vue 4, Rust 2, SQL 2, Blade 2 and React 1 → 0.
15. **Django by the phase `py_names`, and one engine fix** (the branch's a0e95c0b6a, 0677 tail 6, and bulk-not-loop
   from 516477cb21, tail 5).
   - Rules: the branch's Django walks of the run's classes and defs by `class_def` and `qualname` in any file now go
     through the phase: each file turns its defs, decorators and calls into symbols, the phase joins them by symbol,
     and hands them on under a module (`plain_def`, `dj_task_def`, `dj_deferred_task`, `dj_app_defers`,
     `dj_view_model`, `dj_anc_clean_call`, listed in `SETTINGS_BY_FOLDER`). bulk-not-loop's number field is a test of
     the document, over the rows the phase hands on for the modules it reads. The queryset helpers moved to
     python/django-shared, which now includes python/scopes for what they read (`attr_of`, `unparen`).
   - The engine: a magic rule also leaves out a literal that the key's binders need only through a value, where the
     rule ties it to the key through the demanded atom or a literal after it (default-called-once's
     `magic_unparen_bf(v)`, answered through a coarser demand), unless it reads a key variable, and only while the
     kept positive literals still bind the key. In main's form, each magic rule carries its source rule's per-literal
     places. T's golden `magic_home_ffb` changes as on the branch.
   - Not ported: tail 5's symbol-demand fix (`product_demand`, which main lacks); its data-layer rules (out of scope).
   - Tests: `a_coarser_demand_keeps_no_literal_only_the_demanded_atom_tied`, `t_programs_transform_like_t`; Django's
     27 rule tests; the typing tests; every pack's split test; the rule files' own tests and `check_rr`.
   - Lists: split 30 → 28, node 210 → 177 (Django 34 → 1, `dar_non_atomic_wrapped`, as on the branch).
16. **Node: an `extensible` table nothing states is stated empty, and the specifier tables keyed by the target**
   (the branch's 9ad7172f4c, 0677 tail 7).
   - Every Node entry hung off `node_module` or `node_global`, which read node-trace's `module_global(g, m, x)`, an
     `extensible` table whose rows the policies naming globals state. A program naming none states no row, and in a
     batch each stating policy's rows go to its own copy (`q<k>_module_global`). Never stated, it was a base relation
     of no location to the split, so every reader of a value through it went to the cross part.
   - The engine, in main's form: the lowering keeps `extensible` as a qualifier word of a library relation (not core's:
     `env`, `option`, `load` are the engine's), and a printed program reads it back. Where the whole program is known,
     at both ends of `combine_read`, each one that nothing states, that is no input and that has only value columns,
     is stated empty: a run constant (decision 0263: its rows come only from the files that include its own). The
     split does not decide it, since a library module planned alone (0639) does not see its includer's rows.
   - Rules: the branch's node-resolve and Node pack files. `spec_kind`, `spec_package` and `spec_allowed` are keyed
     by the target node; checkPublish is split into a file's targets under its document and the cross part's answer
     by document (`pkg_unpublished_file`, `pkg_dev_only`); n-handle-callback-err tests the option's name at the
     function. `spec_resolved` stays the phase `node_resolve`'s (fix 11).
   - Not ported: locate leaving a request where it is asked (C's configuration tables; main has no C library), and
     binding a per-file part's own empty inputs (its case, `spec_resolved`, is the phase's answer here).
   - Tests: `a_programs_facts_of_a_library_relation_ask_of_the_library` (the shared `lit_repr_threshold` is stated
     empty); the rr, corpus and file-local tests; every pack's split test; the rule files' own tests and `check_rr`;
     Django's rule tests; the executors' coverage (unchanged). The corpus tests that run `git add -A` are refused by
     this machine's sandbox, as before.
   - Lists: node 177 → 166 (Node 11 → 0). Outside the data layer the lists now hold only what the branch keeps:
     Django's `dar_non_atomic_wrapped` (both lists), JavaScript's `significant_bits` (under the engine's
     `radix__key_slice0`), and python-main's 7 (FAST003).

## Consequences

- A new rule that adds a crossing or a node id fails its pack's test; its author fixes it or declares the relation.
- The pack tests take about 4 minutes of CPU with fresh stores and are in the slow tier.
- The checks cost one inference over the program per split made (`NodeColumns::of`), not per run of a prepared
  program.

## Open

- 0652's follow-up `same_document_ties` is not ported: it is a tie of `product_demand`'s reader demands, which main
  lacks, and SIM113, its case here, has not crossed since fix 3.
- The data layer's entries (not in this pull request's scope, 2026-11-25).
