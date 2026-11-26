# Python translation batch 21

The checks below follow Ruff. Ruff was run with `--no-cache` on each differing fixture on a fleet box. Shared string-type handling is in `lib/python/python-21.rerail`: expression rules inspect calls parsed recursively from strings in type contexts, but not strings outside those contexts (including `Annotated` metadata).

## `ruff-pth101-os-chmod`

Follows Ruff's latest class-body binding when deciding whether a path is an integer file descriptor. In `class O: class B: y = 3; B = None`, Ruff reports `os.chmod(O.B.y, 1)` (`PTH101-class-latest-binding-and-int-inference.py:48`); the old translation incorrectly followed the superseded nested class and treated `y` as an integer. The rule-local `replaced_nested_class` clause should eventually fold into `python/pathlib.rerail`'s `pth_class_of`/`file_descriptor` vocabulary; this batch may not edit that library.

## `ruff-pth106-os-rmdir`

Follows Ruff inside string type expressions. Ruff reports `z: "os.rmdir(p)" = 1`, including recursively nested strings such as `-> "list['os.rmdir(p)']"`; the old translation omitted these calls. `python/python-21.rerail` supplies the shared checked-call boundary.

## `ruff-pth107-os-remove`

Follows Ruff inside string type expressions. Ruff reports `z: "os.remove(p)" = 1`, including recursively nested strings; the old translation omitted these calls. Uses the shared `python/python-21.rerail` boundary.

## `ruff-pth108-os-unlink`

Follows Ruff inside string type expressions. Ruff reports `z: "os.unlink(p)" = 1`, including recursively nested strings; the old translation omitted these calls. Uses the shared `python/python-21.rerail` boundary.

## `ruff-pth109-os-getcwd`

Follows Ruff's string-type parser. Ruff reports `n: "os.getcwd()" ""` but not `m: "os.getcwd" "(p)"`; the old translation reported the latter and omitted calls parsed from string type expressions. Uses the shared `python/python-21.rerail` boundary.

## `ruff-pth110-os-path-exists`

Follows Ruff inside string type expressions. Ruff reports `z: "os.path.exists(p)" = 1`, including recursively nested strings; the old translation omitted these calls. Uses the shared `python/python-21.rerail` boundary.

## `ruff-pth111-os-path-expanduser`

Follows Ruff inside string type expressions. Ruff reports `z: "os.path.expanduser(p)" = 1`, including recursively nested strings; the old translation omitted these calls. Uses the shared `python/python-21.rerail` boundary.

## `ruff-pth124-py-path`

Follows Ruff's type-context boundary. Ruff reports the nested call in `a: "List['py.path.local(1)']"` and the call in `c: "((py.path.local))()"`, but not the metadata string in `b: Annotated[int, "py.path.local()"]`. The old translation missed the nested call and reported the metadata call. Uses the shared `python/python-21.rerail` boundary.

No engine primitive or `.decl` addition is required. No differences are intentionally retained.
