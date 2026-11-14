# py-pth: 35 Ruff flake8-use-pathlib rules (PTH), hand-written

The rules follow Ruff 0.16.9, the version the JSON was written from. Its source, fixtures and snapshots, at the
recording's commit 0be08a2, are in
`/Users/jeremy/orca/workspaces/rerail/wolfeel/.rerail/port-readiness/upstream/ruff/0be08a2…/crates/ruff_linter`.
`src/rules/flake8_use_pathlib/helpers.rs` and `ruff_python_semantic/src/analyze/typing.rs` settled what the JSON
left open. Ruff 0.15.7 is installed here, and I also ran it on every case.

All 35 rules share one new library:

| file | lines | what |
|---|---|---|
| `lib/python/pathlib.rr` | 201 | calls by qualified name, with the callee where Ruff reports (`pth_call`); Ruff's Arguments API (`call_len`, `arg_value` = `find_argument_value`, `single_arg` = `check_os_pathlib_single_arg_calls`, `kw_set` = `is_keyword_only_argument_non_default`); Ruff's type inference for one binding (`only_site`, `site_init`, `site_ann`), with ints (`resolved_int`, `file_descriptor` = `is_file_descriptor`) and pathlib paths (`path_obj` = `is_pathlib_path`) |

Most rules are one line of logic. The 17 single-path rules (PTH100, 106-108, 110-115, 117, 119, 120, 202-205)
are each one `single_arg` row: the function's qualified name and its parameter name, which differs (`s` for
isdir and isabs, `p` for basename and dirname, `filename` for the getters). The 35 rules have 710 lines in total,
comments and directives included, from 16 to 29 lines each. PTH115 and PTH210 also read the target
Python version through `python/upgrade`'s `targets`.

## Scores

No recording covers these rules. The Data-layer recording does not run PTH, and its conformance.json does not
list the rules, so `rrlab score python <id> --all` cannot score them. I scored them three ways instead, matching
(path, line, column, code):

- **Ruff 0.16.9's own tests:** the 21 fixtures in `resources/test/fixtures/flake8_use_pathlib/`, against the
  findings in the tests' snapshots (the default-settings snapshots, not preview). A file is scored only for the
  codes its test enables, as `mod.rs` lists them. 309 findings.
- **Cases:** 11 files of my own, run through Ruff 0.15.7: versions of the same fixtures, plus edge cases for
  arguments, file descriptors, type inference, aliases and noqa.
- **All 462 Ruff fixtures of the Data-layer recording**, run through Ruff 0.15.7 for PTH: 57 findings.

The scripts and files are in the session scratchpad, under `pth/`. `snapcmp-real.sh` runs the 0.16.9 tests (fixtures
in `fx16/`), `cmp.sh` runs the cases (`fx/`, `fx2/`, `fx3/`), and `bigcmp.sh` runs the 462 fixtures.

