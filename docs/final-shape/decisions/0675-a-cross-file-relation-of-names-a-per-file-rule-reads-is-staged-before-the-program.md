# 0675. A cross-file relation of names a per-file rule reads is staged before the program

Status: proposed (2026-11-24), by the user's plan for the rules the position report flagged after 0665: "C: the
general staging, 13 rules. This one helps most beyond the report, because Python's resolution is shared library code
read by many packs." It generalises 0665 §6 (a phase written by hand, read as a setting) and amends 0083 §2.3 (a
phase's exports are placed nowhere) and the split (0051, 0652, `typing::file_local`).

## Intro

A program runs in two parts (0051): the per-file part, derived again only for an edited file, and the cross-file part,
maintained over all files. A rule that reads a relation made from every file's rows is cross-file, and so is every rule
above it. After 0672 the report flags 41 rules whose chains are per-file but for one such read. Thirteen of them read a
relation of names: Python's resolution (`resolve_export` under `sym_of`, `sym_at`: five chains), Django's model names
(`dj_model_q`), points-to's decorators (`pt_lwrap`, read from `types__sig`), and others (`app_semantic`, `er_mentions`,
`pha_private`, `plp_xq`, `vmi_callee`).

Such a relation holds names, no node and no position: what module `m` exports, which class is a model. 0665 §6 made
TypeScript's resolution a phase by hand (`tsx_names`). The phase ran first, to its fixed point, and the program read
its exports as settings, keyed per file by the rows the file reads. This decision makes the engine do it for any such
relation, instead of each library writing a `.phase`.

## Decision

### 1. What is staged

A derived relation `s` is staged (`FileLocal::staging`, on the version) when all of these hold:

- **Its rows name no document.** The split gives it no placing column and no nullable location, and it is not a
  covered view (0652). This is wider than "the split finds it cross-file": a relation of names with no location can be
  file-local (`exported(m, x)`, derived per chunk, read only by cross-file rules), and it still holds every file's rows.
- **Its rows are names.** No column is a node. A relation whose rules, or those below them, read a position (0649's
  `POSITIONS`) is staged only where every column is a symbol: a name found through positions (`qn`, a name's binding
  by where it is in its own file) changes only with its file, as any per-file fact, where a number may be a position,
  which moves with every edit above it (0092 §3). A document's id is placed by the split, so a relation holding one
  is never a candidate. A nullary relation is not staged (a flag over every file is step D's).
- **It is the program's own to move.** It is derived, and not an output, a test, a function, a native, a lattice,
  nor declared or moved `cross_file` or read back from its located form (0665). Neither is a native's input (a demand,
  `rr_ctx…`, which the native reads), nor a relation a rule names as a value (a kind set, `nearest(e, a, fk)`), nor
  one the on-request rewrite adds (0174: a widened copy, `rr_wide_…`, a request), which its rewrite reads by name.
- **Its symbols are names.** A column of another domain (a kind, a field: 0270) is branded by the rules that make
  it, and an export reads back as names.
- **It is not answered by a load.** A request (`load`, `c_include`, ...), what a request is derived from, and what is
  derived from an answer, are the program's fixpoint's (0065 §2a): the documents a load brings are read there, after
  any stage ran.
- **It reads none of its readers.** Its component of the dependency graph (Tarjan) holds no relation that is not
  staged. A cycle is refused: the relation stays, the cause recorded ("a cycle through its reader `x`").
- **A file-local rule reads it**, with every staged relation placeless, or a staged relation kept reads it (`exports`,
  read by `resolve_export` alone: the stage holds it). The set is found again until it no longer changes.
- **The split does not make it per-file itself.** Once the set is found, each staged relation no other staged one
  reads is asked, once, whether the split with it unstaged makes it per-file, keeps every per-file relation so and
  makes no crossing. Then it is the split's: a view of values its per-file readers hold (0672) is covered in each
  chunk, and no stage derives it over every file. UP032's `mixed` is read by a rule that is per-file once Python's
  resolution is staged; staged, the format family below it was the stage's cross part's.
