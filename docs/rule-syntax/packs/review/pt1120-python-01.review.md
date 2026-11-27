# pt1120-python-01 translation review

Ground truth: Ruff 0.17.6. I read Ruff 0.17.6's implementation of each rule (`flake8_pyi/rules/*.rs`) and its
callers (`analyze/statement.rs`, `analyze/parameters.rs`, `analyze/bindings.rs`, `checkers/ast/mod.rs`). Then I ran
my own cases through `ruff check --isolated --no-cache --select PYI002,PYI003,PYI004,PYI005,PYI013,PYI014,PYI019,PYI020`
and through `rerail-engine-1120 run-policies`, with all eight policies of `tbelanger/pt1120-python-01` (d31ab93d37), in
/tmp on `junction-pt-w03`. pt-w01, pt-w07 and ts-prisma-b have no `rerail-engine-1120`, and their main-line engine
refuses this branch's library (`not lowered yet: setting on .decl env`). The engine was run with
`RERAIL_SOURCE_ROOT` pointing at a copy of this commit's `docs/rule-syntax` and typing programs.

Translation lines below are `run-policies` output (file:line code). Ruff lines are `--output-format concise`.

| Rule | Verdict |
|---|---|
| ruff-pyi002-complex-if-statement-in-stub | defect 2 |
| ruff-pyi003-unrecognized-version-info-check | defects 1, 2, 3 |
| ruff-pyi004-patch-version-comparison | defects 1, 2 |
| ruff-pyi005-wrong-tuple-length-version-comparison | defects 1, 2 |
| ruff-pyi013-ellipsis-in-non-empty-class-body | ok |
| ruff-pyi014-argument-default-in-stub | defect 4 |
| ruff-pyi019-custom-type-var-for-self | defects 5, 6, 7 |
| ruff-pyi020-quoted-annotation-in-stub | defects 8, 9 (and the known UP037_2.pyi:41) |

## PYI002–PYI005

### Defect 1 (PYI003, PYI004, PYI005): a `\` inside the comparand tuple counts as an element

python/typing's `tuple_len` and `int_tuple` count every non-comment child of the tuple. tree-sitter makes a `\`
line continuation inside the tuple a `line_continuation` child, so it counts as an element, and as one that is not
an int. PYI014 already skips these children (`simple_display`). The version checks do not. Ruff's tuple has two
elements.

`pyi003-cont.pyi`:

```python
import sys
if sys.version_info >= (3, \
    8): ...
if sys.version_info[:2] == (3, \
    8): ...
if sys.version_info >= (3, 8, \
    1): ...
```

```text
Ruff:        pyi003-cont.pyi:6:4 PYI004
Translation: pyi003-cont.pyi:2 PYI003
             pyi003-cont.pyi:4 PYI003
             pyi003-cont.pyi:4 PYI005
             pyi003-cont.pyi:6 PYI003
```

Lines 2 and 4 are false PYI003s and line 4 is a false PYI005. Line 6 is reported under the wrong code: PYI003
instead of Ruff's PYI004, so PYI004 is missed.

### Defect 2 (PYI002; PYI004, PYI005 missed): a `\` inside a comparison or an `and`/`or` counts as an operand

`cmp_operand` (lib/python/pt1120-python-01.rerail) skips `comment` children but not `line_continuation`, so
`sys.version_info >= \` / `(3, 8, 1)` has three operands and is no `one_cmp`. python/ast's `bool_part` has the same
gap, so the `\` after an `and`/`or` becomes a test part of its own. Neither is simple, so PYI002 fires. Because
`vi_cmp` then fails too, PYI004 and PYI005 miss the comparison. A `\` at the end of an `if` line is the usual way to
wrap a long stub test.

`pyi002-cont.pyi`:

```python
import sys
if sys.version_info >= (3, 8) and \
        sys.platform == "linux": ...
if sys.version_info >= \
        (3, 8, 1): ...
if sys.version_info[:2] \
        == (3,): ...
if sys.version_info[:2] == (3, 8) or \
        sys.version_info[0] == "3": ...
```

```text
Ruff:        pyi002-cont.pyi:4:4 PYI004
             pyi002-cont.pyi:6:4 PYI005
             pyi002-cont.pyi:9:9 PYI003
Translation: pyi002-cont.pyi:2 PYI002
             pyi002-cont.pyi:4 PYI002
             pyi002-cont.pyi:6 PYI002
             pyi002-cont.pyi:8 PYI002
             pyi002-cont.pyi:9 PYI003
