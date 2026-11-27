# pt1120-python-08 translation review

Ground truth: Ruff 0.17.6 on `junction-pt-w07`, run `--isolated --no-cache --target-version py313 --select
SLOT000,SLOT001,SLOT002,TD002,TD003,TD004`. I read Ruff's sources at the 0.17.6 tag: `flake8_slots/rules/*.rs` and
`helpers.rs`, `flake8_todos/rules/todos.rs`, `directives.rs` (`TodoDirective::from_comment`), the checker's
`visit_stmt` (a `ClassDef` is analysed after its body, with its own name already bound; function bodies are deferred
to after the module), `SemanticModel::match_builtin_expr`, `lookup_attribute` and `analyze::class::is_enumeration`.
The translations ran with the `#1120` engine (`rerail-engine-1120`, sha256 `aa28dc0b…`) through `run-policies` at
`40954ebd31`. The JSON originals were not run; where a finding says what the original does, it is read from the JSON.

Ruff's own fixtures (`SLOT000.py`, `SLOT001.py`, `SLOT002.py`, `TD002.py`, `TD003.py`, `TD004.py`) agree line for
line: 64 (file, line, code) keys on both sides.

| Rule | Verdict |
|---|---|
| `ruff-slot000-no-slots-in-str-subclass` | 2 defects (A cross-file enum ancestry, B binding order) |
| `ruff-slot001-no-slots-in-tuple-subclass` | 1 defect (B binding order) |
| `ruff-slot002-no-slots-in-namedtuple-subclass` | 1 defect (parenthesised callee) |
| `ruff-td002-missing-todo-author` | 1 defect (C non-ASCII tag boundary) |
| `ruff-td003-missing-todo-link` | 2 defects (C, and non-ASCII digits in links) |
| `ruff-td004-missing-todo-colon` | 1 defect (C) |

Cases that agree with Ruff, kept as checks: `class str(str)` and `class tuple(tuple)` (neither side reports: Ruff
binds the class name before it analyses the class), a class nested in a class body that rebinds `str` after it
(both report), every `__slots__` shape below, `typing_extensions.Tuple[int]`, `((tuple))`, `Tuple[int][0]`,
`builtins.object` and `*()` beside a namedtuple call, `typing_extensions.NamedTuple(...)` (neither side reports),
and `# TODO : fix`, `# TODO :fix`, `# TODO :`, `# TODO(a) : fix` (Ruff trims before the colon test when there is no
author: TD002 only on the first three, TD004 only on the last).

```python
class F(str):
    __slots__ += ()                 # both: SLOT000 (AugAssign)
class G(str):
    pass; __slots__ = ()            # neither
class H(str):
    __slots__: tuple                # neither
class I(str):
    (__slots__) = ()                # neither
class J(str):
    [__slots__] = ()                # both: SLOT000
class K(str):
    __slots__, = ()                 # both: SLOT000
class L(str):
    x = __slots__ = ()              # neither
class M(str):
    __slots__ = x = ()              # neither
class N(str):
    if True:
        __slots__ = ()              # both: SLOT000
class O(str):
    (__slots__): tuple = ()         # neither
```

## Defect A (SLOT000): an enum base is followed into other files

Ruff's `is_enumeration` asks two things of each base (one subscript off): does its qualified name resolve to one of
`enum.Enum`, `Flag`, `IntEnum`, `IntFlag`, `StrEnum`, `ReprEnum`, `CheckEnum`; and is it bound in this file to a
class definition (`lookup_attribute` gives a `BindingKind::ClassDefinition`), whose bases are asked the same. An
imported class is an import binding: Ruff stops there. The translation's third `slot000_enum_ref` rule follows the
base's name through `py_names__ancestor`, which crosses into other modules of the project, so a `str` mix-in of an
enum declared in another file is exempted. That is the common shape of a shared enum base (`from .base import
BaseEnum; class Color(str, BaseEnum)`). The original's `enum-name` takes only an import of `enum.*` or a class of
the file, so it reports these, as Ruff does.

Counterexample (corpus root holds all five files):

```python
# colors.py
from enum import Enum


class Base(Enum):
    pass
```

```python
# s0_cross.py
from colors import Base


class A(str, Base):
    pass
```

```python
# s0_cross_mod.py
import colors


class A(str, colors.Base):
    pass
```

```python
# s0_cross_mid.py
from colors import Base


class Mid(Base):
    pass


class A(str, Mid):
    pass
```

```python
# pkg/__init__.py is empty; pkg/colors.py:
import enum


class Base(enum.Enum):
    pass
# pkg/uses.py:
from .colors import Base


class A(str, Base):
    pass
```

| File | Ruff | Translation |
|---|---|---|
| `s0_cross.py` | 4:7 SLOT000 | nothing |
| `s0_cross_mod.py` | 4:7 SLOT000 | nothing |
| `s0_cross_mid.py` | 8:7 SLOT000 | nothing |
| `pkg/uses.py` | 4:7 SLOT000 | nothing |

Fix: drop the `py_names__ancestor` rule; a base not bound to a class of the file is an enum only by its own symbol
(`sym_of(e, q), slot000_enum_name(q)`).

## Defect B (SLOT000, SLOT001): names read against bindings Ruff has not seen yet

Ruff runs the SLOT rules when the class statement has been visited. `match_builtin_expr` (before `builtins` is
imported) asks whether `str`/`tuple` is still the builtin *in the scopes as visited so far*, and
`is_enumeration`'s `lookup_attribute` takes the binding visited last. At module level that is the bindings before
the class; in a function, whose body Ruff visits after the whole module, it is every module binding plus the
function's own bindings before the class. Python agrees for the module-level cases: the base is evaluated when the
class statement runs.

