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

## Tests, written first, one per requirement

| Requirement | Test |
|---|---|
| A per-file rule reading a setting stays per-file; one reading another file's rows does not | `file_local::tests::a_rule_reading_a_setting_stays_per_file_and_one_reading_another_files_rows_does_not` |
| A join by a file's own path stays per-file; a path written out is one document, two are two; `locate` finds the document, relays, and keeps the read-back cross-file | `file_local::tests::a_join_by_a_files_own_path_stays_per_file` |
| Which settings a relation reads | `corpus::tests::a_relation_reads_its_own_files_settings_only_through_its_own_path` |
| A source edit leaves the settings as they were and every other file hits; a `package.json` edit misses the files under that package alone; after each edit the rows are a fresh run's | `corpus::tests::a_files_entry_is_kept_under_its_own_packages_settings` |
| A constant's join read by a file's rule is not restricted by its requests | `product_demand::deferred::tests::a_constants_join_is_not_restricted_by_its_readers_requests` |
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
- Phase 0's tests, the split's 36 tests, and the 32 pack split tests (the new one included) pass.
- The position report: see below.

## Left open

- **Eager-resolve's `er_last_def` stays cross-file.** Its symbols are built as `cat(module, ".", name)` from
  `er_doc(d, m)`. `cat` is not injective (`a.b` + `c` and `a` + `b.c` are one string), so a symbol does not name one
  document, and the split cannot treat it as one. Making it per-file is a rule rewrite: key the relation by the
  document, not the joined text.
- **The docstring chains** (pack-python's `doc_at`, `doc_indent`, `doc_nlines`) do not come from the docstring
  options (`option_item` was already placeless). They come from `resolve_export` through `sym_of` and `doc_ignored`,
  a cross-file resolution. That is the cross-file part's own work (the position report's mechanism A), not a setting.
- **Python's platform chain now roots at `py_version`.** `py_platform` is read off `env` and is gone from the
  chain. But `pt_shadowed`'s chain (leg-083, django) now stops at `py_version`, whose second rule reads the venv's
  `pyvenv.cfg` through `file_line`: a file read on request, which the cross-file part answers. To make it a setting,
  the repository must read `pyvenv.cfg` as it reads `package.json`, and `python/env.rerail` must read that.
- **"The settings phase never reruns" holds for its rows, not its reading.** The repository relations are read again
  each run: the tree walk and each `package.json` (cheap, and 0660 reads a project's configuration once per run). A
  source file's edit leaves their digest the same and misses no other file. Skipping the walk itself needs the
  configuration files' digests kept between runs.
- `cpu_and_metal_read_the_same_tape_values` fails on Linux hosts with a stratum past its launch budget (0603). It
  fails the same way on this branch's base (`872f3a300b`), so this change did not cause it.
