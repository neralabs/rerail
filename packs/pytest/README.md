# Rerail rule pack: pytest

Rerail's own pytest rules, from the pytest docs and maintainers' review advice, for isolated tests, sound fixtures and mocks that check what they claim.

This pack is private while its rules are written and reviewed: it is not published to npm, and is installed from
a checkout with `rerail packs add packs/pytest`.

## Rules

In reading order: first the tests that pass without testing anything (precise, the first cross-file), then shared
fixture state (cross-file through the conftest.py chain), then global state with a one-line fix, then the widest leak
rule. Django settings writes in tests are the django pack's `django-practice-settings-in-test`: it reads Django's own
test runner as well as pytest.

| Rule | Mode | On | Catches |
|---|---|---|---|
| `pytest-practice-patch-misses-from-import` | Steer | yes | pytest practice: a mock patch on the defining module of a name its callers imported with from-import |
| `pytest-practice-patch-decorator-order` | Steer | yes | pytest practice: stacked @patch mock parameters named in decorator order, so each holds the other's mock |
| `pytest-practice-scoped-fixture-mutated` | Steer | yes | pytest practice: a test changes a class-, module- or session-scoped fixture's value |
| `pytest-practice-environ-without-monkeypatch` | Steer | yes | pytest practice: test code changes os.environ without undoing it |
| `pytest-practice-module-state-without-monkeypatch` | Log | no | pytest practice: test code changes a module global or class attribute without undoing it |

## The three leak rules: one definition of a change and of its undo

`scoped-fixture-mutated`, `environ-without-monkeypatch` and `module-state-without-monkeypatch` check one hazard, state
a test changes and does not put back, on three kinds of state. They read it through one definition in
`rules/lib/pytest.rerail`, so they answer each question the same way:

| Question | Answer, for all three |
|---|---|
| What is a change? | An attribute or item store, `del` or augmented assignment, a list/dict/set mutator call (`update`, `pop`, `clear`, `append`, ...), `setattr`/`delattr`; each read as a path below every expression of the written chain (`app.config["X"]` is `.config['X']` below `app`) |
| Where does a change happen inside a test? | In a test, a fixture, a set-up hook, or a helper of a test document that one of them calls (through the call graph). Not in module-level code (it runs once, at import, before the first test), not in a def no test calls (a runner script's `main()`), not in session-wide set-up (pytest hooks, autouse session fixtures and the session fixtures they request, pytest-django's `django_db_setup`) |
| What undoes it? | A write of the same path, or of a container it runs through (`del`, `pop(k, None)`, `clear()`, `update(saved)`, a whole replacement), in undo code: after the yield of a fixture or `@contextmanager`, in a `finally`, in a teardown hook or `__exit__` of the class or a base class, in `teardown_module`, in an `addCleanup`/`addfinalizer` callback (a lambda, a def, a bound method `self._restore`, or `addCleanup(setattr, o, "x", old)`), in a def that undo code calls, in the teardown of a function-scoped fixture the test requests or of an autouse one; or a patch active where the change runs (monkeypatch, `patch.object`, `patch.dict`, as a call, a with, a decorator, a patcher started in `setUp`) |
| A restore by hand at the end of the test, no `finally` | Reported, once, at the first change: a failing assert skips the restore |
| The undo code itself | Never reported |
| A computed key in a loop in undo code (`for k in saved: os.environ[k] = saved[k]`) | Restores every key |

Each rule then adds what only its kind of state needs: `monkeypatch.setenv` and `os.putenv` for the environment, the
resolution of module globals and class attributes through imports for module state, the fixture's scope through the
conftest.py chain for fixture values.

## Rule details

### `pytest-practice-patch-misses-from-import`

