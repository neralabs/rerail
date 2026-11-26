# Python 11 rule review

Ground truth: Ruff 0.17.6, its official rule documentation, and the v0.17.6 implementations and fixtures for UP014, UP015, and UP018–UP023. Every counterexample below was run on `junction-pt-w01` with both Ruff and `rerail-engine run-policies`; findings are compared by path, line, and code. The known suppression-comment engine gap is excluded.

`python-11.result.json` marks all eight rules `same`; it contains no `different` entries whose `right` side needs validation.

## `ruff-up014-convert-named-tuple-functional-to-class` — defects

### Parentheses are not consistently removed

```python
from typing import NamedTuple

A = (NamedTuple)("A", [("x", int)])
B = (NamedTuple("B", [("x", int)]))
(C) = NamedTuple("C", [("x", int)])
D = NamedTuple("D", [(("x", int))])
E = NamedTuple("E", [(('x'), int)])
```

Observed:

- Ruff: UP014 on lines 3–7.
- Translation: no diagnostic on any line.

`functional_def` requires the raw target and assignment value to be an identifier and call, and passes the raw callee to `typing_ref`. `good_pair` similarly requires the raw field node to be a tuple and its name to be a string. Ruff's Python AST has already discarded these parentheses.

### Line continuations become arguments or field elements

```python
from typing import NamedTuple

B = NamedTuple("B", \
    [("a", int)])
C = NamedTuple("C", [("a", int), \
    ("b", str)])
D = NamedTuple("D", [("a", \
    int)])
```

Observed:

- Ruff: UP014 on lines 3, 5, and 7.
- Translation: no diagnostics.

`up_arglist`, `bad_pair`, and `good_pair` exclude comments but not `line_continuation` nodes.

### `_typeshed.NamedTuple` is not recognized

```python
from _typeshed import NamedTuple as NT

D = NT("D", [("x", int)])
```

Observed: Ruff reports `3:1 UP014`; the translation reports nothing. `typing_ref` admits only `typing` and `typing_extensions`, while Ruff resolves this `_typeshed` export too.

### Starred first arguments are not counted as Ruff counts them

```python
from typing import NamedTuple

a = ()
A = NamedTuple(*a)
B = NamedTuple(*(a))
C = NamedTuple(*a, x=int)
```

Observed: Ruff reports UP014 on lines 4–6; the translation reports nothing. Ruff's matcher receives the starred expression in its positional `args` slice; `up_arg` excludes list splats.

### Workspace re-exports cause a false positive

```python
# reex.py
from typing import NamedTuple

# use.py
from reex import NamedTuple
X = NamedTuple("X", a=int)
```

Observed: Ruff reports no diagnostic in `use.py`; the translation reports `use.py:2 UP014`. `sym_at` follows workspace re-exports through `resolve_export`, unlike Ruff's file-local import resolution.

## `ruff-up015-redundant-open-modes` — defects

### Parenthesized callees are missed

```python
import aiofiles
import builtins

(open)("a", "r")
(builtins.open)("b", "U")
(aiofiles.open)("c", "rt")
```

Observed: Ruff reports UP015 at each mode string; the translation reports nothing. `open_call` passes the raw parenthesized function node to `builtin_at`/`sym_at`.

### Adjacent mode strings inside a string type definition are missed

```python
x: 'open("f", "r" "t")'
```

Observed: Ruff reports `1:15 UP015`; the translation reports nothing. `td_str` accepts only a single string piece, while Ruff sees one folded string literal.

### Workspace re-exports cause a false positive

```python
# reex.py
from aiofiles import open as aopen

# use.py
from reex import aopen
aopen("f", "rt")
```

Observed: Ruff reports no diagnostic; the translation reports `use.py:2 UP015` because `sym_at` follows the re-export.

The mode parser itself matched Ruff on valid, invalid, reducible, keyword, splat, alias, and string-annotation cases.

## `ruff-up018-native-literals` — defects

### Backslash line continuations are counted as arguments

```python
str(\
)
bytes(\
)
int(\
    1)
```

Observed: Ruff reports UP018 on lines 1, 3, and 5; the translation reports nothing. `up_arglist` excludes comments but not `line_continuation` nodes, so `up_argc` sees the wrong arity.

### A sign before a parenthesized number is missed

```python
int(-(1))
float(+(1.0))
complex(-(1j))
x: "int(+(1))"
```