```

There are four false PYI002s, and PYI004 (line 4) and PYI005 (line 6) are missed.

### Defect 3 (PYI003): a trailing comma followed by a comment is read as `[0]` / `[:2]`

`vi_one` detects the trailing comma with the regex `[\s\S]*,\s*\]` on the subscript's text. A comment between the
comma and the `]` defeats it. Ruff's index is a tuple, so `ExpectedComparator::try_from` gives `None`, which is
PYI003. The translation reads the index as "major" / "slice2" and reports nothing.

`pyi003-trailcomma.pyi`:

```python
import sys
if sys.version_info[0,  # c
    ] == 3: ...
if sys.version_info[:2,  # c
    ] == (3, 8): ...
```

```text
Ruff:        pyi003-trailcomma.pyi:2:4 PYI003
             pyi003-trailcomma.pyi:4:4 PYI003
Translation: no diagnostics
```

Checked and agreeing: a missing `import sys` (PYI002 only), aliases (`from sys import version_info as vi`),
`.major`, chained comparisons, `is` / `not in`, a parenthesised `(a and b) and c`, `a and b or c`, `elif` (not
checked), hex, binary, `00` and `1_0` indices, `[:2:]`, `[:2:None]`, `[None:2]`, `[0, 1]`, `[0:1]`, starred, empty,
list and non-int comparands, `[0] == (3)` and `((3, 8, 1))`.

## PYI013: ok

Matches Ruff on `...; ...` on one line, `...` before `pass`, `"""d"""; ...`, `((...))  # c`, `...,`, a lone `...`
with a comment, `...` under an `if` in a class body (not reported), a `class A: \` continuation, and in a `.py` file.

## PYI014

### Defect 4: lambda defaults in stubs are not checked

Ruff runs `argument_simple_defaults` from `analyze::parameters`, which `visit_parameters` calls for lambdas too
(`visit_deferred_lambdas`). The translation asks only `function_definition`s (`kind(f, "function_definition")`).
Lambda parameters cannot be annotated, so every non-simple lambda default in a stub is a Ruff PYI014.

`pyi014-lambda.pyi`:

```python
import os
f = lambda a=os.sep: a
class C:
    g = lambda self, a=[[1]]: a
```

```text
Ruff:        pyi014-lambda.pyi:2:14 PYI014
             pyi014-lambda.pyi:4:24 PYI014
Translation: no diagnostics
```

(The JSON original misses these too.) Everything else I tried matched Ruff, line by line and count by count:
f-, t- and concatenated strings, `-(1)`, `-True`, `+1`, `-2j`, real parts of 10 and 11 characters, `-1 - 2j`,
`(1) + (2j)`, keyword-only and positional-only parameters, `math`/`sys` constants and `-math.nan`, `{**x}`, `[*a]`,
nested containers, 10- and 11-item tuples and dicts, `\` inside a dict of 10 pairs, annotated parameters, and a
lambda as a default.

## PYI019

### Defect 5: `cls: type[_S,]` is reported

Ruff wants the subscript's slice to be a Name (`cls_annotation_typevar.as_name_expr()?`). `type[_S,]`'s slice is
a tuple. In an annotation tree-sitter writes `type[_S,]` as a `generic_type` with one `type` argument and a bare
`,`, so `tsub_n(m, 1)` holds. The JSON original refuses it (`"not": {"has": {"regex": "^,$"}}`).

### Defect 6: an annotated TypeVar assignment counts

Ruff takes the TypeVar only from a plain assignment (`statement.as_assign_stmt()?`). `_A: TypeVar = TypeVar("_A")`
is an AnnAssign, so Ruff does not report `self: _A`. The translation's `typevar_name` reads the binding's value
regardless. A chained `_C = _D = TypeVar("_C")` agrees.

`pyi019-stub.pyi`:

```python
from typing import TypeVar
_S = TypeVar("_S")
_A: TypeVar = TypeVar("_A")
class C:
    @classmethod
    def m1(cls: type[_S,]) -> _S: ...
    def m2(self: _A) -> _A: ...
```

```text
Ruff:        no diagnostics
Translation: pyi019-stub.pyi:6 PYI019
             pyi019-stub.pyi:7 PYI019
