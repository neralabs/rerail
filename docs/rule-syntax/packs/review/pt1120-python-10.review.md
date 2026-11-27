# pt1120-python-10 translation review

Ground truth: Ruff 0.17.6 (`ruff check --isolated --preview --select <code>`, default target py310). I read the
rules' implementation at the 0.17.6 tag (`flake8_implicit_str_concat/rules/{implicit,collection_literal}.rs`,
`pandas_vet/rules/{inplace_argument,nunique_constant_series_check}.rs`, `pandas_vet/helpers.rs`,
`flake8_print/rules/print_call.rs`, `flake8_type_checking/rules/{runtime_cast_value,runtime_string_union}.rs`, the
checker's `handle_node_store`, `in_dunder_all_assignment`, import, `global`, cast and TypedDict visiting in
`checkers/ast/mod.rs`, `resolve_load` and `add_module` in `ruff_python_semantic/src/model.rs`) and the fixtures, then
ran my own cases with Ruff and `rerail-engine-1120 run-policies` on `docs/rule-syntax` at `37fa5dcdbc`, on a fleet
box (pt-w02: pt-w01 has no `-1120` engine build). Translation lines are the finding's line (the engine's output names
the node, not the column).

PD002 and PD101 share `p10_seen`; T201, T203 and TC006 share `p10_tq`. A defect of a shared relation is shown once
and named on each rule it reaches.

## `ruff-isc001-single-line-implicit-string-concatenation`: defects (invalid code only)

Counterexample `mixed_tstring.py`:

```python
x = "a" t"b"
y = f"a" t"b"
z = t"a" "b"
```

```text
Ruff:        mixed_tstring.py:1:5 invalid-syntax (no ISC001)
             mixed_tstring.py:2:5 invalid-syntax (no ISC001)
             mixed_tstring.py:3:5 invalid-syntax (no ISC001)
Translation: mixed_tstring.py:1 ISC001
             mixed_tstring.py:2 ISC001
             mixed_tstring.py:3 ISC001
```

1. **A t-string beside a str or f-string (false positives).** Ruff's `implicit` pairs tokens: `String`/`String`,
   `String`/`FStringStart`, `FStringEnd`/`String`, `FStringEnd`/`FStringStart` and `TStringEnd`/`TStringStart`
   only. A t-string next to any other kind is no pair (and a syntax error). The rule takes any two adjacent
   `string` children. A bytes literal beside a str is a pair in Ruff (`b"a" "b"`: ISC001 and invalid-syntax); the
   translation agrees there.
