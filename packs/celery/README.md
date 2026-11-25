# Rerail rule pack: Celery

Rerail's own Celery rules, from the Celery docs and maintainers' review advice, for idempotent, serializable tasks dispatched after commit.

This pack is private while its rules are written and reviewed: it is not published to npm, and is installed from
a checkout with `rerail packs add packs/celery`.

## Rules

In the order a team should read them: what it loses when the rule is missing comes first.

| Rule | Mode | Catches | Corpus matches (repos) | Read precision |
|---|---|---|---|---|
| `celery-practice-dispatch-before-commit` | Steer | Celery practice: a task sent between a SQLAlchemy write and its commit | 3 (2) | 100% (3 of 3) |
| `celery-practice-pickle-serializer` | Steer | Celery practice: Celery set to send or accept pickle messages | 9 (2) | 100% (9 of 9) |
| `celery-practice-countdown-past-visibility-timeout` | Steer | Celery practice: a task delayed past the broker's redelivery limit | 4 (4) | 100% (4 of 4) |
| `celery-practice-result-get-in-task` | Steer | Celery practice: a task waits on another task's result | 0 | no match (cases and counterexamples only) |
| `celery-practice-task-arg-not-serializable` | Steer | Celery practice: a SQLAlchemy model or session, a set, a file, a lambda or a function passed as a Celery task argument | 0 | no match (cases and counterexamples only) |
| `celery-practice-unknown-task-name` | Log, starts off | Celery practice: task named by a string that no task of the run registers | 0 | no match (cases and counterexamples only) |

## Rule details

### `celery-practice-dispatch-before-commit`

Rerail's own rule, not a port: the SQLAlchemy (and plain-session) side of `django-practice-enqueue-on-commit`. A
Celery task sent while the session holds uncommitted writes can start on a worker before the commit lands: the worker
finds no row (or the old one), or acts on a change that then rolls back.

A send is `t.delay(...)`/`t.apply_async(...)` on a Celery task, resolved through imports (`lib/celery`'s
`cel_task_ref`: a task def of the run, also under an alias or a re-export, a task object such as `app.task(fn)` or a
`Task` subclass instance, or a signature, `t.s(...)` or `signature("name")`), `<x>.send_task(...)`, or a call of a
function of the run that sends, directly or through the call graph, where no function along the way commits before
its send; such a call is reported at the call, naming the helper. RQ's `@job`, a thread pool and any other object
with a `delay` method are not sends. A write is a session call (`add`, `add_all`, `delete`, `merge`, `flush`,
`bulk_*`, `execute` of an `insert`/`update`/`delete`), or an assignment to a mapped column of a row the function loaded
through that session (`s.get(M, pk)`, `s.query(M)...first()`, `s.scalar(select(M))`, `s.scalars(select(M))`, a `for`
over them; columns resolved through imports and base classes: `mapped_column`, `Column`, `relationship`, SQLModel's
`Field`, `Mapped[...]`), which the session flushes at commit. A session is `lib/celery`'s `sa_session`: proven by what
binds it (a session class's constructor, a `sessionmaker`/`scoped_session`, Flask-SQLAlchemy's `db.session`, a run
function returning one, an annotation), else an unannotated name ending in `session` in a run that imports
SQLAlchemy. A write and a send pair only when one path runs both in that order: not in different arms of one
`if`/`elif`/`else`, `match`, or `try`'s `except`/`else`, and not past a `return` or `raise` that ends the write's
block. The rule reports a send

1. that follows a write in the same function, with a `commit()` of the same session after it and no commit between
   them on that path;
2. in the body of `with <x>.begin():`, `with sessionmaker.begin() as s:` or `async with`, after a write in that
   body on the block's session: the block commits only when it ends;
3. in a function that writes, then sends, and never commits, when a caller (through any number of callers that do
   not commit either) commits the same session after the call, or makes the call inside a `begin()` block on it. The
   session is followed from the callee's parameter to the caller's argument (by keyword, position, and the `self`
   offset of a bound method). This is dify's `DatasetService._update_internal_dataset`:
   `session.execute(update(Dataset)...)`, `session.flush()`, then `deal_dataset_vector_index_task.delay(...)`, with
   the commit left to the caller's `with Session(...)` block;
