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

- `.rounds R`: the phase runs again while `R` grows. Each round reads the last round's rows of `R` as `<phase>__R`,
  declared with `R`'s columns. `R` is an export declared `by_path`.
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

The other exports (`reads_at`'s first rule, classes, definitions, ancestors, members, the Django facts) are made as
`py_names.rerail` makes them.

## Measured

The CLI at O0 with `test-hooks`, on this Mac with other runs going, each run with its own stores; user CPU and peak
compared, not wall time.

Findings, the switch off against on, over the data layer's 224-file sample: the same in each of the 18 data-layer
policies, with no crossing. The Django pack's 27 rules' cases and the Python pack's 3 pass with the switch on.

`bandit-b113-request-without-timeout` over the 224 files (`RERAIL_PROFILE=1`):

| | Off | On |
|---|---|---|
| The phase's work (the profile's heads, summed) | 13,571,526 | 15,289,136 |
| The phase's CPU (its heads) | 2.13 s | 3.07 s |
| Its runs | 1 | 2 rounds |
| `reads_at`'s work | 29,031 | 15,169 |
| Cold run, user CPU | 12.2 s | 14.6 s |
| Cold run, peak | 682 MB | 688 MB |
| After a one-file edit (a new top-level name), user CPU | 0.99 s | 0.77 s |
| After that edit, the phase's rounds | 1 | 1 |

Cold, the asked phase costs more. Round 2 reads 164 of the 224 files from the memo and builds the other 60 again
whole, their python/scopes analysis with them, though only their answers read `ask_in`: no partial hit (decision
0225 §2) is taken. The asks are many: `asked` made 93,699 rows and `ask_in` 11,002 over the two rounds, since a
file's `qn` names include every attribute chain through an import and every module outside the workspace, where
the step's estimate counted 227 pairs from import lines alone. After an edit, the run starts from the kept asks and
takes one round.

## Not yet done

- The other exports of `py_names` are not asked: a class's members by an attribute chain, its ancestors by its
  bases, a Django model's options by the model a file names.
- Round 2 builds an asked file again whole. Its entry should be a partial hit, deriving again only the relations that
  read `ask_in`; why it is not is not found yet.
- The asks should be only the names that can be a workspace module's: `asked` sends a name of an outside module
  nowhere, but makes its row.
- The rows each export hands on are reported per phase (the profile's "phase NAME: handed on"); they are not
  measured here yet.
