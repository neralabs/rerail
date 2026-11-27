# Batch pt1120-python-06

Eight rules of the `python` pack, checked with `translations:check` on `tbelanger/pack-translations-1120`
(`ptcheck`, the 1120 engine). Ground truth: Bandit 1.9.4 and Ruff 0.17.6 on a fleet box, run over the whole
`fixtures` corpus and `samples-python`, and compared key by key with the translation run alone (`run-policies`).
After the changes below, every translation reports exactly what the tool reports on both corpora.

| Rule | Status | Right | What differs from the JSON |
|---|---|---|---|
| bandit-b103-set-bad-file-permissions | same | | |
| bandit-b110-try-except-pass | different | translation | 9 handlers the JSON misses |
| bandit-b405-import-xml-etree | different | translation | 5 imports spelled with continuations or spaces |
| bandit-b406-import-xml-sax | different | translation | `from xml.sax.saxutils import …` |
| ruff-air001-airflow-variable-name-task-id-mismatch | same | | |
| ruff-air302-airflow3-moved-to-provider | same | | |
| ruff-air311-airflow3-suggested-update | same | | |
| ruff-air312-airflow3-suggested-to-move-to-provider | same | | |

## Rules

- **B103.** Bandit's `call_args_count` counts `ast` arguments. Tree-sitter puts a `line_continuation` node among
  an argument list's children, and `python/security`'s `in_call_args` counted it as an argument. Bandit says:
  - `os.chmod("f", \` / `755)` is B103 (two arguments). The translation missed it: it counted three.
  - `Path("x").chmod(\` / `777)` is not B103 (one argument). The translation reported it: it counted two.

  The rule now counts and indexes positional arguments without continuations (`chmod_arg`, `chmod_args`). Both
  the JSON and Bandit agree with it now.
- **B110.** Kept. The translation matches Bandit on all 56 fixture findings and all 36 in samples-python. The
  JSON's patterns miss a handler with a comment on its `except` line (`except:  # noqa`, `ruff/rerail/noqa/E722.py`
  lines 8, 13, 18 and 23) and `ruff/flake8_bugbear/B012.py:22` (`except Exception: pass` in a `finally`). Bandit
  reports all of them.