2. **Unterminated strings (Ruff's own fixtures).** Ruff's lexer emits a string token for an unterminated string,
   so it reports pairs that tree-sitter's recovery does not give as a `concatenated_string`. On the fixtures
   `ISC_syntax_error.py` and `ISC_syntax_error_2.py`, Ruff reports ISC001 at `ISC_syntax_error.py:2, 11` and
   `ISC_syntax_error_2.py:2, 7`, and the translation reports nothing there. On lines both report, the counts
   differ: `ISC_syntax_error.py:3` and `:4` are two findings each in Ruff (columns 1 and 5) and one each in the
   translation; `ISC_syntax_error_2.py:5` is two in Ruff and three in the translation. The suite compares line
   keys, so none of this shows up as a difference.

Both affect only files Ruff reports as `invalid-syntax`.

## `ruff-isc004-implicit-string-concatenation-in-collection-literal`: defects

Counterexamples `comment_comma.py` and `except_pep758.py`:

```python
d = {}
y = d[
    "a" "b"  # note,
]
```

```python
try:
    pass
except "a" "b", ValueError:
    pass
```

```text
Ruff:        except_pep758.py:3:8 ISC004
Translation: comment_comma.py:3 ISC004
```

1. **A comma in a comment inside the brackets (`comment_comma.py:3`, false positive).** `d[…]` has one item, so
   there is no tuple and Ruff reports nothing. The third `comma_beside` clause matches the parent's whole
   `node_text` against `[\s\S]*,\s*\]?`, and that text includes the comment, `# note,` then `]`. The same comment
   placed earlier on the line (`d["a" "b"  # one, two` / `]`) is not reported, because the text does not end in
   `,`. The clause should read only tokens, not comments.
2. **PEP 758 unparenthesized `except` tuple (`except_pep758.py:3`, missed).** Ruff parses
   `except "a" "b", ValueError:` as an `except` with a tuple type and visits that tuple. It reports ISC004 at the
   default target (py310) and at py314. None of the `in_tuple` clauses covers `except_clause`.

## `ruff-pd002-pandas-use-of-inplace-argument`: defects

Counterexamples:

```python
# before_import.py
df.drop("a", inplace=True)
import pandas as pd
```

```python
# import_in_def.py
def load():
    import pandas
df.drop("a", inplace=True)
```

```python
# relative_import.py
from .pandas import helpers
df.drop("a", inplace=True)
```

```python
# type_string.py
import pandas
from typing import cast
x: "df.drop('a', inplace=True)" = 1
y = cast("df.fillna(0, inplace=True)", 1)
```

```text
Ruff:        type_string.py:3:18 PD002
             type_string.py:4:24 PD002
Translation: before_import.py:1 PD002
             import_in_def.py:3 PD002
             relative_import.py:2 PD002
```

1. **`seen_module` is the set of imports visited so far, not every import in the file (`before_import.py:1`,
   `import_in_def.py:3`, false positives).** Ruff sets the flag as it visits each import. Module-level code runs
   before function bodies, which are deferred, so module code above the import is not gated, and neither is module
   code when the import is inside a def. misc-seen's header names the first case as a known divergence. It also
   says "Function bodies … agree either way", which is wrong for an import made *in* a function body: module code
   does not see it. `p10_seen` inherits both. The rule's comment says "in a file that imports pandas" and does not
   mention this.
2. **Relative imports set no flag in Ruff (`relative_import.py:2`, false positive).** `add_module` is called for a
   `from` import only when `level == 0`. misc-seen counts `from .pandas import …` as pandas.
3. **Calls in string type definitions (`type_string.py:3, 4`, missed).** Ruff visits the parsed code of an
   annotation's string and of a `cast` type string, so `inplace=True` there is PD002. `last_inplace` reads
   `call_item`/`attr_of`, which have no rows in a type string's tree (the batch library says so for python/ruff's
   argument relations). The batch builds `p10_kwarg` for exactly this, but PD002 does not use it. T201, T203 and
   TC006 cover string type definitions; PD002, which shares the library, does not.

## `ruff-pd101-pandas-nunique-constant-series-check`: defects

Counterexamples:

```python
# match_capture.py
import pandas as pd
match pd.Series():
    case [first, *rest]:
        rest.nunique() == 1
    case cap:
        cap.nunique() == 1
```

```python
# later_def.py
import pandas as pd
s = pd.Series()
def check():
    return s.nunique() == 1
def s(): ...
```

```python
# walrus_all.py
import pandas as pd
__all__ = [(w := pd.Series())]
w.nunique() == 1
```

```python
# stub.pyi
import pandas as pd
s: pd.Series
b: bool = s.nunique() == 1
```

```python
# global_only.py
import pandas as pd
def check():
    global frame
    return frame.nunique() == 1
```

```python
# before_import.py
s = load()
s.nunique() == 1
import pandas as pd
```

```text
Ruff:        global_only.py:4:12 PD101
             match_capture.py:4:9 PD101
             match_capture.py:6:9 PD101
             stub.pyi:3:11 PD101
Translation: before_import.py:2 PD101
             later_def.py:4 PD101
             walrus_all.py:3 PD101
```

1. **Match captures (`match_capture.py:4, 6`, missed).** `visit_pattern` binds `case cap`, `*rest` and mapping
   rests as `BindingKind::Assignment`, which `test_expression` takes as RelevantLocal. `local_form` has no form for
   pattern bindings.