```

### Defect 7 (minor): in a `.py` file, a TypeVar assigned after the class counts

With Ruff's default target (3.10) and no `from __future__ import annotations`, a `.py` file's signature
annotations are resolved where the def stands, so `_L` below is unresolved and Ruff reports nothing. The
translation resolves to the later module binding. The same layout in a `.pyi` (annotations deferred) is reported
by both. The code raises NameError at import, so this is low value, but it is a mismatch.

`pyi019-late.py`:

```python
from typing import TypeVar
class C:
    def m(self: _L) -> _L: ...
_L = TypeVar("_L")
```

```text
Ruff:        no diagnostics
Translation: pyi019-late.py:3 PYI019
```

Checked and agreeing: `self: _S`, `self: (_S)`, `"_S"`, `"_" "S"`, `cls: "type[_S]"`, `type[(_S)]`, `Type[_S]` and
`cls: _S` (not reported), `__init_subclass__`, `__new__`, `def m[T](self: T)`, `def m[**Q](self: Q)` and ParamSpec
(not reported), static, overload, abstract, a keyword-only `self`, a `type(...)` metaclass, a class's PEP 695
parameter, a stub TypeVar assigned after the class, `from typing_extensions import TypeVar`, and a module-level
`type = int` after the class.

## PYI020

### Defect 8: a quoted type inside a quoted annotation is not reported

The rule's second clause (`PYI020(q) :- … parsed_within(s, q), quoted(q), parses(q)`) produces no rows.
`quoted(q)` needs `plain_str(q, _)`, and strings of the "annotation" guest tree have no `plain_str`
(`string_form`/`value`) rows. I checked this with probe clauses on the box: for those nodes `parsed_within(s, q)`
and `string_annotation(q)` hold, and `plain_str(q, _)` does not. Ruff reports the nested string as well. Both
keys are on the same line, so a comparison by line set does not show the miss.

`pyi020-nested.pyi`:

```python
a: "list['int']"
b: list["list['int']"]
```

```text
Ruff:        pyi020-nested.pyi:1:4 PYI020
             pyi020-nested.pyi:1:10 PYI020
             pyi020-nested.pyi:2:9 PYI020
             pyi020-nested.pyi:2:15 PYI020
Translation: pyi020-nested.pyi:1 PYI020
             pyi020-nested.pyi:2 PYI020
```

### Defect 9: strings Ruff parses as a bare expression are accepted when Ruff refuses them

Ruff's `parse_type_annotation` parses a triple-quoted string's source in parentheses. It parses every other
string as a bare expression: a single-quoted string's source range, or the decoded value when the string has
escapes or is implicitly concatenated. A leading space or tab, or a newline outside brackets, is then a syntax
error (F722) and gives no PYI020. The "annotation" guest always wraps the value in `( … )`, so `parses(s)` holds
for all of these.

`pyi020-bare.pyi`:

```python
a: " int"
b: "\tint"
c: " " "int"
d: "int |\nstr"
```

```text
Ruff:        pyi020-bare.pyi:1:4 F722   (and :2:4, :3:4, :4:4 F722; no PYI020)
Translation: pyi020-bare.pyi:1 PYI020
             pyi020-bare.pyi:2 PYI020
             pyi020-bare.pyi:3 PYI020
             pyi020-bare.pyi:4 PYI020
```

The same parse-mode split explains why `"list[\nint]"` (newline inside brackets) and `""" int"""`
(triple-quoted) are PYI020 in Ruff. Both agree in the translation.

Checked and agreeing: a non-typing generic's argument (`Foo["bar"]`), `NewType`'s second argument,
`Callable[["int"], "str"]`, a class base `List["int"]`, module-level `list["int"]` and `Union["int", str]`, a call
`print(list["int"])`, `Annotated` metadata (not reported), TypedDict / NamedTuple functional fields, TypeVar
constraints, `type X = "int"`, `X: TypeAlias = "int"`, `Final["int"]`, `Literal["a"] | "int"`, `*args: "int"`, a
default string (not reported), `b"int"` and `Foo("x")` (not reported), `"int  # c"`, `"(int)"`, `"lambda: 1"`, and a
`\`-continued string.

## The author's `different` entry

PYI020, `right: neither`. Ruff 0.17.6 on the fixture `ruff/pyupgrade/UP037_2.pyi` (from `~/pt/corpora/recorded`)
reports PYI020 at 32:4 and 41:14. The translation reports line 32 and not line 41. The entry is accurate: the
translation is right at :32, and the original is right at :41, which remains a translation miss (the engine's
annotation guest, as NOTES-pt1120-python-01.md says).
