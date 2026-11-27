# pt1120-python-08: flake8-slots SLOT000–SLOT002, flake8-todos TD002–TD004

Six rules on #1120's line, checked with `translations:check` (ptcheck, engine `rerail-engine-1120`) against their
JSON originals, and each differing key against Ruff 0.17.6 (`ruff check --select <CODE> --no-cache`) on a fleet
box. Final: SLOT000, SLOT001, SLOT002 `same`; TD002, TD003, TD004 `different` by one key each, the original right,
fixed only by a change to an existing library (below). Every rule stays per file: no relation is declared
`cross_file`, and a run of all six under `RERAIL_STRICT_CROSSINGS`, `RERAIL_STRICT_NODES` and
`RERAIL_STRICT_POSITIONS` (each with `RERAIL_SHOW_CROSSINGS=1`) passed and printed nothing.

New library: `lib/python/pt1120-python-08.rerail` (SLOT000 and SLOT001 share it): `slots_unsub` (Ruff's
map_subscript, one level) and `slots_tp_builtin` (a base naming the class's own type parameter, still read as the
builtin).

## SLOT000 no-slots-in-str-subclass: same

Followed Ruff (and the original) on a base read as a member of a class of the file. Ruff resolves `Outer.Gone`
through Outer's scope, which keeps the last binding of `Gone`; misc2-slots' `slots_enum` reads the class's ancestry
by qualname (`class_anc`, through the phase py_names), where `Outer.Gone` is the class `Outer.Gone` whatever Outer
binds to `Gone` afterwards. The rule now reads the enum test base by base (`slot000_enum`): a name or member that
names a class of the file is followed to that definition by node (a name's binding just before it, a member's last
binding in the class body) and to its own bases; any other base by its symbol's ancestry, as `slots_enum` did.
Counterexample (Ruff: SLOT000 at lines 7 and 8; translation before: neither; fixtures
`ruff/rerail/probes/SLOT000-nested-member-enums-rebinding-and-depth.py:28` is the first):

```python
import enum
class Outer:
    class Gone(enum.Enum): ...
    Gone = 1
    class B(enum.Enum): ...
    class B: ...
class Rebound(str, Outer.Gone): ...
class Shadowed(str, Outer.B): ...
```

Also checked against Ruff, all agreeing: a member rebound under `if` (reported), an intermediate member rebound
(`Outer.Deep.Deeper` after `Deep = 2`: reported), `(Enum)`, `enum.IntEnum`, a class of the file deriving from an
imported Enum through two levels and through a nested class, a local `class Enum` after the import (only the bases
after it lose the enum).

Also followed Ruff for `class B[str](str)` (Ruff: SLOT000; the type parameter does not hide the builtin, see
SLOT001). No corpus file has it; the original's `str-base` reads a type parameter `str` as the builtin too.

## SLOT001 no-slots-in-tuple-subclass: same

Followed Ruff on two keys of `ruff/rerail/probes/SLOT001-type-parameter-does-not-shadow-base.py`:

- Line 16, `class D(Tuple[int][0])`: Ruff's map_subscript takes one subscript off, so this is no Tuple subclass;
  the translation unwrapped subscripts recursively. Now one level (`slots_unsub`), through parentheses.
- Line 4, `class A[tuple](tuple)`: Ruff asks whether `tuple` is the builtin from the scope the class statement sits
  in, when it reaches the statement, before the class's type-parameter scope: the builtin. python/scopes reads a
  name inside a definition declaring it as a type parameter as that parameter (`tp_ref`), which is right for
  typing's `Tuple` (`class C[Tuple](Tuple)`, line 12: Ruff resolves typing names through the recorded binding, and
  reports nothing). `slots_tp_builtin` gives the builtin back where Ruff's lookup from the statement finds no
  binding: none in the enclosing scopes before the class, none anywhere in a scope outside the nearest enclosing
  def (its body is visited after the module), no enclosing definition's own type parameter of that name, and no
  `builtins` import in the file (then Ruff resolves through the recorded binding, the type parameter).

Counterexamples checked against Ruff (all agree now):

```python
class A[tuple](tuple): ...          # SLOT001
tuple = list                        # after the class: A still reported
def f():
    tuple = list
    class B[tuple](tuple): ...      # nothing
def g():
    class C[tuple](tuple): ...      # nothing: the module's `tuple = 3` below is bound when g's body is visited
tuple = 3
class K[tuple]:
    class D[tuple](tuple): ...      # nothing: K's type parameter
```

With `import builtins` anywhere in the file, `class A[tuple](tuple)` is not reported (Ruff and the original).
Residual, not in any corpus: an `import builtins` inside a def after the class would not stop Ruff (its body is
visited later); the rule, as the original, takes any import in the file.

## SLOT002 no-slots-in-namedtuple-subclass: same

Followed Ruff: a `\` line continuation among the bases is tree-sitter's `line_continuation` extra, which
misc2-slots' `slots_base` keeps as a base, so the "every other base is `object` or a namedtuple call" count failed.
The rule takes it as no base. Counterexample (Ruff: SLOT002 at line 1; translation before: nothing; fixtures
`ruff/rerail/probes/SLOT002-line-continuation-in-bases.py:4`):

```python
class A(namedtuple("A", "x"), \
        object):
    pass
```

Library change proposed: misc2-slots' `slots_base` should leave out `line_continuation` as it leaves out
`comment` (`!kind(x, "line_continuation")`); SLOT002's `slot002_ok` clause for it would then go.

## TD002 / TD003 / TD004 missing author / link / colon: different (original right)

One key each: fixtures `ruff/rerail/probes/FIX002-unicode-word-boundaries.py:3`, `# TODO→ arrow`. Ruff takes the
first word as a run of Unicode alphanumerics (`char::is_alphanumeric`), so `→` (a symbol) ends it and `TODO` is a
tag: TD002, TD003 and TD004 at 3:3. The original reports all three; the translation none. misc2-todo's `todo_tag`
and misc2-todo-span's `todo_at` allow only ASCII punctuation or a space after the tag (its own comment names the
gap: "a non-ASCII punctuation mark right after the tag … is missed"). Both relations are an existing library's,
not `extensible`, and every TD rule's author, colon and link logic is keyed on them, so the rules cannot fix it
without a second copy of the TODO parser; they are left as they are.

Library change proposed (both files, one character class each), with what it changes:

```
lib/python/misc2-todo.rerail, todo_tag; lib/python/misc2-todo-span.rerail, todo_at:
-   …(?:[\\t -/:-@\\[-\\x60{-~].*)?…
+   …(?:[^\\p{Alphabetic}\\p{N}].*)?…
```

`\p{Alphabetic}` and `\p{N}` are Rust's `is_alphabetic` and `is_numeric`, which `is_alphanumeric` joins. Run on a
box with the library copied to /tmp and changed: over the fixtures (1963 files) TD002–TD004 gain exactly the three
keys above and nothing else; over samples-python (2313 files) nothing changes. On extra cases it agrees with Ruff
everywhere: `TODO：`, `TODO—`, `TODO€`, `TODO\u00a0x` and `TODO_x` are tags; `TODOⅫ` (a number, Nl), `TODO²` and
`TODOé` are not. With it, the three rules are `same` [INFERENCE from the /tmp run; not run through
`translations:check`, which takes this checkout's library]. TD001 and TD005–TD007 read the same relations and would
change with it too; they are not this batch's.

## Engine gaps

None found.