| rule | lines | Ruff 0.16.9 tests | cases (0.15.7) | 462 fixtures (0.15.7) | notes |
|---|---|---|---|---|---|
| ruff-pth100-os-path-abspath | 20 | 4 / 0 / 0 | 8 / 0 / 0 | 2 / 0 / 0 | |
| ruff-pth101-os-chmod | 21 | 8 / 0 / 0 | 10 / 0 / 0 | 0 / 0 / 0 | V1 |
| ruff-pth102-os-mkdir | 18 | 6 / 0 / 0 | 8 / 0 / 0 | 0 / 0 / 0 | |
| ruff-pth103-os-makedirs | 18 | 8 / 0 / 0 | 8 / 0 / 0 | 0 / 0 / 0 | |
| ruff-pth104-os-rename | 18 | 9 / 0 / 0 | 9 / 0 / 0 | 0 / 0 / 0 | |
| ruff-pth105-os-replace | 18 | 6 / 0 / 0 | 6 / 0 / 0 | 0 / 0 / 0 | |
| ruff-pth106-os-rmdir | 20 | 4 / 0 / 0 | 6 / 0 / 0 | 0 / 0 / 0 | |
| ruff-pth107-os-remove | 20 | 4 / 0 / 0 | 5 / 0 / 0 | 0 / 0 / 0 | |
| ruff-pth108-os-unlink | 20 | 4 / 0 / 0 | 5 / 0 / 0 | 0 / 0 / 0 | |
| ruff-pth109-os-getcwd | 19 | 9 / 0 / 0 | 11 / 0 / 0 | 1 / 0 / 0 | |
| ruff-pth110-os-path-exists | 20 | 4 / 0 / 0 | 5 / 0 / 0 | 6 / 0 / 0 | |
| ruff-pth111-os-path-expanduser | 20 | 4 / 0 / 0 | 5 / 0 / 0 | 0 / 0 / 0 | |
| ruff-pth112-os-path-isdir | 20 | 4 / 0 / 0 | 5 / 0 / 0 | 0 / 0 / 0 | |
| ruff-pth113-os-path-isfile | 20 | 4 / 0 / 0 | 5 / 0 / 0 | 0 / 0 / 0 | |
| ruff-pth114-os-path-islink | 20 | 4 / 0 / 0 | 5 / 0 / 0 | 0 / 0 / 0 | |
| ruff-pth115-os-readlink | 23 | 5 / 0 / 0 | 5 / 0 / 0 | 0 / 0 / 0 | 3.9+ |
| ruff-pth116-os-stat | 21 | 4 / 0 / 0 | 7 / 0 / 1 | 0 / 0 / 0 | V1, V2 (the FN) |
| ruff-pth117-os-path-isabs | 20 | 4 / 0 / 0 | 5 / 0 / 0 | 0 / 0 / 0 | |
| ruff-pth118-os-path-join | 26 | 14 / 0 / 0 | 19 / 0 / 1 | 2 / 0 / 0 | H1 |
| ruff-pth119-os-path-basename | 20 | 4 / 0 / 0 | 5 / 0 / 0 | 0 / 0 / 0 | |
| ruff-pth120-os-path-dirname | 20 | 4 / 0 / 0 | 5 / 0 / 0 | 3 / 0 / 0 | |
| ruff-pth121-os-path-samefile | 16 | 5 / 0 / 0 | 6 / 0 / 0 | 0 / 0 / 0 | |
| ruff-pth122-os-path-splitext | 16 | 4 / 0 / 0 | 6 / 0 / 0 | 0 / 0 / 0 | |
| ruff-pth123-builtin-open | 27 | 22 / 0 / 0 | 36 / 0 / 4 | 42 / 0 / 0 | V1 (the 4 FN) |
| ruff-pth124-py-path | 18 | 2 / 0 / 0 | 3 / 0 / 0 | 0 / 0 / 0 | |
| ruff-pth201-path-constructor-current-directory | 22 | 15 / 0 / 0 | 26 / 0 / 0 | 1 / 0 / 0 | |
| ruff-pth202-os-path-getsize | 20 | 35 / 0 / 0 | 2 / 0 / 0 | 0 / 0 / 0 | |
| ruff-pth203-os-path-getatime | 20 | 14 / 0 / 0 | 1 / 0 / 0 | 0 / 0 / 0 | |
| ruff-pth204-os-path-getmtime | 20 | 6 / 0 / 0 | 1 / 0 / 0 | 0 / 0 / 0 | |
| ruff-pth205-os-path-getctime | 20 | 6 / 0 / 0 | 1 / 0 / 0 | 0 / 0 / 0 | |
| ruff-pth206-os-sep-split | 19 | 10 / 0 / 0 | 12 / 0 / 0 | 0 / 0 / 0 | |
| ruff-pth207-glob | 20 | 3 / 0 / 0 | 9 / 0 / 0 | 0 / 0 / 0 | |
| ruff-pth208-os-listdir | 24 | 7 / 0 / 0 | 10 / 0 / 0 | 0 / 0 / 0 | V1 |
| ruff-pth210-invalid-pathlib-with-suffix | 29 | 54 / 0 / 0 (H2) | 44 / 0 / 0 | 0 / 0 / 0 | H2 |
| ruff-pth211-os-symlink | 17 | 9 / 0 / 0 | 12 / 0 / 0 | 0 / 0 / 0 | |

Summary: all 35 rules match Ruff 0.16.9's tests exactly, 309 findings. PTH210 needs `pythonVersion` 3.14 to do
so, as the tests run at 3.14 (H2). Against 0.15.7, the only differences are V1 and V2 (0.16.9 is newer) and H1
(a harness gap). The reviewer's cases (scratchpad `rev-py-pth/`) give the same 309 with its `score.py`.

All the scores above come from rrlab on the shared `lib/`. A first run used a copy with one fix in `names.rr`;
the re-run on the committed `lib/`, with that fix, gives the same 309 findings.

## Deviations

None from the JSON's meaning. Where the JSON's form and Ruff 0.16.9's code differ, the rule follows the code:

