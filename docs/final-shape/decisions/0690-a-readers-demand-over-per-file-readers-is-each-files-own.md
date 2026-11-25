# 0690. A reader's demand over per-file readers is each file's own

Status: decided (2026-11-25), by the user's plan for the rules the position report flagged after 0665: "E and F, read
against the originals, 14 rules." This is its first part: plan-helpers-allowlist and versioned-merged-identity, read
against their linters, and what the engine needed for them. It amends the split (0051, 0652, 0672,
`typing::file_local`) and product demand's deferred joins (0488).

## Intro

Group E of the report is joins that bring two documents together. The plan said to read each against its original
linter: some are joins within one file that the split cannot prove, to be fixed in the port's keys; some are true
comparisons across files.

Two policies were read first. What held them cross-file was three things, only one of them in the original:

- **plan-helpers-allowlist** (5 flagged rules). The linter keeps one map of names per file. The port's `pha_latest`
  took "the latest earlier write of `x`" as a visit index and then found the write at that index with `visit_index(v,
  k)`, `v` unbound. A visit index is a document's, so `v` was every file's node at that index: a join of two documents,
  and a wrong answer where two files write the same name at the same index. Its helpers `pha_private` and
  `pha_eager_alias` gathered names from every file (`kind(n, "identifier"), text(n, x)`, `pha_local(l)`) where the
  linter tests the name alone (`_is_private_plan_helper_name`, `_is_eager_alias`).
- **versioned-merged-identity** (4). Its whole analysis went cross-file through `rr_reader_demand_vmi_callee`, the
  restriction product demand installs on `vmi_callee(f, x)` at its name: names gathered from every file's aliases. And
  VMI003's report rule, which joins a constructor to the path the plugin reports it at, demanded `vmi_backed_ctor`
  through that join.
- The plugin does report a caller's findings at the last path it recorded for that caller (`paths_by_caller`, a
  dict): two files of one name under the two production roots, with a method of one qualname, are both reported at the
  graph query's path, each at its own line. That is across files in the original.

## Decision

### 1. A reader's demand over per-file readers is each file's own

A reader's demand (`rr_reader_demand_r`, `READER_DEMAND`) is the optional guard product demand installs on `r`'s
producers (`product_demand::upstream`). Its rules are the prefixes of `r`'s readers, so it holds every key they ask for.
Its rows are values and name no document, so the split took it as cross-file, and `r` and everything above with it.

The split now takes it as each file's own, as a covered view of values (0672), where:

- each of its rules reads one document's rows at most in each derivation (`one_document_each`): a positive read whose
  location is left open (`alias(_, x)`) is on a document of its own, and a rule reading no document's rows is on none;
- the relation it guards is per-file, and not an output;
- every reader of that relation is per-file, other guards' rules aside.

Then in each file it holds the keys that file's readers ask for, made from the file's rows, and `r` there holds every
row they read. A reader across files may join one file's prefix with another file's rows, and then the guard is
every file's, as before. A relation of values read by guards alone (the preparation's projection of an open read,
`rr_any_alias_1(x) :- alias(_, x)`) is part of them. The guard is held where `covers` holds it or where it holds as a
guard (`guard_holds`), and is asked again whenever a relation leaves, as a covered view is.

### 2. A deferred join's request that reaches its key by order alone is not made

A deferred join's request (0488) asks a producer for the keys its readers will read. `void_request` already declines one
that asks for every key the producer makes. A request whose every atom naming the key says only its document and its
order (`node(d, k, _, _, _)`, `docof(k, d)`, `visit_index(k, o)`: `earlier(i, c)` as the lowering writes it) is declined
too (`order_only`). It asks for every node of a document before or after some point, which restricts nothing the
producer does not, and costs a join of the document's nodes with the other side. Made per file, versioned-merged-
identity's `rr_deferred_pairs_vmi_alias(i)`, every node before some call, ran past 300 s on 10 files, where the
policy takes 20 s on 1,560. A join left as written is always right.

