# 0705: An asked phase answers only what its files ask

Status: built (2026-11-26); the rounds and the asked `py_names` behind `RERAIL_ASKED_PHASES`. Amends 0083 §1 (a
phase's run) and 0665 (which relations are settings, and how each keys a reader's memo). Follows 0702.

## Intro

**The problem.** A phase such as `py_names` works out every module's whole interface ahead of time, whether or not
anything reads it. On the data layer's 224-file sample it hands on 125,520 rows, while the files' imports ask about
227 (module, name) pairs of the sample's own modules (0702, "Open").

**What changes.** An asked phase runs in rounds:

1. Each file states what it asks of other modules (`asked_at(p, "m.x")`).
2. The phase's cross part sends each ask to the file of the module it names (`ask_in(p, x)`).
3. In the next round, the asked file reads that row at its own path and answers in its per-file part.
4. An answer that is an import of another module becomes an ask for the round after.
5. The rounds stop when a round's asks are the ones it was given.

Each round is a run of the same phase program. The asks reach the next round as a setting read at a file's own path
(0665), so the rule that answers stays per-file, and a file's memo entry is keyed by its own asks. The design was
weighed on the "Asked phases" page, which also lists what was turned down: demand inside one program, rounds inside
one run, and asks keyed per file.

## The declarations

```
.phase q
.rounds ask_in
.export asked_at by_path 1, ask_in by_path, answer by_key, resolved by_key
.decl q__ask_in(p: symbol, x: symbol)
```

- `.rounds R, S`: the phase runs again while any of them changes. Each round reads the last round's rows of each as
  `<phase>__R`, declared with `R`'s columns. Each is an export declared `by_path`.
- `by_path`, `by_path C`, `by_key`, `by_key C` after an export: how a reader's memo entry is keyed by it.
  `by_path`: the rows at the file's own path (column 0). `by_path C`: those rows, and every row of a `by_key` export
  under each key named in their column `C`. `by_key`: rows keyed by column 0, read where a `by_path` row names the
  key. `by_key C`: those rows, and in turn every `by_key` row under each key named in their column `C`, to the
  closure. Key columns are symbols. An export with a key is a setting to the split (0665): placeless, read per file.
- `R as E` hands on relation `R` as export `E` (`asks_at as asked_at by_path 1`): a phase hands a relation on under
  the name its readers read, where its program makes another of that name, as an included library's.
- The project's settings are declared the same way, beside their declarations in the library's `core.rerail`:
  `.keyed package_for by_path 1, tsconfig_for by_path 1, package by_key, package_field by_key, tsconfig by_key,
  tsconfig_path by_key, package_export, env` (an item with no key is read whole by every file).
- With the switch off, a phase that declares `.rounds` is refused, with an error that names the switch.

No list in the engine names a setting. `file_local::SETTINGS_BY_PATH`, `SETTINGS_BY_FOLDER` and `setting_input`'s
own list are gone: `tsx_names`, `py_names` and `points_to` declare their exports' keys on their `.export` lines,
and `core.rerail` the project's.

## What was built

- `phases::declared` reads `.rounds` and the keys after an export. `phases::keyed` checks them: the export exists,
  and the key column is a symbol other than column 0. `text_demand::node_anchors` reads an export's name as its
  item's first word.
- `phases::facts` runs an asked phase in a loop. Each round hands down `<phase>__R` with the rows the last round
  ended with. The loop stops when a round's `R` equals what it was given. Past 16 rounds (`MAX_ROUNDS`), the run
  fails with an error naming the asks the last round added, and nothing is handed on.
- Round 1 starts from the asks the last run of the phase over the same root ended with. They are kept in the
  store's run records under the phase, its text's digest and the root (`kept_asks_key`). So an edit that changes no
  ask takes one round. A stale kept ask changes no answer, because `R` is derived afresh each round from the files'
  own asks.
