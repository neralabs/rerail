# pt1120-python-04 translation review

Ground truth: Ruff 0.17.6 (`ruff check --isolated --no-cache --target-version py310 --select <code>`). I read the rules
at the 0.17.6 tag (`pyupgrade/rules/{super_call_with_parameters,extraneous_parentheses,deprecated_import,
outdated_version_block,unnecessary_class_parentheses,timeout_error_alias,unnecessary_default_type_args}.rs`, their
call sites in `checkers/ast/analyze/{expression,statement}.rs` and `checkers/tokens.rs`, the `DunderClassCell` scopes in
`checkers/ast/mod.rs`, `ruff_python_semantic`'s `resolve_qualified_name` and `ruff_python_ast`'s `QualifiedName`), then
ran my own cases with Ruff and `rerail-engine-1120 run-policies` on the seven `docs/rule-syntax/packs/python/<id>.rerail`
at `ea9523b0b3`, with that commit's `docs/rule-syntax` and `native/src/**/*.rerail` as the engine's root and
`RERAIL_RR_LIB` its `docs/rule-syntax/lib`. The runs are on pt-w02 (`/tmp/rv-p04`): pt-w01, pt-w07 and ts-prisma-b have
no `rerail-engine-1120`, the engine `ptcheck` uses for the #1120 refs. Translation lines are the finding's line; keys are
compared by (path, line, code). No case uses a suppression comment.

`--target-version py310` on every Ruff run: Ruff 0.17.6 defaults to 3.11 (`PythonVersion::default()` is `PY311`;
`ruff check --isolated` reports UP041 on `except asyncio.TimeoutError:` with no target given), while `python/version`
and the product default to 3.10 and `run-policies` passes no options. That is the library's and the product's
default, not this batch's; it moves UP035's 3.11 rows, UP036's minimum and UP041's asyncio alias.

What agreed, beyond the defects below:

- UP035: every `from M import X` for the eight modules Ruff looks at times every name in Ruff's tables and in
  `ruff-up035-deprecated-import.moves.rerail` (872 statements), and parenthesized, aliased, `as`, mixed, `;`-joined,
  `if True:`-inlined, relative, star and `typing . io` forms: identical at 3.10. **ok** apart from the syntax gate.
- UP039: `class A():`, `class B[T]():`, empty parentheses over lines, with a comment, with a `\` continuation, only a
  comment, `( )`, a class in a function, `*()` and `**{}` (not reported): identical. **ok** apart from the syntax gate.
- UP034: about 100 hand cases (calls, generators, `*`/`**`, f- and t-string fields, `=` and nested format specs,
  walrus, `yield`/`yield from`/`await`, comments and `\` between the pairs, empty tuples, patterns in `case`, `with`,
  `for`, `del`, decorators, defaults, annotations, bases, subscripts, dict and list items) agreed apart from the
  lambda case below, and 3,800 fuzzed statements (two generators; lines near a Ruff syntax error left out) agreed
  apart from the operator and chain cases below.
- UP041: names, attributes, `import socket as s`, `from socket import timeout`, `(s).timeout`, parenthesized and nested
  handler tuples, `as`, `except*`, PEP 758 `except a, b:`, `raise` with and without a call, a parenthesized callee,
  `raise … from`, calls outside raise, class-body and deferred-function rebinding of `socket`: identical.
- UP043 in stubs: typing, typing_extensions and collections.abc spellings, three and two arguments, `(None)`, the
  `X[(a, b)]` and `X[(a, b),]` slices, trailing commas, multi-line subscripts, a comment inside, runtime subscripts,
  nested subscripts, plain, raw and triple-quoted string annotations, the concatenated and escaped strings Ruff skips:
  identical.

Not run: UP043 in a `.py` at a 3.13 target and UP036 with `python-minor-version`; `run-policies` gives a policy no
options. Read against the source the gates (`targets(13) ; stub_file(d)`, `minimum`) are right.

## All six AST rules: no syntax-error gate

UP008, UP035, UP036, UP039, UP041 and UP043 report in a file Ruff cannot parse; Ruff reports no AST rule there.
UP034 is token-based and Ruff runs it on such files too: it agrees.

`syntax_py2.py`:

```python
import sys
import socket
from collections import Mapping
from typing import Generator
print "hello"


class A():
    def f(self):
        return super(A, self).f()


x = ((1))
if sys.version_info >= (3, 8):
    pass
try:
    pass