The library's `slots_tp_builtin` models this order for a type parameter named `str`/`tuple`, but the plain path
(`sym_of(b, "builtins.str")`, `sym_of(e, "builtins.tuple")`) and the enum lookup read the name flow-insensitively,
and `slots_tpb_builtins` reads an `import builtins` anywhere in the file, where Ruff's `seen_module(BUILTINS)` holds
only for imports visited before the class.

```python
# s0_after.py
class A(str):
    pass


str = 1
```

```python
# s1_after.py
class A(tuple):
    pass


tuple = list
```

```python
# s0_func_after.py
def f():
    class A(str):
        pass

    str = int
```

```python
# s0_rebind_after_func.py
from enum import Enum


class Base(Enum):
    pass


def f():
    class A(str, Base):
        pass


Base = int
```

```python
# s0_enum_after_mod.py
from enum import Enum


class A(str, Base):
    pass


class Base(Enum):
    pass
```

```python
# s0_tp_import_after.py
class A[str](str):
    pass


import builtins
```

```python
# s1_tp_import_after.py
class A[tuple](tuple):
    pass


import builtins
```

| File | Ruff | Translation |
|---|---|---|
| `s0_after.py` | 1:7 SLOT000 | nothing |
| `s1_after.py` | 1:7 SLOT001 | nothing |
| `s0_func_after.py` | 2:11 SLOT000 | nothing |
| `s0_rebind_after_func.py` | 9:11 SLOT000 | nothing |
| `s0_enum_after_mod.py` | 4:7 SLOT000 | nothing |
| `s0_tp_import_after.py` | 1:7 SLOT000 | nothing |
| `s1_tp_import_after.py` | 1:7 SLOT001 | nothing |

`s0_after.py`, `s1_after.py` and `s0_rebind_after_func.py` are valid programs whose base at run time is the one Ruff
names; the rest are edge cases (`s0_enum_after_mod.py` raises `NameError`). A control that agrees:
`s0_enum_after_func.py` (`Base(Enum)` defined at module level after the function that uses it) is exempt on both
sides. The original's `resolves: global` and its anywhere-in-the-module `builtins` test read the same way as the
translation [INFERENCE: not run], so this defect is likely shared with it.

## `ruff-slot002-no-slots-in-namedtuple-subclass`: a parenthesised callee is missed

Ruff's AST has no parentheses: `(namedtuple)("A", "x")` is a call of `namedtuple`. `slot002_call` asks
`sym_of(f, …)` of the call's `function` field as tree-sitter gives it, a `parenthesized_expression`, so the base is
no namedtuple call and, being no `object` either, clears the class. The original's `nt-func` takes the
parenthesised form; the translation lost it.

```python
# s2.py (excerpt; line 7)
from collections import namedtuple


class A((namedtuple)("A", "x")):
    pass
```

Ruff: `s2.py:7:7 SLOT002`. Translation: nothing. Fix: `unparen` the function before `sym_of`.

## Defect C (TD002, TD003, TD004): a tag followed by a non-ASCII non-alphanumeric

Ruff's `from_comment` ends the first word at the first character that is not `char::is_alphanumeric` (Unicode).
misc2-todo's `todo_tag` (and `todo_at`) accept only ASCII punctuation or a space after the tag, so a full-width colon
(common in CJK comments) or a no-break space hides the directive and all three rules miss it. misc2-todo's header says
so ("Word characters are ASCII here; Ruff's are Unicode"); the original's boundary is `[^\p{Alphabetic}\p{N}]`.

```python
# t.py, lines 21 and 23
# TODO：fix this
# TODO fix this          (U+00A0 after the tag)
```

Ruff: TD002, TD003 and TD004 at 21:3 and 23:3. Translation: nothing on either line.

## `ruff-td003-missing-todo-link`: non-ASCII digits are links to Ruff

Ruff's link regexes use `\d`, which is Unicode in the `regex` crate (`#\d+` on the TODO's line, `^#\s*\d+$` and
`^#\s*[A-Z]+\-?\d+$` below it). misc2-todo's `todo_linked` and `todo_link_line` use `[0-9]`.

```python
# t.py, lines 13-19
# TODO(a): fix #١٢

# TODO(a): fix
# ١٢٣

# TODO(a): fix
# ABC-١٢
```

Ruff: no TD003 on lines 13, 15, 18. Translation: TD003 at 13, 15 and 18. Fix: `\d` (or `\p{Nd}`) in the three link
patterns.

## The author's result file

TD002, TD003, TD004 `different`, `right: original`, key `ruff/rerail/probes/FIX002-unicode-word-boundaries.py:3`
(`# TODO→ arrow`): Ruff 0.17.6 on the recorded corpus file reports TD002, TD003 and TD004 at 3:3. The original is
right; this is Defect C, still open on the branch (its fix is only proposed in the NOTES).

SLOT000–SLOT002 `same`: the suite compares with the original, so Defect A and the SLOT002 defect, where the
translation departs from the original, are absent from the corpora; Defect B is likely shared. One claim in the NOTES is wrong
against Ruff: "With `import builtins` anywhere in the file, `class A[tuple](tuple)` is not reported (Ruff and the
original)". Ruff reports it when the import comes after the class at module level (`s1_tp_import_after.py`, Defect
B).