- `plan::phased::declared_key(rel)`: how `rel` is keyed, from the keys a run's phases declared (bound in the run's
  context once they ran, and by an asked phase's rounds for their own read of `R`), else from the engine's phase
  program's `.export` line (`<phase>__<export>`), else from `core.rerail`'s `.keyed` line. Each file is read once a
  process for each state of the file. So a check outside a run (a placement check of a program that runs no phase)
  sees the same keys as a run. `file_local::setting_input`, `setting_by_path` and `setting_by_folder` are its
  answers alone; `memo::settings_scopes` and `SettingsDigests::of` read them through those. The prepared program's
  key hashes the keys of the inputs it reads.
- `PhaseRun::rounds` gives each round's asks and files built. `PhaseRun::keys` gives the reader's relations' keys.

## Tests

In `tests_corpus/…/corpus/phases/tests.rs`, on a small phase `q` and its reader:

- `an_asked_phase_answers_only_what_is_asked`: only `m.x` is answered, not `y` or `z`. Two rounds, and round 2
  builds `m.py` alone. With the switch off, the phase is refused.
- `an_asked_phase_follows_a_reexport_in_rounds`: a star import is followed in three rounds. A cycle ends with no
  answer. The reader's negation reads the closed rounds.
- `an_asked_phase_past_its_rounds_fails`: asks that grow forever fail after 16 rounds, with the error naming them.
- `an_edit_reruns_the_files_its_asks_reach`: four edits in turn, each with its rounds and the files the phase and
  the reader build; the rows then equal a fresh run's.
- `an_exports_key_is_declared_by_its_phase`: the keys are reported, the names are in neither list, and a bad key
  column is refused.
- `an_asked_phase_starts_from_its_last_asks`: an unchanged tree runs one round; asks that shrink leave no removed
  ask in any export.

## The asked `py_names`

`typing/programs/py_names.asked.rerail` runs in place of `py_names.rerail` while the switch is on
(`phased::builtin_phase_file`, `phases::candidates`). It hands on the same exports, which the same rules read
(python/names, python/django, python/upgrade), but answers only what is asked:

- `spelled_at(p, k)`: each qualified name the file at `p` spells through an import (`qn`, "m.x").
- `asked(m, x)` and `ask_in(p, x)`: the cross part sends each ask to the file of module `m`, and follows it through
  every re-export and star import it reaches (`fwd`, `star_of`). Those are each file's own module-level imports,
  made whatever is asked, so round 1 has every ask, round 2 answers, and a cold run takes two rounds: before, a
  forward was made only for an asked name, and each re-export an ask passed cost a round (three on the PostHog
  sample).
- In the next round, the asked file answers at its own path: a top-level definition (`ans_def`), and whether the
  name is public there (`ans_pub`, which reads the file's own `__all__` once, `all_here` and `all_assigned`, where a
  count over its assignments was made for each ask). With the forwards, these are python/imports' three rules of
  `exports` and `exported`, for the asked names alone.
- `asked_exports` follows the answers through the forwards, as `exports` and `resolve_export` do, and is handed on
  as each ask's `resolved` row. The phase's own lookups (bases, members' calls and references, Django's receivers and
  managers) read it in place of `exports`, and `reads_at`'s second rule takes the modules the file's own asks
  resolve to, not every export of every module it names.

`class` and `defined` are answered for the symbols the files may name:

- `sym_at(p, q)`: each symbol the file at `p` may give a node, from its own tree: python/scopes' `sym_of`, `qn`, and
  an attribute of either at any depth. `obj_at` and `attr_at` are the same of an attribute's object and of an
  attribute, where python/names' `sym_of` rule 3 reads `class` and `defined`.
- `res(k, q)`: what the symbol denotes, as the readers resolve it: itself, an import through the asked exports (rule
  1), a definition `c.a` of the run where `c` is what its prefix denotes (rule 3; bounded by a split of a `defined`
  row, since a made `@dot` is refused as an unbounded value).