Observed: Ruff reports UP018 on all four lines; the translation reports nothing. The unary branch of `lit` sends the raw operand to `num_lit` instead of removing parentheses first.

### Workspace re-exports cause a false positive

```python
# reex.py
from builtins import str

# use.py
from reex import str
str("a")
```

Observed: Ruff reports no diagnostic; the translation reports `use.py:2 UP018` because `builtin_at` follows the re-export.

Other exercised forms matched: all six constructors, builtin aliases, parenthesized callees, accepted keywords, splats, nested f-strings, and calls in string type definitions.

## `ruff-up019-typing-text-str-alias` — defects

### Qualified names in match patterns are missed

```python
import typing

def f(value):
    match value:
        case typing.Text():
            pass
        case typing.Text:
            pass
```

Observed: Ruff reports `5:14 UP019` and `7:14 UP019`; the translation reports nothing. Tree-sitter represents these references as `dotted_name` inside `case_pattern`, but UP019 only accepts identifiers and attributes. The JSON original has a dedicated pattern branch.

### Workspace re-exports cause a false positive

```python
# reex.py
from typing import Text

# use.py
import typing
from reex import Text
x: Text
```

Observed: Ruff reports no diagnostic; the translation reports `use.py:3 UP019`. The explicit `typing` import satisfies Ruff's module-presence guard, so the mismatch is name resolution: `sym_at` follows `reex.Text` to `typing.Text`, while Ruff does not.

Ordinary imports and aliases, `typing_extensions.Text`, runtime uses, and nested string annotations matched.

## `ruff-up020-open-alias` — defects

### Parenthesized callees are missed

```python
import io
from io import open as io_open

(io.open)("a")
(io_open)("b")
```

Observed: Ruff reports UP020 on lines 4 and 5; the translation reports nothing. UP020 passes the raw function node to `sym_at`.

### Workspace re-exports cause a false positive

```python
# reex.py
from io import open

# use.py
from reex import open as io_open
io_open("f")
```

Observed: Ruff reports no diagnostic; the translation reports `use.py:2 UP020`.

Plain calls, direct aliases, shadowing, and string type definitions matched.

## `ruff-up021-replace-universal-newlines` — defects

### Parenthesized callees are missed

```python
import subprocess
from subprocess import run

(subprocess.run)("a", universal_newlines=True)
(run)("b", universal_newlines=True, text=False)
```

Observed: Ruff reports UP021 at each `universal_newlines`; the translation reports nothing. `un_kw` passes the raw parenthesized function node to `sym_at`.

### Workspace re-exports cause a false positive

```python
# reex.py
from subprocess import run

# use.py
import subprocess
from reex import run
run(["ls"], universal_newlines=True)
```

Observed: Ruff reports no diagnostic; the translation reports `use.py:3 UP021`.

Plain calls, direct aliases, duplicate `text=`, shadowing, and string type definitions matched.

## `ruff-up022-replace-stdout-stderr` — defects

### Parenthesized callees are missed

```python
import subprocess
from subprocess import PIPE, run

(subprocess.run)("a", stdout=PIPE, stderr=PIPE)
(run)("b", stdout=PIPE, stderr=PIPE, capture_output=False)
```

Observed: Ruff reports UP022 on lines 4 and 5; the translation reports nothing. `piped` passes the raw parenthesized function node to `sym_at`.

### Workspace re-exports cause a false positive

```python
# reex.py
from subprocess import PIPE, run

# use.py
import subprocess
from reex import PIPE, run
run(["ls"], stdout=PIPE, stderr=PIPE)
```

Observed: Ruff reports no diagnostic; the translation reports `use.py:3 UP022`.

Plain calls, direct aliases, parenthesized PIPE values, `capture_output=` presence, shadowing, and string type definitions matched.

## `ruff-up023-deprecated-c-element-tree` — ok

Ruff and the translation agreed on all exercised forms:

```python
from xml.etree.cElementTree import XML
import xml.etree.cElementTree as ET
from xml.etree import cElementTree as CET
import contextlib, xml.etree.cElementTree as ET2
import xml.etree.cElementTree, xml.etree.cElementTree as ET3
from xml.\
 etree.cElementTree import SubElement
import xml.\
 etree.cElementTree as ET4
from xml.etree import (cElementTree as CET2, ElementTree)
```

Both reported UP023 on all eight relevant import sites. Both skipped the bare `import xml.etree.cElementTree`, bare `from xml.etree import cElementTree`, and relative-import forms required by Ruff's implementation.
