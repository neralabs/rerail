# Rerail rule pack: Celery

Rerail's own Celery rules, from the Celery docs and maintainers' review advice, for idempotent, serializable tasks dispatched after commit.

This pack is private while its rules are written and reviewed: it is not published to npm, and is installed from
a checkout with `rerail packs add packs/celery`.

## Rules

| Rule | Mode | Catches |
|---|---|---|
| `celery-practice-countdown-past-visibility-timeout` | Steer | Celery practice: a task delayed past the broker's visibility timeout |
| `celery-practice-dispatch-before-commit` | Steer | Celery practice: a task sent between a SQLAlchemy write and its commit |
| `celery-practice-pickle-serializer` | Steer | Celery practice: Celery set to send or accept pickle messages |
| `celery-practice-result-get-in-task` | Steer | Celery practice: a task waits on another task's result |
| `celery-practice-task-arg-not-serializable` | Steer | Celery practice: a SQLAlchemy model or session, a set, a file, a lambda or a function passed as a Celery task argument |
| `celery-practice-unknown-task-name` | Steer | Celery practice: task named by a string that no task of the run registers |

## Rule details

### `celery-practice-countdown-past-visibility-timeout`

Rerail's own rule, not a port. A worker holds an eta or countdown task unacknowledged until the task runs. Redis and
SQS hand an unacknowledged message to another worker once the visibility timeout passes (Redis: one hour by default,
SQS: 30 minutes), so a task delayed longer runs twice, or "again, and again in a loop".

It matches, at the delay, a value it can prove is longer than the threshold:

- `countdown=` on `.apply_async(...)` and `.send_task(...)`, and on `.retry(...)` called on a bound task's own first
  parameter (`self.retry`, `task.retry`) or on a task;
- `eta=<now> + <timedelta>` on the same calls (either order), `<now>` being `datetime.now()`, `datetime.utcnow()`,
  `datetime.today()`, Django's `timezone.now()`, or a name bound to one of them;
- a task's `default_retry_delay=`, when the task retries with it: it calls `.retry()` without a countdown (in its body
  or a function it runs, through the call graph), or sets `autoretry_for` without `retry_backoff`;
