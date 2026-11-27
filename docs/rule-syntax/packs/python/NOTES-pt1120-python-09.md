# pt1120-python-09: TRY002, TRY003, TRY401, B030, B905, INT001, INT002

All seven rules are `same` as their JSON originals on the examples, fixtures and samples-python (ptcheck on
`tbelanger/pack-translations-1120` with this batch's files, engine `rerail-engine-1120`). Every difference the suite
found before was the translation's error: Ruff 0.17.6 (`ruff check --select <CODE> --no-cache`) agrees with the
original on each sample key. TRY002/TRY003 and INT001/INT002 each share a fix, so the batch has a new library,
`lib/python/raise-and-gettext.rerail` (`p09_*`). Everything is per file, keyed by node; with
`RERAIL_STRICT_CROSSINGS`, `RERAIL_STRICT_NODES` and `RERAIL_STRICT_POSITIONS` the rules run.

## TRY002, TRY003: a raise continued with a backslash (`raise_value`)

tree-sitter keeps `\` as a `line_continuation` child of the raise statement, so in `raise \` / `    E("m")` the first
child is the continuation. exceptions' `exc_raise` reads child 0 and finds no call. Counterexample
`EM101-backslash-continuation-after-raise.py` (and its EM102 twin): Ruff TRY003 at lines 3, 8, 18; the old
translation nothing. `raise_value` takes the child that is no continuation, comment or cause; `raised_class` is
`exc_class` over it.

**Library change proposed:** `exc_raise` (python/exceptions) should skip `line_continuation` children, and
`misc2-messages`' `msg_raised_call` with it (python-30's `em_ruff_raised_call` works around the same gap); then
`raise_value` and `raised_class` go.

## TRY002: the name where Ruff reads it

`exc_qual` is names' `sym_of`, which reads every binding of the first scope that binds the name. Ruff resolves in
visit order. `TRY002-later-local-binding.py`: `def f(): raise Exception("a"); Exception = ValueError` is TRY002 at line
2 (nothing is bound yet when the raise is visited); `def h(): raise BaseException("c"); import BaseException` is
TRY002 at 15. The old translation reported neither. `def g(): def inner(): raise Exception("b")` with
`Exception = ValueError` after it in g is not reported: inner's body is visited after g's. The rule reads
upgrade's `sym_at`, which models that order.

## TRY401: a lone generator argument

`logger.exception(str(x) for x in e)`: tree-sitter puts the generator in place of the argument list, so `exc_item`
has no row. Ruff's `args[0]` is the generator and its names are visited (`TRY401-bare-generator-argument.py`, lines
10–12, at `e`). `positional` adds the generator.

## B030: an unparenthesized handler list

`except (IOError, KeyError), e:` (Python 2 syntax; PEP 758's `except A, B:` parses the same): Ruff reads one tuple
whose elements are `(IOError, KeyError)` and `e`, and an element tuple is not taken apart: B030 at the inner tuple
(`UP024-string-annotation-and-comma-except.py:7`). tree-sitter gives the except clause one `value` field per element;
the old translation read each as a handler type of its own and took the tuple apart. `bare_list(x)` (two `value`
fields) makes each a part, with no tuple taken apart. `except KeyError, e:` stays silent (two names).

## B905: `[*zip(a, b)]` and a continued `repeat(1, \ None)`

- tree-sitter parses a starred call first in a list display as a call whose callee is `*zip`: `[*zip(a, b)][0]`
  (`B905-trivia-and-starred-call.py:18`, `RUF015.py:58`) is B905 in Ruff, missed by the old translation.
  `zip_call` adds a call whose callee is a `list_splat` of `builtins.zip`. `[*zip(a, b, strict=True)]` and
  `[*zip(a)]` stay silent.
- `zip(a, itertools.repeat(1, \` / `None))` (line 12): bugbear's `bb_pos` counts the continuation, so `None` was
  argument 2 and the repeat not infinite: a false B905. The rule reads python/ruff's `ruff_arg`, which skips
  continuations. Not followed: Ruff's `args[1]` of `repeat(*xs, None)` is the None (infinite); `ruff_arg` has no row
  after a `*xs`, so that call would be reported. No corpus has one.

**Library change proposed:** `bb_pos` and `bb_arg` (python/bugbear) should skip `line_continuation` as python/ruff's
`call_item` does.

## INT001, INT002: translation calls inside string type definitions (`gettext_arg`)

Ruff checks the code it parses from string annotations, `cast("…", x)`'s type and TypeVar bounds
(`INT001-string-annotations.py`: lines 5, 6, 7, 11, 12; `INT002-string-annotations.py`: 4, 5, 6, 11). misc2-messages'
`msg_arg` counts positional arguments in an aggregate, which does not see a guest tree's nodes, so it has no row
there. `gettext_arg` adds python-03's `tree_call_arg` for a `type_str_node` call.

Literal strings: a guest tree's rows are reached only from its string, through `parsed`. names' `literal_str` starts
from `kind(l, "subscript")`, so it finds no subscript of a guest tree, and the string in
`x4: "Literal['_(f\"{a}\")']"` passes `ann_str` and is parsed as a type. Ruff does not (a Literal's string is a
value): line 8 of the INT001 probe, line 9 of the INT002 one, were false findings of the first fix. `guest_in_literal`
finds such subscripts from the guest's nodes (`type_str_node`) and leaves their strings' trees out.

**Library change proposed:** names' `literal_str` (python/scopes) should be seeded from the guest's nodes too (a
clause over `parsed_in`, as `guest_in_literal`); then the nested strings of a Literal are not parsed for any rule, and
`guest_in_literal` goes. `msg_arg` could fall back to `tree_call_arg` on a `type_str_node`, and `gettext_arg` go.

## Engine gaps

None needed: every fix is expressible. The guest-row reach above (a relation not seeded from `parsed` sees no guest
nodes, aggregates see none either) is the engine's documented behaviour (decision 0058 §1), not a gap of this batch.