- **Staging makes no crossing the split refuses.** A relation that staging leaves joining documents undeclared (0652)
  was cross-file before only because it read a staged relation's cone (`used_def(q) :- defq(_, q), imp(_, q)`, `defq`
  per-file once its input is staged). Every relation in its cone is unstaged. Where its cone holds none, its readers'
  cones are: a view the split proves per-file through its readers' rows (`pc_modattr`, read by `pc_base_s`) is not
  proved once a relation they read is staged. Where neither holds one, nothing is staged, so the search always ends.

### 1a. Reads the split could not place

Two reads left a reader cross-file though what it reads is a relation of names, or of its own document:

- **An existence read.** A read of a located relation that leaves every placing column open (`class_def(_, c)`: is
  `c` a class in any file) reads every file's rows. Where the columns it fixes hold no node, it is read from a relation
  of those columns, `rr_any_<relation>_<columns>(c) :- class_def(_, c)` (`phases::existence_reads`), a relation of
  names a stage takes. Python's `sym_of` reads `class_def(_, c)` so.
- **A nearest ancestor.** `nearest(f, s, k)` places both nodes: `s` is an ancestor of `f`, in its tree. The split's
  placing fixpoint counts it (`placed_classes_over`), as Python's scope walk (`rdefn`, `rbound`) needs.

And `mro_method` is declared `cross_file` in `names.rerail`: an ancestor's method is in the ancestor's file, a join
of two documents (0652), which staging `subclass` had made an undeclared crossing.

### 2. A stage is a phase cut from the program

The staged relations and their cone are a phase program of their own, `staged_<k>` (`k` a digest of its rules). It is
run before the program, through 0083's phase path (`phases::facts`, from the run's `Stages`), in its own fact space,
to its fixed point. Its exports are the staged relations, `staged_<k>__s`. The program reads those in place of the
relations: its readers' atoms are renamed, and the staged rules leave it. A cone relation the program still reads
itself stays in the program too. It is derived in both: a fact derived where it is computed, not a copy of a block
(0353).

A stage's first column need not be a granule, and its exports name symbols (the phase path's check). A derived
relation's qualifiers stay with its rules: the stage keeps those of the relations it holds and of the base relations,
not of one the program keeps (a `reach` form asked of a relation no rule of the stage reads failed the stage's
preparation). A stage is run
per run, as a phase is. Its rows are a run's input (`run_constant`), so an edit that changes them leaves the program,
its prepared plans and its keys as they were. `RERAIL_STAGE=off` stages nothing.

### 3. Read per file, keyed by what the file asks

Every earlier phase's export is placeless to the split, as the settings are (0665 §3), whether a stage's or a
`.phase` written by hand. This amends 0083 §2.3, where a phase's exports were placed nowhere and their readers
cross-file. A file's memo entry, though, cannot be a function of every row: one edit to an export would miss every
file. It is keyed by the rows it reads:

- **Probes.** For each rule reading an export `e`, a probe output `rr_probe_<e's digest>_<columns>` holds a location
  of the rule's document and the values the rule fixes `e`'s columns at: its positive atoms other than exports, and
  the comparisons over what they bind. The location is a column the split locates, else a node, else a document key.
  A read inside an aggregate, a read fixing no column, or a rule binding no location reads `e` whole.
- **The entry keeps its probes.** Probes are outputs of the file-local part, kept in the entry's outputs and never
  written as findings.
- **A relation's identity** in a file's entry digests, for each export it reaches, `e`'s rows at the entry's probes,
  in place of all of `e`'s rows (`memo::StageDigests`; `Strata::identities` leaves exports out). A hit recomputes that
  digest over the export's current rows: an edit that changes `e`'s rows at no probed value keeps the entry.