2. **A deferred body resolves to the module's last binding (`later_def.py:4`, false positive).** Ruff visits
   `check`'s body after the whole module, so `s` resolves to `def s()`, an IrrelevantBinding. `ref_site` takes the
   assignment. The author's NOTES name this ("`ref_site` is may-precise … Ruff's the latest") but cover only a
   module-level reading. Inside a function this is the ordinary case: any name later rebound by a def, class or
   import.
3. **A walrus inside a module-level `__all__ = …` (`walrus_all.py:3`, false positive).** `handle_node_store` tests
   `in_dunder_all_assignment` before the walrus case, so `w` is an Export. `export_site` covers only the `assign`
   and `augassign` forms.
4. **An annotation-only binding in a stub (`stub.pyi:3`, missed).** `resolve_load` resolves to an Annotation
   binding in stub files, and `test_expression` takes Annotation as RelevantLocal. `local_form` has no annotation
   form.
5. **`global` with no module binding (`global_only.py:4`, missed).** The load resolves to the function's
   `Global(None)` binding, which counts as RelevantLocal. `ref_site` has nothing to resolve to. Minor: if any module
   or other function assigns `frame` through `global`, both sides report.
6. **The `seen_module` order (`before_import.py:2`, false positive).** This is the same `p10_seen` defect as
   PD002 1 and 2 (import order, an import in a def, relative imports).

## `ruff-t201-print`: defects (string type definitions only)

Counterexamples:

```python
# two_imports.py
from typing import cast
from builtins import print
from rich import print
v = cast("print(1)", 1)
```

```python
# global_in_def.py
from typing import cast
def reset():
    global print
    print = None
v = cast("print(1)", 1)
```

```text
Ruff:        global_in_def.py:5:11 T201
Translation: two_imports.py:4 T201
```

1. **Two module imports of the name (`two_imports.py:4`, false positive).** String type definitions are visited
   after the module, and `resolve_load` takes the module scope's binding, which is the last one (`rich.print`).
   `p10_mod_import` takes every `ibind` of the name, so `builtins.print` matches too.
2. **A `global` rebinding inside a def (`global_in_def.py:5`, missed).** In Ruff the assignment binds in `reset`'s
   scope, so the module scope still has the builtin `print`, and the call is T201. `p10_tq` drops the builtin
   clause once `bound_in(m, "print")` holds, and the import clause has no import to take. [INFERENCE] `bound_in`
   counts the `global` rebinding as a module binding; I saw the result, not that relation's rows.

The rest agrees with Ruff in my cases: `file=` resolved through `from sys import stdout as so`, `file=(None)`,
`**{"file": f}`, `file=sys.stdout.buffer`, class-scope `print = 1` with a `cast("print(1)")` and annotation
string in the class, `def g(print)` with a string annotation, a module import after the cast (`from rich import
print` below), `sys` rebound at module level against `cast("print(…, file=sys.stdout)")`, and nested strings.

## `ruff-t203-p-print`: defect (string type definitions only)

Counterexample `fallback_import.py`:

```python
from typing import cast
try:
    from pprint import pprint
except ImportError:
    from rich.pretty import pprint
v = cast("pprint(1)", 1)
```

```text
Ruff:        (nothing)
Translation: fallback_import.py:6 T203
```

1. **The import-fallback pattern (false positive).** This is the same `p10_mod_import` defect as T201 1. Ruff's
   module-scope binding is the last one, `rich.pretty.pprint`. The translation also takes `pprint.pprint`.
   Swapping the two imports makes both sides report.

## `ruff-tc006-runtime-cast-value`: defects (string type definitions only)

Counterexamples:

```python
# two_imports.py
from typing import cast
from mylib import cast
x: "cast(int, 3)" = 1
```

```python
# starred_in_string.py
from typing import cast
y = cast("cast(*a)", 1)
```

```text
Ruff:        starred_in_string.py:2:16 TC006
Translation: two_imports.py:3 TC006
```

1. **Two module imports of `cast` (`two_imports.py:3`, false positive).** This is the same `p10_mod_import` defect
   as T201 1. In Ruff the string's `cast` is `mylib.cast`.
