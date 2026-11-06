# py-misc1: 52 Ruff rules (Airflow AIR, flake8-logging-format G, flake8-logging LOG, flake8-datetimez DTZ, pandas-vet PD, flake8-boolean-trap FBT, flake8-debugger T10, flake8-print T20, flynt FLY, flake8-implicit-str-concat ISC, eradicate ERA, flake8-fixme FIX, flake8-commas COM, flake8-tidy-imports TID), hand-written

Every rule follows Ruff 0.16.9, read from its source at the commit the JSON was written from
(`wolfeel/.rerail/port-readiness/upstream/ruff/0be08a2…/crates/ruff_linter`). That means the rule function, the
place in `checkers/ast/analyze` that calls it, and the helpers it uses. Where the JSON says less, or says
something else, the rule follows Ruff. Messages are the JSON's, with `$X` placeholders turned into fields.

## Libraries

| file | lines | what |
|---|---|---|
| `lib/python/misc-seen.rr` | 19 | `seen_module(d, m)`: document d imports m or a module under it (Ruff's `seen_module`), for the modules a library asks about with `seen_ask(m)` |
| `lib/python/logging.rr` | 83 | logging calls (`log_call`) on a logger (Ruff's `is_logger_candidate`: inline `getLogger`, qualified names, the `logger-objects` option, the name heuristic, which skips a class or def defined in the file) or by a bare `logging.x` name; the message argument, keywords, `exc_info`, "in an except handler" |
| `lib/python/datetime.rr` | 43 | datetime calls in a file that imports datetime; `naive_tz` (the tz argument is missing or `None`, by position or keyword); `then_astimezone` (`.replace(…)` chains ending in `.astimezone`) |
| `lib/python/misc-boolean-trap.rr` | 32 | FBT's positional parameters and the defs it exempts (dunder operators, `__post_init__`, setters, `@override`, stubs) |
| `lib/python/misc-comments.rr` | 19 | `fixme_tag(c, t, d)`: Ruff's TodoDirective, the first task tag right after a `#` |
| `lib/python/misc-airflow.rr` | 93 | `air_ref` (Airflow names read by qualified name, at the name or the attribute's last part), `air_kw` (first keyword of a call), `air_call`, `air_dag` (shared by AIR002, AIR301 and AIR311), `air_kind(q, m, s)` (Ruff's `is_airflow_builtin_or_provider`, one regex for every rule), `air_value` / `air_instance` (Ruff's `resolve_assignment`: the binding current at the read; shared by AIR301 and AIR303), `air_task` / `air_in_task`, `air_guarded` (Ruff's `is_guarded_by_try_except`: an `except ImportError` fallback import, or an `except AttributeError` attribute, whose try reads the new name) |

Data tables, generated from the JSON (`gen-<id>.py` writes, `check-<id>.py` round-trips; each prints `0 differences`):

| table | rows | from |
|---|---|---|
| `ruff-air301-airflow3-removal.names.rr` | 76 | the name clauses' `old-name` lists, prefix flags, `new-import` guards and messages |
| `ruff-air302-airflow3-moved-to-provider.moved.rr` | 182 | each clause's `old` list, plus the provider, version, module and name parsed from its message |
| `ruff-air312-airflow3-suggested-to-move-to-provider.moved.rr` | 31 | the `deprecated` list. The new module is the old one under `airflow.providers.standard`, with Ruff's two exceptions (`sensor.bash`, sic, and `sensors.time`). The check compares the old names with the JSON and every row with the arms of Ruff's `suggested_to_move_to_provider_in_3.rs` (`RUFF_SRC` overrides its path): 31 names, 31 arms, 0 differences |

The 52 rules have 1,536 lines, comments and directives included. Most are 16 to 60 lines. AIR311 has 64,
because its name table is inline. ERA001 has 91, for its grammar (E1). AIR301 has 176, above the 130 aimed
for: that one Ruff rule has eight checks (names, call arguments, instance methods and attributes, context
keys, context parameters, `@apply_defaults`, plugin extensions, `trigger_rule`). About 60 of its lines are
tables copied from Ruff's source: the removed keywords by callee class, the exact method and attribute
renames (`air301_exact`, Ruff's per-class arms), the class-family renames (`air301_family_row`), and the
context-key hints (`air301_hint`). They are inline because the JSON does not carry them, so no gen- script
can write them.

## Scores

I ran Ruff 0.16.9's own fixtures against its snapshots (`tests__<rule>_<file>.snap`) with `rrlab run` and
compared them at (path, line). I used every snapshot of a rule, including option variants (G with
`logger-objects`, FBT003 with `extend-allowed-calls`, ISC003 with `allow-multiline = false`, and TID252 with
`parents`, `all` and package roots). Only T100 has a recording in `packs/python`: `rrlab score python
ruff-t100-debugger --all` gives 27 / 0 / 0, at both line and column. `rrlab score` finds no recording for
the other 51 rules.

| rule | lines | snapshots TP / FP / FN | notes |
|---|---|---|---|
| ruff-air001-airflow-variable-name-task-id-mismatch | 25 | 3 / 0 / 0 | |
| ruff-air002-airflow-dag-no-schedule-argument | 20 | 2 / 0 / 0 | |
| ruff-air301-airflow3-removal | 176 | 198 / 0 / 0 | A1 A2 A3 A4 |
| ruff-air302-airflow3-moved-to-provider | 27 | 170 / 0 / 0 | A1 A2 |
| ruff-air303-airflow3-incompatible-function-signature | 51 | 33 / 0 / 0 | A3 |
| ruff-air311-airflow3-suggested-update | 64 | 41 / 0 / 0 | A1 A2 |
| ruff-air312-airflow3-suggested-to-move-to-provider | 25 | 34 / 0 / 0 | A1 A2 |
| ruff-g001-logging-string-format | 19 | 15 / 0 / 0 | S1 |
| ruff-g002-logging-percent-format | 18 | 4 / 0 / 0 | S1 |
| ruff-g003-logging-string-concat | 17 | 4 / 0 / 0 | S1 |
| ruff-g004-logging-f-string | 19 | 25 / 0 / 0 | S1 |
| ruff-g010-logging-warn | 19 | 4 / 0 / 0 | S1; safe fix `warn` → `warning` |
| ruff-g101-logging-extra-attr-clash | 29 | 4 / 0 / 0 | S1 |
| ruff-g201-logging-exc-info | 17 | 4 / 0 / 0 | S1 |
| ruff-g202-logging-redundant-exc-info | 16 | 4 / 0 / 0 | S1 |
| ruff-log001-direct-logger-instantiation | 17 | 2 / 0 / 0 | S1 |
| ruff-log002-invalid-get-logger-argument | 23 | 4 / 0 / 0 | S1 |
| ruff-log004-log-exception-outside-except-handler | 18 | 7 / 0 / 0 | S1 |
| ruff-log007-exception-without-exc-info | 19 | 5 / 0 / 0 | S1; T1 |
| ruff-log009-undocumented-warn | 18 | 2 / 0 / 0 | S1 |
| ruff-log014-exc-info-outside-except-handler | 22 | 7 / 0 / 0 | S1; T1 |
| ruff-log015-root-logger-call | 18 | 16 / 0 / 0 | S1 |
| ruff-dtz001-call-datetime-without-tzinfo | 23 | 5 / 0 / 0 | S1 |
| ruff-dtz002-call-datetime-today | 17 | 2 / 0 / 0 | S1 |
| ruff-dtz003-call-datetime-utcnow | 17 | 2 / 0 / 0 | S1 |
| ruff-dtz004-call-datetime-utcfromtimestamp | 17 | 2 / 0 / 0 | S1 |
| ruff-dtz005-call-datetime-now-without-tzinfo | 24 | 10 / 0 / 0 | S1 |
| ruff-dtz006-call-datetime-fromtimestamp | 23 | 5 / 0 / 0 | S1 |
| ruff-dtz007-call-datetime-strptime-without-zone | 40 | 5 / 0 / 0 | S1; an f-string's `%z` counts only in its literal parts, as in Ruff |
| ruff-dtz011-call-date-today | 16 | 2 / 0 / 0 | S1 |
| ruff-dtz012-call-date-fromtimestamp | 16 | 2 / 0 / 0 | S1 |
| ruff-dtz901-datetime-min-max | 25 | 8 / 0 / 0 | S1 |
| ruff-pd002-pandas-use-of-inplace-argument | 36 | 11 / 0 / 0 | the snapshot plus inline snippets |
| ruff-pd012-pandas-use-of-dot-read-table | 20 | 4 / 0 / 0 | |
| ruff-pd101-pandas-nunique-constant-series-check | 44 | 12 / 0 / 0 | N2 |
| ruff-fbt001-boolean-type-hint-positional-argument | 49 | 11 / 0 / 0 | N3 |
| ruff-fbt002-boolean-default-value-positional-argument | 18 | 4 / 0 / 0 | |
| ruff-fbt003-boolean-positional-value-in-call | 40 | 9 / 0 / 0 | |
| ruff-t100-debugger | 43 | 25 / 0 / 0 | recording 27 / 0 / 0; relative imports (`from . import pdb`, `from .pdb import set_trace`) count, as in Ruff |
| ruff-t201-print | 22 | 4 / 0 / 0 | |
| ruff-t203-p-print | 24 | 8 / 0 / 0 | |
| ruff-fly002-static-join-to-f-string | 60 | 10 / 0 / 0 | |
| ruff-isc001-single-line-implicit-string-concatenation | 21 | 25 / 0 / 0 | P1 |
| ruff-isc003-explicit-string-concatenation | 32 | 29 / 0 / 0 | P1 |
| ruff-isc004-implicit-string-concatenation-in-collection-literal | 24 | 6 / 0 / 0 | P1; also a subscript tuple, `d["a" "b", 1]` |
| ruff-era001-commented-out-code | 91 | 19 / 3 / 0 | E1 E2 |
| ruff-fix001-line-contains-fixme | 17 | 2 / 0 / 0 | C1 |
| ruff-fix002-line-contains-todo | 17 | 3 / 0 / 0 | C1 |
| ruff-fix003-line-contains-xxx | 17 | 2 / 0 / 0 | C1 |
| ruff-fix004-line-contains-hack | 17 | 2 / 0 / 0 | C1 |
| ruff-com818-trailing-comma-on-bare-tuple | 24 | 7 / 0 / 0 | |
| ruff-tid252-relative-imports | 25 | 35 / 0 / 0 | options `ban-relative-imports`: "parents" (default) or "all" |

Total: 859 TP, 3 FP and 0 FN over the snapshots. The 3 FPs are E2, an rrlab limit. Ruff 0.16.9's
unit cases for `comment_contains_code` (144 cases in `eradicate/detection.rs`) all agree. On about 400 Python
fixtures copied from Ruff's test tree, ERA001 here and the installed Ruff 0.15.7 differ only by E2 (inline
and file-level noqa) and one 0.15.7-vs-0.16.9 `ruff: ignore[…]` difference.

The reviewer's cases (AIR301 `DatasetManager` methods, AIR303 rebinding, ERA001 unbalanced lines, ISC004
subscript tuples, LOG007/LOG014 values, T100 relative imports, DTZ007 interpolations, G010 local classes) each
match Ruff 0.15.7 on the same file.

RUF rules that read `truth_of` (T1), rescored on their snapshots after the change: RUF008 9 / 0 / 0, RUF009
24 / 0 / 0, RUF012 17 / 0 / 0, RUF033 19 / 0 / 0, RUF049 7 / 0 / 0. None changed.

Options this batch reads (`option_item`): `logger-objects` (G, LOG), `flake8-boolean-trap.extend-allowed-calls`,
`flake8-implicit-str-concat.allow-multiline`, `flake8-tidy-imports.ban-relative-imports`.

### Deviations

- **S1. `seen_module` is whole-file.** Ruff's `seen_module` is set in visit order, so a call before the
  file's `import logging` or `import datetime` is not checked. Here any import in the file counts. The
  same goes for `airflow` in every AIR rule.
- **N1. Names resolve without flow.** names.rr's `qn` does not follow flow: a later import binding also counts
  for a read before it. This shows only in contrived code.
- **N2. PD101 reads any binding.** Its name resolution takes any binding of the receiver, where Ruff takes
  the current one.
- **N3. FBT001's `bool_rebound` is approximated at module level.** This check asks whether `bool` is rebound
  so that `bool` is no longer the builtin.
- **C1. FIX reports at the comment.** Ruff reports at the tag inside the comment. The line is the same and
  only the column differs. `fixme_tag` treats letters and digits as ASCII only.
- **P1. ISC syntax-error fixtures are not scored.** The ISC fixtures with deliberate syntax errors are not
  scored, because the engine withholds documents with syntax errors.
- **A1. String annotations are not read.** Ruff also reads names in string annotations (`x: "S3Hook"`). The
  engine's `string_annotation` relation would give them, but rrlab does not model it, so the AIR rules do
  not use it. No snapshot row needs it.
- **A2. Import fixes are omitted.** Ruff's import-rewriting fixes for AIR301/302/311/312 are not offered:
  they add an import and rename uses. AIR301's keyword renames, AIR301's `trigger_rule` value and AIR311's
  `datasets=` → `assets=` are offered as safe fixes, as in Ruff. AIR301's message for its two prefix entries
  (`airflow.contrib`, `airflow.operators.subdag`) names the prefix, not the full name read.
- **A3. Only direct base classes count.** Ruff's `any_qualified_base_class` follows base classes
  transitively; here only direct bases are checked. This affects AIR301's `execute` of a BaseOperator
  subclass and AIR303's `get_link` of a BaseOperatorLink subclass. Ruff's `resolve_assignment` (the binding
  current at the read) is `air_value` in misc-airflow.rr, used by AIR301 and AIR303 alike.
- **A4. Two AIR301 quirks.** Ruff's `DayOfWeekSensor` arm for `use_task_execution_day` is unreachable,
  because it sits under the operator test, which needs a name ending in `Operator`. It is left out here
  too. AIR301's `apply_defaults` fix (delete the line) is omitted.
- **E1. ERA001's parse test is approximated.** Ruff's last test is `parse_module(line).is_ok()`. Here it is
  three regex checks, until a native `py_parses` exists (request 1 below):
  - balance: brackets and quotes nest and close, to depth 4 (`era_bal`), so `# x = (1, 2`,
    `# x = "unterminated` and `# result = compute(a,` are not reported;
  - a small grammar (`era_re`): import, a simple keyword statement, assignment (augmented and annotated)
    to assignable targets, a bare expression, and a header (`if`, `for`, `while`, `with`, `def`, `class`,
    `async with/for`…) with its body. Expressions take operands (names, numbers, strings, calls,
    subscripts, attribute chains after a call, `(x)`, displays, comprehensions, `lambda`) joined by binary
    and comparison operators and ternaries, so `# x = 1 +` fails;
  - adjacency (`era_bad`): two operands side by side, or an operator followed by a keyword that cannot
    start an operand, before the first quote or `#`.
  `app_label="core", model="user"` is no longer reported. Lines over 200 characters are not tried. Task
  tags are Ruff's default (TODO, FIXME, XXX), not the `task-tags` setting. The `isort:` pragma options
  match in lower case only. The `# /// script` metadata block is read only in files that have that
  comment (`era_has_script`).
- **T1. `truth_of` follows Ruff's Truthiness.** ruff-classes.rr's `truth_of` (shared with the RUF rules) now
  also knows floats and imaginary numbers (zero is falsey), bytes, t-strings, the ellipsis, lambdas and
  generators, displays whose only elements are splats (unknown), and builtin constructors
  `list()`/`tuple()`/`set()`/`dict()`/`frozenset()` (no arguments: falsey; one positional argument: its
  truth, where Ruff recurses). LOG007 and LOG014 read it. LOG014 also keeps Ruff's `is_literal_expr`
  gate, so `exc_info=ValueError()` and a tuple are not reported.
- **E2. rrlab missed a `# noqa` that follows other text in the same comment (fixed in rrlab 08d977aa5).** These
  were ERA001.py lines 16, 25 and 30, for example `#import os  # noqa: ERA001`. rrlab now finds `# noqa` anywhere
  in the comment, as Ruff does. rrlab's Rust regex then refused ERA001's look-around (`(?!…)`), which cost 6 FNs;
  the merge rewrote those tests without it, and ERA001 is 19 / 0 / 0 on Ruff's snapshots (`rrlab score
  --snapshots`).

## Library and core changes

1. **Core: a native "parses as Python" relation for ERA001.** For example `py_parses(s: symbol)`: the text
   s parses as a module under Ruff's parser, the native one `syntax_error` already uses. It would be
   computed once per distinct string, and only for strings a rule asks about, via an `ask` relation as
   `seen_ask` does. With it, `era_parses` becomes one atom, and E1's approximation goes away.
2. **rrlab: find `# noqa` anywhere in a comment, and honour file-level noqa.** Ruff finds `#\s*noqa`
   after other text: `x = 1  # note # noqa: E501`, or `#import os  # noqa: ERA001`. rrlab's `suppress`
   matches only at the comment's start. It also ignores file-level `# ruff: noqa` and `# flake8: noqa`.
   This hits any rule whose line ends in a comment with text before the noqa, or whose file opts out (E2).
3. **rrlab lowering: `split_dot(q, _, x)` fails.** Soufflé rejects it with "Underscore in binary relation",
   because the lowering emits `_ = @rr_dot_init(q)`. A `_` there should become a fresh variable. The rules
   here name it (`split_dot(q, m, x), m != ""`).
4. **Parser: `fix safe` on the `suppress` line is dropped.** A `fix safe name "…"` written on the same line as
   `suppress ruff "X"` is silently ignored, so the fix relation becomes an ordinary rule. check_rr then
   reports it as unbound. Either accept the form or report it as an error.
5. **Known limits I worked around:** `(?i:…)` inside a regex (std::regex ECMAScript; use character
   classes); a negated disjunction `!(a ; b)`; a disjunction inside an aggregate body (use a helper relation).

## Duplicates to merge

- `misc-comments.rr` `fixme_tag(c, t, d)` and `misc2-todo.rr` `todo_tag` (arity 4) read the same Ruff
  TodoDirective.
- `misc-airflow.rr` `air_guarded` / `air_in_handler` / `air_catches` and `misc2-numpy.rr` `npy_guarded` /
  `npy_in_handler` / `npy_catches` implement the same Ruff helper, `is_guarded_by_try_except`.
  `exceptions.rr`'s `exc_handler` / `exc_type` cover the handler part too. One library relation,
  `guarded_fallback(e, new_q)`, would serve AIR and NPY.
- ERA001's `era_own_line` and `misc2-noqa.rr`'s `own_line_comment` are the same relation.
- `misc-seen.rr` `seen_module(d, m)` and `simplify.rr` `sim_os_seen(d)` (Ruff's `seen_module` for `os`) are the
  same.
- `logging.rr` `log_call` / `logger_expr` and `exceptions.rr` `exc_log` both implement Ruff's
  `is_logger_candidate`.
- misc-airflow.rr's `air_value` / `air_instance` (Ruff's `typing::resolve_assignment`) would fit in ruff.rr
  beside `typed_as`.
