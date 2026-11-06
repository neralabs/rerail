# py-f-e: 43 Ruff rules (pyflakes F, pycodestyle E4/E7), hand-written

Every rule follows Ruff 0.16.9, the version the recording was made with. Where Ruff and the JSON differ, the rule
follows Ruff (D1-D6 below). Ruff 0.15.7 is installed here (`/opt/homebrew/bin/ruff`). I ran it on the
counterexamples; where it is older than the recording, the notes say so.

The rules share two new libraries:

| file | lines | what |
|---|---|---|
| `lib/python/pyflakes.rr` | 291 | bare handlers; where code runs (`run_scope`: function, lambda, class, module, comprehension, generator, as CPython compiles it); loop bodies; Ruff's literal tests; full builtins; unbound names and star imports; `__all__` entries; statement-level noqa; dict-key values (`1 == 1.0 == True == 1+0j`) |
| `lib/python/pyflakes-format.rr` | 182 | printf-style and str.format strings parsed as CPython parses them (one regex step per token over the string's value); format-call arguments; operands whose type is known not to be a tuple |

The 43 rules have 987 lines in total, comments and directives included. They range from 15 to 60 lines (E721 carries the JSON's 92 class names).

## Scores

`rrlab score python <id> --all`. Ruff's fixtures sit in per-plugin directories (`pyflakes/`, `pycodestyle/`),
not one per code, so a rule's "own fixtures" set is empty. `--all` scores the rule's code on all 462 fixture
files. Where a row has FP or FN, they are the same at (path, line) and at (path, line, column).

| rule | lines | TP / FP / FN | notes |
|---|---|---|---|
| ruff-f402-import-shadowed-by-loop-var | 44 | 3 / 0 / 0 | D4 |
| ruff-f403-undefined-local-with-import-star | 16 | 18 / 0 / 0 | |
| ruff-f405-undefined-local-with-import-star-usage | 21 | 4 / 0 / 0 | L2 |
| ruff-f406-undefined-local-with-nested-import-star-usage | 17 | 4 / 0 / 0 | |
| ruff-f407-future-feature-not-defined | 28 | 4 / 0 / 0 | |
| ruff-f501-percent-format-invalid-format | 19 | 3 / 0 / 0 | L3 |
| ruff-f502-percent-format-expected-mapping | 16 | 14 / 0 / 0 | |
| ruff-f503-percent-format-expected-sequence | 16 | 6 / 0 / 0 | |
| ruff-f504-percent-format-extra-named-arguments | 17 | 8 / 1 / 1 | H3 |
| ruff-f505-percent-format-missing-argument | 17 | 3 / 0 / 0 | |
| ruff-f506-percent-format-mixed-positional-and-named | 16 | 9 / 0 / 0 | |
| ruff-f507-percent-format-positional-count-mismatch | 29 | 43 / 0 / 0 | D1 |
| ruff-f508-percent-format-star-requires-sequence | 16 | 3 / 0 / 0 | |
| ruff-f509-percent-format-unsupported-format-character | 17 | 3 / 0 / 0 | L3 |
| ruff-f521-string-dot-format-invalid-format | 19 | 9 / 0 / 0 | L3 |
| ruff-f522-string-dot-format-extra-named-arguments | 17 | 9 / 0 / 0 | |
| ruff-f523-string-dot-format-extra-positional-arguments | 19 | 28 / 0 / 0 | |
| ruff-f524-string-dot-format-missing-arguments | 22 | 11 / 0 / 0 | |
| ruff-f525-string-dot-format-mixing-automatic | 15 | 4 / 0 / 0 | |
| ruff-f541-f-string-missing-placeholders | 23 | 44 / 2 / 0 | H2 |
| ruff-f601-multi-value-repeated-key-literal | 21 | 40 / 0 / 0 | D2, L4 |
| ruff-f602-multi-value-repeated-key-variable | 20 | 24 / 0 / 0 | |
| ruff-f622-multiple-starred-expressions | 25 | 3 / 0 / 0 | |
| ruff-f631-assert-tuple | 17 | 4 / 0 / 0 | |
| ruff-f632-is-literal | 27 | 38 / 0 / 1 | D3, H4 |
| ruff-f633-invalid-print-syntax | 30 | 3 / 0 / 0 | D5 |
| ruff-f634-if-tuple | 18 | 5 / 0 / 0 | |
| ruff-f701-break-outside-loop | 17 | 7 / 1 / 0 | H5 |
| ruff-f702-continue-outside-loop | 16 | 6 / 0 / 0 | |
| ruff-f704-yield-outside-function | 26 | 26 / 0 / 0 | |
| ruff-f706-return-outside-function | 15 | 8 / 0 / 0 | |
| ruff-f707-default-except-not-last | 18 | 5 / 0 / 0 | |
| ruff-f722-forward-annotation-syntax-error | 53 | 0 / 0 / 13 | H4, L1 (partial) |
| ruff-f822-undefined-export | 32 | 9 / 3 / 0 | H1, D6 |
| ruff-f823-undefined-local | 45 | 7 / 0 / 0 | D6 |
| ruff-f842-unused-annotation | 28 | 5 / 0 / 0 | D6 |
| ruff-f901-raise-not-implemented | 21 | 6 / 0 / 0 | |
| ruff-e401-multiple-imports-on-one-line | 16 | n/a | H6 |
| ruff-e711-none-comparison | 25 | 14 / 0 / 0 | |
| ruff-e721-type-comparison | 60 | 18 / 0 / 0 | |
| ruff-e722-bare-except | 21 | 26 / 0 / 0 | |
| ruff-e742-ambiguous-class-name | 16 | 5 / 0 / 0 | |
| ruff-e743-ambiguous-function-name | 16 | 5 / 0 / 0 | |

Summary: 37 rules are perfect, and E401 has no recording. Five more miss only on harness artefacts (H1-H5).
F722 cannot be scored: rrlab does not model `parsed`.

## Deviations from the JSON (the rule follows Ruff)

- **D1, F507.** The JSON checks only a tuple operand. Ruff 0.16.9 also counts one value for any operand whose
  type is known not to be a tuple (`known_non_tuple`). It also reports a string with no placeholder at all for
  any operand but a tuple. Counterexamples recorded by Ruff and missed by the JSON: `'%s %s' % 42`,
  `'%s %s' % [*a]`, `'%s %s' % (not x)`, `'hello' % banana` (F50x.py:22-66), and `"%s %s" % {"bob": "bob"}`
  (F503.py:17). Ruff 0.15.7 predates this and reports none of them.
- **D2, F601.** The JSON compares keys by structure hash. Ruff compares them by value:
  `{1: "a", True: "b"}` is F601 at True (Ruff 0.15.7 agrees). So are `{1.0: 2, 1: 0}`, `{False: 1, -0: 2}`
  and `{-1: 1, -1 + 0j: 2}`. `{9007199254740993: 1, 9007199254740993 + 0j: 2}` is not, because a complex's parts
  are floats (F601.py:63-82). Every repeat is reported, whatever its value, as in the JSON.
- **D3, F632.** The JSON treats an f-string without a placeholder as a literal. Ruff's `is_constant` does not:
  `x is f"abc"` is not F632 (Ruff 0.15.7 agrees). `-1` is a unary operation in both and is not a literal.
- **D4, F402.** The JSON's `binds … redefines import` stays inside one scope. Ruff's shadowed binding also looks
  in the enclosing def or module. `import sys` then `def main(): for sys in …` is F402 (F823.py:64). It also
  requires the same branch: `if c: import re` / `else: for re in …` is not F402 (Ruff 0.15.7 agrees).
- **D5, F633.** The JSON tests "no binding of print anywhere in the module". Ruff resolves names in visit
  order. In `def p(): print >> sys.stderr; print = 1` the later binding does not count, so this is F633 (Ruff
  0.15.7 agrees). The rule looks only at earlier bindings in the own scope, and at any binding in an enclosing
  scope.
- **D6, F822, F823, F842.** The JSON's `resolves {unresolved | beforeLocal}` and `binds {annotation, latest,
  used}` are engine terms, with no Datalog form. The rules write Ruff's logic out:
  - **F823.** The first read in a function that comes before every local binding of the name, while an
    enclosing def or the module binds it. `x += 1` reads first. A comprehension's reads are its own scope's.
  - **F823.** Two refinements, checked against Ruff 0.15.7. A builtin counts as an outer binding, so
    `print(len); len = 1` is F823. A function whose only binding of x is `except E as x` is not reported. A bare
    annotation binds nothing: `print(x); x: int` is F823, and so is `x: int; print(x)`.
  - **F842.** Ruff keeps a name's first binding. The annotation is reported only if the function has no other
    binding of x, before or after, and it is the first bare annotation of x. A read or `del x` below the
    annotation uses it, nested functions included. A read above it does not. Ruff 0.15.7 agrees on these:
    - `x: int; print(x)` is not F842;
    - `w2: int; w2: str` gives one F842, at the first;
    - `x = 1; x: int` is not F842;
    - `x: int; del x` is not F842;
    - `print(x); x: int` is F842.
  - **F822.** In a `.pyi`, a bare `b: int` binds b. A module-level `del x` keeps x defined (Ruff 0.15.7).
    Ruff 0.15.7 also accepts a bare `b: int` in a `.py`. The 0.16.9 fixture F822_0.pyi says that is a binding
    "in a `.pyi` stub file, not in a `.py` runtime file", so the rule keeps the stub-only form.

## Review fixes (after Ruff 0.15.7 probes)

### Changes to lib/python/names.rr

All the changes are additive. The interface relations keep their names and arities. `scope`, `scope_node` and
`qualname` are unchanged.

- **Comprehension scopes.**
  - New kind sets `comp_scope`, `lscope` (scope plus comprehensions) and `lfscope` (fscope plus
    comprehensions).
  - New `first_for(c, fc)`: a comprehension's first `for`, found without assuming it is child 1, since a
    comment can come first.
  - New `tscope(t, s)`: a `for` target in a comprehension binds in the comprehension. A walrus binds in the
    nearest real scope.
  - `rotgt` and `binding` now use `tscope`. Before, they used `nearest(t, s, scope)`.
  - `walk` starts at the new `wstart(n, s)`: the nearest scope or comprehension. For a comprehension's
    first iterable, it starts at the scope around the comprehension.
  - `walk` now steps through `lfscope`.
  - `site_scope` gains the kinds "comprehension" and "generator".
- **Match captures.** `tgt` gains these forms: `case x`, `[x, *rest]`, `Point(x=px)`, `{**kw}`, `A() as a`
  and `q | r`. The new `capture_parent` lists where a capture can appear. `nameref` no longer counts the
  attribute key of a keyword pattern (the `x` in `Point(x=px)`), through the new `kwpat_key`.
- **PEP 695.**
  - `type TA = ...` and `type A[T] = ...` bind the alias name. The binding form is "type", through new
    `target_stmt` and `form_of_kind` rows.
  - New `tp_name(o, x)`, with `tp_list` and `tp_decl`: the type parameters of a def, class or alias. These
    could not reuse ast.rr's `type_param`, because ast.rr includes names.rr.
  - New `tp_ref(n, x)`: a read of T inside the definition that declares it. Such a read is not looked up
    (`walk` skips it), so it is never a miss, in methods included.
- **Class bodies** bind `__qualname__` and `__module__`. This adds a `bound_in` rule.

### Fixes in this batch's files

- **F823.** Comprehension targets no longer count as the def's bindings. Builtins count as outer bindings.
  `except E as x` alone is not reported. Bare annotations are handled as described in D6.
- **F722.** The new `not_a_type` skips `Literal[...]` arguments, `Annotated[T, ...]` metadata and jaxtyping
  shapes. It uses `sub_arg`, which covers both a subscript and a tree-sitter `generic_type`.
- **run_scope / first_iter** use `first_for`. This fixes the false F704 when a comment comes before the
  `for`.
- **E721.** `builtin_class` is now the JSON's list of 92 names, copied verbatim. The regex is gone, and
  `Exception` and `Warning` are back.
- **F842.** Ruff keeps the first binding (see D6). `del` counts as a use.
- **F405 / `unbound_ref`.**
  - `unbound_ref` follows Ruff's order (L2).
  - `x += 1` reads x (`load_ref`).
  - A name annotated on the lookup chain is F821 for Ruff, so it is not F405.
  - The names in `from __future__ import ...` are not reads.
- **F523.** The new `arg_pos` counts positions over all arguments. A `*args` takes one position and is never
  reported itself, so `"{} {}".format(1, *a, 2)` is F523. `call_pos` keeps its "before any `*args`" meaning,
  which F524 and UP030 use.
- **F601.** f-string keys compare by their text.
- **F402.** `from __future__ import annotations` then `for annotations in x` is F402. With comprehension
  scopes, `[b for b in x]` no longer hides an import from a later loop in a def.
- **F822.** A module-level `del x` keeps x defined.
- **Idiom.**
  - `pct_error` uses `pct_stuck` and `pct_badchar`, so its regex is written once.
  - The dict-display tests are now library relations: `dict_str_key`, `dict_spread`, `open_dict`,
    `sequence_display` and `mapping_display`.
  - `positional` is renamed `fmt_positional`, after a library clash.
  - Stale comments on `brace_nested`, `call_pos` and F523 are fixed.

Against Ruff 0.15.7 on the review's probe files, the only differences left are these:
- the `__all__` extend/append forms (kept, L2);
- the bare `b: int` in a `.py` (D6);
- `%(a(b))s` (L3);
- keys.py's `1e3`, a nested tuple and `\N{...}` (L4);
- L5 and L6;
- the unary-operator keys, where 0.15.7 is older than the recording (D2).

## Explained misses

- **H1, F822 (3 FP).** `pyflakes/__init__.py`. The rule's `.scope exclude "**/__init__.py"` (from the JSON, as in
  Ruff) is not modelled by rrlab.
- **H2, F541 (2 FP; fixed in rrlab 08d977aa5).** rrlab applied `# noqa` only on a finding's first line. Ruff also
  applies it on the last line of a multi-line string (W605_1.py:39), and honours a file-level `# ruff: noqa: F541`
  (RUF100_7.py:7). rrlab now does both; F541 is 22 / 0 / 0 (python merge rescore).
- **H3, F504 (1 FP + 1 FN, same finding).** F504.py contains a lone `\r`. Ruff counts it as a line break, so the
  finding is at line 20. tree-sitter, and so rrlab, puts it at line 19. core.rr's `src_line` splits Python as
  `str.splitlines()` does, so the engine's `line` should agree with Ruff.
- **H4, F632 (1 FN), F722 (13 FN).** Findings inside string annotations (`variable: "123 is not y"`, F632.py:34,
  and every F722). They need `parsed`, which rrlab leaves empty. Both rules read the parsed tree through the
  ordinary tree relations, so they report there once `parsed` has rows.
- **H5, F701 (1 FP).** flake8_bugbear/B012.py:118. tree-sitter misparses `match *0, 1, *2:` (an ERROR node), so
  the `break` ends up outside its `while`. The engine withholds files with parse errors.
- **H6, E401.** The recording has no E401 findings, and the Data-layer checkout's conformance.json does not list
  the rule. On pycodestyle/E40.py the rule gives the same 9 findings as Ruff 0.15.7, at the same columns.

## Limits (real, not harness)

- **L1, F722.** Written: the syntax-error clause, with the no_type_check and jaxtyping exclusions. Not written: the
  JSON's two other clauses, semantic errors and `await` inside a string annotation. They need scopes over the
  parsed tree, and `run_scope` does not reach into it. The verdict "does not parse" is read as "the parsed tree
  has an ERROR node". That is tree-sitter's verdict, not CPython's: see C3.
- **L2, F405.** `unbound_ref` now resolves in Ruff's order. A read that runs as it is read comes before
  every binding of the name in its scope, and no outer scope binds the name. That covers module and class bodies,
  and the comprehensions in them. Such a read is unbound: `print(later); later = 1` is F405 with a star import.
  Function bodies run last, so they see the whole module. Class bodies are approximate: a read in a class body
  before the class binds x does not fall back to the module. Remaining gaps:
  - F405 follows `__all__.extend([...])` and `.append(...)`. Ruff 0.15.7 ignores both for F405 and F822.
    No 0.16.9 fixture covers them, so they are kept and unverified.
  - `from __future__ import annotations` binds nothing in python/names, so a later read of `annotations` is
    reported. F402 handles the future name itself.
- **L3, format parsers.** `%(key)` does not nest parentheses, although CPython allows `%(a(b))s`. `%5%` counts as
  an unsupported character. In str.format, `[` inside a field name does not protect `!` or `:`. None of these
  occurs in the fixtures.
- **L4, F601.** A float in exponent form (`1e3`) keeps its text, so it does not equal `1000`. Tuple keys compare
  one level deep: a recursive form would put a count over `elem_key` in a cycle. `"\N{BULLET}"` is not decoded.
  f-string keys compare by their source text, so `{f"a": 1, f"a": 2}` is F601. A unary `+1` equals `1`
  here. Ruff 0.15.7 disagrees, and no 0.16.9 fixture covers it.
- **L5, grammar.** tree-sitter parses `print >> f` at statement level as a Python 2 `print_statement`, so
  `print` is a keyword there and not a name. `def f(): print >> sys.stderr; print = 1` is F823 for Ruff, but
  not here. F633 reads the statement form directly.
- **L6, F823.** `x: int; x += 1` in a def, with a module-level x, is F823 here. Ruff 0.15.7 says nothing.
  `nonlocal x; x: int` at module-level def scope is F842 for Ruff, but not here. Both are near-syntax errors.

## Cost

The format parsers run one anchored regex per token, on the remainder of the string. Each remainder is a
distinct interned string, tested once per run, as for every text test. A string with k placeholders costs
k tests over O(length) text. Format strings are short, but a 10 KB template with 500 placeholders would cost
about 5 MB of scanning. A native tokenizer (C4) would make this linear.

## Library and core change requests

- **C1, python/names: complete `builtin`.** The `builtin` table lacks about 45 names: `None`, `True`,
  `ExceptionGroup`, `BlockingIOError`, `SyntaxError`, `exit`, `__builtins__`, …. As a result
  `builtin_ref`/`sym_of` miss them (E721 needs `BlockingIOError`, F822 needs `__builtins__`).
  `lib/python/pyflakes.rr` carries the rest as `builtin_more`. Proposal: add those rows (the `builtin_more`
  list, verbatim) to `builtin` in names.rr, and make `builtin_name` an alias, or drop it.
- **C2, python/names: export `bound_in` and `walk`, and add visit-order lookup.** F402, F405, F633, F822,
  F823 and F842 use `bound_in(s, x)`, `walk(n, x, s)` and `miss(n, x)`. Those are "layers 1-6 (internal)", so
  they should be interface. pyflakes.rr's `unbound_ref` now does a visit-order lookup for module and class
  bodies. As a names.rr relation, F633 and F821 could share it. Also proposed:

  ```
  // bound_before(n, x, s): scope s has a binding of x before n in visit order. Ruff resolves a read in its own
  // scope sequentially; enclosing function scopes are complete (deferred). Used by F405, F633, F821.
  .decl bound_before(n: node, x: symbol, s: node)
  ```
- **C3, core: a parse verdict for embedded code.** `parsed(s, "annotation", r)` gives a tree. F722 needs
  CPython's verdict, as `syntax_error` gives for files. Proposal:

  ```
  // parse_error(s, "annotation", msg): string s's contents do not parse as the grammar, by the same native parser
  // that gives syntax_error (Ruff's, for Python). One row per string asked about; none when it parses.
  .decl parse_error(s: node, grammar: symbol, msg: symbol)
  ```
- **C4, core (optional): format-string tokens.** If format strings turn out costly, add a native
  `format_part(s, style, i, kind, name, spec)` ("percent" or "brace"), the way `str_part` decodes literals. The
  Datalog parser in pyflakes-format.rr is exact for the fixtures, and could stay as the reference.
- **C5, suppression on a parent statement.** Ruff lets the first line of a finding's statement silence it for
  F822 (`__all__ = [  # noqa: F822` covers each name below). `suppress` checks the finding's own line. F822 does
  this in Datalog (pyflakes.rr's `noqa`/`noqa_ask`). A directive form would keep it out of rules:
  `suppress ruff "F822" at statement`.
- **rrlab (harness):**
  - model `.scope exclude`;
  - noqa on a multi-line string's last line, and `# ruff: noqa: CODES` file-level directives;
  - count lone `\r` as a line break in Python files;
  - `lower.py`: two aggregates in one body that share a variable name inside their braces (`count : { child(e,
    s, _) … }` and `join … : { child(e, s, i) … }`) are lowered with that variable as a shared group key, and
    Soufflé rejects the program. I renamed the variables; the lowering should rename them itself.
