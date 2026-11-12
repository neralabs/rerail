# django-practice-select-for-update-outside-atomic

Invariant: a `select_for_update()` queryset is evaluated inside `transaction.atomic()`, on every call path that reaches it.

## Sources

- Django docs, QuerySet API, `select_for_update()` (https://github.com/django/django/blob/main/docs/ref/models/querysets.txt,
  https://docs.djangoproject.com/en/dev/ref/models/querysets/#select-for-update):
  > Evaluating a queryset with ``select_for_update()`` in autocommit mode on backends which support
  > ``SELECT ... FOR UPDATE`` is a :exc:`~django.db.transaction.TransactionManagementError` error because the rows are
  > not locked in that case. If allowed, this would facilitate data corruption and could easily be caused by calling
  > code that expects to be run in a transaction outside of one.

  > Although ``select_for_update()`` normally fails in autocommit mode, since :class:`~django.test.TestCase`
  > automatically wraps each test in a transaction, calling ``select_for_update()`` in a ``TestCase`` even outside an
  > :func:`~django.db.transaction.atomic` block will (perhaps unexpectedly) pass without raising a
  > ``TransactionManagementError``.
- Django ticket #22491 "TestCase + select_for_update()", Aymeric Augustin (core developer)
  (https://code.djangoproject.com/ticket/22491):
  > Otherwise we'll have to document that code involving select_for_update should be tested in a TransactionTestCase.
- django-developers, "select_for_update and transactions (bad dev UX in a feature)", Klemen Štrajhar, 2021-11-17
  (https://www.mail-archive.com/django-developers%40googlegroups.com/msg56291.html):
  > 2. transaction.atomic context manager was accidentally removed. The test still passed as test runner wraps test in
  > it's own transaction. 3. Code was happily deployed 4. On production server there was no more transaction wrapper
  > and exception was raised at select_for_update

  Adam Johnson's reply (https://www.mail-archive.com/django-developers%40googlegroups.com/msg56292.html):
  > FWIW I always recommend enabling ATOMIC_REQUESTS and ensuring transactions are used by default in other code paths
  > such as background tasks.

## Linter coverage

None. Checked the rule lists of Ruff (DJ, DTZ, S, ASYNC: no transaction or select_for_update rule), flake8-django
(DJ01–DJ13: model/form/template rules), pylint-django (no transaction checker), Bandit (no Django ORM check besides
`extra`/`RawSQL` injection), django-upgrade (rewrites only), Django system checks (none look at code paths). None
follows calls, and none knows about the TestCase transaction.

## What it matches

`<anything>.select_for_update(…)` in a def f outside test code, not lexically inside `with transaction.atomic()` or
an `@transaction.atomic` def (dj_in_atomic), when f is exposed:
- entry point: no call in the run reaches f (python/calls call_target), no call of unknown target (call_open) is spelled
  with f's name, and f is not a view (takes `request`, or a view-class method) of a project whose settings say
  `"ATOMIC_REQUESTS": True`;
- or an exposed function g calls f at a site outside any atomic region (transitively; atomic defs stop it).

Cross-file facts: the call graph (callers in other modules), the settings module's ATOMIC_REQUESTS, view classes
resolved across files. One-file facts: the atomic regions.

Misses: querysets built with select_for_update in one function and evaluated by a caller (the caller's call site is
judged instead); class-attribute querysets (`queryset = X.objects.select_for_update()`); a def whose callers are all
of unknown target (the rule stays silent); transactions opened by other means (`set_autocommit(False)`, a custom
context manager that wraps atomic, `ATOMIC_REQUESTS` set through a variable).

## Corpus

INCOMPLETE. The rule depends on python/calls, whose typing program dominates run time, and the machine ran at load
50+ with other batches. Two background runs (private tape caches, DJ_TIMEOUT=3600) were killed (SIGTERM) after
~2 h; per-repo counts from their logs (`/tmp/djxf-sfu-full.log`, `/tmp/djxf-sfu-b.log`):
DefectDojo 0, label-studio 2, LibrePhotos 2, ralph 0, zulip 6, umap 0, taiga-back 1, linkding 0, plus djangoproject 0,
healthchecks 0, inventree 1 from an earlier run. Weblate and saleor timed out at 3600 s; the other repos never ran.
Only 5 rows survived to disk (run.py buffers its output file), all zulip; with the inventree one, 6 read.

Read (6), real 1 (pattern present), false positive 5:
- `inventree__InvenTree:src/backend/InvenTree/order/api.py:707` (`check_received` ← `validate_delete` ←
  `CommonBulkDeleteMixin._delete`): real by the bar; harmless at run time, the one viewset using it wraps
  `bulk_delete` in atomic (the call graph is context-insensitive).
- zulip `zerver/lib/message.py:437`, `:485`, `zerver/lib/thumbnail.py:462`, `zerver/lib/user_groups.py:124`: the lock
  is switched on by a parameter (`if lock_message:`, `if lock:`, `else:` of `if for_read:`), and the reported
  callers pass the safe value or the default. FP class "parameter-gated lock", fixed: `sfu_guarded` skips a site in
  an `if`/conditional whose test reads a parameter of the enclosing def (cases `get_hold`, `get_group`). Not rerun
  on the corpus after this fix.

A static AST survey (`/tmp/djxf/sfu_survey.py`) found 293 select_for_update calls outside a lexical atomic region in
non-test code (PostHog 155, Weblate 36, saleor 35, zulip 10, pretix 9, netbox 8, …): the candidate pool the call
graph filters.

False-positive classes fixed before the runs (each has a case):
1. `with transaction.atomic(), self.lock_objects(qs):` (netbox): a later with-item runs inside the transaction
   (`sfu_in_atomic`).
2. Locking guarded by the flag that opens the transaction, `if self.is_atomic: …` (Misago TargetedView).
3. Test callers made tasks look covered: callers in test code no longer count.
Recall fix: `@contextmanager` defs have no call_target; `sfu_target` falls back to the def the callee's name
resolves to.

## Mode

`log`, likely DROP pending a complete run: 1 real of 6 read (17 %) before the last fix, and every real-by-the-bar
match so far is harmless in practice. The bar needs at least one real match, which exists, so the rule is kept in
`log`; a full corpus rerun (each big repo needs over an hour on a loaded machine) decides keep or drop.

## Helpers worth a library

- `sfu_exposed(f)` / `sfu_entry(f)` / `sfu_bare_call(c, f)`: "f may run with no transaction open", for any rule about
  code that needs a transaction (on_commit outside atomic is the opposite; `durable=True` nesting; savepoint misuse).
  Belongs in python/django.rr as `dj_maybe_autocommit(f)`.
- `sfu_atomic_requests()`: the project sets ATOMIC_REQUESTS; `dj_atomic_requests()` in python/django.rr.
- `sfu_open_name(x)`: some call of unknown target is spelled x; python/calls.rr as `call_open_name(x)`, for any
  "entry point" test.
