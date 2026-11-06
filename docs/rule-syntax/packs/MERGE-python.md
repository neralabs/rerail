# Merge: python (Ruff and Bandit)

The python libraries after the hand conversion: each concept that two batches wrote separately is now defined
once, the change requests against lib/python are applied or answered, and the rules use the survivors. Core,
functor and harness requests are in scratchpad/batches/CORE-QUEUE-python.md.

Size: lib/python 7,118 → 6,963 lines (−155), packs/python rules 14,429 → 14,404 (−25): −180 in all. 93 rule
files and 30 library files changed. (After the rrlab b402cbfa8 follow-ups below, which added python/names'
`parsed_in` and rewrote ERA001 without look-around; before them lib/python was −182 and the rules −49.)

## Triage: duplicates

| Concept | Copies | Survivor | Owner | Rules changed |
|---|---|---|---|---|
| Parentheses off, comments inside skipped | ast `unparen` (read child 0), ruff `ruff_bare`, bugbear `bb_unparen`, exceptions `exc_unparen`, compare `same_bare` | `unparen` (fixed: the one non-comment child) | ast.rr | 45 rules and 17 libraries renamed |
| Python's builtins | names `builtin` (subset), pyflakes `builtin_name`/`builtin_more`, exceptions `exc_more_builtin` | `builtin`, now Python 3.13's full list plus `WindowsError` and the module attributes | names.rr | F405, F822, F823 (`builtin_name` → `builtin`); UP024 (WindowsError through `builtin_at`) |
| Expression sameness (ComparableExpr) | compare `cmp_same`, pytest `pytest_same` (+ `pytest_atom`, `_kid`, `_pair`, `_differ`), comprehensions `c4_same` (+ `c4_tuple_pair`, `_part`, `_width`) | `cmp_same` / `cmp_same_ask`, with pytest_same's extras (floats by value, f-string prefixes without quotes, complex) and tuple spellings as one kind | compare.rr | PT014, C420; comprehensions' `c4_passthrough` |
| str literal with its value | pyflakes `plain_str`, pytest `pytest_str`, docstrings `plain_doc` | `plain_str` | ast.rr (moved from pyflakes.rr, so `docstring` can use it) | PT003, PT007, PT019 |
| Docstring | ast `docstring` (on `str_const`, no rows in rrlab, parentheses not taken off), typing `py_docstring` / `doc_stmt`, docstrings `doc_expr`, upgrade `docstring_stmt` | `docstring(o, e)` and `doc_stmt(s, e)` (Ruff's is_docstring_stmt: parentheses off, adjacent literals, not bytes or f-strings) | ast.rr | PYI021, RUF001, RUF002; docstrings.rr and upgrade.rr read it |
| Stub file | typing `stub_file`, docstrings `doc_stub`, upgrade `up_stub` | `stub_file(d)` | ast.rr | D418, UP025, UP043, TC006; 5 libraries |
| Hard keywords | bugbear `bb_keyword`, upgrade `py_keyword` | `py_keyword` | names.rr | none |
| typing modules | typing `typing_mod`, upgrade `up_typing_module` | `typing_mod` | names.rr | none (libraries only) |
| Identifier test | bugbear `bb_identifier` (non-ASCII approximated by `\x80-\xff`), upgrade `identifier` (ASCII only) | `identifier(x)`: `[_\p{XID_Start}]\p{XID_Continue}*`, not a keyword, as Ruff's is_identifier | pyflakes.rr (needs plain_str) | B009, B010, PIE804 (and UP013/UP014 through upgrade.rr) |
| Argument counts, keywords, splats | bugbear `bb_npos`, `bb_nkw`, `bb_kw`, `bb_star`, `bb_kwsplat`; upgrade `up_npos`, `nkw`, `up_kw`, `kwsplat`, `up_splat`; pyflakes-format `call_star`, `call_kwsplat`, `call_kw` | ruff's `call_nargs`, `call_nkwargs`, `call_splat`, and new `call_kw` (value unparenthesised), `call_star`, `call_kwsplat` | ruff.rr | 18 rules: B004, B005, B009, B010, B022, B028, B034, B039, B905, C408, PIE796, PIE810, UP013, UP014, UP015, UP018, UP022, UP032 (pyflakes-format's keyword node: `call_kwarg`); bugbear.rr, upgrade.rr and pyflakes-format.rr now include ruff.rr |
| Truthiness | ruff-classes `truth_of`, simplify `bare_truth`/`sim_truth`, PT015's `always_false`/`falsy_atom`/`empty_piece` | `truth_of(m, t)` (Ruff's Truthiness::from_expr, with simplify's f-string rules: non-empty f-strings are true, t-strings always true); `sim_truth` is a one-line view of it | ruff.rr (moved from ruff-classes.rr) | PT015 (rewritten on it), SIM222, SIM223 through sim_truth; LOG007, LOG014 unchanged |
| t-string literal | simplify `sim_tstring`, ruff `string_form(m, "tstr")` inline | `tstring_lit(m)` | ruff.rr | SIM905 |
| Side effects (contains_effect) | ruff `side_effect`, simplify `sim_effect` (+ `effect_node`, `safe_operand`, `iterable_init`), TRY300's private `may_effect` | `side_effect` | ruff.rr | SIM222, SIM223, TRY300 |
| Ruff's seen_module | misc-seen `seen_module`, simplify `sim_os_seen` | `seen_module(d, "os")` | misc-seen.rr | SIM112 |
| Own-line comment | ERA001's `era_own_line`, misc2-noqa `own_line_comment` | `own_line_comment` | misc2-noqa.rr | ERA001 |

