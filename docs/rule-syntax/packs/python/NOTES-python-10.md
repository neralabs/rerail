# python-10: eight pyupgrade (UP) translations made to report what Ruff reports

Rules: UP001, UP003, UP004, UP006, UP007, UP010, UP011, UP012. Ground truth: Ruff 0.17.6 on a fleet box,
`ruff check --isolated --target-version py310 --select <CODE>` (3.10 is the pack's default target). Its findings
on the whole fixture corpus equal the recorded ones (`expected.json`) for all eight codes, so every difference
below was decided against Ruff itself, on the corpus file and on counterexamples (scratch, not committed).

New library: `lib/python/python-10.rerail` (UP006, UP007): typing names inside strings Ruff reads as types.

## UP001 useless-metaclass-type: follows Ruff

- Ruff checks every `__metaclass__ = type`, not only those in a class body: `__metaclass__ = type` at module
  level and in a function are UP001. The translation required a class scope.
- A parenthesised target is a Name to Ruff: `(__metaclass__) = type` and `((__metaclass__)) = (type)` are UP001.
  tree-sitter reads `(x)` as a one-element tuple_pattern; python/typing's `plain_target` reads it.
- An annotated assignment is not an Assign: `__metaclass__: type = type` is not UP001 (the translation reported it).

## UP003 type-of-primitive: follows Ruff

- Ruff wants one positional argument and ignores keywords: `type("", x=1)` is UP003.
- Parentheses: `type(("a"))` and `(type)(1)` are UP003.
- A comment or a backslash continuation in the argument list is no argument: `type(\` / `1)` is UP003. The
  continuation is a `line_continuation` child of argument_list, which python/upgrade's `up_arglist` keeps (it
  skips comments only); the rule leaves it out itself. Library change proposed: `up_arglist` should skip
  `line_continuation` like `comment`.

## UP004 useless-object-inheritance: follows Ruff

- A parenthesised base is the base: `class A((object))` and `class B(( builtins.object ), metaclass=object)` are
  UP004, at the inner expression.

## UP006 non-pep585-annotation: follows Ruff; since: three JSON misses, no translation misses

- The JSON original misses pyflakes/F401_0.py 44 (`foo: Type["NamedTuple"]` in a class body), 59 (`TypeVar("Y",
  bound="Dict")`) and 60 (`TypeVar("Z", "List", "Set")`); Ruff reports all three, and so does the translation.
- String annotations: `def f(x: "List[str]")`, `x: """List['Path']"""`, `"Li" "st[str]"` are UP006. python/upgrade's
  `sym_at` resolves a name from its own scope in Ruff's walk order; a node of a parsed string has no scope, so
  `typing_ref` never held inside strings (UP006_0.py 29-57, the C40x/PTH20x/T20x string-annotation probes,
  UP037_1.py, F401_11.py, the dvc sample). python-10 reads them through python/typing's `typing_sym` (names' `qn`
  and `look`, which start a parsed name's lookup at its string).
- Strings that are types outside annotations: `X: TypeAlias = 'Dict'`, `type B = 'Dict'`, `TypeVar("Y",
  bound="Dict")`, `TypeVar("Z", "List", "Set")`, `Annotated[("List[int]", "List[int]"), …]` at runtime (TC008.py
  16, 32; TC008_typing_execution_context.py 14; F401_0.py 59, 60; UP006-annotated-metadata-in-generic.py 5). The
  engine parses these when python/typing's `type_ctx` asks, but names starts no lookup in them (its `wstart` covers
  `parsed_in`, annotation strings only). python-10's `xsym` resolves such a name from the string's scope outwards
  to the first scope that binds it, and reads that scope's import (Ruff resolves these strings after the module).
  Library change proposed: names' `parsed_in` (and so `wstart`) should cover every string typing's `type_ctx`
  parses; `xsym` would then go.
- Annotated metadata is not a type: `a: Optional[Annotated[int, "List[int]"]]` is not UP006 (names' `parsed_in`
  parses that string; python-10 also requires `type_ctx`, which stops at the metadata).
- PEP 695 type parameters: in `class C[Tuple](Tuple)` the base reads the type parameter (SLOT001-type-parameter-
  does-not-shadow-base.py 12). `sym_at` does not know type parameters; the rule leaves out names read inside a
  definition declaring a type parameter of that name (python/ast's `tparam_named`). Library change proposed:
  upgrade's `sym_at` should honour type-parameter scopes, as names' `tp_ref` does.
- Fixed since: flake8_pyi/PYI036.pyi:82, the previous remaining translation miss. In a deferred annotation inside a function (a stub, `from
  __future__ import annotations`, or a string), Ruff resolves a name to the module's binding even when the
  function rebinds it before the use: 

  ```python
  from typing import List
  def h(List):                 # or `List = list` in the body; in a stub, `from collections import deque as List`
      z: "List[int]" = []      # Ruff 0.17.6: UP006; before the fix, the translation read the parameter
  ```

  With no module binding, the function's own import is used (`def h(): from typing import List; x: "List[int]"`
  is UP006). This was the remaining corpus line before the semantic-model rule below was implemented.

  Fixed since (rule fixes on #1120's line). The rule is PEP 563's, in Ruff's semantic model: `resolve_load` and
  `lookup_symbol_in_scope` ask the module scope first when `in_forward_reference()` (a string type definition, or a
  deferred annotation that is typing-only), and take its binding unless that is unbound (`is_unbound`: a deletion,
  an `except … as` target after its handler; a bare annotation never replaces a binding). python/upgrade's
  `up_global` and python-10's `str_global` do the same through `global_site`, the module's last binding; a
  singledispatch implementation's runtime-required annotations (`@fun.register`, its non-variadic parameters and
  return) read their scopes as before. Checked against Ruff on the rule's `.test`s; the 83 rules that include
  python/upgrade keep their counts on fixtures and samples-python but UP006, which gains PYI036.pyi:82.

## UP007 non-pep604-annotation-union: follows Ruff

- String annotations, and strings that are types elsewhere (`TypeAliasType("Json", "Union[…]")`, F401_34.py;
  `cast("list[tuple[Union[…]]]", 3.0)`, TC006.py 34): python-10, as for UP006. `TypeAliasType`'s value is not in
  python/typing's `type_ctx`; python-10 adds it for its own strings.
- Ruff skips a "complex" string: adjacent strings or an escape in it (`"Union[flo" "at, complex]"`, `"Uno[in\`
  newline `t, …]"`, `"Union[type[int], \x74ype[str]]"`, PYI041_3.py 122-128, PYI041_4.py 4 and 7, the PYI055
  escaped/concatenated probe). UP006 does not skip them (UP006_0.py 45 `"Li" "st[str]"` is UP006).
