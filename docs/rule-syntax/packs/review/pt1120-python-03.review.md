# Review: batch pt1120-python-03 (F541, F601, F631, F632, F701, F722, F822, F823)

Ground truth: Ruff 0.17.6 (`ruff check --isolated --no-cache --select <CODE>`). Translation: `rerail-engine-1120
run-policies` (the fleet's #1120 engine, sha256 `aa28dc0b5bc6…`) on the branch's `.rerail` files at 391b6d12bb, both
on one fleet box in /tmp (pt-w01, pt-w07 and ts-prisma-b have no #1120 engine, so pt-w02 ran them). Ruff's columns are
given; run-policies gives lines only. Every counterexample below was run as written; a file name stands for a file
holding exactly the code under it.

On its corpora, the suite agrees with the author: `ptcheck --ref tbelanger/pt1120-python-03 -- --pack python --rules
<the eight>` gives 8 same (examples 4 files, fixtures 1963 files, samples-python 2313 files). The defects below are
outside those corpora.

| Rule | Verdict |
|---|---|
| ruff-f541-f-string-missing-placeholders | defects: files Ruff cannot parse; f-strings in string annotations |
| ruff-f601-multi-value-repeated-key-literal | defects: files Ruff cannot parse; key equality in string annotations |
| ruff-f631-assert-tuple | ok, but for files Ruff cannot parse |
| ruff-f632-is-literal | ok, but for files Ruff cannot parse; one finding per chain, not per link |
| ruff-f701-break-outside-loop | ok, but for files Ruff cannot parse |
| ruff-f722-forward-annotation-syntax-error | defects: starred, `;`, semantic syntax errors, `print >>x` |
| ruff-f822-undefined-export | defects: `__all__ = x = […]`, `(__all__) = […]`, `list(… + …)`, `global` + `del` |
| ruff-f823-undefined-local | defects: `for x in x`, `except … as x`, class bases, nested-def annotations |

## Shared cause: files Ruff cannot parse

Ruff runs its AST rules only on a file that parses. All eight rules are AST rules. A file with a parse error gets only
its `invalid-syntax` diagnostics. Semantic syntax errors do not stop the lint: `nonlocal q` at module level,
`f(a=1, a=2)` and `x = *a` still get F541. F601, F631, F632, F722, F822 and F823 never check for this. F541 checks for
any tree-sitter `ERROR` node in the document. F701 checks only for an `ERROR` around the `break`. Each rule reports in
both files below, except F541 (`zz_unclosed.py` only is right) and F701 (as noted):

`z.py` (a Python 2 file: tree-sitter reads `print_statement`, no `ERROR`)
```python
print "py2"
x = f"a"                                # F541 corpus
d = {1: 1, 1: 2}                        # F601 corpus
assert (x, "m")                         # F631 corpus
x is 1                                  # F632 corpus
a: "a b"                                # F722 corpus
__all__ = ["zz"]                        # F822 corpus
x = 0                                   # F823 corpus: with the next three lines
def f():
    print(x)
    x = 1
```
(each corpus's `z.py` is the `print "py2"` line and that rule's lines alone)

`zz_unclosed.py`: the same violation lines followed by `y = (`.

```text
Ruff:        z.py:1:7: invalid-syntax: Simple statements must be separated by newlines or semicolons
             zz_unclosed.py:3:1: invalid-syntax: unexpected EOF while parsing      (no F-code in any corpus)
Translation: z.py:2 F541, F601, F631, F632, F722, F822; z.py:4 F823
             zz_unclosed.py:1 F601, F631, F632, F701 (`break` at module level), F722, F822; zz_unclosed.py:3 F823
```

F541's and F701's `ERROR` test misses every Ruff parse error that tree-sitter-python accepts. With `x = f"a"` on the
next line, the translation reports F541 and Ruff reports only `invalid-syntax` for each of these first lines:
`del 1`, `f(**k, *a)`, `print "hi"`, `exec "code"`, `y = 0777`, `a, b += 1`, `f(x for x in y, 1)`,
`def g(a=1, b): pass`, `def g(*, **k): pass`, `g = lambda x=1, y: x`, `` y = `x` ``, `y = 1 <> 2`, and a dedent to a
column no block opened (`if True:\n    pass\n  bad = 1`, F541 before it). F701 the same: `print "x"\nbreak` and
`x = 0777\nbreak` give F701 at line 2 to the translation and only `invalid-syntax` to Ruff. Python 2 files are the
real-code case. The engine's `syntax_error` is CPython 3.13's verdict, which the F541 comment rejects for t-strings.
A fix needs Ruff's verdict: CPython's minus what Ruff parses (t-strings, PEP 758 `except A, B:`), or the cases above
listed. It belongs in one library relation that every Ruff AST rule reads, not in two rules.

## ruff-f541-f-string-missing-placeholders: defects

1. Files Ruff cannot parse: above.
2. Ruff parses string annotations and runs F541 in them, as it does F601 and F632. The translation checks F601 and
   F632 there (`ann_tree_node`) but not F541:

`a07_in_annotation.py`
```python
x: "f'abc'" = 1
```
`a26_cast.py`
```python
from typing import cast
y = cast("f'a'", 1)
z: "list[f'b']"
```
```text
Ruff:        a07_in_annotation.py:1:5 F541; a26_cast.py:2:11 F541; a26_cast.py:3:10 F541
Translation: (none)
```

Otherwise the same: adjacent pieces (`f"a" "b" F"c"` twice), nested format specs, `f"{{a}}"`, t-strings,
PEP 695 and PEP 758 files.

## ruff-f601-multi-value-repeated-key-literal: defects

1. Files Ruff cannot parse: above.
2. In a string annotation's tree, `ann_key` compares only str values and decimal integer text. Ruff compares those
   keys by value, as it does outside annotations. Missed:

`f601ann.py`
```python
a2: "{0x10: 1, 16: 2}"
a11: "{(1, 2): 1, (1, 2): 2}"
a12: "{-1: 1, -1: 2}"
a2: "{'a' 'b': 1, 'ab': 2}"
a3: "{1e2: 1, 100: 2}"
a5: "{0b1: 1, 1: 2}"
```
Ruff reports F601 on each line (`16`, `(1, 2)`, `-1`, `'ab'`, `100`, `1`); the translation reports none of them.
The rule's comment names `0x10`. The tuple, negative-number and concatenated-string keys are not named. Outside
annotations all 32 key forms I tried agree, including `True`/`1.0`/`1 + 0j`/`-0.0`/`0j`, repeats and `f"a"`.

## ruff-f631-assert-tuple: ok

No difference beyond files Ruff cannot parse. Agreed on 21 forms: `(x,)`, `((x, y))`, `(*a,)`, `()`, `(  # c\n)`,
`\` continuation, a comment inside the tuple, `assert(x, y)`, `(()), "m"`, `((),)`, lists, generators, comparisons.

## ruff-f632-is-literal: ok (one count difference)

Agreed on 29 forms, including the ones where Ruff reports nothing: `-1`, `+1`, `~1`, `(-1,)`, `f"a"`, `t"a"`,
`list()`, and in string annotations, where `x is -1` gets nothing. Two differences:

1. Files Ruff cannot parse: above.
2. Ruff reports once per offending link of a chain. The translation reports once per comparison and message:

`f632chain.py`
```python
1 is x is 2
x is 1 is y
x is not 1 is not 2
```
```text
Ruff:        1:1 F632, 1:1 F632, 2:1 F632, 2:1 F632, 3:1 F632, 3:1 F632
Translation: 1, 2, 3 (once each)
```
The suite compares sets, so it does not see this. A count-based consumer would.

## ruff-f701-break-outside-loop: ok

Agreed on module-level `break`, a `break` in a class or def inside a loop, in a loop's `else`, in an inner loop's
`else` inside an outer loop, in `finally`, `match` and `async for`, and in a PEP 758 `except A, B:` handler. The only
difference is files Ruff cannot parse (above): an `ERROR` elsewhere in the file, or a parse error tree-sitter accepts.

## ruff-f722-forward-annotation-syntax-error: defects

Agreed on 33 of 37 contexts (`cast`, `TypeVar` bound/constraints, `NewType`, `TypeAlias`, `type X =`, PEP 695
bounds, `Generic[...]`, `list["…"]` at runtime, `NamedTuple`/`TypedDict` fields, `typing.cast(typ=…)`, `Literal`,
`Annotated`, `@no_type_check`, nested strings). It also agreed on most string forms: indents, escapes, triple quotes,
comments, `:=`, `as`, `yield x`, statements, `0777`, backticks, `<>`. Differences:

1. Starred expressions: one `ann_text` is a module whose expression statement is a starred expression or a tuple
   with one. Ruff parses it as an expression and refuses it ("Starred expression cannot be used here").
2. A trailing `;`: tree-sitter takes `int;` as one statement. Ruff's expression parse refuses the token.
3. Semantic syntax errors: Ruff reports the annotation's semantic syntax errors as F722, one diagnostic each.
   `@parse` accepts the code.
4. `print >>x`: tree-sitter reads a Python 2 `print_statement` (not an expression statement), so the translation
   reports it. Ruff parses `print >> x`, a binary operation, and reports nothing.

`f722.py`
```python
a03: "*a"
a15: "*a, b"
a14: "*a,"
a53: "int;"
a08: "int; "
a15b: "int;\n"
a09: "await y"
a03b: "(await x)"
a16: "(yield)"
a02: "[(yield) for x in y]"
a24: "x if True else (yield)"
a01: "[x async for x in y]"
a06: "f(a=1, a=2)"
a18: "print >>x"
```
```text
Ruff:        lines 1-13 F722; lines 7 and 8 twice ("`await` outside of an asynchronous function", "`await` statement
             outside of a function"); line 14 nothing
Translation: line 14 only
```
(`lambda: (yield)` is not F722 to either.)

5. Files Ruff cannot parse: above.

## ruff-f822-undefined-export: defects

Agreed on 47 forms, among them: magic globals (`__path__`, `__qualname__`, `__module__`, `__dict__` reported),
builtins, `extend`/`append`/`remove` and sets not read, `except … as zz`, `type zz`, match captures, comprehension
walrus, `del` of an unbound name, `global zz` assigned or imported in a def, `global zz` at module level, bare
annotations, `__future__` names, bare tuples, `+` chains, `list([…])`, starred entries, f-string/bytes/t-string entries
not read, `__all__` in `if`/`try`/`with`, and in a def. Differences:

`g01_chain_first.py`
```python
__all__ = _x = ["zz"]
```
`g39_paren_target.py`
```python
(__all__) = ["zz"]
```
`g54_list_of_list.py`
```python
__all__ = list(("zz",) + ("yy",))
```
`g56_global_in_def_with_del.py`
```python
def f():
    global zz
    del zz
__all__ = ["zz"]
```
```text
Ruff:        g01_chain_first.py:1:17 F822 zz; g39_paren_target.py:1:14 F822 zz
Translation: g54_list_of_list.py:1 F822 yy, zz; g56_global_in_def_with_del.py:4 F822 zz
```

- `__all__ = _x = […]`: Ruff reads an `Assign` whose *first* target is `__all__`. `all_part` refuses any
  assignment whose right side is an assignment (`!kind(v, "assignment")`), so it drops this form along with
  `_x = __all__ = […]` (which Ruff does not read either; both agree there).
- `(__all__) = […]`: the target is `__all__` to Ruff. `all_part` wants `left` to be an `identifier`.
- `list(… + …)`: Ruff reads `list(…)`/`tuple(…)` only around a list or tuple display. `all_part` recurses from the
  call's argument into `+`.
- `global zz; del zz` in a def: Ruff takes the name as bound at module level. Exotic; listed for completeness.
- Files Ruff cannot parse: above.

## ruff-f823-undefined-local: defects

Agreed on 49 of 59 forms. Among them: rebinding after the read, `+=` twice, builtins, bare annotations before and
after, a class between the def and the module (reported by both), comprehension element vs. first iterable, lambdas,
decorators, defaults, walrus, match, `with x as x`, `x = x + 1`, `x: int = x`, class bodies, nested defs and
`global` elsewhere. Differences:

1. The iterable of a `for` is read before its target binds. `early_read` sees the target textually first:

```python
x = 0
def f():
    for x in x: pass            # h27; also `async for x in x` (h27b), `for x, y in x` (h27d)
```
```text
Ruff:        3:14 F823 (h27b 3:20, h27d 3:17)
Translation: (none)
```

2. `except E as x` as the function's only binding of x: the rule leaves it out (`form != "except"`, "Ruff does not
   report those"). Ruff 0.17.6 reports it:

```python
x = 0
def f():
    print(x)                    # h42: Ruff 3:11 F823
    try:
        pass
    except E as x:
        pass
```
The same holds with the read in the `try` body (h42d, Ruff 4:15) and after the handler (h42b,
`try: pass / except E as x: pass / print(x)`, Ruff 7:11). Translation: none of the three. With a later `x = 1` both
report (h43).

3. Reads that Ruff does not count, reported by the translation:

```python
x = 0
def f():
    class C(x): pass            # h35; also `class C(metaclass=x)` (h35b)
    x = 1
```
```python
x = 0
def f():
    def g() -> x: pass          # h23; also `def g(a: x)` (h23c)
    x = 1
```
```python
from __future__ import annotations
x = 0
def f():
    y: x = 1                    # h22b (without the import both report, h22)
    x = 2
```
```text
Ruff:        (none in any of the five)
Translation: line 3 (h35, h35b, h23, h23c), line 4 (h22b)
```
A class *decorator* read is F823 to both (h35c).

4. Files Ruff cannot parse: above.

## The author's result file and notes

`pt1120-python-03.result.json` lists all eight as `same` and has no `different` entry, so it has no `right` side to
check. The suite run above reproduces the eight `same`. It also reports, for each of the eight, "reports the same
findings as its original now; remove it from different". The branch's `python/equivalence.json` still lists all
eight under `different` with `"right": "unknown"` and the old counts. It does not gate (`equivalent: false`), but
those entries are stale.

Keys the notes cite, run through both tools on the recorded fixtures (non-noqa copies): they agree on F601
`UP031-annotation-operands-and-dicts.py:4`, F631 `F631-F634-continuation-before-test.py:2`, all 12 F632 keys of
`pyflakes/F632.py` including `:34` in the annotation, F701 `B012.py` (none), F722 `N805-annotation-with-equals.py:6`,
and F722 `UP037_0.py`/`UP037_2.pyi` (none). Every F822 name in `RUF022.py` and `PYI056.py` agrees, with
`PYI056.py:4-5` not read. F823 `G002-binding-forms.py` (none) and F541 `ISC_syntax_error.py` (only `invalid-syntax`
to Ruff, nothing to the translation) agree too.

The notes cover the findings above as follows:
- F541's note says Ruff reports nothing in a file it cannot parse for all eight rules. It adds the check to F541 only,
  since only F541 had such a key in the corpora. The other rules' gap and the `ERROR` test's misses are open (shared
  cause above).
- F601's note names the `0x10` gap and proposes guest `value` rows. Tuple, negative and concatenated keys are not named.
- F822's note excludes "a chained `x = __all__ = […]`". Right for that order, but `__all__ = x = […]` is read by Ruff.
- F823's note does not cover the `except` exclusion, which Ruff 0.17.6 contradicts.
