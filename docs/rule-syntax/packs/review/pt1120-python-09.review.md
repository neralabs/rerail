# pt1120-python-09 translation review

Ground truth: Ruff 0.17.6, `ruff check --isolated --select <CODE> --no-cache` (stable). The pack ports Ruff's default,
non-preview behaviour (packs/python/README.md), so Ruff's preview-only check of `ngettext`'s plural argument
(INT001/INT002, `is_plural_ngettext_check_enabled`) is not counted as a miss. I read the rules at the 0.17.6 tag
(`tryceratops/rules/{raise_vanilla_class,raise_vanilla_args,verbose_log_message}.rs`, `tryceratops/helpers.rs`,
`flake8_bugbear/rules/{except_with_non_exception_classes,zip_without_explicit_strict}.rs`, `flake8_bugbear/helpers.rs`,
`flake8_gettext/{mod.rs,rules/*.rs}`, `ruff_python_semantic` `resolve_qualified_name`, and the call sites in
`checkers/ast/analyze/`), then ran my own cases with Ruff and `rerail-engine-1120 run-policies` on
`docs/rule-syntax/packs/python/<id>.rerail` at `64a2794203`, with `RERAIL_SOURCE_ROOT` set to a copy of that commit's
`docs/rule-syntax` and `native/src/code_rule_dsl/typing/programs`. Translation lines are the finding's line (the
engine names the node, not the column).

Fleet note: pt-w01, pt-w07 and ts-prisma-b have no `rerail-engine-1120`; their `rerail-engine` stops on these
policies (`phase 0: line 296: not lowered yet: setting on .decl env`). The runs below are on pt-w02 (in
`/tmp/pt-review-p09`), which has the 1120 engine and the same Ruff 0.17.6.

One cause reaches six of the seven rules: identifiers are compared as written, not NFKC-normalized. Python and Ruff's
lexer normalize them (`Ｅｘｃｅｐｔｉｏｎ` is `Exception`, `ｚｉｐ` is `zip`); the translations' names (`sym_at`, `sym_of`,
`text(m, x)` in `msg_gettext`, the attribute name in `exc_handler_log`) are not. It is listed under each rule it
reaches; the fix belongs in the names library, not in the rules.

## `ruff-try002-raise-vanilla-class`: defects

Counterexample `try002.py`:

```python
def f():
    raise Ｅｘｃｅｐｔｉｏｎ("full-width name")


def g():
    raise Exception("control")
```

Counterexample `script/builtins.py` (a directory with no `__init__.py`):

```python
class Exception(BaseException):
    pass


raise Exception("own class in a module named builtins")
```

```text
Ruff:        script/builtins.py:5:7 TRY002
             try002.py:2:11 TRY002
             try002.py:6:11 TRY002
Translation: try002.py:6 TRY002
```

1. **NFKC names (`try002.py:2`, missed).** `Ｅｘｃｅｐｔｉｏｎ` is the builtin `Exception` to Python and to Ruff.
2. **A module named `builtins` (`script/builtins.py:5`, missed).** `resolve_qualified_name` gives a module-level
   class its module's qualified name; in a script that is the file stem, so the class is `builtins.Exception` and
   matches `["" | "builtins", "Exception"]`. `sym_at` gives a local class its `qualname` without the module. Contrived;
   listed for completeness.

The backslash-continued raise, the visit-order resolution (class scope skipped by a method, a later local binding),
`raise (Exception)(…)`, `raise Exception()()` (not reported) and the `from` cause all match Ruff on my cases.

## `ruff-try003-raise-vanilla-args`: defects

Counterexample `try003.py`:

```python
class E(Exception):
    pass


x = 1
raise E(f"a\x20{x}")
raise E(f"a\040{x}")
raise E(f"a\u2003{x}")
raise E(f"a\x0bb{x}")
raise E(f"a\N{LATIN SMALL LETTER A}{x}")
raise E(f"a\
b{x}")
raise ＮｏｔＩｍｐｌｅｍｅｎｔｅｄＥｒｒｏｒ("not yet done")
raise E("a\x20b")
```

```text
Ruff:        try003.py:6:7 TRY003
             try003.py:7:7 TRY003
             try003.py:8:7 TRY003
             try003.py:9:7 TRY003
             try003.py:14:7 TRY003
Translation: try003.py:10 TRY003
             try003.py:11 TRY003
             try003.py:13 TRY003
             try003.py:14 TRY003
```