**Catches** a mock patch aimed at the module that defines a name, when the code the test runs copied that name with a
top-level `from pkg.mod import name`. The copy is an assignment: patching `pkg.mod.name` afterwards replaces a name that
module's callers never read again, so the code under test still sends real e-mail or makes real HTTP calls, and
`mock.assert_not_called()` passes vacuously. The forms read: `mock.patch("pkg.mod.name")` (decorator, context manager,
`.start()`), `mocker.patch("pkg.mod.name")`, `patch.object(mod, "name")`, `mocker.patch.object(mod, "name")`,
`monkeypatch.setattr("pkg.mod.name", v)` and `monkeypatch.setattr(mod, "name", v)`.

It reads across files: the target string is resolved to a workspace module that binds the name; the run's non-test
modules that copied it with a top-level from-import are collected, following re-exports (a package `__init__.py`
doing `from .sms import send_sms` passes its copy on); and the rule reports only when the test file imports one of
those modules, which the message names with the patch that works (`patch("<that module>.name")`). The message also
says to drop the patch when the test never runs that module.

```python
# shop/orders.py
from shop.mail import send_email

def place(order):
    send_email("a@example.com", str(order))

# tests/test_orders.py
from shop.orders import place

@mock.patch("shop.mail.send_email")        # reported: patch "shop.orders.send_email"
def test_place(send):
    place(1)
    send.assert_called_once()
```

**Leaves alone:**

- the name patched where it is read (`patch("shop.orders.send_email")`), or patched in both places by the same test
  or fixture;
- targets outside the workspace (`requests.get`): the run cannot see the library's importers;
- names any module reads as `mod.name` (`http.post(...)`, `getattr(mod, "name")`), names the defining module reads
  itself, and names imported inside a function (that import runs after the patch);
- attribute-of-attribute targets (`patch("shop.http.Client.get")`): a class attribute reaches every holder of the class;
- names the module does not bind (`patch` raises then), `patch.dict`, `monkeypatch.delattr`; `create=True` and
  `raising=False` change nothing when the module binds the name, so those patches are read like any other;
- names a Django management command imports: `call_command` imports the command's module when it runs, inside the
  patch, so its copy is the mock;
- a test that imports an importer in its own body while the patch is active, or a test file that reloads modules
  with `importlib` (both load the copy under the patch);
- copies read only in type annotations;
- a test file that imports no module holding a copy: on the corpus these were dead patches (the test never reaches an
  importer) or patches guarding the defining module's own code (`mod.run()` then `helper.assert_not_called()`).

**Gaps:** a patch made in a conftest.py fixture (an autouse "no real e-mail" guard) is not read against the test files
under that conftest.py, which import the importers; tests that reach the importer only through a test client or a
fixture, with no import of it in the test file, are not reported (`fastapi__full-stack-fastapi-template`
`backend/tests/api/routes/test_users.py:41` patches `app.utils.send_email` while the route imports it by name: real,
missed); when any module reads `mod.name`, the rule is silent even if the test runs another module's copy
(`sissbruecker__linkding` `bookmarks/tests/test_bookmarks_tasks.py:819`); a test module that imports the name itself
and calls it misses the patch the same way, but test modules are not read as importers; `from m import *` copies and
names read through `sys.modules` or a dotted string are not seen.

**Overlap:** none. No linter resolves patch targets; Ruff's PT family (ported in the `python` pack) has no such
check, and PT008 is about `lambda` as the patch's `new`.