- **A probe the split finds cross-file keys nothing.** Its export is read whole by every entry that reaches it, and
  its crossing is the engine's, not the author's: `crossings_checked` accepts it.

This replaces `SETTINGS_BY_PATH` and `SETTINGS_BY_FOLDER` for `tsx_names`: a read of `target_at` at the file's own path
is a probe like any other. The repository's settings keep their keying (a path is not a value a rule fixes from the
file's own rows).

## Found on the way

A stage is a second program over the same documents in one run, kept in the same store. Five things the store and
the runner kept once per run, or per program only where the program read base relations, were wrong for it:

- **The documents' table is a program's.** The table of the documents a kept part's base relations were read off
  (0458) was one entry per tree and cross part's binds, with the memo plan it was written under. A stage and its
  program have the same binds: each wrote the table in turn, each run read the other's plan and found no unchanged
  document, so every changed input was hashed whole (`r2_…`), and a warm run wrote the table again. It is keyed by the
  plan too (`retain::base_table_ident`).
- **Every kept part keeps it.** The table was written only by a part that keeps a base relation digest-only. A part
  whose inputs are the per-file part's exports alone (a stage's: `edge_at`) found none in a new process and hashed each
  input whole. It is written for every kept part. Without a digest-only input, it is unchanged when the table kept under
  the plan names the same documents (path, key and content).
- **A hit names its tape values.** An entry keeps the requests its fixpoint answered with tape values (0431 §8,
  `memo::VALUES`). On a hit they were made again, which names their tapes' sources in the manifest, only where a corpus
  batch is made (a part keeping base relations). With staging, Python's string annotations are asked per-file; a warm
  run's hits named none, and its manifest was a new one. An embedded string's row now keeps its source, the text's
  sha256 (`Guests::value_source`), and a hit names each value's source where the tapes read before the waves hold it
  (`Parses::name_kept`): no write, as a warm run makes none. An entry made before keeps no source: its hit names none,
  as before.
- **A declared input nothing gives is empty in a file's part.** `node-resolve.rerail`'s `spec_resolved` (a proposed
  core input) has no source. The cross part binds such a base relation empty; a chunk bound nothing, and the planner
  had no statistics for it. Its readers were cross-file before; staged names make them per-file. A chunk binds an empty
  source, measured empty, for each input its program declares that nothing bound.
- **`through` is bounded in depth.** A covered view's proof asks views through views (0672 §3); a stage's run nested
  in the program's went 1,000 levels deep and overflowed a test thread's 2 MB stack. Past 64 levels a view is not
  covered (sound: its readers stay cross-file); the format family's proof is five deep.

## Crossings staging uncovered

Staging never makes a program the split refuses (0652): a relation that would join documents undeclared once what it
reads is staged keeps what it reads unstaged, and failing that nothing is staged. 0652 found that "a crossing inside a
pack can hide behind another". Here, every rule reading Python's resolution was cross-file behind `sym_of`, and staging
it showed what each did on its own: in the first report run, Django's batches staged nothing, and 22 relations across
the packs stood refused. The refusal names every relation that would cross, each with its own cause, so one run lists
them all. Each was read, as 0652 says: a gap in the split is fixed in the split; a crossing that is meant is declared.

**Gaps, fixed in the split:**

- **A read at `unset` of a location column that never holds it** (`kind(-1, "none")`, `!pth101_file_descriptor(-1)`,
  `value(-1, $false)`) matches no row of any document: it reads none (`Rels::at_unset`). The patterns' lowering writes
  one for a metavariable a branch leaves unbound (`unset(T)`, then `kind(T, "none")`). A location column of a base
  relation, or a derived relation's located one, never holds -1; the tape's parent column does, at a root, and a
  nullable location (0649) holds it by meaning. Both placements agree (`one_document` and `why_not_local`, the second
  checking the first), each after the refusal of a cross-file read. Bandit's B113, B614 and B004, Ruff's PTH101 and
  PTH211.
