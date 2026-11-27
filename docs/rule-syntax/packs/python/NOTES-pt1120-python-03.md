# pt1120-python-03

Ground truth: Ruff 0.17.6 on a check box (`ruff check --select CODE --no-cache FILE`). Each rule was checked with the
translation suite on `tbelanger/pack-translations-1120` and its engine (`rerail-engine-1120`), and each differing key
was run through Ruff. Every rule stays per file: run with `RERAIL_SHOW_CROSSINGS=1`, none reports a crossing.

New library: `lib/python/pt1120-python-03.rerail`, `ann_tree_node(n)` (a node of a string annotation's parsed tree,
the tree asked for), read by F601 and F632.

## ruff-f541-f-string-missing-placeholders

Followed Ruff and the JSON. Ruff reports nothing in a file it cannot parse: `x = f"a"` then `y = (` gives only the
syntax error. The translation reported F541 at lines 9-11 of `ISC_syntax_error.py` and line 7 of
`ISC_syntax_error_2.py` (unterminated f-strings). It now skips a document with an ERROR node.

It does not read `syntax_error`: that is CPython 3.13's verdict, which refuses t-strings (`t"x"`). Ruff reads them,
and a policy that reads `syntax_error` loses the files it refuses. Reading it lost 55 keys in 11 fixture files with
t-strings (`W605_1.py`, `ISC.py`, `RUF037.py`, …). Ruff 0.17.6 also reports no F601, F631, F632, F701, F722, F822 or
F823 in a file with a syntax error. Only F541 had such a key in the corpora, so only F541 checks it.

## ruff-f601-multi-value-repeated-key-literal

Followed Ruff and the JSON. Ruff runs its rules inside string annotations too: in `j: "'%(a)s' % {'a': 1, 'a': 2}"`
(`UP031-annotation-operands-and-dicts.py:4`) the second `'a'` is F601. The rule asks for each string annotation's
tree (`ann_tree_node`).

Engine gap: in a guest tree, literals have no `value` rows, and `literal_key` reads `value` for a str and an
integer. Inside annotations `ann_key` reads a str by `str` and a decimal integer by its text, so `"{0x10: 1, 16: 2}"`
in an annotation is missed. Wanted: `value(n, v)` for guest nodes, as for a file's nodes.

## ruff-f631-assert-tuple

Followed Ruff and the JSON. In `assert \` followed by `(1, 2)` on the next line
(`F631-F634-continuation-before-test.py:2`), child 0 of the assert is the `line_continuation` extra, so the test was
never read. The test is now the first child that is not a continuation or a comment.

## ruff-f632-is-literal

Followed Ruff and the JSON. `variable: "123 is not y"` (`F632.py:34`) is F632: the comparison inside the string
annotation is checked through `ann_tree_node`, as for F601. `cast("1 is y", 2)` is F632 for Ruff and here.

## ruff-f701-break-outside-loop

Followed Ruff and the JSON. `B012.py:118`: tree-sitter cannot parse `match *0, 1, *2:` in a `while` body, so the
break sits in ERROR nodes and its loop's body is gone from the tree. Ruff parses it and reports nothing. A break
inside an ERROR node is no longer reported.

## ruff-f722-forward-annotation-syntax-error

Followed Ruff. Rewritten: the old rule looked for ERROR nodes in `parsed(s, "annotation", r)`, but the engine holds
back a guest that does not parse (decision 0055's `skip`), so it found nothing (37 keys missing). The engine's guest
is also not Ruff's parse. It wraps every string as `( … )`, which:

- accepts what Ruff refuses: `w: "\tNoReturn"` and `r: "  [1] + foo"` (a leading indent), `'''list[int]\\` + newline
  + `'''` (a trailing continuation), and `"''"` nested in an annotation, which it reads as `()`;
- refuses what Ruff accepts: `def f() -> "Literal[0]#"` and `"A\n#"`, where the comment swallows the `)`
  (`UP037_0.py:112, 115, 125`, `UP037_2.pyi:3, 17, 22, 41`).

The rule now parses each string annotation as Ruff does (`parse_type_annotation`). A single triple-quoted string
whose source contents are its value is parsed in parentheses, with the `)` on a line of its own. Any other string is
parsed from its value as it is. Both go through `@parse("python", x)`, and the rule requires a module of one
expression statement that is not an assignment. Each annotation's guest is asked for (`sub_tape`), so its nested
strings get `string_annotation` rows: `def f(a: "'' and 'b'")` is F722 at the empty `''`.

The rule is `same` on fixtures and samples. Three engine facts it works around:

- It does not read the value document's `syntax_error`: a text that does not parse has no `doc_root`, and with the
  read, `SIM223.py:159` was lost: the file's `t""` makes it a file CPython 3.13 refuses (see F541).
- Equal texts share a tape, so the tape of `@parse("python", "(x)")` also has the engine's annotation guest of `"x"`
  as a document. `one_expression` reads only a `module` root.
- A relation over the value document's nodes written apart from the rule that makes the tape
  (`statement_form(e) :- child(e, a, _), kind(a, "assignment")`) had no rows, and `x: "a=b"` was missed
  (`N805-annotation-with-equals.py:6`, `N804-…py:68`). The test is inlined in the rule that reads `tape_of`.

## ruff-f822-undefined-export

Followed Ruff 0.17.6 where it differs from the JSON and from the old translation (6 original-only, 33
translation-only before):

- `__all__.extend(...)` and `.append(...)` are not definitions of `__all__` for F822 (`PYI056.py:4-5`, `RUF022.py:24-26`).
  The library's `all_entry` (python/pyflakes) reads them for F405 too, so the rule has its own `export`.
- A bare tuple (`__all__ += "foo3", "foo2", "foo1"`, `RUF022.py:19`), parenthesized entries (`RUF022.py:23, 207,
  214`) and adjacent strings joined (`"don't" "care" "about"` names `don'tcareabout`, `RUF022.py:219, 359`) are read.
  A set, an f-string, bytes, a chained `x = __all__ = […]` and an `__all__` in a def are not.
- A bare annotation `c: int` binds c in a `.py` too (`RUF068-…py:20-21`), and `from __future__ import x` binds x
  (`UP010_1.py:18`).

Counterexample checked: a file with 30 `__all__` forms gives the same names, line by line, as Ruff's messages.

## ruff-f823-undefined-local

Followed Ruff 0.17.6 against the old notes (written for 0.15.7). A read after a bare annotation of the name is not
F823: `def f(): log5: int; log5.info(...)` with a module-level `log5` (`G002-binding-forms.py:29`). A read before
it still is: `print(y); y: int`.

## Proposed engine changes

- `value(n, v)` for the literals of a guest tree (F601 above).
- The annotation guest (`corpus::guests`) closes its `( … )` wrapper on a new line, and parses as Ruff does: in
  parentheses only for a triple-quoted string whose source is its value. Then `parsed(s, "annotation", r)` would be
  Ruff's verdict, and F722 could read it instead of `@parse`.
- A Python file CPython 3.13 refuses but Ruff 0.17 parses (t-strings, PEP 750) loses every finding of a policy that
  reads `syntax_error`. A verdict per Python version, or Ruff's own, would let a rule ask whether Ruff parses the file.
