# python-12: UP024, UP026, UP028-UP033 against their JSON originals

Ground truth: Ruff 0.17.6 on a fleet box (`ruff check --select <CODE> --no-cache`), and its source at tag 0.17.6
(`crates/ruff_linter/src/rules/pyupgrade/rules/*.rs`, the checker's `add_binding` and `handle_node_delete`). Each
counterexample below was run through Ruff and through the translation (`rerail-engine run-policies`).

## ruff-up024-os-error-alias

Followed Ruff. `except A, B:` is Python 3.14's unparenthesized tuple (PEP 758); tree-sitter gives the clause one
`value` field per element, so the library's `up_handler_type` saw each element as a handler type of its own and the
first one, if a tuple, as the tuple to look in. Ruff's type is one tuple, its elements are checked (parentheses
off, not unpacked further), and the finding is where the tuple starts.

    except (IOError, KeyError), e:   Ruff: nothing (elements are a tuple and `e`); translation was: UP024
    except KeyError, \
      socket.error:                  Ruff: UP024 at line of KeyError

Review fixes (review/python-12.review.md, its ten counterexamples and their variants are the rule's tests). Ruff
resolves a name when its walk reaches it: the module first, then function bodies one depth at a time, then
type-parameter bounds and defaults and `type` values, lambdas, deferred annotations, and last every string read as a
type. The first scope on the name's chain that offers a binding wins; a scope already finished by then offers its
last binding. python/upgrade's `sym_at` differed in ten ways, so the rule resolves names itself (`rch`, `offer`,
`rsel`, `r_qual`):

- comprehensions are scopes, their targets bound before the element and the `if`s;
- `except E as x` binds x for the clause only, its type included; after it, x is what it was before;
- an unconditional `del` unbinds the name (not under if/while/match/except/else);
- PEP 695 type parameters give no qualified name, but a def's defaults are read outside them;
- `global`/`nonlocal` leave the binding in the function (Ruff's synthetic module binding of a non-builtin global
  resolves to nothing, as unbound);
- a def's parameters and return annotation, a lambda's parameters, a class's bases and a comprehension's first
  iterable are read in the enclosing scope;
- `type` values, type-parameter bounds and defaults, stub class bases and every parsed string are read after the
  whole file; strings read as types prefer the module's final binding (forward references);
- a relative import with no package, or one that climbs above the top package, names nothing;
- workspace re-exports are not followed; the callee is matched with parentheses off.

Counterexamples (Ruff → old translation): `from .mmap import error; raise error` in a root `a.py`: nothing → UP024;
`(IOError)(1)`: UP024 → nothing; `[IOError() for IOError in e]`: nothing → UP024; `del IOError; raise IOError`:
nothing → UP024; `except Exception as IOError: pass` then `raise IOError`: UP024 → nothing;
`cast("socket.error(1)", x)`: UP024 → nothing; `type X = socket.error(1); import socket`: UP024 → nothing.
Not followed: strings in the functional forms of NamedTuple/TypedDict and mypy_extensions, `Annotated` metadata
nested inside a type, Ruff's full table of generic classes (typing names, builtin generics and `collections.abc`
are known).

## ruff-up026-deprecated-mock-import

Followed Ruff; three fixes.

- `mock.mock.X` is reported once Ruff has *seen* a `mock` import (`seen_module`), in its walk order: the module,
  then function bodies one nesting depth at a time (all depth-1 bodies in source order, then depth 2, …), then
  lambda bodies, then deferred annotations (string annotations, and all annotations in a stub, under
  `from __future__ import annotations` or from 3.14). The rule orders an attribute and the imports by
  (function depth, offset), and an attribute in a lambda body or deferred annotation sees every import of the
  file. Probe `UP026-deferred-visit-order.py`: `def f(a=mock.mock.X): return mock.mock.Z` before an `import mock`
  inside a later function is not reported (lines 1, 2); `(mock).mock.X` in a nested function after it is (line 11).
- The object `mock.mock` is matched with parentheses off (`(mock).mock.X`, `(mock.mock).C`).
- `from mock.mock import x` is not reported (Ruff: only `from mock`); `import mock . mock` is (spaces dropped).
  Relative imports (`from .mock import x`) do not count as seeing `mock`.

