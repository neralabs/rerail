# Rerail rule pack: FastAPI

Rerail's own FastAPI rules, from the FastAPI docs and maintainers' review advice, for safe dependencies, response models and async endpoints.

This pack is private while its rules are written and reviewed: it is not published to npm, and is installed from
a checkout with `rerail packs add packs/fastapi`.

## Rules

| Rule | Mode | Catches |
|---|---|---|
| `fastapi-practice-cors-wildcard-with-credentials` | Steer | FastAPI practice: CORSMiddleware allows every origin together with allow_credentials=True |
| `fastapi-practice-http-client-per-request` | Log | FastAPI practice: an httpx or aiohttp client built per request, in a route, dependency or middleware or a helper they call |
| `fastapi-practice-on-event-handler` | Steer | FastAPI practice: startup/shutdown handler registered with on_event instead of the lifespan |
| `fastapi-practice-request-session-in-background-task` | Steer | FastAPI practice: the request's database session handed to a background task, which runs after its dependency or middleware has closed it |
| `fastapi-practice-response-exposes-secret-field` | Steer | FastAPI practice: a route's response sends a password hash, key or secret field to clients |
| `fastapi-practice-route-missing-sibling-auth` | Log | FastAPI practice: one route of a router leaves out the auth dependency every other route requires |
| `fastapi-practice-route-shadowed` | Steer | FastAPI practice: an earlier route of the same router answers this route's paths first |
| `fastapi-practice-sync-session-in-async-route` | Steer | FastAPI practice: blocking sync SQLAlchemy session work in an `async def` route, dependency or middleware, written there or reached through sync helpers |
| `fastapi-practice-yield-dependency-cleanup` | Steer | FastAPI practice: a yield dependency releases its connection outside finally, or swallows the route's error |

## Rule details

### `fastapi-practice-cors-wildcard-with-credentials`

**Catches** Starlette's `CORSMiddleware` (imported from `starlette.middleware.cors` or `fastapi.middleware.cors`, or a
subclass of it in the run, such as open-webui's `BrowserCORSMiddleware`) configured with `allow_credentials=True` (or a
name bound to `True`) while its origins allow everyone. The middleware is configured by `app.add_middleware(M, ...)`,
`Middleware(M, ...)` (in `FastAPI(middleware=[...])` or `Starlette(middleware=[...])`) or `M(app, ...)`. Origins allow
everyone when `allow_origins`:

- is a list or tuple literal containing `"*"`, or the string `"*"`, or a `+` of lists one of which is;
- is a name or attribute whose value, in this module or another module of the run, is such a value;
- is `os.getenv("X", "*")` or `os.environ.get("X", "*")`, maybe followed by `.split(...)`: an environment variable's
  default;
- is a settings field, `settings.CORS_ORIGINS`, of an instance of a class of the run (`settings = Settings()`, or the
  `Settings()` that `get_settings()` returns) built without that field, whose class default is such a value
  (`CORS_ORIGINS: list[str] = ["*"]`, `Field(default=["*"])`, also inherited from a base);

or when `allow_origin_regex` is a pattern that matches every origin (`".*"`, `"^.*$"`, `".+"`, `"https?://.*"`, ...).

```python
app.add_middleware(CORSMiddleware, allow_origins=["*"], allow_credentials=True)        # reported
CORS_ALLOW_ORIGIN = os.getenv("CORS_ALLOW_ORIGIN", "*").split(";")
app.add_middleware(CORSMiddleware, allow_origins=CORS_ALLOW_ORIGIN, allow_credentials=True)  # reported: env default
```

**Why.** Given `"*"` and `allow_credentials=True`, Starlette's CORSMiddleware echoes the request's `Origin` back with
`Access-Control-Allow-Credentials: true` (`send`: `if origin is not None and self.allow_all_origins and
self.allow_credentials: self.allow_explicit_origin(headers, origin)`; preflights echo the Origin whenever credentials are
on), so any website a logged-in user visits can send cookie-authenticated requests to the API and read the responses.
Browsers refuse `Access-Control-Allow-Origin: *` on credentialed responses for exactly this reason; Starlette's echo
undoes that protection. The message tells the agent to list the allowed origins explicitly (from settings) or to drop
`allow_credentials=True` for a public API; when the wildcard is a default (an environment variable or a settings
field), it says so and suggests an empty default, which keeps deployments that set real origins working.

**Leaves alone** explicit origin lists (`[settings.FRONTEND_HOST]`, `["http://localhost:3000"]`), port patterns such as
`"http://localhost:*"` (Starlette compares origins as strings, so they are no wildcard), the wildcard without
credentials or with `allow_credentials=False` (a public API), values the rule cannot settle (`config["CORS_origins"]`,
a call, a comprehension, a parameter, `self.x`), a settings instance built with the field or of a subclass overriding
it, a project's own origin regex, a class that only shares the name `CORSMiddleware`, and test files.