### 3. The ports, read against their linters

- **plan-helpers-allowlist.** `pha_latest` reads the write at that index in `e`'s own file (`docof(e, d),
  visit_index(v, k), docof(v, d)`). `pha_private` and `pha_eager_alias` test the text their reader holds (`text(_, x)`,
  a string filter, 0037): private and eager-sounding by spelling alone, as the linter's functions are. `pha_local` is
  gone.
- **versioned-merged-identity.** The analysis is `vmi_finding(c, k, s)`, each constructor's finding in its own file.
  Only the two report rules read `vmi_report_path`, the path the plugin reports a caller at: across files, as the
  original is, and they read the constructor's line there.

## Tests, written first, one per requirement

| Requirement | Test |
|---|---|
| A reader's demand whose rules are each on one document, guarding a per-file relation all of whose readers are per-file, is local and not handed on; one guarding a relation a cross-file rule reads is every file's | `file_local::tests::a_readers_demand_guarding_per_file_readers_is_each_files_own` |
| A run's rows with such a guard are a whole run's, the guard derived in the files' part in every split, through the preparation's projection of its open read, and an edit dropping a file's alias gives a fresh run's rows | `corpus::tests::a_readers_demand_over_per_file_readers_is_each_files_own` |
| A reader reaching the key only through its document and order makes no request; one narrowing it otherwise still does | `deferred::tests::a_reader_reaching_the_key_by_order_alone_leaves_the_join_as_written` |
| plan-helpers-allowlist finds what its linter finds: a name's helper from its own file's map (the old port gave each of two files the other's helper), private and eager names by spelling, a private call through the module alias, an eager import | `pack_split_tests::plan_helpers_allowlist_finds_what_its_linter_finds` |
| versioned-merged-identity finds what its checker finds: VMI001 to VMI003, an alias, a projection backed through a chain or assigned after the call, and a caller two files share reported at the graph query's path with each line | `pack_split_tests::versioned_merged_identity_finds_what_its_checker_finds` |

The original linters have no recorded events for these two checks in the data-layer port's fixtures, so the cases are
read from their code.

## Results

- The position report (every data-layer policy over the 334-file corpus, every pack over a small corpus, one Linux
  host, every run completed): 31 flagged rules after 0686, 23 now. None newly flagged.
  - plan-helpers-allowlist: 5 to none. Its cross-file part reads no position.
  - versioned-merged-identity: 4 to 2. The analysis is per-file. `rr_place_VMI002` and `rr_place_VMI003` read the
    constructor's line where the plugin moves a finding to another file's path.
- Both policies over 1,560 files of the 9,010-file tree (every production-root file, every file naming a helper), old
  text and new, one Linux host: the same findings (none: the tree passes its linters), plan-helpers 37 s to 29 s,
  versioned-merged-identity 25 s to 20 s.
- The split's tests, the deferred tests, the 35 pack split tests, and `rerail-logic`, `rerail-typing`,
  `rerail-rr-root`, `rerail-corpus-prep`, `tests_logic`, `tests_corpus`, `tests_rr`, `tests_lang`, `tests_findings` and
  the device crate's tests pass (1,499). The device crate's two GPU tests that fail on a Linux host fail as in 0686.

## Left open

- versioned-merged-identity's relocation reads a line across files. A finding of one file's constructor placed at
  another file's path, derived per file with the path read as a setting, is not something the split places: its row
  would be in a document its rules do not read. Left as the original's behaviour, across files.
- The rest of E and F: the relations declared `cross_file` (eager-resolve's `er_facade_site`, lazy-architecture's
  `la_eager_delegate`, Django's `mro_method`, `dj_instance_value` and `p20_sfu_atomic_def`), purity-calls' `pc_tgt`,
  PLP-007's `plp_xq`, Python's `resolve_export` under `rr_agg207` and `rr_agg215`, cached-pure-lazy's `pe_ord` and
  `rr_agg65`, eager-resolve's `er_mentions`.