except socket.timeout:
    pass
```

```
ruff:        syntax_py2.py:5:7: invalid-syntax: Simple statements must be separated by newlines or semicolons
             syntax_py2.py:13:6: UP034 Avoid extraneous parentheses
translation: syntax_py2.py:3 UP035, :4 UP035, :8 UP039, :10 UP008, :13 UP034, :14 UP036, :18 UP041
```

With `print("hello")` on line 5 Ruff gives the translation's seven findings. `syntax_stub.pyi`
(`from typing import Generator` / `print "x"` / `def f() -> Generator[int, None]: ...`): Ruff only the syntax error, the
translation `:1 UP035` and `:3 UP043`.

## ruff-up008-super-call-with-parameters

### 1. The first parameter Ruff compares is the first of `args`, not of the positional-only ones

`parent_parameters.args.first()`: Ruff's `args` holds the parameters after `/`; `posonlyargs` is apart. The
translation takes `sig_param(f, _, nm, 0, pk, …)` with `pk = "posonly"` allowed.

`up008_posonly.py`:

```python
class A:
    def f(self): ...


class B(A):
    def f(self, /):
        return super(B, self).f()

    def g(self, /, x):
        return super(B, x).f()

    h = lambda self, /: super(B, self).f()
```

```
ruff:        up008_posonly.py:10:21: UP008
translation: up008_posonly.py:7, :12 UP008
```

### 2. `@dataclass(slots=True)` on the class is not excepted

Ruff skips the call when the enclosing class has a `dataclasses.dataclass(…, slots=True)` decorator (`super()` without
arguments raises a `TypeError` there). The translation has no such clause.

`up008_slots.py`:

```python
from dataclasses import dataclass


class A:
    def f(self): ...


@dataclass(slots=True)
class B(A):
    def f(self):
        return super(B, self).f()
```

```
ruff:        (nothing)
translation: up008_slots.py:11 UP008
```

### 3. A call in a nested def's decorator, default or return annotation

Ruff's function is `current_statements().find(is_function_def_stmt)`, and while it visits a def's decorators, defaults
and annotations the current statement is that def. So `super(B, self)` there is compared with the nested def's first
parameter (`x`), not the method's. The translation takes the scope the call runs in (the method).

`up008_inner_def.py`:

```python
class A:
    def f(self): ...


def deco(x):
    return lambda f: f


class B(A):
    def f(self):
        @deco(super(B, self))
        def g(x): ...

        def h(x, y=super(B, self)): ...

        def k(x) -> super(B, self).f: ...

        return g, h, k
```

```
ruff:        (nothing)
translation: up008_inner_def.py:11, :14, :16 UP008
```

### 4. `builtins.super(…)`: Ruff's `__class__` test walks the whole def

`has_local_dunder_class_var_ref` visits the def statement: its decorators, defaults and annotations, and comprehensions
in the body; only nested classes are skipped. `cell_user` counts only loads whose run scope is the function itself or a
nested function or lambda: not the def's decorators, defaults or annotations, and not a comprehension.

`up008_cell.py`:

```python
import builtins


class A:
    def f(self): ...


def deco(x):
    return x


class B(A):
    def f(self, s=super):
        return builtins.super(B, self).f()

    @deco(super)
    def g(self):
        return builtins.super(B, self).f()

    def h(self) -> __class__:
        return builtins.super(B, self).f()

    def k(self):
        [super for _ in range(1)]
        return builtins.super(B, self).f()

    def m(self):
        return builtins.super(B, self).f()
```

```
ruff:        up008_cell.py:14:30, :18:30, :21:30, :25:30 UP008
translation: (nothing)
```

(`m`, line 28, is reported by neither.)

### 5. Ruff looks for a binding of the class name in the call's own scope only

Ruff's only check that `C` names the class is the text, plus `callable_scope.has(id)`: a binding (a `global`/`nonlocal`
declaration counts) in the function or lambda the call runs in. `names_class` instead follows the name's lookup:
`look(…)` must reach a class binding, and `redeclared` refuses a `global`/`nonlocal` in any scope on the way. A
binding or a declaration in an enclosing function, or a `global` in the function that defines the class, hides a
finding Ruff gives.

`up008_scopes.py`:

```python
class A:
    def f(self): ...


class B(A):
    def f(self):
        B = 1

        def g(this):
            return super(B, this).f()

        return g

    def h(self):
        global B

        def g(this):
            return super(B, this).f()

        return g