**Gaps.** A project's own CORS config object that a framework later turns into the middleware (zenml's example
pipeline `CORSConfig(allow_origins=["*"], allow_credentials=True)`); settings reached through a parameter or `self`; a
settings validator that fills in `"*"` (zenml's server config); `allow_credentials` read from a setting; positional
middleware options; a value imported from a module of a namespace package (a directory without `__init__.py`), whose
module name the run takes from its path (the middleware subclass itself is matched there).

**Overlap.** semgrep's `python/fastapi/security/wildcard-cors` flags the `"*"` literal alone and ignores the
credentials, so it also fires on public APIs where the wildcard is safe; flake8-fastapi's CF008 is about middleware
order; Bandit and Ruff have nothing. This rule follows the origins across modules, through environment defaults and
settings defaults, and requires the credentials.

**Sources.** Starlette's CORSMiddleware source
(https://github.com/encode/starlette/blob/master/starlette/middleware/cors.py); FastAPI's CORS tutorial
(https://fastapi.tiangolo.com/tutorial/cors/); PortSwigger on CORS origin reflection with credentials
(https://portswigger.net/web-security/cors); maintainers who removed the pairing: onyx's `cors_allow_credentials`
("A wildcard origin must never be paired with allow_credentials=True: ... Starlette compensates by echoing arbitrary
request Origins on preflights, which would let any site make credentialed (cookie-authenticated) cross-origin
requests", `backend/shared_configs/configs.py`) and langflow's startup warning about "permissive defaults (all origins
+ credentials)" (`src/backend/base/langflow/main.py`).

**Corpus.** 12 FastAPI repositories; every CORSMiddleware configuration outside tests was read by hand (9 calls, in 9
repositories): 2 matches, both real, precision 2/2. The other 7 stay silent as they should.

| Repository | Commit | Matches | Real | Silent CORS calls (correctly) |
|---|---|--:|--:|---|
| open-webui/open-webui | `0666e59d` | 1 | 1 | |
| rachelos/we-mp-rss (added for this rule) | `8fe5bf93` | 1 | 1 | |
| fastapi/full-stack-fastapi-template | `77e550a7` | 0 | | `[settings.FRONTEND_HOST]` |
| freqtrade/freqtrade | `7fcb509a` | 0 | | `config["api_server"].get("CORS_origins", [])` |
| mealie-recipes/mealie | `8dbb9280` | 0 | | `["http://localhost:3000"]` |
| polarsource/polar | `872272fb` | 0 | | `CORSMiddleware(app, allow_origins=self.allow_origins, ...)` in `kit/cors.py`, fed by `CORSConfig([str(o) for o in settings.CORS_ORIGINS], allow_credentials=True)` and `CORSConfig(["*"], allow_credentials=False)` |
| letta-ai/letta | `11315357` | 0 | | `settings.cors_origins`, a localhost list |
| zenml-io/zenml | `61569b1e` | 0 | | `server_config().cors_allow_origins or []` (a validator fills `"*"`: gap) |
| khoj-ai/khoj (added for this rule) | `ae229ca8` | 0 | | `default_origins + custom_origins`, `"http://localhost:*"` |
| Netflix/dispatch | `dd2837e8` | 0 | | no CORS middleware |
| fastapi-users/fastapi-users | `e9c57ff7` | 0 | | no CORS middleware |
| Buuntu/fastapi-react | `af17f2d4` | 0 | | no CORS middleware |

Real matches:

- open-webui `backend/open_webui/main.py:803`: `BrowserCORSMiddleware` (a CORSMiddleware subclass in
  `utils/asgi_middleware.py`, a namespace package) with `allow_origins=CORS_ALLOW_ORIGIN, allow_credentials=True`,
  where `backend/open_webui/config.py:2209` has `CORS_ALLOW_ORIGIN = os.getenv('CORS_ALLOW_ORIGIN', '*').split(';')`.
  The project logs a warning for this default; an install that never sets the variable runs with it.
- we-mp-rss `web.py:74`: `allow_origins=["*"], allow_credentials=True`. Its API authenticates with a bearer token, so
  the cookie impact there is small; the configuration is still the one the rule exists for, and dropping
  `allow_credentials=True` changes nothing for that API.

Missed (gap): zenml's example pipeline `examples/deploying_ml_model/pipelines/inference_pipeline.py:42`,
`CORSConfig(allow_origins=["*"], ..., allow_credentials=True)`, which zenml's deployer turns into CORSMiddleware.

### `fastapi-practice-http-client-per-request`

Rerail's own rule, not a port. An `httpx.AsyncClient`, `httpx.Client` or `aiohttp.ClientSession` owns a connection
pool. Building one per request pays DNS, TCP and TLS setup on every call, throws the keep-alive pool away, and under
load runs the process out of sockets and file descriptors. Outside test code, the rule reports the constructor call
(bare, in `with`/`async with`, or returned from a factory) of one of those classes, resolved through imports (`httpx2`
and subclasses in the run included), when the call is:

- in a def FastAPI calls per request: a route (any verb, `websocket`, `add_api_route`, a decorator applied by hand), a
  dependency (a function, a class's `__init__`, an instance's `__call__`), or HTTP middleware (`@app.middleware("http")`,
  `BaseHTTPMiddleware.dispatch`);
- in a workspace def such a function reaches through sync or awaited calls (the cross-file call graph), including a
  class's `__init__` when the request path constructs the class (`Weather(url).today()`). The message names the helper
  and one per-request function that reaches it (`fetch_json` ... `via_helper` at `app/routes.py:48`).

Each constructor is reported once. The message tells the agent to create one client for the app (in the lifespan, on
`app.state`, or a module-level client closed on shutdown) and reuse it, passing timeouts and headers per call and
keeping no cookies on a shared client when calls are per user (`aiohttp.DummyCookieJar()`), and to leave the call when
it needs settings a request cannot pass (custom transport, proxy, certificates).

It leaves alone:

- clients built at module level, in the lifespan, or anywhere no per-request function reaches: CLI commands, scripts,
  background work handed to `BackgroundTasks.add_task` or `asyncio.create_task` (an argument, not a call);
- a def decorated `@functools.lru_cache`, `@functools.cache` or `@async_lru.alru_cache`, and code reached only through
  one;
- a client stored for later requests: assigned to a module variable (`global _session; _session =
  aiohttp.ClientSession()`, open-webui's `utils/session_pool.py:72`) or to an attribute outside `__init__`
  (`self._client = httpx.AsyncClient()`, `app.state.http = ...`);
- a client with a transport of its own: `transport=`, `mounts=`, or aiohttp's `connector=`. On the corpus these are
  SSRF-pinning transports that must be built per call (mealie's `safehttp.SafeTransport`, polar's `SSRFSafeTransport`,
  open-webui's `get_ssrf_safe_session()`), mock transports, or a proxy per call;
- a def nested in a route that is never called; other libraries' classes of the same name (`socketio.AsyncClient`,
  `mcp.ClientSession`); `TestClient` and `ASGITransport`; test files.

Overlap: none. Ruff and Bandit (`B113`, `S113`) check request timeouts, not where the client is built, and know no
FastAPI request path. `sqlalchemy` owns engines built per request.

**Gaps:** a call of unknown target is not followed (python/calls' `call_open`): methods on an object a dependency
provides (letta's `server.agent_manager...`, so its `services/webhook_service.py` and `llm_api/` clients are not
reported), callables held in containers or attributes. Modules in a directory without `__init__.py` (open-webui's
`routers/` and `utils/`) get no cross-file call targets, so constructors in helpers there are reported only when a
same-file per-request function reaches them (`openai.py:94`, `auths.py:1586`); `pipelines.py:71,137` and the
`utils/oauth.py` sessions are missed. A helper is followed through every branch, so a constructor on a rare path (an
error fallback) is reported. A client kept in a dict or another container is not recognised as shared. A `proxy=`
keyword is not excluded (polar's Discord OAuth client sets a proxy from settings, which a shared client can carry too).

**Sources:**
- aiohttp, [Client quickstart](https://docs.aiohttp.org/en/stable/client_quickstart.html): "Don't create a session per
  request. Most likely you need a session per application which performs all requests together."
- HTTPX, [Clients](https://www.python-httpx.org/advanced/clients/) and [Async support](https://www.python-httpx.org/async/):
  a client reuses its connection pool; "make sure you're not instantiating multiple client instances".
- Starlette, [Lifespan](https://starlette.dev/lifespan/): an `httpx.AsyncClient` created in the lifespan and handed to
  requests through the lifespan state.
- [Kludex/fastapi-tips](https://github.com/Kludex/fastapi-tips): create the `httpx.AsyncClient` once, in the lifespan,
  and reuse it in the endpoints.

**Corpus** (`~/rerail-bench/python-arch/corpus`, commits from `MANIFEST.tsv`):

CORPUS_TABLE

Every match was read. Real ones: open-webui's model-list helper `send_get_request` (`routers/openai.py:94`, run once per
configured connection on each model-list request), `get_models` and `verify_connection` (`routers/openai.py:844,1038`),
the pipelines admin routes (`routers/pipelines.py:246,316,366,409,449,496,537`), `unload_model` (`main.py:997,1039`),
the version check (`main.py:2755`), sign-out and token exchange (`routers/auths.py:977,1586`), and the tool and function
loaders and verifiers (`routers/tools.py:491`, `routers/functions.py:126`, `routers/configs.py:387,436`), all beside its
own shared pool `utils/session_pool.py:54-72`; letta's Anthropic and Z.ai proxies and the git HTTP proxy
(`server/rest_api/routers/v1/anthropic.py:145,201,314`, `zai.py:141,197,310`, `git_http.py:315`); khoj's web readers
and code sandbox, run from the chat websocket (`processor/tools/online_search.py:577,604,622,640`,
`processor/tools/run_code.py:113,328`; `run_code.py:113` never closes its session); mealie's FlareSolverr escalation
reached from the recipe-scrape routes (`pkgs/safehttp/flaresolverr.py:44`).

Streaming proxies count: a client kept open for a `StreamingResponse` (letta's `stream_response`, `git_http.py:315`)
can be a shared one; only the response is closed per request.

Silent where they should be: mealie's `safehttp` clients (`pkgs/safehttp/transport.py:245`, `fetch.py:185`,
`core/security/providers/openid_provider.py:238`) and polar's (`kit/http.py:169`, `mcp_event/service.py:149`) use an
SSRF-pinning transport per call; open-webui's `get_ssrf_safe_session()` (`retrieval/web/utils.py:283`, a custom
`connector=`) and its shared `get_session()`; freqtrade's `binance_public_data.py` sessions (a custom connector, in a
download job); zenml's only client is in `examples/`.

Why Log: a performance rule, and the fix is not always free: a shared aiohttp or httpx client keeps cookies across
requests, so moving per-user calls onto one needs cookie persistence turned off. Many matches sit on low-traffic admin
routes, where the cost is small.

### `fastapi-practice-on-event-handler`

**Catches** startup and shutdown handlers registered the deprecated way:

- `@o.on_event("startup")`, also applied by hand (`app.on_event("shutdown")(fn)`);
- `o.add_event_handler("startup", fn)` (also `event_type=`), including `app.router.add_event_handler(...)`;
- `on_startup=[...]` / `on_shutdown=[...]` on `FastAPI(...)`, `Starlette(...)`, `APIRouter(...)`, `Router(...)` or a
  subclass of the run (a value of `None` or an empty list is left alone).

The receiver `o` is an app or router (a name bound to its constructor in any module of the run, `self.<attr>`, a
parameter annotated `FastAPI`/`APIRouter`, or `app.router`), or any receiver when the event is the literal
`"startup"` or `"shutdown"`: letta registers on an unannotated parameter `app`. It reports one of two messages:

1. **never runs**: the registration is on an app built with `lifespan=`. The receiver refers to that constructor, or
   it is a parameter of a function that some call of the run passes such an app (positionally or by keyword; the
   call graph of `python/calls`). FastAPI then runs only the lifespan, so the handler is dead code that looks alive.
   The message tells the agent to move its body into the lifespan (before / after `yield`) or delete it.
2. **deprecated**: every other registration. The message tells the agent to move startup code before and shutdown
   code after the `yield` of an `@asynccontextmanager` lifespan passed as `FastAPI(lifespan=...)`, and to move all of
   the app's handlers at once, since a lifespan silences the ones left behind.

```python
app = FastAPI(lifespan=lifespan)

@app.on_event("startup")          # reported: never runs
async def warm_cache(): ...

legacy = FastAPI(on_startup=[init_db])   # reported: deprecated

def install(app):
    @app.on_event("shutdown")     # reported: never runs if a call passes an app with lifespan=, else deprecated
    async def stop(): ...
```

For an unannotated parameter the rule follows the callers (the call-graph choice), not "the run has exactly one app
constructor": a helper nobody calls, or one called only with apps without a lifespan, gets the deprecated message.

**Why.** FastAPI's docs on lifespan events: "If you provide a `lifespan` parameter, `startup` and `shutdown` event
handlers will no longer be called. It's all `lifespan` or all events, not both." `on_event` is deprecated since
Starlette 0.26, and Starlette 1.0 removed `on_event`, `add_event_handler`, `on_startup` and `on_shutdown`; FastAPI
keeps its own deprecated copy (`fastapi/routing.py`, `_DefaultLifespan`, "kept for backward compatibility after
Starlette removed support"). A plain `Starlette(on_startup=...)` fails on Starlette 1.0. Agents still write
`on_event` from older tutorials, and an existing `lifespan=` makes the new handler silently dead.

A router's handlers are deliberately **not** reported as never running: FastAPI's `include_router` merges the
router's own default lifespan, which runs the router's `on_startup` handlers, into the including app's lifespan
(`_merge_lifespan_context` in `fastapi/routing.py`, checked in FastAPI 0.124 to 0.162), so they still run under an
app's lifespan and get the deprecated message only.

**Leaves alone** `on_event` / `add_event_handler` of unrelated classes: an event bus of the run that defines those
methods itself (`bus.on_event("startup", fn)` resolves to it), and calls on a receiver that is no app or router with a
non-literal event (`events.on_event(handler)`, `self.on_event(self.name)`); Slack bolt's `app.event(...)`, socketio's
`sio.on(...)`; `lifespan=` and `@asynccontextmanager async def lifespan`; and test files.

**Gaps.** An app passed down through a second helper (`setup(app)` calling `install(app)`): the inner helper gets the
deprecated message; a parameter after `/` or after a comment inside the signature; `app.router.on_startup.append(fn)`;
a sub-app mounted with `app.mount(...)`, whose lifespan Starlette never runs at all; an event bus of an installed
library (not the run) called with the literal `"startup"`/`"shutdown"` (none in the corpus).

**Overlap.** None: Ruff's FAST rules have no `on_event` check, and FastAPI only emits a `DeprecationWarning` at
runtime, which says nothing about the handler being dead under a lifespan.

**Sources.** FastAPI, Lifespan Events (https://fastapi.tiangolo.com/advanced/events/); Starlette, Lifespan
(https://starlette.dev/lifespan/) and release notes (https://starlette.dev/release-notes/, 0.26.0 deprecation and
1.0.0 removal); Kludex/fastapi-tips (https://github.com/Kludex/fastapi-tips), "use lifespan state instead of
`app.state`" and the lifespan examples; FastAPI's source, `fastapi/routing.py` (`_DefaultLifespan`,
`_merge_lifespan_context`, `include_router`).

**Corpus.** CORPUS_TABLE

### `fastapi-practice-request-session-in-background-task`

Rerail's own rule, not a port. Background tasks run after the response is sent. The yield dependency or middleware
that owns the request's SQLAlchemy session commits, rolls back and closes it around that response: before the task in
FastAPI 0.106-0.117, racing it since 0.118 (and under a `BaseHTTPMiddleware`, whose `call_next` returns before the
task has run). A task handed that session works on a closed or reset one: its writes escape the request's commit or
rollback handling, objects loaded in the request are detached (lazy loads raise `DetachedInstanceError`, or
`MissingGreenlet` on an `AsyncSession`), and an `AsyncSession` can be used by two tasks at once.

In a route, a dependency or HTTP middleware, outside test code, a call that schedules work (`bg.add_task(fn, ...)` on
a `BackgroundTasks` parameter, or `BackgroundTask(fn, ...)`) is reported:

- at an argument after the callable, positional or keyword, that is the request's session: a parameter holding it
  (annotated `Session`/`AsyncSession`, through `Annotated` or an alias such as dispatch's
  `DbSession = Annotated[Session, Depends(get_db)]` in another module, or filled by a dependency that yields or
  returns one), a local alias of it (`s = db`), or `request.state.<x>` when the run assigns a session to
  `.state.<x>`;
- at the callable, when the callable itself reads such a session: a lambda or nested def reading it from the
  enclosing function, `functools.partial(task, db)`, or a bound method of an object built in the same function from
  it (`ReportService(db).run`, or `svc = ReportService(db)` then `svc.run`);
- either way, for a session the function opened with `with SessionLocal() as s` (or `async with`): the block closes it
  before the task runs, and the message says so.

The message tells the agent to pass ids or plain values and open a new session inside the task (`with SessionLocal()
as db:`), committing in the request first if the task must read what the request wrote.

It leaves alone: ids, slugs and plain values (`case_id=case.id`, a value read from the session); a task that opens its
own session; a session the function opens without `with` and hands to the task (`s = SessionLocal()`: the task owns
it); `add_task` on something that is not a `BackgroundTasks` parameter (`queue.add_task(...)`); `requests.Session`
and other non-SQLAlchemy sessions, told apart by their import; a plain helper that takes `bg` and `db` as parameters
(not a route or dependency); test files.

Overlap: none. Ruff's FAST rules, Bandit and mypy know nothing of background tasks or of who owns a session.

**Gaps:** a session reached through `self` in a class-based view (mealie's controllers: `RecipeBulkScraperService(
self.service, self.repos, ...)` then `bg_tasks.add_task(bulk_scraper.scrape, bulk)` at
`routes/recipe/recipe_crud_routes.py:516` is not reported: the repos come from the controller's own dependency, which
the rule does not follow through `self`); a session inside a container, or an object the function did not build from
it; a `BackgroundTasks` held in an attribute (`self.bg.add_task(...)`); the `Request` itself passed to a task that
reads `request.state.db` later; a helper that schedules the task, called from a route with the request's session.

**Sources:**
- FastAPI release notes, [0.106.0](https://fastapi.tiangolo.com/release-notes/): "Using resources from dependencies
  with yield in background tasks is no longer supported. … instead of using the same database session, you would
  create a new database session inside of the background task … you would pass the ID of that object and then obtain
  the object again inside the background task function."
- FastAPI docs, [Advanced Dependencies](https://fastapi.tiangolo.com/advanced/advanced-dependencies/): when the exit
  code of a dependency with `yield` runs relative to the response and background tasks (changed again in 0.118).
- FastAPI docs, [Background Tasks](https://fastapi.tiangolo.com/tutorial/background-tasks/), and Starlette,
  [Background Tasks](https://starlette.dev/background/): tasks run after the response is sent.
- FastAPI discussion [#11107](https://github.com/fastapi/fastapi/discussions/11107), "Context managers in `Depends`
  are broken after 0.106": resources a yield dependency hands out are already closed when background tasks use them.

**Corpus** (`~/rerail-bench/python-arch/corpus`, commits from `MANIFEST.tsv`):

| Repository | Commit | Matches | Real |
|---|---|--:|--:|
| Netflix/dispatch | `dd2837e8` | 4 | 4 |
| Buuntu/fastapi-react | `af17f2d4` | 0 | 0 |
| fastapi/full-stack-fastapi-template | `77e550a7` | 0 | 0 |
| fastapi-users/fastapi-users | `e9c57ff7` | 0 | 0 |
| open-webui/open-webui | `0666e59d` | 0 | 0 |
| freqtrade/freqtrade | `7fcb509a` | 0 | 0 |
| mealie-recipes/mealie | `8dbb9280` | 0 | 0 |
| letta-ai/letta | `11315357` | 0 | 0 |
| zenml-io/zenml | `61569b1e` | 0 | 0 |
| polarsource/polar | `872272fb` | 0 | 0 |
| khoj-ai/khoj | `ae229ca8` | 0 | 0 |
| rachelos/we-mp-rss | `8fe5bf93` | 0 | 0 |

Every match was read; all four are real. dispatch's case routes pass `db_session=db_session` (a `DbSession`
parameter: `Annotated[Session, Depends(get_db)]`, where `get_db` returns `request.state.db`, the session
`db_session_middleware` in `src/dispatch/main.py` opens, commits and closes around `call_next`) to
`case_new_create_flow` (`src/dispatch/case/views.py:229`), `case_create_resources_flow` (`:277`),
`case_remove_participant_flow` (`:458`) and `case_add_or_reactivate_participant_flow` (`:482`). The last two are
wrapped in dispatch's `@background_task`, which opens a session of its own only when none is passed, so they too run on
the request's.

Silent where they should be: dispatch's other 25 `add_task` calls (case, incident and project views) pass ids,
emails, Pydantic read models and `organization_slug`, and let `@background_task` open the session; its Slack endpoints
schedule `handler.handle` with the `Request`; open-webui's nested `background_handler`s read no session
(`routers/channels.py:1514`, `routers/files.py:485`: "We intentionally do NOT pass db to background_handler");
freqtrade's and zenml's tasks take ids, paths and config, khoj's a Django user; `BackgroundTask(rmtree, temp_path)`-style
cleanups (mealie, zenml, letta); we-mp-rss's `TaskQueue.add_task` is not a `BackgroundTasks`. The one known miss is
mealie's bulk scraper (Gaps).

Why Steer: 4 matches, all read, all real (100%), and the fix (pass ids, open a session in the task) is the one the
FastAPI release notes give.

### `fastapi-practice-response-exposes-secret-field`

Rerail's own rule, not a port. FastAPI's response model is its output filter: what a route returns is validated into
the model and serialized with the model's fields only. The rule reports a route whose response can carry a password
hash or a stored secret to the client:

1. **Its response model declares one** (reported at the model): `response_model=X`, else the return annotation,
   names a class that declares such a field, also inside `List[X]`, `list[X]`, `Page[X]`, `Optional[X]`, `X | None`,
   `Annotated[X, ...]` and module aliases (`UserList = list[UserInDB]`). Routes are `@router.<verb>(...)` on an app or
   router of the run and `add_api_route(...)`.
2. **It has no response model and returns a row that declares one** (reported at the `return`): no `response_model=`
   and no return annotation, `-> Any`, or `response_model=None`, returning `db.get(User, id)` (awaited too),
   `db.query(User)....first()/one()/one_or_none()/all()/get()`, `session.exec(select(User)).first()`,
   `session.scalars(select(User)).all()`, `.scalars().all()` on a name assigned `await session.execute(select(User))`,
   `User(...)`, a parameter a dependency fills (`current_user: CurrentUser`, the alias in another module), a call of a
   function of the run annotated to return an ORM model (`-> Optional[User]`), or a local name whose last assignment
   before the `return` is one of those.

A class declares a field when its body or a base class's in the run (mixins, `SQLModel` bases in a third module)
assigns or annotates the name: SQLAlchemy `Column(...)`, `mapped_column(...)`, `Mapped[...]`, SQLModel and pydantic
fields. Names match whole:

- `password`, `hashed_password`, `password_hash`, `passwd`: in any model.
- `secret`, `client_secret`, `api_key`, `secret_key`, `private_key`, `otp_secret`, `totp_secret`, `mfa_secret`,
  `access_token`, `refresh_token`: only on an ORM model (`table=True`, `__tablename__`, a `Column`/`mapped_column`/
  `Mapped` field, a subclass of `DeclarativeBase` or of a `declarative_base()` result), that is, the stored row sent
  as it is.

It leaves alone: a response model without the field (`response_model=UserPublic`, the idiom of the FastAPI template);
a response model given beside a return annotation that names the table model (the response model wins); pydantic
schemas written to carry a key or token (a login route's `Token`, a new API key shown once, a TOTP enrolment secret,
Stripe's `client_secret`, which the browser needs); fields typed `SecretStr`/`SecretBytes` or `ClassVar`; fields with
`Field(exclude=True)` (as the value or in `Annotated` metadata); `response_model_exclude` naming the field,
`response_model_include` leaving it out, and either one given as a variable; names that only contain a secret word
(`password_changed_at`, `password_reset_required`, `has_password`, `secret_name`); request bodies, also when a route
echoes one back; hand-built `Response`/`JSONResponse` objects; `select(User.email)` and multi-entity rows
(`query(User, Hero)`); helpers that are not routes; test files.

**Gaps:** secret fields of nested models (`owner: User` inside the response model); dicts and lists built around a
row (`{"user": user}`); generic CRUD helpers whose return type is a TypeVar, and methods called on instances
(`Users.get_user_by_id(...)` on a module-level table object); a field whose column name differs from the attribute
(`_password = Column("password")`); pydantic v1 `Config.fields` exclusions; a field named for a token in general
(open-webui's `OAuthSession.token`, a JSON blob with the provider's access and refresh tokens, is returned by the admin
route `routers/users.py:817` through a pydantic model the rule cannot see from the route: neither the name nor the
call is matched). Pydantic response schemas that declare a key name on purpose are left alone by design, so a schema
that mirrors a stored key without meaning to is missed.

**Corpus:** no match in 10 FastAPI repositories, all at the commits in `MANIFEST.tsv`:

| Repository | Commit | Matches |
|---|---|--:|
| `fastapi/full-stack-fastapi-template` | `77e550a73bb4` | 0 |
| `Buuntu/fastapi-react` | `af17f2d422d0` | 0 |
| `fastapi-users/fastapi-users` | `e9c57ff7ff2f` | 0 |
| `open-webui/open-webui` | `0666e59d26a4` | 0 |
| `mealie-recipes/mealie` | `8dbb92808d5a` | 0 |
| `Netflix/dispatch` | `dd2837e82a0b` | 0 |
| `letta-ai/letta` | `1131535716e8` | 0 |
| `zenml-io/zenml` | `61569b1e08e7` | 0 |
| `freqtrade/freqtrade` | `7fcb509a434c` | 0 |
| `polarsource/polar` | `872272fbfc6e` | 0 |

Every repository already follows the advice: the template returns its `User` table model (which declares
`hashed_password`) only through `UserPublic`/`UsersPublic`, dispatch returns `DispatchUser` (a `password` column)
through `UserRead`, mealie returns `PrivateUser` (a `password` field) through `UserOut`, Buuntu returns `schemas.User`
rather than `UserCreate`. A first draft that counted key names on any model matched 36 routes, none real: polar's 31
(`Checkout`/`CheckoutPublic`/`CheckoutLink` and payment schemas carrying Stripe's `client_secret` for the browser,
`WebhookEndpoint.secret` shown to the endpoint's owner, `TOTPEnrollment.secret` for the QR code) and letta's 5
(`routers/v1/providers.py:17`, `:54`, `:67`, `:88`, `:147`: `Provider.api_key` is deprecated and the provider manager
sets it to `None` before every return). Key names now count only on ORM models, which removed all 36. To check recall
on real code, a copy of the template with two routes changed back to the unsafe shapes (`/me` with
`response_model=User`, `/{user_id}` with no response model returning `session.get(User, user_id)`) gives 3 matches,
all expected: `backend/app/api/routes/users.py:124`, `:171`, `:179`, through the `CurrentUser` alias in `deps.py` and
the table model in `models.py`.

**Sources:**
- FastAPI docs, [Response Model - Return Type](https://fastapi.tiangolo.com/tutorial/response-model/): "Now, whenever a browser is creating a user with a password, the API will return the same password in the response. … we could be sending our user's passwords to every client."
- FastAPI docs, [Extra Models](https://fastapi.tiangolo.com/tutorial/extra-models/): "The output model should not have a password."
- FastAPI docs, [SQL (Relational) Databases](https://fastapi.tiangolo.com/tutorial/sql-databases/): the `Hero` table model is returned through `HeroPublic`, which leaves out `secret_name`.
- OWASP API Security Top 10 2023, [API3 Broken Object Property Level Authorization](https://owasp.org/API-Security/editions/2023/en/0xa3-broken-object-property-level-authorization/): an API that returns an object's sensitive properties to a client that should not read them.

### `fastapi-practice-route-missing-sibling-auth`

Rerail's own rule, not a port. FastAPI runs a route's dependencies before the route, and authorization is one of
them, written per route: `user: CurrentUser`, `Depends(get_current_user)`, `Security(...)`, or the decorator's
`dependencies=[...]`. A write route that leaves it out while its neighbours carry it is published without
authorization (OWASP API5, Broken Function Level Authorization).

The rule reports, outside test code, a POST, PUT, PATCH or DELETE route (a decorator's verb, or `methods=` of
`api_route`/`add_api_route`) on an app or router when at least 3 other routes of that router require an auth
dependency and no other route of the router goes without one (public routes aside). A route requires auth when a
dependency FastAPI runs for it, at any depth (its parameters', its decorator's, and theirs), is:

- a `Security(...)` call;
- a `fastapi.security` scheme instance (`OAuth2PasswordBearer`, `HTTPBearer`, `APIKeyHeader`, `HTTPBasic`, ..., or a
  subclass), named by `Depends(scheme)` or built in place;
- a dependency whose name says it authenticates: `get_current_user`, `get_verified_user`, `current_active_user`,
  `get_*_user`, `require_*`, `*_required`, `*auth*` (not `author`), `*permission*`, `*token*`, `*jwt*`, `*bearer*`,
  `*api_key*`, `*superuser*`, `*admin*`, `*login*`, role and scope checkers. The name is the one the `Depends` call
  gives (a function, a variable such as fastapi-users' `current_active_user`, an instance's class), the factory
  that builds the instance a variable holds (polar's `_CustomerWrite = Authenticator(...)`), or the def it runs.

A router whose constructor (`APIRouter(dependencies=[...])`, `FastAPI(dependencies=[...])`) or an
`include_router(router, dependencies=[...])` of it, or of a router that includes it, lists an auth dependency is
covered: none of its routes is reported. Includes are followed across modules, also from routers in directories
without `__init__.py` (dispatch's `api.py` includes `dispatch/case_cost/views.py`'s router that way).

It leaves alone:

- routes whose name or path says they are public: login, logout, signin, signup, register, token, refresh, oauth,
  oidc, callback, sso, saml, authorize, mfa, otp, challenge, webhook, health, ready, live, ping, docs, openapi,
  public, verify, confirm, reset, forgot, recover, invite, unsubscribe, jwks, `.well-known`, magic links, captcha;
- routes that read credentials themselves: a `Header(...)` parameter named for auth, a token, a key, a signature, a
  secret or the client IP (`x_forwarded_for`), any `Cookie(...)` parameter, `request.headers` / `request.cookies` in
  the body, a call in the body of a helper named for auth, verification or an IP allowlist
  (`get_current_user(request)`, `verify_signature(...)`, `decode_token(...)`, `check_ips(...)`), a parameter not
  filled by a dependency named for a token, secret, code, password or API key (`token: str` in the query), or a body
  model, or a base of it, with such a field (polar's `SeatClaim.invitation_token`; `country_code` and the like do not
  count);
- routers with fewer than 3 other authenticated routes, or with a second unauthenticated, non-public route (a router
  that mixes public and private routes on purpose);
- GET routes (they count as siblings, but are not reported), HEAD, OPTIONS and websocket routes;
- test files.

**Why writes only.** On the corpus, every GET outlier of a mostly authenticated router was a public read: open-webui's
`get_app_version`, `get_app_changelog`, `get_manifest_json`, `get_opensearch_xml`, `healthcheck`, the SCIM discovery
documents (`routers/scim.py:421`, `:443`, `:473`) and ollama's `get_status` (`routers/ollama.py:225`). Their names
are open-ended, so a GET outlier cannot be told from a public read by name; a write outlier can.

**Gaps:** auth that FastAPI does not run as a dependency (middleware, a project decorator, a check in a helper whose
name does not say so); a dependency that authenticates under a neutral name; an unprotected GET (we-mp-rss's
`apis/tools.py:321` `download_export_file`, whose `Depends(get_current_user)` is commented out, is missed); a
write whose body carries a token, key or password it stores rather than checks (saving a provider's API key,
changing a password) is left alone; a public write named for nothing in the list (a contact form) beside three
authenticated routes is reported; routes are grouped by the router they are declared on; a router included both with
and without auth counts as covered.

**Corpus:** no match in the FastAPI repositories, at the commits in `MANIFEST.tsv`, after tuning:

| Repository | Commit | Matches |
|---|---|--:|
| `fastapi/full-stack-fastapi-template` | `77e550a73bb4` | 0 |
| `Buuntu/fastapi-react` | `af17f2d422d0` | 0 |
| `fastapi-users/fastapi-users` | `e9c57ff7ff2f` | 0 |
| `open-webui/open-webui` | `0666e59d26a4` | 0 |
| `Netflix/dispatch` | `dd2837e82a0b` | 0 |
| `mealie-recipes/mealie` | `8dbb92808d5a` | 0 |
| `letta-ai/letta` | `1131535716e8` | 0 |
| `freqtrade/freqtrade` | `7fcb509a434c` | 0 |
| `polarsource/polar` | `872272fbfc6e` | 0 |
| `zenml-io/zenml` | `61569b1e08e7` | 0 |
| `rachelos/we-mp-rss` | `8fe5bf938743` | 0 |
| `khoj-ai/khoj` | `ae229ca894c0` | 0 |
| 9 FastAPI apps of `_wild-pydantic-dump` (remnawave-admin, notify-server, Phoenix-Agent, AgentCapture, themecp-backend-v2, tomata, MemoLite, ToolOrchestra, speaches) | `MANIFEST.tsv` | 0 |

Every write route without auth on a mostly authenticated router was public by design, and each draft match was one
of them, now left alone: open-webui's MFA enrollment step (`routers/mfa.py:130`, a challenge token in the form) and
device-login request (`routers/sessions.py:158`), which the words `mfa` and `authorization` now cover; dispatch's
`case_cost/views.py:27`, `case_cost_type/views.py:27`, `email_templates/views.py:35`, `forms/type/views.py:30`, all
covered by `api.py`'s `include_router(..., dependencies=[Depends(get_current_user)])` of routers in directories
without `__init__.py` (the include is now followed there); notify-server's `app/api/services.py:96`, a machine
endpoint behind an IP allowlist (`check_ips(x_forwarded_for)`); polar's `customer_seat/endpoints.py:332`
`claim_seat`, whose body's `invitation_token` is the credential. The other unauthenticated writes beside 3+
authenticated siblings (open-webui's signin, signup, signout, oauth callbacks, webhook; remnawave-admin's 11 login,
OAuth, refresh and password-reset routes; we-mp-rss's login, token and password reset; the template's
`register_user`) are public by name. Precision cannot be measured on zero matches; the rule ships in `log`.

Recall, on copies with an outlier put back: the template with `current_user: CurrentUser` taken out of
`delete_item` gives 1 match (`backend/app/api/routes/items.py:100`); open-webui with `user=Depends(...)` taken out of
`sessions.py`'s `revoke` and `knowledge.py`'s `delete_knowledge_by_id` gives 2 (`routers/sessions.py:311`,
`routers/knowledge.py:1789`).

**Sources:**
- OWASP API Security Top 10 2023, [API5 Broken Function Level Authorization](https://owasp.org/API-Security/editions/2023/en/0xa5-broken-function-level-authorization/): asks "Can a user perform sensitive actions (e.g. creation, modification, or deletion) that they should not have access to …?", and prevents it with an enforcement that should "deny all access by default, requiring explicit grants to specific roles for access to every function."
- FastAPI docs, [Bigger Applications](https://fastapi.tiangolo.com/tutorial/bigger-applications/): dependencies given to `APIRouter(dependencies=[...])` or `include_router(..., dependencies=[...])` run for every route of the router.
- FastAPI docs, [Global Dependencies](https://fastapi.tiangolo.com/tutorial/dependencies/global-dependencies/) and [Dependencies in path operation decorators](https://fastapi.tiangolo.com/tutorial/dependencies/dependencies-in-path-operation-decorators/): `FastAPI(dependencies=[...])` and the decorator's `dependencies=[...]`, the places a route's auth can come from besides its signature.

### `fastapi-practice-route-shadowed`

Rerail's own rule, not a port. Starlette, and so FastAPI, tries a router's routes in the order they were added and
serves the first one whose path pattern and method match. A plain `{param}` matches any one segment, so
`GET /{case_id}` declared before `GET /minimal` answers `/minimal` itself: a 422 when `case_id` must be an int or an ID
of a given shape, the wrong handler otherwise. The later route is never served. FastAPI's documentation calls this out
("Order matters") and its fix is to declare the literal route first.

The rule reports a route (a decorator such as `@router.get(...)`, the decorator applied by hand, or
`router.add_api_route(...)`) when an earlier route of the same router, in the same file and block, with the same
method, matches every path it serves:

- **shadowed:** the earlier path has a parameter where this one has a literal segment, and the other segments are
  equal. `{x}` and `{x:str}` match any segment; `{x:int}` a segment of digits (and a later `{y:int}`); `{x:float}`
  digits with an optional fraction (and a later `{y:int}` or `{y:float}`); `{x:uuid}` a UUID; `{x:path}` the rest of
  the path, slashes included (`/files/{p:path}` before `/files/recent` or `/files/{id}/meta`);
- **duplicate:** the same path, parameter names aside (`/{id}` and `/{item_id}`), and the same method.

Methods compare one by one: the decorator's verb, or `api_route`'s `methods=[...]` (GET when it lists none). HEAD and
GET are distinct, and a websocket route meets only websocket routes. The message names the earlier route and its
line, and tells the agent to declare this route before it or give it another path; for a duplicate, to merge it into
the earlier handler or give it its own path.

It leaves alone:

- a literal route declared before the parametrized one (the documented fix);
- another method, another number of segments, another router; an `:int`, `:float` or `:uuid` parameter before a word
  (`/pages/{page:int}` then `/pages/latest`); a one-segment `{key}` before a `{key:path}`;
- routes on one router declared in different modules: their order is the order the modules are imported in, which
  the rule does not know;
- routes in different blocks: one under `if legacy:` and the other under `else:` (only routes of one module body,
  function body or branch are compared);
- one function under two paths (stacked route decorators);
- **versioned duplicates:** a duplicate where either def carries a decorator besides its route decorators, or whose
  route calls pass a different `name=` or any keyword named for a version. polar declares
  `POST /organizations/` twice (`server/polar/organization/endpoints.py:307` and `:329`), the second with
  `name="create"` and `@version(starting_from=V2027_01)`, and its custom route class picks one by the request's API
  version; both tests keep it silent;
- paths that are not one string literal, paths with a custom converter, test files.

**Gaps:** routes added in a loop or by a project's own helper are not seen; the order between modules that share a
router is unknown, so such pairs are missed; a route only partly shadowed (`/{x}` before `/{y:path}`, whose deeper
paths still reach it) is not reported; a custom route class that matches on more than path and method (a header, a
host) can make a reported pair reachable; a duplicate under a decorator unrelated to routing (a rate limiter) is left
alone with the versioned ones.

CORPUS_PLACEHOLDER

**Sources:**
- FastAPI docs, Path Parameters, [Order matters](https://fastapi.tiangolo.com/tutorial/path-params/#order-matters):
  "Because path operations are evaluated in order, you need to make sure that the path for `/users/me` is declared
  before the one for `/users/{user_id}`. Otherwise, the path for `/users/{user_id}` would match also for `/users/me`
  ... Similarly, you cannot redefine a path operation ... The first one will always be used since the path matches
  first."
- Starlette docs, [Routing](https://starlette.dev/routing/#route-priority), "Route priority": "Incoming paths are
  matched against each `Route` in order. [...] you should take care to ensure that more specific routes are listed
  before general cases."

### `fastapi-practice-sync-session-in-async-route`

Rerail's own rule, not a port. FastAPI runs a `def` route or dependency in its threadpool and an `async def` one on the
event loop, so a query on a sync SQLAlchemy/SQLModel session in an `async def` stalls every concurrent request for its
duration. In an `async def` FastAPI calls per request (a route, also one registered as `app.get(path)(f)`; a
dependency; HTTP middleware), outside test code, in its own frame (not a nested def or lambda) and not awaited, it
reports:

- a blocking session method (`query`, `execute`, `scalars`, `scalar`, `get`, `commit`, `flush`, `refresh`,
  `rollback`, `delete`, `merge`, `exec`, `close`, and scoped_session's `remove`) on a sync session: the parameter
  holding the request's session (annotated `Session`, through `Annotated` or an alias such as `SessionDep` in another
  module, or filled by a dependency that yields or returns a sync session: `db=Depends(get_db)` with `get_db` yielding
  `SessionLocal()`); a session the function opens (`with SessionLocal() as s`, `s = sessionmaker(...)()`,
  `s = open_session()` for a sync def annotated `-> Session` or returning one, `with session_scope() as s` for a
  `@contextmanager` yielding one); `request.state.<x>` when the run assigns a sync session to `.state.<x>`; or a class
  attribute holding one (`Trade.session` declared `session: ClassVar[scoped_session[Session]]`, `self.session`
  assigned from a `Session` parameter). In a chain (`db.query(User).filter(...).all()`) only the root call is reported.
- a call whose target (the cross-file call graph, with no unknown target) is a sync def that does such work on a
  session it receives (a parameter annotated `Session`, or one a caller passes a session to, by position or keyword),
  opens, or reads from an attribute holding one, directly or through further sync defs. Where the call graph has no
  target for `r.p.m()`, it follows a property or cached_property `p` of r's class (proven by the type program) whose
  return annotation names a class of the run (`repos.recipes.get_one(...)`), also through parametrized bases
  (`class RepositoryRecipes(HouseholdRepositoryGeneric[RecipeOut, RecipeModel])`). The message names the helper and the
  first session call it reaches (`get_users()` ... `db.query()` at `app/db/crud.py:23`).

The message gives the two safe fixes: plain `def` when the function awaits nothing and sets no ContextVar the request
reads (FastAPI then runs it in a threadpool), else an `AsyncSession` or `await run_in_threadpool(helper, ...)`.

It leaves alone: `def` routes and dependencies, even calling the same helpers; an `AsyncSession` and awaited calls;
`await run_in_threadpool(helper, db)`, `asyncio.to_thread`, `anyio.to_thread.run_sync`, `loop.run_in_executor`, and
work in a nested def or lambda; `db.add()`, `add_all()`, `expunge()` (no IO); async helpers (calling one only makes a
coroutine); calls of unknown target; a `Session` from another library (`requests.Session`), told apart by its import;
`.state.<x>` attributes that hold no session (`request.state.cache.get(...)`); test files.

Overlap: Ruff's ASYNC2xx know no SQLAlchemy and never follow a helper. Blocking calls that are not session methods
(`inspect(engine)`, sleep, HTTP, file IO) are left to `python-practice-blocking-call-reached-from-async`.

**Gaps:** a session reached through a value neither the call graph nor the type program resolves: a property without a
return annotation (mealie's controllers' `self.mixins`/`self.service`, so `recipe_crud_routes.py:849,979` and
`controller_group_ai_providers.py:81` `self.mixins.get_one(...)` are not reported), a session held in a container or
passed through `**kwargs`; an awaited `async def` helper that does sync session work inside; the chain is followed
through every branch, so a call is reported when any branch of the reached code queries; the location named is the
first session call by position in the run, not necessarily the one the call's arguments take.

**Sources:**
- FastAPI docs, [Concurrency and async / await](https://fastapi.tiangolo.com/async/): "When you declare a path
  operation function with normal `def` instead of `async def`, it is run in an external threadpool that is then
  awaited, instead of being called directly (as it would block the server)."
- FastAPI discussion [#12089](https://github.com/fastapi/fastapi/discussions/12089): a sync session in a `def` route
  runs in the threadpool and does not block; in an `async def` one it does.
- [zhanymkanov/fastapi-best-practices](https://github.com/zhanymkanov/fastapi-best-practices): "if you violate that
  trust and execute blocking operations within async routes, the event loop won't be able to run other tasks".
- BaoDev, [the fastapi + pydantic + sqlalchemy god combo, and where it bites](https://baodev.studio/blog/fastapi-pydantic-sqlalchemy/):
  a sync session in an async route took p99 latency from 40 ms to 1.9 s at about 30 concurrent requests.
- SQLAlchemy 2.0, [Asynchronous I/O (asyncio)](https://docs.sqlalchemy.org/en/20/orm/extensions/asyncio.html): the
  `AsyncSession` for asyncio code.

**Corpus** (`~/rerail-bench/python-arch/corpus`, commits from `MANIFEST.tsv`):

| Repository | Commit | Matches | Real |
|---|---|--:|--:|
| Buuntu/fastapi-react | `af17f2d4` | 9 | 9 |
| Netflix/dispatch | `dd2837e8` | 8 | 8 |
| mealie-recipes/mealie | `8dbb9280` | 11 | 11 |
| freqtrade/freqtrade | `7fcb509a` | 3 | 2 |
| fastapi/full-stack-fastapi-template | `77e550a7` | 0 | 0 |
| fastapi-users/fastapi-users | `e9c57ff7` | 0 | 0 |
| open-webui/open-webui | `0666e59d` | 0 | 0 |
| letta-ai/letta | `11315357` | 0 | 0 |
| zenml-io/zenml | `61569b1e` | 0 | 0 |
| polarsource/polar | `872272fb` | 0 | 0 |

Silent where they should be: the template's routes are `def`; polar and open-webui use `AsyncSession`; zenml's
`track_last_user_activity` middleware hands its store update to `await to_thread.run_sync(...)`
(`src/zenml/zen_server/middleware.py:259`).

Every match was read. Real ones: fastapi-react's `async def` CRUD routes and auth dependency calling `app/db/crud.py`
(`api/api_v1/routers/users.py:31,59,76,92,107`, `routers/auth.py:16,43`, `core/auth.py:29`) and its middleware's
`request.state.db.close()` (`main.py:23`); dispatch's `db_session_middleware` commit, rollback, close and `remove()`
(`src/dispatch/main.py:156,164,182,184`) and the Slack endpoints' `get_request_handler()`, which opens a session with
`refetch_db_session()` and queries it (`plugins/dispatch_slack/endpoints.py:85,110,135,144`); mealie's OIDC and
reverse-proxy logins reaching user lookups and creation (`routes/auth/auth.py:322,328,399,405,440`), the SPA meta
routes (`routes/spa/__init__.py:246,251,264`), and `OpenAIService(self.repos)`, whose constructor queries the
provider settings (`routes/admin/admin_debug.py:38`, `routes/groups/controller_group_ai_providers.py:64,87`);
freqtrade's async dependency `get_rpc` closing the scoped sessions (`rpc/api_server/deps.py:33,34`). The one not
counted, `deps.py:29` `Trade.rollback()`, rolls back a session the request-id scope has just created, which does no
IO. freqtrade's `get_rpc` must stay `async def` (it sets the ContextVar its scoped sessions key on), so the message
names that exception and offers `run_in_threadpool`.

Not reported, by design: dispatch's `inspect(engine).get_schema_names()` in the same middleware (an engine call, not a
session method).

Why Steer: 31 matches, all read, 30 real (97%); the one exception runs no IO and is harmless to fix.

### `fastapi-practice-yield-dependency-cleanup`

Rerail's own rule, not a port. A FastAPI dependency that yields hands its value to the route and runs its exit code
once the route is done. Since FastAPI 0.110, as plain generator semantics have it, an exception the route raises is
thrown into the dependency at its `yield`: code after the `yield` that no `finally` covers does not run, so every
failing request leaks the session or connection it would have released, and repeated errors exhaust the pool
(`QueuePool limit of size 5 overflow 10 reached`) until the app stops serving. An `except` around the `yield` that does
not re-raise hides the error: the client gets a 500 and the server logs nothing.

The rule reads every generator some `Depends(...)` or `Security(...)` of the run names (lib/fastapi's `fa_yield_dep`:
the generator in `db.py`, the `Depends` in a route, or in an `Annotated` alias in `deps.py`; a class-based
dependency's `__call__` too), outside test code, and reports in its own frame:

1. **Cleanup not protected** (reported at the call): a call to `close`, `aclose`, `release`, `remove`, `dispose`,
   `shutdown`, `disconnect`, `terminate`, `cleanup`, `unlink` or `rollback`, awaited or not, after the `yield`, unless
   the `yield` is inside a `with`/`async with` block, the call is in the `finally` of a `try` whose body holds the
   `yield`, the call is in an `except` handler (it runs on the error path), or the call is in the body of a `try`
   around the `yield` whose `finally` or `except` also releases something.
   `db = SessionLocal(); yield db; db.close()` is reported at `db.close()`; so is a close after a
   `try: yield db except SQLAlchemyError: db.rollback(); raise`, and `await pool.release(conn)` after `yield conn`.
2. **Swallowed exception** (reported at the `except`): an `except` clause of a `try` whose body holds the `yield`,
   catching everything (bare `except:`), `Exception`, `BaseException`, or a database or HTTP error class
   (`SQLAlchemyError`, `DBAPIError`, `DatabaseError`, `OperationalError`, `IntegrityError`, `HTTPException`, anything
   from `sqlalchemy.exc`), alone or in a tuple, whose handler can finish without raising: its last statement is not a
   `raise`, or is an `if` without `else`, or one with a branch that can finish.
   `try: yield db except Exception: db.rollback()` is reported.

It leaves alone: `try: yield db finally: db.close()`; `with SessionLocal() as db: yield db` and `async with`;
`try: yield db; db.commit() except: db.rollback(); raise finally: db.close()`; a `commit()` after the `yield` (an
error skips it on purpose, and moving it into `finally` would commit failed requests, so the rule never asks for
that); handlers that re-raise, or raise a new exception, on every path; handlers for other classes
(`except KeyboardInterrupt`); statements after the `yield` that release nothing (logging, metrics); generators no
`Depends` names (`@contextmanager` helpers, pytest fixtures with the same shape); test files.

**Gaps:** cleanup done by a helper the generator calls after the `yield` (`close_all(db)`) is not seen; a `with` around
the `yield` silences every release after it, even one of another resource; a handler that ends in a call that always
raises (`abort(500)`) counts as finishing and is reported; an `except ValueError:` (or another narrow class) that
swallows around the `yield` is not reported; the same unsafe shape in a `@contextmanager` helper or a lifespan
function is out of scope.

**Corpus:** no match in 11 FastAPI repositories, all at the commits in `MANIFEST.tsv`. A throwaway probe listing every
yield dependency lib/fastapi finds (outside tests) counted how many each repository gives the rule to judge (not
counted for zenml: the probe waited in the shared run queue too long). `polarsource/polar` was not run: its run sat in
the shared queue for over 20 minutes behind other workers' big-repository runs.

| Repository | Commit | Yield dependencies | Matches |
|---|---|--:|--:|
| `Buuntu/fastapi-react` | `af17f2d422d0` | 1 | 0 |
| `fastapi/full-stack-fastapi-template` | `77e550a73bb4` | 1 | 0 |
| `fastapi-users/fastapi-users` | `e9c57ff7ff2f` | 9 | 0 |
| `rachelos/we-mp-rss` | `8fe5bf938743` | 0 | 0 |
| `khoj-ai/khoj` | `ae229ca894c0` | 0 | 0 |
| `open-webui/open-webui` | `0666e59d26a4` | 0 | 0 |
| `freqtrade/freqtrade` | `7fcb509a434c` | 1 | 0 |
| `mealie-recipes/mealie` | `8dbb92808d5a` | 1 | 0 |
| `Netflix/dispatch` | `dd2837e82a0b` | 0 | 0 |
| `letta-ai/letta` | `1131535716e8` | 0 | 0 |
| `zenml-io/zenml` | `61569b1e08e7` | not counted | 0 |

Every yield dependency found already follows the advice: Buuntu's `backend/app/db/session.py:16` and mealie's
`mealie/db/db_setup.py:80` close in `finally`, the template's `backend/app/api/deps.py:21` and fastapi-users'
`examples/sqlalchemy/app/db.py:28` use `with`/`async with`, freqtrade's `freqtrade/rpc/api_server/deps.py:24` removes
its scoped sessions in `finally`; dispatch and open-webui hand out a session from `request.state` or a context
manager and have no yield dependency. A text search of the whole corpus for a release call within five lines after a
`yield` and outside `finally`/`except` found 123 lines, most in test code and fixtures; the 7 outside test code that
were read are `lifespan` context managers and streaming generators, not dependencies. Precision cannot be measured
without matches, so the cases carry the proof. To
check recall on real code, a copy of Buuntu's backend with `get_db`'s `try`/`finally` taken off
(`db = SessionLocal(); yield db; db.close()`) gives 1 match, the expected one: `app/db/session.py:19`, through the
`Depends(get_db)` calls in `app/api/api_v1/routers/*.py` and `app/core/auth.py`.

**Sources:**
- FastAPI docs, [Dependencies with yield](https://fastapi.tiangolo.com/tutorial/dependencies/dependencies-with-yield/): "you can use `finally` to make sure the exit steps are executed, no matter if there was an exception or not"; under "Always `raise` in Dependencies with `yield` and `except`": "If you catch an exception using `except` in a dependency with `yield` and you don't raise it again (or raise a new exception), FastAPI won't be able to notice there was an exception" … "the client will see an *HTTP 500 Internal Server Error* response … but the server will not have any logs or any other indication of what was the error".
- FastAPI docs, [Advanced Dependencies](https://fastapi.tiangolo.com/advanced/advanced-dependencies/): when the exit code of a dependency with `yield` runs, and how that changed across versions.
- FastAPI [release notes 0.110.0](https://fastapi.tiangolo.com/release-notes/), breaking change: "refactor dependencies with yield and except to require raising again as in regular Python"; it was reported "as a memory leak when the server had unhandled exceptions".
- SQLAlchemy docs, [QueuePool limit of size <x> overflow <y> reached](https://docs.sqlalchemy.org/en/20/errors.html#error-3o7r): "possibly the most common runtime error", what sessions leaked by failing requests lead to.