- **V1, PTH101, 116, 123, 208: class attributes as file descriptors.** 0.16.9's `is_file_descriptor` resolves
  `C.x` and `C.D.x` through `lookup_attribute`. It takes the class scope's latest binding of x and asks
  `typing::is_int` about it. The JSON does the same, and so does the rule. Ruff 0.15.7 reads names only: with
  `class A: fd = 3`, it reports `open(A.fd)` and 0.16.9 does not. These are the cases' 4 FN. A bare `fd: int` in
  the class body counts too, because the class-scope lookup sees annotation bindings (`BindingKind::Annotation`).
  The JSON's PTH101 has this. The JSON's PTH116 applies it to plain names too, but Ruff resolves a load past a
  bare annotation, so `k: int` then `open(k)` is reported. The rule follows Ruff (Ruff 0.15.7 agrees).
- **V2, PTH116 with no path argument.** 0.16.9's `os_stat` returns when `find_argument_value("path", 0)` finds
  nothing, so `os.stat()`, `os.stat(*a)` and `os.stat(**kw)` are not reported. 0.15.7 reports them (the cases'
  1 FN). The JSON binds the path and agrees with 0.16.9.
- **PTH115 needs Python 3.9.** `Path.readlink` is new in 3.9. The JSON's `when: {pythonVersion: {min: "3.9"}}`
  and 0.16.9's `os_readlink` (`target_version() < PY39` returns) agree, and so does the rule (`targets(9)`).
  With `pythonVersion` 3.8 it reports nothing, as `ruff --target-version py38` does.
- **Ruff's only_binding.** Ruff types a name only if the binding it reads has no earlier binding of the name in
  its scope. The rule counts the scope's bindings of the name before the use (`only_site`). When the use is in a
  def or lambda nested in that scope, which runs later, it counts all of them. A class body or a comprehension
  runs at once, so it counts only the earlier ones. `x = 1; open(x); x = "a"` is not reported, as in 0.15.7.
- **Unpacking and chains** follow Ruff's `match_value`: `fd, name = 3, "x"` gives fd the value 3, element by
  element, when both sides are displays of the same length with no `*`. `g = h = 3` gives both 3. The same holds
  for `with` items.
- **PTH210 and `with_suffix(".")`.** Ruff reports it only when the target is below Python 3.14, which allows it
  (`invalid_pathlib_with_suffix.rs`). The rule reads the `pythonVersion` option, default 3.10, through
  `python/upgrade`'s `targets`.
- **PTH201** reports `pathlib.Path` and `PurePath` only. Preview mode adds the four other classes. The JSON and
  the stable snapshot agree.

## Explained misses

- **H1, PTH118 (1 FN), and potentially every rule: calls in string annotations.** Ruff parses a string
  annotation and checks the calls in it: `x: "os.path.join(p, q)" = 1` is PTH118. The JSON adds an "in-string"
  clause to each rule for this, and it parses every string. The rules here use only the tree relations. If the
  engine gives the parsed annotation tree (`parsed(s, "annotation", r)`) the same tree relations, as names.rr's
  `annotation` and ast.rr's `ann_node` expect, the rules report there with no extra clause. rrlab leaves `parsed`
  empty. For this to work, names inside the parsed tree must also resolve (C1). None of Ruff's own PTH fixtures
  has such a case.
- **H2, PTH210 (13 FP at the default target).** Ruff's tests run at Python 3.14 (`LinterSettings::for_rule`
  uses `PythonVersion::latest()`). At 3.14, `with_suffix(".")` is not reported. rrlab's default is 3.10. With
  `--options '{"pythonVersion": "3.14"}'` the score is 54 / 0 / 0. `deferred_annotations_diff_…PTH210.py.snap`
  confirms that 3.13 reports those 7 calls in PTH210.py, and the 6 in PTH210_1.py are the same.

## Limits

- **Type inference** covers what Ruff's checkers read: one binding, with its value or annotation. `global`
  names read in a function are not covered. A comment inside an unpacked display shifts the element pairing.
- **`del` is not a binding** in python/names, so `w = 3; del w; open(w)` is not reported. Ruff reports it: after
  the `del`, w has no binding to type. Rare in practice; a `del` site in python/names would fix it.
- **ResolvedPythonType** is followed as the JSON gives it. Every binary operator but `/` and `@` gives an int on
  two numbers. `and`/`or`/`if-else` give an int when both sides are numbers and one of them is an int.
  Comparisons are not typed, so `(1 < 2) + 1` is not an int here.

## Library and core change requests

- **C1. Parsed string annotations in the tree relations.** H1 needs two things from the engine. The parsed tree
  of a string annotation must have the ordinary tree relations (`kind`, `field`, `child`). Name lookup inside it
  must also work from the string's scope (`look`), so that `ruff_qual` resolves `os.path.join` in
  `x: "os.path.join(p, q)"`. If that is not the engine's model, core should say how a rule reaches a parsed
  tree. No rule here needs a new relation.
- No other changes. `lib/python/pathlib.rr` is new. check_rr reports no other file declaring any of its
  relations.
