# 0249: A run is handed programs and rows, not text

Status: draft (2026-11-15). Step 4 of decision 0239's plan: rules and facts, like outputs, never pass through text
inside the engine. Agrees with the ast-index-spike's R23 (no SQL string interpolation; design index §22): code is
built as structures, not by gluing strings.

## Intro

A user's `.rr` file is text, and the engine reads it once. After that the engine is meant to hold programs as
`front::Program` and facts as rows. A survey after 0239 stage 6 found five places where the engine printed what it
had made back into rule text, and then read that text again:

| where | what was printed | read again by |
|---|---|---|
| the run's settings (`settings::with_settings`) | `env("k", "v").` lines appended to every program text | `rr::parse` of the whole text |
| the typing declarations (`corpus::typing::facts`) | `declared_ret("q", "t").` lines | the whole program combined again from text |
| `tools::run_program`, another dialect | the program read, its `.language` changed, printed | `corpus::run_with_options` of that text |
| `tools::logic`, another dialect | the program's `.language` line rewritten in its text | the same |
| `tools::check_rr` | the program read, printed | `corpus::combine` of that text |
| the waves (`policies::wave_text`) | the policies' rules, printed | `rr::parse` of the wave's source with them appended |

Text added to a program also made it another program. A program read from a file (`run_read`) could not be used
as it was once the settings added a line, so it was lowered a second time.

## Decision

### 1. What a run adds is rows (`corpus::added::Added`)

The run's settings are `env(key, value)` rows. The typing declarations are `declared_ret(q, t)` rows. Each program
that declares the relation is given the rows once it is read (`Added::add_to`, in `combine_read`), whichever way it
was read: from a text, from its read text (`rr::Text::lower`), with its libraries' declarations only
(`parse_own`), or handed already read (`run_read`). Once the settings are rows, a handed program stays the
program: it is never lowered again.

A program that declares none of the relations is unchanged, as with the text. A program in module form reads
`core.rr`, which declares `env`.

### 2. The dialect is a setting the run sets

A program of a language of several dialects runs once per dialect. The run is given the dialect as the `dialect`
option. `Added::add_to` sets the program's `language` setting to it, when the program names one. `run_program`
hands the program it read to every dialect's run (`run_read`). `logic` hands each text as written. Neither prints
nor edits text.

The token refusal (`rr::token_reads::refuse`) was never run for the printed dialect texts (`rr::parse` does not run
it), and it is not added: this decision changes no requirement.

### 3. Keys are of the rows

Everything a run kept by its text's digest now keeps by the text and the added rows (`Added::hash`):

- the combined program (`texts_digest`, and the process and store caches under it);
- the manifest's lane (`rerail.corpus-lane/v2`);
- the prepared program's stored source;
- the whole-run record (`rerail.run-reuse/v4`, `reuse::settings`);
- the constants' record (`rerail.constants/v4`).

A run with nothing added hashes as before, except that every key's version moved.

### 4. The waves join programs

`policies::Logic::rules` is a `Program` (`wave_program`: node columns as the wave's integers). `matches::inputs`
joins `SOURCE`, the policies' program and `KEPT`, in that order, as programs (`joined`): declarations, inputs,
outputs, rules after the ones before them, facts. `SOURCE` and `KEPT` are the engine's own constant texts, parsed
once per process. The compiled plan is keyed by a digest of the joined program, not by its text.

## Still text

- A read program's key text. `run_read` is keyed by `rr::print` of the program it is handed. It is never parsed into
  the run's program, but phase 0's `Texts::of` tokenizes it for includes and names. The next step keys a handed
  program by a digest of the program itself (`plan::choices::key` digests a whole program by its structure),
  gives `Texts` its names from the program, and hands the program to the constants' run.
- Phase programs. `phases` appends `.output …` (and phase 1's `.language …`) to a phase program's source before
  running it.
- `test_store`'s template, which prints a parsed program to warm the store as the tests' printed programs do.

External inputs stay text: the user's `.rr` files, `.input` files, JSON policies, the typing declaration files.

## Tests

- `added_rows_go_to_the_programs_that_declare_them_and_key_by_value` (`corpus::added`): rows to a program that
  declares the relation and none to one that does not; no rows, no relation; keys by every value and its type; the
  dialect set as a program's language and keyed.
- `settings_become_env_rows_for_a_program_that_declares_env` (`tools`): `rerail.env` and `--env` read, and given
  as rows.
- The waves' `merged` test checks that the wave program holds the merged rules and facts as they are.
- The typing tests (`typing::oracle_tests`, `plan::tests`) run with `declared_ret` rows. The rr tests'
  `a_language_of_several_dialects_runs_over_each` runs every dialect through the option.