- **B405.** Bandit reads an imported name from `ast`, which drops spaces and line continuations:
  - `import xml . etree . ElementTree as ET` is B405.
  - `import xml.etree\` / `.ElementTree as ET` is B405 (`N817-import-convention-near-misses.py` lines 1–3,
    `UP023-line-continuation-in-dotted-name.py` lines 1, 3, 5 and 7).

  `python/security`'s `import_name` takes the dotted name's text as written, so the translation missed all seven.
  The JSON's regex allows spaces but not continuations: it found lines 1 and 2 of N817, not the other five. The
  rule now strips whitespace and backslashes from each import entry before Bandit's prefix test (`b405_hit`).
  Calls still go through `blacklisted`. Kept against the JSON on those 5 keys.
- **B406.** Kept. `from xml.sax.saxutils import XMLGenerator` (django `utils/xmlutils.py:5` in samples-python)
  is B406 in Bandit, because "xml.sax.saxutils" starts with "xml.sax". The JSON exempts saxutils (NOTES-py-bandit
  D7). The same normalisation as B405 is applied (`import xml . sax` is B406). No fixture changes.
- **AIR001.** Ruff's AST has no parentheses, so `g = (airflow.providers.google).operators.bq.Op(task_id="h")` is
  AIR001 (`AIR001-provider-chain-comments.py` lines 9 and 10). `python/ruff`'s `ruff_qual` reads through
  parentheses only on an attribute's immediate object. The rule now resolves the callee with `air_callq` (below),
  which walks the whole chain through parentheses. It also drops `operator_class(q) :- call_q(_, q), …`: that was a
  table of every file's callee names (decision 0677), and its test is now made where the call is.
- **AIR302, AIR311, AIR312.** Three gaps in `python/misc-airflow`, all in Ruff's favour. Every case below was run
  through Ruff.
  1. *Which import a read sees.* `ruff_qual` (through `qn`) takes every import of the name in the scope Python's
     lookup ends in, wherever it sits. Ruff's walk takes the binding it has reached, in the innermost scope that
     has one by then. Module and class bodies are visited in order. Function and lambda bodies are visited once
     the module is done, so they see the enclosing scope's last binding. Examples:
     - `from airflow.operators.python import PythonOperator`, later
       `from airflow.providers.standard.operators.python import PythonOperator`, then `PythonOperator()`: no AIR312.
       The translation reported 12 such reads in `AIR301_args.py` and `AIR302_standard.py`.
     - The same with `from airflow.decorators import task` then `from airflow.sdk import task`: no AIR311
       (`AIR301_args.py` 295–303, and `AIR303.py` 135–173 for `BaseOperatorLink`).
     - `from airflow.providers.google.cloud import hooks as operators` after `… import operators`, then
       `operators.FooOperator(sla=1)`: no AIR311 (`AIR311-sla-callee-forms.py:16`).
     - A scope with no binding yet is passed over. In `class K:`, `BashOperator()` above the class's own
       `from … import BashOperator` reads the module's import: AIR312. So does `def h(x=BashOperator()):` whose
       body binds `BashOperator` later.
  2. *String annotations.* Ruff reads `def f(a: "S3Hook")` (AIR302 line 28 of `AIR302-exact-replacement-guard.py`,
     AIR312 line 19 of `AIR312-forward-reference-guards.py`). `air_ref` tested `seen_module` on the guest's own
     document, which imports nothing, so no string annotation was ever reported. Ruff also looks a string
     annotation's names up in the **module** scope, even inside a function body. For example,
     `hint: "PythonOperator" = PythonOperator()` in `load()` (line 15): the string is AIR312, because it reads the
     module's unguarded import, while the call reads `load`'s guarded fallback.
     - Probe that shows it: a module-level guarded import with a function-local unguarded one leaves
       `c: "PythonOperator"` in the function quiet.
     - With only a function-local import, Ruff resolves nothing.
  3. *Handler types.* Ruff's `Exceptions::from_try_stmt` reads a handler's Name, or a tuple's elements. Only a flat
     tuple counts. `except (ValueError, (ImportError,)):` is not an ImportError handler, so
     `provide_bucket_name()` (line 12) is AIR302. `air_caught` recursed into the nested tuple.

  New library `lib/python/pt1120-python-06.rerail`, shared by the four Airflow rules. It provides `air_q` (the
  binding Ruff sees, through parentheses), `air_at`, `air_callq`, `air_dag_at`, `air_kind_at`, and `air_safe` (the
  guard for that binding, with flat handler types). The guard is still asked through misc-airflow's
  `air_needs_guard`. The library is per file: every relation is keyed by node, and no name table crosses (run with
  `RERAIL_STRICT_CROSSINGS`, `RERAIL_STRICT_NODES` and `RERAIL_STRICT_POSITIONS`: no refusal).

  Not modelled, none in the corpora:
  - A non-string annotation that Ruff defers under `from __future__ import annotations` is read here in order.
  - A string annotation inside a nested function is looked up in the module, as Ruff does for a function at
    module level. Ruff was not probed deeper.

## Library changes proposed

- `lib/python/security.rerail`:
  - `in_call_args` should leave out `line_continuation` (`k != "line_continuation"`). B103 then goes back to
    `arg_count` and `pos_arg`. Every Bandit rule that reads positional arguments has the same gap.
  - `import_name` should strip whitespace and backslashes from the dotted name, as `ast` reads it
    (`@replace_all(t, "[\\s\\\\]+", "")`). B405 and B406 then go back to `blacklisted(n, code)`. Every B4xx rule
    has the same gap. The workaround stays in the two rules, not in a shared file: a new library that includes
    `python/security` beside this batch's Airflow library would load the Airflow stack into the Bandit rules.
- `lib/python/misc-airflow.rerail`: `air_ref`, `air_call`, `air_kind`, `air_dag` and `air_guarded` should be
  replaced by this batch's file. AIR301 and AIR303 read them too, and have the same three gaps.
- `lib/python/ruff.rerail`: the `ruff_qual` clause for a parenthesized object should recurse through any
  attribute chain, not only the immediate object (`air_tail` here walks the whole chain). The flow-sensitive
  binding (`air_cur`) does not depend on Airflow and could move to `python/scopes` as `binding_at(r, b)`.

## Engine gaps

None. Every rule runs as written.
