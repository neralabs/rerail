# Rerail rule pack: Python practice

Rerail's own Python rules, from the Python docs and core developers' review advice, for correct async code, exceptions, typing and packaging.

This pack is private while its rules are written and reviewed: it is not published to npm, and is installed from
a checkout with `rerail packs add packs/python-practice`.

## Rules

| Rule | Mode | Catches |
|---|---|---|
| `python-practice-blocking-call-reached-from-async` | Log | Python practice: an async def calls a sync helper, in any module, that sleeps, does network I/O, runs a subprocess or starts an event loop |
| `python-practice-cancellation-swallowed` | Steer | Python practice: asyncio.CancelledError caught in an async def and not re-raised |
| `python-practice-contextmanager-cleanup-skipped` | Log | Python practice: release after an unguarded yield in a @contextmanager is skipped on error |
| `python-practice-coroutine-never-awaited` | Steer | Python practice: coroutine of an async def created and never run |
| `python-practice-requests-session-without-timeout` | Steer | Python practice: an HTTP call on a requests Session without a timeout |
| `python-practice-retry-without-limit` | Log | Python practice: a retry that never gives up |

## Rule details

### `python-practice-blocking-call-reached-from-async`

Rerail's own rule, not a port. A coroutine runs on the event loop's thread until its next `await`. A blocking call
anywhere below it, also in a plain helper in another module, stops every other task on the loop until it returns:
under load one slow upstream stalls the whole service. And a helper that starts an event loop (`asyncio.run()`,
`run_until_complete()`, asgiref's `async_to_sync`) raises RuntimeError when a coroutine reaches it, because a loop
already runs in that thread.

It reports, in an `async def`'s own frame (not a nested def, lambda or class body) and outside test files, a call
whose target, by the call graph across the files of the run, is a sync function that blocks its thread, itself or
through a chain of sure calls through other sync functions in any module. The message names the helper and the
nearest blocking site (`path:line`), one finding per call:

- **blocking calls:** `time.sleep`, the sync `requests` and `httpx` module functions (`requests.get`, `httpx.post`,
  `requests.request`, ...), `urllib.request.urlopen`, `subprocess.run`/`call`/`check_call`/`check_output`/
  `getoutput`/`getstatusoutput`, `os.system`, `os.popen`, `os.wait*`, `socket.create_connection`/`getaddrinfo`/
  `gethostbyname`, `smtplib.SMTP(...)`/`SMTP_SSL(...)`. Message: run the helper with
  `await asyncio.to_thread(helper, ...)` or make it async, and leave it when it runs only before the server serves;
- **loop starts:** `asyncio.run()`, `<loop>.run_until_complete()`, `async_to_sync(f)(...)`, with their own message
  (a crash, not a stall): await the coroutine instead. A loop start written directly in the `async def` is reported
  too: no linter checks it.

Names resolve through imports, so `from time import sleep` counts, and `from app import requests; requests.get(...)`
with `app/requests.py` a module of the run does not. FastAPI and Starlette routes are `async def`s like any other and
are reported.

It leaves alone:

- a blocking call written in the `async def` itself: the `python` pack's ASYNC210/221/222/230/251 ports report it;
- a helper run in a thread: `await asyncio.to_thread(helper, x)`, `await loop.run_in_executor(None, helper)`,
  `await sync_to_async(helper)()`, `anyio.to_thread.run_sync(helper)`, or a helper decorated with
  `@sync_to_async`/`@database_sync_to_async`;
- a chain through an `async def` (calling it only makes a coroutine) or through a call whose target the call graph
  cannot settle (a parameter, a dict lookup, a method on an object of unknown type);
- a call in a nested def, lambda or class body inside the `async def`;
- look-alikes: `trio.sleep`, `asyncio.sleep`, `httpx.AsyncClient().get`, a module of the run named `requests`;
- a blocking helper called from a FastAPI/Starlette lifespan (an `asynccontextmanager` passed as `lifespan=`) or an
  `on_event("startup")`/`on_event("shutdown")` handler: it runs while no request is served. A loop start there is
  still reported, since it raises all the same;
- a blocking call under a once-guard: `if self._x is None:` (or a module global) whose body assigns that same name,
  the lazy probe that runs once per process (`git --version` checked and cached);
- `self.m()` in a method of class `K` follows `K`'s own `m` (or the one it inherits), not a subclass's override: a
  base class's `async def` that calls `self.m()` is judged with the base's `m`, so a subclass that overrides both
  methods is not blamed on the base;
- test files.

**Gaps:** file I/O (`open()`, `Path.read_text()`) is left out on purpose: short local reads are common in async code
and rarely what stalls a loop. Sync clients reached through an object (`requests.Session().get`, `smtplib.SMTP().sendmail`,
a sync database driver) are not followed; the Django ORM is `django-practice-sync-orm-reached-from-async`'s, sync
SQLAlchemy sessions are the sqlalchemy and fastapi packs'. A class body nested in the `async def` runs at once, but
is skipped with defs and lambdas. A call the call graph resolves to several targets is followed only when none is
unknown. A subclass that inherits a base's `async def` and overrides only the sync method it calls is not reported.
Startup code outside a lifespan (an `initialize()` or `start()` that runs before the server listens) still blocks
the loop and is reported; the message says to leave it when it runs only before serving.

**Overlap:** Ruff's ASYNC210/221/222/230/251 (ported in the `python` pack) see only a call written in the
`async def`; the helper in `services/http.py` the route calls is invisible to them, and no Ruff rule checks a loop
start in a coroutine.

**Sources:**
- Python docs, [Developing with asyncio: Running Blocking Code](https://docs.python.org/3/library/asyncio-dev.html#running-blocking-code): "Blocking (CPU-bound) code should not be called directly. ... if a function performs a CPU-intensive calculation for 1 second, all concurrent asyncio Tasks and IO operations would be delayed by 1 second."
- Python docs, [Runners: asyncio.run](https://docs.python.org/3/library/asyncio-runner.html#asyncio.run): "This function cannot be called when another asyncio event loop is running in the same thread."
- asgiref, [`sync.py`](https://github.com/django/asgiref/blob/main/asgiref/sync.py): AsyncToSync raises "You cannot use AsyncToSync in the same thread as an async event loop - just await the async function directly."
- Ruff, [blocking-http-call-in-async-function (ASYNC210)](https://docs.astral.sh/ruff/rules/blocking-http-call-in-async-function/): the check and its limit to calls written in the async function.

**Corpus:** 19 repositories of the FastAPI, asyncio and Django stacks, every match read (12), 10 judged real:
**83%**, so the rule starts in `log`. A match counts as real when the blocking call can run while other tasks are on
the loop: in a request path, or at startup while background tasks already run.

| Repository | Commit | Matches | Real |
|---|---|--:|--:|
| jupyterhub/jupyterhub | 3ca37442a18e | 2 | 1 |
| letta-ai/letta | 1131535716e8 | 6 | 5 |
| mealie-recipes/mealie | 8dbb92808d5a | 1 | 1 |
| paperless-ngx/paperless-ngx | faa6f09f289b | 1 | 1 |
| apache/superset | fdf61721d056 | 2 | 2 |

No match in fastapi-users/fastapi-users (e9c57ff7ff2f), fastapi/full-stack-fastapi-template (77e550a73bb4),
Buuntu/fastapi-react (af17f2d422d0), open-webui/open-webui (0666e59d26a4: its sync web-search and license helpers
are already run through `asyncio.to_thread`), freqtrade/freqtrade (7fcb509a434c), Netflix/dispatch (dd2837e82a0b),
zenml-io/zenml (61569b1e08e7), healthchecks/healthchecks (c1039ad23395), HumanSignal/label-studio (2f42caecbf6c),
makeplane/plane (723bb178f97b), inventree/InvenTree (939df605f43d), taigaio/taiga-back (8ab6a4e3b5f6),
Netflix/lemur (3a3c2c201d42) or TandoorRecipes/recipes (22b5841a7828).

Real matches:
- letta `server/rest_api/routers/v1/sandbox_configs.py:146` and `:150`: an async route recreates a venv and runs
  `pip install` through `subprocess.run` in the loop's thread: every request waits minutes.
- letta `services/mcp_manager.py:821`, `services/mcp_server_manager.py:992`: each MCP client build resolves the
  server's host with `socket.getaddrinfo` (`helpers/url_validation.py:62`) in the loop's thread.
- letta `schemas/providers.py:907`: `OllamaProvider.list_llm_models_async` calls `get_model_context_window`, a
  `requests.post` with no timeout, once per model.
- mealie `pkgs/safehttp/transport.py:216` and paperless-ngx `src/paperless/network.py:178`: the async SSRF-pinning
  httpx transports resolve the host with `socket.getaddrinfo` on every request.
- superset `mcp_service/system/tool/health_check.py:78`, `generate_bug_report.py:312`: async MCP tools run
  `git rev-parse` (`utils/version.py:111`) on each call when no `GITHUB_*` variable is set.
- jupyterhub `proxy.py:730`: `start()` may `time.sleep` up to 3 s killing a stale proxy, while `init_spawners` can
  still be running in the background.

Not real: jupyterhub `app.py:3612` (`initialize()` runs the alembic upgrade before any task starts) and letta
`server/server.py:420` (`init_async`, called from the lifespan, prepares the sandbox venv before serving): startup
code outside a lifespan, which the rule cannot tell apart. Three shapes found on the way were fixed and are cases now:
mealie's lifespan (`app.py:68`, the database wait loop), letta's base `Provider.list_embedding_models_async`
(`schemas/providers.py:61`, blamed for a subclass override that never runs there) and letta's once-cached
`git --version` probe (`services/memory_repo/git_operations.py`, 5 matches).

### `python-practice-cancellation-swallowed`

Rerail's own rule, not a port. asyncio implements `Task.cancel()`, `asyncio.timeout()`, `wait_for()` and `TaskGroup` by
throwing `CancelledError` into the coroutine. A coroutine that catches it and goes on keeps running after it was
cancelled: a timeout no longer fires, a TaskGroup waits on it, shutdown hangs, and the awaiting caller resumes as if
the work had finished.

It reports, in an `async def`'s own frame and outside test code, an `except` clause that catches cancellation
(`asyncio.CancelledError`, also through `asyncio.exceptions`, an import alias or a tuple; `BaseException`; a bare
`except:`) of a try whose body awaits (`await`, `async with`, `async for`), when some path through the handler ends
without re-raising: it falls through, returns, breaks, continues, or raises another exception first. A path
re-raises when it ends in a bare `raise`, `raise <the caught name>`, `raise CancelledError()`, an `if` whose every
branch re-raises or tests `isinstance(e, T)` against a class a cancellation is not (`if isinstance(e, OSError): ...
else: raise`), a `with` or a nested `try ... finally` that does, `if isinstance(e, CancelledError): raise` (when
the handler also catches `Exception`), or a call of a function of the run whose body always raises.

It leaves alone the idioms a reviewer accepts:

- the drain: `t.cancel()` (or a loop or comprehension cancelling each of `ts`) earlier in the function, and the try
  awaits `t`, `asyncio.gather(*ts)`, `asyncio.wait_for(t, ...)` or `asyncio.shield(t)`: the CancelledError is the
  child's. `asyncio.wait(ts)` is not a drain: it never raises a child's CancelledError, so the one it catches is the
  function's own;
- a handler that reads `asyncio.current_task().cancelling()` (or calls `uncancel()`) and raises;
- the coroutine an event-loop runner starts: `asyncio.run(main())`, `loop.run_until_complete(main())`,
  `asyncio.run_coroutine_threadsafe(main(), loop)`;
- a coroutine the run only ever starts as a task (`create_task`, `loop.create_task`, `ensure_future`, or a helper of
  the run, in any module, that spawns its first argument, such as `safe_create_task(pump())`) and never awaits
  directly, when the handler ends the task: its try is a statement of the body, or of a top-level loop the handler
  leaves with `break` or `return`, followed by nothing but a `return`. Cancelling such a task asks it to stop, and
  it stops;
- a deferred re-raise: the handler sets `cancelled = True` and a later `if cancelled: raise asyncio.CancelledError`
  raises it after the bookkeeping;
- a handler nested in a cancellation handler that itself re-raises on every path;
- `except Exception` (CancelledError has been a BaseException since Python 3.8), sync defs (also one nested in an
  async def), a try body that does not await, an application class named `CancelledError`,
  `concurrent.futures.CancelledError`, and test files.

**Gaps:** a drain whose task travels through another name (`t = self._task; self._task.cancel(); await t`) is
reported; a CancelledError that an async generator or library hands up from a child it cancelled is reported when the
function does not cancel that child by name; a handler that sets a flag a loop reads and then exits cleanly is
reported; `contextlib.suppress(asyncio.CancelledError)` is not a handler and is not read; anyio and trio cancellation
are not read.

**Sources:** Python docs, [Task cancellation](https://docs.python.org/3/library/asyncio-task.html#task-cancellation)
("asyncio.TaskGroup and asyncio.timeout() are implemented using cancellation internally and might misbehave if a
coroutine swallows asyncio.CancelledError"); flake8-async
[ASYNC103/ASYNC104](https://flake8-async.readthedocs.io/en/latest/rules.html) (no-reraise-cancelled,
cancelled-not-raised); Ruff has not ported them ([ruff#8451](https://github.com/astral-sh/ruff/issues/8451)).

**Overlap:** flake8-async ASYNC103/104, a flake8 plugin that is not in Ruff or the `python` pack. ASYNC103 reports
every handler of the drain idiom, the `cancelling()` check, the runner's `main()`, a task's last handler and a
handler nested in one that re-raises, the shapes the corpus shows are deliberate. This rule leaves them alone, and
reads across files what a single-file linter cannot: whether a coroutine is only ever started as a task (also
through a spawn helper in another module, decorated or not), and whether a handler's call goes to a helper that
always raises.

**Corpus** (pinned commits from `MANIFEST.tsv`): 20 matches, 19 real (95%).

| Repository | Commit | .py files | Matches | Real |
|---|---|--:|--:|--:|
| jupyterhub/jupyterhub | `3ca37442` | 158 | 2 | 1 |
| freqtrade/freqtrade | `7fcb509a` | 484 | 3 | 3 |
| open-webui/open-webui | `0666e59d` | 330 | 3 | 3 |
| letta-ai/letta | `11315357` | 878 | 12 | 12 |
| fastapi-users/fastapi-users | `e9c57ff7` | 88 | 0 | – |
| mealie-recipes/mealie | `8dbb9280` | 723 | 0 | – |
| Netflix/dispatch | `dd2837e8` | 717 | 0 | – |

Real matches: freqtrade `freqtrade/exchange/exchange.py:3658` and `:3697` (a cancelled trade download breaks out of
its loop and returns the partial list to an awaiting caller as a success); freqtrade `freqtrade/rpc/telegram.py:144`
(`except BaseException` around every Telegram command); open-webui `backend/open_webui/main.py:1782`, `:1784` (an
`except BaseException` that logs the cancellation `MCPClient.disconnect()` takes care to propagate) and `:1808`;
jupyterhub `jupyterhub/app.py:4124` (`asyncio.wait()` never raises a child's CancelledError, so the handler only
ever swallows the shutdown task's own); letta `letta/services/tool_executor/tool_execution_manager.py:131` (a
cancelled tool call returns an error result and the agent loop goes on),
`letta/services/tool_sandbox/e2b_sandbox.py:103` and `letta/server/rest_api/streaming_response.py:358` (turned
into another exception), `letta/server/rest_api/streaming_response.py:248`, `letta/server/rest_api/routers/v1/tools.py:753`,
and seven streaming-interface handlers (`letta/interfaces/openai_streaming_interface.py:264`, ...) that log
"Cancelled stream attempt but overriding" and keep yielding: deliberate, and exactly what the Python docs warn about.
The one false positive, jupyterhub `jupyterhub/apihandlers/users.py:1097`, catches a CancelledError that the
spawner's progress generator can hand up from a future it cancelled; nothing in the function names that future.
Left alone on the way, as intended: drains at open-webui `tasks.py:338`, letta `streaming_response.py:148`,
`mcp_manager.py:1230`; the `cancelling()` checks at open-webui `utils/mcp/client.py:370` and
`utils/subagents.py:851`; the deferred re-raise at open-webui `utils/subagents.py:702`; the handlers nested in a
re-raising handler at open-webui `utils/middleware.py:6564`, `:6598`; the `isinstance` re-raise at open-webui
`utils/tether.py:851`; the thread's main coroutine at freqtrade `rpc/external_message_consumer.py:152` and the
tasks it spawns (`:168`); letta's background tasks started through `safe_create_task` (`redis_stream_manager.py:182`,
`:324`).

### `python-practice-contextmanager-cleanup-skipped`

Rerail's own rule, not a port. contextlib runs a generator context manager up to its `yield`, runs the with-block, and,
when the block raises, throws the exception in at the `yield`. Code after a yield that no `try`/`finally` (or `with`)
guards then never runs: the lock stays held, the file descriptor, cursor or connection leaks, the temp directory
stays, and a patched attribute, environment variable, signal handler, working directory or thread-local user stays
changed for every later caller.

It reports, outside test code, a `yield` in the own frame of a def decorated with `contextlib.contextmanager` or
`asynccontextmanager` (resolved through imports and aliases) that sits in the body of no `try` with an `except` or
`finally` clause and of no `with` of that def, when a release follows it on the normal path (a later statement of a
block that holds the yield, with no `return` or `raise` between them):

- `X.close()`, `aclose()`, `release()`, `unlock()`, `shutdown()`, `stop()`, `terminate()`, `kill()`,
  `disconnect()`, `dispose()`, `rollback()`, `cleanup()`, `unlink()`;
- `os.remove()`, `os.unlink()`, `os.rmdir()`, `shutil.rmtree()`;
- `X.reset(...)`/`X.restore(...)` of something the def touched before the yield (`token = var.set(v)`);
- `T = old` where the def saved `old = T` before the yield (T an attribute or subscript, by its text), and
  `T = False` where it set `T = True` before the yield;
- writes to `os.environ` (`os.environ[k] = old`, `del os.environ[k]`, `pop()`, `update()`);
- a call that puts back a saved value: the same function was called before the yield and now gets a name bound before
  it (`old = get_user(); set_user(u); yield; set_user(old)`, `os.chdir(old)`, `signal.signal(sig, old)`).

The message names the release and its line, and says to leave the code alone when the release must happen only on
success.

It leaves alone: `try: yield x finally: x.close()`; `with lock: yield`; `try: yield except: rollback(); raise`
followed by `commit()`; code after the yield that commits, flushes, enqueues, logs or records success (meant to run
only on success); an early `if x is None: yield; return` (code after it is unreachable from that yield);
`X.cleanup()` of a `tempfile.TemporaryDirectory`, whose finalizer removes the directory as soon as the generator's
frame drops it; `X.reset()` of something the def did not touch before the yield; ASGI lifespan functions (named
`*lifespan*`, or passed as `lifespan=`), whose shutdown is the server's; pytest fixtures (pytest runs the code after a
fixture's yield also when the test fails); a yield in a nested def; a `contextmanager` decorator that is not
contextlib's; test files.

**Gaps:** a yield guarded by a `try` whose handler catches only some exceptions counts as guarded; a release in a
helper called after the yield (`self._teardown()`) is not followed; `del X.attr` after the yield is not a release; a
release written after the `try` of a guarded yield is not reported; a release that is meant to be skipped on error (a
marker file left behind on purpose so the next run retries) is reported, and the message says to leave it.

**Sources:** Python docs, [contextlib.contextmanager](https://docs.python.org/3/library/contextlib.html#contextlib.contextmanager)
("If an unhandled exception occurs in the block, it is reraised inside the generator at the point where the yield
occurred. Thus, you can use a try…except…finally statement to trap the error (if any), or ensure that some cleanup
takes place"); pylint
[R1732 consider-using-with](https://pylint.readthedocs.io/en/stable/user_guide/messages/refactor/consider-using-with.html)
("the release of the allocated resources is ensured even in the case of an exception").

**Overlap:** none. Ruff SIM115 and pylint R1732 check `open()`, `Popen`, pools and lock acquires used without `with`;
neither reads what a generator context manager runs after its `yield`.

**Corpus** (pinned commits from `MANIFEST.tsv`): CORPUS_TABLE

### `python-practice-coroutine-never-awaited`

Rerail's own rule, not a port. Calling an `async def` runs none of its body: it returns a coroutine (an async
generator when the def yields), which runs only when it is awaited, scheduled as a task or iterated. When the
coroutine is thrown away, the work silently never happens; the only sign is `RuntimeWarning: coroutine '...' was
never awaited`, printed when the object is garbage collected, far from the call.

It reports, outside test code:

- **a dropped coroutine**: a call written as a statement of its own, `notify(user)` instead of `await notify(user)`,
  in a sync def, an async def or module code, whose every target by the call graph (across files, through imports,
  `self.m()` with every override in the run's subclasses) is an `async def` of the run, and which can call nothing
  else. A dropped async generator (`stream()` for an `async def` that yields) gets its own message (`async for`);
- **an async def handed to a sync caller**: an async def of the run, named directly, through an import, as
  `self.m` or inside `functools.partial(...)`, passed to an API that calls it as a plain function and ignores (or
  only stores) the coroutine it returns: `threading.Thread(target=...)`, `multiprocessing.Process(target=...)`,
  `threading.Timer(interval, fn)`, `<executor>.submit(fn, ...)`, `<loop>.run_in_executor(executor, fn, ...)`,
  `asyncio.to_thread(fn, ...)`, `atexit.register(fn)` (also as a decorator), `signal.signal(sig, fn)`, and an
  asyncio loop's `call_soon`, `call_soon_threadsafe`, `call_later` and `call_at` (asyncio refuses a coroutine
  function there only in debug mode). The message names the fix that fits the API and whether the code runs in a
  coroutine (`await`, `asyncio.run`, `asyncio.create_task`, `asyncio.run_coroutine_threadsafe`,
  `loop.add_signal_handler`).

It leaves alone: `await f()`, `asyncio.create_task(f())`, `ensure_future`, `TaskGroup.create_task`, `asyncio.run(f())`,
`return f()`, `x = f()` (someone may await it), `f()` as an argument (`gather(f(), g())`); a call that may also reach a
sync def (a sync override in a subclass); a def under a decorator from outside the run (`@async_to_sync`: the call
graph leaves the call open) or a workspace decorator returning a sync wrapper; a method resolved through a base
outside the run; a reference to a decorated async def (only `staticmethod`, `classmethod` and `abstractmethod` are
known to keep it a coroutine function); `submit` on a receiver neither built nor named as an executor
(`JobQueue().submit(fn)` may await what it gets); `call_soon`/`call_later` on a receiver not built as an asyncio
loop in the same def, module or class (tornado's `IOLoop`, often named `loop`, runs the awaitable a callback
returns); sync defs handed to any of these APIs; and test code.

**Gaps:** `Thread(target=lambda: worker())` (the lambda returns the coroutine; lambdas are not followed); an async
def reached through a variable, a container or `getattr`; `executor.map(async_fn, ...)`; async defs outside the run
(a library's coroutine function), which the call graph does not see; an asyncio loop held in a parameter or another
module's variable; `self.hook()` in a framework base class when a subclass in the run has a base the call graph cannot
follow (in fastapi-users, the `examples/` and `docs/` subclasses `UserManager(UUIDIDMixin, BaseUserManager[User,
uuid.UUID])` leave `self.on_after_register(...)` in `BaseUserManager` open, so a dropped `await` there is not
reported). `loop.add_signal_handler` is left out on purpose: asyncio always raises `TypeError` for a coroutine
function there, so the bug is loud.

**Overlap:** mypy's `unused-coroutine` error (on by default) reports the dropped-call shape, but only in functions
mypy checks (annotated ones, unless `check_untyped_defs`) and only in projects that run mypy; this rule needs no
annotations and resolves the callee across files by the call graph. Neither mypy nor any Ruff rule reports the
second shape (an async def passed as a `Thread` target, to `submit`, `to_thread`, `atexit` or `signal`). Ruff's
RUF006 is about tasks (`asyncio.create_task(...)` whose result is dropped), not coroutines; ASYNC105 is trio only.

**Corpus:** 0 matches in 9 runs over 3,439 Python files:

| Repository | Commit | .py files | Matches |
|---|---|--:|--:|
| fastapi-users/fastapi-users | `e9c57ff7ff2f` | 88 | 0 |
| fastapi/full-stack-fastapi-template | `77e550a73bb4` | 43 | 0 |
| jupyterhub/jupyterhub | `3ca37442a18e` | 158 | 0 |
| open-webui/open-webui | `0666e59d26a4` | 330 | 0 |
| freqtrade/freqtrade | `7fcb509a434c` | 484 | 0 |
| Netflix/dispatch | `dd2837e82a0b` | 717 | 0 |
| mealie-recipes/mealie | `8dbb92808d5a` | 723 | 0 |
| letta-ai/letta | `1131535716e8` | 878 | 0 |
| saleor/saleor, `saleor/asgi/` only (all 10 of its non-test `async def`s) | `e1ccf4d58002` | 18 | 0 |

Precision cannot be measured on merged code, which rarely keeps this bug: the work visibly never happens and the
RuntimeWarning names the coroutine, so it tends to be fixed before merge; the shape is one that coding agents write.
letta alone has about 1,570 non-test `async def`s, and open-webui and freqtrade use asyncio throughout; none of
their merged code drops a coroutine of the run or hands an async def to a thread, executor, `atexit` or `signal`
(the pack's earlier mining probe also found none in polar). As a check that the rule fires on a real
codebase, a copy of fastapi-users with three seeded bugs (an `await` removed from `self.on_after_register(...)` in
`fastapi_users/manager.py`, a new `threading.Thread(target=self.on_after_login, ...)` in a `BaseUserManager`
subclass, and a cross-module `ping()` dropped in a sync def) reported the last two with the full repository, and
all three once `examples/` and `docs/` were removed (see Gaps). The rule ships in `steer` on the strength of its
shape: every match is a coroutine that cannot run, and each message's fix only adds an `await` or a loop entry.

**Sources:**
- Python docs, [Developing with asyncio: Detect never-awaited coroutines](https://docs.python.org/3/library/asyncio-dev.html#detect-never-awaited-coroutines):
  "When a coroutine function is called, but not awaited (e.g. `coro()` instead of `await coro()`) or the coroutine
  is not scheduled with `asyncio.create_task()`, asyncio will emit a RuntimeWarning".
- mypy docs, [Check that coroutine return value is used [unused-coroutine]](https://mypy.readthedocs.io/en/stable/error_code_list.html#check-that-coroutine-return-value-is-used-unused-coroutine):
  "Mypy ensures that return values of async def functions are not ignored, as this is usually a programming error,
  as the coroutine won't be executed at the call site".
- SuperFastPython, [Asyncio common errors and warnings](https://superfastpython.com/asyncio-common-exceptions-and-warnings/):
  lists `RuntimeWarning: coroutine '...' was never awaited` among the common asyncio errors, from calling a
  coroutine function without awaiting it.
- Python docs, [asyncio event loop: run_in_executor](https://docs.python.org/3/library/asyncio-eventloop.html#asyncio.loop.run_in_executor),
  [asyncio.to_thread](https://docs.python.org/3/library/asyncio-task.html#asyncio.to_thread),
  [threading.Thread](https://docs.python.org/3/library/threading.html#threading.Thread),
  [Executor.submit](https://docs.python.org/3/library/concurrent.futures.html#concurrent.futures.Executor.submit),
  [atexit.register](https://docs.python.org/3/library/atexit.html#atexit.register) and
  [signal.signal](https://docs.python.org/3/library/signal.html#signal.signal): each calls the function it is given
  as `fn(*args)` and keeps or drops the return value; none awaits it. In debug mode asyncio's `run_in_executor`
  refuses a coroutine function with `TypeError: coroutines cannot be used with run_in_executor()`.

### `python-practice-requests-session-without-timeout`

Rerail's own rule, not a port. requests has no default timeout, on a `Session` as on the module functions: a server
that accepts the connection and then stops answering hangs the calling worker, request handler or task forever.
Bandit B113 (the `python` pack's `bandit-b113-request-without-timeout`, Ruff S113) reads only `requests.get(...)` and
its siblings, so code that follows the advice to "use a Session" loses the check. This rule reads the calls on a
Session instance, and binds the instance across statements, methods and modules.

It reports, in a def, lambda or class body outside test code, `.get/.post/.put/.patch/.delete/.head/.options/.request(...)`
with no `timeout=` (**`SessionCallWithoutTimeout`**) or with `timeout=None` (**`SessionCallTimeoutNone`**, as B113
reports it), on a receiver that holds a Session made by `requests.Session()`, `requests.session()`, or a class of the
run deriving from `requests.Session` that sets no timeout itself. The message names where the Session was created. The
receiver is bound through:

- a local name: `s = requests.Session(); s.get(url)` (the last assignment before the read), `with requests.Session() as
  s: s.post(...)`, and a name copied from one (`other = s`);
- `self.x` set in any method of the class or of a base class (`self.session = requests.Session()` in `__init__`, read in
  another method or in a subclass's), and a class attribute `session = requests.Session()`;
- a module global, read in its own module or imported from another (`from .net import session`, `net.session`);
- a call of a function or method of the run that returns a new Session (`make_session().get(url)`,
  `self._session = self._new_session()`), through its return value, across files.

It leaves alone:

- a call with any `timeout=` other than `None` (also a variable, or `(5, None)` for a stream), a call passing `*args`
  or `**kwargs` (a timeout may be inside), and `request()` with a ninth positional argument (the timeout);
- a Session of a class of the run that overrides `request()` or `send()` with a body naming `timeout` (the
  `TimeoutSession` idiom);
- a Session on which `mount()` installs an adapter of a class of the run deriving from `HTTPAdapter` whose `send()`
  names `timeout` (`TimeoutHTTPAdapter`), when the `mount()` receiver binds to the same Session: in the factory, the
  function, or any method of the class. A plain `HTTPAdapter(max_retries=...)` sets no timeout and does not count;
- `httpx.Client` (5 s default timeout), `aiohttp.ClientSession`, SQLAlchemy's `Session`, and `requests.get(...)`
  (B113's);
- receivers it cannot bind: a parameter (annotated or not), `self.x` set from a parameter, a dict of sessions, a
  property;
- a call at module level, outside every def and class: a script run by hand, whose hang is in front of its operator
  (zenodo-rdm `scripts/new_api_flow.py`, 9 calls);
- test files.

**Gaps:** a Session subclass whose `__init__` sets `self.timeout` is reported, rightly: requests never reads that
attribute. A timeout adapter mounted through a helper (`configure(session)`) is not seen, so that Session is reported.
A module-level function taking `self` as a plain parameter (lemur `lemur_azure_dest/plugin.py:81`) is not bound.
`session.send(prepared)` is not read.

**Sources:** requests docs, [Timeouts](https://requests.readthedocs.io/en/latest/user/advanced/#timeouts) ("By
default, requests do not time out unless a timeout value is set explicitly. Without a timeout, your code may hang for
minutes or more"); Bandit [B113 request_without_timeout](https://bandit.readthedocs.io/en/latest/plugins/b113_request_without_timeout.html)
and its port `packs/python/rules/bandit-b113-request-without-timeout.json`.

**Overlap:** Bandit B113 / Ruff S113 cover the `requests.<verb>()` module functions, and this rule leaves them alone.
What it adds: calls on Session instances, bound across methods, subclasses, factories and modules. The
python-security pack owns SSRF on the same calls; this rule is about timeouts only.

**Corpus:** 16 repositories, every match read; 63 matches, 63 real (100%) with the final rule. The 10 matches its
module-level and example-directory exclusions removed (zenodo-rdm `scripts/new_api_flow.py`, 9; jupyterhub
`examples/server-api/start-stop-server.py:164`, 1), hand-run scripts against a local server, were the only ones
judged not real.

| Repository | Commit | Matches | Real |
|---|---|--:|--:|
| pretix/pretix | 40ea9097d | 1 | 1 |
| HumanSignal/label-studio | 2f42caecb | 0 | 0 |
| open-webui/open-webui | 0666e59d2 | 0 | 0 |
| Netflix/dispatch | dd2837e82 | 0 | 0 |
| DefectDojo/django-DefectDojo | cd0a4483a | 0 | 0 |
| Netflix/lemur | 3a3c2c201 | 24 | 24 |
| rachelos/we-mp-rss | 8fe5bf938 | 15 | 15 |
| ecds/readux | fcf0d36d5 | 9 | 9 |
| zenodo/zenodo-rdm | b5bf82ec8 | 9 | 9 |
| khoj-ai/khoj | ae229ca89 | 5 | 5 |
| jupyterhub/jupyterhub, freqtrade/freqtrade, letta-ai/letta, mealie-recipes/mealie, wger-project/wger, makeplane/plane | (MANIFEST.tsv) | 0 | 0 |

Real matches, by file: pretix `plugins/paypal2/client/core/paypal_http_client.py:141` (`PayPalHttpClient.execute`
sends each PayPal API request through `self.session.request(...)`, set in `__init__` with a retry-only `HTTPAdapter`); lemur
`plugins/lemur_digicert/plugin.py` (11 calls to DigiCert, `self.session` set in `__init__`), `lemur_entrust/plugin.py`
(5; the same file's module functions pass `timeout=(15, 40)`), `lemur_verisign/plugin.py` (5), `lemur_cfssl/plugin.py`
(2), `lemur_azure_dest/plugin.py:187`; zenodo-rdm `openaire/tasks.py:144` and `:151` (Session from
`openaire_request_factory()` in `openaire/utils.py`) and
`partners/client.py` (7, Session from the static `_create_session()`); readux `apps/export/github.py` (9); khoj
`processor/content/notion/notion_to_entries.py` (4) and `github/github_to_entries.py:179` (a `stream=True` download);
we-mp-rss `driver/wx_api.py` (14, two calls of the same class pass `timeout=15`) and `core/wx/base.py:214`.

The repositories that report nothing bind their Sessions and pass a timeout on every call: DefectDojo's
`api_bugcrowd`, `api_sonarqube` and `api_cobalt` clients (20 calls), label-studio `core/utils/contextlog.py:103`
(reached through `self._new_session()`), open-webui `retrieval/web/utils.py:445` and `:1163` (through
`get_ssrf_safe_requests_session()`). label-studio's `ssrf_safe_request` forwards `**kwargs`; dispatch's
`cli.py:1055` Session is passed on as a parameter.

### `python-practice-retry-without-limit`

Rerail's own rule, not a port. A retry that never gives up turns a permanent failure (bad credentials, a 4xx, a deleted
row, a schema error) into an endless loop: a request that never returns, a worker that never takes the next job, a hot
loop hammering the dependency that already failed.

It reports, outside test code, three forms, each as its own violation so their precision can be read apart:

- **`TenacityWithoutStop`**: tenacity's `@retry`, `@retry(...)`, `retry(...)(f)`, `Retrying(...)` and
  `AsyncRetrying(...)`, names resolved through imports (`import tenacity`, `from tenacity import retry as r`), with no
  `stop=`. Tenacity's default is `stop_never` with `wait_none`: it retries forever, without waiting, on any
  exception. A `retry=` predicate alone does not bound it: it picks which errors are retried, not how often.
- **`BackoffWithoutLimit`**: `@backoff.on_exception(...)` and `@backoff.on_predicate(...)` (also imported by name)
  with neither `max_tries=` nor `max_time=`. `max_value=` caps the wait between tries, not their number.
- **`RetryLoopWithoutLimit`**: a hand-written `while True:` (or `while 1:`) loop with a `try` among its statements
  that leaves the loop once its work is done (a `return`, or a `break` of this loop, as a statement of the try body
  or its `else`, or of a `with` there), when one of the try's `except` clauses never raises, returns, breaks the loop
  or calls `sys.exit()`/`os._exit()`, and nothing else in the loop (a statement before or after the try, its
  `finally`) does either: no attempt counter or deadline the loop checks. A raise in a def or lambda nested in the
  loop does not end it.

It leaves alone:

- a `stop=` of any value (`stop_after_attempt(3)`, `stop_after_delay(10)`, a variable, and `stop_never`, which says
  the forever is meant), `max_tries=`/`max_time=` of any value (also `None` or a callable), and keywords passed
  through `**kwargs`, which may hold the limit; Retrying's positional form `Retrying(sleep, stop)`;
- a `for attempt in range(n)` loop; a `while True` loop whose handler re-raises once a counter or deadline is
  exceeded, or whose loop checks one before or after the try (healthchecks `hc/api/transports.py:180`, mealie
  `mealie/repos/repository_group.py:25`);
- a `while True` worker, polling or reconnect loop whose exits are only conditional (`if msg is None: break`,
  `if stop.is_set(): break`: a stop signal, not a retry's success) or that has none, since it is meant to run until
  told to stop (jupyterhub `singleuser/extension.py:502`, open-webui `utils/automations.py:87`, the subscriber's
  reconnect loop in open-webui `socket/redis_room_channels.py:136`, dify `api/core/mcp/client/streamable_client.py:414`);
- a loop whose looping handler catches only a wait's timeout: `q.get(timeout=1)` then `except queue.Empty: continue`
  (also `asyncio.QueueEmpty`) waits for the next item, and a redis `Lock(..., blocking_timeout=3)` then
  `except LockError` waits for the lock; every other error still leaves the loop (dify
  `api/core/mcp/session/base_session.py:235`, breatheco-de/apiv2 `breathecode/celery.py:182`);
- a handler of a name-conflict error (`EntityExistsError`, `IntegrityError`, a class named for a duplicate, conflict
  or unique error) that assigns a new value the try reads: `name = f"{base}-{random_str(4)}"` searches for a free
  name, each turn sends something new (zenml `src/zenml/zen_stores/sql_zen_store.py:11509`);
- a loop that prompts the person at the terminal each turn (`input()`, `getpass()`, `click.prompt()`,
  `typer.prompt()`, rich's `Prompt.ask()`): each turn is new input, not the same call repeated (superset
  `superset-extensions-cli/src/superset_extensions_cli/cli.py:732`);
- a `while` with a real condition;
- `retry` names of other libraries: urllib3's `Retry`, the `retrying` package's `@retry(stop_max_attempt_number=...)`,
  Celery's `self.retry()` (the celery pack's), a function of the run named `retry`;
- test files.

**Gaps:** a loop whose bound sits in a helper its handler calls (`self._check_attempts()`, which raises) is reported;
a poll-until-ready loop (`try: r = get(); if r.ok: break`) is not read, nor a loop whose try is nested in an `if` or
`with` of the loop body; the `retrying` and `retry` packages, also unbounded by default, are not read; a tenacity
`.retry_with(stop=...)` at the call site is not followed.

**Sources:** tenacity API reference, [`Retrying(stop=stop_never, wait=wait_none)`](https://tenacity.readthedocs.io/en/latest/api.html);
tenacity docs, ["the default behavior is to retry forever without waiting"](https://tenacity.readthedocs.io/en/latest/);
backoff [configuration](https://backoff.readthedocs.io/en/latest/user-guide/configuration.html) (`max_tries` and
`max_time` are its only limits).

**Overlap:** none. Ruff, pylint and Bandit have no retry rule; Celery task retries belong to the celery pack.

**Corpus:** run over 26 repositories (commits from `MANIFEST.tsv`). With the final rule it reports 3 matches, all
read and all real:

| Repository | Commit | Matches | Real |
|---|---|--:|--:|
| Netflix/dispatch | `dd2837e` | 1 (TenacityWithoutStop) | 1 |
| khoj-ai/khoj | `ae229ca` | 2 (RetryLoopWithoutLimit) | 2 |
| langgenius/dify | `724ac15` | 0 (5 before the queue and stop-signal exclusions) | – |
| apache/superset | `fdf6172` | 0 (3 before the prompt exclusion) | – |
| zenml-io/zenml | `61569b1` | 0 (2 before the free-name exclusion) | – |
| breatheco-de/apiv2 | `2055837` | 0 (1 before the LockError exclusion) | – |
| letta-ai/letta | `1131535` | 0 (1 before success meant an unconditional `return`/`break`) | – |
| jupyterhub, open-webui, mealie, healthchecks, freqtrade, paperless-ngx, plane, label-studio, InvenTree, linkding, pretix, netbox, indico, taiga-back, wger, Tandoor recipes, escriptorium, we-mp-rss, Netflix/lemur | see `MANIFEST.tsv` | 0 | – |

The real matches: dispatch `src/dispatch/plugins/dispatch_google/drive/drive.py:117`
(`@retry(wait=wait_exponential(multiplier=1, max=10))` around Google Drive calls: a revoked credential or a 404 is
retried forever, the only library retry in the corpus without a limit); khoj `src/khoj/utils/initialization.py:257`
and `:266` (server start-up retries creating the admin user and the chat configuration on `except Exception`, with
no sleep and no bound: with the credentials in environment variables, a database error spins the start-up in a hot
loop). Every other tenacity and backoff use in the corpus sets a stop (superset, dify, khoj, dispatch); Netflix/lemur
uses the `retrying` package with `stop_max_attempt_number`. The 12 loop matches the first version reported, each a
shape the rule now leaves alone, are listed above under "It leaves alone"; they are why the rule starts in `log`:
each new large repository showed a new shape of `while True: try:` that is not a retry.
polarsource/polar (`872272f`) and mozilla/addons-server (`7e3374f`) were queued but never got a run slot; a text
search finds no tenacity or backoff use in either, and the polar `while True: try:` loops read by hand
(`sdk/python/polar/metering/buffered.py:373`, `sdk/python/polar/outpost/client.py:281` and `:1618`,
`dev/cli/shared.py:171`) bound the retry with a deadline or an attempt budget, or return on the first failure.
