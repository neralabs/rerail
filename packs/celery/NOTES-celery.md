# Celery pack: mining notes

What a reviewer on a Celery + SQLAlchemy (or Celery + Django) team flags on every pull request, checked against the
Celery 5.x docs and source, kombu's changelog, maintainers' issue comments, well-known posts, the `python` pack's Ruff
and Bandit ports, and the 20-repository corpus (`~/rerail-bench/python-arch/corpus`). The django pack already owns the
Django dispatch rules (`django-practice-enqueue-on-commit`, `-dispatch-in-atomic-request`, `-task-arg-model-instance`);
nothing here repeats them.

## 1. Sources read

Docs base: `D = https://docs.celeryq.dev/en/stable` (Celery 5.x "stable").

| Source | URL | What it contributed |
|---|---|---|
| Celery tasks guide, "Database transactions" | D/userguide/tasks.html#database-transactions | A task sent inside a transaction can run before the commit and miss the row. |
| Celery tasks guide, "State" | D/userguide/tasks.html#state | Don't pass ORM objects to tasks; re-fetch by id, since old data leads to race conditions (`expand_abbreviations(article)` vs `article_id`). |
| Celery tasks guide, intro and "Retrying" | D/userguide/tasks.html, #retrying | Tasks should be idempotent; `acks_late` only for idempotent tasks; `retry()` raises `Retry` ("a semi-predicate"); `autoretry_for`, `max_retries` (default 3, `None` = forever), `retry_backoff`, `retry_jitter`. |
| Celery tasks guide, "Avoid launching synchronous subtasks" | D/userguide/tasks.html#avoid-launching-synchronous-subtasks | Waiting on another task's result in a task is inefficient and "may even cause a deadlock if the worker pool is exhausted". |
| Celery tasks guide, "Multiple decorators", "Names" | D/userguide/tasks.html#multiple-decorators, #names | Task decorator must be outermost; a task's name is module + function unless `name=` is given, and the message carries only the name. |
| Celery FAQ, "Should I use retry or acks_late?" | D/faq.html#should-i-use-retry-or-acks-late | acks_late re-runs a task after a worker crash; combine it with retries only for idempotent tasks. |
| Celery security guide, "Serializers" | D/userguide/security.html#serializers | Pickle is "inherently insecure"; JSON default since 4.0; whitelist with `accept_content`. |
| Celery 4.0 "What's new" | D/history/whatsnew-4.0.html | JSON became the default; the settings that opt back into pickle; the new lowercase setting names. |
| Celery configuration reference | D/userguide/configuration.html | `accept_content` default `{'json'}`; `task_time_limit`/`task_soft_time_limit` default "no limit"; `imports`/`include`; timezone/enable_utc. |
| Celery calling guide, "ETA and Countdown" | D/userguide/calling.html#eta-and-countdown | `delay` takes no execution options; eta must be timezone-aware; countdowns past the Redis/SQS visibility timeout are redelivered; keep them to minutes. |
| Celery Redis broker, "Caveats: Visibility timeout" | D/getting-started/backends-and-brokers/redis.html#redis-caveats | ETA/countdown/retry tasks longer than the visibility timeout (1 h default) are "executed again, and again in a loop". |
| Celery SQS broker, "Caveats" | D/getting-started/backends-and-brokers/sqs.html#caveats | The same caveat; SQS default 30 min. |
| Celery source `celery/result.py` | https://github.com/celery/celery/blob/main/celery/result.py | `E_WOULDBLOCK = "Never call result.get() within a task!"`, `assert_will_not_block()` raises RuntimeError; `allow_join_result()`, `disable_sync_subtasks`. |
| Celery source `celery/exceptions.py` | https://github.com/celery/celery/blob/main/celery/exceptions.py | `Retry(TaskPredicate)` → `CeleryError(Exception)`: `except Exception` catches `Retry`. |
| Celery source `celery/worker/request.py`, `celery/app/amqp.py` | https://github.com/celery/celery/tree/main/celery | Naive eta is read in `app.conf.timezone`; `apply_async` refuses non-tuple args with TypeError. |
| Celery worker consumer source | D/_modules/celery/worker/consumer/consumer.html | "Received unregistered task of type %s. The message has been ignored and discarded." |
| Celery issue #4898 | https://github.com/celery/celery/issues/4898 | `autoretry_for=(Exception,)` with `raise self.retry()` ran a task 2047 times: `Retry` is an `Exception`. |
| Celery issues #6552, #3884 | https://github.com/celery/celery/issues/6552, https://github.com/celery/celery/issues/3884 | `async def` tasks are not supported (maintainer: "not currently supported"). |
| kombu `kombu/utils/json.py` and Changelog | https://github.com/celery/kombu/blob/main/kombu/utils/json.py | kombu ≥ 5.3 round-trips datetime, date, time, Decimal, UUID and bytes through JSON; sets are never registered (TypeError). Before 5.3 datetime/Decimal/UUID arrive as strings. |
| Adam Johnson, "Common Issues Using Celery (And Other Task Queues)" | https://adamj.eu/tech/2020/02/03/common-celery-issues-on-django-projects | Enqueue inside a transaction → DoesNotExist; late acks need idempotency; avoid long eta/countdown. |
| Steve Dignam, "The Many Problems with Celery" | https://steve.dignam.xyz/2023/05/20/many-problems-with-celery | No transactional enqueue, retry defaults without backoff, no asyncio, pickle risk. |
| Hatchet, "The problems with (Python's) Celery" | https://docs.hatchet.run/blog/problems-with-celery | No async-function tasks out of the box. |
| Deni Bertović, "Celery – Best Practices" | https://denibertovic.com/posts/celery-best-practices | No. 7: don't pass ORM objects to tasks. |
| Olzhas Arystanov, "Celery best practices" | https://olzhasar.com/posts/celery-best-practices | acks_late tasks must be safe to run twice; set timeouts on I/O. |
| eghuro, "Celery retry is an exception" | https://www.eghuro.cz/posts/celery-retry-is-an-exception | `except Exception` around `self.retry()` swallows the Retry after the retry message is already queued; re-raise `TaskPredicate` first. |
| BaoDev, "SQLAlchemy flush vs commit" | https://www.baodev.studio/blog/sqlalchemy-flush-vs-commit-difference | `session.flush(); task.delay(order.id)` → the worker sees no row; dispatch from `after_commit`. |
| Ines Panker, "Celery explained" | https://www.ines-panker.com/2020/10/28/celery-explained.html | What acks_late changes; redelivery after a crash. |
| Stack Overflow 47161489, 30746807, 67845987, 72529998 | https://stackoverflow.com/questions/47161489 … | Unregistered-task reports (relative imports, missing module); a beat `args` string split into characters; async-task workarounds. |
| pylint-celery | https://github.com/pylint-dev/pylint-celery | Suppresses one false positive; performs no checks. |
| Ruff rules index; `packs/python/pack.json` | https://docs.astral.sh/ruff/rules/ | No Celery-aware rule in Ruff, Bandit, pylint or a flake8 plugin. Nearest generic ports in the python pack: `bandit-b301-pickle` (pickle calls only), `bandit-b113-request-without-timeout`, `ruff-dtz00*` (naive datetimes), `bandit-b110-try-except-pass`, `ruff-e722-bare-except`. BLE001 (blind except) is not ported. |

