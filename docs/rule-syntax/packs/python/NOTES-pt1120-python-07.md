# pt1120-python-07: ARG002-ARG005, G002, G003, G201, G202

All eight rules are `same` as their JSON originals on the examples, fixtures and samples-python (ptcheck on
`tbelanger/pack-translations-1120` with this batch's files, engine `rerail-engine-1120`). Every difference the suite
found before was the translation's error: Ruff 0.17.6 (`ruff check --select <CODE> --no-cache`) agrees with the
original on each sample key. The rules now share a new library, `lib/python/visit-order-bindings.rerail` (`visit_*`),
because the fixes are each needed by two rules or more and touch relations of `misc2-args`, `logging`, `ruff` and
`scopes`, which this batch may not edit. Everything there is per file, keyed by node; with `RERAIL_STRICT_CROSSINGS`,
`RERAIL_STRICT_NODES` and `RERAIL_STRICT_POSITIONS` the eight rules run.

## Ruff's binding where a name is read (`visit_seen`, `visit_qual`): ARG002, ARG004, G002, G003

Ruff visits a scope in order, and a def's or lambda's body after the scope around it. `ruff_qual`'s `rebound` takes
the latest binding before the reference in every case, ignores deletions, and lets an `except … as x` binding stand
after its clause. Counterexamples, Ruff's findings beside them:

- `ANN001-staticmethod-shadowed-later-in-module.py`: `class Early: @staticmethod def s(a, b)` with
  `staticmethod = lambda fn: fn` later at module level. Ruff: ARG004 at line 3 (the decorator is read eagerly,
  before the module's binding: the builtin); the old translation said ARG002. Inside `def factory(): class Late:`
  the same decorator is read after the module: ARG002 (both agree).
- `G003-deferred-and-class-scope.py`: `def f(): log1.info("a" + str(x))` then `log1 = None` after it: Ruff G003 (f's
  body sees the assignment, a logger by name); the old translation saw the import. `log2 = None`, `def g(): …`,
  `from foo import log2`: Ruff nothing (g sees the import); the old translation reported it.
- `G002-binding-forms.py`: `for _ in []: del log2` then `log2.info("%s" % x)`: Ruff G002 (an unconditional `del`
  unbinds the import: a name again). `except Exception as log4: pass` then `log4.info(…)`: Ruff nothing (the import
  is back after the clause); the old translation reported it.

`visit_seen(r, b, f)` is the latest candidate binding of the scope r's lookup lands on: those before r, or all of them
when a def or lambda body lies between r and that scope; an `except` name only inside its clause; scopes' `del_site`
(Ruff's unconditional `Deletion`) as form "deletion". An eager read with no binding yet is the builtin. Asked only
for callees, their objects' roots and decorators (`visit_need`). `misc2-self`'s `slf_bind` is a near relative (latest
before in its own scope, last in an outer one), but reads a class body's reference to the module as deferred, which
Early's decorator shows wrong.

**Library change proposed:** `ruff_qual` (python/ruff) should resolve through this binding, and logging's
`logger_expr`/`local_def_chain` and misc2-args' `argt_dec` with it; then `visit_qual`, `visit_dec`, `visit_logger` and
`visit_log_call` go.

## Unused arguments (`visit_unused`): ARG002-ARG005

- **Annotated `*args: T` and `**kw: T`** (scopes' `param` misses them, so `argt_read` and `argt_rebound` never saw
  them): dask `highlevelgraph.py:617` `def merge(cls, *graphs: Graph)` reads `graphs`; pydantic `main.py` and dvc
  `**kwargs: Any` likewise; `PYI034.py:366`'s `__new__(…, **kwargs: Any)` passes it to `super().__new__`. Ruff
  reports none of them; the old translation reported all (8 ARG002 and 6 ARG003 samples, 1 ARG004 fixture). A
  typed splat rebound in the body (`a = 1`) is not reported either. `visit_read` takes every `sig_param` name and
  a load in the body whose `walk` passes through f.
- **Defaults are read outside**: `lam20 = lambda a, c=a: c` is ARG005 at `a` (Ruff); a read must be in f's body.
- **`locals()` in a lambda** (`ARG005-lambda-locals-scopes.py`, 13 keys): it counts only in the lambda's own scope,
  not in a nested lambda or a comprehension past its first iterable (`lambda a: [y for y in range(3) if (z :=
  locals())]` is ARG005 at a; `lambda a: [y for y in (locals)() if y]` is not); `[*locals()]` is a call of
  `*locals` in tree-sitter and counts (`lam10`, not reported by Ruff). In a class body, at any depth, Ruff reports
  a lambda's unused parameters whatever it calls (`class D: e = [lambda w: locals() for _ in range(2)]` is ARG005
  at w; so is a lambda in a method's default or nested function there). Followed: Ruff, as the original.
- **The variable stub with no names** (`ARG002-variable-stub-generator-argument.py`, lines 2 and 10): Ruff's
  `is_not_implemented_stub_with_variable` compares the assignment target's name with the raised argument's as
  `Option`s, so `self.msg = "…"; raise NotImplementedError(i for i in ())` (neither a name) is a stub, not reported.
  `visit_raise_none` adds that case to `arg_stub`.

**Library change proposed:** scopes' `param` (and so `bound_in`, `binding`'s "param" sites, `bind_only`) should
take the identifier inside a typed `list_splat_pattern`/`dictionary_splat_pattern`; misc2-args' `argt_read` should
require the read in the body, `argt_locals` should take the lambda scope rules above, and `arg_stub` the `None ==
None` case. ARG001 (not this batch) has the same defects in `arg_unused`.

## Logging (`visit_log_msg`, `visit_in_handler`): G002, G003, G201, G202

- **`msg=` first**: Ruff's `find_argument_value("msg", pos)` takes the keyword before the position.
  `logging.info("a" + str(x), msg="x")` (`G004-message-slot.py:13`): Ruff no G003; the old translation reported it.
- **The handler flag stops at a def's or class's body**, not at a lambda (`G201-handler-flag-cleared-by-class-body.py`):
  in `except:`, `class K: f = lambda: logging.error("a", exc_info=True)` and `def one(): logging.error(…)` are
  not G201 (lines 6, 19), and the G202 at 7, 20 likewise; `@dec(lambda: logging.error("b", exc_info=True))` (9),
  a def's default (13) and return annotation (16) are. logging's header says the flag "survives into functions
  defined in the handler"; Ruff 0.17.6 does not.

**Library change proposed:** logging's `log_msg` keyword-first and `log_in_handler` as `visit_in_handler`.

## Engine gaps

None needed.
