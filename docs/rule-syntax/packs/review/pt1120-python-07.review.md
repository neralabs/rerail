# pt1120-python-07 translation review

Ground truth: Ruff 0.17.6 (`ruff check --isolated --preview --select <code>`). I read the rules' implementation at the
0.17.6 tag (`flake8_unused_arguments/rules/unused_arguments.rs`, `flake8_logging_format/rules/logging_call.rs`,
`ruff_python_semantic/src/analyze/{function_type,visibility,logging}.rs`, the checker's binding and flag handling in
`checkers/ast/mod.rs`) and the fixtures, then ran my own cases with Ruff and `rerail-engine-1120 run-policies` on
`docs/rule-syntax/packs/python/*.rerail` at `2d77f994af`, on a fleet box. Translation lines are the finding's line
(the engine's output names the node, not the column).

The four ARG rules share `p07_unused` and the four G rules share `p07_log_call`/`p07_exc_info`, so each defect below
is shown on every rule it reaches.

## `ruff-arg002-unused-method-argument`: defects

Counterexample `arg002.py`:

```python
import builtins
from typing import cast


def locals():
    return {}


class C:
    def late_local(self, x):
        def g():
            print(x)
            x = 1
        return g

    def shadowed_locals(self, x):
        return locals()

    def builtins_locals(self, x):
        return builtins.locals()

    def string_annotation(self, x):
        y: "x" = 1
        return y

    def string_cast(self, x):
        return cast("x", 1)
```

Counterexample `arg002_default.py` (apart, as `arg002.py` shadows `locals`):

```python
class C:
    def nested_default(self, x):
        def g(a=locals()):
            return a
        return g
```

```text
Ruff:        arg002.py:19:31 ARG002
Translation: arg002.py:10 ARG002
             arg002.py:16 ARG002
             arg002.py:22 ARG002
             arg002.py:26 ARG002
             arg002_default.py:2 ARG002
```

1. **A nested scope's read before its own later binding (line 10, false positive).** Ruff resolves a load against
   the bindings that exist when it visits it: `g`'s `x = 1` comes after `print(x)`, so the load resolves to the
   parameter, which is then used. `p07_read` looks the name up with `walk(n, x, f)`, which stops at `g` because `g`
   binds `x` anywhere in its body.
2. **`locals` by name, not by resolution (line 16 false positive, line 19 missed).** Ruff sets `uses_locals` for any
   call whose callee is the bare name `locals` in a load (`checkers/ast/mod.rs`, Step 1 of `visit_expr` for
   `Expr::Call`), whatever the name resolves to; `builtins.locals()` is an attribute and does not count.
   `p07_locals_call` (and `argt_locals` under it) asks `call_q(c, "builtins.locals")`, the reverse on both.
3. **`locals()` in a nested def's default (`arg002_default.py:2`, false positive).** A nested def's defaults,
   annotations and decorators are visited in the enclosing scope, so the call marks `nested_default` as using
   `locals()`. `argt_locals` takes `nearest(c, f, scope)`, which is `g`.
4. **Names in string type expressions (lines 22 and 26, false positives).** Ruff parses a string annotation and a
   `typing.cast` type argument and resolves the names in it; a typing-only reference still makes the parameter used.
   `p07_read` reads only `ast_name` loads.

## `ruff-arg003-unused-class-method-argument`: defects

The same five cases, each method decorated `@classmethod` with `cls` first (`arg003.py`, `arg003_default.py`: the files
above with `@classmethod` above each `def` and `self` renamed `cls`).

```text
Ruff:        arg003.py:22:30 ARG003
Translation: arg003.py:11 ARG003          late_local
             arg003.py:18 ARG003          shadowed_locals
             arg003.py:26 ARG003          string_annotation
             arg003.py:31 ARG003          string_cast
             arg003_default.py:3 ARG003   nested_default
```

Line 22 (`builtins_locals`) is missed; causes as in ARG002, items 1-4.

## `ruff-arg004-unused-static-method-argument`: defects

The same cases with `@staticmethod` and no receiver (`arg004.py`, `arg004_default.py`).

```text
Ruff:        arg004.py:22:25 ARG004
Translation: arg004.py:11 ARG004          late_local
             arg004.py:18 ARG004          shadowed_locals
             arg004.py:26 ARG004          string_annotation
             arg004.py:31 ARG004          string_cast
             arg004_default.py:3 ARG004   nested_default
```

Line 22 (`builtins_locals`) is missed; causes as in ARG002, items 1-4.

## `ruff-arg005-unused-lambda-argument`: defects

Counterexample `arg005.py`:

```python
import builtins
from typing import cast

late_local = lambda a: (lambda: (print(a), (a := 1)))
shadowed_locals = lambda locals, a: locals()
builtins_locals = lambda a: builtins.locals()
string_cast = lambda a: cast("a", 1)
```

```text
Ruff:        arg005.py:6:26 ARG005
Translation: arg005.py:4 ARG005
             arg005.py:5 ARG005
             arg005.py:7 ARG005
```

Line 4: the inner lambda's walrus binds `a` after the read (item 1). Line 5 false positive and line 6 missed: `locals`
by name (item 2; `p07_lambda_locals` builds on `p07_locals_call`). Line 7: the string type expression (item 4).

## `ruff-g002-logging-percent-format`: defects

Counterexample `g002_g003.py`:

```python
from logging import getLogger


def f(name):
    getLogger().info("Hello " + name)
    getLogger().info("Hello %s" % name)


str.log.info("Hello " + "World!")
str.log.info("Hello %s" % "World!")

getLogger = None
```

```text
Ruff:        no diagnostics
Translation: g002_g003.py:5 G003
             g002_g003.py:6 G002
             g002_g003.py:9 G003
             g002_g003.py:10 G002
```

1. **An inline logger's constructor is resolved without the binding in effect (line 6).** `f`'s body is deferred and
   sees the module's last `getLogger`, the assignment; `resolve_qualified_name` gives nothing for it, and
   `is_logger_candidate` returns false for a call receiver it cannot resolve to `logging.getLogger`/`Logger`.
   `p07_logger`'s first clause uses `call_q`, not `p07_qual`, so it still sees the import: the binding-in-effect rule
   the library adds for the other two clauses misses this one.
2. **An attribute chain rooted at a builtin (line 10).** Ruff resolves `str.log` to `builtins.str.log`, a qualified
   name other than `logging`, so the receiver is not a logger and the name heuristic is never reached. `p07_qual` has
   no row for it (`p07_builtin` covers a bare builtin name only), so `p07_logger`'s heuristic clause takes `log`.
   Rare in real code; shown for completeness.

## `ruff-g003-logging-string-concat`: defects

Same counterexample and causes as G002: lines 5 (inline `getLogger()`) and 9 (`str.log`) are translation-only.

## `ruff-g201-logging-exc-info`: defect

Counterexample `g201_g202.py`:

```python
import sys
from logging import getLogger


def f():
    try:
        pass
    except ValueError:
        getLogger().error("Hello", exc_info=True)
        getLogger().exception("Hello", exc_info=True)


def g(logger):
    try:
        pass
    except ValueError:
        logger.error("Hello", exc_info=sys.exc_info())
        logger.exception("Hello", exc_info=sys.exc_info())


getLogger = None
sys = None
```

```text
Ruff:        no diagnostics
Translation: g201_g202.py:9 G201
             g201_g202.py:10 G202
             g201_g202.py:17 G201
             g201_g202.py:18 G202
```

Line 9: the inline `getLogger()` of G002 item 1. Line 17: `p07_exc_info` matches `sys.exc_info()` with `call_q` too;
Ruff's `logging::exc_info` uses `matches_qualified_name`, and in the deferred body `sys` is the assignment.

## `ruff-g202-logging-redundant-exc-info`: defect

Same counterexample and causes as G201: lines 10 and 18 are translation-only.

## Checked and agreeing

Cases that agree with Ruff, beyond the fixtures: `except*` handlers; G201/G202 in an `except` type expression, a def's
decorator and default, a class's bases, a lambda and a comprehension in the handler, and not in a def's or class's
body or `finally`; `exc_info=(True)`, `exc_info=1`, `**{"exc_info": True}`; `msg=` beside a positional message; a
starred argument before the message; deferred reads after `del log` and an `except … as logging` name; relative
imports inside and outside a package; decorators rebound after the class (`staticmethod`, `abstractmethod`,
`override`) and shadowed in the class body; defs in `if`/`try` in a class body; receivers of `*args`, keyword-only,
positional-only and annotated variadic signatures; `__new__`, `__init_subclass__` and `__call__`; stubs (f-string
message variable, unmatched variable, `(...)`, `raise NotImplemented`, bytes and f-string statements, a shadowed
`NotImplementedError`); `nonlocal`; an `except … as x` rebinding of a parameter; the walrus in a comprehension;
`locals()` in a comprehension's first iterable, element and nested comprehension, in a lambda's default and in a
class-body lambda; `.pyi` files.

## The author's result file

`pt1120-python-07.result.json` lists all eight rules as `same`: no `different` entry to check. ptcheck on
`tbelanger/pt1120-python-07` agrees: 8 same, 0 different (examples, fixtures, samples-python). Its only problems are
the eight rules still listed in `equivalence.json`'s `different` ("recorded by the suite, not yet explained"), as they
are on `tbelanger/pack-translations-1120`, like the other 1120 batches' rules: a record to update, not a translation
defect. None of the defects above is in the corpora the suite runs (the files above are new), so `same` holds for the
suite and the defects are beside it.
