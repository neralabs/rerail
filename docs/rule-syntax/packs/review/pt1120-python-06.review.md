# pt1120-python-06 translation review

Ground truth: Bandit 1.9.4 (Python 3.15.2) and Ruff 0.17.6 on `junction-pt-w07`. I read each test's source
(`bandit/plugins/general_bad_file_permissions.py`, `try_except_pass.py`, `core/blacklisting.py`,
`blacklists/imports.py`, `core/context.py`, `core/node_visitor.py`, `core/utils.py`; Ruff's
`rules/airflow/rules/{task_variable_name,moved_to_provider_in_3,suggested_to_update_3_0,suggested_to_move_to_provider_in_3}.rs`,
`rules/airflow/helpers.rs`, `rules/numpy/helpers.rs`, `ruff_python_semantic`'s `Exceptions::from_try_stmt` and the
checker's deferral of annotations). The translations ran with the `#1120` engine (`rerail-engine-1120`,
sha256 `aa28dc0b…`) through `run-policies` at `86f9dbed03`. Bandit ran with all tests enabled unless noted; Ruff ran
`--isolated --no-cache --select AIR001,AIR302,AIR311,AIR312`.

Ruff's own fixtures `AIR001.py`, `AIR302_*.py`, `AIR311_*.py` and `AIR312.py` (31 files) agree line for line:
252 (path, line, code) keys on both sides.

| Rule | Verdict |
|---|---|
| `bandit-b103-set-bad-file-permissions` | ok |
| `bandit-b110-try-except-pass` | 1 defect |
| `bandit-b405-import-xml-etree` | 1 defect |
| `bandit-b406-import-xml-sax` | 1 defect |
| `ruff-air001-airflow-variable-name-task-id-mismatch` | 2 own defects, plus binding defects A and B |
| `ruff-air302-airflow3-moved-to-provider` | binding defects A–E |
| `ruff-air311-airflow3-suggested-update` | binding defects A–E |
| `ruff-air312-airflow3-suggested-to-move-to-provider` | binding defects A–E |

## `bandit-b103-set-bad-file-permissions`: ok

Counterexample attempts in `b103.py`, all matching Bandit (lines 5, 6, 8–14, 17–21, 24, 25, 27, 28, 30, 31 on both
sides, with the same HIGH/MEDIUM split; 19–21 are MEDIUM):

```python
import os
from os import chmod as setmode
from somewhere import evil as mychmod

os.chmod("a", 0o777)
setmode("b", 0o777)                      # alias to os.chmod: reported
mychmod("c", 0o777)                      # Bandit's name is "evil": not reported
os.chmod("d", 0O777)
os.chmod("e", 0x1ff)
os.chmod("f", 0b111111111)
os.chmod("g", 0o7_7_7)
os.chmod("h", 1_023)
os.chmod("i", 0xFFFFFFFFFFFFFFFFFFFF)
os.chmod("j", 0o777, follow_symlinks=False)
os.chmod("k", *[0o777])                  # not reported: one positional
os.chmod("l", True)                      # not reported: Bandit reads "True"
os.chmod("m", (0o777))
os.chmod(*["n"], 0o777)
os.chmod("o", 18446744073709551633)      # MEDIUM
os.chmod("p", 0o20)                      # MEDIUM
os.chmod("q", 0o1)                       # MEDIUM
os.chmod("r", 0o644)                     # not reported
f()("s", 0o777)                          # not reported: name ""
f().chmod("t", 0o777)
os.fchmod(3, 0o777)
os.chmod("u", 511j)                      # not reported
os.chmod("v", 0x_1ff)
os.chmod("w", 0o0000000000000777)
os.chmod("x", 000)                       # not reported
os.chmod("y", 0o00000000000000000000002)
os.chmod("z", 123456789012345678901234567890)
```

## `bandit-b110-try-except-pass`: defect