def make():
    global C

    class C(A):
        def f(self):
            return super(C, self).f()

    return C
```

```
ruff:        up008_scopes.py:10:25, :18:25, :28:25 UP008
translation: (nothing)
```

### 6. A class path written with spaces, a line break or parentheses

`collect_attribute_chain` reads the attribute chain; `names_class` compares the argument's source text with the
dotted path (`text(a, p)`).

`up008_path.py`:

```python
class A:
    def f(self): ...


class Outer:
    class Inner(A):
        def f(self):
            return super(Outer. Inner, self).f()

        def g(self):
            return super((Outer).Inner, self).f()

        def h(self):
            return super(Outer
                         .Inner, self).f()

        def k(self):
            return super(Outer.Inner, self).f()
```

```
ruff:        up008_path.py:8:25, :11:25, :14:25, :18:25 UP008
translation: up008_path.py:18 UP008
```

## ruff-up034-extraneous-parentheses

### 1. An operator between a failed attempt's inner `)` and the next `(`

`up34_reads`' last clause takes the `(` of x's next sibling as "the token after x's `)`" when only blanks stand between
them in `structural_after`, but a binary, boolean or comparison operator is not in that text: in `((a) + ((b)))` the
attempt at the first `(` reads the `+` and stops, while the translation has it read the `(` of `((b))`, so `((b))`
starts no attempt. With a subscript after the next group it goes the other way: the `(` it wrongly takes as read is
an attempt that would have read the reported pair's `(`.

`up034_operator.py`:

```python
x = ((a) + ((b)))
x = ((a) or ((b)))
x = ((f('s', c)) == ((b)))
x = ((a) + (((a))[a]))
```

```
ruff:        up034_operator.py:1:13, :2:14, :3:22 UP034
translation: up034_operator.py:4 UP034
```

### 2. A lambda's trailing comma

A `,` after a lambda's last parameter is directly inside the inner pair, so Ruff's scan stops there; `up34_cut`
special-cases the trailing comma only for tuples and tuple patterns.

`up034_lambda_comma.py`:

```python
a1 = ((lambda a,: 0))
a2 = ((lambda a=1,: 0))
a3 = ((lambda *a,: 0))
a4 = [((lambda **k,: 0))]
```

```
ruff:        (nothing)
translation: up034_lambda_comma.py:1, :2, :3, :4 UP034
```

### 3. More than three rounds of attempts

`up34_start` stops after three alternating rounds. A chain of attempts, each reading the `(` after its inner pair
(the next call's arguments), needs more: in `((a)(((a)(((c))))))` the attempt at `(`1 reads `(`3, so `(`4 runs and
reads `(`6, so `(`7 runs and reports `((c))`. The translation's third round still counts `(`7's opener as read.

`up034_chain.py`:

```python
x = ((a)(((a)(((c))))))
x = ((f)(((a)(((b))))))
```

```
ruff:        up034_chain.py:1:16, :2:16 UP034
translation: (nothing)
```

## ruff-up035-deprecated-import

ok, apart from the syntax gate above.

## ruff-up036-outdated-version-block

### 1. Ruff stops reading the `if` at some branches; the translation reads every branch

In `outdated_version_block` three branches `return` from the whole statement, not `continue`: a tuple with an element
that is no int literal (`extract_version` gives `None`), an int literal other than 2 and 3 that fits in a `u8`, and 2
or 3 under an operator other than `==`, `!=`, `<`, `<=`, `>`, `>=`. Later `elif` tests are not looked at.

`up036_return.py`:

```python
import sys

x = 1
if sys.version_info >= (3, x):
    pass
elif sys.version_info >= (3, 8):
    pass

if sys.version_info[0] == 4:
    pass
elif sys.version_info < (3, 8):
    pass

if sys.version_info >= (3, -1):
    pass
elif sys.version_info < (3, 6):
    pass
```

```
ruff:        (nothing)
translation: up036_return.py:6, :11, :16 UP036
```

### 2. Int comparisons: invalid under every operator, nothing under `is`/`in`

An int that does not fit in a `u8` is invalid under any operator (`(None, _) => Reason::Invalid`); the translation
reports it under `==` and `!=` only. And 2 or 3 under `is`, `is not`, `in`, `not in` fall to `_ => return`; the
translation's first clause takes any operator.

`up036_ints.py`:

```python
import sys

if sys.version_info[0] < 300:
    pass
if sys.version_info.major >= 1000:
    pass
if sys.version_info[0] is 3:
    pass
if sys.version_info[0] in 3:
    pass
```

```
ruff:        up036_ints.py:3:26, :5:30 UP036 Version specifier is invalid
translation: up036_ints.py:7, :9 UP036
```

### 3. Tuples: Ruff reads only the elements it needs

`version_always_less_than` stops at the major version when it is not 3, and reads at most three elements. So an element
over 255 past those is never seen (`(4, 300)` is just "not settled", `(2, 300)` "always", at the test), and a fourth
element is ignored (`(3, 10, 0, 5)` is "at" the minimum, as `(3, 10, 0)`). The translation calls any element over 255
invalid, at the tuple, and has the "at" case for two and three elements only. Where the test and the tuple start on
different lines the keys differ too.

`up036_tuples.py`:

```python
import sys

if sys.version_info >= (4, 300):
    pass
if sys.version_info < (3, 10, 0, 5):
    pass
if (sys.version_info
        >= (2, 300)):
    pass
```

```
ruff:        up036_tuples.py:5:4, :7:5 UP036 Version block is outdated for minimum Python version
translation: up036_tuples.py:3, :8 UP036
```

## ruff-up039-unnecessary-class-parentheses

ok, apart from the syntax gate above.

## ruff-up041-timeout-error-alias

### 1. A comprehension's own variable is no import

`python/upgrade`'s `sym_at` does not see a comprehension's target binding, so `socket` inside
`[… for socket in …]` resolves to the module's `import socket`. Ruff resolves it to the comprehension's binding.

`up041_comprehension.py`:

```python
import socket

[socket.timeout() for socket in [1]]


def f():
    return [socket.timeout() for socket in [1]]


def g():
    return [socket.timeout() for _ in [1]]
```

```
ruff:        up041_comprehension.py:11:13 UP041
translation: up041_comprehension.py:3, :7, :11 UP041
```

The cause is the library's (`sym_at`); the other rules that resolve names through it (UP008's `super`, UP036's `sys`,
UP043's `Generator`) were not tried with a comprehension variable.

## ruff-up043-unnecessary-default-type-args

### 1. String type definitions outside annotations

Ruff parses as a type definition every string it reads in a typing context, not only annotations: the first argument
of `cast`, `TypeVar`'s `bound=`, `NewType`'s second argument, a string inside a subscript (`list["…"]`,
`Optional["…"]`), the value of an `X: TypeAlias = "…"`. The translation's string clause reaches only annotation strings
(`parsed_in` over `parsed(s, "annotation", _)` trees).

`up043_typedefs.pyi`:

```python
from typing import Generator, TypeVar, cast, NewType, Optional, TypeAlias
import typing

A = cast("Generator[int, None]", 1)
B = typing.cast("Generator[int, None]", 1)
T = TypeVar("T", bound="Generator[int, None]")
X = list["Generator[int, None]"]
Y = Optional["Generator[int, None]"]
Z: TypeAlias = "Generator[int, None]"
N = NewType("N", "Generator[int, None]")
def f(x: "Generator[int, None]") -> None: ...
```

```
ruff:        up043_typedefs.pyi:4:11, :5:18, :6:25, :7:11, :8:15, :9:17, :10:19, :11:11 UP043
translation: up043_typedefs.pyi:11 UP043
```

## The author's `different` entries

Both say the translation is right; Ruff agrees on every sample key. On the files from
`~/pt/corpora/recorded/python/files/ruff/{pyflakes/F401_0.py,pyupgrade/UP045_1.py}`, Ruff 0.17.6 at 3.10 parses
F401_0.py (`match *0, 1, *2:` at lines 91 and 110 is valid to it) and reports:

```
F401_0.py:13:1: UP035 `typing.Dict` is deprecated, use `dict` instead   (and List, Set, Type)
F401_0.py:81:4: UP036 Version block is outdated for minimum Python version
F401_0.py:84:5: UP035 [*] Import from `typing` instead: `TypeAlias`
UP045_1.py:55:1: UP035 [*] Import from `typing` instead: `Optional`
```

`run-policies` on the same two files gives the same four keys (F401_0.py:13, :81, :84; UP045_1.py:55). So
`ruff-up035-deprecated-import` (fixtures, translationOnly 3) and `ruff-up036-outdated-version-block` (fixtures,
translationOnly 1), `right: translation`, are confirmed.
