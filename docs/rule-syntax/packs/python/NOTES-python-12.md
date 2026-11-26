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

## ruff-up028-yield-in-for-loop

Followed Ruff. Ruff checks the scope's latest binding of x that is not unbound (a `del` makes an unbound one),
and that binding carries the references of every binding it shadowed, back to the last unconditional `del x`
(`add_binding` copies references unless the shadowed binding is a Deletion; a `del` under an if, while, match,
handler or else makes none). The rule orders bindings (end of their site) and unconditional `del`s (end of the
name) by offset: a read in the scope's own walk counts when the latest event above it is a binding with no `del`
between it and the latest binding; a read from a nested function sees the final state.

    def f():
        for x in y:
            yield x
        print(x)
        del x
        x = 1          Ruff: UP028 (x = 1 carries no reads); translation was: not reported

`del x; del x` after the loop: the latest real binding is the loop's own, whose references include the first
`del` (probe function i): not reported, as Ruff.

## ruff-up029-unnecessary-builtin-import

Followed Ruff. Ruff runs the check after the statement has bound all its names and asks whether the *latest*
binding of the name shadows another binding of the same scope. So a name the statement binds twice is a rebinding
(`from builtins import str, str`: nothing; `from builtins import zip, map as zip, max`: only `max`), and a
`global`/`nonlocal` declaration above it is one too (Ruff adds a Global/Nonlocal binding to the scope).

## ruff-up030-format-literals

Followed Ruff: a `.format` call inside a string annotation (`x: "'{1} {0}'.format(a, b)"`) is reported. The
library's `format_call` reads `plain_str`, which reads `value`, and the engine gives `str` but no `value` rows for
literals in a guest document (a tree parsed from a string annotation). New library `lib/python/python-12.rerail`
adds `plain_str` rows from `str` for those literals (UP030, UP031, UP032 include it).

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
- String annotations: python-12.rerail, as UP030 (UP031_0.py:131, probe annotation-operands lines 3-5).

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
- String annotations (UP032_0.py:270): python-12.rerail.

The JSON is wrong on `"{} {1}".format(1, 2)` (F525.py:1, 2 and rerail/noqa/F525.py:1, 5): Ruff returns early when
`{}` and `{1}` are mixed (a ValueError at run time); the JSON reports it, the translation does not.

## ruff-up033-lru-cache-with-maxsize-none

Followed Ruff: parentheses count for nothing (`@(functools.lru_cache)(maxsize=(None))`), and the range starts at the
end of the decorator's function, so the finding is at its last name: `@(functools.lru_cache  # c\n)(maxsize=None)`
is on line 4 (probe UP033-comments-around-callee.py).

## Library and engine

- New `lib/python/python-12.rerail`: `plain_str` rows for literals of string annotations, from `str`. Proposed
  instead (engine): `value` rows for literals in guest documents, as for a file's own; the file then goes.
- UP031 and UP032 add seed clauses to library relations (`pct_value`, `brace_value`) to parse each piece of
  adjacent literals; UP032 adds a `format_call` clause for a parenthesized callee. Proposed (library): a
  `pct_piece`/`brace_piece` view in pyflakes-format and the parenthesized callee in `format_call` itself.
- UP026's walk order (function depth, lambdas, deferred annotations) is Ruff's visit order in general; a library
  `ruff_walk(n, phase, depth)` would serve other `seen_module` rules.

## Status (last ptcheck, all eight rules together)

Seven `same`. `ruff-up032-f-string` `different`, right: translation: fixtures 6 original-only (the mixed
`{}`/`{N}` strings of pyflakes/F525.py:1, 2 and rerail/noqa/F525.py:1, 5, 7, 9, which Ruff does not report), 0
translation-only; samples-python 0/0; examples 0/0.
