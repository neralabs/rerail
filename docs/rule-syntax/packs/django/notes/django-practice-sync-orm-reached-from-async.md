# django-practice-sync-orm-reached-from-async (DROP for now: mode log, no corpus match)

Invariant: an `async def` never calls, directly, a sync function that runs (itself or through further sync calls) a
blocking ORM query; such a function is called through `sync_to_async`/`database_sync_to_async`.

## Sources

- Django docs, Asynchronous support, "Async safety"
  (https://docs.djangoproject.com/en/stable/topics/async/#async-safety, source docs/topics/async.txt):
  "Note that you don't have to be inside an async function directly to have this error occur. If you have called a
  sync function directly from an async function, without using :func:`sync_to_async` or similar, then it can also
  occur. This is because your code is still running in a thread with an active event loop, even though it may not be
  declared as async code."
- Channels docs, Database Access (https://channels.readthedocs.io/en/latest/topics/databases.html):
  "If you are writing asynchronous code, however, you will need to call database methods in a safe, synchronous
  context, using ``database_sync_to_async`` or by using the asynchronous methods prefixed with ``a`` like
  ``Model.objects.aget()``."

## Mode

log (DROP): no corpus match on the finished repos.

## Linter coverage

None. Ruff's ASYNC rules (https://docs.astral.sh/ruff/rules/#flake8-async-async) cover blocking HTTP, subprocess,
`open`, `time.sleep`, not the ORM, and never follow calls. Ruff DJ / flake8-django (DJ001–DJ013), pylint-django,
Bandit, django-upgrade, django-migration-linter: no async or call-graph check. Django raises
SynchronousOnlyOperation only at run time, and only on the path a test exercises.

## What it matches

A call in an async def's own frame (not a nested def/lambda; not test code) whose target, per python/calls, is only
sync workspace functions (no call_open), not decorated `sync_to_async`/`database_sync_to_async`, from which a chain
of sync, non-offloaded functions reaches a query in a function's own frame: a blocking QuerySet method (the sibling's
soa_blocking list), `for` over a queryset, `with transaction.atomic`, save()/delete()/refresh_from_db() on a model
instance. Cross-file facts: call targets and the call graph (python/calls), model classes across files
(dj_model/class_def/mro_method), types (python/types proven_type).

Also follows `obj.m()` where obj is a model instance whose class the rule knows (`self` in a model method, `X(...)`,
`X.objects....get()/aget()`): the type program does not model managers, so these calls are call_open; the rule
resolves them to the model's method through mro_method.

Not reported: calls `sync_to_async(f)`, `asyncio.to_thread(f)`, `run_in_executor(None, f)` (f is not called there),
calls in lambdas/defs handed to them, chains through an async def (calling one only makes a coroutine) or through an
offloaded def, calls with unknown targets, calls the sibling django-practice-sync-orm-in-async reports.

Misses: chains whose links are call_open (receivers of unknown type, getattr dispatch), ORM reached through custom
managers' querysets (dj_qs starts at `objects`), related managers.

## Corpus

Only repos with an `async def` can match (16 of 32; the others have none). Finished, 0 matches each: umap,
paperless-ngx, kiwitcms, healthchecks (final version); cvat, label-studio (earlier version without the class-keyed
self dispatch, a superset); goauthentik (first version, 2998 s). Timed out or not reached before the deadline:
WeblateOrg (>3000 s), zulip and saleor (>2900 s, killed), ansible/awx, pretix, netbox, LibrePhotos, openedx,
PostHog. Matches 0, read 0, precision n/a. No real match found, so by the bar: DROP (kept, `.mode log`). It needs
the big async repos (Weblate, zulip, saleor, PostHog, authentik's outposts) to be judged.

Runtime: the typing program (python/calls) dominates: paperless-ngx 88 s user for a probe rule that reads only
call_target vs 101 s for this rule; umap 16 s vs 25 s. Big repos (authentik, Weblate, zulip, saleor) take 40–50+ CPU
minutes under load, for any rule reading call_target. Running both reach rules in one `run-policies` call shares the
program's cost.

FP classes found and fixed (cases added): late-bound `self.m()` going to every subclass override (now `self.m()` in
a class's methods goes to that class's m, mro_method; a subclass whose override queries is reported at `class Sub`,
consumers_late.py).

Library gap found (python/imports.rr, not this rule's): `from .. import X` and `from ..m import X` bind nothing
sym_of can follow (impmod handles one dot only; rifrom does not use module_text/rel_base), so classes based on a
two-dot import are neither views nor models (makeplane/plane's `from .. import BaseAPIView`). Fix: derive rifrom
from module_text (which already resolves any depth) instead of impmod.

## Helpers worth a library

`sora_instance_class` / `sora_qs_model` / `sora_fetch_call` / `sora_model_call` (identical to
django-practice-get-handler-writes' `ghw_*`): "identifier holds an instance of model class q" and "`obj.m()` runs
model q's method m". Belong in python/django.rr as `dj_instance_class(r, h, q)`, `dj_qs_model(e, q)`,
`dj_model_call(c, g)`. `sora_blocking` duplicates django-practice-sync-orm-in-async's `soa_blocking`: a
`dj_query_method(m)` in python/django.rr would serve both.