## 2. Candidates

Corpus probes: `rg` over the eight Celery repositories (dify, superset, plane, saleor, pretix, addons-server,
paperless-ngx, DefectDojo) and the library probe (section 4).

| Id | Catches | Why it is a bug | Sources | Scope | Linter overlap | Engine detection | Corpus | Verdict |
|---|---|---|---|---|---|---|---|---|
| dispatch-before-commit | `.delay()`/`.apply_async()`/`send_task()` after a SQLAlchemy session write (add, flush, execute of insert/update/delete) and before that function's `commit()`, inside `with session.begin():`, through a helper that dispatches, or from a flush-time mapper/session event listener | The worker can run before the commit: it reads no row or the old row, or acts on a change that rolls back | Celery "Database transactions"; Adam Johnson; BaoDev; Dignam | cross-file (helpers through the call graph, tasks imported) | none | lib: sa_write, sa_commit, sa_tx_with, cel_send; `earlier`, `reach` | dify `services/knowledge/dataset_service.py:755` dispatches after `session.flush()` with the commit left to the caller; many dify service functions | **build** |
| task-arg-not-serializable | A SQLAlchemy model instance, a session, a set, an open file, a lambda/function or a generator passed as a task argument | JSON (the default) raises EncodeError at the call; with pickle the worker gets a detached, stale snapshot | Celery "State"; Bertović No. 7; kombu json.py (sets unregistered) | cross-file (task and model defs in other modules) | none (django pack covers Django models only) | lib: cel_task_arg, sa_instance; value kinds | probe: needs the corpus run; dify passes ids by convention | **build** |
| unknown-task-name | `send_task("x.y")`, `signature("x.y")` or a beat entry `"task": "x.y"` naming no task of the run, when the name's module part is a module of the run | The worker discards it as unregistered; a beat job silently never runs | Celery "Names"; consumer source; SO 30746807 | cross-file | none | lib: cel_name_ref, cel_task_named | DefectDojo 9 beat entries, superset 5, dify ~15 | **build** |
| result-get-in-task | `.get()`/`.join()` on an AsyncResult (from `delay`, `apply_async`, `AsyncResult(...)`, a group) inside a task or a function a task reaches | Blocks a pool slot on another task: deadlock when the pool is exhausted; Celery raises RuntimeError in the worker unless overridden | Tasks guide; result.py | cross-file (call graph) | none | lib: cel_task_runs; result-producing calls | DefectDojo uses `disable_sync_subtasks=False` deliberately (left alone) | **build** |
| retry-swallowed | A task with `autoretry_for`, or calling `self.retry()`, whose body catches `Exception`/`BaseException`/bare except around it without re-raising | `Retry` is an Exception: the handler eats it (retry queued but the task reports success, or autoretry never fires) | celery/exceptions.py; issue #4898; eghuro | single-file (task and handler) | `ruff-e722`, `bandit-b110` flag the except itself, not the retry it defeats | lib: cel_task, cel_task_option; except clauses | probe needed | **build** |
| pickle-serializer | `task_serializer`/`result_serializer`/`accept_content` naming pickle (any spelling: conf, Django `CELERY_*`, config class) | A message from anyone who can write to the broker runs code in the worker | Security guide; whatsnew 4.0; config reference | single-file config, any settings spelling | `bandit-b301` flags `pickle.loads` calls only | lib: cel_setting | paperless uses its own HMAC "signed-pickle" (left alone) | **build** |
| countdown-past-visibility-timeout | `countdown`/`eta`/`retry(countdown=)`/`default_retry_delay` known to exceed the broker's visibility timeout (1 h Redis, 30 min SQS) when the run sets no larger `visibility_timeout` | Redis/SQS redeliver the unacked message: the task runs twice or loops | Redis and SQS caveats; calling guide; Adam Johnson; pretix's own comment (`api/webhooks.py:545`) | cross-file (broker settings elsewhere) | none | lib: cel_setting; constant folding of literals, `min()`, `timedelta`, products | dify `knowledge_fs_background_tasks.py:56` `retry(countdown=min(delay, 86_400))` (dify sets visibility_timeout from config) | **build** |
| retry-non-idempotent | A retrying task (`acks_late=True`, `autoretry_for`, `self.retry`) that sends mail, posts a webhook, charges or increments a counter before a later step that can fail | The retry or redelivery repeats the effect: duplicate mail, double charge | Tasks guide; FAQ; Adam Johnson; Arystanov | cross-file (effects in helpers) | none | lib: cel_task_flag, cel_task_runs | pretix mail/webhook tasks with acks_late; dify IM tasks with acks_late | **build** (expect log) |
| network-task-without-time-limit | A task that reaches network I/O (requests, httpx, urllib, smtplib, boto3) with no `time_limit`/`soft_time_limit` on the task and no `task_time_limit` in the app's settings | A hung connection holds a pool slot forever; enough of them stop the worker | Tasks guide warning; config reference; Arystanov | cross-file (helpers, settings) | `bandit-b113` checks a requests call's timeout only | lib: cel_task_runs, cel_setting | most webhook tasks in plane/saleor/dify | **build** (expect log) |
| async-def-task | `@app.task`/`@shared_task` on an `async def` | Celery calls it and gets an unawaited coroutine: the body never runs, the task "succeeds" | Issues #6552, #3884; Hatchet; Dignam | single-file | none (`ruff-ruf006` is about tasks of asyncio) | lib: cel_task | none in corpus | **build**: cheap, exact, silent failure; FastAPI teams hit it (SO 72529998) |
| delay-execution-options | `t.delay(x, countdown=10)` where the task has no such parameter | The option becomes a task kwarg: TypeError in the caller (task typing) or silently ignored | Calling guide | cross-file | mypy with `celery-types` stubs types `delay` by the task's ParamSpec | cel_task_call + params | plane's two `.delay(countdown=...)` are real task parameters | drop: no real occurrence, covered by mypy + celery-types |
| task-signature-mismatch | `.delay()` arguments that don't fit the task's parameters | TypeError | Calling guide | cross-file | mypy + celery-types; Celery checks at call time (`Task.typing`) | — | none | drop: typed by celery-types, raised at the call by Celery |
| task-decorator-order | `@other` above `@app.task` | The worker registers the undecorated function, or `.delay` is lost | Tasks guide "Multiple decorators" | single-file | none | lib: decorator order | DefectDojo `management/commands/test_celery_decorator.py:78` is a deliberate demonstration only | drop: no real occurrence; the wrong order fails loudly the first time |
| task-module-not-registered | A task defined in a module no `imports`/`include`/autodiscovery reaches | Unregistered task on the worker | Consumer source; SO 47161489 | cross-file | none | would need the worker's import closure | dify lists 25 modules in `imports` while ~45 task modules load through the Flask app's own imports | drop: which modules the worker imports is not knowable statically; the module-list check would be noise in dify |
| eta-naive-datetime | `eta=datetime.now()`/`utcnow()` | Naive eta read in the worker's configured timezone | Calling guide; request.py | single-file | `ruff-dtz001/003/005` flag the naive construction itself | — | none | drop: the DTZ ports already catch it at construction |
| retry-not-raised | `self.retry(...)` without `raise` | — | Tasks guide: `retry()` raises unless `throw=False` | — | — | — | pretix uses bare `self.retry()` 13 times, correctly | drop: not a bug (style only) |
| unbounded-retries | `max_retries=None`, `autoretry_for=(Exception,)` without backoff | Retry storm | Tasks guide; Dignam | single-file | none | — | superset `tasks/scheduler.py:396`, dify `knowledge_fs_background_tasks.py:35`: deliberate, with capped countdowns | drop: deliberate in every corpus case; the harmful part (side effects repeated) is retry-non-idempotent |
| large-payload-arg | File contents, base64 blobs, whole query results as task arguments | Broker memory, slow messages | Tasks guide "State"; Bertović | single-file | none | no reliable size signal | dify `workflow_based_app_execution_task.delay(payload_json)` is a deliberate JSON payload; no `.read()`/`b64encode` arguments found | drop: no static signal that separates large from fine |
| json-loses-datetime-decimal | datetime/Decimal/UUID arguments under JSON | Arrive as strings | whatsnew 4.0 (stale) | — | — | — | — | drop: kombu ≥ 5.3 round-trips them; sets folded into task-arg-not-serializable |
| apply-async-args-not-tuple | `apply_async(("abc"))` | Characters as arguments | SO 67845987 | single-file | none | — | none | drop: current Celery raises TypeError at the call; the character split happens only for a beat entry's string `args` |
| always-eager-in-production | `task_always_eager = True` outside test settings | Tasks run inside the request | Config reference | single-file | none | lib: cel_setting | only in test settings (addons-server `settings_test.py:82`) | drop: no occurrence |

