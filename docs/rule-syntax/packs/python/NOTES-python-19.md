# NOTES python-19

Ground truth: ruff 0.17.6 on a fleet box (`ruff check --select N8xx --no-cache`). Checked with `ptcheck` against the
integration branch. Every baseline difference was on a `ruff/rerail/probes/` or `ruff/pep8_naming/` fixture; Ruff
agreed with the original on each, so each was a translation defect, fixed below, except one key the engine cannot
match (N812, below).

Final ptcheck (exit 0): N801, N803, N804, N805, N806, N807, N811 `same` on examples, fixtures and samples-python;
N812 `different` by that one fixtures key, original-only. Since fixed (below): all eight `same`.

New library: `lib/python/python-19.rerail`, used by all eight rules. It holds what `lib/python/naming.rerail` gets
wrong for them; each part is a proposed change to naming.rerail, which the rules can switch back to once it lands.

## Shared: names as Ruff reads them (all eight rules)

Ruff's parser NFKC-normalises every identifier, as CPython does, and every pep8-naming rule reads the normalised
name; `ruff_python_stdlib::str`'s case tests are Unicode's (Rust's `char::is_uppercase` / `is_lowercase` /
`is_alphabetic`). naming.rerail tests the raw text with ASCII classes (`[A-Z]`). Counterexamples, all Ruff's:

| Code | Ruff | naming.rerail |
|---|---|---|
| `class Ｅrror(Exception)` (N801) | not reported: `Error` | reported: `Ｅ` is not `[A-Z]` |
| `def _＿x__(): …` (N807) | reported: `__x__` | not |
| `def __ｇｅｔａｔｔｒ__(name)` at module level (N807) | not: PEP 562's `__getattr__` | reported |
| `from m import Ⅱ as Two` (N811) | reported: `II` is cased uppercase, two characters | not |
| `from m import Ⅰ as One` (N811) | not: `I` is one character, `One` CamelCase | not |
| `from mod import foo as 日本` (N812) | reported: `日` is alphabetic, not lowercase | not |
| `from m import a日 as X1` (N812) | reported: `a日` is cased lowercase | reported |
| `def f(x日): …` (N803), `x日 = 1` in a def (N806) | reported | not |
| `from m import Ä as Ab` (N811) | not: the one-character exemption counts characters, not bytes | (not) |

`pep8_name` / `pep8_word` normalise with core's `@nfkc`; `pep8_lower`, `pep8_cased_lower`, `pep8_cased_upper`,
`pep8_camel`, `pep8_capwords` are the Unicode regexes; `pep8_ignored` is `naming_ignored` over the normalised word.
Proposed: naming.rerail's `naming_word` normalises, and its case regexes use `\p{Lowercase}`, `\p{Uppercase}`,
`\p{Alphabetic}` as here.

## Shared: method kinds as Ruff classifies them (N804, N805)

`function_type::classify` resolves each decorator (call taken off) as the name was bound when Ruff visited it.
naming.rerail uses `sym_of`, which is flow-insensitive and does not open parentheses; python/ruff's `ruff_qual` is
ordered but misses that a class body at module level is visited with the module, and treats a deferred body as
ordered too. `pep8_qual` / `pep8_builtin` follow Ruff (each row checked with ruff on the box):

| Case | Ruff |
|---|---|
| `@(staticmethod)` / `@(classmethod)` on a method | static / class method |
| `@staticmethod` in a module-level class, `staticmethod = …` later in the module | the builtin |
| the same class inside a `def` (Ruff visits function bodies after the module) | not the builtin: N805 |
| `classmethod: object` (no value) anywhere in the module | the builtin |
| `except E as classmethod` before the class, nothing else | the builtin (the name is unbound after the handler) |
| `classmethod = 1`, then `except E as classmethod`, then the class | not the builtin: the handler restores `1` |
| `classmethod = 1` in the class body between two `@classmethod` defs | the first is the builtin, the second not |
| `classmethod = 1` later in the def that holds the class | the builtin (that body is visited in order) |

`is_metaclass` follows bases through the classes of the file, a subscripted base by its value
(`class Meta(type, Generic[T])`, `class A(Meta[int])`: A's methods are class methods, `def __enter__(self)` is
N804). `pep8_meta_any` adds that to `naming_meta_any`.

Proposed: naming.rerail's `naming_classmethod` / `naming_staticmethod` resolve decorators by `pep8_qual`, and
`naming_meta_any` follows subscripted bases. `pep8_builtin` is a general "the builtin as Ruff resolves a name at
visit time" and could replace `ruff_qual`'s builtin clauses in python/ruff.rerail.

## ruff-n801-invalid-class-name

Baseline: fixtures 1 translation-only, `N818-nfkc-suffix-and-base.py:5` (`class Ｅrror(Exception)`); Ruff does not
report it. Fixed by the normalised names.

