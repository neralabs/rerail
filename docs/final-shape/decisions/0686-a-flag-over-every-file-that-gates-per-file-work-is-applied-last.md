# 0686. A flag over every file that gates per-file work is applied last

Status: decided (2026-11-25), by the user's plan for the rules the position report flagged after 0665: "D: apply the
condition last, 11 rules." It amends the split (0051, 0652, `typing::file_local`) and how a run reads its inputs
(`corpus::inputs`).

## Intro

A program runs in two parts (0051). The per-file part is derived again only for an edited file. The cross-file part
is maintained over all files. A rule that reads a relation made from every file's rows is cross-file, and so is every
rule above it.

Group D of the report is rules whose per-file work is switched by such a relation and nothing else:

- **A flag over every file.** pure-lazy-runtime-purity narrows the files it analyses as its linter's `scan_paths`
  does: the candidate files (`plp_cand`, reached across files through the call graph), or every file when there is no
  candidate (`plp_docin(d) :- plp_cand(d)`, `plp_docin(d) :- doc(d, _, _), !plp_anycand()`). Every analysis rule
  reads `plp_in_rec(f) :- pc_rec(f), docof(f, d), plp_docin(d)`, so the whole analysis (`plp_bfs`, `plp_tnt_at`,
  `plp_opq_before`, the `__all__` reads under `rr_agg52`) was cross-file.
- **An input file the corpus does not have.** swallowed-exceptions reads its ignores file
  (`.input ignored(IO=file, …)`), and pure-lazy-runtime-purity its allowlist directory (`plp_grand_line`). Where the
  file is missing, as in the 334-file sample, the relation was neither an input nor a fact: a base relation with no
  place. Its readers (`ignored_at`, `plp_entry` and through it `plp_approved`, `plp_report`) went cross-file.

The flag has one value in each file: every rule reading it reads it at its own document. So each file can derive what
lies above it under both values, and the cross-file part, which knows the value, picks.

## Decision

### 1. A flag over every file is applied last

A **flag** is a derived relation the split finds cross-file, with no column (`plp_anycand()`) or one column holding a
document's id (`plp_docin(d)`: its placing column), and read by a rule the split leaves cross-file.

Its **cone** is the relations that are per-file once the flag is placeless and are not before, reached from the flag's
readers through each other (`Program::without`, as a stage's exports are read in 0675). The flag is applied last when:

- **Each read is at the reader's own document.** A one-column flag's argument in every rule of the cone is a variable
  that a positive read of another relation holds at a column placing its row's document (`docof(f, d)`, `doc(d, …)`).
  A reader at another document (`remote(n) :- …, docin(0)`) is per-file with the flag placeless, so it is in the cone,
  and it refuses the flag. No rule of the cone aggregates over the flag.
- **The flag reads none of its cone**, so its value is no copy's.
- **Each relation of the cone is an ordinary one**: not a native, a native's input, a lattice, a function, a test, a
  demanded relation or a demand gate, a load's request or answer, the on-request rewrite's, a kind set a rule names,
  nor one with facts stated. The engine reads those by name.
- **A relation of the cone read outside it** (by a cross-file rule, or written out) has a location: the cross-file
  part reads the flag at the document of each of its rows.
- **It makes no crossing** the split refuses (0652), taken alone.

The rewrite (`FileLocal::flags_applied_last`, `flags_last.rs`):

- **Two copies.** Each relation `r` of the cone is derived twice, `rr_flag1_r` under the flag holding and `rr_flag0_r`
  under it failing (`flag_copy`), from the cone's rules with every read of the cone renamed to the copy. A read of the
  flag that holds under the copy's value is dropped. A rule with one that fails is left out. A copy no rule derives
  is stated empty (a constant to the split).
- **The cross-file part's choice.** A relation of the cone read outside it keeps its name and is derived from the
  copies: `r(x) :- rr_flag1_r(x), docof(x_c, d), flag(d).` and `r(x) :- rr_flag0_r(x), docof(x_c, d), !flag(d).`
  (`x_c` its location column; read directly where that is a document's id). A nullary flag is read at its unit
  (`rr_unit`). Outputs keep their names.
- **Exact.** Under the flag's value in a file, the copy of that value derives what the original rules derive there:
  every rule of the cone is on that file's rows, and the flag is one value over them. Negation, aggregates and
  recursion inside the cone are each copy's own. A rule the flag does not gate (`an(n, 2) :- option(…), …`) is in both
  copies.
- **Rounds.** Flags whose cones overlap wait for the next round, which finds them on the program the last left: a
  copy that reads another flag is applied in turn. A relation a round made the cross-file part's choice is no cone's
  again. Four rounds at most.

The run applies it before staging (0675), so a stage sees the copies (`phases::flags_last`, `RERAIL_FLAGS_LAST=off`
leaves it out). A phase's own run and the constants' are left as they are, as with staging.

The cost is the cone derived for every file rather than for the candidate files alone, once per value. Under the
value most files have, a copy is the analysis. Under the other, the reads of the flag that fail leave most rules out,
so that copy is mostly empty. An edit derives both copies again for the edited file only, and the cross-file part
picks per row. Before, the whole analysis was cross-file and was derived again over every file at each edit.

### 2. An input file missing or empty is stated empty

`populate` states an input's facts empty when its repository file is missing or holds no rows (`set_facts(rel, &[])`),
unless the program states facts of it already. Its readers read it so already: no row. Stated, the split takes it as a
constant, as a relation stated empty in the program text is (`a_relation_stated_empty_is_a_constant`), and a per-file
rule reading it stays per-file. An input that is incomplete rather than empty is left unstated: an artifact that is
missing, stale, failed or malformed, and a file read at the base revision that cannot be read. Its readers are
withheld (FINDINGS-LAYER section 2), as before.

### 3. A cause names what holds a relation whatever the flag

A relation's cause names the first cross-file read the split met (0652's report follows it to its root). A flag's
choice is cross-file because the flag is, and a read of the head itself because the head is. So a rule that also
reads another cross-file relation names that one (`why_not_local`). A relation whose cause still names a flag's choice
takes another of its rules' reasons where one has another (after the split's fixpoint). `plp_colored` was reported
through `plp_in_rec` and `plp_docin` to `plp_anycand`. It is now reported through its walk over `pc_edge` to `pc_tgt`,
which holds it cross-file whatever the flag.