- `class`, `defined` and `class_named` are handed on for the classes an object may denote (`handed_obj`), the
  definitions an attribute may (`handed_attr`), and the classes any node may (`handed_sym`, python/django's
  `dj_model_ref`). They are read in the cross part, where the phase's own rules (`reads_at`, the bases, the members)
  read every file's `class` and `defined` already, so no round asks a file for them. `.rounds` takes several
  relations (`.rounds R, S`), but `py_names` needs only `ask_in`.
- A file's own definitions are asks of their own (`own_def`, in `handed_any`), for their ancestry, members and
  Django facts, never for `class` or `defined`.

python/django's `dj_model_at` asks whether a class is a model of a class it knows by name alone, a proven type
among them, which no file names and so none can ask of `py_names`. The phase applies the same tests to each class
of the run and hands on the verdict, `dj_model_class(m, q)`, which `dj_model_at` and the Django pack's
`tam_instance` read in place of `class`, `ancestor` and `member_call` (`dj_class_at`, `dj_expression_at`,
`tam_model_type` and `tam_expression` are gone; the tests' helpers `dj_field_module`, `dj_relation_class` and
`dj_expression_base` moved to python/django-shared, which both include). `py_names.rerail` hands it on too.

The answers read the asked file's tree, not its binding analysis: a round that asks a file anew derives its answers
as a partial memo hit (decision 0225 §2), and a cone that reads `qn` builds that analysis again. So `fwd` takes the
file's module-level imports, and the cross part drops a tuple the file imports only under `if TYPE_CHECKING:`
(`tc_only`, which reads no ask, held in the entry from round 1). A file on the memo's kept route whose outputs
missed now takes that partial hit too (`corpus.rs`, `Unbuilt::partial`): it was built again whole.

A file's entry holds the relations the ask-reading rules read without reading an ask (`corpus::asked_held`): of
the rules that read `<phase>__ask_in`, or a relation such a rule derives, the derived relations they read that
read no ask, are not exports or outputs, name a file's node, and are no view of the tape (a base relation, an input,
a view of an input's columns as `kind` is: read in place, decision 0353). For py_names these are `def_name`,
`qualname`, `all_here` and `all_assigned`. Each is kept to the chunk's end, not handed off, and its rows go into the
files' entries alone, never to the cross part. A later round's partial hit stops its cone at them
(`wanted_cone`'s boundary): it reads their rows from its files' entries as inputs (`flat_source`) and derives only
the rules that read the asks, and the new entries carry those rows on. Before, round 2 derived `def_name`,
`qualname`, `value`, `all_assigned`, `rr_value_req` and the `modname` strata again for each asked file (about
0.6 s of round 2's CPU over bwh's Django sample): now it runs `ans_def_at` and `ans_pub_at` alone.
`Report::memo_partial_rules` (`Round::partial_rules`) is the most rules a chunk of partial hits ran:
`a_later_round_derives_only_the_rules_that_read_its_asks` holds it at 1 for round 2, where without the held `named` its cone
would be 4 rules (`answer`, `named`, `sig`, `defn`).

A later round reads its chunks' constants as the first round measured them (`phases::RoundsMeasured`, bound for
the rounds, keyed by the prepared program and the chunk's mix), as a later chunk of a mix reads the first chunk's
within one run: each round runs the same prepared program over the same tree. Over bwh's Django sample the
measurements went from four (two mixes, two rounds; 0.35 s) to two (0.14 s). `Report::constants_measured`
(`Round::constants_measured`) counts them, and the test above holds round 2 at none. The other costs a round pays
again are small there: the base rows' statistics are round 1's kept inputs (0.11 s) and about 6 ms in round 2, and
writing the entries is 0.05 s over both rounds. The cross part runs in each round, since it makes the asks, and
its session derives again only the strata the new asks reach.

`asked` makes a row only for a module of the workspace: a name of an outside package, or an attribute chain past a
module, is asked of no file.

## Every export keyed by the file's asks

`py_names` hands on every fact by its symbol, and a reader's memo entry is keyed by the answers to its own asks. Both
programs include python/py-names-handed, one definition of what is handed on; each names the symbols it hands facts on
of (every one of the run's, eagerly; those the files' asks reach, asked):

- `asked_at(p, k) by_path 1`: the file's asks: each symbol a node of it may denote (`sym_at`) that a fact is handed
  on of, or that resolves through an export.
- `own_at(p, q) by_path 1`: the file's own classes, what its readers ask of their own definitions (ancestry, their
  body's and ancestors' members, their Django facts). A def or method is no row. A separate export, so a reader that
  reads no fact of its own definitions (python/names' rules 1 and 3) is handed none (0702).
- `own_rcv_at(p, f) by_path 1`: the file's own functions that are receivers (`dj_receiver_fn`), the one fact a
  reader reads of its own functions, which python/django's `dj_receiver_def` reads. A function is a receiver through
  a `connect` call that may be in another file, so this one is made of the fact, and only for a reader that reads it.
  `own_at` is made of the file alone: filtering it by the facts handed on of each class made every fact, for every
  reader, whether it read them or not (0702), which cost the Django policy's phase 1 s of its 3.
- `star_public(p, x) by_path`: x is public in the file's module through a star import. python/names' `site_exported`
  reads the rest of a name's being public from the file itself (its `__all__`, its module-level defs and import-time
  imports: python/imports' `public_name` and `exported`'s first two cases), where the phase handed on `public(m, x)`
  for every module-level name, and each file asked for each of its own.