- A quoted member blocks the rewrite only where the code runs. Code under `if TYPE_CHECKING:`, a PEP 695 bound and
  a `type X = …` value never run, so `b: Union[int, "Foo"]` there is UP007 (FA100-parenthesized-type-checking.py 7,
  UP007-typing-only-contexts.py 6, 9, 12). The rule now asks python/typing-context's `typing_only_ctx` instead of
  "not in an annotation, or in a runtime one".

## UP010 unnecessary-future-import: follows Ruff

- Ruff takes a relative `from .__future__ import division` (`..__future__` too; not `.pkg.__future__`, not
  `from . import __future__`) for a `__future__` import. Such an import binds its names like any import, so a name
  counts as used when a load lands on the import's scope, or when the module's `__all__` lists it:

  ```python
  from .__future__ import print_function
  print(print_function)        # not UP010 (nor with __all__ = ["print_function"] instead)
  ```

## UP011 lru-cache-without-parameters: follows Ruff

- Parentheses around the callee or the decorator, comments and backslash continuations between the parentheses:
  `@(functools.lru_cache  # a` / `)(  # c` / `\` / `)` is UP011.
- Ruff's range starts at the callee's end, so the finding's line is the line the callee ends on: the rule reports
  at the callee's last name (it was at the argument list, a line later in that probe).

## UP012 unnecessary-encode-utf8: follows Ruff

- A parenthesised callee: `("a".encode)()` and `(("a").encode)(encoding="utf8")` are UP012.
- In the batch: pyupgrade/UP012.py:86, `x: '"foo".encode("utf-8")'`, original only. Ruff checks the call inside the
  string annotation; the engine parsed the string, but its strings had no `value` row, so python/ast's `plain_str`,
  and with it upgrade-format's `str_lit` and `utf8_only_arg`, held for none of them.
- Fixed since (rule fixes on #1120's line): parsed Python guests keep their tapes, so their strings have `value`
  rows, and `encode_on` also takes calls in a type string (`annotation(c)`). The rule's `.test` checks ASCII,
  non-ASCII and f-string receivers there against Ruff 0.17.6, with Literal values and ordinary strings not
  reported. `translations:check`: `same` (fixtures 51 shared findings).