Where two copies differed, the survivor takes the more correct one, and the rescore below shows each difference:

- `unparen`: ast's read child 0, which is a comment in `(  # why` + newline + `x)`. The four copies all skipped
  comments; that is now ast's own rule, so the copies are identical to it and go.
- `cmp_same` vs `pytest_same`: pytest's compared floats by value (`1.0` = `1.00`) and f-strings without their
  quotes; compare's compared floats by text. compare.rr now does both, plus complex literals. The tuple spellings
  (c-sim change 5) are one kind there too, so `c4_same`'s one-level bridge is gone and nested tuple targets
  compare as well.
- `truth_of` vs `bare_truth` vs `always_false`: three partial copies of one Ruff function. truth_of lacked
  non-empty f-strings and gave `t""` false; bare_truth lacked "none"; always_false handled only falsy values.
  The survivor is truth_of with bare_truth's f-string rules and Ruff's "t-strings are always truthy".
- `identifier`: bugbear's `\x80-\xff` meant bytes under the old matcher and Latin-1 code points under the Rust
  one, so `"ſ"` stopped being an identifier; the XID classes are exact (B009/B010 gain their FN).
- `docstring`: ast's version read `str_const`, which needs `str_part` (no rows in rrlab), and did not take
  parentheses off; Ruff does. Only upgrade.rr used it.

### Kept apart (different semantics, or a semantic change that needs its own pass)