- a task's `retry_backoff_max=`, when it sets `autoretry_for` and `retry_backoff` and its retries reach the cap: the
  backoff before the last retry (`retry_backoff * 2 ** (max_retries - 1)`; `max_retries` from `retry_kwargs`, else the
  option, else Celery's 3; `None` is unbounded) is past the threshold.

The values it reads: int and float literals, `+` and `*` of values (`60 * 60 * 2`), file-local constants and
module-level names (`TWO_HOURS = 2 * 60 * 60`), `timedelta(...)` with readable fields and its `.total_seconds()`,
`min(a, b)` (its smallest readable argument: `min(delay, 86_400)` can wait a day) and `max(a, b)` (any readable
argument). The threshold is the largest literal `visibility_timeout` the run sets outside tests (a
`"visibility_timeout": N` pair or keyword anywhere, `x["visibility_timeout"] = N`, `app.conf.visibility_timeout = N`),
else 3600 seconds.

It leaves alone: values at or under the threshold, and values it cannot read (`countdown=delay`, `settings.X`, another
module's constant); a run that sets `visibility_timeout` to a value it cannot read, or whose
`broker_transport_options` is not a dict literal (read from configuration: the team chose a value); a run whose
broker is known to be RabbitMQ (every URL literal in its `broker_url` settings and `Celery(broker=...)`, the value
itself or an environment default, is `amqp`, `amqps`, `pyamqp` or `librabbitmq`); test code and test settings files;
`.retry()` on other receivers; `default_retry_delay` on a task that never retries with it; `retry_backoff_max` on a
task whose retries never reach it, or without `retry_backoff`.

**Gaps:** a broker chosen by an environment variable with no literal default is taken to be Redis or SQS; on RabbitMQ
the limit is the server's `consumer_timeout` (30 minutes by default since RabbitMQ 3.8.17), which the rule does not
read; SQS's 30-minute default is not used (the rule takes Redis's hour); a countdown that grows over retries with no
cap (`countdown=30 * 2 ** self.request.retries`) is left alone; `.set(countdown=...)` and `options={...}` on
signatures, class attributes of Task subclasses, `self.retry` in a Task subclass's `run`, and `min()`/`max()` inside
arithmetic are not read.

**Overlap:** none. No linter reads Celery's delays or broker settings.

**Corpus:** 2 matches in 15 Celery repositories, both real (100%; too few to measure precision well, so the shapes
rest on the cases). The base corpus has no reportable delay: its long delays sit in runs the rule leaves alone, so six
repositories were cloned for this rule (marked *).

- ecds/readux* (`fcf0d36`): `apps/export/tasks.py:86`, `delete_download_task.apply_async((zipfile_name,),
  countdown=86400)`; the repository configures no broker (Celery's default, or the `CELERY_BROKER_URL` environment
  variable, decides) and no `visibility_timeout`. On Redis the task is redelivered after an hour; on RabbitMQ a day's
  wait passes the 30-minute `consumer_timeout`.
- breatheco-de/apiv2* (`2055837`): `breathecode/authenticate/receivers.py:354`, a Copilot revocation sent with
  `countdown=7200` and `task_acks_late=True`; the broker is CloudAMQP when `CLOUDAMQP_URL` is set and Redis otherwise,
  with `broker_transport_options` that set no `visibility_timeout`. On Redis it is redelivered after an hour; on
  RabbitMQ the same delay passes the 30-minute `consumer_timeout`.
- Silent, as the rule intends: langgenius/dify (`724ac15`; `tasks/knowledge_fs_background_tasks.py:56`
  `self.retry(countdown=min(delay, 86_400))` would be reported, but `extensions/ext_celery.py:111` sets
  `visibility_timeout` from configuration); pretix/pretix (`40ea909`; transport options read from its configuration
  file, and its webhook retries past five minutes go through the database); mozilla/addons-server (`7e3374f`; RabbitMQ
  by its environment default, which silences `abuse/tasks.py:86` `retry_backoff_max=2 * 60 * 60`); lookit/lookit-api*
  (`78562b5`; RabbitMQ by its environment default, which silences `studies/models.py:1950` `countdown=60 * 60 * 24 *
  7`); makeplane/plane (`723bb17`; RabbitMQ); andreynovikov/django-shop* (`2b8dc60`; `visibility_timeout` of 8 days in
  `sewingworld/settings/celery.py:8`, so its one-day countdowns stay silent); indico/indico (`8e64139`;
  `visibility_timeout` of 2 days); UB-Mannheim/escriptorium* (`7e06bd3`) and zenodo/zenodo-rdm* (`b5bf82e`;
  `default_retry_delay=60 * 60`, exactly the hour); paperless-ngx/paperless-ngx (`faa6f09`), saleor/saleor (`e1ccf4d`),
  DefectDojo/django-DefectDojo (`cd0a448`) and apache/superset (`fdf6172`), whose delays are minutes or unreadable.

One miss was found and fixed before these numbers: breatheco-de/apiv2's `BROKER_URL = CLOUDAMQP_URL.replace("amqps://",
"pyamqp://", 1)` first read as a RabbitMQ URL and silenced the run; scheme strings now count only as the broker value
itself or an environment read's default (case `env_broker/`).

**Sources:**
- Celery docs, [Redis broker caveats](https://docs.celeryq.dev/en/stable/getting-started/backends-and-brokers/redis.html#redis-caveats):
  "If a task isn't acknowledged within the Visibility Timeout the task will be redelivered to another worker and
  executed. This causes problems with ETA/countdown/retry tasks where the time to execute exceeds the visibility
  timeout; in fact if that happens it will be executed again, and again in a loop."
- Celery docs, [SQS caveats](https://docs.celeryq.dev/en/stable/getting-started/backends-and-brokers/sqs.html#caveats):
  the same for SQS, whose default visibility timeout is 30 minutes.
- Celery docs, [Calling: ETA and Countdown](https://docs.celeryq.dev/en/stable/userguide/calling.html#eta-and-countdown):
  "tasks are not acknowledged until the worker starts executing them. If using Redis as a broker, task will get
  redelivered when *countdown* exceeds *visibility_timeout*", so eta and countdown are "not recommended for
  scheduling tasks for a distant future. Ideally, use values no longer than several minutes."
- Adam Johnson, [Common Issues Using Celery (And Other Task Queues)](https://adamj.eu/tech/2020/02/03/common-celery-issues-on-django-projects):
  avoid long etas and countdowns.
- pretix's own comment, `src/pretix/api/webhooks.py:545`: retries longer than the one-hour `visibility_timeout` "will
  be restarted many times because celery believes the worker has crashed", so pretix stores them in the database
  instead.

### `celery-practice-dispatch-before-commit`

Rerail's own rule, not a port: the SQLAlchemy (and plain-session) side of `django-practice-enqueue-on-commit`. A
Celery task sent while the session holds uncommitted writes can start on a worker before the commit lands: the worker
finds no row (or the old one), or acts on a change that then rolls back.

A send is `t.delay(...)`/`t.apply_async(...)` on a task, or `<x>.send_task(...)` (`lib/celery`'s `cel_send`), or a
call of a function of the run that sends, directly or through the call graph (`python/calls`' known call targets),
where no function along the way commits before its send; such a call is reported at the call, naming the helper. A
write is `lib/celery`'s `sa_write` on a session: `add`, `add_all`, `delete`, `merge`, `flush`, `bulk_*`, or `execute`
of an `insert`/`update`/`delete` statement. The rule reports a send

1. that follows a write in the same function, with a `commit()` of the same session (by spelling) after it and none
   between the write and the send (statement order: branches of one function count);
2. in the body of `with <x>.begin():`, `with sessionmaker.begin() as s:` or `async with`, after a write in that
   body: the block commits only when it ends;
3. in a function that writes, then sends, and never commits, when a caller (through any number of callers that do
   not commit either) commits after the call or makes it inside a `with <x>.begin():` body. This is dify's
   `DatasetService._update_internal_dataset`: `session.execute(update(Dataset)...)`, `session.flush()`, then
   `deal_dataset_vector_index_task.delay(...)`, with the commit left to the service's callers;
4. in a flush-time SQLAlchemy event listener, `@event.listens_for(X, "<event>")` or `event.listen(X, "<event>", fn)`
   for `before_insert`, `after_insert`, `before_update`, `after_update`, `before_delete`, `after_delete`,
   `before_flush`, `after_flush`, `after_flush_postexec` and `before_commit`: these run inside the flush or before
   the commit.

The message names the send (or the helper) and which of these it is, and tells how to fix it: send after `commit()`,
or from an `after_commit` session event.

It leaves alone: a send after the commit with no write in between; a write after the send; a helper that commits
before its own send; a send in an `after_commit` listener, or in a lambda or nested def (one handed to
`event.listen(session, "after_commit", ...)` runs after the commit); a send in an except handler that always
re-raises (the transaction rolls back, so there is nothing to wait for); `r = t.apply_async(...)` whose result the
function reads afterwards (the task id is stored on the row: the send cannot simply move after the commit, as in the
django rule); `.delay()` on a parameter or local not known to be a task (`throttle.delay(2.0)`); a function that
writes and sends but has no committing caller in the run; test files; and documents that import django (the django
pack's three dispatch rules own those).

**Gaps:** sessions passed across many layers, or a commit on a session object spelled differently from the one
written to; writes through repository objects or by assigning to a loaded model's attributes (the session flushes
them at commit); a transaction a framework opens and commits around the request (Flask-SQLAlchemy teardown,
superset's `@transaction()` decorator, indico's request handler); a caller's commit reached only through a call the
call graph cannot resolve; loops, where a write later in the body precedes the next iteration's send; a flush-time
listener that is a lambda calling a method on an untyped argument (superset's dashboard thumbnail listener).

**Overlap:** none. Ruff, Bandit and pylint have no Celery or SQLAlchemy transaction checks; the django pack's
`enqueue-on-commit`, `dispatch-in-atomic-request` and `task-arg-model-instance` read Django's ORM only.

**Corpus:** 3 matches in 5 repositories, all read, all real (precision 100%, a small sample). langgenius/dify at
`724ac15121ee` (5,191 files): 2, `api/services/knowledge/dataset_service.py:755` and `:758`,
`deal_dataset_vector_index_task.delay` and `regenerate_summary_index_task.delay` after
`session.execute(update(Dataset)...)` and `session.flush()` in `_update_internal_dataset`, reached through
`DatasetService.update_dataset`, whose caller in `services/knowledge/datasets/adapters.py` commits after the call; both
tasks load the dataset row the update changed.
apache/superset at `fdf61721d056` (3,190 files): 1, `superset/models/slice.py:656`, `cache_chart_thumbnail.delay` in
`event_after_chart_changed`, registered for `after_insert` and `after_update` on `Slice`; the task loads the chart by
id and has a "No chart found, skip computing" branch for the uncommitted insert. indico/indico at `8e641393b90c` (1,290
files): 0 (its sends follow attribute changes that the request handler commits, a gap above).
paperless-ngx/paperless-ngx at `faa6f09f289b` and makeplane/plane at `723bb178f97b`: 0, as expected (Django
projects: every document importing django is left to the django pack). A scan of every function in dify and superset
for a session write, a send and a commit in that order found no other site: dify's other service functions commit
before they send (`delete_documents`, `retry_document`, `batch_update_document_status`, the evaluation runs), and
superset's SQL Lab path commits before `.delay()` with a comment about this race.

**Sources:**
- Celery docs, [Tasks: Database transactions](https://docs.celeryq.dev/en/stable/userguide/tasks.html#database-transactions):
  "If the asynchronous task starts executing before the transaction is committed, it may attempt to query the
  article object before it exists. To prevent this, we need to ensure that the transaction is committed before
  triggering the task."
- Adam Johnson, [Common Celery issues on Django projects](https://adamj.eu/tech/2020/02/03/common-celery-issues-on-django-projects/):
  "If you enqueue a task from within a database transaction, it might execute before the database commits the
  transaction."
- BaoDev, [flush vs commit: the sqlalchemy write that isn't saved yet](https://www.baodev.studio/blog/sqlalchemy-flush-vs-commit-difference):
  "flush sends your INSERT to the database. commit makes it real for everybody else"; in the gap a Celery worker
  looked for the flushed order, found nothing, and raised `NoResultFound`.
- Steve Dignam, [The Many Problems with Celery](https://steve.dignam.xyz/2023/05/20/many-problems-with-celery), "No
  transactional job enqueuing": you can't enqueue a job inside a transaction.

### `celery-practice-pickle-serializer`

Rerail's own rule, not a port. A worker that accepts pickle unpickles whatever a message on the broker holds, so anyone
who can write to the broker (a shared Redis, a leaked URL) runs code in every worker that takes the message. Celery 4.0
made JSON the default for that reason.

It matches, at the value that names pickle (`"pickle"` or its MIME type `"application/x-python-serialize"`, a list,
tuple or set holding one, or a module constant of the same file that holds either):

- the settings `task_serializer`, `result_serializer` and `event_serializer` set to pickle, and `accept_content` or
  `result_accept_content` holding it, in every spelling `lib/celery`'s `cel_setting` reads: `app.conf.x = v`,
  `app.conf["x"] = v`, `app.conf.update(x=v)` and `app.conf.update({"x": v})`, `Celery(..., x=v)`, a Django setting
  `CELERY_X = v`, and bare keys in a class or module named for Celery (`class CeleryConfig:`, `celeryconfig.py`);
- a task's own option, `@app.task(serializer="pickle")` or `@shared_task(serializer=...)`, and `serializer =
  "pickle"` in the body of a class deriving (through the run's classes) from celery's `Task`;
- a send's option: `t.apply_async(..., serializer="pickle")` and `app.send_task(..., serializer="pickle")`.

It leaves alone: other serializer names, among them custom signed ones (paperless-ngx registers an HMAC-signed
`"signed-pickle"` and accepts `"application/x-signed-pickle"`); `"json"`, `"msgpack"` and `"yaml"`; every run that
calls `app.setup_security(...)` outside tests (Celery then sends and accepts only signed messages); test code and test
settings files (`test*.py`, `*_test*.py`, and the pack's test paths); a bare `task_serializer =` outside a class or
module named for Celery; `serializer="pickle"` on calls other than `apply_async` and `send_task` (a cache client's
`set(..., serializer=...)`).

**Gaps:** values read from the environment (`os.environ.get("CELERY_TASK_SERIALIZER", "pickle")`) or from another
module's constant; a serializer registered with kombu under another name that wraps pickle; `options={"serializer":
...}`, `.set(serializer=...)` on a signature; a configuration dict passed to `config_from_object`.

**Overlap:** Bandit's B301 and Ruff's S301 (`bandit-b301-pickle` in the python pack) flag `pickle.load(s)` calls only;
no linter reads Celery's configuration.

**Corpus:** 6 matches in 9 Celery repositories, all 6 real (100%). taigaio/taiga-back (`8ab6a4e`, cloned for this
rule because the base corpus has no plain pickle configuration): `settings/common.py:53`, `:54`, `:55` and
`docker/config.py:112`, `:113`, `:114`, `CELERY_ACCEPT_CONTENT = ['pickle', ]`, `CELERY_TASK_SERIALIZER = "pickle"`,
`CELERY_RESULT_SERIALIZER = "pickle"` in the settings it deploys, with no `setup_security()`. No match in
paperless-ngx/paperless-ngx (`faa6f09`; its `"signed-pickle"` settings at `src/paperless/settings/__init__.py:709`,
`:744`, `:746` stay silent), makeplane/plane (`723bb17`), pretix/pretix (`40ea909`), mozilla/addons-server (`7e3374f`),
saleor/saleor (`e1ccf4d`), DefectDojo/django-DefectDojo (`cd0a448`; its task serializer is read from the environment,
default `"json"`), apache/superset (`fdf6172`) or langgenius/dify (`724ac15`): all configure JSON. The other shapes
(conf, `Celery(...)` keywords, config classes and modules, constants, task and send options, Task subclasses) are
proven by the cases only.

**Sources:**
- Celery docs, [Security: Serializers](https://docs.celeryq.dev/en/stable/userguide/security.html#serializers):
  "*pickle* is inherently insecure, and should be avoided whenever clients are untrusted or unauthenticated"; and, on
  message signing, `setup_security()` "will also disable all insecure serializers so that the worker won't accept
  messages with untrusted content types".
- Celery docs, [What's new in Celery 4.0](https://docs.celeryq.dev/en/stable/history/whatsnew-4.0.html): JSON became
  the default serializer, and pickle has to be turned back on explicitly with `task_serializer`, `result_serializer`
  and `accept_content`.
- Celery docs, [Configuration: accept_content](https://docs.celeryq.dev/en/stable/userguide/configuration.html#accept-content):
  the default is `{'json'}`.
- Steve Dignam, [The Many Problems with Celery](https://steve.dignam.xyz/2023/05/20/many-problems-with-celery):
  pickle among Celery's footguns.

### `celery-practice-result-get-in-task`

Rerail's own rule, not a port. A task that waits for another task's result holds its worker pool slot until the
other task finishes. When every slot is held by a waiting task, nothing is left to run the tasks they wait for, and the
pool deadlocks. Celery guards against it at run time: `AsyncResult.get()`/`wait()` and `ResultSet.join()`/
`join_native()` call `assert_will_not_block()`, which raises `RuntimeError("Never call result.get() within a task!")`
in the worker unless the call passes `disable_sync_subtasks=False` or runs inside `with allow_join_result():`. The
same code runs in a shell, a test with eager tasks, or a web process, so it fails only in the worker, in production.

It matches, at the wait, `.get()`, `.join()`, `.join_native()`, `.wait()` or `.collect()` on a result, when the call is
in a task's body (a `@shared_task`/`@app.task` def, or the `run` method of a class deriving from celery's `Task`) or in
a function a task reaches through the run's call graph, across files. A wait in a helper names the helper and one task
that runs it. A result is:

- `t.delay(...)`, `t.apply_async(...)`, `<x>.send_task(...)`;
- `AsyncResult(id)`, `app.AsyncResult(id)`, `self.AsyncResult(id)`, `celery.result.AsyncResult(id)`, `GroupResult(...)`,
  `ResultSet(...)`, `GroupResult.restore(id)` (also `app.GroupResult.restore`), and the same for a class of the run
  deriving from one of them (pretix's `SafeAsyncResult`);
- a group, chain or chord sent: `group(...)()`, `group(...).apply_async()`, `chord(header)(body)`, `chain(...)()`,
  `(a.s() | b.s()).apply_async()`, and `.delay()`/`.apply_async()` on a signature (`t.s(...)`, `signature(...)`);
- a call of a function of the run that returns one of those (pretix's `get_task_result`);
- a name whose latest plain assignment before the wait is one of those (`r = t.delay(x)` then `r.get()`).

It leaves alone: a wait that passes `disable_sync_subtasks=` anything but `True` (DefectDojo's
`dojo/celery_dispatch.py:141` does so deliberately, on an eager result); a wait inside `with allow_join_result():`, and a
helper a task reaches only through calls made inside such a block; `.get()` on anything not known to be a result
(dicts, caches, querysets, `options.get(...)`, a name last rebound to something else); status reads (`ready()`,
`state`, `successful()`, `id`, `completed_count()`); `t.apply(...)`, whose EagerResult never blocks; code no task
reaches (a view polling a result is fine); tasks and waits in test files. A function reached from both web code and a
task is reported: the task's path blocks.

**Gaps:** results passed in as parameters, kept in attributes or containers (`for r in results: r.get()`); calls the
call graph cannot resolve; a project with `task_always_eager` on, where the wait does not block, is reported all the
same.

**Overlap:** none. No linter knows which functions run in a Celery worker.

**Corpus:** CORPUS

**Sources:**
- Celery docs, [Tasks: Avoid launching synchronous subtasks](https://docs.celeryq.dev/en/stable/userguide/tasks.html#avoid-launching-synchronous-subtasks):
  waiting on another task's result in a task "is very inefficient, and may even cause a deadlock if the worker pool is
  exhausted"; make the steps asynchronous with a chain or callbacks instead.
- Celery source, [`celery/result.py`](https://github.com/celery/celery/blob/main/celery/result.py): `E_WOULDBLOCK =
  "Never call result.get() within a task!"`, `assert_will_not_block()`, `allow_join_result()`, and the
  `disable_sync_subtasks` argument of `get()`/`join()`.
- Celery API reference, [`celery.result`](https://docs.celeryq.dev/en/stable/reference/celery.result.html):
  `disable_sync_subtasks` — "Disable tasks to wait for sub tasks this is the default configuration. CAUTION do not
  enable this unless you must."

### `celery-practice-task-arg-not-serializable`

Rerail's own rule, not a port. Celery puts a task's arguments into the message with JSON by default, and kombu's
encoder adds only datetime, date, time, Decimal, UUID and bytes (and objects with a `__json__` method). Any other
object makes the send raise EncodeError. With pickle, a SQLAlchemy model arrives in the worker as a detached snapshot
that is stale by the time the task runs, and saving it there overwrites newer changes.

It matches an argument the task receives (`t.delay(x)`, `t.s(x)`, `t.si(x)`, positional or keyword;
`t.apply_async((x,))`, `t.apply_async(args=[x])`, `t.apply_async(kwargs={"k": x})`; `app.send_task("n", args=[x])`)
where `t` resolves through imports to a Celery task, or is a send on a task reference, and the argument is:

- a SQLAlchemy model instance: `M(...)`, `session.get(M, pk)`, `session.scalar(select(M)...)`,
  `M.query...first()`, `session.query(M)...one()`, a name bound to one, a parameter annotated `M`, or a value the type
  program proves is a model. Message: pass the primary key and load the row in the task.
- a SQLAlchemy session: `session`, `db_session`, `db.session`, or a parameter annotated `Session`/`AsyncSession`.
  `session_id`, `chat_session` and a `session: str` parameter are not sessions. Message: pass the ids and open a
  session in the task.
- a set or frozenset: a literal, a comprehension, a `set(...)`/`frozenset(...)` call, or a name whose last
  assignment before the send is one. Message: pass `sorted(...)` of it.
- an open file: `open(...)`, a name whose last assignment is one, or the target of `with open(...) as f`. Message:
  pass the path or the contents.
- a lambda, a generator expression, or a function of the run passed by name. Message: pass the data, or a name the
  task maps to the function.

It leaves alone: `order.id` and other attributes, primitives, lists, tuples and dicts, `sorted(s)` and `list(s)`, a
name reassigned to a list before the send; datetime, Decimal, UUID and bytes (kombu round-trips them since 5.3);
`.delay` on a plain undecorated function; files that import django for the model shape (the django pack's
`django-practice-task-arg-model-instance` owns Django sends); sets and functions under a pickle serializer (a task's
`serializer="pickle"`, a send's `serializer="pickle"`, or an app's `task_serializer` naming pickle, `signed-pickle`
too), since pickle carries them; the model shape in a run whose Task subclass overrides `apply_async` and rewrites
`args`/`kwargs` there (indico's `_CelerySAWrapper` sends each model's identity key and re-fetches it in the worker);
test files. Under pickle the model, session, file, lambda and generator shapes are still reported: pickle raises on
the last four, and ships a stale snapshot of the first.

**Gaps:** values whose kind only a type checker knows (an unannotated parameter, a helper whose return the type
program cannot follow); instances inside a list or dict; related objects (`order.customer`); dataclass and Pydantic
instances (left out: kombu honours `__json__`, and projects register their own types); a model class that defines
`__json__` (it encodes, but the snapshot is still stale, so the message stays right); a project that registers `set`
with kombu's `register_type`; a serializer chosen in task routes or a Task subclass's `serializer` attribute.

**Overlap:** none. Ruff, Bandit and mypy do not look at task arguments; `delay` is untyped without celery-types, and
celery-types accepts any object.

**Sources:**
- Celery docs, [Tasks: State](https://docs.celeryq.dev/en/stable/userguide/tasks.html#state): "Another gotcha is Django model objects. They shouldn't be passed on as arguments to tasks. It's almost always better to re-fetch the object from the database when the task is running instead, as using old data may lead to race conditions."
- Deni Bertović, [Celery Best Practices](https://denibertovic.com/posts/celery-best-practices/), No. 7: "You shouldn't pass Database objects (for instance your User model) to a background task because the serialized object might contain stale data. What you want to do is feed the task the User id and have the task ask the database for a fresh User object."
- BaoDev, [flush vs commit](https://www.baodev.studio/blog/sqlalchemy-flush-vs-commit-difference): "pass primitives to your queue anyway, never ORM objects."
- kombu, [`kombu/utils/json.py`](https://github.com/celery/kombu/blob/main/kombu/utils/json.py): the encoder registers datetime, date, time, Decimal and UUID and handles bytes; any other type, sets included, falls through to `json.JSONEncoder.default`, which raises TypeError.

**Corpus:** CORPUS

### `celery-practice-unknown-task-name`

Rerail's own rule, not a port. A Celery message carries only the task's name; the worker looks the name up in its
registry and, for a name no task registered, logs "Received unregistered task of type ... The message has been ignored
and discarded." A beat entry whose `"task"` has a typo, or names a task that was renamed or moved to another module, is
sent on schedule and dropped every time: the job silently never runs, and nothing fails at deploy.

It matches, at the string (or at a module constant of the same file that holds it), a task name that no task of the run
is registered under, when the run owns the name:

- the first argument (or `name=`) of `<x>.send_task(...)`, of celery's `signature(...)` and of `<app>.signature(...)`;
- the `"task"` of a beat schedule entry: a dict with a `"task"` key and a `"schedule"` or `"options"` key (the second
  also covers entries a run lists before it adds their schedules, as paperless-ngx does with crons read from the
  environment).

A task is registered under its `name=` (a literal, or a module constant of the run: dify's `name=OPERATION_TASK`), else
under `<module>.<function>`, which a string also matches by a dotted tail (`tasks.x.f` for a module the run sees as
`api.tasks.x`); a class deriving from celery's `Task` is registered under its `name = "..."` attribute, else
`<module>.<Class>`. The run owns a name when its module part (before the last dot) is a module of the run (a document
whose module name is it, or ends with `.` + it), or when the run registers tasks under explicit dotted names that share
its first segment (superset names its tasks `reports.scheduler`, `reports.prune_log` with `name=`). When a task whose
function has the name's last segment is registered elsewhere (a moved task), the message names it.

It leaves alone: Celery's own and library tasks (`celery.*`, `celery_*`, `django_celery_beat.*`,
`django_celery_results.*`, `djcelery`, `kombu`, `flower`, `redbeat`); names whose module part is not a module of the run
and whose first segment no explicit task name shares (a task of another service: `send_task` is often used exactly for
that); names built at runtime (f-strings, variables, parameters) and constants imported from another module;
commented-out entries; a dict with a `"task"` key but neither `"schedule"` nor `"options"`; test code.

**Gaps:** tasks registered by a decorator wrapper that computes the name, or by `app.task(fn)` called as a function (both
would read as unregistered, so a run that does this needs the rule off); task modules autodiscovered from an installed
package outside the run; whether the worker actually imports the module that defines the task (`include`, `imports`,
`autodiscover_tasks`) is not checked, so a defined task the worker never loads stays silent.

**Overlap:** none. No linter resolves Celery task names across files.

**Corpus:** no match in 8 Celery repositories, and none was due: every task name they use by string resolves to a
registered task (checked by hand against each definition). paperless-ngx/paperless-ngx (`faa6f09`, 9 beat entries in
`src/paperless/settings/custom.py`, listed with `"options"` before their crons are added), makeplane/plane (`723bb17`,
13 beat entries in `apps/api/plane/celery.py`), taigaio/taiga-back (`8ab6a4e`, 3 in `taiga/celery.py`),
DefectDojo/django-DefectDojo (`cd0a448`, the 9 live entries of `dojo/settings/settings.dist.py:1104`-`1167`; the two
commented out are not read), saleor/saleor (`e1ccf4d`, 20 in `saleor/settings.py`), apache/superset (`fdf6172`, the 5
entries of `superset/config.py:2158`-`2181` and 3 in `docker/pythonpath_dev/superset_config.py`, all explicit `name=`
tasks), langgenius/dify (`724ac15`, 25 literal beat entries of `api/extensions/ext_celery.py`, among them the explicit
`human_input_form_timeout.check_and_resume` and `community_telemetry.send_heartbeat`, the `signature(...)` of
`api/services/rag_pipeline/rag_pipeline_task_proxy.py:68`, and `send_task(RETRY_TASK)` and
`send_task(AGENT_TRACE_EXPORT_TASK)` through same-file constants in `api/services/tracing/adapters.py`; its
`"task": OPERATION_TASK` is an imported constant, left alone), and pretix/pretix (`40ea909`, no task named by a string).
Precision has no matches to measure. To show the rule fires on real code, a copy of paperless-ngx with two beat
entries changed was run: `"documents.tasks.train_clasifier"` (a typo) and `"documents.tasks.process_mail_accounts"`
(a task that lives in `paperless_mail.tasks`) were both reported, the second naming
`paperless_mail.tasks.process_mail_accounts`, and nothing else.

**Sources:**
- Celery docs, [Tasks: Names](https://docs.celeryq.dev/en/stable/userguide/tasks.html#names): every task has a unique
  name, generated from the module and function name unless `name=` is given, and the name is what the message carries.
- Celery source, [worker consumer](https://docs.celeryq.dev/en/stable/_modules/celery/worker/consumer/consumer.html):
  `UNKNOWN_TASK_ERROR`, "Received unregistered task of type %s. The message has been ignored and discarded."
- Stack Overflow, [Celery "Received unregistered task of type"](https://stackoverflow.com/questions/30746807) and
  [Python Celery Received unregistered task of type - import app error](https://stackoverflow.com/questions/47161489):
  messages dropped because the name sent does not match the name the worker registered.
