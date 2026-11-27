# pt1120-python-04: UP008, UP034, UP035, UP036, UP039, UP041, UP043

Five rules are `same` as their JSON originals on the examples, fixtures and samples-python (ptcheck on
`tbelanger/pack-translations-1120` with this batch's files, engine `rerail-engine-1120`). UP035 and UP036 are
`different`, the translation right: Ruff reports the keys the original leaves out. Ground truth is Ruff 0.17.6
(`ruff check --select <CODE> --no-cache --target-version py310`; the rules' default target is 3.10, Ruff's own
default here is 3.11). Every rule that changed runs with `RERAIL_STRICT_CROSSINGS`, `RERAIL_STRICT_NODES` and
`RERAIL_STRICT_POSITIONS`; all its relations are per file, keyed by node. No new library file: no new concept is
shared by two of these rules.

## UP008 (super call with parameters): same

The translation missed and added one key each in `probes/UP008-deep-chain-and-parens.py`:

- Line 8, `super(((A.B.C.D)), (self)).h()`: Ruff reports (its AST has no parentheses). `pos_arg` now takes each
  argument with its parentheses off, and super's function too (`(super)(C, self)`, `super_call`).
- Line 15, `nonlocal C` in the method, then `super(C, self)`: Ruff does not report: the `nonlocal` is a binding of its
  own in the method, not the class. `redeclared(h, x)`: a scope the name's lookup passes (scopes' `walk`) declares
  it `nonlocal` or `global`.

Over UP008.py and the probe, the translation's lines are Ruff's (32 and 2).

## UP034 (extraneous parentheses): same

The old translation matched a pair of tree nodes; Ruff scans tokens. Ruff only (16 keys): a comment or line
continuation between the parentheses (`set( # c\n(…))`, PERF401.py:204, RUF010.py:88 in an f-string, RUF058_0.py:32),
a pattern's parentheses (`((x)) = foo()` F841_3.py:172, `for ((i)) in items`, `case Point(((1))):`). Translation
only (3 keys): `((lambda a=((1)), b=2: 0))` and `((v for ((a)), b in c))` (Ruff stops at the `,` directly inside
the inner pair, and the tokens it read before it start nothing), and `C403.py:31`, an inner double of the one Ruff
reports at line 30.

The rule now follows Ruff's scan (`extraneous_parentheses.rs`): a `(` is named by the first node after it, comments
skipped (`up34_lead`); an attempt at a `(` reads the inner pair (`up34_owner`, the node owning the next `(`) to its
`)`, or to the first `,` or `yield` directly inside it (`up34_cut`: positions from `end_point`, through the nodes
with no brackets of their own, `up34_flat`); the token after the inner `)` is read too. A `(` that an earlier
attempt read starts no attempt. Which attempts run is recursion through negation (an attempt runs only where none
before it read its `(`); three alternating rounds (`up34_read1..3`) take it. Checked on ad-hoc files against Ruff
(22 lines: `f((a)((b)))` nothing, `((lambda a, b=((1)): 0))` at the second double, `(((a)), ((b)))` at `((b))`,
`for ((a,)) in b` nothing, `with ((a)):`, a line continuation) and over the whole fixtures corpus: 93 keys, Ruff's
93.

Two facts of the tape the rule works around, worth a note for core:
- The source after a tuple's last child stops short of a trailing `,` and the `)` (`(1,)`: `structural_after(1)`
  is ""), though after a parenthesized expression's child it holds the `)`. A tuple is cut at its first element;
  a pattern `(a,)` by its text.
- A rule cannot bind a node's kind to a symbol and use it as text (`kind(n, k), … cat(k, …)` is refused: "$k is
  a Name and a Kind").

At 103 lines the rule is long. The vocabulary it lacks is a token stream: `token(n, i, t)` of a document in order,
with `(`, `,` and `)` (the tape leaves them out, decision 0161), would make it Ruff's loop over tokens.

## UP035 (deprecated import): different, the translation right

Translation only, fixtures: `pyflakes/F401_0.py:13`, `:84` and `pyupgrade/UP045_1.py:55`. Ruff 0.17.6 reports all
three (`typing.Dict`/`List`/`Set`/`Type` at 13, `typing_extensions.TypeAlias` at 84, `typing_extensions.Optional` at
55). The original reports nothing in either file: its run fails on them, "source-incomplete: file has syntax errors"
(checked on a box with each file alone; F401_0.py holds `match *0, 1, *2:`). Not changed.

## UP036 (outdated version block): different, the translation right

- Original only, `probes/PYI003-comments-inside-slice-and-after-operator.pyi:6` and `:8`:
  `(sys.version_info >=  # c\n (3, 8))`. Ruff reports both. ast's `cmp_pair` counts the comment as an operand, so
  the test was `L >= <comment>`. The rule now reads the comparison's operands without comments (`operand`) and its
  operator by `op_at`. Fixed.
- Translation only, `pyflakes/F401_0.py:81`, `if sys.version_info >= (3, 10):` at a 3.10 minimum: Ruff reports it;
  the original fails on the file (above).

**Library change proposed:** ast's `cmp_pair` should skip comments and line continuations when it pairs operands
(misc2-version's `ytt_single_cmp` reads it too); then `operand` goes.

## UP039 (unnecessary class parentheses): same

Original only, `probes/UP039-line-continuation-in-parentheses.py:1` and `:6`: `class A(\\\n):`. Ruff reports both
(line 6 holds a comment too, so its fix is not offered). A line continuation is a node of the argument list; it now
counts as no argument, as a comment does.

## UP041 (timeout error alias): same

Original only, `probes/UP041-parenthesized-sites.py:30`, `(socket.timeout)("message")`: Ruff reports it (no
parentheses in its AST). upgrade-imports' `exc_site(f, "call")` takes the call's function as written; the rule adds
the parenthesized function with its parentheses off.

**Library change proposed:** `exc_site`'s "call" case should take `unparen` of the function. UP024 reads it too
(`(IOError)(1)`); then UP041's extra clause goes.

## UP043 (unnecessary default type args): same

In `probes/UP043-string-annotations-and-tuple-forms.pyi`:

- Original only, lines 3 and 4, `Optional["Generator[int, None]"]` and a string nested in one: Ruff reports both.
  upgrade-typing's `sub_of` resolves by `sym_at`, which finds no imported name in a string annotation's tree (it
  gives `builtins.int` for `int`, nothing for `Generator`). The rule adds a clause over the parsed trees
  (`parsed_in`) with typing's `tsub` and `pyi_sym`, as PYI030 does. Ruff does not report in a "complex" string
  (escaped, or implicitly concatenated: lines 5 and 6), where a name reads the last binding Ruff resolved at the
  string's one position; `complex_str` is PYI030's.
- Translation only, line 7, `Generator[(int, None),]`: the trailing comma makes the slice a tuple of one tuple, one
  argument. `one_tuple` reads it from the subscript's text (the tape leaves the `,` out).

Checked on an ad-hoc stub against Ruff (`"Generator[(int, None)]"` reported, `"Generator[(int, None),]"` not,
`r"Generator[int, None]"` reported).

**Library changes proposed:** upgrade's `sym_at` should resolve a name in a string annotation's tree as names'
`sym_of` does (Ruff resolves those after the module: the last binding); then UP043's string clause is its main
one. `complex_str` (here and in PYI030) belongs in python/typing.
