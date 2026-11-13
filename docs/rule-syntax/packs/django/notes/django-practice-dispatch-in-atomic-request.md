# django-practice-dispatch-in-atomic-request

Invariant: with `ATOMIC_REQUESTS` on, a view (or code a view calls) enqueues tasks and sends mail through
`transaction.on_commit`, never directly, because the request's transaction has not committed yet.

## Sources

- Django docs, Database transactions, "Tying transactions to HTTP requests"
  (https://docs.djangoproject.com/en/stable/topics/db/transactions/#tying-transactions-to-http-requests): "It works
  like this. Before calling a view function, Django starts a transaction. If the response is produced without
  problems, Django commits the transaction. If the view produces an exception, Django rolls back the transaction."
- Same page, "Performing actions after commit"
  (https://docs.djangoproject.com/en/stable/topics/db/transactions/#performing-actions-after-commit): "Sometimes you
  need to perform an action related to the current database transaction, but only if the transaction successfully
  commits. Examples might include a background task, an email notification, or a cache invalidation."
- Celery docs, Tasks, "Database transactions"
  (https://docs.celeryq.dev/en/stable/userguide/tasks.html#database-transactions): "There is a race condition because
  transactions are atomic. This means the article object is not persisted to the database until after the view
  function returns a response. If the asynchronous task starts executing before the transaction is committed, it may
  attempt to query the article object before it exists."
- TestDriven.io, "Working with Celery and Django Database Transactions"
  (https://testdriven.io/blog/celery-database-transactions/): "Since the task executes immediately, user =
  User.objects.get(pk=user_pk) fails as the user is not in the database because the transaction in Django has not yet
  been committed."

## Linters checked

Ruff (DJ001–DJ013 in crates/ruff_linter/src/codes.rs, no rule mentions on_commit, atomic or Celery; S, DTZ, ASYNC
unrelated), flake8-django (README rule table DJ01–DJ13), pylint-django (checkers/: auth_user, forms, models,
json_response, foreign_key_strings, format_html, migrations), Bandit, django-upgrade, django-migration-linter, Django
system checks: none reads ATOMIC_REQUESTS or dispatch placement. The sibling single-file rule
django-practice-enqueue-on-commit reports dispatches inside an explicit `transaction.atomic`; this rule skips those
(`!dj_in_atomic`), so no line is reported twice.

## What it matches

Cross-file facts: the settings module that turns ATOMIC_REQUESTS on (any file of the run), view classes resolved
through the run's classes (dj_view_class), and the cross-file call graph (python/calls `owner`/`call_target`) from
every view to the function that dispatches. One-file facts: the dispatch call, `on_commit` around it, atomic around it,
`@non_atomic_requests` on the view.

- On: a dict entry `"ATOMIC_REQUESTS": True`, `DATABASES[...]["ATOMIC_REQUESTS"] = True`, `ATOMIC_REQUESTS=True` as
  a keyword argument, or the entry's value a parameter whose default is True (addons-server's
  `get_db_config(..., atomic_requests=True)`). Test files are skipped. Env-driven values (`env.bool(...)`,
  `cfg.get(...)`) are unknown and do not turn it on. A top-level `ATOMIC_REQUESTS = True` is not a Django setting
  (Django reads it only per database) and does not turn it on.
- Views: a def whose first parameter is `request` and that is not a method (function views, decorator wrappers,
  admin-style helpers); every method of a Django/DRF view class. Not async defs (Django raises for ATOMIC_REQUESTS on
  async views). Not non-atomic views: `@non_atomic_requests` on the def, the class or a method (method_decorator
  included), `non_atomic_requests(...)` anywhere in the class body (ralph's and addons-server's `as_view` override),
  or a urls-level `non_atomic_requests(views.f)` in any file.
- Reached: a function the view calls through the call graph, directly or transitively, except calls written inside
  `on_commit(...)`.
- Dispatch: as enqueue-on-commit (`.delay`, `.apply_async`, `.send_task`, `.enqueue`, `.aenqueue`, `async_task`,
  send_mail family, `EmailMessage.send`), not inside `transaction.on_commit(...)` or `<connection>.on_commit(...)`.
- Project-wide deferral: when some class of the run defines `apply_async` with an on_commit call in its body (a Task
  base wired with `Celery(task_cls=...)`, addons-server's AMOTask), `.delay()`/`.apply_async()` are not reported; mail
  still is.

Misses: views wired through `urls.py` whose first parameter is not named `request`; dispatches reached only through
framework code (DRF calling a serializer's `create`), signals, or calls whose target is unknown (`call_open`);
ATOMIC_REQUESTS set from environment variables.

Limitation: the ATOMIC_REQUESTS fact is global to the run; a monorepo with two projects, one atomic, applies it to
both. cases.py runs one directory per rule, so the fact-absent and other settings forms were checked on copies of the
case directory run with rerail-engine directly: the True entry deleted (leaving the top-level `ATOMIC_REQUESTS = True`
and an env-driven entry) gives 0 findings; the param-default, subscript and `dict(ATOMIC_REQUESTS=True)` forms each
give all 9; adding a Task class whose `apply_async` calls on_commit leaves only the 4 mail findings.

## Corpus

Which repos turn ATOMIC_REQUESTS on (grep of all 32, then the rule's dar_on):
- on, dict literal: allegro/ralph (src/ralph/settings/base.py:258), ansible/awx (awx/settings/defaults.py:26),
  django-oscar (sandbox/settings.py:27), openedx/edx-platform (openedx/envs/common.py:405, lms|cms/envs/test.py);
- on, parameter default: mozilla/addons-server (`get_db_config(environ_var, atomic_requests=True)`);
- not on: readthedocs (`ATOMIC_REQUESTS = True` as a class attribute of its settings class) and taiga-back
  (`ATOMIC_REQUESTS = True` at top level): neither is read by Django, which takes it per database only, so both run
  autocommit (a finding in itself, out of this rule's scope); mathesar (env-driven `cfg.get("ATOMIC_REQUESTS")`);
  archesproject (False, test fixture); the rest never mention it, so dar_on is empty and the rule reports nothing by
  construction. Run to confirm on the near-miss repos: taiga-back 0, readthedocs 0.

Results (final rule): ansible/awx 6, allegro/ralph 0, django-oscar 0, mozilla/addons-server 0, taiga-back 0,
readthedocs 0; openedx/edx-platform timed out (900 s, 3600 s, and stopped after ~3000 s on a third run: the typing
program the call graph needs, on 4459 files, on a loaded machine). Matches 6, repos 1, read 6, real 6, precision 100%.

Real examples:
- ansible__awx:awx/conf/views.py:133 — `reconfigure_rsyslog.delay()` in a view's perform_update, three lines after
  `connection.on_commit(lambda: clear_setting_cache.delay(...))` for the same settings change: the worker may
  reconfigure from the uncommitted settings.
- ansible__awx:awx/api/views/__init__.py:2565 — `update_inventory_computed_fields.delay(...)` right after
  bulk-deleting hosts in the request: the counts can be computed before the delete commits.
- ansible__awx:awx/api/views/__init__.py:551 — health-check task sent right after `obj.save()`.
- ansible__awx:awx/api/serializers.py:2361 — reached from a view through the call graph (a serializer's create).
Caveat: awx's tasks go through dispatcherd's pg_notify broker; if it publishes on Django's connection, NOTIFY is
delivered only at commit and these are harmless at run time. They still hold the pattern (and awx itself wraps the
neighbouring sends in on_commit), so they count as real by the bar.

False-positive classes found and fixed:
1. `connection.on_commit(lambda: task.delay(...))` (awx, 9 of the first 15): only `transaction.on_commit` was
   recognised. Now any `.on_commit(...)` call counts; negative cases `connection.on_commit` and
   `connections["default"].on_commit`.
2. addons-server (17 matches, all `.delay`/`.apply_async`): its AMOTask overrides `apply_async` to send through
   `transaction.on_commit`, and `Celery("olympia", task_cls=AMOTask)` makes every task use it, so no send runs before
   commit. Fixed with the project-wide deferral fact (dar_held); checked on a case-directory copy.
3. Non-atomic views by call, not decorator (`non_atomic_requests(super().as_view(...))` in ralph and addons-server,
   urls-level wrapping): now recognised; negative cases ImportView and `upload` (urls.py).

## Mode

steer: 6 of 6 read matches are real. Few matches because only 5 of 32 repos turn ATOMIC_REQUESTS on and the largest
of them (edx-platform) did not finish.

## Helpers worth a library

- dar_side_effect / dar_task_method / dar_mail_fn / dar_message: a copy of enqueue-on-commit's eoc_side_effect; both
  rules should read one `dj_side_effect(c, what)` in python/django.rr.
- dar_on_commit / dar_after_commit: "inside an on_commit callback" (also eoc_in_on_commit, which misses
  `connection.on_commit`).
- dar_on (ATOMIC_REQUESTS on) and dar_view (sync request handler) for other request-transaction rules.