- `denotes(k, q) by_key 1`: what each ask denotes (`res`): itself, a definition through an import's exports, an
  attribute's definition of a class of the run.
- `resolved(k, q) by_key`: `m.x`, as a file spells it through an import, is module m's export x, which denotes q.
  It replaces `exports(m, x, q)`, read through `reads_at`: python/names' `export_at` and python/upgrade's `up_export`
  read it under the file's own ask.
- Each fact by its symbol in column 0, with the module that defines it: `class(q, m)`, `defined(q, m)`,
  `ancestor(q, m, b)`, `library_ancestor`, the `anc_*` and `member_*` facts, `plain_def`, and the
  Django facts (`dj_group`, `dj_hook`, `dj_receiver_fn`, `dj_task_def`, `dj_view_model`, …), each `by_key`.
- `dj_number_fields(q, m, x)`: class q's number fields, its own and its ancestors' (django-practice-bulk-not-loop's
  `bnl_number_field`, which read every member's call of every module the file reads, now reads those of the classes
  it names or defines).
- By the module, which `reads_at` names: `dj_model_class(m, q)`, what a rule asks of a class it knows by a proven
  type alone, which no file names. `dj_app_defers(b)`, a fact of the whole run, is handed on whole, not once under
  every module.

So a reader's entry is keyed by its asks and own definitions at its path, each ask's symbols, and the facts of those
symbols: the
engine's `by_key C` hop (`phased::Keyed::ByKey(Some(C))`, `file_local::setting_key_hop`) closes `SettingsDigests`'
keys through the `denotes` rows, and `settings_scopes` anchors a read keyed by a symbol a `denotes` row names. A def
nobody imports changes no ask's answer but its own file's, so it reruns no other reader; a changed answer reruns the
files whose asks reach it.

The readers read each fact under one of their asks or own definitions: python/names (`export_at`, rule 3,
`site_exported`, the `class_anc*` rules), python/django (`dj_model_ref`, the `dj_model_*` rules, `dj_receiver_def`, `dj_plain_at`,
`dj_task_waits_for_commit`, `dj_app_defers`), python/upgrade, and the Django pack (`bwh_in_hook`, `cfr_helper`,
`dco_field`, `ghw_view_model`, `tam_task_call`, `bnl_number_field`). `dj_number_field` moved to
python/django-shared.

