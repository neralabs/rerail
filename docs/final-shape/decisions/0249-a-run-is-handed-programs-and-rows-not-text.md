# 0249: A run is handed programs and rows, not text

Status: draft (2026-11-15). Step 4 of decision 0239's plan: rules and facts, like outputs, never pass through text
inside the engine. Agrees with the SQL index prototype's R23 (no SQL string interpolation; design index §22): code is
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

`policies::Logic::rules` is a `Program` (`wave_program`: node columns as the wave's integers). The wave joins
`SOURCE`, the policies' program and `KEPT`, in that order, as programs (`joined`): declarations, inputs, outputs,
rules after the ones before them, facts. `SOURCE` and `KEPT` are the engine's own constant texts, read once for
each wave (§8). The compiled plan is keyed by the joined program's structural digest, not by its text.

### 5. A handed program is keyed by itself

`run_read` takes the program alone. Its key, in the place of a text, is `rerail.read-program <digest>`
(`read_key`): `plan::choices::key`, which digests a whole program by its structure (its rules structurally, every
other field by value), named so that it reads as no program. `Texts::of_read` holds it: no graph to derive (the
program includes nothing; its libraries are in it), and its names (`rr::TextNames::of_program`: the relations it
declares, the phase it names, the engine's library programs) as a print of it had them. Whether it reads the
components (`components.rr`) is asked of its settings and relations, not searched for in a text. The constants' run
of it is handed the same program (`constants::derive`, `run_handed`). `tools::run_program` no longer prints the
program it read.

### 6. A finding is a row of typed columns

A finding was one `rr_diagnostic` row: a JSON text the rules concatenated (`json_str`, `to_string`, `concat`),
which the CLI parsed with serde. Each `.violation` (and each error directive) now has its own relation,
`rr_diagnostic_<digest of policy, clause and form>` (`rr::lower::findings`, `is_diagnostic`). A finding's node form
and its `@place` form are two relations, since their `at` columns differ in type. A row describes itself:

| columns | type |
|---|---|
| `policy`, `clause`, `code`, `path` | text (a code from a non-text column: its text) |
| `line` | number |
| `at` | the finding's node (a node), or `0` (a place's) |
| `message` | text |
| `tagged`, then `tag` when it is 1 | number, then the tag column's type |
| `name<i>`, `value<i>` for each parameter | text, then the parameter's own type |

A reader needs no program. `tools::diagnostics` makes the CLI's JSON object of each row from its values: a text is a
string, an integer a number, a node its value as the run held it (`Row::raw`), NULL `null`. These are the values the
JSON text held. A pack's programs share the relations, as they share `violation` (`corpus::own_names`): each is
named by its policy. `run_pack` gives each member the rows whose `policy` is its own.

Since decision 0283 (the merge into the hub), the rule leaves `message` empty and ends the row with the message's
holes (`findings::HOLE`); the writer composes `message` from the `.message` template (decision 0225 §3) and writes
the row as the table above has it.

### 7. Phase programs and the typing bridge are handed

A phase program is read once from its file (`phases::source`, its includes read in) and handed to its run
(`run_read`): its exports pushed onto its outputs and, for phase 1, the run's language pushed onto its settings, as
the `.output` and `.language` lines appended to its text once were. The typing bridge's `typing_bridge_at` is
declared and its rules (one for each node column of each bridged output) are added to the program read
(`bridge::add_spans`), reading `node` as lowering would have it (`rr::lower::base_relation`). Neither is lowered
a second time.

### 8. The wave's programs are made once, keyed by structure

`Wave::with_logic` reads `SOURCE` and `KEPT` and joins them with the policies' rules into its two programs, cold
and warm (`Wave::sources`), each keyed by `plan::choices::key`, the run's structural digest of a program. No
process-wide memo of them is kept, and no key is a print of a program. `joined` takes every declaration-like
field of the program it adds (declarations, nullary relations, lattices, kind sets), as reading the texts one after
the other did.

### 9. A handed program's cutoff reads the program

The whole-run cutoff asks whether a run would read alike of the program it was handed (`corpus::changes`, its
`read`), never of the handed program's key as a text. `rr::tests::a_handed_program_run_again_finds_the_same` runs
a policy twice with a store: the second run, over an unchanged root, finds what the first did.

## Still text

- JSON policies (decision 0175). A JSON policy is run as the `.rr` text it translates to (`json_rules::Translator`,
  `rr::json_program_shared`, `settings::program_texts`), which is then parsed. Handing its statements instead is a
  change to 0175, open.
- `test_store`'s template, which prints a parsed program to warm the store as the tests' printed programs do.

External inputs stay text: the user's `.rr` files, `.input` files, the typing declaration files.

## Tests

- `added_rows_go_to_the_programs_that_declare_them_and_key_by_value` (`corpus::added`): rows to a program that
  declares the relation and none to one that does not; no rows, no relation; keys by every value and its type; the
  dialect set as a program's language and keyed.
- `settings_become_env_rows_for_a_program_that_declares_env` (`tools`): `rerail.env` and `--env` read, and given
  as rows.
- The waves' `merged` test checks that the wave program holds the merged rules and facts as they are.
- `lowered_findings` (rr tests) and the pack test (`tape_tests`) read the findings through `tools::diagnostics`; the
  tests of `run-rr` compare the JSON it reports, as before.
- The typing tests (`typing::oracle_tests`, `plan::tests`) run with `declared_ret` rows. The rr tests'
  `a_language_of_several_dialects_runs_over_each` runs every dialect through the option.