1. **An f-string's literal text is read as source, not as its value (lines 6–9 missed, 10–11 false).** Ruff tests
   `char::is_whitespace` on the decoded literal elements. `space_part`'s f-string clause matches `\s` against the
   `string_content` source text and accepts only the escapes `\n \t \r \v \f`. So whitespace written as `\x20`, `\040`,
   `\u2003` or `\x0b` is missed; `\N{LATIN SMALL LETTER A}` (decodes to `a`) matches on the spaces in the escape's
   name; a backslash-newline (removed from the value) matches on the newline. Plain strings are right (line 14, and
   `"a\N{LATIN SMALL LETTER A}b"` / `"a\` + newline + `b"` are silent in both): `value` is decoded there.
2. **NFKC names (line 13, false).** `ＮｏｔＩｍｐｌｅｍｅｎｔｅｄＥｒｒｏｒ` is `NotImplementedError` to Ruff's
   `match_builtin_expr`, so it is exempt; `exc_qual` does not see it.

Raw f-strings (`rf"a\t{x}"`), `r"a\tb"`, bytes, t-strings, `f"{x = }"`, format-spec text (`f"{x:> 10}"`), a starred
first argument, `(NotImplementedError)(…)` and a parenthesized message match Ruff.

## `ruff-try401-verbose-log-message`: defects

Counterexample `try401.py`:

```python
import logging

logger = logging.getLogger(__name__)
try:
    pass
except ValueError as e:
    logger.exception("lambda", lambda: e)
    logger.exception("called lambda", (lambda: e)())

    def inner():
        logger.exception("nested def", e)

    logger.exception("control", e)
try:
    pass
except ValueError as e:
    e = str(e)
    logger.exception("assignment", e)
try:
    pass
except ValueError as e:
    for e in range(3):
        pass
    logger.exception("loop target", e)
try:
    pass
except ValueError as e:
    if (e := 1):
        pass
    logger.exception("walrus", e)
try:
    pass
except ValueError as e:
    logger.ｅｘｃｅｐｔｉｏｎ("full-width method", e)
```

```text
Ruff:        try401.py:13:33 TRY401
             try401.py:34:43 TRY401
Translation: try401.py:7 TRY401
             try401.py:8 TRY401
             try401.py:11 TRY401
             try401.py:13 TRY401
             try401.py:18 TRY401
             try401.py:24 TRY401
             try401.py:30 TRY401
```

1. **The name's binding is not the one the load reads (lines 18, 24, 30, false).** Ruff reports a name only when
   `resolve_name` gives a binding whose kind is `BoundException`. After `e = str(e)`, `for e in …` or `(e := 1)` in the
   handler, the load reads that assignment. `ref_site(n, s), site_form(s, "except")` holds as long as some reaching
   site is the `except … as`.
2. **Deferred scopes (lines 7, 8, 11, false).** The check runs when the `try` statement is analysed; lambda bodies
   and nested function bodies are visited later (deferred), so their names have no resolution yet and are skipped.
   `positional`/`has(a, n, "identifier")` takes names inside a lambda, and `exc_handler_log` takes a call in a nested
   `def` of the handler. Comprehensions are not deferred: `[e for _ in y]` is reported by both, rightly.
3. **NFKC names (line 34, missed).** `logger.ｅｘｃｅｐｔｉｏｎ` is `exception` to Ruff's `LoggingLevel::from_attribute`.

Keyword arguments (`extra=e`, not reported), `dict(k=e)` (reported), `*e`, `e[0].x` and `e.args` (not reported), the
logger forms (`self.logger`, `logging.getLogger().exception`, `from logging import exception`, an import from another
module and a name not looking like a logger, both silent) and a bound exception of an enclosing handler match Ruff.

## `ruff-b030-except-with-non-exception-classes`: ok

No difference on 15 findings over two files of my cases: `except*` (parenthesized and PEP 758 bare lists), a list
type, `[A] + [B]`, starred lists and starred sums in the top tuple, `*[KeyError] + [1]`, a nested tuple as a top
element and inside a `+` operand, `A | B`, a walrus, a conditional, `(A, 1) as e`, a bare list with a tuple element,
and a comment inside the tuple. Each Ruff finding has a translation finding on the same line, and the two on line 11
of each file are two findings in both.

## `ruff-b905-zip-without-explicit-strict`: defects

Counterexample `b905.py`:

```python
from itertools import repeat
from typing import cast

a = b = xs = []
zip(a, repeat(*xs, None))
x: "zip(a, b)" = 1
y = cast("zip(a, b)", 1)
ｚｉｐ(a, b)
zip(a, b)
```

```text
Ruff:        b905.py:6:5 B905
             b905.py:7:11 B905
             b905.py:8:1 B905
             b905.py:9:1 B905
Translation: b905.py:5 B905
             b905.py:9 B905
```

1. **`repeat(*xs, None)` (line 5, false).** Ruff's `args` includes the starred argument, so `args[1]` is `None` and the
   repeat is infinite. `ruff_arg(a, 1, n0)` has no row after a `*xs`. The author's notes name this and leave it
   ("no corpus has one"); it is still a difference from Ruff.
2. **zip calls in string type definitions (lines 6 and 7, missed).** Ruff runs its expression checks on the code it
   parses from a string annotation and from `cast("…", x)`'s type, as the batch's own INT001/INT002 notes say. B905
   reads the file's tree only; `p09_gettext_arg`'s `type_str_node`/`type_arg` route was added for INT but not here.
3. **NFKC names (line 8, missed).** `ｚｉｐ` is the builtin `zip`.

`[*zip(a, b)]`, `[*(zip(a, b))]`, `(*zip(a, b),)`, `{*zip(a, b)}`, `[1, *zip(a, b)]`, `print(*zip(a, b))`,
`f"{zip(a, b)}"`, `repeat(*xs)`, `repeat(1, (None))`, `repeat(times=None, object=1)`, `repeat(1, **{})`,
`[*zip(a, count())]` and `zip(a, b, **{})` match Ruff.

## `ruff-int001-f-string-in-get-text-func-call`: defects

Counterexample `int_alias.py`:

```python
from typing import Literal as L

x = 1
a: "L['_(f\"{x}\")']" = 1
b: "L['_(\"{}\".format(x))']" = 1
_((x.format)(1))
ｇｅｔｔｅｘｔ(f"{x}")
ｇｅｔｔｅｘｔ("{}".format(x))
```

Counterexample `int_other_literal.py`:

```python
from mylib import Literal

x = 1
a: "Literal['_(f\"{x}\")']" = 1
b: "Literal['_(\"{}\".format(x))']" = 1
```

```text
Ruff:        int_alias.py:7:9 INT001
             int_other_literal.py:4:4 INT001
Translation: int_alias.py:4 INT001
```

1. **`Literal` by spelling, not by resolution (`int_alias.py:4` false, `int_other_literal.py:4` missed).** Ruff
   leaves a string inside `Literal[…]` unparsed only when the subscript's value resolves to `typing.Literal` or
   `typing_extensions.Literal`. `p09_in_literal` takes any subscript whose value's text is `Literal` or ends in
   `.Literal`: an alias (`L`) is parsed and reported, a `Literal` from another module is skipped. (A bare `Literal`
   that nothing binds is parsed by Ruff as well and reported; the translation skips it too.)
2. **NFKC names (`int_alias.py:7`, missed).** `msg_gettext` compares `text(m, x)` with `_`/`gettext`/`ngettext`;
   Ruff compares the normalized name. packs/python/README.md says the JSON original does normalize these.

`_("a" f"{x}")`, `_((f"{x}"))`, `builtins._(…)` after `import builtins`, `gettext.ngettext` through an import,
`dgettext` (not a translation call), a starred first argument, t-strings, `cast("…")` types, `list['…']` inside an
annotation and an imported `typing.Literal` (by name or as `typing.Literal`) match Ruff.

## `ruff-int002-format-in-get-text-func-call`: defects

Counterexamples `int_alias.py` and `int_other_literal.py` above.

```text
Ruff:        int_alias.py:6:3 INT002
             int_alias.py:8:9 INT002
             int_other_literal.py:5:4 INT002
Translation: int_alias.py:5 INT002
```

1. **`Literal` by spelling (`int_alias.py:5` false, `int_other_literal.py:5` missed).** As INT001's 1.
2. **A parenthesized `.format` callee (`int_alias.py:6`, missed).** Ruff's AST has no parentheses: `(x.format)(1)` is a
   call whose `func` is the attribute `format`. `msg_form(a, "format")` reads `field(a, "function", f)` and
   `attr_of(f, …)` without `unparen`. packs/python/README.md lists `("{}".format)(a)` as reported by the original.
3. **NFKC names (`int_alias.py:8`, missed).** As INT001's 2.

## The author's result file

`pt1120-python-09.result.json` marks all seven rules `same`; it has no `different` entry to check. ptcheck on
`tbelanger/pt1120-python-09` (`64a2794203`) for the seven rules agrees: `7 same, 0 different` on examples, fixtures
(1963 files) and samples-python (2313 files). It also says each of the seven is still listed as `different` in the
pack ("remove it from different"), a bookkeeping step left for the integration branch. None of the defects above
occurs in those corpora; the counterexamples are the only evidence for them.