## Tests, written first, one per requirement

| Requirement | Test |
|---|---|
| A one-column flag read at each reader's own document, and a nullary one, are applied last; the copies are per-file, a negated read, an aggregate and an ungated rule inside the cone among them; a copy no rule derives is stated empty; outputs keep their names and read the flag; no crossing made | `file_local::tests::a_flag_over_every_file_gating_per_file_work_is_applied_last` |
| A reader of the flag at another document keeps it from being applied, with that reader named | same test (`remote`) |
| A run's rows are the original program's under each of the flag's values, the copies in the files' part, and an edit turning the flag over gives a fresh run's rows | `corpus::tests::a_flag_over_every_file_is_applied_last` |
| An input file missing or empty is stated empty, and a per-file rule reading it stays per-file | `inputs::tests::an_input_file_missing_or_empty_is_stated_empty` |
| An incomplete artifact or base revision is not stated: its readers are withheld | `inputs::tests::an_incomplete_artifact_gives_an_input_error_and_withholds_its_negative_readers`, `an_unreadable_base_gives_an_input_error_and_withholds_the_comparison` |
| A rule reading a flag's choice and another cross-file relation, and a relation one rule of which reads only the choice and another itself and a cross-file relation, are named cross-file by the other relation | `file_local::tests::a_flag_over_every_file_gating_per_file_work_is_applied_last` (`both`, `either`) |
| Every pack and the data layer split with their crossings declared, batched and policy by policy | `pack_split_tests` |

## Results

- The position report (every data-layer policy over the 334-file corpus, every pack over a small corpus, one Linux
  host, every run completed): 32 flagged rules after step 2 (0675), 31 now. No rule is newly flagged.
  - swallowed-exceptions' `ignored_at` is per-file (§2).
  - pure-lazy-runtime-purity's flag `plp_docin` is applied last: `plp_in_rec`, `plp_fq`, the `__all__` reads and
    `rr_agg52` are derived per file under each value. Its rules still flagged are each held by another cause, now
    named (§3). `plp_bfs`, `plp_opq_before` and `plp_tnt_at` are held by `plp_colored`'s walk over the call graph,
    which `pc_tgt` joins across files. `plp_report` and the demand `rr_demand_agg_rr_agg57` no longer read the
    allowlist with no place (§2). They and `plp_found` and `rr_ctx58` are held by `plp_xq`, PLP-007's resolution
    through re-exports. It reads the flag at the document of another module (`doc(d, _, pm), plp_docin(d)`), so it
    is cross-file, as resolution is.
  - Django's `p34_rr_not2`, which the staging refused on the way (0675, Left open), is not flagged: Django's batches
    stage.
- The split's 50 tests, the 33 pack split tests, and `rerail-logic`, `rerail-typing`, `rerail-rr-root`,
  `tests_logic`, `tests_corpus`, `tests_rr`, `tests_lang`, `tests_findings` and the device crate's tests pass (1,496).
  Two of the device crate's GPU tests fail on a Linux host, now on every run, and at 0675's commit too:
  `views_made_on_the_device_are_the_hosts_rows` (CUDA error 716, a misaligned address) and
  `an_index_built_twice_that_a_batch_took_is_kept` (a launch outside any evaluation, past the launch budget).
  Neither reads the split.

## Left open

- purity-calls' `pc_tgt` (a call joined to its target in another file, undeclared) holds pure-lazy-runtime-purity's
  analysis cross-file: step 4 (E), read against the original linter. Once it is declared, `plp_colored` is a
  coloring across files, cross-file as meant, and what reads it follows.
- PLP-007's `plp_xq` reads the flag at another module's document. Staging it (0675) needs the flag's cone in its
  stage: step 4, with `pc_tgt`.
- The cost of a cone derived for every file, rather than for the candidate files alone, is not measured. pure-lazy's
  cone is small until `pc_tgt` is settled.