## ruff-n803-invalid-argument-name

Baseline: fixtures 2 translation-only, `ANN002-star-spacing-and-overloads.py:5` and `:7`:
`def spaced(*  # star comment` + newline + `args, …)`. python/ast's `param_parts` takes the splat's first child as
its name, here the comment, whose text is not lowercase. The rule now takes the splat's identifier when that child
is a comment (`n803_param`). Proposed: `param_parts` skips comments.

## ruff-n804-invalid-first-argument-name-for-class-method

Baseline: fixtures 1 original-only, `PYI034-subscripted-same-file-bases.py:5` (the metaclass through `Meta[int]`).
Fixed by `pep8_meta_any`.

## ruff-n805-invalid-first-argument-name-for-method

Baseline: fixtures 5 translation-only: `ANN001-staticmethod-shadowed-later-in-module.py:3,7`,
`ANN206-annotation-only-and-except-bound-classmethod.py:11`, `B019-parenthesized-decorator.py:5,9`. Fixed by
`pep8_qual` (table above).

## ruff-n806-non-lowercase-variable-in-function

Baseline: fixtures 6 original-only, 1 translation-only.

- `N806.py:65, 67, 69, 71` (×2): match captures (`case BadName`, `case int(GoodName)`, `case [*Rest]`,
  `case {"key": Val} as Alias`). naming.rerail's `store_name` documents them but reads `expr_ctx`, which captures
  lack (ast keeps them as strings). The rule adds python/ast's `py_capture`.
- `N806-comprehension-first-iterable-walrus.py:2`, `:3`: `[x for x in (Y1 := range(3))]` in a def. A
  comprehension's first iterable runs in the scope around it, so Ruff is in the function; `store_name` puts the
  walrus in the comprehension. Ruff reports neither a walrus in a nested comprehension's first iterable (the outer
  comprehension's scope), nor in a condition or a later iterable.
- `N815-scope-edges-nested-typeddict-and-with-targets.py:19` (translation-only):
  `def f(self, x: (aB := int) = (cD := 1)) -> (eF := int)` in a class body. The default runs in the scope around
  the def, here the class, and Ruff never puts the annotations in a function's scope; `store_name` put all three in f.

`n806_scope` follows Ruff: a node in a def's or lambda's parameters (a default), or in a comprehension's first
iterable, is visited in the scope around that one (recursively). A def's parameter and return annotations are never
in a function's scope: in `def f(): def g(p: (Aa := int), q=(Bb := 1)) -> (Cc := int): …` Ruff reports only `Bb`,
and `x: (Dd := int) = 1` in f's body is reported (`n806_def_ann`). Proposed for naming.rerail's `store_name`.

## ruff-n807-dunder-function-name

Baseline: fixtures 1 original-only (`N807-nfkc-dunder-names.py:1`, `_＿x__`) and 1 translation-only (`:2`,
`__ｇｅｔａｔｔｒ__` at module level). Fixed by the normalised names.

## ruff-n811-constant-imported-as-non-constant

Baseline: fixtures 6 original-only, `N811-nfkc-expanding-single-char.py:1–5, 7`. Fixed by the normalised names and
Unicode case; the one-character test is `match(".", x)` on the normalised name (`Ǆ` is `DŽ`, two).

## ruff-n812-lowercase-imported-as-non-lowercase

Baseline: fixtures 5 original-only, `N812-normalized-alias-and-ignored-names.py:3, 5, 8, 14, 15`. Fixed by the
normalised names and Unicode case, except line 15.

Engine gap in the batch, line 15, `from mod import foo as ˤx`: NFKC gives `ʕx`. Unicode 17 made U+0295 `ʕ` a `Lo`
letter (no case); Ruff's Rust std has Unicode 17, so `ʕx` is not lowercase and Ruff reports it. The engine's
regexes use regex-syntax 0.8.12, whose tables are Unicode 16.0, where `ʕ` is `Ll` and in `\p{Lowercase}`, so
`pep8_lower("ʕx")` held.

Fixed since (rule fixes on #1120's line): core's numeric `@is_lowercase`, `@is_uppercase`, `@is_cased_lowercase`
and `@is_cased_uppercase` follow Rust std's `char` case properties, as Ruff's `ruff_python_stdlib::str` does, and
python-19's `pep8_lower`, `pep8_cased_lower`, `pep8_cased_upper` and `pep8_capwords` use them instead of regex
Unicode classes. The rule's `.test` checks NFKC aliases (`Lower`, U+02E4 to U+0295, Japanese, titlecase) against
Ruff 0.17.6. `translations:check`: N812 `same` (fixtures 33, samples-python 4 shared findings), and all 15 N8xx
rules in that run `same`.