`a_reader_is_keyed_by_the_answers_to_its_asks` locks the hop on a small phase: an edit to a fact no ask resolves to
builds its own file alone; one to a fact asks resolve to, through a re-export and directly, builds those files and
not a file that asks the same module for another name.

## Measured

The CLI at O0 with `test-hooks`, on this Mac with other runs going (load 14 to 53); user CPU and peak compared, not
wall time; each set run back to back. A reader's files "built" are those not read from its memo.

**The data layer's 224-file sample**, `bandit-b113-request-without-timeout` (python-main's, which reads python/names),
a cold run and then two edits on the same stores: A, a def nobody imports added to `_internal/native.py`; B,
`native_module` renamed there, which 25 files import.

| | Off | On |
|---|---|---|
| Rows handed on, cold | 17,326 | 7,603 |
| `asked_at` | 3,572 | 3,572 |
| `denotes` | 3,064 | 3,064 |
| `resolved` | 2,988 | 276 |
| `defined` | 6,091 | 8 |
| `class` | 832 | 37 |
| `reads_at` | 779 | 646 |
| The phase's rounds, cold | 1 | 2: 300 asks each; 224 files built, then 60 as partial hits |
| Edit A: the reader's files built | 1 | 1 |
| Edit B: the reader's files built | 29 | 25 |
| Cold run, user CPU | 12.6 s | 14.0 s |
| Edit A, user CPU | 0.93 s | 0.73 s |
| Edit B, user CPU | 4.49 s | 3.84 s |

`asked_at` is each file's asks of the symbols its nodes denote, read by its own file alone. Before `own_at` was an
export of its own, it held each file's own definitions and module-level names too: 17,570 rows on, 31,374 handed on
off and 21,601 on. b113 reads no fact of its own classes, so it is handed no `own_at` row.

**The PostHog sample's 291 files**, the Django pack's 27 policies in one run: HEAD's rules (module-keyed exports),
the switch off, and on; then edits on the same stores: A, a def nobody imports added to `posthog/models/user.py`; B,
a field added to `User`, which 76 files import. Each edit's findings equal a cold run's of the same tree.

| | HEAD's rules | Off | On |
|---|---|---|---|
| Findings, cold | 76 | 76, the same | 76, the same |
| Rows handed on, cold | 18,925 | 7,932 | 7,932 |
| Cold run, user CPU (load about 40) | 72.6 s | 81.1 s | 89.0 s |
| Edit A: files built, of the readers | | 1 | 1 |
| Edit B: files built, of the readers | | 76 | 76 |

HEAD's rules here are those before every export was keyed by the file's asks (4956a96757). Before `own_at` held
only classes, and before `site_exported` read a name's being public from its own file, the rows handed on were
30,074 off and 23,128 on.

The phase alone, on the Django pack's `django-practice-bulk-write-skips-hooks` over the same sample, back to back with
the previous commit's rules (5a85a02741): 3 rounds to 2, the phase's CPU 2.43 s to 2.03 s, and the run's
instructions 532 M to 479 M. `ans_pub`'s count over the file's assignments, made once for each ask, was 0.32 s of the
2.43; `all_here` and `all_assigned`, read once, make 133 rows. A first `own_at` filtered by the facts handed on
of each definition made every fact for every reader, whatever it read (0702): the phase took 3.14 s.

Before `dj_number_fields` was keyed by the class (it was keyed by the module, which `reads_at` names), edit B built
198 files off and 92 on: every file that reads posthog.models.user.

Findings, the switch off against on, over the data layer's 224-file sample: the same in each of the 18 data-layer
policies, with no crossing. The Django pack's 27 cases and the Python pack's 3 pass, with the switch off and on.

Round 2 builds the 60 asked files as partial hits: a chunk of them derives 5 of its 16 memoised relations, with 41
of the program's 238 rules (before `fwd` read `qn`, 206 of 225, and before a kept-route miss took a partial hit,
the files were built whole). The binding analysis is not built again: `wstart`'s work is 1.23 M, as with the
switch off, where the asked phase's was 1.81 M before.