Review fixes (four counterexamples, plus variants, are the rule's tests). Ruff's deferred order is: stub class
bases met in the module's walk, then function bodies by depth, then type-parameter bounds and defaults and `type`
values, lambdas, deferred annotations, and string type definitions; the bases of a stub class defined in a function
come in the next round.

- A stub base of a class outside functions sees only the imports of the module's own walk.
- `type` values, type-parameter bounds and defaults, and strings read as types outside annotations (`cast`,
  `List["…"]`, …) are walked late and see every import.
- A `case` pattern's dotted name `mock.mock.…` (value, class, mapping and keyword patterns) is reported where the
  name starts; a bare `case mock.mock:` is not.

Counterexamples (Ruff → old translation): `import mock; cast("mock.mock.Y", 1)`: lines 1, 3 → line 1;
`type X = mock.mock.Y` then `import mock`: lines 1, 2 → line 2; `case mock.mock.Y: pass` after `import mock`:
lines 1, 3 → line 1; in a `.pyi`, `class A(mock.mock.B): ...` then `import mock`: lines 1, 2 → line 2 (with
`def f(): import mock` instead, Ruff gives line 3 only: the bases come before function bodies).

## ruff-up028-yield-in-for-loop

Followed Ruff (`yield_in_for_loop.rs`, a deferred check against the final scope). Ruff takes the latest binding of x
in the loop's own scope that is still bound; a `del`, a bare annotation, and the end of an `except … as x` handler
leave x unbound. That binding carries the reads of the bindings it replaced (`add_binding` copies references), back
to the last unconditional `del`, the last handler end, or a `global`/`nonlocal` declaration (`scope.add`, which
copies nothing). The rule asks the batch library `lib/python/python-12-scopes.rerail` for those reads: Ruff's scopes
(a binding stays in the scope it is written in, whatever `global`/`nonlocal` says), its events by offset (a for
loop's binding after its iterable, an assignment's after its value, `x += 1` after reading x, a handler binding at
its start and unbinding at its end), a read belonging to the latest event above it (a read from a nested function or
lambda to the final event; class bodies skipped), and its conditional `del` (under if, while, match or any else
clause, even outside the def; in an except handler of its own function or class only).

    def f():
        for x in y:
            yield x
        print(x)
        del x
        x = 1          Ruff: UP028 (x = 1 carries no reads)

The yield's shape is Ruff's `is_same_expr` without parentheses, compared recursively: `(x)` is the name, `(x,)` a
one-tuple (tree-sitter parses `for (x)` as a one-element `tuple_pattern` and the tape has no `,` token, so the
comma is found in the text after the element). Loops at module and class level are checked; only an async function
blocks the rule.

Review fixes (ten counterexamples and their variants are among the rule's 48 tests; Ruff → old translation):
`print(x)` then `except E as x: pass` then `x = 1` after the loop: UP028 → nothing; `for x, (y, z) in items: yield
x, (y, z)`: UP028 → nothing; `yield (a), (b)` and `(yield x)`: UP028 → nothing; `for (x) in y: yield x,`: nothing →
UP028; `for x, x in z`: nothing → UP028; `x += 1` after the loop: nothing → UP028; `global x` loop target: UP028 →
nothing; a `del` in an outer `except` around the def: UP028 → nothing; `for x in x`: UP028 → nothing; a module-level
loop: UP028 → nothing.

## ruff-up029-unnecessary-builtin-import

Followed Ruff. Ruff runs the check after the statement has bound all its names and asks whether the *latest*
binding of the name shadows another binding of the same scope, other than the builtin. So a name the statement binds
twice is a rebinding (`from builtins import str, str`: nothing; `from builtins import zip, map as zip, max`: only
`max`). The rule reports when the import's own binding is the only event of the name in its scope up to the end of
the statement, with the events of `lib/python/python-12-scopes.rerail`: any binding written in that scope (not
moved by global/nonlocal), a `global`/`nonlocal` declaration in a function or class (Ruff ignores one at module
level), a bare annotation in a function or class (at module level the builtin is already bound, so Ruff sets it
aside), and an unconditional `del`. The module name is taken from its `name` children, not text, so spaces go.

Review fixes (six counterexamples and variants, 20 tests; Ruff → old translation): `def f(): global str; str = 1`
then `from builtins import str`: line 4 → nothing; a nested `nonlocal str; str = 2` before `from builtins import
str`: reported → nothing; module-level `global str`: reported → nothing; `def f(): str: type; from builtins import
str`: nothing → UP029; `try: del str` … `from builtins import str`: nothing → UP029; `from six . moves import zip`:
UP029 → nothing.

## ruff-up030-format-literals

Followed Ruff: a `.format` call inside a string Ruff reads as a type is reported (`x: "'{1} {0}'.format(a, b)"`).
The library's `format_call` reads `plain_str`, which reads `value`, and the engine gives `str` but no `value` rows
for literals in a guest document (a tree parsed from a string); `lib/python/python-12.rerail` adds `plain_str` rows
from `str` for those literals (UP030, UP031, UP032 include it).

Review fixes (five counterexamples and variants, 7 tests). Ruff (`format.rs`, `FormatSummary`) parses the decoded
value with its own non-raw parser, not CPython's; the rule now uses python-12's `rfmt_*` parser instead of
pyflakes-format's `brace_*`:

- `\N{…}` after an unescaped backslash is literal text: `r"\N{x} {0}".format(a)`: UP030 (was nothing);
  `"\\N{0}".format(a)`: nothing (was UP030).
- The conversion is any one character: `"{0!x}"`: UP030 (was nothing); `{0!}` and `{0!rr}` do not parse.
- An attribute part runs to the next `.` or `[`: `"{0.a]}".format(a)`: UP030 (was nothing).
- An index that overflows usize is a keyword: `"{0}{99999999999999999999999}".format(a)`: nothing (was UP030). The
  parser takes 20 digits or more as a keyword, where Ruff reads up to usize::MAX; no call has that many arguments.
- Ruff parses the strings it reads as types: annotations (names' `parsed_in`), and the strings python-12-types finds
  outside them (`cast`, `TypeForm`, `NewType`, `TypeVar`, `TypeAliasType`, `TypeAlias`, `type`, PEP 695 bounds and
  defaults, generic subscripts such as `List[…]`, `list[…]` and `collections.abc` classes); not `Literal[…]`, call
  arguments or `Annotated` metadata wherever it stands. Adjacent literals inside one are one literal:
  `t: "'{0}' '{1}'.format(a, b)"`: UP030 (was nothing).

## ruff-up031-printf-string-formatting

Followed Ruff (`printf_string_formatting.rs`):

- Each piece of adjacent literals is parsed on its own, as source text between its quotes; the first piece that
  does not parse ends the check. `"%" "s" % (x,)`: Ruff nothing, translation was UP031. The pieces seed the
  library's percent parser (`pct_value`).
- A piece Ruff cannot convert (`%d`, `%i`, `%u`, `%c`, `*`, `%()`, `%r`/`%a` with modifiers, `%5s`, an unnamed
  placeholder with a dict) is reported whatever the right side: `"%04i" % (n + 1)` (django makemigrations.py:484).
- A dict right side is always reported; by hand when it cannot become keywords (empty, a non-identifier or
  non-string key, a repeated key).
- Where: a convertible one at the `%` expression, whose range starts at a `(` around the left string
  (UP031_0.py:89); a by-hand one at the string itself (django testcases.py:434, `msg_prefix + ("…%d…") % (…)`).
- String annotations and other type strings: python-12.rerail, as UP030 (UP031_0.py:131, probe annotation-operands
  lines 3-5).

Review fixes (six counterexamples and variants, 7 tests). The rule now uses python-12's `rpct_*` parser, Ruff's
`cformat.rs`:

- `%b` is bytes-only: `"%b" % x`: nothing (was UP031).
- A mapping key runs to its balancing parenthesis: `"%(a(b))s" % x`: UP031 (was nothing); `"%(a(b)s" % x`: nothing
  (was UP031). Keys are matched to three levels of nesting; a deeper one is taken as not parsing.
- Width and precision are i32: `"%2147483648f" % x`: nothing (was UP031); `%2147483647f` is reported.
- A dict over several lines is converted only if some string key starts its line; otherwise Ruff reports it by hand
  at the string: `x = (\n "%(a)s"\n) % {"a": [\n 1]}`: by hand, line 2 (was fixable, line 1).
- A t-string right side is left: `"%s" % t"x"`: nothing (was UP031).
- Adjacent literals inside a string annotation are one literal: `z: "'%s' '%s' % (a, b)"`: UP031 (was nothing).
  A type string with escapes or adjacent literals makes Ruff 0.17.6 panic in this rule (the file then reports
  nothing); the rule leaves such strings rather than model the panic.

The tests expect the code `UP031`: test-rr names findings by code, so they cannot tell the fixable finding from the
by-hand one; the line tells them apart here.

## ruff-up032-f-string

Followed Ruff 0.17.6 (`f_strings.rs`); several differences from the earlier translation:

- No line-length test: 0.17.6 has none (the conan samples' long lines, profile_loader.py:165, lockfile.py:48, are
  reported). `fits`, `new_width`, `old_width` are gone.
- Every argument, used or not, parentheses off, must be plain: no `*`/`**`, no quote character in its text, not
  spanning lines, and at least one argument. `"Hello".format("world")` (F523.py:36) and
  `"Hello, {name}".format(greeting="Pikachu", name=print(1))` (F522.py:14): Ruff nothing, translation was UP032.
  `"{}".format((a  # it's\n))`: the argument is `a`, Ruff reports.
- A repeated field with an effectful argument stops it only within one piece of adjacent literals (Ruff's `seen`
  set is per piece): `"{0}" "{0}".format(x for x in a)` is reported. Pieces seed the brace parser to number their
  fields. Effects follow `contains_effect`: comprehensions, generators and yields too; an operator only when an
  operand is not a literal or display.
- `("{}".format)(x)` (EM103 probe lines 2, 6, 11) is a format call: Ruff's `call.func` has no parentheses; the rule
  adds that `format_call` clause.
- String annotations (UP032_0.py:270) and other type strings: python-12.rerail.

The JSON is wrong on `"{} {1}".format(1, 2)` (F525.py:1, 2 and rerail/noqa/F525.py:1, 5): Ruff returns early when
`{}` and `{1}` are mixed (a ValueError at run time); the JSON reports it, the translation does not.

Review fixes (ten counterexamples and variants, 12 tests). Ruff converts each piece of the literal from its source
text, undecoded, with its own parser (raw pieces as raw), python-12's `rfmt_*`:

- `"{{ {}".format(a)`: UP032 (was nothing; the brace-count `split_field` is gone). `"\x7b}".format(a)`: nothing (a
  lone `}`; was UP032).
- An assignment expression is an effect: `"{x} {x}".format(x=(y := 1))`: nothing (was UP032).
- Each field needs an `s`/`r`/`a` conversion, attribute parts that are identifiers and not keywords
  (`"{0.class}"`: nothing, was UP032), and string keys with neither a backslash nor the f-string's quote
  (`"{0[it's]}"`: nothing, was UP032; `'''{0[it's]}'''`: UP032). Automatic fields count across pieces.
- A bare generator argument's range has no parentheses: `"{}".format(\n x for x in y\n)`: UP032 (was nothing).
- In a type string with escapes or adjacent literals Ruff's nodes have no tokens of their own and nothing is
  reported: `z: "'{}'" ".format(a)"`: nothing (was UP032).
- Type strings as UP030: `cast("'{}'.format(a)", q)`: UP032 line 2 (was nothing); `u = List["'{}'.format(a)"]`:
  UP032; `Foo[…]`, `Annotated[int, "'{}'.format(a)"]` and `cast(Annotated[int, "…"], q)`: nothing (the last two
  were UP032).
- gettext is resolved through the file's own imports only: after `from mymod import gettext` (mymod re-exporting
  django's), `gettext("{}".format(a))` is UP032 (was nothing).

## ruff-up033-lru-cache-with-maxsize-none

Followed Ruff: parentheses count for nothing (`@(functools.lru_cache)(maxsize=(None))`), and the range starts at the
end of the decorator's function, so the finding is at its last name: `@(functools.lru_cache  # c\n)(maxsize=None)`
is on line 4 (probe UP033-comments-around-callee.py).

Review fixes (tests `class_decorators_are_not_looked_at`, `another_modules_reexport_is_not_followed`,
`a_direct_import_is_reported`):

- Ruff checks function decorators only: `@functools.lru_cache(maxsize=None)` on a class, Ruff nothing, translation
  was UP033. The decorated definition must be a function.
- Ruff resolves a name in its own file: `from mymod import lru_cache` is `mymod.lru_cache`, whatever mymod.py
  imports. upgrade's `sym_at` follows workspace re-exports (`resolve_export`), so `@lru_cache(maxsize=None)` after
  that import was UP033. The rule uses `up_chain_name` (the binding's own qualified name) instead.
  Proposed (library): `sym_at` without `resolve_export` for the Ruff rules, or a `ruff_sym_at` beside it; every
  pyupgrade rule built on `sym_at`/`builtin_at`/`up_qual` follows re-exports Ruff does not.

## Library and engine

Batch libraries (new vocabulary, per CONVENTIONS):

- `lib/python/python-12.rerail` (UP030-UP032): `plain_str` rows for literals of type strings (from `str`); Ruff's
  own str.format parser (`rfmt_*`, `rspec_*`) and printf parser (`rpct_*`); `tdef_guest`/`simple_guest`, the type
  strings' guests (annotations from names' `parsed_in`, the others from python-12-types). The rules no longer seed
  pyflakes-format's `brace_value`/`pct_value`.
- `lib/python/python-12-types.rerail` (UP024, UP026, and UP030-UP032 through python-12): the strings Ruff reads as
  types outside annotations and their parsed trees (`type_node`, `type_str`, `type_guest`; `Annotated` metadata is
  skipped everywhere, `tys_meta`), and the deferred `type` code (`late_type`, `in_stub_bases`).
- `lib/python/python-12-scopes.rerail` (UP028, UP029): Ruff's binding and reference model on demand
  (`ruff_scope_want`, `ruff_site_scope`, `ruff_cond_del`, `ruff_event`, `ruff_read_at`, `ruff_bound`,
  `ruff_carries`).

Proposed (shared libraries; not changed):

- python/upgrade: `sym_at` follows workspace re-exports (`resolve_export`), Ruff never does; and its scoping misses
  comprehension, except-target, `del` and type-parameter scopes, header parts read in the enclosing scope, forward
  references in type strings, and root-relative imports. Replace `sym_at`/`up_site` with UP024's resolver (UP033
  uses `up_chain_name` meanwhile; python-12-types' `typing_ref` still goes through `sym_at`).
- python/names or python/ruff: move python-12-scopes there as a Ruff view beside names' Python view, and
  python-12-types' type strings beside `parsed_in` (one `type_string_guest(e, s)`); `parsed_in` parses `Annotated`
  metadata and strings in call arguments within annotations, Ruff neither.
- python/pyflakes-format: Ruff's F52x rules use the same parsers, so `brace_*`/`pct_*` could take `rfmt_*`/`rpct_*`'s
  behaviour (`\N{…}`, any one-character conversion, `]` in an attribute, usize indices, balanced key parentheses,
  i32 width/precision, no `%b` in a str); `format_call` should take a parenthesized callee itself (UP032 still adds
  that clause).
- A library `ruff_walk(n, phase, depth)` (module walk, stub bases, function depth, type params, lambdas, deferred
  annotations, strings) would replace UP026's `walk_depth` and UP024's own phase order.

Open engine requests (none worked around by an engine change):

- `value` rows for literals in guest documents, as for a file's own; python-12's `plain_str` rows then go.
- A finding at a node of a guest a rule parsed itself (`parsed(s, "annotation", r)`) is dropped unless the violation
  clause names the guest relation; UP024 and UP026 split their clauses on `type_guest(e, _)`.
- In a guest document, `count : { child(e, s, _) }` with `child` the only atom gives 0; with another atom the count
  is right. And a `plain_str` clause with `count`/`join` in its body gave no rows on guest nodes, the same body in a
  relation of its own does (`guest_concat`).

check_rr reads `.test` fixture lines as rule strings, so a backslash escape in a fixture's `"…"` literal is flagged
as an unknown escape; the fixtures with escapes use single-quoted literals.

## Status (last ptcheck, all eight rules together, after the review fixes)

Seven `same`. `ruff-up032-f-string` `different`, right: translation: fixtures 6 original-only (the mixed
`{}`/`{N}` strings of pyflakes/F525.py:1, 2 and rerail/noqa/F525.py:1, 5, 7, 9, which Ruff does not report), 0
translation-only; samples-python 0/0; examples 0/0. Unchanged by the review fixes; equivalence.json now says so
(the seven `same` rules left `different`, UP032's entry has this reason and count). test-rr: 117 tests pass.
