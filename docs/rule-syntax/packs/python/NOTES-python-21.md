# Python translation batch 21

The checks below follow Ruff 0.17.6 (`uvx ruff@0.17.6 check --select PTH… --no-cache --isolated`, and its source:
`ruff_python_semantic/src/model.rs`, `ruff_linter/src/checkers/ast/mod.rs`, `flake8_use_pathlib/helpers.rs`). The
eight rules share `lib/python/python-21.rerail`, which finds the calls and resolves their callees as Ruff does:

- **Where Ruff looks.** Ordinary code, and every string Ruff parses as a type (python-01's `typedef_node`):
  annotations, `cast("…", x)`, TypeVar bounds and constraints, `list["…"]`, and strings nested in those; not
  `Literal[…]` strings or `Annotated` metadata.
- **When Ruff resolves a name.** At the moment its walk visits it: a scope holds the bindings visited so far. Def and
  lambda bodies are visited after the module (a nested one after its parent's), so they read an enclosing scope's
  last binding; decorators, defaults, a def's annotations and class bodies read it in place.
- **Forward references** (type strings, and annotations under `from __future__ import annotations` or Python 3.14)
  are visited last and resolve first against the module's last binding of the name (when it is not a `del`), and
  only otherwise from their own scope.
- **Complex strings.** A concatenated or escaped string's tree carries the whole string's range, and Ruff keys a
  name's resolution by its start offset, so all its names share one record. A call there is checked with its
  callee's head read as the last name of the call that resolved (a builtin counts, an undefined name does not):
  `x: "os.remove('a\\tb')"` and `y: "os.getcwd(" ")"` are reported, `z: "os.remove(" "p)"` (the callee reads as
  `p.remove`) is not.

## Review fixes (`review/python-21.review.md`)

Every defect of the review is fixed, and each counterexample is a `.test` in each rule expecting exactly Ruff
0.17.6's lines (`review_shared`, `review_concat`, and PTH101's `review_pth101`). Further Ruff-checked tests pin the
resolution above: PTH106's `complex_strings`, `complex_string_names`, `forward_references`, `deferred_bodies`,
`future_annotations`; PTH101's `string_file_descriptors`, `class_attributes`; PTH109's, PTH110's and PTH124's
`import_forms`.

- S1 `cast` strings, S2 escaped strings: found through `typedef_node` and resolved from the string's place with
  the batch's own lookup (names' `parsed_in` covers annotations only).
- S3 implicit concatenations: the complex-string rule above replaces "one piece holds the callee and `(`".
- S4 forward references in a function: module first.
- S5 deferred bodies: last bindings of the enclosing scopes.
- PTH101: `C.D.x` follows Ruff's `lookup_attribute` (the head's binding where it is read, then each class body's
  last binding), so `Redefined.B.fd` (the later `B` binds an int) is a file descriptor and `Rebound.fd` after
  `Rebound = None` is not; the rule-local `replaced_nested_class` is gone. In a type string the path argument and
  `dir_fd=` are read from the parsed tree: `"os.chmod(3, 0o644)"` and `"os.chmod('f', 0o644, dir_fd=3)"` are not
  reported. A name is a file descriptor when its binding (found as above) shadows no earlier one in its scope and
  is an int; in a complex string a name never is (it resolves as the call's last name, the callee's module when
  the call is reported at all).

## Per rule

- `ruff-pth101-os-chmod`: every `os.chmod` call, unless the path is a file descriptor or `dir_fd` is set and not
  None.
- `ruff-pth106-os-rmdir`, `ruff-pth107-os-remove`, `ruff-pth108-os-unlink`, `ruff-pth110-os-path-exists`,
  `ruff-pth111-os-path-expanduser`: the call with exactly one argument, the path, by position or keyword.
- `ruff-pth109-os-getcwd`: every `os.getcwd` and `os.getcwdb` call.
- `ruff-pth124-py-path`: every `py.path.local` call.

## Not followed

- Ruff visits a call's keywords after all its positional arguments, a comprehension's generators before its
  element and a conditional's test before its body; the complex-string rule takes the last name in source order.
  Only a call whose last two resolved names come in a different order there could differ.
- Ruff's `global x` in a function adds a binding to the module when the function body is visited; the batch places
  bindings by their position in the file only.
- A bare annotation (`x: int`) in a function makes a later read of x there unresolved in Ruff; the batch's lookup
  passes over it to the outer scopes.

## Proposed library changes

The resolution is the batch's own (`p21_*` in `lib/python/python-21.rerail`), because this batch may not edit the
shared libraries:

- `python/ruff`'s `ruff_qual` (and so `pathlib`'s `pth_call`) should resolve at Ruff's visit time: deferred bodies
  read last bindings (it reads the latest binding before the reference), forward references go module first
  (it starts from the enclosing function), and names in non-annotation type strings resolve too (names'
  `parsed_in` covers annotations only; python-03 adds the others).
- `python/pathlib`'s `file_descriptor` should take `pth_class_of`'s head from that resolution and each class's last
  binding of the next name (`lookup_attribute`), as `p21_attr_site` does.

No engine primitive or `.decl` addition is required.
