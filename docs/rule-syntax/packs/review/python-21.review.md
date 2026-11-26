# python-21 translation review

Reviewed commit `22ece87d06` against Ruff 0.17.6. I read Ruff's official rule pages, the `flake8_use_pathlib` rule implementations and helpers, and its `full_name.py`, import/alias, and `py_path_*.py` fixtures. The ordinary documented calls, imports, aliases, argument counts, PTH101 `follow_symlinks`, and non-`None` `dir_fd` cases agree. The cases below do not.

## Shared defects

All eight rules have the five defects below through `python/python-21.rerail` or the name-resolution library it calls.

1. **Missing non-annotation type strings.** Ruff checks strings passed to `typing.cast` and `TypeVar`, not only annotation roots. `pth_type_source` requires `parsed_in`, so each `cast` call below is missed.
2. **Missing escaped annotation strings.** Ruff reports calls in these strings. `pth_escape` rejects the complete source string when it contains an escape, even when the callee maps unambiguously.
3. **False positives for implicit concatenation split after `(`.** Ruff reports none of `concat.py`; the translation reports every line. Checking only that one piece contains the callee and `(` is insufficient to reproduce Ruff's complex-string source mapping.
4. **Wrong scope for forward references inside functions.** Ruff resolves these quoted type expressions against the module imports and reports them. The translation starts from the function's local `os`/`py` bindings and misses them.
5. **False positives in deferred function bodies after module rebinding.** Ruff checks the function body after the later module assignment and reports nothing. The translation resolves only bindings before the call's source position and reports all eight. This mismatch predates the new type-string helper but remains in every reviewed rule.

`shared.py` (S1, S2, S4, S5):

```python
import os
import py
from typing import cast
p = "x"

# S1
c101 = cast("os.chmod(p, 0o644)", p)
c106 = cast("os.rmdir(p)", p)
c107 = cast("os.remove(p)", p)
c108 = cast("os.unlink(p)", p)
c109 = cast("os.getcwd()", p)
c110 = cast("os.path.exists(p)", p)
c111 = cast("os.path.expanduser(p)", p)
c124 = cast("py.path.local(p)", p)

# S2
e101: "os.chmod('a\\tb', 0o644)"
e106: "os.rmdir('a\\tb')"
e107: "os.remove('a\\tb')"
e108: "os.unlink('a\\tb')"
e109: "(os.getcwd(), 'a\\tb')"
e110: "os.path.exists('a\\tb')"
e111: "os.path.expanduser('a\\tb')"
e124: "py.path.local('a\\tb')"

# S3 (the all-rule rerun is shown separately below)
s101: "os.chmod(" "p, 0o644)"
s106: "os.rmdir(" "p)"
s107: "os.remove(" "p)"
s108: "os.unlink(" "p)"
s109: "os.getcwd(" ")"
s110: "os.path.exists(" "p)"
s111: "os.path.expanduser(" "p)"
s124: "py.path.local(" "p)"

# S4
def forward_scope():
    os = None
    py = None
    f101: "os.chmod(p, 0o644)"
    f106: "os.rmdir(p)"
    f107: "os.remove(p)"
    f108: "os.unlink(p)"
    f109: "os.getcwd()"
    f110: "os.path.exists(p)"
    f111: "os.path.expanduser(p)"
    f124: "py.path.local(p)"

# S5
import os as late_os
import py as late_py
def deferred_scope():
    late_os.chmod(p, 0o644)
    late_os.rmdir(p)
    late_os.remove(p)
    late_os.unlink(p)
    late_os.getcwd()
    late_os.path.exists(p)
    late_os.path.expanduser(p)
    late_py.path.local(p)
late_os = None
late_py = None
```

Ruff-only output for S1, S2, and S4; the translation emitted nothing:

```text
PTH101:7  PTH106:8  PTH107:9  PTH108:10  PTH109:11  PTH110:12  PTH111:13  PTH124:14
PTH101:17 PTH106:18 PTH107:19 PTH108:20  PTH109:21  PTH110:22  PTH111:23  PTH124:24
PTH101:40 PTH106:41 PTH107:42 PTH108:43  PTH109:44  PTH110:45  PTH111:46  PTH124:47
```

Translation-only output for S3 (apart from `PTH109:31`, which both tools reported) and S5:

```text
PTH101:27 PTH106:28 PTH107:29 PTH108:30 PTH110:32 PTH111:33 PTH124:34
PTH101:53 PTH106:54 PTH107:55 PTH108:56 PTH109:57 PTH110:58 PTH111:59 PTH124:60
```

`concat.py` (S3):

```python
import os
import py
p = "x"
s101: "os.chmod(" "p, 0o644)"
s106: "os.rmdir(" "p)"
s107: "os.remove(" "p)"
s108: "os.unlink(" "p)"
s109: "os.getcwd(" "p)"
s110: "os.path.exists(" "p)"
s111: "os.path.expanduser(" "p)"
s124: "py.path.local(" "p)"
```

```text
Ruff:        no findings
Translation: PTH101:4 PTH106:5 PTH107:6 PTH108:7 PTH109:8 PTH110:9 PTH111:10 PTH124:11
```

## PTH101-specific defects

`replaced_nested_class` does not reproduce Ruff's latest-binding lookup, and PTH101 applies ordinary-tree argument helpers to parsed type trees.

`pth101.py`:

```python
import os

class Redefined:
    class B:
        fd = "path"
    class B:
        fd = 3
os.chmod(Redefined.B.fd, 0o644)

class Rebound:
    fd = 3
Rebound = None
os.chmod(Rebound.fd, 0o644)

literal_fd: "os.chmod(3, 0o644)"
dir_fd: "os.chmod('f', 0o644, dir_fd=3)"
```

```text
Ruff:        PTH101:13
Translation: PTH101:8 PTH101:15 PTH101:16
```

- Line 8 is a false positive: Ruff resolves the latest nested `B`, whose `fd` is an integer. `replaced_nested_class` reports whenever a later binding follows an earlier nested class, even when that later binding is the class Ruff resolves.
- Line 13 is a miss: after `Rebound = None`, Ruff can no longer prove that `Rebound.fd` is an integer. `pth_class_of` follows the earlier class definition and suppresses the call. Rebinding an outer class or a deeper parent has the same failure.
- Lines 15 and 16 are false positives: Ruff suppresses an integer path and a non-`None` `dir_fd` inside quoted annotations. `fd_arg` and `kw_set` do not inspect parsed type trees.

## Per-rule verdicts

- `ruff-pth101-os-chmod`: **defects** — shared S1–S5 plus all four PTH101-specific mismatches above.
- `ruff-pth106-os-rmdir`: **defects** — shared S1–S5.
- `ruff-pth107-os-remove`: **defects** — shared S1–S5.
- `ruff-pth108-os-unlink`: **defects** — shared S1–S5.
- `ruff-pth109-os-getcwd`: **defects** — shared S1–S5.
- `ruff-pth110-os-path-exists`: **defects** — shared S1–S5.
- `ruff-pth111-os-path-expanduser`: **defects** — shared S1–S5.
- `ruff-pth124-py-path`: **defects** — shared S1–S5.

## Author result comparison

`python-21.result.json` marks all eight rules `same`; it contains no `different` entries, so there were no `right` samples to validate. The remote counterexamples above contradict each `same` claim.