- **A rule whose comparison of two constants fails** (`-1 != -1`, `1 = 2`) derives nothing: the split leaves it out
  (`dead`), its head still derived. The lowering writes one where a branch's `unset(SPAN)` meets `!unset(SPAN)`:
  Ruff's PT001's dead branch filled `at` with -1, and the column was a location no longer. PT001, PT002, PT025 and
  PT026's findings were per-file with no location, and `violation` could not read them.

**A policy that called a library relation by its old signature:** ESLint's `no-useless-computed-key` read
`touching(t, k)`, the signature of NOTES-js-2.md, where the library's is `touching(n, x)` (the code before `n`), and it
never put its key in `layout_at`. `touching(t, k)` joined any node whose preceding code equalled the key's id, in any
file; and no key was spaced. Written as `prefer-exponentiation-operator` writes it (`touching(k, a)`, `layout_at(k)`),
ESLint's own cases hold: `class A { static[0]() {} }` and `get[1]()` are spaced (`static 0()`), `{ ["c"]: 2 }` is not.

**Crossings meant, declared `cross_file`** (each rule joins a reference to a definition named in another file):

- `dj_task_waits_for_commit` (the task's definition and the Celery app), `dar_non_atomic_wrapped` (the wrapping call);
- a model's base class: `dj_model_super`, `goc_meta_from`, `goc_meta_opt`, and a model's method through it,
  `bwh_method`, `bwh_in_hook`;
- a name resolved to its class or function: `dj_qs_model`, `dj_instance_value`, `rmw_class`, `ghw_model_ref`,
  `goc_call`, `sfu_atomic_def`, `tam_task_call`, `tam_instance_value`, `tam_instance`;
- the settings module `DJANGO_SETTINGS_MODULE` names: `smi_doc`.

With them, Django's batches stage Python's resolution.

## Tests, written first, one per requirement

| Requirement | Test |
|---|---|
| A relation of names read by a per-file rule at the file's own values is staged; the reader is per-file | `file_local::tests::a_relation_of_names_read_per_file_is_staged` |
| A relation holding a node, or reading a position, is not staged | same test |
| A cycle (the relation reads its reader) is refused, the relation cross-file | same test |
| A phase's export, like an option, is placeless: a rule joining it is file-local (0083 §2.3 amended) | `file_local::tests::a_rule_joining_a_phase_export_or_an_option_is_file_local` (was `a_rule_joining_a_phase_export_is_cross_file_and_one_joining_an_option_is_not`) |
| An edit that changes an export's rows at no file's probes keeps every other file's entry; one that does misses those files alone; rows are a fresh run's | `corpus::tests::a_files_entry_is_kept_under_the_stages_rows_it_reads` |
| `tsx_names` through the probes: 0665's test holds | `corpus::tests::a_files_entry_is_kept_under_its_own_imports_resolution` |
| A relation a cross-file join reads stays cross-file where its program says so (`called` declared `cross_file`) | `made_batch_tests` (the late program) |
| A context's `.from` read of a staged relation changes with the export | `session::tests::a_contexts_strata_run_only_on_changes_to_its_roots_and_reads` |
| Not staged: a nullary flag, a request's cone (C's includes), a kind set, a native's demand | `nullary_relations_preserve_boolean_set_semantics`, `c_includes_resolve_by_the_search_order`, `a_header_edit_asks_again_only_for_the_units_that_read_it`, `axes_read_node_sets_and_negate`, `match_all_over_a_node_gives_each_match_and_count_ranges_over_wildcards` |
| Staging leaves no crossing the split refuses, in every pack and the data layer, batched and policy by policy | `pack_split_tests` |
| Existence reads: a read leaving every placing column open is read from a relation of names, staged; an edit misses the files whose probe changed alone; rows are a fresh run's | `corpus::tests::an_existence_read_is_staged_as_a_relation_of_names` |
| `nearest` places both nodes: Python's scope walk is per-file | `file_local::tests::a_nearest_ancestor_is_placed_with_its_node` |
| A view of values whose reader is per-file once a relation of names is staged is covered, not staged | `file_local::tests::a_relation_of_names_read_per_file_is_staged` (`mixed`, `up`) |
| The format family stays per-file in every split the run makes, a stage's too | `pack_split_tests::a_format_strings_ranked_fields_are_per_file` |
| A stage keeps no qualifier of a relation the program keeps (`reach`) | `corpus::tests::the_reach_index_is_derived_again_when_cycles_and_connections_change_in_a_new_process`, `session::tests::a_session_derives_the_reach_index_again_when_cycles_and_connections_change` |
| A stage's and its program's documents' tables are their own; a part whose inputs are exports alone keeps one | `maintain_tests::r2_a_new_process_hashes_only_the_changed_documents_rows_of_its_inputs` |
| A warm run's hits name their tape values: it writes nothing | `corpus::tests::the_run_after_a_cold_one_with_tape_values_writes_nothing` |
| A declared input nothing gives is empty in a file's part | `corpus::tests::a_declared_input_nothing_gives_is_empty_in_a_files_part` |
| A covered view's proof nested in a stage's run fits a test thread's stack | `session::tests` (every session test aborted on it) |
| A read at `unset` of a location column reads no document; the tape's parent column is read as a row's; a nullable one's read places nothing apart | `file_local::tests::a_read_at_unset_of_a_location_reads_no_document` |
| A rule comparing two constants falsely derives nothing: it places no column | `file_local::tests::a_rule_comparing_two_constants_falsely_derives_nothing` |
| A refusal names every relation that would cross, with its cause; each pack's batches stage what the declarations allow | `pack_split_tests` (each pack) |
| A stage's held programs are the run's state, not a kept map | `facts_only::every_kept_map_is_state_or_listed` (`Stages` listed with 0104's state) |

## Results

- The position report (every data-layer policy over the 334-file corpus, every pack over a small corpus, one Linux
  host, every run completed): 41 flagged rules after step 1 (0672), 32 now. No rule is newly flagged. The 9 gone:
  authentication-plan-purity's `rr_agg28`; lazy-architecture's `la_first`, `la_intr`, `rr_agg73` and its demand;
  post-op-no-internal-dispatch's `rr_agg29`; Python's `p51_later_finalizer`; Django's `p15_mis_later` and
  `p19_siuf_sets`, once the Django crossings above were declared and its batches staged.
- The Python pack over one file, cold, one Linux host: 12.1 s and 0.94 GB with `RERAIL_STAGE=off`, 13.6 s and
  0.86 GB staged.
- The split's 49 tests, the 33 pack split tests, and `rerail-logic`, `rerail-typing`, `tests_logic`, `tests_corpus`,
  `tests_rr`, `tests_lang`, `tests_findings` and the device crate's tests pass (1,440), but two of the device crate's
  GPU tests, which fail on a Linux host with a backend failure, a different one each run.

## Left open

- Staging still refused where a relation would join documents undeclared, each step 4's or step D's:
  plan-helpers-allowlist's `pha_latest` and versioned-merged-identity's VMI002 (joins already flagged, E), the lazy
  architecture's LAO003, purity-calls' `pc_tgt`, data-layer-module demands (`rr_agg28`, `rr_agg5`: a class by its
  exported name), FAST003's `rr_or142` (two nodes joined by their text), a demand flag over every file (`p34_rr_not2`,
  D), and three data-layer `violation` rules reading a finding with no location.
- Django's rules below `mro_method` and the names now declared (`p12_rr_ordered…`, `p23_rr_ordered…`, `p20_rr_agg10`)
  read positions across files: F's, read against the originals.
- A relation whose rules are all dead is not yet taken as placeless.

Node ids handed between the split's parts (the per-file part's located exports) are not re-keyed; only a phase's
exports name symbols alone. That is a decision of its own.
