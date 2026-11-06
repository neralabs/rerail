# py-pyi: 54 Ruff flake8-pyi rules, hand-written

Each rule follows Ruff 0.16.9, the version the JSON was written from. I read its source for the rules where the
behaviour was unclear. I ran ruff 0.15.7, the installed version, on every counterexample below. The two versions
differ only where D6 says. Every rule has the JSON's revision raised to 2, its scope, its codes and its messages.
The rules share two new libraries:

| file | lines | what |
|---|---|---|
| `lib/python/typing.rr` | 515 | stub files; `sys.version_info` and `sys.platform` tests; typing names and TypeVar calls; stub bodies and docstrings; type expressions (both tree-sitter subscript shapes, unions, Literals, Ruff's type context); the values a stub may keep; unused private declarations; method kinds, metaclasses, enum bodies, class ancestors (same file), `__all__` exports |
| `lib/python/typing-context.rr` | 43 | `typing_only_ctx(n)`: code Ruff's execution context calls typing, not runtime: deferred annotations (stub, future import, 3.14+, local variables), string annotations, `type X = …` values, PEP 695 bounds and defaults, `if TYPE_CHECKING:` bodies |

The rules are 14 to 92 lines long, 1,212 lines in all, comments and directives included. The longest is PYI036
(92 lines), which has six kinds of message.

## Scores

`rrlab score` cannot score these rules. None of them is in `python/conformance.json` ("cannot tell which
upstream tool to score against"), and the recording has no PYI findings. Its only flake8-pyi fixture is
`PYI021_1.pyi`, which records one PIE790 finding.

I scored each rule instead on Ruff 0.16.9's own fixtures (`crates/ruff_linter/resources/test/fixtures/flake8_pyi`)
against its expected-output snapshots. I took only the plain snapshots `<rule-name>_<fixture>.snap` and skipped
the `preview_` and `py38_` variants. I ran each fixture through `rrlab run` with `{"pythonVersion": "3.14"}`,
because Ruff's tests run at 3.14.

The TP / FP / FN column counts (file, line) pairs; the next column counts (file, line, column) findings.

- 49 of the 54 rules are perfect, at the column level too.
- The other five miss only what the harness cannot model, G1–G4 below.
- In all: 739 / 1 / 18 at the line level, and 766 of 784 findings exact to the column.

| rule | lines | TP / FP / FN | exact / expected | deviations, gaps |
|---|---|---|---|---|
| `ruff-pyi001-unprefixed-type-param` | 23 | 3 / 0 / 0 | 3 / 3 | - |
| `ruff-pyi002-complex-if-statement-in-stub` | 21 | 4 / 0 / 0 | 4 / 4 | - |
| `ruff-pyi003-unrecognized-version-info-check` | 27 | 17 / 0 / 0 | 17 / 17 | - |
| `ruff-pyi004-patch-version-comparison` | 16 | 4 / 0 / 0 | 4 / 4 | - |
| `ruff-pyi005-wrong-tuple-length-version-comparison` | 19 | 2 / 0 / 0 | 2 / 2 | - |
| `ruff-pyi006-bad-version-info-comparison` | 24 | 16 / 0 / 0 | 16 / 16 | - |
| `ruff-pyi007-unrecognized-platform-check` | 23 | 3 / 0 / 0 | 3 / 3 | - |
| `ruff-pyi008-unrecognized-platform-name` | 18 | 1 / 0 / 0 | 1 / 1 | - |
| `ruff-pyi009-pass-statement-stub-body` | 16 | 2 / 0 / 0 | 2 / 2 | - |
| `ruff-pyi010-non-empty-stub-body` | 16 | 3 / 0 / 0 | 3 / 3 | - |
| `ruff-pyi011-typed-argument-default-in-stub` | 16 | 18 / 0 / 0 | 18 / 18 | - |
| `ruff-pyi012-pass-in-class-body` | 15 | 6 / 0 / 0 | 6 / 6 | - |
| `ruff-pyi013-ellipsis-in-non-empty-class-body` | 16 | 19 / 0 / 0 | 19 / 19 | - |
| `ruff-pyi014-argument-default-in-stub` | 15 | 13 / 0 / 0 | 13 / 13 | - |
| `ruff-pyi015-assignment-default-in-stub` | 24 | 11 / 0 / 0 | 11 / 11 | - |
| `ruff-pyi017-complex-assignment-in-stub` | 23 | 4 / 0 / 0 | 4 / 4 | - |
| `ruff-pyi018-unused-private-type-var` | 15 | 10 / 0 / 0 | 10 / 10 | D1 |
| `ruff-pyi019-custom-type-var-for-self` | 32 | 69 / 0 / 10 | 69 / 79 | D7, G1, G4 |
| `ruff-pyi020-quoted-annotation-in-stub` | 26 | 9 / 0 / 0 | 9 / 9 | - |
| `ruff-pyi021-docstring-in-stub` | 14 | 5 / 0 / 0 | 5 / 5 | - |
| `ruff-pyi024-collections-named-tuple` | 14 | 6 / 0 / 0 | 6 / 6 | - |
| `ruff-pyi025-unaliased-collections-abc-set-import` | 19 | 11 / 0 / 0 | 11 / 11 | - |
| `ruff-pyi026-type-alias-without-annotation` | 20 | 6 / 0 / 0 | 6 / 6 | - |
| `ruff-pyi029-str-or-repr-defined-in-stub` | 18 | 3 / 0 / 0 | 3 / 3 | - |
| `ruff-pyi030-unnecessary-literal-union` | 14 | 57 / 0 / 0 | 57 / 57 | - |
| `ruff-pyi032-any-eq-ne-annotation` | 17 | 4 / 0 / 4 | 4 / 8 | G1 |
| `ruff-pyi033-legacy-type-comment` | 17 | 11 / 0 / 0 | 11 / 11 | - |
| `ruff-pyi034-non-self-return-type` | 42 | 50 / 0 / 0 | 50 / 50 | D6 |
| `ruff-pyi035-unassigned-special-variable-in-stub` | 20 | 3 / 0 / 0 | 3 / 3 | - |
| `ruff-pyi036-bad-exit-annotation` | 92 | 34 / 0 / 0 | 39 / 39 | - |
| `ruff-pyi041-redundant-numeric-union` | 24 | 38 / 0 / 0 | 38 / 38 | - |
| `ruff-pyi042-snake-case-type-alias` | 14 | 8 / 0 / 0 | 8 / 8 | - |
| `ruff-pyi043-t-suffixed-type-alias` | 15 | 8 / 0 / 0 | 8 / 8 | - |
| `ruff-pyi044-future-annotations-in-stub` | 17 | 2 / 0 / 0 | 2 / 2 | - |
| `ruff-pyi045-iter-method-return-iterable` | 18 | 14 / 0 / 0 | 14 / 14 | - |
| `ruff-pyi046-unused-private-protocol` | 16 | 6 / 0 / 0 | 6 / 6 | D1 |
| `ruff-pyi047-unused-private-type-alias` | 32 | 8 / 0 / 0 | 8 / 8 | D1, D2 |
| `ruff-pyi048-stub-body-multiple-statements` | 16 | 3 / 0 / 0 | 3 / 3 | - |
| `ruff-pyi049-unused-private-typed-dict` | 27 | 7 / 0 / 0 | 7 / 7 | D1, D2 |
| `ruff-pyi050-no-return-argument-annotation-in-stub` | 21 | 13 / 0 / 2 | 14 / 16 | D3, G1 |
| `ruff-pyi051-redundant-literal-union` | 24 | 16 / 0 / 0 | 24 / 24 | - |
| `ruff-pyi052-unannotated-assignment-in-stub` | 17 | 14 / 0 / 0 | 14 / 14 | - |
| `ruff-pyi053-string-or-bytes-too-long` | 39 | 8 / 1 / 0 | 8 / 8 | D8, G2 |
| `ruff-pyi054-numeric-literal-too-long` | 28 | 8 / 0 / 0 | 8 / 8 | - |
| `ruff-pyi055-unnecessary-type-union` | 19 | 31 / 0 / 0 | 31 / 31 | D4 |
| `ruff-pyi056-unsupported-method-call-on-all` | 20 | 6 / 0 / 0 | 6 / 6 | - |
| `ruff-pyi057-byte-string-usage` | 23 | 8 / 0 / 0 | 8 / 8 | - |
| `ruff-pyi058-generator-return-from-iter-method` | 40 | 17 / 0 / 0 | 17 / 17 | D5 |
| `ruff-pyi059-generic-not-last-base-class` | 23 | 13 / 0 / 0 | 13 / 13 | - |
| `ruff-pyi061-redundant-none-literal` | 18 | 48 / 0 / 0 | 53 / 53 | - |
| `ruff-pyi062-duplicate-literal-member` | 15 | 24 / 0 / 2 | 32 / 34 | G3 |
| `ruff-pyi063-pep484-style-positional-only-parameter` | 20 | 20 / 0 / 0 | 20 / 20 | - |
| `ruff-pyi064-redundant-final-literal` | 28 | 10 / 0 / 0 | 10 / 10 | - |
| `ruff-pyi066-bad-version-info-order` | 26 | 5 / 0 / 0 | 5 / 5 | - |

## Deviations from the JSON

Each counterexample is what Ruff reports.

**D1. PYI018, PYI046, PYI047, PYI049 check every file.** The JSON limits them to `.pyi`, but Ruff checks module
and function scopes in `.py` files too. It checks class bodies only in stubs.

```python
# a.py
from typing import TypeVar
_T = TypeVar("_T")       # PYI018: Private TypeVar `_T` is never used
```

**D2. In a stub, PYI047 and PYI049 report an unread comprehension variable inside the declaration.** Ruff checks
the comprehension's scope with the same rule. I kept this quirk.

```python
# a.pyi
_F2: TypeAlias = list[[0 for _y in range(3)]]   # PYI047 at `_F2`, and at `_y`
```

**D3. PYI050 applies to `.py` files when the target is Python 3.11 or later.** Ruff's gate is
`is_stub() || target_version() >= PY311`. The target comes from the `pythonVersion` option, read by
python/version's `targets`; Ruff's default is 3.10. The JSON checks only `.pyi`.

```python
# a.py, target 3.12
def f(x: NoReturn): ...   # PYI050
```

**D4. PYI055 reads Ruff's typing-only context** (python/typing-context's `typing_only_ctx`). Ruff checks a
union only where its execution context is not runtime:
- an annotation in a stub, or under `from __future__ import annotations`;
- any annotation at a 3.14+ target (PEP 649);
- a local variable's annotation in a function. A def's parameter and return annotations run, a nested def's too;
- a `type X = …` value, at any version;
- the body of `if TYPE_CHECKING:`.

Values that run are not checked, in a stub too: `X: TypeAlias = …`, `cast(…)`, a TypeVar's bound, `W = …`. The
JSON's clauses are a stub or the future import, and inside annotations.

```python
# a.pyi
X: TypeAlias = type[int] | type[str]   # fine: the value runs
type Y = type[int] | type[str]         # PYI055
# a.py, target 3.10, no future import
def f():
    x: type[int] | type[str] = int     # PYI055
```

**D5. PYI058 checks every file.** The JSON limits it to `.pyi`, and Ruff's snapshot for `PYI058.py` has findings.
Outside a stub, Ruff reports only a body that just yields. Each statement must be one of these:
- `pass`, a string or `...`;
- `yield` or `yield from`;
- a `return` with no value, or None, once a yield has run.

```python
# a.py
class C:
    def __iter__(self) -> Generator[int, None, None]: ...   # PYI058
class D:
    def __iter__(self) -> Generator[int, None, None]:
        yield 1
        return 5                                            # not reported: returns a value
```

**D6. PYI034's in-place operators follow 0.16.9.** An in-place operator is reported only when it returns the
class by name. Ruff 0.15.7 also reports one that returns another type: `def __isub__(self, x) -> int` in class
`D` is PYI034 in 0.15.7 and clean in 0.16.9.

```python
class D:
    def __iadd__(self, x) -> D: ...     # PYI034 (both versions)
    def __isub__(self, x) -> int: ...   # 0.16.9: fine
```

**D7. PYI019 does not read `builtins.type[S]` as `type[S]`.** Ruff matches the plain name `type` only, a known
false negative (ruff#15821). I followed the tool. `typing.Type[S]` is not read either.

```python
class C:
    @classmethod
    def m[S](cls: builtins.type[S]) -> S: ...   # no PYI019
```

**D8. PYI053 skips strings in a module-level `__all__` assignment (`=`, `+=`, annotated).** That is 0.16.9;
0.15.7 reports them. It also skips every argument of `warnings.deprecated(…)` / `typing_extensions.deprecated(…)`,
keyword arguments included. `typing.deprecated` is not on that list.

```python
# a.pyi
__all__ = ["aaaa…a"]                        # 60 a's: 0.16.9 fine, 0.15.7 PYI053
@deprecated(message="aaaa…a")               # fine
def f(): ...
```

These are Ruff behaviours that the rules model. I could not tell from the JSON whether it intends them:
- PYI002, 003, 004, 005, 007 and 008 read only an `if` test, not its elifs. Each operand of a top-level
  `and`/`or` is checked on its own (typing.rr's `stub_test`). PYI006 and PYI066 read the if and the elif tests,
  with one level of `and`/`or`.
- PYI003 reads an int in a tuple through parentheses: `(3, (8))` is fine. PYI005 checks the tuple's length
  whatever its elements are: `sys.version_info[:2] == (3, "8", 1)` is reported.
- PYI015, PYI017 and PYI052 skip function bodies. PYI026 and PYI052 skip enum members.
- PYI041 reads parameter annotations only, where the union is the whole annotation.
- PYI053 counts code points of the value. It skips string statements, docstrings included (and see D8).
- PYI048 counts a docstring as a statement.
- PYI051 covers float and complex literals (`Literal[1J] | complex`).
- PYI034 skips classes that Ruff's `is_metaclass` calls Yes or Maybe. That includes a base of `type(X)`, or bases
  together with a metaclass-shaped `__new__(cls, name: str, bases: tuple, attrs: dict, **kw)`. PYI019 skips
  only Yes (`metaclass_yes`).
- PYI019 counts only the def's own PEP 695 parameters: in `class Foo[S]: def m(self: S) -> S` it is silent.
- PYI036 checks only defs that are statements of the class body itself (`class_member`): a def under an `if` in
  the class is skipped, and overload pairs are counted the same way. For `L | R` it takes L if L is not None,
  else R, as Ruff's non_none_annotation_element does: `type[BaseException] | int` is accepted for slot 1.
- PYI032 counts only ordinary positional parameters: `def __eq__(self, other: Any, /)` is not reported.
- PYI018, 046, 047 and 049 count a string in the module's `__all__` as a use.
- PYI034 and PYI045 match typing and collections.abc iterator types, not typing_extensions ones. PYI058 matches
  all three.
- Base classes are followed within the file only (`class_ancestor`), as Ruff's semantic model does.

## Harness gaps (rrlab), not rule limits

- **G1.** `parsed`, `str_part` and `string_annotation` have no rows, so string annotations (`x: "NoReturn"`) are
  not read. This accounts for all 18 FN:
  - PYI019: 8, the stringized forms;
  - PYI055: 0 here, but the reviewer's cases with a quoted annotation (`a: "type[int] | type[str]"`);
  - PYI032: 4, `other: "Any"`;
  - PYI050: 2;
  - PYI062: 0 here, but 1 in my own cases;
  - in my own cases, PYI054, 055, 057 and 061 each miss one quoted annotation, and PYI018 has 2 FP because
    the name is read only inside a string.
- **G2.** rrlab's `value` does not decode `\U0001f600`: it gives `U0001f600`, nine characters for one. This
  causes the one FP, in PYI053 (`PYI053.pyi:10`, a 50-character string ending in that escape).
- **G3.** `@tree_digest` is a stub in rrlab. PYI062 compares a non-scalar Literal member, such as
  `Literal[{1, 3, 5}, "x", {1,3,5}]`, by `@tree_digest`, so the 2 FN (`PYI062.py:9`, `PYI062.pyi:9`) need the
  real functor.
- **G4.** The 2 remaining PYI019 FN (`PYI019_0.py:54`, `.pyi:54`) come from the test's settings:
  `pep8-naming.classmethod-decorators = ["foo_classmethod"]`. The rule has no such option.

## Library and core changes

1. **python/ast `docstring` depends on `str_part`.** Under rrlab it finds nothing. typing.rr adds `doc_stmt(s, e)`
   and `py_docstring(o, e)`, which read `plain_str`. The fix is either that `docstring` falls back to
   `plain_str`, or that rrlab models `str_part`.
2. **python/names: bind `type X = …` statements.** PYI047 reads them with a text-based `type_alias_read`,
   because names has no binding for a PEP 695 type alias. The proposal: `binding(scope, "X", <X>, …)` for the
   `left` name of a `type_alias_statement`, and `look` for its uses.
3. **python/names `class_base` resolves only unsubscripted bases.** typing.rr's `class_base_sym` and
   `class_ancestor` read through `Base[T]`. The proposal is that `class_base` / `subclass` do the same, as
   Ruff's `map_subscript` does. `metaclass`, `abc_base` and `in_enum` could then use `subclass`.
4. **python/names `capture_parent`.** While I worked, the working tree had `.decl capture_parent(k: symbol)`,
   which broke every rule that includes names. It is now `p: node`. I note it only as observed.
5. **rrlab:** model `parsed` (G1), decode `\U` and `\N{…}` escapes in `value` (G2), and implement
   `@tree_digest` (G3).
6. **Name clash: `typing_only`.** The review asked for a library relation `typing_only(n)`, but upgrade.rr
   already declares `typing_only(n)` with another meaning (n is in an annotation). Mine is
   `typing_only_ctx(n)`, in lib/python/typing-context.rr. It uses upgrade.rr's `future_annotations` and
   `up_stub`, version.rr's `targets`, and misc2-typecheck.rr's `tc_block` and `tc_ann_owner`.
7. **lib/python/security.rr:** its qualified-call vocabulary follows Bandit's view, not Ruff's resolution, so
   these rules use names' `sym_of` instead. No change is needed for this batch.

## Duplicates to merge

These are relations with the same meaning in other batches' new libraries. Each line gives the name here, its
equivalents elsewhere, and how they differ.

- `type_bare(n, m)`: ruff-classes.rr `ann_strip(t, m)`.
- `typing_sym(e, x)`:
  - upgrade.rr `typing_ref(e, x)`, which resolves by `sym_at`;
  - ruff-classes.rr `typing_name(q, r)`, which works on the qualified name.
- `typing_mod(m)`: upgrade.rr `up_typing_module(m)`.
- `tsub(s, h, i, a)`, `tsub_n(s, k)`, `typing_subscript(s, x)`:
  - upgrade-typing.rr `sub_item` / `type_arg`, `up_type_nargs`, `up_typing_sub`, `sub_of`;
  - typing.rr skips comments between arguments and reads both tree-sitter shapes.
- `stub_file(d)`: upgrade.rr `up_stub(d)`.
- `plain_target(t, x)`, a name or `(x)`: bugbear-loops.rr `assign_name(r, x)`, which also takes names inside tuple
  targets. Mine was renamed from `assign_name` to avoid the clash.
- `pyi_int(e)`: ruff.rr `ruff_int(e, v)`.
- `doc_stmt` / `py_docstring`: upgrade.rr `docstring_stmt(m, st)`.
- `typing_only_ctx(n)` / `deferred_ann(r)` (typing-context.rr):
  - misc2-typecheck.rr `tc_runtime_typedef`, the complement for annotations;
  - upgrade-typing.rr `runtime_annotation` / `local_ann`;
  - upgrade.rr `late_annotation`.

  One "is this evaluated" library should serve all of them.
- `in_dunder_all(n)`: python/names or pyflakes may already model `__all__`. If one does, PYI053 and the unused
  private rules should read it.
- `pyi_method(c, f, x)`: calls.rr `method_of(f, c)`, which has no name column, and names' `method(cq, x, f)`.
- `class_base_sym(c, q)`: ruff-classes.rr `class_base_expr(c, e)` and names' `class_base` (see change 3).
