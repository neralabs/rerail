# 0665. A file's path places its document, and the project's settings are read per file

Status: decided (2026-11-24), by the user's milestone: "settings, and keys that name their own document. It's the
part of the plan that needs no rule rewrites, it fixes the one open refusal, and it's checked by the position report
we already have." It amends the split (0051, 0281, 0652, `typing::file_local`), `locate` (0052, 0290) and the per-file
memo's identities (0225 §2).

## Intro

A program runs in two parts (0051). The per-file part is derived again only for an edited file; the cross-file part
is maintained over all files. The split puts a rule in the per-file part when it can prove the rule's reads are all
about one document. Until now the only proofs were locations: a node, or a document's key.

Two kinds of rule failed that proof though they are about one file:

- **A rule keyed by a file's path.** bounded-composed-graph-paths keys everything by the path `p` it reads off
  `doc(d, p, _)`: `bcgp_line(p, l, sp)`, then `bcgp_lead(p, l)` joining two of them by `p`. A path is not a location,
  so the join looked like two documents meeting, and 0652 refused it (`rr_ctx21 (made by the lowering of bcgp_lead):
  reads bcgp_line with no location`). That was the one open refusal over the 334-file corpus.
- **A rule that reads the project's settings.** React's gate reads the package a file is under (`other_jsx(d) :- …,
  doc(d, p, _), package_for(p, dir), deps_on(dir, "other")`); Python's platform reads `env("python.platform", p)`.
  These inputs have no location, so every reader went cross-file, and with it 10 of React's chains of position reads.

Both are about one file. The change teaches the split why, and keeps the memo sound now that per-file results read
the settings.

## 1. A file's path places its document

`doc`'s path column is a **path key** (`file_local::path_key_column`). A chunk holds its own documents' `doc` rows
alone, and a path names one document, so a variable bound there places the rule's document as the document's key
does. The split's placing columns are a relation's locations and its path keys (`FileLocal::placing`). A join by a
path stays in one file, and a relation filled from a path somewhere is per-file.

A path is still not a location. The memo, the loads and the cone read a row's document off a location's value (a node
`key << 32 | index`, a document key), which a path is not. So the split keeps two fixpoints: the placing columns,
which it places by, and the real locations among them (`real`), which it publishes as `locations`. The rest are
`FileLocal::path_keys`, persisted (tag 14) and handed to the parts as `part:key=`. `row_documents` and `same_doc` keep
real locations only.

A path written out names one document too (`bcgp_line("impl/…/sql_graph_query_builder.py", l, sp)`): the same in
every atom that writes it, a different one per spelling (`Form::paths`). Two paths written out are two documents.

**Settings are not path keys.** `package_for(path, dir)` and `tsconfig_for(path, dir)` have a row per file, but they
are bound whole beside every chunk: a chunk sees every file's rows. A rule `x(q, dir) :- package_for(q, dir)` would be
"one document" by `q` while every chunk derives every file's rows. They are settings (§3). Only `doc`'s rows are the
chunk's own. A folder (`package_field(dir, …)`) is no key either: two files of one package are two documents.

## 2. `locate` finds a path's document

`locate` gives every relation the chunks hand on a located form, `r_at(d, …)`, so each row names its document
(0052). A rule with no document column and no node now finds `d` through a path it binds or writes out, adding
`doc(d, p, …)` (`Doc::ByPath`).

Two cases that used to be refused or moved are now located:

- **A relation keyed by a path that per-file rules also read** (`bcgp_block`, `bcgp_def`, … read by `bcgp_lead`'s
  chunk rules and handed on). `locate` used to block it, and 0290 moved it cross-file, where 0652 refused it ("its
  rows name no document (moved by `locate`)"). Now it is **relayed**: it stays, `r_at(d, …) :- r(…), doc(d, p, …)` is
  derived beside it in the chunks, and the cross-file rules read `r_at(_, …)` (`Item::Relay`). A relay made in an
  earlier round is not made again.
- **The cross part's read-back `r(…) :- r_at(_, …)`.** Its column is a path key, so the split would place it in one
  file, handing `r` on again with no location (phase 0's `p0_file_t_shape`: "no location column" at the load's
  answer). It is cross-file by what it is for: `locate` qualifies it `READ_LOCATED`, and the split holds it
  cross-file. That is neither a declaration nor a move to 0652.

## 3. The settings are read per file

The settings are the inputs the repository reads from configuration files alone (`file_local::setting_input`):
`package`, `package_field`, `package_export`, `package_for`, `tsconfig`, `tsconfig_for`, `tsconfig_path`, and the
run's `env`. A source file's edit changes none of them. To the split they are placeless, as the run's options are:
a per-file rule that joins them stays per-file, and a relation derived from them alone is a constant
(`pkg_dep(dir, m) :- package_field(dir, "dependencies/react", m)`). `file` (the tree's paths) is not a setting: a
per-file rule reading it is cross-file still ("reads file with no location").

**The memo's key.** A file's memo entry is kept under each relation's identity (0225 §2). Now that a per-file rule
reads the settings, the identity must change when the settings it reads do. Otherwise an edit of `package.json` would
leave stale results. It must not change for the settings it cannot read, or one package's edit would rerun every
file.

`memo::settings_scopes` reads the file-local program and says, for each memoised relation, which settings its rows
can read:

- **its own file's** (`SettingsScope::OwnFile`): every read of a setting keyed by a path is by the rule's own path
  (`doc(d, p, _), package_for(p, dir)`), and every read of one keyed by a folder is by a folder its own path's row
  names, directly or through a relation keyed by that folder (`deps_on(dir, r) :- package_dep(dir, _, m), …`);
- **every file's** (`SettingsScope::All`): any other key (a constant folder, a wildcard path, a key its head drops),
  or a memoised relation still keyed by one.

`memo::SettingsDigests` digests the rows the run binds: all of them, and per file its own (its `package_for` and
`tsconfig_for` rows, and the rows of the folders those name). A file's identities are `memo_ids`, each relation that
reads the settings hashed with its scope's digest, made once per digest of a file's own settings (`ids_of` in
`corpus::run`). The lookup (`missing_of`) and the write (`memo::cover`) both use them. The memo's space and the kept
statistics keep the global identities: neither depends on a file's rows.

`Report::settings` is the digest of every settings row the run bound, for a program whose file-local part reads them.

## 4. A constant's join is not driven by the files' requests

With the settings placeless, React's gate was still cross-file. The planner had deferred `deps_on`'s join (0488):
`deps_on(dir, r) :- package_dep(dir, _, m), dep_package(m, r)` was cut to the folders some file asks for,
`rr_deferred_pairs_deps_on_at(d, dir, "react") :- package_for(p, dir), doc(d, p, _)`. So a constant of settings and
facts depended on every file's rows, was cross-file, and took `exists_0`, `other_jsx` and `react_doc` with it.

A constant is the same for every file and made once; a demand cut buys it nothing. So `product_demand::deferred`
leaves as written a join the split holds a constant when a non-constant rule reads it (`Plans::constants`). A
constant that only constants read is driven as any join is: its requests are no file's.

## 5. The run's options and settings are decided before the run

The settings phase is the run's constants phase (0094 §6): relations made from no document are derived first, by a
run over no documents with the file system's requests answered, and given to the program as facts. It is kept by
what it consulted, so a source file's edit derives no constant again. Two gaps kept the settings out of it:

- **`env` with no rows was not given.** The run states every `env` row (decision 0249, `Added`), but `foldable_of`
  took a relation as given only when it had facts, and `add_facts` states none for no rows. A run with no
  `rerail.env` (the position report's) left `py_version` unfolded: its `file_line` read of `pyvenv.cfg` is a request,
  answered in the cross-file part, and Python's platform chain went cross-file with it. Now a relation the run states
  whole (`file_local::run_stated`: `env`, `option`, `option_item`) is given, none or some.
- **The options were inputs bound at run time**, so a test of them was never decided. pydocstyle's
  `doc_ignored(f) :- …, option_item("pydocstyle.ignore-decorators", _, q), sym_of(c, q)` reads `sym_of`, which
  resolves exports across files: every docstring rule (`doc_of`, `doc_at`, `doc_indent`, `doc_nlines`) read it
  through `!doc_ignored(o)` and went cross-file, though a run that lists no decorator ignores nothing. Now the
  constants step states the options as facts (`constants::state_options`): each of `option` and `option_item` whose
  reads name the options they can match, the rows of those options (with the program's declared defaults), and it is
  an input no more. Naming only those keeps the program the same for another batch of files (`inputFiles` is an
  option too). The names are read off the program (`option_names`): a string at a read's name, the facts that bind
  it (`gi_option(o), option_item(o, …)`), or the readers of a head it is carried into (`method_dec_named(e, o) :- …,
  option_item(o, _, q), …`, read as `method_dec_named(e, "pep8-naming.classmethod-decorators")`). A read that can
  match any name (a wildcard, a computed name such as pytest's `option(cat(c, "-require-match-for"), p)`, an
  output's column) keeps its relation an input; the other relation is stated still. A relation stated with no row
  holds an empty fact list (`set_facts`, as an empty folded constant does): otherwise `simplify::BaseInputs` took it
  for a base relation nothing fills and made it an input again.

`DecidedFactTests` then decides them: a relation the run states is whole even with no row, and a test of variables
is decided where no row has its constants (`option_item("pydocstyle.ignore-decorators", _, q)` with no such row
cannot hold). The rule goes, `doc_ignored` is empty, `!doc_ignored(o)` holds and is dropped, and the docstring
rules are per-file. With decorators listed, the rule stays and reads across files, as it must. The prepared program's
key is its version's identity, which holds the stated rows, so other options are another prepared program.

A program whose every file read was decided away (the constants' own run, whose `pyvenv.cfg` read cannot hold with
no `env`) loads nothing, where it used to refuse for want of a request.

## Found on the way

- **A line's text was twinned as a node's.** `text_demand::text_kind_demand` skipped a span's `text(sp, t)` for the
  nodes it reads but still counted it among the reads of a node of any kind, so `Twins` gave the rule a long-text
  twin (`t = @rr_node_text(sp)`). The twin's string is computed during the run, so a test on a copy of the line
  (`selected(d, l, t) :- candidate(d, l, t)`, `contains("inspect", t)`) was tested per row, not from the vocabulary
  batch (`strings::tests::logical_line_predicates_and_aliases_use_the_vocabulary_batch`, red since the merge of
  roll-ups 66 to 80). A span's text is read whole (`span_text_demand`): it gets no twin.
- **A `symbol` column was not a `sym` one** in three of the preparation's checks (`text::vocabulary_columns`, the kind
  sets of `lowering_tail`, `whole`'s helpers): both spellings are read.
- **An output or a request decided empty still has its table.** With no `env` row, `DecidedFactTests` dropped
  every rule of `rr_file_req` (the request for `pyvenv.cfg`) and of a constants' run's copy of `c_cc_has_std`, and
  five of `tests_logic`'s plan tests found `no such table`. A request whose every rule goes keeps them as written
  (they hold no row; the world's answer reads its table). An output whose every rule goes holds an empty fact list,
  and the run's outputs step makes a table still pending before it reads it (`plan.rs`): a relation of neither rules
  nor rows was made only when SQL first read it, and an output's read was not one.
- **A computed view is inlined only when each domain atom shares a variable with its head.** With `commentPattern`
  stated, ESLint's `note(v) :- comment(_, w), v = @replace_all(w, …), match(…, v)` became one rule, so a computed view
  (0649), and its copy in `excused(b)` scanned every document's comments beside the switch's own: a join of two
  documents, refused (0652; `pack_split_tests`, the JavaScript pack). A domain atom keyed by none of the head's
  variables stays in its relation; 0649's views (`ca_split(r, …) :- ca_dotted(_, r), …`) still inline.

## Tests, written first, one per requirement

| Requirement | Test |
|---|---|
| A per-file rule reading a setting stays per-file; one reading another file's rows does not | `file_local::tests::a_rule_reading_a_setting_stays_per_file_and_one_reading_another_files_rows_does_not` |
| A join by a file's own path stays per-file; a path written out is one document, two are two; `locate` finds the document, relays, and keeps the read-back cross-file | `file_local::tests::a_join_by_a_files_own_path_stays_per_file` |
| Which settings a relation reads | `corpus::tests::a_relation_reads_its_own_files_settings_only_through_its_own_path` |
| A source edit leaves the settings as they were and every other file hits; a `package.json` edit misses the files under that package alone; after each edit the rows are a fresh run's | `corpus::tests::a_files_entry_is_kept_under_its_own_packages_settings` |
| A constant's join read by a file's rule is not restricted by its requests | `product_demand::deferred::tests::a_constants_join_is_not_restricted_by_its_readers_requests` |
| An option the run does not set is decided before the run, and `env`'s default folds: the reader is per-file; with the option set it reads across files; rows are each run's | `corpus::tests::an_option_the_run_does_not_set_is_decided_before_the_run` |
| The data layer's refusals are caught as one policy's run makes them | `pack_split_tests::the_data_layer_is_split_with_its_crossings_declared_policy_by_policy` |

**Why the pack test missed the refusal.** `pack_split_tests` runs a directory's policies as one pack
(`RERAIL_BATCH_PACK=strict`). bounded-composed-graph-paths, split as a member of the data layer's pack of 41, was not
refused, and its findings were its solo run's (30 over the mini root, the same rows). Alone it was refused, and alone
is how the 334-file run ran it. The corpus was not the cause: with path keys turned off, the solo run over `a.py`
alone is refused too. The new test runs each data-layer policy alone (`RERAIL_BATCH_PACK=off`). With path keys
turned off it fails on `rr_ctx21 (made by the lowering of bcgp_lead)`; with them it passes. It takes 23 to 38 s on a
farm host, so it is in the slow tier.

## Results

- bounded-composed-graph-paths over the 334-file corpus completes: 30 findings. It was refused before.
- Phase 0's tests, the split's 36 tests, and the 32 pack split tests (the new one included) pass; so do `rerail-logic`
  (301), `tests_logic` (226), `tests_sql` (216), `tests_lang` (166) and `tests_corpus` (235 of 236, below).
- The position report (every data-layer policy over the 334-file corpus and every pack over a small corpus; one
  host, every run completed): 75 flagged rules before, 62 after. No rule is newly flagged. The 13 gone:
  - React's gate (`early_read`, `render_early`, `p19_holds_read`, `p35_rr_agg1`, `p7_rr_agg1`, `rr_agg43` and their
    demands): React's 12 become 4, `p36`'s chain through `cap_name`, which was there before and reads no setting;
  - the docstring options' chains (`doc_at`, `doc_indent`, `doc_nlines`): with no `ignore-decorators` option,
    `doc_ignored` is decided before the run (§5);
  - Python's platform and version (`pt_shadowed`, and leg-083's one finding chain): `py_version` folds.
  No chain roots at `package_for`, `py_platform`, `py_version` or an option any more. Where a root cause is now named one step further along (a
  read-back of a located form, two documents joined), it was "reads R with no location" before.

- **The data layer's reference graph declares its two crossings.** `rf_own_fn` (a scope's function, defined in any
  file) and `rf_imported` (a module's from-imports, by its name) are cross-file by meaning; since 0652 the split
  refused them undeclared, and `reference_stage_tests`' three tests were red. Declared in
  `lib/python/references.rerail` and in the staged overlays that inline it (`publish/`).
- **`def_unit`'s lookup by unit missed the roots.** A document root's row holds `-1` (it has no unit, `d5920f3a86`),
  and a lookup of `u = -1` found no node: no rows, where the scan has every root's
  (`native_shards_match_the_original_batch_and_both_probe_directions`). It is each document's root now, as `tape`'s
  parent `-1` is. `tests_sql`'s memo test declares its `dup` cross-file (0652).

- **React's `cap_name` joined documents by accident.** `cap_name(x)` was the names of every file's identifiers and
  property names that pass `isFirstLetterCapitalized`; `fn_component` read it for its own name's text, so a file's
  components depended on every other file's names. ESLint's test is of the string alone. It is written where it is
  read (`!match("_*[a-z].*", x)`, `lib/tsx/react-class.rerail`). The split could not cover it (0652's `covers`): a
  reader's name is not always a node of the view's kinds (a member's `property` may be a private name). Checked
  against the original over React's seven label fixtures: the same findings (72 rows). React's four chains through
  `p36` now root at `sym_of` (`wrapper_call`'s `react#memo`, `create_element`), the import resolution.

- **The root's settings are read one way.** The run decided whether to read `rerail.env` (not over a Git revision
  whose outputs read metadata alone and no `env`: the commit's tree may be absent), but its summary and its reuse key
  read it always, and failed on a commit without its tree (`revision_cli_reads_commit_metadata_without_its_tree`).
  `base::root_settings` decides once for the three.
- **The converter's tests read the staged overlays.** Two reference tests read the pulled dead-code overlay, which
  still reads `tape_line` (removed by 0612, whose fixed overlay is staged in `publish/` for the next archive). The
  tests' overlay directory is each file's staged copy where the tree stages one, else the pulled one (links).

- **The configuration files' rows are kept.** A run read every `package.json` and project configuration (and the
  chain each `extends`) again. Their rows (`package`, `package_field`, `tsconfig`, `tsconfig_path`) are now kept in
  the store under the root and the configuration files' paths, with each question their reading asked of the disk
  (a file's digest, whether a path is a file or a folder, a real path) and its answer. A later run asks them again
  through the observer, which answers a digest from the file's kept stat without reading it, and reuses the rows when
  every answer is the same; otherwise it reads them and keeps them anew. A base that a chain looked for and that
  appears later is such a question. An unchanged run, or one after a source edit, reads no configuration file
  (`kept_settings_are_not_read_again_while_their_files_are_unchanged`).

- **An embedder's program's places are read in place.** `cpu_and_metal_read_the_same_tape_values` failed on Linux
  hosts (and at this branch's base) with a stratum past its launch budget (0603): 6 `join_run`s and 2 `view_expand`s.
  The two were the `line` and `docof` views of an HTML file's script, made into device relations (0353 step 4's copy)
  because the views' directory refuses the `Loc` views of a guest placed by an embedder's segments
  (`views.refused.locs.guest_segments`). The views' reads now read that placement (`PLACED_SEGMENTS`): the segments
  are written once in the batch's block beside the entry (`ViewDoc::segments`), and the read in CLIF finds the
  segment holding a span's start, takes each segment's piece, and grows the run of pieces holding the first until no
  piece grows it (`SegmentMap::file_ranges(..).primary()`). The test now also pins the calls' host lines (3 and 5);
  it passes on Linux and on Metal.

## Left open

- **React's four `p36` chains root at `sym_of`.** `sym_of(f, "react#memo")` resolves an identifier through every
  file's `exports` (re-exports): cross-file work, keyed by names, with no position. A per-file rule that reads such a
  relation of names stays per-file only once the cross part's rows of names are an input of the per-file part, a
  second round of the per-file fixpoint (the two-phase design). The same holds of the docstring rules with
  `ignore-decorators` set.
- **Eager-resolve's `er_last_def` stays cross-file.** Its symbols are built as `cat(module, ".", name)` from
  `er_doc(d, m)`. `cat` is not injective (`a.b` + `c` and `a` + `b.c` are one string), so a symbol does not name one
  document, and the split cannot treat it as one. Making it per-file is a rule rewrite: key the relation by the
  document, not the joined text.
- **With `pydocstyle.ignore-decorators` set**, the docstring rules read `sym_of`'s cross-file resolution, and are
  cross-file: that is the cross-file part's own work (the position report's mechanism A), not a setting.
- **"The settings phase never reruns" holds for its rows, not its reading.** The repository relations are read again
  each run: the tree walk and each `package.json` (cheap, and 0660 reads a project's configuration once per run). A
  source file's edit leaves their digest the same and misses no other file. Skipping the walk itself needs the
  configuration files' digests kept between runs.