4. in a flush-time SQLAlchemy event listener, `@event.listens_for(X, "<event>")` or `event.listen(X, "<event>", fn)`
   for `before_insert`, `after_insert`, `before_update`, `after_update`, `before_delete`, `after_delete`,
   `before_flush`, `after_flush`, `after_flush_postexec` and `before_commit`: these run inside the flush or before
   the commit.

The message names the send (or the helper) and which of these it is, and tells how to fix it: send after `commit()`,
or from an `after_commit` event, and to leave it when the task reads nothing written there.

It leaves alone: a send after the commit with no write in between; a write after the send; a write and a send in
sibling branches (`if is_new: session.add(o)` / `else: ship.delay(o.id)`); a helper that commits before its own send;
a send in an `after_commit` listener, or in a lambda or nested def; a send in an except handler that always re-raises;
`r = t.apply_async(...)` whose result the function reads afterwards (the task id is stored on the row); `.delay()` on
anything not resolved to a Celery task; a caller that commits a different session than the one the helper wrote; a
name bound to an HTTP client (`requests.Session()`, an `aiohttp.ClientSession` annotation); test files; and documents
that import django (the django pack's three dispatch rules own those).

**Gaps:** loops, where a write later in the body precedes the next iteration's send; writes through repository
objects, or to a row loaded elsewhere and passed in; a Flask `Model.query.get()` row (no session spelled); a
transaction a framework opens and commits around the request (Flask-SQLAlchemy teardown, superset's `@transaction()`
decorator, indico's request handler); a caller's commit reached only through a call the call graph cannot resolve; a
commit in only some arms between the write and the send counts as a commit between them; an unannotated parameter
named like a session (`http_session`) is taken for a SQLAlchemy session, the fallback where nothing resolves.

**Overlap:** none. Ruff, Bandit and pylint have no Celery or SQLAlchemy transaction checks; the django pack's
`enqueue-on-commit`, `dispatch-in-atomic-request` and `task-arg-model-instance` read Django's ORM only.

**Corpus:** 3 matches, all read, all real (precision 3/3, a small sample). Re-measured after the rework with the final
rule (runs `celery-final-*`, on the Junction corpus boxes, which take repositories under 1,000 .py files):
Netflix/lemur (`3a3c2c2`, 337 files), flaskbb/flaskbb (`63afaa4`, 242), zenodo/zenodo-rdm (`b5bf82e`, 264),
makeplane/plane (`723bb17`, 784) and Buuntu/fastapi-react (`af17f2d`, 30): 0 findings. Not re-run (5,000 and 3,000
files, over the boxes' limit), with the review's figures: langgenius/dify at `724ac15121ee`: 2,
`api/services/knowledge/dataset_service.py:755` and `:758`, `deal_dataset_vector_index_task.delay` and
`regenerate_summary_index_task.delay` after `session.execute(update(Dataset)...)` and `session.flush()` in
`_update_internal_dataset`, whose caller's `with Session(...)` block commits after the call; both tasks load the
dataset row the update changed. apache/superset at `fdf61721d056`: 1, `superset/models/slice.py:656`,
`cache_chart_thumbnail.delay` in `event_after_chart_changed`, registered for `after_insert` and `after_update` on
`Slice`. Both stayed reported in the rule writer's run of the first rework (path arms, `celery-fix-dbc-*`), before
the engine moved off the laptop. On the review's counterexamples (`celery-final-cx-dbc`): 5 findings, 4 real
(`flaskapp.py:11`, `orders.py:32` the attribute write, newly caught, `:54`, `:63`); the fifth is the contrived
unannotated `http_session` (`orders.py:24`), the name fallback; the review's if/else false positive, RQ and the pool
are silent (review: 6 findings, 3 real).

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
- a send's option on a Celery task or any `send_task`: `t.apply_async(..., serializer="pickle")` (`t` resolving to a
  Celery task) and `app.send_task(..., serializer="pickle")`.

It leaves alone: other serializer names, among them custom signed ones (paperless-ngx registers an HMAC-signed
`"signed-pickle"` and accepts `"application/x-signed-pickle"`); `"json"`, `"msgpack"` and `"yaml"`; every run that
calls `app.setup_security(...)` outside tests (Celery then sends and accepts only signed messages); test code and test
settings files (`test*.py`, `*_test*.py`, and the pack's test paths); a bare `task_serializer =` outside a class or
module named for Celery; `serializer="pickle"` on calls that are not Celery sends (a cache client's
`set(..., serializer=...)`, a multiprocessing pool's or another library's `apply_async`, kombu's `publish`).

**Gaps:** values read from the environment (`os.environ.get("CELERY_TASK_SERIALIZER", "pickle")`) or from another
module's constant; a serializer registered with kombu under another name that wraps pickle; `options={"serializer":
...}`, `.set(serializer=...)` on a signature; a configuration dict passed to `config_from_object`, and
`app.conf.update(**{...})`.

**Overlap:** Bandit's B301 and Ruff's S301 (`bandit-b301-pickle` in the python pack) flag `pickle.load(s)` calls only;
no linter reads Celery's configuration.

**Corpus:** 9 matches in 2 of 20 Celery repositories, all 9 real (100%). taigaio/taiga-back (`8ab6a4e`, 933 files,
cloned for this rule): `settings/common.py:53`, `:54`, `:55` and `docker/config.py:112`, `:113`, `:114`,
`CELERY_ACCEPT_CONTENT = ['pickle', ]`, `CELERY_TASK_SERIALIZER = "pickle"`, `CELERY_RESULT_SERIALIZER = "pickle"`
in the settings it deploys, with no `setup_security()` (the same six with the reworked library, run
`celery-fix-three-taigaio__taiga-back`). indico/indico (`8e64139`, 1,301 files, not re-run; the review's figures):
`indico/core/celery/core.py:66`, `:67`, `:68`, `result_serializer`, `task_serializer` and `accept_content` set to
pickle on purpose ("that way we can pass along all types"), still the exposure the security guide names. No match in
paperless-ngx/paperless-ngx (its `"signed-pickle"` settings stay silent), makeplane/plane, pretix/pretix,
mozilla/addons-server, Netflix/lemur, flaskbb/flaskbb, zenodo/zenodo-rdm and Buuntu/fastapi-react (re-run after the
rework), nor in saleor, DefectDojo (serializer from the environment, default `"json"`), superset or dify (review).
Both repositories with matches pass objects to tasks by design, so `setup_security()` is the realistic fix there.
On the review's counterexamples (`celery-final2-cx-pickle`): 1 finding, `settings/dev.py:2` (pickle in a development
settings file, debatable as the review says); the pool's `apply_async(serializer="pickle")` false positive is gone,
and the spellings the review listed (dict passed to `config_from_object`, environment defaults, imported constants,
`**{...}` updates, `.set(serializer=...)`) stay documented gaps.

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

### `celery-practice-countdown-past-visibility-timeout`

Rerail's own rule, not a port. A worker holds an eta or countdown task unacknowledged until the task runs. Redis and
SQS hand an unacknowledged message to another worker once the visibility timeout passes (Redis: one hour by default,
SQS: 30 minutes), so a task delayed longer runs twice, or "again, and again in a loop". RabbitMQ (3.8.17 and later)
closes the channel of a consumer that holds a message past its `consumer_timeout` (30 minutes by default), and the
worker fails with `PreconditionFailed`.

It matches, at the delay, a value it can prove is longer than the broker's limit:

- `countdown=` and `eta=` on calls resolved to a Celery task, never by method name alone: `.apply_async(...)`,
  `.set(...)` and `.signature(...)` on a task or signature (`lib/celery`'s `cel_task_ref`, which also takes a Celery
  wrapper package's `@task` imported beside celery, as breatheco-de/apiv2's `task_manager`), `.send_task(...)`,
  `celery.signature(...)`, and `.retry(...)` on a bound task's own first parameter (`self.retry`) or on a task;
- `eta=` the current time plus a timedelta: `datetime.now()`, `utcnow()`, `today()`, Django's `timezone.now()` or
  `timezone.localtime()` (resolved through imports), with `+`/`-` timedeltas in any order and chained;
- a task's `default_retry_delay=`, when the task retries with it: it calls `.retry()` without a countdown (in its body
  or a function it runs, through the call graph), or sets `autoretry_for` without `retry_backoff`;
- a task's `retry_backoff_max=`, when it sets `autoretry_for` and `retry_backoff` and its retries reach the cap: the
  backoff before the last retry (`retry_backoff * 2 ** (max_retries - 1)`; `max_retries` from `retry_kwargs`, else the
  option, else Celery's 3; `None` is unbounded) is past the limit.

The values it reads: int and float literals, `+` and `*` of values, `timedelta(...)` with readable fields and its
`.total_seconds()`, `min(a, b)` (its smallest readable argument) and `max(a, b)` (any readable argument), and names,
resolved by scope (module, enclosing function, own function) through every assignment the name may still hold:
`TWO_HOURS = 2 * 60 * 60`, a local `later = timezone.now() + timedelta(days=1)` passed as `eta=later`, `step = 600`
then `if urgent: step = 7200`, a decorator factory's local `max_retries = 60`.

The limit follows the run's broker (the schemes of every broker URL literal in its `broker_url` settings and
`Celery(broker=...)`, the value itself or an environment default; the schemes are Kombu's transport names):

- RabbitMQ (`amqp`, `amqps`, `pyamqp`, `librabbitmq` only): the largest `"x-consumer-timeout"` queue argument the run
  sets, else 1800 seconds, RabbitMQ's default `consumer_timeout`. The message says the channel closes, and offers
  raising `consumer_timeout` in `rabbitmq.conf` as the alternative.
- Otherwise: the largest literal `visibility_timeout` the run sets outside tests, else 1800 seconds on SQS, else 3600
  seconds (Redis, mixed brokers, or a broker it cannot name). The message says the task is redelivered and that a
  longer visibility timeout also delays the recovery of tasks a crashed worker held.

It leaves alone: values at or under the limit, and values it cannot read (`countdown=delay` of a parameter,
`settings.X`, another module's constant, a loop variable, a binding replaced before the end of its scope);
`.apply_async`, `.set` and `.retry` on receivers that are not Celery tasks (a pool's `apply_async`, a client's
`retry`, invoke's `@task` in a module without celery); a run that sets the limit (`visibility_timeout`, or
`x-consumer-timeout` on RabbitMQ) to a value it cannot read, or a non-RabbitMQ run whose `broker_transport_options`
is not a dict (read from configuration: the team chose a value); test code and test settings files;
`default_retry_delay` on a task that never retries with it; `retry_backoff_max` on a task whose retries never reach
it.

**Gaps:** a broker chosen by an environment variable with no default is taken for Redis, so a delay between 30 and
60 minutes on such a RabbitMQ or SQS run is missed; a `consumer_timeout` raised in `rabbitmq.conf` or a policy,
outside the repository, is not seen (the message names it); a countdown that grows over retries with no cap
(`countdown=30 * 2 ** self.request.retries`) is left alone; `options={...}` on signatures, Task subclasses' class
attributes, `self.apply_async`/`self.retry` in a Task subclass's methods, constants built from other constants
(`DAY = 24 * HOUR`) and `min()`/`max()` inside arithmetic are not read.

**Overlap:** none. No linter reads Celery's delays or broker settings.

**Corpus:** 4 matches in 15 Celery repositories, all read, all real (4/4; the review had 2/2 and two misses). Six
repositories were cloned for this rule (marked *). Runs `celery-final-*` used the final rule on the Junction corpus
boxes (repositories under 1,000 .py files); `celery-fix-countdown4-*` are the rule writer's runs of the same rework
before the engine moved off the laptop, on the larger ones.

- ecds/readux* (`fcf0d36`, final run): `apps/export/tasks.py:86`, `delete_download_task.apply_async((zipfile_name,),
  countdown=86400)`; no broker and no `visibility_timeout` configured, so the 3600-second limit applies.
- lookit/lookit-api* (`78562b5`, final run): `studies/models.py:1950`, `countdown=60 * 60 * 24 * 7` with the broker's
  environment default `amqp://`: past RabbitMQ's 1800-second `consumer_timeout`. A review miss, now caught.
- mozilla/addons-server (`7e3374f`, 1,426 files, writer's run): `src/olympia/abuse/tasks.py:86`,
  `retry_backoff_max=2 * 60 * 60` with `retry_backoff=30` and the decorator factory's `max_retries = 60`, broker
  default `amqp://`. A review miss, now caught.
- breatheco-de/apiv2* (`2055837`, 2,164 files, not re-run): `breathecode/authenticate/receivers.py:354`,
  `countdown=7200` on a `task_manager` `@task` (a Celery wrapper package imported beside celery, which the library now
  resolves), broker CloudAMQP or Redis by environment. Reported in the review; the case `proj/` reproduces the shape.
- Silent: makeplane/plane (`723bb17`), zenodo/zenodo-rdm* and paperless-ngx/paperless-ngx (final runs);
  andreynovikov/django-shop* (`visibility_timeout` of 8 days), indico/indico (2 days), langgenius/dify
  (`visibility_timeout` from configuration), UB-Mannheim/escriptorium* (`default_retry_delay=60 * 60`, exactly the
  hour), saleor, DefectDojo and superset (review); pretix/pretix (transport options from its configuration file;
  writer's run of the previous rework).

On the review's counterexamples (`celery-final-cx-cv-*`, `celery-final2-cx-cv-*`): 7 findings, all real. The four
review misses are caught (RabbitMQ `countdown=2 * 60 * 60`, SQS `countdown=45 * 60`, `.s(uid).set(countdown=7200)`,
`eta=later`), with the three it already caught; `countdown=3000` and `expires=7200` stay silent.

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

- `t.delay(...)`, `t.apply_async(...)` on a Celery task (a name that resolves to a task def, a task object or a
  signature; RQ's `@job` or a thread pool's `apply_async` is not one), `<x>.send_task(...)`;
- `AsyncResult(id)`, `app.AsyncResult(id)`, `self.AsyncResult(id)`, `celery.result.AsyncResult(id)`, `GroupResult(...)`,
  `ResultSet(...)`, `GroupResult.restore(id)` (also `app.GroupResult.restore`), and the same for a class of the run
  deriving from one of them (pretix's `SafeAsyncResult`);
- a group, chain or chord sent: `group(...)()`, `group(...).apply_async()`, `chord(header)(body)`, `chain(...)()`,
  `(a.s() | b.s()).apply_async()`, and `.delay()`/`.apply_async()` on a signature (`t.s(...)`, `signature(...)`);
- a call of a function of the run that returns one of those (pretix's `get_task_result`);
- a name whose latest plain assignment before the wait is one of those (`r = t.delay(x)` then `r.get()`);
- an element of a list, tuple, dict or comprehension of results (`pending[i]`), and the variable of a `for` statement
  or a comprehension over one (`results = [t.delay(i) for i in ids]`, then `[r.get() for r in results]`).

It leaves alone: a wait that passes `disable_sync_subtasks=` anything but `True` (DefectDojo's
`dojo/celery_dispatch.py:141` does so deliberately, on an eager result); a wait inside `with allow_join_result():`, and a
helper a task reaches only through calls made inside such a block; `.get()` on anything not known to be a result
(dicts, caches, querysets, `options.get(...)`, a name last rebound to something else); status reads (`ready()`,
`state`, `successful()`, `id`, `completed_count()`); `t.apply(...)`, whose EagerResult never blocks; code no task
reaches (a view polling a result is fine); tasks and waits in test files. A function reached from both web code and a
task is reported: the task's path blocks.

**Gaps:** results passed in as parameters or kept in attributes; calls the call graph cannot resolve; a project with
`task_always_eager` on, where the wait does not block, is reported all the same.

**Overlap:** none. No linter knows which functions run in a Celery worker.

**Corpus:** no match in 20 Celery repositories, before or after the rework, so precision on real code is
unmeasured and the cases carry the rule. Re-run after the rework: paperless-ngx/paperless-ngx, pretix/pretix,
mozilla/addons-server, taigaio/taiga-back, Netflix/lemur, flaskbb/flaskbb, zenodo/zenodo-rdm, Buuntu/fastapi-react
(runs `celery-fix-three-*`) and makeplane/plane (`celery-final-*`): 0. The review's text search of 10 Celery
repositories found one task-side wait, DefectDojo's `dojo/celery_dispatch.py:141`, which opts out with
`disable_sync_subtasks=False` and stays silent. On the review's counterexamples (`celery-final-cx-rget`): 5 findings,
all real: the list and dict fan-outs the review listed as misses (`[r.get() for r in results]`,
`pending[i].get()`), `AsyncResult(task_id).get()`, a child's `get(timeout=10)` and `get(propagate=False)`; the
thread pool's `apply_async(...).get()`, the review's false positive, is silent.

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

It matches an argument the task receives (`t.delay(x)`, `t.s(x)`, `t.si(x)`, positional, keyword, or through a
`**{...}`/`*[...]` spread; `t.apply_async((x,))`, `t.apply_async(args=[x])`, `t.apply_async(kwargs={"k": x})` or
`kwargs=dict(k=x)`; `app.send_task("n", args=[x])`) where `t` resolves through imports to a Celery task (RQ's
`@job(...)` functions and thread pools have `.delay`/`.apply_async` too, and are not), and the argument is:

- a SQLAlchemy model instance: `M(...)`, `session.get(M, pk)`, `session.scalar(select(M)...)`,
  `M.query...first()`, `session.query(M)...one()`, a name bound to one, a parameter annotated `M`, a value the type
  program proves is a model, Flask-Login's `current_user`, or a list, tuple or dict literal holding one. Message: pass
  the primary key and load the row in the task.
- a SQLAlchemy session, proven by what binds it: a session class's constructor (SQLAlchemy's, SQLModel's, or a
  subclass), a `sessionmaker`/`async_sessionmaker`/`scoped_session` factory or its call or `.begin()`,
  Flask-SQLAlchemy's `db.session` (`db` resolving to a `SQLAlchemy()` object), a run function that returns or yields
  one (`get_db`), `with ... as s` of one, or a parameter annotated with a session class (also inside `Annotated[...]`
  or through an alias such as `SessionDep`). Where nothing binds it, an unannotated parameter whose name ends in
  `session`, never rebound in its function, counts in a run that imports SQLAlchemy. Message: pass the ids and open a
  session in the task.
- a set or frozenset: a literal, a comprehension, a `set(...)`/`frozenset(...)` call, a `|`, `&`, `-` or `^` with one
  of those (`tags | {"x"}`), or a name whose last assignment before the send is one. Message: pass a list.
- an open file: `open(...)`, a name whose last assignment is one, or the target of `with open(...) as f`. Message:
  pass the path or the contents.
- a lambda, a generator expression, or a function of the run passed by name. Message: pass the data, or a name the
  task maps to the function.

It leaves alone: `order.id` and other attributes, primitives, lists, tuples and dicts of plain values, `sorted(s)` and
`list(s)`, a name reassigned to a list before the send; datetime, Decimal, UUID and bytes (kombu round-trips them
since 5.3); `.delay` on a plain undecorated function or another library's job; a local named `session` bound to
something else (a request field, `str(uuid4())`, a dict, a `requests` or `aiohttp` session) and a parameter annotated
with a non-SQLAlchemy type; `session_id`; files that import django for the model shape (the django pack's
`django-practice-task-arg-model-instance` owns Django sends); sets and functions under a pickle serializer (a task's
`serializer="pickle"`, a send's `serializer="pickle"`, or an app's `task_serializer` naming pickle, `signed-pickle`
too), since pickle carries them; the model shape in a run whose Task subclass (resolved through its bases, or the
app's `self.Task`) overrides `apply_async` and rewrites `args`/`kwargs` there (indico's `_CelerySAWrapper` sends each
model's identity key and re-fetches it in the worker); test files. Under pickle the model, session, file, lambda and
generator shapes are still reported: pickle raises on the last four, and ships a stale snapshot of the first.

**Gaps:** values whose kind only a type checker knows (a helper whose return the type program cannot follow);
instances inside comprehensions or containers built elsewhere; related objects (`order.customer`); dataclass and
Pydantic instances (left out: kombu honours `__json__`, and projects register their own types); a model class that
defines `__json__` (it encodes, but the snapshot is still stale, so the message stays right); a project that registers
`set` with kombu's `register_type`; a serializer chosen in task routes or a Task subclass's `serializer` attribute.

**Overlap:** none. Ruff, Bandit and mypy do not look at task arguments; `delay` is untyped without celery-types, and
celery-types accepts any object.

**Corpus:** no match in 20 Celery repositories, before or after the rework, so precision on real code is
unmeasured; every reported shape is a deterministic EncodeError at the send, except a model under pickle. Re-run after
the rework: paperless-ngx/paperless-ngx, pretix/pretix, mozilla/addons-server, taigaio/taiga-back, Netflix/lemur,
flaskbb/flaskbb, zenodo/zenodo-rdm, Buuntu/fastapi-react (runs `celery-fix-three-*`) and makeplane/plane
(`celery-final-*`): 0. indico's pickled models go through its `_CelerySAWrapper`, which the rule exempts. On the
review's counterexamples (`celery-final-cx-targ`): 7 findings, all real: a model in a list, Flask-Login's
`current_user`, a `**{...}` spread, `kwargs=dict(...)`, `tags | {"x"}` (the review's misses), and the two it caught;
the three strings and dicts named `session` and RQ's `rq_tag.delay(set(...))`, the review's four false positives, are
silent.

**Sources:**
- Celery docs, [Tasks: State](https://docs.celeryq.dev/en/stable/userguide/tasks.html#state): "Another gotcha is Django model objects. They shouldn't be passed on as arguments to tasks. It's almost always better to re-fetch the object from the database when the task is running instead, as using old data may lead to race conditions."
- Deni Bertović, [Celery Best Practices](https://denibertovic.com/posts/celery-best-practices/), No. 7: "You shouldn't pass Database objects (for instance your User model) to a background task because the serialized object might contain stale data. What you want to do is feed the task the User id and have the task ask the database for a fresh User object."
- BaoDev, [flush vs commit](https://www.baodev.studio/blog/sqlalchemy-flush-vs-commit-difference): "pass primitives to your queue anyway, never ORM objects."
- kombu, [`kombu/utils/json.py`](https://github.com/celery/kombu/blob/main/kombu/utils/json.py): the encoder registers datetime, date, time, Decimal and UUID and handles bytes; any other type, sets included, falls through to `json.JSONEncoder.default`, which raises TypeError.

### `celery-practice-unknown-task-name`

Rerail's own rule, not a port. A Celery message carries only the task's name; the worker looks the name up in its
registry and, for a name no task registered, logs "Received unregistered task of type ... The message has been ignored
and discarded." A beat entry whose `"task"` has a typo, or names a task that was renamed or moved to another module, is
sent on schedule and dropped every time: the job silently never runs, and nothing fails at deploy.

It ships in `log` mode, off by default: no corpus repository has a real unknown name, so its precision on real code is
unmeasured, and the cases and counterexamples are its evidence.

It matches, at the naming site, a task name that no task of the run registers, when the run owns the name:

- the first argument (or `name=`) of `send_task(...)` and `signature(...)` on a Celery app (resolved: a module-level
  `Celery(...)`, a factory's result, `current_app`, `self.app`; a look-alike class's `send_task` is not one), and
  celery's own `signature(...)`;
- the `"task"` of a beat entry: a dict (or `dict(...)` call) with `"task"` and `"schedule"`, or one with `"task"` and
  `"options"` whose value reaches the beat setting: the `beat_schedule`/`CELERY_BEAT_SCHEDULE` value, an item assigned
  to it or passed to its `.update(...)`, a module constant it reads (through imports and `**` spreads), or a run
  function called from it or reached from one through the call graph (paperless-ngx's `parse_beat_schedule()`).

Every name is read by one value resolver: a string literal, a module constant of the run through imports, re-exports
and module aliases, a class attribute (`ExportTask.name`), or a local of its own function, each when bound once. A task
is registered under its `name=`, else `<module>.<function>` (matched also by a dotted tail), whether the decorator is
applied with `@` or called as a function (`app.task(fn)`, `shared_task(name="ops.rotate")(fn)`), and on any resolved
app, also one built by a factory (`cel = make_celery()`). A class deriving from celery's `Task` or from `<app>.Task`
registers its `name` attribute (resolved the same way), else `<module>.<Class>`. The run owns a name when its module
part is exactly a module of the run, or when an explicit task name of the run shares its first segment (superset's
`reports.scheduler`). When a task whose function has the name's last segment is registered elsewhere, a second message
names it as a moved task. Both messages say to check the name against the worker's registered tasks and to leave it
when another service or an installed package registers it.

It leaves alone: Celery's own tasks (`celery.*`); names whose module part is not exactly a module of the run and
whose first segment no explicit name shares (another service's task, or a library's); a dict with `"task"` and
`"options"` that never reaches the beat setting (an ML job list); names built at runtime or held by a constant outside
the run; commented-out entries; test code.

**Gaps:** a name computed by a decorator wrapper, or held by an installed package's constant (that task registers
nothing, so a string naming it is reported if its module is the run's); a run whose module names carry a leading
package the worker does not (`api.tasks.x` imported as `tasks.x`) no longer owns `tasks.*`; beat entries added at
run time (`add_periodic_task`, django-celery-beat rows); whether the worker imports the module that defines a task.

**Overlap:** none. No linter resolves Celery task names across files; the worker's log reports them at run time.

**Corpus:** no match in 20 Celery repositories, before or after the rework, and none was due: the hand check of the
beat entries and string sends in 8 of them found every name registered. Re-run with the reworked rule:
paperless-ngx/paperless-ngx, pretix/pretix, mozilla/addons-server, makeplane/plane and apache/superset (rule writer's
runs `celery-fix-unknown-name-*`), and plane again with the final library (`celery-final-*`): 0. Evidence that it
fires: on the review's counterexamples (`celery-final-cx-uk`) 2 findings, both real (the typo `app.tasks.local_jbo`
and the review's missed `dict(task="app.tasks.missing", ...)` item assignment), and the review's four false positives
(a factory-built app, a class name from a constant, another service's `tasks.x`, an ML job list with `"options"`) are
silent; a copy of paperless-ngx's settings with one name changed (`documents.tasks.empty_trsh`) reports exactly that
line through `parse_beat_schedule()`. Precision on real code stays unmeasured, hence log, off.

**Sources:**
- Celery docs, [Tasks: Names](https://docs.celeryq.dev/en/stable/userguide/tasks.html#names): every task has a unique
  name, generated from the module and function name unless `name=` is given, and the name is what the message carries.
- Celery source, [worker consumer](https://docs.celeryq.dev/en/stable/_modules/celery/worker/consumer/consumer.html):
  `UNKNOWN_TASK_ERROR`, "Received unregistered task of type %s. The message has been ignored and discarded."
- Stack Overflow, [Celery "Received unregistered task of type"](https://stackoverflow.com/questions/30746807) and
  [Python Celery Received unregistered task of type - import app error](https://stackoverflow.com/questions/47161489):
  messages dropped because the name sent does not match the name the worker registered.