| Pair | Why it stays |
|---|---|
| `ruff_qual` (ruff.rr), `sym_of` (names), `sym_at` / `up_qual` (upgrade), `exc_qual` (exceptions) | Three resolution engines: names' file-wide may-resolution, upgrade's visit-order lookup, ruff's latest-binding with rebinding checks. Ruff has one (resolve_qualified_name). Merging them changes what dozens of rules resolve; it is the next pass, not a rename. The same holds for `typing_sym` / `typing_ref` / `typing_name` / `annt_typing` and `exc_call` / `call_q`, which are each engine's "callee is q". |
| `bb_pos` (bugbear), `up_arg` (upgrade), `ruff_arg` (ruff), `call_pos` / `arg_pos` (pyflakes-format), `exc_arg` (exceptions) | Different views of "the i-th positional": `*xs` counted as one, skipped, or ending the list. Ruff uses each in different rules. They could share one relation with a "how" column; not done here. |
| `ruff_int(e, v)` and typing's `pyi_int(e)` | Same idea; typing.rr does not include ruff.rr and PYI003/PYI051 only need the kind. Left. |
| `doc_exports` / `all_entry` (docstrings vs pyflakes) | F822 counts `__all__.extend`/`append`, visibility does not (d-n). Different. |
| `typing_dec` (naming) and `typing_sym` (typing) | typing_dec is on a decorator, through a call. |
| `fixme_tag` (misc-comments) and `todo_tag` (misc2-todo) | Same tag regex, different columns; FIX00x and TD00x both score clean. A follow-up can make fixme_tag read todo_tag. |
| `air_guarded` / `npy_guarded` (Ruff's is_guarded_by_try_except), `log_call` / `exc_log` (is_logger_candidate), `air_value` (resolve_assignment), `fast_strip` / `tc_strip`, `ytt_sys`, `async_name_value` / `only_site`, `async_ctx`, `msg_literal` / `py_literal`, `slots_enum` / `in_enum`, `argt_abstract` / `abstract`, `argt_method` / `method_kind`, `ann_method` / `pyi_method`, `type_bare` / `ann_strip`, `tsub` / `sub_item`, `plain_target` / `assign_name`, `ruff_type_nargs` / `up_type_nargs`, `future_annotations_in` (RUF001) / `future_annotations` | Each pair is the same Ruff helper with different arguments or edge cases (the batch NOTES say which). Each merge needs a signature decision and a rescore of its users; none blocks a rule. Listed for the next pass. |
| `typing_only_ctx` / `tc_runtime_typedef` / `runtime_annotation` / `late_annotation` | One "is this annotation evaluated" library should serve all four (pyi's note); their scope rules differ today. |

## Change requests against lib/python

| Request | Status |
|---|---|
| ast `unparen` skips comments (b-pie-ret 1, pt-try 2, ruf L1, up) | Applied. `bb_unparen`, `exc_unparen`, `ruff_bare`, `same_bare` removed. |
| ast `bool_operand` ignores comments (ruf L2) | Applied (`bool_part`). RUF019 keeps its count, which is now redundant but harmless. |
| names `builtin` complete (f-e C1, pt-try 1, ruf L4, up WindowsError) | Applied: 47 names added (the 3.13 exceptions, `True`/`False`/`None`, `exit`, `quit`, `copyright`, `credits`, `license`, `__build_class__`, `__loader__`, `__package__`, `__annotations__`, `__builtins__`, `__cached__`, `WindowsError`). `builtin_name` and `builtin_more` dropped; `exc_more_builtin` and exceptions' second `exc_qual` rule dropped. Not version-gated (Ruff gates ExceptionGroup on 3.11, etc.). |
| names `not_name` for `__future__` imports (up) | Applied; UP010's workaround removed. |
| names: export `bound_in`, `walk`, `miss` (f-e C2) | Applied in names' header (they were already used). `bound_before` (a visit-order lookup) not added: pyflakes.rr's `unbound_ref` does it, and only F405/F633/F821 would share it. |
| typing-context `deferred_ann` scope test only on variable annotations (misc2 request 5) | Already done: the scope rule has `kind(a, "assignment")`. No change. |
| compare.rr treats tuple targets as tuples (c-sim 5) | Applied (`same_kind`); `c4_same` reduced to `cmp_same` and removed. |
| `docstring` falls back to plain_str (pyi 1) | Applied: docstring is now doc_stmt-based (see the table). |
| names: bind `type X = …` (pyi 2) | Not done. It needs a binding kind for type aliases and a rescore of every scope rule. PYI047's text-based `type_alias_read` stays. |
| names `class_base` through `Base[T]` (pyi 3) | Not done, same reason: class_base feeds subclass/mro for many rules. typing.rr's `class_base_sym` stays. |
| names: `ruff_latest_site` (misc2 request 4) | Not done; misc2-perf's `perf_latest_site` stays. It belongs with the resolution merge above. |
| `truth` clash (c-sim 4) | Moot: truth_of is the one Truthiness, in ruff.rr. |
| `capture_parent` (pyi 4), `security.rr` vocabulary (pyi 7), `str_const` (bandit 3), `star_arg` removal (c-sim 6) | Observations; no change needed. |
| Core, functor, check_rr and rrlab items (all batches) | Queued with exact text in CORE-QUEUE-python.md, or marked there as answered by rrlab 08d977aa5 or b402cbfa8. |

Coordinator items (during this merge):

- **RUF001 confusables:** lines 18 and 21 of the generated `.confusables.rr` (and the generator,
  gen-ruff-ruf001-…py) used `glob` where they meant a regex. Now `match`; the round-trip check passes. RUF001
  finds all 15 Ruff findings (was 6 TP / 9 FN).
- **TD001–TD007 at the tag:** every TD rule reports at `todo_at`'s span (Ruff's column; TD006 checked on
  TD006.py: 5:3, 6:3, 7:9 as the snapshot), and TD006's fix replaces only the tag (`replace(t, `TODO`)`).
  misc2-todo-span.rr is kept; its comment is updated and its regex now matches todo_tag's exactly.
- **Direct regex forms:** python/pyflakes' `identifier` uses `\p{XID_…}`; FAST003 uses `match_all` with span
  captures instead of a recursive `match`, and reports at the `{…}` as Ruff does (score unchanged). No other python
  rule still had an ASCII copy or a counting workaround for a missing regex feature: D404 already used `\p{…}`,
  and UP015/UP032's `@count_matches` count real characters.
- **rrlab b402cbfa8 follow-ups:** ERA001 rewritten without look-around (no other python rule used it). SIM222
  and SIM223 skip nodes in a tree parsed from a string annotation (python/ast's `in_string_ann`), as Ruff's
  `in_string_type_definition` does. python/names gains `parsed_in(e, s)`, which ties a parsed node to its
  outermost string annotation, and `wstart` starts a parsed name's lookup at that string's scope, so `look`,
  `sym_of`, `sym_at` and `ruff_qual` resolve inside string annotations. `annotation` covers the parsed trees, and
  `ann_root` moved from python/ast to python/names (ast's `ann_node` reads `parsed_in`). Strings in `Literal[…]` and
  the parts of an implicit concatenation are not parsed. RUF001 skips string annotations and their trees, as
  Ruff does.

## Scores

Every one of the 537 rules was rescored, since ast.rr and names.rr are under all of them. "Before" is HEAD's lib/
and packs/python (b402cbfa8; scratchpad/merge-py/brepo, read through `RRLAB_RR`), scored back to back with "after"
(the live tree) by the same rrlab, rule by rule (scratchpad/merge-py/rrpair.sh; results in rrscores.tsv).

Scorers:
- `rrlab score python <id> --snapshots <ruff 0be08a2>` (Ruff 0.16.9's snapshot tests, default settings): 411 Ruff
  rules.
- `rrlab score python <id>` on the Data-layer-rule-expressivity recording: the 49 Bandit rules and the 111 Ruff
  rules it holds.
- Where rrlab finds no snapshot: scratchpad/merge-py/snapscore.py (snapshot names that fall back to the code:
  23 PT rules, all unchanged); anndef.py (ANN001-ANN206 and ANN401 against flake8_annotations' `defaults.snap`);
  `rrlab score … --data scratchpad/ruf/data16` (RUF001-RUF003, a recording made from their snapshot).
- Not scored by any of these, because Ruff tests them only with options: C901, COM818, EM101-EM103, ICN001, PT001,
  PT007, PT011, PT023, PT030, PTH100-PTH122, RUF063, TID252 (36 rules). None of their files changed except through
  the shared libraries.

Scores that changed (TP / FP / FN; every other rule is the same before and after on every scorer):

| Rule | Scorer | Before | After | Why |
|---|---|---|---|---|
| B009 | recording | 21 / 0 / 1 | 22 / 0 / 0 | `identifier` uses `\p{XID_…}`, so `ſ` is an identifier (rrlab's Rust regex reads `\x80-\xff` as code points). |
| B009 | snapshots | 19 / 0 / 1 | 20 / 0 / 0 | Same. |
| B010 | recording | 8 / 0 / 1 | 9 / 0 / 0 | Same. |
| B010 | snapshots | 6 / 0 / 1 | 7 / 0 / 0 | Same. |
| ERA001 | snapshots | 13 / 0 / 6 | 19 / 0 / 0 | Rewritten without look-around, which Rust regex refuses ("bad regex": those tests matched nothing). |
| FAST003 | snapshots | 27 / 7 / 4 | 31 / 7 / 0 | `match_all` finds every `{param}` of a path; the recursive `match` it replaced missed some. The 7 FPs are unchanged. |
| SIM222 | recording, snapshots | 59 / 1 / 0 | 59 / 0 / 0 | Skips trees parsed from string annotations (`in_string_ann`), as Ruff does (SIM222.py:199). |
| SIM223 | recording, snapshots | 51 / 1 / 0 | 51 / 0 / 0 | Same (SIM223.py:159). |
| UP006 | snapshots | 8 / 0 / 12 | 17 / 0 / 3 | names' `parsed_in`: a name in a string annotation is looked up from the string's scope. The 3 FNs are strings nested in a parsed string, which rrlab does not parse (core queue 18). |
| UP007 | snapshots | 18 / 0 / 6 | 19 / 0 / 5 | Same as UP006. |
| PYI019 | snapshots | 69 / 0 / 10 | 71 / 0 / 8 | Same. |
| PYI050 | snapshots | 10 / 0 / 6 | 11 / 0 / 5 | Same (`def stringized(arg: "NoReturn")`). |
| RUF013 | snapshots | 21 / 0 / 4 | 25 / 0 / 0 | Same. |
| ANN401 | anndef.py | 18 / 0 / 1 | 19 / 0 / 0 | Same. |
| UP032 | snapshots | 91 / 0 / 2 | 92 / 0 / 1 | `unparen` skips comments (UP032_0.py:94). |
| RUF001 | data16 | 6 / 3 / 9 | 15 / 3 / 0 | The confusables table's two `glob`s are `match`es; strings in or holding a parsed annotation are skipped (confusables.py:61). The 3 FPs are `−`, which Ruff's test allows by option (core queue 23). |
| RUF002 | data16 | 1 / 0 / 0 | 1 / 1 / 0 | The table fix finds `ρ`, which the same option allows (confusables.py:12). |

F632 and F822, which differed in an earlier unpaired run, score the same before and after when paired: the
difference came from rrlab changing between the runs, not from the merge.

## To delete

- Nothing. lib/python/misc2-todo-span.rr is used again (by TD001–TD007), so it is no longer to delete.
