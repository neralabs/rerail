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
- `by_path`, `by_path C`, `by_key` after an export: how a reader's memo entry is keyed by it. `by_path`: the rows at
  the file's own path (column 0). `by_path C`: those rows, and every row of a `by_key` export under each key named in
  their column `C`. `by_key`: rows keyed by column 0, read where a `by_path` row names the key. Key columns are
  symbols. An export with a key is a setting to the split (0665): placeless, read per file.
- `R as E` hands on relation `R` as export `E` (`asked_exports as exports by_key`): an asked phase answers under the
  name its readers already read, where an included library makes a relation of that name eagerly.
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
(python/names, python/django, python/upgrade), but `exports` holds only what is asked:

- `asked_at(p, k)`: each qualified name the file at `p` spells through an import (`qn`, "m.x").
- `asked(m, x)` and `ask_in(p, x)`: the cross part sends each ask to the file of module `m`, and forwards it through
  that module's re-exports and star imports.
- In the next round, the asked file answers at its own path: a top-level definition (`ans_def`), a re-export
  (`fwd`), a star import (`fwd_star`), and whether the name is public there (`ans_pub`). These are python/imports'
  three rules of `exports` and `exported`, for the asked names alone.
- `asked_exports`, handed on as `exports`, follows the answers through the forwards, as `exports` and
  `resolve_export` do. The phase's own lookups (bases, members' calls and references, Django's receivers and
  managers) read it in place of `exports`, and `reads_at`'s second rule takes the modules the file's own asks
  resolve to, not every export of every module it names.

`class` and `defined` are answered for the symbols the files may name:

- `sym_at(p, q)`: each symbol the file at `p` may give a node, from its own tree: python/scopes' `sym_of`, `qn`, and
  an attribute of either at any depth. `obj_at` and `attr_at` are the same of an attribute's object and of an
  attribute, where python/names' `sym_of` rule 3 reads `class` and `defined`.
- `res(k, q)`: what the symbol denotes, as the readers resolve it: itself, an import through the asked exports (rule
  1), a definition `c.a` of the run where `c` is what its prefix denotes (rule 3; bounded by a split of a `defined`
  row, since a made `@dot` is refused as an unbounded value).
- `asked_class`, `asked_defined` and `asked_named`, handed on as `class`, `defined` and `class_named`: the classes an
  object may denote, the definitions an attribute may, and the classes any node may (python/django's
  `dj_model_ref`). They are read in the cross part, where the phase's own rules (`reads_at`, the bases, the members)
  read every file's `class` and `defined` already, so no round asks a file for them. `.rounds` takes several
  relations (`.rounds R, S`), but `py_names` needs only `ask_in`.
- `dj_model_ref`'s second rule, a class of the reading file itself, reads the file's own module, not a `class` row:
  so a file asks nothing of its own classes. `py_names.rerail` hands on `class_named` as `class`.

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

`asked` makes a row only for a module of the workspace: a name of an outside package, or an attribute chain past a
module, is asked of no file.

The other exports (`reads_at`'s first rule, ancestors, members, the Django facts) are made as `py_names.rerail`
makes them.

## Measured

The CLI at O0 with `test-hooks`, on this Mac with other runs going (load 24 to 44); user CPU and peak compared, not
wall time; each pair run back to back.

Findings, the switch off against on, over the data layer's 224-file sample: the same in each of the 18 data-layer
policies, with no crossing. The Django pack's 27 cases and the Python pack's 3 pass, with the switch off and on.

`bandit-b113-request-without-timeout` over the 224 files (`RERAIL_PROFILE=1`), a cold run and then one after a
one-file edit (a new top-level name in `data_layer/batching.py`), on the same stores, at load 27:

| | Off | On |
|---|---|---|
| Rows handed on (the profile's "phase py_names: handed on") | 23,330 | 967 |
| `exports` | 15,628 | 276 |
| `reads_at` | 779 | 646 |
| `defined` | 6,091 | 8 |
| `class` | 832 | 37 |
| The phase's rounds, cold | 1 | 2: 300 asks each; 224 files built, then 60 as partial hits |
| After the edit | 1 round | 1 round: 1 file built |
| The phase's work (its heads), cold | 13,609,444 | 13,324,438 |
| The phase's CPU (its heads), cold | 3.34 s | 3.18 s |
| Cold run, user CPU | 15.3 s | 13.6 s |
| After the edit, user CPU | 0.78 s | 0.62 s |
| Cold run, peak | 657 MB | 669 MB |

Round 2 builds the 60 asked files as partial hits: a chunk of them derives 5 of its 16 memoised relations, with 41
of the program's 238 rules (before `fwd` read `qn`, 206 of 225, and before a kept-route miss took a partial hit,
the files were built whole). The binding analysis is not built again: `wstart`'s work is 1.23 M, as with the
switch off, where the asked phase's was 1.81 M before.

## Not yet done

- The other exports of `py_names` are not asked: a class's members by an attribute chain, its ancestors by its
  bases, a Django model's options by the model a file names, and `reads_at`'s first rule (the modules a file names,
  and those above and below them).