PEP 758 (Python 3.14) allows an unparenthesized tuple of handler types. `ast` reads it as a `Tuple`, which has no
`id`, so Bandit skips the handler. The translation (python/security's `catch_all`) reads the first element as the
handler's type.

Counterexample `b110_758.py`:

```python
try:
    a()
except Exception, ValueError:
    pass
try:
    a()
except* Exception, ValueError:
    pass
```

```text
Bandit:      no issues
Translation: b110_758.py:3 B110
             b110_758.py:7 B110
```

The cause is in `security.rerail`'s `catch_all`/`except_type`, so B112 (same helper) inherits it.

The rest of `b110.py` agrees (B110 at 5, 13, 17, 21, 28, 40, 44 on both sides): `except (Exception):`,
`except* Exception:`, `except Exception as e:`, `except:`, a comment before `pass`, `except ((Exception)) as e:`,
and `except Exception: pass;`; not `builtins.Exception`, `pass; pass`, `...`, or `(Exception, ValueError)`.

## `bandit-b405-import-xml-etree`: defect

The revision strips spaces and line continuations from the import *entry* (`b405_hit`), but not from the alias
table Bandit also reads: `security.rerail`'s `alias_site` keeps the dotted name's text, so a call through an alias
of `xml . etree . cElementTree` qualifies as `"xml . etree.cElementTree"` and misses Bandit's call blacklist
(`call_function_name_qual == "xml.etree.cElementTree"`).

Counterexample (lines 4–5 of `b40x.py`):

```python
from xml . etree import cElementTree as E
E()
```

```text
Bandit:      b40x.py:4 B405
             b40x.py:5 B405
Translation: b40x.py:4 B405
```

## `bandit-b406-import-xml-sax`: defect

Same cause as B405: aliases keep spaces and continuations.

Counterexample (lines 2–3 and 14–16 of `b40x.py`):

```python
import xml . sax as s
s()
...
import xml.\
    sax as s3
s3()
```

```text
Bandit:      b40x.py:2 B406, b40x.py:3 B406, b40x.py:14 B406, b40x.py:16 B406
Translation: b40x.py:2 B406,                 b40x.py:14 B406
```

The rest of `b40x.py` agrees: relative `from .xml import sax` and `from . xml . sax import x`, the prefix quirk
`from xml import saxutils` and `import xml.etree.ElementTreeX`, `__import__("xml.sax")`,
`importlib.import_module("xml.etree.ElementTree")`, `from xml import sax as s2; s2()`, `from xml.sax import *`, and
`import pickle, xml.sax` (B403 only, with all tests enabled).

## Airflow rules: binding defects shared through `lib/python/pt1120-python-06.rerail`

Each counterexample below names an AIR302 (`S3Hook`), an AIR311 (`Dataset`) and an AIR312 (`BashOperator`) name,
and AIR001 where a call applies.

### A. `del` does not end a binding

Ruff resolves a read after `del` to the deletion (unbound) and reports nothing. `air_cur` picks the latest row of
python/scopes' `binding`. Deletions aren't in that relation (scopes keeps them apart, in `del_site`), so the import
stays visible.

`d1_del.py`:

```python
from airflow.hooks.S3_hook import S3Hook
from airflow.datasets import Dataset
from airflow.operators.bash import BashOperator
from airflow.operators.bash import BashOperator as Op

del S3Hook, Dataset, BashOperator, Op
S3Hook()
Dataset("s3://x")
BashOperator()
t = Op(task_id="u")
```

```text
Ruff:        no diagnostics
Translation: d1_del.py:7 AIR302, :8 AIR311, :9 AIR312, :10 AIR001, :10 AIR312
```

### B. An `except … as name` handler unbinds back to the earlier binding in Ruff

After the handler, Ruff's binding is `UnboundException(Some(previous))` and resolves to the import before it, so it
reports. The translation takes the `as` target as the latest binding (not an import) and reports nothing.

`d2_except_as.py`:

```python
from airflow.hooks.S3_hook import S3Hook
from airflow.datasets import Dataset
from airflow.operators.bash import BashOperator
from airflow.operators.bash import BashOperator as Op

try:
    pass
except ValueError as S3Hook:
    pass
except TypeError as Dataset:
    pass
except KeyError as BashOperator:
    pass
except OSError as Op:
    pass
S3Hook()
Dataset("s3://x")
BashOperator()
t = Op(task_id="u")
```

```text
Ruff:        d2_except_as.py:16:1 AIR302, :17:1 AIR311, :18:1 AIR312, :19:1 AIR001, :19:5 AIR312
Translation: no diagnostics
```

### C. Annotations Ruff defers: `from __future__ import annotations`, `.pyi` stubs, PEP 695 aliases and bounds

Ruff defers these (`visit_expr` pushes them to `future_type_definitions` or the type-parameter queue) and resolves
them once the module is done, against the module's last binding. `air_late` defers only function and lambda bodies,
so the translation resolves them in order. The author's notes list the `__future__` case as "not modelled"; stubs
and PEP 695 aliases and bounds go unmentioned and fail the same way, even without the future import.

`d3_future.py`:

```python
from __future__ import annotations

from airflow.hooks.S3_hook import S3Hook
from airflow.datasets import Dataset
from airflow.operators.bash import BashOperator


def f(a: S3Hook, b: Dataset, c: BashOperator) -> None: ...


from airflow.providers.amazon.aws.hooks.s3 import S3Hook
from airflow.sdk import Asset as Dataset
from airflow.providers.standard.operators.bash import BashOperator
```

`d4_type_alias.py` (same imports and rebinding, without the future import):

```python
type Hooks = S3Hook | Dataset | BashOperator


def g[T: (S3Hook, Dataset, BashOperator)](x: T) -> T: ...
```

`d7_stub.pyi` (same imports and rebinding, no future import):

```python
def f(a: S3Hook, b: Dataset, c: BashOperator) -> None: ...

BashOperator()
```

```text
Ruff:        d7_stub.pyi:7:1 AIR312            (the call: not a type definition)
Translation: d3_future.py:8 AIR302, AIR311, AIR312
             d4_type_alias.py:5 AIR302, AIR311, AIR312
             d4_type_alias.py:8 AIR302, AIR311, AIR312
             d7_stub.pyi:5 AIR302, AIR311, AIR312
             d7_stub.pyi:7 AIR312
```

### D. The import guard (name form): Ruff takes the innermost `try` while the binding is in any handler

`is_guarded_by_try_except` requires only that the binding was made inside some except handler
(`binding.in_exception_handler()`), then reads the innermost `try` around the import: its handlers and its body.
`air_safe` requires the import to sit in a handler of its innermost `try`, so an import in the body of a `try` nested
in a handler is never guarded.

`d5_guard_name.py`:

```python
try:
    pass
except ImportError:
    try:
        from airflow.providers.amazon.aws.hooks.s3 import S3Hook
        from airflow.hooks.S3_hook import S3Hook
        from airflow.sdk import Asset
        from airflow.datasets import Dataset as Asset
        from airflow.providers.standard.operators.bash import BashOperator
        from airflow.operators.bash import BashOperator
    except ImportError:
        pass
S3Hook()
Asset("s3://x")
BashOperator()
```

```text
Ruff:        no diagnostics
Translation: d5_guard_name.py:13 AIR302, :14 AIR311, :15 AIR312
```

### E. The import guard (attribute form): the same, through `air_in_handler`

For an attribute Ruff needs `semantic.in_exception_handler()` and reads the innermost `try` among the current
statements, wherever in it the read sits. `air_in_handler` requires the read in a handler of its innermost `try`.

`d6_guard_attr.py`:

```python
import airflow

try:
    pass
except AttributeError:
    try:
        airflow.providers.amazon.aws.hooks.s3.S3Hook
        airflow.hooks.S3_hook.S3Hook
        airflow.sdk.Asset
        airflow.datasets.Dataset
        airflow.providers.standard.operators.bash.BashOperator
        airflow.operators.bash.BashOperator
    except AttributeError:
        pass
```

```text
Ruff:        no diagnostics
Translation: d6_guard_attr.py:8 AIR302, :10 AIR311, :12 AIR312
```

Cases that agree: a function's own imports read in order (`a1_local.py`), a local bare annotation skipped to the
module's import (`a4_ann.py`), relative `from .airflow… import` unresolved (`a5_relative.py`), and AIR311's keyword
checks (`sla_miss_callback=` on `airflow.sdk.dag` and `airflow.DAG`, `sla=`, `datasets=`; `a9_air311.py`).

## `ruff-air001-airflow-variable-name-task-id-mismatch`: defects

Besides A and B above:

`d8_air001.py`:

```python
from airflow.operators.bash import BashOperator

a = b = BashOperator(task_id="c")
(d) = BashOperator(task_id="e")
```

```text
Ruff:        d8_air001.py:4:2 AIR001
Translation: d8_air001.py:3 AIR001
```

- Line 3: Ruff skips an assignment with more than one target (`let [target] = targets else { return }`).
  tree-sitter nests `b = …` as the right side of `a = …`, and the translation reports the inner one.
- Line 4: `ast` drops the parentheses, so the target is the Name `d`. The translation requires `left` to be an
  `identifier`, but tree-sitter gives a `parenthesized_expression` and the rule misses it.

Agreeing cases in `a8_air001.py`: implicit concatenation `"g" "h"`, `u"x"`, `(BashOperator)(…)`, and
`task_id=("o")` report; f-strings and bytes don't.

## General note, all Bandit rules

Bandit skips a file `ast` cannot parse (`py2/b_py2.py`: "syntax error while parsing AST from file"), and the
translations still report in it (B110 and B406 there). This affects every Bandit translation, not just this batch.

## The author's result file

I checked every `different` entry's `right: translation` against Bandit on the sample keys the notes name. All of
them hold; the translation reports exactly these keys too:

| Rule | Sample keys | Bandit |
|---|---|---|
| B110 | `ruff/rerail/noqa/E722.py` 8, 13, 18, 23 (`except:  # noqa`); `ruff/flake8_bugbear/B012.py` 22 (in a `finally`) | B110 on each |
| B405 | `N817-import-convention-near-misses.py` 3, 5, 6, 8; `UP023-line-continuation-in-dotted-name.py` 1, 3, 5, 7 | B405 on each |
| B406 | samples-python `django/utils/xmlutils.py` 5 (`from xml.sax.saxutils import XMLGenerator`) | B406 |

The four Airflow entries say `same`. That holds on the corpora (and on Ruff's fixtures above), but defects A–E show
the translations still differ from Ruff outside them.