**Sources:** unittest.mock, ["Where to patch"](https://docs.python.org/3/library/unittest.mock.html#where-to-patch)
("patch where an object is looked up, which is not necessarily the same place as where it is defined"); Ned
Batchelder, ["Why your mock doesn't work"](https://nedbatchelder.com/blog/201908/why_your_mock_doesnt_work.html)
(`from mod import val` is an assignment).

**Corpus:** 14 matches in 3 of the 20 repositories, 14/14 real. Re-measured after this pack's last changes on the 13
repositories under 1,000 `.py` files: `freqtrade` 1 (`tests/rpc/test_rpc_emc.py:298` patches
`freqtrade.rpc.api_server.ws.channel.create_channel`, which `external_message_consumer.py` imports by name), the
other 12 none. Not re-run (1,000 files or more; the independent review's reading, all real): `apache/superset` 8
(`tests/unit_tests/commands/databases/oauth2_test.py:78` patches `superset.utils.oauth2.decode_oauth2_state` beside a
correct patch of the importer), `saleor/saleor` 5 (`saleor/warehouse/tests/test_stock_management.py:837`, `:1249`,
`:1270`: a vacuous `assert_not_called()` on the back-in-stock webhook; `saleor/plugins/webhook/tests/test_webhook.py:1962`;
`saleor/graphql/webhook/tests/mutations/test_webhook_trigger.py:130`). The `create=True` / `raising=False` change
adds no match on the re-measured repositories.

### `pytest-practice-patch-decorator-order`

Rerail's own rule, not a port. Stacked `@mock.patch(...)` / `@mock.patch.object(...)` decorators pass their mocks
bottom-up: the decorator nearest the `def` gives the first argument. A test whose mock parameters are named after the
targets in top-down order gives each parameter another target's mock, so its set-ups go to the wrong mock and its
asserts check the wrong one, and the test passes without testing anything.

```python
@mock.patch("shop.tasks.cache")
@mock.patch("shop.tasks.statsd")
def test_cache_miss(self, celery_cache, celery_statsd):   # reported: celery_cache holds the statsd mock
    celery_cache.get.return_value = None                   # sets up statsd, not the cache
```

It matches a test (a function or method `test*` in test code, as `pt_test` reads it) with two or more decorators that
pass a mock: `unittest.mock.patch` (also the `mock` package), `patch.object`, without a replacement (`new`, or the
second positional argument); `new_callable` and `autospec` still pass one. Each such decorator's target name is the
last dotted part of the patch string (`target=` included), or `patch.object`'s attribute. Target and parameter names
are compared lower-cased with every underscore removed, so `mock_exists` matches `os.path.exists`, `mockExists` and
`exists_mock` do too, `mock_assert_true` matches `assertTrue` and `mock_email_client` matches `EmailClient`. The first
N positional parameters after `self`/`cls` (N mocks) are read: the rule reports when every parameter's name contains
its top-down target and one does not contain its bottom-up target. It reports at the first such parameter, saying
which target's mock it really receives, and asks for the parameters in bottom-up order and a rerun of the test (its
asserts were checking the wrong mock, so it may now fail).

It leaves alone: bottom-up names; names that follow neither order (`m1, m2`); two targets with the same name
(`shop.mail.send` and `shop.sms.send`); a parameter whose name contains both of its candidate targets (`mock_user`,
`mock_user_email` over `user` and `user_email`); a single mock; fewer positional parameters than mocks (`*mocks`);
marks, `patch.dict`, `patch.multiple` and patches with `new` between the others (skipped: only the mock-passing
patches count, in their order); a target given as a variable; parameters after the mocks (fixtures); defs that are not
tests, and `test*` defs outside test code; `patch` names that do not resolve to `unittest.mock` or `mock`.

**Gaps:** patches on a test class (their mocks follow the method's own and are not read); `patch.multiple` with
`DEFAULT`; a swap hidden by a parameter that matches both targets (`mock_send`, `mock_send_email`); names matched by
substring, so a short target (`get`, `id`) can match a parameter by accident, which only reports when that accident
also lines up with the top-down order.

**Overlap:** none. Ruff's flake8-pytest-style (PT), PGH005 and B003, pylint and flake8 have no check on the order of
patch parameters.

**Sources:**
- Python docs, [unittest.mock: Nesting patch decorators](https://docs.python.org/3/library/unittest.mock.html#nesting-patch-decorators):
  "When you nest patch decorators the mocks are passed in to the decorated function in the same order they applied
  (the normal Python order that decorators are applied). This means from the bottom up".
- Ned Batchelder, [Why your mock still doesn't work](https://nedbatchelder.com/blog/202202/why_your_mock_still_doesnt_work.html):
  the stacked-decorator mix-up, whose asserts on the wrong mock still pass.
- Stack Overflow, [Mock patches appearing in the wrong order?](https://stackoverflow.com/questions/47042196/mock-patches-appearing-in-the-wrong-order).

**Corpus:** 4 matches in 2 of the 20 repositories, 4/4 real, all in repositories of 1,000 files or more and not re-run
(the independent review's reading); the 13 re-measured repositories under 1,000 files have none, before and after
the underscore-free comparison. An independent `ast` scan of the corpus (2,558 tests with two or more mock-passing
patches) found the same 4 and none more with underscores removed.

- `src/olympia/amo/tests/test_celery.py:133` (addons-server): `@mock.patch('olympia.amo.celery.cache')
  @mock.patch('olympia.amo.celery.statsd') def test_handle_cache_miss_for_stats(self, celery_cache, celery_statsd)`:
  the cache-miss set-up `celery_cache.get.return_value = None` goes to the statsd mock, and
  `assert not celery_statsd.timing.called` checks the cache mock, so the assert can never fail.
- `src/olympia/amo/tests/test_monitor.py:70` (addons-server): `@patch('os.path.exists') @patch('os.access') def
  test_path(self, mock_exists, mock_access)`: swapped names; the body does not read them yet.
- `saleor/payment/gateways/stripe/tests/test_webhooks.py:1075` and `:1315` (saleor): the `order_voided` /
  `order_refunded` spies swapped; both asserts compare `.called` with the same expectation, so they pass only because
  the two functions are called together.

### `pytest-practice-scoped-fixture-mutated`

**Catches** a test, or a function-scoped fixture, that changes the value of a fixture with a broader scope
(`scope="class"`, `"module"`, `"package"`, `"session"`). It reads the value through the parameter that requests it,
through a name bound to `request.getfixturevalue("app")`, and through the parameter of a plain function of a test
document the test hands it to (`_enable_feature(app)`). A change is the library's: an attribute or item store, a
`del`, an augmented assignment, `setattr`/`delattr`, or a list, dict or set mutator called on the value or on a chain
over it (`app.config["X"] = 1`, `app.appbuilder = MagicMock()`, `del data["k"]`, `items.append(x)`,
`app.config.update(...)`). The fixture is resolved by name the way pytest resolves it: the test's class first, then
its module, then the `conftest.py` files from the test's directory up to the root, so a nearer function-scoped override
hides a broad fixture. pytest creates such a fixture once and gives every test of its scope the same object, so the
change leaks into the tests that run after it and results depend on test order. The message names the fixture and
its scope, and suggests `monkeypatch.setitem`/`monkeypatch.setattr`, which are undone after the test, or a
function-scoped fixture that copies the value; it says to keep a change to a fixture that collects state on purpose.

```python
# conftest.py
@pytest.fixture(scope="module")
def app():
    return create_app()

# tests/test_auth.py
def test_resolver(app):
    app.config["RESOLVER"] = fake_resolver       # reported: every later test of the module gets the fake
    ...

def test_resolver(app):
    app.config["RESOLVER"] = fake_resolver       # left alone: put back in the finally
    try:
        ...
    finally:
        app.config.pop("RESOLVER", None)
```

**Leaves alone:**

- Function-scoped fixtures, and fixtures that no `conftest.py` or test module of the run defines (plugins).
- Changes put back the way the three leak rules agree on (above): `pop(k, None)`, `del` or `clear()` in a `finally` or
  after a fixture's yield, a finalizer `lambda: setattr(app, "debug", old)`, a helper the `finally` calls with the
  fixture, the teardown of a function-scoped fixture the test requests, `monkeypatch.setitem/delitem/setattr`,
  `patch.object` or `patch.dict` over the container.
- `monkeypatch.setitem(app.config, …)` itself, and writes to a local copy (`cfg = dict(app.config)`).
- `pop(key, default)` outside undo code: a defensive removal of a key another test may have set.
- Method calls that are not collection mutations, by their shape and the fixture's name (the value has no type the run
  can resolve: a factory's return, a plugin's object):
  - HTTP-client calls (`api_client.delete(...)`; `delete`, `save` and `post` are not mutators);
  - database-session calls (`app.db.add(row)`, `db.session.remove()`);
  - `update(key, obj)` with two positional arguments, and mutator calls with keyword arguments;
  - calls whose result is used (`agent = client.agents.update(...)`);
  - any method call through a fixture named `client`, `*client`, `api`, `*_api` or `sdk`.
- A broader-scoped fixture changing its own broader-scoped dependency. This is set-up that runs once for its scope.
- An autouse fixture changing a class- or module-scoped value. Every test that shares the value runs it.
- A fixture that returns a function of its own (a factory).
- A collector fixture that asserts on its yielded value after the `yield`.
- A parameter rebound before the write, or one that `parametrize` supplies directly (but not with `indirect=`).

A restore by hand without `finally`, and two plain stores to the same target, are reported at the first change, as the
other leak rules report them.

**Gaps:**

- Aliases (`cfg = app.config; cfg["X"] = 1`) are not followed, nor methods or functions outside test documents the
  value is passed to.
- Mutating methods outside the list/dict/set set are not checked.
- Scopes given by a callable are skipped.

**Overlap:** no Ruff (PT family), pylint or flake8-pytest-style rule resolves fixtures across `conftest.py` files or
reads their scope.

**Sources:**

- pytest docs, [fixture scopes](https://docs.pytest.org/en/stable/how-to/fixtures.html#scope-sharing-fixtures-across-classes-modules-packages-or-session):
  every test in the scope shares the fixture instance.
- pytest docs, [Flaky tests](https://docs.pytest.org/en/stable/explanation/flaky.html): tests that don't clean up
  shared state make results depend on order.
- pytest docs, [monkeypatch](https://docs.pytest.org/en/stable/how-to/monkeypatch.html): changes are undone after
  the test.
- flakytest.dev, [Keep tests independent of each other](https://flakytest.dev/guides/test-isolation).

**Corpus:** re-measured after the rework on the 13 repositories under 1,000 `.py` files: `jupyterhub` 19, the other 12
none. 18 of the 19 are real: the review's 16 (`app.services.append(service)`, `app.load_groups = {...}`,
`app.user_redirect_hook = ...` on the module-scoped `app`), and two restores by hand that the rule now reports like
its siblings (`jupyterhub/tests/test_api.py:746`, `app.authenticator.username_pattern` set and put back after the
request, skipped when it fails; `jupyterhub/tests/test_auth.py:397`). The false positive is
`jupyterhub/tests/conftest.py:354`: `_mockservice` replaces `app.services` and its finalizer empties it with
`app.services[:] = []`, a slice store the library does not read as a restore of the attribute. Not re-run:
`apache/superset` (3,191 files), where the review read 72 matches, 47 real; its 25 false positives are the
`app.config[...] = v` ... `finally: app.config.pop(..., None)` shape, which the cases now hold as left alone, so the
expected reading there is 47/47 `[not re-measured]`. Re-measured precision 18/19 (95%); with the review's superset
reading minus the fixed shape, 65/66.

### `pytest-practice-environ-without-monkeypatch`

Rerail's own rule, not a port. The process environment is global: a variable a test sets or removes on `os.environ`
stays changed for every test that runs after it in the same process, and for the subprocesses those tests start, so
results come to depend on test order. pytest's `monkeypatch.setenv` / `monkeypatch.delenv` and unittest.mock's
`patch.dict(os.environ, ...)` put the environment back when the test (or fixture) ends.

```python
import os

@pytest.fixture
def insecure_transport():
    os.environ["AUTHLIB_INSECURE_TRANSPORT"] = "true"   # reported: every later test runs with it set

@pytest.fixture
def insecure_transport(monkeypatch):                    # the fix: undone after the test
    monkeypatch.setenv("AUTHLIB_INSECURE_TRANSPORT", "true")
```

It matches a change to `os.environ`, resolved through imports (`from os import environ`), made inside a test (a test,
fixture, set-up hook, or a helper of a test document that one of them calls): an assignment, augmented assignment or
`del` of `os.environ[k]`; a call of its `update`, `pop`, `setdefault`, `clear`, `popitem`, `__setitem__` or
`__delitem__`; `os.environ |= {...}`; and `os.putenv` / `os.unsetenv`. A def reports each variable once, at its first
change.

It leaves alone:
- `monkeypatch.setenv` / `delenv` / `setitem(os.environ, ...)`, and a direct write of a variable the same def also
  hands to monkeypatch (monkeypatch puts back its original value);
- code under `patch.dict(os.environ, ...)` (also `patch.dict("os.environ", ...)` and `mocker.patch.dict`): as a `with`
  around the write, a decorator of the def or its class, a call in the same def, a patcher started in the class's
  set-up hook, or a call in a function-scoped fixture the test requests;
- a change put back the way the three leak rules agree on (above): a base class's `tearDown` that pops the variable,
  a `@contextmanager` helper whose `finally` restores each variable in a loop or the whole environment
  (`os.environ.clear(); os.environ.update(saved)`), an autouse fixture that restores the environment after its yield,
  a bound-method cleanup; the undoing change itself;
- session-wide set-up that every test sees the same: pytest hooks, helpers only hooks call, autouse session fixtures
  and the session fixtures they request;
- module-level code, of `conftest.py` and of test modules alike: it runs once when pytest imports the module, before
  the first test, so every test of the run sees one value. This is the deliberate "set the environment before
  importing the app" idiom (`os.environ["POLAR_ENV"] = "testing"` at the top of a conftest.py), not a leak between
  tests;
- a def no test calls: a runner script's `main()` or a check script under `tests/` (saleor's
  `asgi/tests/checks/check_uvicorn_starts.py`);
- a WSGI `environ` dict, `request.environ`, `os.environ.copy()` and its writes, reads;
- rebinding `os.environ = {...}`: Ruff's B003 reports it;
- code outside test files.

**Gaps:** an alias (`env = os.environ; env["K"] = "v"`) is not followed; a helper outside test documents is not read;
a session-scoped autouse fixture in a sub-directory's `conftest.py` is left alone, though tests outside that directory
that run after it see its value; a context-manager helper that restores after its `yield` without a `finally` is taken
as a restore, though an exception in the `with` body skips it.

**Overlap:** Ruff's B003 reports only rebinding `os.environ = ...`, which this rule does not report; SIM112 is about
the variable name's case. No Ruff PT rule, pylint or flake8 plugin reports an environment write in a test.

**Sources:**
- pytest docs, [How to monkeypatch/mock modules and environments](https://docs.pytest.org/en/stable/how-to/monkeypatch.html):
  `monkeypatch.setenv` / `delenv`; "All modifications will be undone after the requesting test function or fixture
  has finished."
- Python docs, [unittest.mock `patch.dict`](https://docs.python.org/3/library/unittest.mock.html#patch-dict): "patch.dict
  can be used to add members to a dictionary, or simply let a test change a dictionary, and ensure the dictionary is
  restored when the test ends"; its own example patches `os.environ`.
- pytest docs, [Flaky tests](https://docs.pytest.org/en/stable/explanation/flaky.html): tests that depend on global
  state another test leaves behind fail depending on order.

**Corpus:** re-measured after the rework on the 13 repositories under 1,000 `.py` files: 9 matches, all real, the
same as before: `jupyterhub/tests/test_roles.py:1418`, `bookmarks/tests/test_oidc_support.py:42` and `:56`
(linkding: `LD_ENABLE_OIDC` set and deleted by hand at the end of the test), freqtrade
`tests/test_directory_operations.py:51`, `src/backend/InvenTree/InvenTree/test_tasks.py:671`, and
`label_studio/tests/conftest.py:91` to `:94` (moto's AWS credential variables set in a function fixture). Not re-run
(1,000 files or more; the review's reading): `polarsource/polar` 3, real
(`server/tests/observability/observability/test_http_middleware.py:21`, `:33`: `PROMETHEUS_MULTIPROC_DIR` left
pointing at a deleted temporary directory), `apache/superset` 1, real, and `saleor/saleor` 1, the review's one false
positive (`saleor/asgi/tests/checks/check_uvicorn_starts.py:31`, a check script no test calls), which the rework
leaves alone (its shape is a case) `[not re-measured]`; `langgenius/dify` was never measured. Read precision 13/14
before the rework, 13/13 expected after it.

### `pytest-practice-module-state-without-monkeypatch`

Rerail's own rule, not a port. A module is imported once per test session and a class is defined once, so a value a
test writes on a module global, on a class attribute or into a module-level dict or list stays there for every test
that runs after it: results then depend on test order. pytest's `monkeypatch.setattr`/`setitem`/`delattr` and
unittest.mock's `patch.object`/`patch.dict` undo their changes when the test ends.

```python
from shop import checkoutflow

def test_flow_order(event):
    checkoutflow.DEFAULT_FLOW = (ConfirmStep, PaymentStep)      # reported: every later test gets this flow

def test_flow_order(event, monkeypatch):                        # the fix: undone after the test
    monkeypatch.setattr(checkoutflow, "DEFAULT_FLOW", (ConfirmStep, PaymentStep))
```

It matches a change made inside a test (a test, fixture, set-up hook, or a helper of a test document that one of them
calls) to state of the run's own code that outlives the test, the written object resolved through imports:
- an attribute of a module of the run: `registry.TIMEOUT = 1`, `+=`, `setattr(registry, "TIMEOUT", 1)`;
- an attribute of a class of the run: `Locks.timeframe = "5m"`, `registry.Locks.acquire = fake`;
- an attribute of a module-level object of the run: `config.debug = True` (`config = Config()` in `shop/conf.py`), a
  pydantic settings instance (`settings.debug = True`);
- an item of a module-level object, or of a module's or class's attribute: `registry.HANDLERS["x"] = f`,
  `app.config["TESTING"] = True`, `app.dependency_overrides[get_db] = fake`;
- a list/dict/set mutator called on a module-level or class-level container (a literal, a comprehension, or
  `dict`/`list`/`set`/`defaultdict`/`OrderedDict`/`Counter`/`deque`): `PLUGINS.append(x)`, `Locks.seen.append(x)`,
  `HANDLERS.pop("x")`.

A def reports each target once, at its first change. The message names the changed object (the receiver, for a method
call).

It leaves alone:
- `monkeypatch.setattr`/`setitem`/`delattr`, `patch.object`, `patch.dict`, `patch("a.b.X")`, `patch.multiple`, and a
  write inside one of them;
- a change put back the way the three leak rules agree on (above). Any write of the target or of a container it runs
  through counts as the undo, a method call on any object included: a fixture that sets
  `app.dependency_overrides[get_db]` and calls `app.dependency_overrides.clear()` (or `pop(get_db, None)`) after its
  yield or in a `finally` is left alone, and so is a bound-method cleanup (`self.addCleanup(self._restore_resolver)`);
- a reset or cleanup: `clear()`, `discard()`, `pop(k, default)` or `del` (`_ENGINE_CACHE.clear()` before a cold-cache
  assertion, `TaskRegistry._tasks.pop(k, None)` at the end of a test, `del FxaView.keys`). Alone it empties a cache or
  removes what other code registered; after a write of the test's own it is a restore by hand, and the write is
  reported. `pop(k)` without a default removes an entry that was there and is reported;
- an annotation without a value (`app.config["X"]: Callable`): no write;
- objects of other libraries (`mail.outbox = []`, Flask's `g`); `os.environ` has a rule of its own, and
  `django.conf.settings` the django pack's `django-practice-settings-in-test`;
- instances, fixture values, `self.x = ...`, `cls.x = ...`; a method call on a class attribute that is not a
  container (`Order.objects.update(...)`);
- a class of the test module itself that no other module names, and a class defined inside a def;
- session-wide set-up and module-level code, as for the other leak rules; code outside test files.

**Gaps:** an alias (`h = registry.HANDLERS; h["x"] = f`) is not followed; a change deeper than one attribute and one
item below the shared object (`app.extensions["mail"].enabled = False`) is not read; a reset by assignment
(`registry.errors = {}`, `policy._cache = None`) is reported like any other write, though it is often deliberate
isolation; a helper outside test documents is not read.

**Overlap:** none in Ruff, pylint or flake8. pytest-randomly and detect-test-pollution find the same leaks at run time,
only when an order exposes them; the rule finds them before.

**Sources:**
- pytest docs, [Flaky tests](https://docs.pytest.org/en/stable/explanation/flaky.html): tests that depend on global
  state another test leaves behind fail depending on order.
- pytest docs, [How to monkeypatch/mock modules and environments](https://docs.pytest.org/en/stable/how-to/monkeypatch.html):
  `monkeypatch.setattr`/`setitem`/`delattr` are undone after the test.
- asottile, [detect-test-pollution](https://github.com/asottile/detect-test-pollution): a bisecting tool for exactly
  this kind of leak, which says how common it is.

**Corpus:** re-measured after the rework on the 13 repositories under 1,000 `.py` files: 43 matches, `freqtrade` 38,
`inventree/InvenTree` 4, `mealie` 1 (the review had 38, 4 and 1). In freqtrade one match left (`del Trade.session`
in `tests/plugins/test_pairlist.py:1235`, a reset) and one came (`ApiBG.jobs.update(...)` at
`tests/rpc/test_rpc_apiserver.py:3272`, a second key of the same registry in the test that writes `ApiBG.jobs[job_id]`).
All 38 freqtrade matches read: 36 real (`Exchange._ccxt_params = {...}` never restored, `PairLock.session =
MagicMock()`, `Trade.use_db` and `PairLocks.timeframe` restored by hand at the end, `LocalTrade.bt_trades_open =
trades`), 2 resets by assignment at the start of a test (`PairLocks.timeframe = ""` in
`tests/persistence/test_db_context.py:8`, `ApiBG.exchanges = {}` in `tests/rpc/test_rpc_apiserver.py:4209`). Not
re-run (1,000 files or more): superset 80, dify 28, polar 21, saleor 6, pretix 4, addons-server 2, DefectDojo 1 in
the review, where it read 69 of 110 sampled matches as actionable; the shapes the rework now leaves alone (FastAPI
`dependency_overrides` popped or cleared in undo code, polar 11 and `server/tests/oauth2/conftest.py:95`; caches and
registries cleared or popped; the bound-method cleanup in DefectDojo `unittests/test_locations_feature_flag.py:21`;
the bare annotation in superset `hive_tests.py:187`) were 26 of the 41 non-actionable ones, so the sample is expected
to read about 69/84 (82%) `[not re-measured]`. Below 90%: log, off. Resets by assignment (`policy._cache = None`,
`registry.errors = {}`) and test fakes are the remaining noise, which a static rule cannot tell from a leak.