Placement: dispatch-before-commit spans Celery and SQLAlchemy; it lives here, as the brief says, because a Celery
user expects it with the task rules. It reports Django ORM code only where the django pack's rules do not reach
(it skips documents that import django).

## 3. Build list, ranked

1. **`celery-practice-dispatch-before-commit`**: data safety, critical (`tx.side-effect-before-commit`).
2. **`celery-practice-pickle-serializer`**: security, critical.
3. **`celery-practice-countdown-past-visibility-timeout`**: data safety, high (duplicate execution).
4. **`celery-practice-task-arg-not-serializable`**: correctness/data safety, high (EncodeError or stale overwrite).
5. **`celery-practice-unknown-task-name`**: correctness, high (a scheduled job that never runs).
6. **`celery-practice-result-get-in-task`**: availability, high (deadlocked or failing worker).
7. **`celery-practice-retry-swallowed`**: correctness, high (failures reported as success, retries lost).
8. **`celery-practice-retry-non-idempotent`**: data safety, medium (duplicate side effects; expected log).
9. **`celery-practice-network-task-without-time-limit`**: availability, medium (expected log).
10. **`celery-practice-async-def-task`**: correctness, high, rare.

## 4. Library proof (`rules/lib/celery.rerail`)

A probe rule (one finding per relation, deleted after use) over a case repository (apps, tasks with options, invoke's
`@task`, sends, signatures, `send_task`, beat entries, every settings spelling, DeclarativeBase / `declarative_base()` /
`db.Model` / SQLModel models in the same and other modules, model instances from constructors, `session.get`,
`session.scalar(select(M))`, `M.query...first()`) reported each relation where expected and nowhere else. Two fixes
came from it: a class deriving from DeclarativeBase is a base, not a model; and a base that is a module variable of the
same module (`db.Model`, `Base = declarative_base()`), which `class_base` does not resolve, is read through `cel_sym`.

On paperless-ngx (`faa6f09`, 424 files) the probe found 26 tasks with the names the worker registers
(`documents.tasks.consume_file`, `documents.workflows.webhooks.send_webhook`, ...), 29 sends, 46 task calls, 15 task
flags, and the Django-namespace settings at `src/paperless/settings/__init__.py:705`-`749` (`imports`, `broker_url`,
`result_serializer`, `broker_transport_options`, `task_time_limit`, `task_serializer`, `accept_content`,
`beat_schedule`). The writers' own corpus runs exercise the SQLAlchemy relations on dify and superset.

## 5. Corpus results

Filled in from the writers' reports at integration.