2. **A starred first argument in a type string's tree (`starred_in_string.py:2`, missed).** Ruff's cast visiting
   takes source-order item 0 when it is any positional `Arg`, `*a` included, and reports it. `ann_cast_type` reads
   `p10_arg0`, which is python-04's `ann_arg`, and that excludes `list_splat`. In ordinary code the rule agrees:
   `cast(*args)` is reported on both sides.

The rest agrees with Ruff in my cases: `cast(("int"), 1)`, a generator argument, `cast(val=1, typ=int)`, an
f-string, a concatenated str, a str+f-string concatenation, bytes, `cast(**kw)`, `cast(1, *args)`, `cast("int",
typ=str)`, `cast(int, typ=str)`, `def g(cast)` with a string annotation, and `typing` shadowed by a parameter.

## `ruff-tc010-runtime-string-union`: defects

Counterexamples:

```python
# bytes.py
y: b"int" | str
```

```python
# literal.py
from typing import Literal
z: Literal["a" | "b"]
```

```python
# typeddict_splat.py
from typing import TypedDict
D = TypedDict("D", {"x": int}, **{"a": "int" | str})
I = TypedDict("I", **{"a": "int" | str})
```

```text
Ruff:        typeddict_splat.py:2:40 TC010
             typeddict_splat.py:3:28 TC010
Translation: bytes.py:1 TC010
             literal.py:2 TC010
             literal.py:2 TC010
```

1. **Bytes members (`bytes.py:1`, false positive).** Ruff queues only `Expr::StringLiteral` as a string type
   definition (`visit_expr`, `Expr::StringLiteral` arm), and `runtime_string_union` runs on those alone. The rule
   matches `kind(m, "string")`, which includes bytes, and its comment says "a string (or bytes) literal".
2. **`Literal`'s strings (`literal.py:2`, two false positives).** Strings inside `Literal[…]` are not queued
   (`in_type_definition() && !in_typing_literal()`). The rule's comment still gives `x: Literal["a" | "b"]  ← TC010
   twice`. The author's NOTES say this example "is corrected" and that Ruff reports nothing. The NOTES treat it as
   shared with the original, but against Ruff it is a defect, and the rule's comment contradicts the tool.
3. **`**mapping` in a TypedDict call (`typeddict_splat.py:2, 3`, missed).** Ruff visits an unpacked keyword
   dictionary as a type definition (`None => true`, "Unpacked keyword dictionaries can contain type
   expressions"), so its string values are type strings at runtime. Neither misc2-typecheck nor `td_value` gives
   a dictionary splat's value as a runtime type definition.

The rest agrees with Ruff in my cases: module/class/def annotations, local annotations, `from __future__ import
annotations`, nested unions, `cast`, `TypeVar`, `NewType`, `list[…]`, `Optional[…]`, `Annotated` metadata,
f-strings, implicit concatenation, `TYPE_CHECKING`, nested strings, and the legacy and non-legacy TypedDict keyword
forms (`a=`, `None, a=`, `{…}, a=`, `extra_items=`, `total=`, `*args, a=`).

## The author's `different` entries

- **ISC001 (`right: translation`).** Ruff 0.17.6 reports ISC001 at all four translation-only keys:
  `ISC_syntax_error.py:3, 4, 10` and `ISC_syntax_error_2.py:4`. The verdict holds. The reason's last clause is
  inaccurate. `ISC_syntax_error_2.py:3` and `:5` are not lines "which neither side reports": run-policies reports
  both, and the suite counts them as `both`. The clause also leaves out `ISC_syntax_error.py:2`, which Ruff reports
  and both sides miss. See ISC001 2 above.
- **T201 (`right: translation`).** Ruff 0.17.6 reports T201 at `pyflakes/F401_0.py:73:1` and `:101:1`, and so does
  the translation. Confirmed.

I reran ptcheck at `tbelanger/pt1120-python-10` only for ISC001 (the counts above), not for the `same` rules. Every
defect above comes from my own cases.
