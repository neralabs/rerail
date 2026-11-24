# Rerail rule pack: pytest

Rerail's own pytest rules, from the pytest docs and maintainers' review advice, for isolated tests, sound fixtures and mocks that check what they claim.

This pack is private while its rules are written and reviewed: it is not published to npm, and is installed from
a checkout with `rerail packs add packs/pytest`.

## Rules

| Rule | Mode | Catches |
|---|---|---|
| `pytest-practice-django-settings-in-test` | Steer | pytest practice: test code changes django.conf.settings directly |
| `pytest-practice-environ-without-monkeypatch` | Steer | pytest practice: test code changes os.environ without undoing it |
| `pytest-practice-module-state-without-monkeypatch` | Log | pytest practice: test code changes a module global or class attribute without undoing it |
| `pytest-practice-patch-decorator-order` | Steer | pytest practice: stacked @patch mock parameters named in decorator order, so each holds the other's mock |
| `pytest-practice-patch-misses-from-import` | Steer | pytest practice: a mock patch on the defining module of a name its callers imported with from-import |
| `pytest-practice-scoped-fixture-mutated` | Steer | pytest practice: a test changes a class-, module- or session-scoped fixture's value |

## Rule details

### `pytest-practice-django-settings-in-test`

Rerail's own rule, not a port. A value a test writes on `django.conf.settings` stays there: Django does not put the
old value back after the test and sends no `setting_changed` signal for the write, so the value leaks into the tests
that run after it, and caches built from the setting (DRF's throttle rates, storage backends, template engines) go
stale. pytest-django's `settings` fixture, `override_settings`, `modify_settings` and a TestCase's `self.settings()`
revert their changes and send the signal.

```python
from django.conf import settings

def test_svg_upload_sanitize(client):
    settings.SVG_SECURITY_CLEANUP = True        # reported: every later test runs with it on
    ...

def test_svg_upload_sanitize(client, settings):  # the fix: pytest-django's fixture reverts it
    settings.SVG_SECURITY_CLEANUP = True
```

It matches, in test code (a `tests`/`test`/`testing`/`unittests` directory, `test_*.py`, `*_test.py`, `*_tests.py`,
`tests.py`, `test.py`, `conftest.py`: tests, fixtures, set-up hooks, helpers and module-level code), a write to an
upper-case attribute of `django.conf.settings`, resolved through imports (`from django.conf import settings as s`,
`django.conf.settings.X`): an assignment, augmented assignment or `del` of the setting, or of an item or attribute
inside it (`settings.CACHES["default"]["LOCATION"] = ...`), a list/dict/set mutator called on it
(`settings.MIDDLEWARE.append(...)`), and `setattr`/`delattr` on settings with a literal name. A function reports each
setting once, at its first write, so a restore by hand at the end of a test is not reported a second time (the first
write is: a failing assert skips that restore).

It leaves alone:
- pytest-django's `settings` fixture parameter (it resolves to no import, so it is not `django.conf.settings`);
- `override_settings`, `modify_settings` and `self.settings()`, and a plain write inside one of them (as a `with`, or
  a decorator of the test or its class): it lands on the override's copy, which is dropped with it. An item write
  inside an override still changes the shared value and is reported, unless the override replaces that setting;
- a write whose setting the same code puts back: in a `finally`, after a fixture's `yield`, in a `tearDown` /
  `tearDownClass` of the same class, in `teardown_module` / `teardown_function`, or in an `addCleanup` /
  `addfinalizer` callback (a lambda, a local def, or `self.addCleanup(setattr, settings, "X", old)`); the restoring
  write itself;
- session-wide set-up that every test sees the same: pytest hooks (`pytest_configure`, `pytest_sessionstart`, ...),
  helpers that only pytest hooks call (through the call graph), autouse session-scoped fixtures, pytest-django's
  database set-up fixtures (`django_db_setup`, `django_db_modify_db_settings*`), and module-level code of a
  `conftest.py`. A session-scoped fixture that is not autouse is reported: the tests that run before the first one
  requesting it see another value;
- lower-case attributes (`settings._wrapped`) and calls (`settings.configure(...)`), reads, `setattr` with a computed
  name;
- code outside test files: `django-practice-settings-runtime-assignment` reports it there;
- another library's `settings` (Dynaconf, a pydantic settings object).

**Gaps:** a restore in a base class's `tearDown` or in a helper the test calls is not seen (the write is reported); an
alias (`apps = settings.INSTALLED_APPS; apps.append(x)`) is not followed; `setattr(settings, name, v)` with a
computed name is not read. A restore by hand without `finally` is reported on purpose: it leaks whenever the test
fails before it.

**Overlap:** none in Ruff (its PT family, PGH005 and B003 do not look at Django settings), pylint or flake8.
`django-practice-settings-runtime-assignment` reports the same writes outside test code and skips test code on
purpose; the two rules read the same test-document set, so a write is reported by exactly one of them.

**Corpus:** 33 matches in 18 Django repositories, all 33 real (precision 100%), after two fixes the corpus showed
(a helper only `pytest_configure` calls, addons-server `amo/tests/__init__.py:1297`; saleor's session
`django_db_setup` fixture, `conftest.py:57`: both session-wide set-up, now left alone and in the cases). Every match
was read.

| Repository | Commit | .py files | Matches | Real |
|---|---|--:|--:|--:|
| sissbruecker/linkding | `27b7303baf41` | 229 | 0 | — |
| babybuddy/babybuddy | `1638cfb02e26` | 241 | 0 | — |
| UB-Mannheim/escriptorium | `7e06bd31732c` | 260 | 0 | — |
| lookit/lookit-api | `78562b5039ae` | 329 | 1 | 1 |
| ecds/readux | `fcf0d36d5f9c` | 374 | 3 | 3 |
| TandoorRecipes/recipes | `22b5841a7828` | 418 | 0 | — |
| paperless-ngx/paperless-ngx | `faa6f09f289b` | 424 | 0 | — |
| andreynovikov/django-shop | `2b8dc600166b` | 512 | 0 | — |
| healthchecks/healthchecks | `c1039ad23395` | 658 | 0 | — |
| inventree/InvenTree | `939df605f43d` | 731 | 2 | 2 |
| wger-project/wger | `f903c66d3c0c` | 766 | 2 | 2 |
| HumanSignal/label-studio | `2f42caecbf6c` | 775 | 5 | 5 |
| makeplane/plane | `723bb178f97b` | 784 | 0 | — |
| taigaio/taiga-back | `8ab6a4e3b5f6` | 933 | 3 | 3 |
| pretix/pretix | `40ea9097d0b2` | 1,228 | 2 | 2 |
| mozilla/addons-server | `7e3374fe5383` | 1,426 | 0 | — |
| DefectDojo/django-DefectDojo | `cd0a4483a224` | 2,601 | 15 | 15 |
| saleor/saleor | `e1ccf4d58002` | 4,385 | 0 | — |

Paperless-ngx, saleor and most of label-studio change settings only through pytest-django's `settings` fixture,
which the rule leaves alone; addons-server and InvenTree restore theirs in `finally` or `tearDown`; DefectDojo's
`@override_settings` test classes (`unittests/test_tag_inheritance.py:719`) write inside the override and are left
alone. Real matches:

- `apps/templates/tests/test_ga_id.py:12`, `:19`, `:23` (readux): one test sets `GOOGLE_ANALYTICS_ID`, another
  reads it and two `del` it: the tests pass only in file order, and the second `del` (or the read after it) raises
  AttributeError in any other.
- `label_studio/tests/test_upload_svg.py:13`, `:43`, `:67` (label-studio): the SVG upload tests switch
  `SVG_SECURITY_CLEANUP` on and off and leave it set for every later test, where the repository's other tests use
  the `settings` fixture; its conftest's `enable_sentry` fixture (`label_studio/tests/conftest.py:83`) does the same.
- `src/tests/base/test_mail.py:691` (pretix): `test_send_mail_private_ip` writes `EMAIL_CUSTOM_SMTP_BACKEND` and
  `MAIL_CUSTOM_SMTP_ALLOW_PRIVATE_NETWORKS` on django.conf.settings while the tests above it request the `settings`
  fixture: a forgotten fixture parameter.
- `tests/integration/test_users.py:1408` (taiga-back): mutates `REST_FRAMEWORK["DEFAULT_THROTTLE_RATES"]` in place
  and restores it by hand at the end, skipped when an assert fails; DRF reads the rates into its throttle classes.
- `unittests/test_deduplication_logic.py:1637` (DefectDojo): `DEDUPE_ALGO_ENDPOINT_FIELDS` changed and reset by
  hand at the end of the test, not in `finally`; `unittests/test_cleanup_alerts.py:24` sets `MAX_ALERTS_PER_USER`
  per test and never restores it.
- `src/backend/InvenTree/common/test_tasks.py:35` (InvenTree): `setUp` sets `USE_TZ = True` (the test settings
  have it off) and `tearDown` restores only `INVENTREE_NEWS_URL`.

Three DefectDojo matches (`test_generic_meta_import.py:23`, `test_tags.py:418`, `:435`) set
`SECURE_SSL_REDIRECT = False`, the value the settings already default to: a real leak with no effect today.

**Sources:**
- Django docs, [Testing tools: Overriding settings](https://docs.djangoproject.com/en/5.2/topics/testing/tools/#overriding-settings):
  "Don't manipulate django.conf.settings directly as Django won't restore the original values after such
  manipulations"; `setting_changed` is sent only by `override_settings` and its kin.
- pytest-django docs, [the `settings` fixture](https://pytest-django.readthedocs.io/en/latest/helpers.html#settings):
  it will "automatically revert any changes made to the settings (modifications, additions and deletions)".
- flakytest.dev, [Keep tests independent of each other](https://flakytest.dev/guides/test-isolation): order-dependent
  pytest tests come from shared state such as module globals, the environment and shared fixtures.

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

It matches, in the defs of test code (tests, fixtures, set-up hooks and helpers in a `tests`/`test`/`testing`/
`unittests` directory, `test_*.py`, `*_test.py`, `*_tests.py`, `tests.py`, `test.py` or `conftest.py`), a change to
`os.environ`, resolved through imports (`from os import environ`): an assignment, augmented assignment or `del` of
`os.environ[k]`; a call of its `update`, `pop`, `setdefault`, `clear`, `popitem`, `__setitem__` or `__delitem__`; and
`os.putenv` / `os.unsetenv`. A function reports each variable once, at its first change, so a restore by hand at the
end of a test is not reported a second time (the first change is: a failing assert skips that restore).

It leaves alone:
- `monkeypatch.setenv` / `delenv` / `setitem(os.environ, ...)`, and a direct write of a variable the same def also
  hands to monkeypatch (monkeypatch puts back its original value);
- code under `patch.dict(os.environ, ...)` (also `patch.dict("os.environ", ...)` and `mocker.patch.dict`): as a `with`
  around the write, a decorator of the def or its class, a call in the same def (a started patcher), or a call in the
  class's set-up hook;
- a change the same code undoes for the same variable: in a `finally`, after a fixture's `yield`, in a `tearDown` /
  `teardown_method` / `teardown_module` of the class or module, or in an `addCleanup` / `addfinalizer` callback (a
  lambda, a local def, or `self.addCleanup(os.environ.pop, "K")`); a restore of the whole environment
  (`os.environ.clear(); os.environ.update(saved)`) undoes every variable; the undoing change itself;
- session-wide set-up that every test sees the same: pytest hooks (`pytest_configure`, `pytest_sessionstart`, ...),
  helpers that only pytest hooks call (through the call graph), and `autouse=True, scope="session"` fixtures;
- module-level code, of `conftest.py` and of test modules alike: it runs once when pytest imports the module, before
  the first test, so every test of the run sees one value. This is the deliberate "set the environment before
  importing the app" idiom (`os.environ["POLAR_ENV"] = "testing"` at the top of a conftest.py), not a leak between
  tests;
- a WSGI `environ` dict, `request.environ`, `os.environ.copy()` and its writes, reads;
- rebinding `os.environ = {...}`: Ruff's B003 reports it;
- code outside test files.

**Gaps:** a restore in a base class's `tearDown`, in a fixture the test requests, or in a helper the undo code calls
is not seen (the change is reported); an alias (`env = os.environ; env["K"] = "v"`) is not followed; a session-scoped
autouse fixture in a sub-directory's `conftest.py` is left alone, though tests outside that directory that run after
it see its value; a restore by hand without `finally` is reported, though it leaks only when the test fails half-way;
a context-manager helper that restores after its `yield` without a `finally` is reported (an exception in the `with`
body skips that restore).

**Overlap:** Ruff's B003 reports only rebinding `os.environ = ...`, which this rule does not report; SIM112 is about
the variable name's case. No Ruff PT rule, pylint or flake8 plugin reports an environment write in a test.
`pytest-practice-django-settings-in-test` is the same check for `django.conf.settings`.

**Corpus:** CORPUS_TABLE

**Sources:**
- pytest docs, [How to monkeypatch/mock modules and environments](https://docs.pytest.org/en/stable/how-to/monkeypatch.html):
  `monkeypatch.setenv` / `delenv`; "All modifications will be undone after the requesting test function or fixture
  has finished."
- Python docs, [unittest.mock `patch.dict`](https://docs.python.org/3/library/unittest.mock.html#patch-dict): "patch.dict
  can be used to add members to a dictionary, or simply let a test change a dictionary, and ensure the dictionary is
  restored when the test ends"; its own example patches `os.environ`.
- pytest docs, [Flaky tests](https://docs.pytest.org/en/stable/explanation/flaky.html): tests that depend on global
  state another test leaves behind fail depending on order.

### `pytest-practice-module-state-without-monkeypatch`

Do not change a module global or class attribute of your code directly in a test; use monkeypatch.setattr/setitem or mock.patch.object, which put it back.

Rerail's own rule, not a port. A module is imported once per test session and a class is defined once, so a value a
test writes on a module global, on a class attribute or into a module-level dict or list stays there for every test
that runs after it: results then depend on test order (pytest docs, "Flaky tests"; asottile/detect-test-pollution
finds such pairs by bisection). pytest's `monkeypatch.setattr`/`setitem`/`delattr` and unittest.mock's
`patch.object`/`patch.dict` undo their changes when the test ends (pytest docs, "How to monkeypatch/mock modules
and environments").

It matches, in a test, fixture or set-up hook (pt_test_code; lambdas inside count, nested defs do not), a write to
state of the run's own code that outlives the test, the written object resolved through imports (python/names'
sym_of):
- an attribute of a module of the run: `registry.TIMEOUT = 1`, `+=`, `del`, `setattr(registry, "TIMEOUT", 1)`;
- an attribute of a class of the run: `Locks.timeframe = "5m"`, `registry.Locks.acquire = fake`;
- an attribute of a module-level object of the run: `config.debug = True` (`config = Config()` in shop/conf.py);
- an item of a module-level object, or of a module's or class's attribute: `registry.HANDLERS["x"] = f`,
`app.config["TESTING"] = True`;
- a list/dict/set mutator called on a module-level container or a class-level container (a literal, a
comprehension, or dict/list/set/defaultdict/OrderedDict/Counter/deque): `PLUGINS.append(x)`,
`registry.HANDLERS.clear()`, `Locks.seen.append(x)`.
One function reports each target once, at its first write: a restore by hand at the end of the test is not
reported again (the first write is: a failing assert skips that restore).
Left alone:
def test_x(monkeypatch): monkeypatch.setattr(registry, "TIMEOUT", 1)
the fix (also monkeypatch.setitem/delattr, patch.object,
patch.dict, patch("a.b.X"), patch.multiple)
with mock.patch.object(registry, "TIMEOUT", 1): registry.TIMEOUT = 3
a patch of the same target in the same def, as a decorator of
the def or its class, or started in its class's set-up hook,
puts the original back
old = registry.X; registry.X = 1; try: ... finally: registry.X = old
restored for the same target: in a finally, after a fixture's
yield, in a teardown hook of the class or module, or by an
addCleanup / addfinalizer callback (`addCleanup(setattr,
registry, "X", old)`, `addCleanup(registry.HANDLERS.clear)`)
mail.outbox = []; g.user = None                   objects of other libraries (Django resets mail.outbox per test;
flask's g is per request); os.environ and django.conf.settings
have rules of their own
order = Order(); order.status = "paid"            instances, fixture values, `self.x = ...`, `cls.x = ...`
Order.objects.update(status="x")                  a method call on a class attribute that is not a container
class Spy: calls = [] ... Spy.calls = []          a class of the test module itself that no other module names,
and a class defined inside a def
def pytest_configure(): registry.X = 1            session-wide set-up: pytest hooks, an autouse session-scoped
fixture, module-level code (it runs once, at import)
shop/registry.py: HANDLERS.clear()                not test code
Gaps: a write in a helper the test calls is not read; a restore in a base class's tearDown, in a fixture the test
requests, or in a helper is not seen; an alias (`h = registry.HANDLERS; h["x"] = f`) is not followed; a restore by
hand without finally is reported.

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
last dotted part of the patch string (`target=` included), or `patch.object`'s attribute, lower-cased with leading
underscores stripped. The first N positional parameters after `self`/`cls` (N mocks) are read: the rule reports when
every parameter's lower-cased name contains its top-down target (`mock_exists` ↔ `os.path.exists`, `mockExists`,
`exists_mock`) and one does not contain its bottom-up target. It reports at the first such parameter, saying which
target's mock it really receives and which parameter receives the one it is named for.

It leaves alone: bottom-up names; names that follow neither order (`m1, m2`); two targets with the same name
(`shop.mail.send` and `shop.sms.send`); a parameter whose name contains both of its candidate targets (`mock_user`,
`mock_user_email` over `user` and `user_email`); a single mock; fewer positional parameters than mocks (`*mocks`);
marks, `patch.dict`, `patch.multiple` and patches with `new` between the others (skipped: only the mock-passing
patches count, in their order); a target given as a variable; parameters after the mocks (fixtures); defs that are not
tests, and `test*` defs outside test code; `patch` names that do not resolve to `unittest.mock` or `mock`.

**Gaps:** patches on a test class (their mocks follow the method's own and are not read); `patch.multiple` with
`DEFAULT`; a swap hidden by a parameter that matches both targets; names matched by substring, so a short target
(`get`, `id`) can match a parameter by accident, which only reports when that accident also lines up with the
top-down order.

**Overlap:** none. Ruff's flake8-pytest-style (PT), PGH005 and B003, pylint and flake8 have no check on the order of
patch parameters.

**Corpus:** 4 matches in 8 repositories, all 4 real (precision 100%):

| Repository | Commit | .py files | Matches | Real |
|---|---|--:|--:|--:|
| jupyterhub/jupyterhub | `3ca37442a18e` | 158 | 0 | — |
| paperless-ngx/paperless-ngx | `faa6f09f289b` | 424 | 0 | — |
| freqtrade/freqtrade | `7fcb509a434c` | 487 | 0 | — |
| healthchecks/healthchecks | `c1039ad23395` | 658 | 0 | — |
| HumanSignal/label-studio | `2f42caecbf6c` | 775 | 0 | — |
| mozilla/addons-server | `7e3374fe5383` | 1,426 | 2 | 2 |
| apache/superset | `fdf61721d056` | 3,191 | 0 | — |
| saleor/saleor | `e1ccf4d58002` | 4,385 | 2 | 2 |

- `src/olympia/amo/tests/test_celery.py:133` (addons-server): `@mock.patch('olympia.amo.celery.cache')
  @mock.patch('olympia.amo.celery.statsd') def test_handle_cache_miss_for_stats(self, celery_cache, celery_statsd)`:
  the cache-miss set-up `celery_cache.get.return_value = None` goes to the statsd mock, and
  `assert not celery_statsd.timing.called` checks the cache mock, so the assert can never fail.
- `src/olympia/amo/tests/test_monitor.py:70` (addons-server): `@patch('os.path.exists') @patch('os.access') def
  test_path(self, mock_exists, mock_access)`: swapped names; the body does not read them yet.
- `saleor/payment/gateways/stripe/tests/test_webhooks.py:1075` and `:1315` (saleor): `wrapped_update_payment_with_new_transaction`
  holds the `order_voided` / `order_refunded` spy and the other way round; both asserts compare `.called` with the
  same expectation, so they pass only because the two functions are called together.

An independent Python `ast` scan of every test path in the bench corpus, its 20 original repositories and the ones
added later (2,835 tests with two or more mock-passing patches), found the same 4 and no test the ambiguity rules
left out.

**Sources:**
- Python docs, [unittest.mock: Nesting patch decorators](https://docs.python.org/3/library/unittest.mock.html#nesting-patch-decorators):
  "When you nest patch decorators the mocks are passed in to the decorated function in the same order they applied
  (the normal Python order that decorators are applied). This means from the bottom up".
- Ned Batchelder, [Why your mock still doesn't work](https://nedbatchelder.com/blog/202202/why_your_mock_still_doesnt_work.html):
  the stacked-decorator mix-up, whose asserts on the wrong mock still pass.
- Stack Overflow, [Mock patches appearing in the wrong order?](https://stackoverflow.com/questions/47042196/mock-patches-appearing-in-the-wrong-order).

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
those modules, which the message names with the patch that works (`patch("<that module>.name")`).

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
- `create=True`, `raising=False`, `patch.dict`, `monkeypatch.delattr`;
- names a Django management command imports: `call_command` imports the command's module when it runs, inside the
  patch, so its copy is the mock;
- a test that imports an importer in its own body while the patch is active, or a test file that reloads modules
  with `importlib` (both load the copy under the patch);
- copies read only in type annotations;
- a test file that imports no module holding a copy: on the corpus these were dead patches (the test never reaches an
  importer) or patches guarding the defining module's own code (`mod.run()` then `helper.assert_not_called()`).

**Gaps:** tests that reach the importer only through a test client or a fixture, with no import of it in the test
file, are not reported (`fastapi__full-stack-fastapi-template` `backend/tests/api/routes/test_users.py:41` patches
`app.utils.send_email` while the route imports it by name: real, missed); when any module reads `mod.name`, the rule
is silent even if the test runs another module's copy (`sissbruecker__linkding`
`bookmarks/tests/test_bookmarks_tasks.py:819`); a test module that imports the name itself and calls it misses the
patch the same way, but test modules are not read as importers; `from m import *` copies and names read through
`sys.modules` or a dotted string are not seen.

**Overlap:** none. No linter resolves patch targets; Ruff's PT family (ported in the `python` pack) has no such
check, and PT008 is about `lambda` as the patch's `new`.

**Sources:** unittest.mock, ["Where to patch"](https://docs.python.org/3/library/unittest.mock.html#where-to-patch)
("patch the name used by the system under test"); Ned Batchelder,
["Why your mock doesn't work"](https://nedbatchelder.com/blog/201908/why_your_mock_doesnt_work.html).

**Corpus** (BRIEF corpus, pinned commits from `MANIFEST.tsv`):

| Repository (commit) | .py files | Matches | Real | Notes |
|---|--:|--:|--:|---|
| `fastapi/full-stack-fastapi-template` (`77e550a`) | 43 | 0 | – | one real miss not reported (test goes through the HTTP client; see Gaps) |
| `fastapi-users/fastapi-users` (`e9c57ff`) | 88 | 0 | – | |
| `sissbruecker/linkding` (`27b7303`) | 229 | 0 | – | one real miss not reported (other modules read `website_loader.load_website_metadata`) |
| `Netflix/lemur` (`3a3c2c2`) | 337 | 4 | 4 | `lemur/plugins/lemur_acme/tests/test_powerdns.py:136` patches `lemur.extensions.metrics`; `powerdns.py` does `from lemur.extensions import metrics` (also `test_nsone.py:191`, `test_ultradns.py:65`, `:88`) |
| `paperless-ngx/paperless-ngx` (`faa6f09`) | 424 | 0 | – | |
| `freqtrade/freqtrade` (`7fcb509`) | 484 | 1 | 1 | `tests/rpc/test_rpc_emc.py:298` patches `freqtrade.rpc.api_server.ws.channel.create_channel`; `external_message_consumer.py` imports it by name |
| `wger-project/wger` (`f903c66`) | 766 | 0 | – | management-command importers (loaded by `call_command` inside the patch) correctly silent |
| `HumanSignal/label-studio` (`2f42cae`) | 775 | 0 | – | |
| `makeplane/plane` (`723bb17`) | 784 | 0 | – | `plane.settings.redis.redis_instance` is also read as `redis_settings.redis_instance()` by a test: silent by design |
| `mozilla/addons-server` (`7e3374f`) | 1,426 | 0 | – | |
| `DefectDojo/django-DefectDojo` (`cd0a448`) | 2,601 | 0 | – | |
| `apache/superset` (`fdf6172`) | 3,190 | 8 | 8 | `tests/unit_tests/commands/databases/oauth2_test.py:78` patches `superset.utils.oauth2.decode_oauth2_state` beside a correct `superset.commands.database.oauth2.get_user_id` patch (4 matches); `tests/integration_tests/reports/commands_tests.py:1292` (`superset.utils.csv.get_chart_dataframe`, mock never used; 2); `tests/unit_tests/common/test_query_context_processor.py:768` (`time_range_utils.get_since_until_from_query_object`, read by `models/helpers.py` by name; 2) |
| `langgenius/dify` (`724ac15`) | 5,191 | 0 | – | a fixture that reloads the importer with `importlib` correctly silent |
| `netbox-community/netbox` (`6997809`) | 1,299 | 0 | – | |
| `indico/indico` (`8e64139`) | 1,290 | 0 | – | |

Total: 13 matches in 15 repositories, all 13 real (100%). Before the rule required the test file to import a module
holding a copy, it also reported 5 patches whose test never reaches an importer (`superset`
`tests/integration_tests/tasks/commands/test_prune.py`, 4; `freqtrade` `tests/data/test_dataprovider.py:283`): dead
patches, for which the message's advice was off target, plus the real `full-stack-fastapi-template` miss. Shapes
cut on the way, each now a case: Django management commands (`wger`, 15 matches), a patch the same fixture also makes on
the importer's copy (`paperless-ngx`), a test guarding the defining module's own code (`paperless-ngx`), an importer
imported inside the patching test (`superset` `celery_tests.py:527`), a fixture reloading modules with `importlib`
(`dify`), and copies used only in type annotations.

### `pytest-practice-scoped-fixture-mutated`

**Catches** a test, or a function-scoped fixture, that changes the value of a fixture with a broader scope
(`scope="class"`, `"module"`, `"package"`, `"session"`) it requests. The rule looks for an attribute or item store,
a `del`, or an augmented assignment through the parameter (`app.config["X"] = 1`, `app.appbuilder = MagicMock()`,
`del data["k"]`). It also counts a list, dict or set mutator called on the value or on a chain over it
(`items.append(x)`, `app.config.update(...)`). The fixture is resolved by name the way pytest resolves it: the test's
class first, then its module, then the `conftest.py` files from the test's directory up to the root. A nearer
function-scoped override therefore hides a broad fixture. pytest creates such a fixture once and gives every test of
its scope the same object, so the change leaks into the tests that run after it and results depend on test order.
The message names the fixture and its scope. It suggests `monkeypatch.setitem`/`monkeypatch.setattr`, which are
undone after the test, or a function-scoped fixture that copies the value.

**Leaves alone:**

- Function-scoped fixtures, and fixtures that no `conftest.py` or test module of the run defines (plugins).
- Changes the same test undoes:
  - in a `finally`, after a fixture's `yield`, or in an `addCleanup`/`addfinalizer` callback (a `setattr`/`delattr`
    there counts), when the undo writes the same target or a chain the target runs through;
  - a restore by hand (`old = app.x; app.x = 1; …; app.x = old`);
  - a target the test also patches with `monkeypatch.setitem/delitem/setattr`, `patch.object` or `patch.dict` over
    its container.
- `monkeypatch.setitem(app.config, …)` itself, and writes to a local copy (`cfg = dict(app.config)`).
- Method calls that are not collection mutations:
  - HTTP-client calls (`api_client.delete(...)`; `delete`, `save` and `post` are not mutators);
  - database-session calls (`app.db.add(row)`, `db.session.remove()`);
  - `update(key, obj)` with two positional arguments, and mutator calls with keyword arguments;
  - calls whose result is used (`agent = client.agents.update(...)`);
  - any method call through a fixture named `client`, `*client`, `api`, `*_api` or `sdk`.
- `pop(key, default)`, used to remove a key defensively.
- A broader-scoped fixture changing its own broader-scoped dependency. This is set-up that runs once for its scope.
- An autouse fixture changing a class- or module-scoped value. Every test that shares the value runs it.
- A fixture that returns a function of its own (a factory).
- A collector fixture that asserts on its yielded value after the `yield`.
- A parameter rebound before the write, or one that `parametrize` supplies directly (but not with `indirect=`).

**Gaps:**

- Aliases (`cfg = app.config; cfg["X"] = 1`) are not followed, and neither are helpers the test passes the fixture to.
- Mutating methods outside the list/dict/set set are not checked.
- Scopes given by a callable are skipped.
- A restore by hand without `finally` counts as a restore, even though the test still leaks if it fails half-way.
  Two plain stores to the same target in one test are taken as a restore too.
- `pop(key, None)` on a key that the base value does have is missed.

**Sources:**

- pytest docs, [fixture scopes](https://docs.pytest.org/en/stable/how-to/fixtures.html#scope-sharing-fixtures-across-classes-modules-packages-or-session):
  every test in the scope shares the fixture instance.
- pytest docs, [Flaky tests](https://docs.pytest.org/en/stable/explanation/flaky.html): tests that don't clean up
  shared state make results depend on order.
- pytest docs, [monkeypatch](https://docs.pytest.org/en/stable/how-to/monkeypatch.html): changes are undone after
  the test.
- flakytest.dev, [Keep tests independent of each other](https://flakytest.dev/guides/test-isolation).

**Overlap:** no Ruff (PT family), pylint or flake8-pytest-style rule resolves fixtures across `conftest.py` files or
reads their scope.

**Corpus** (pinned commits from `MANIFEST.tsv`; every match was read):

| Repository | Commit | Matches | Real |
|---|---|--:|--:|
| `jupyterhub__jupyterhub` | `3ca37442a18e` | 16 | 16 |
| `apache__superset` | `fdf61721d056` | SUPERSET_N | SUPERSET_REAL |
| `letta-ai__letta` | `1131535716e8` | 0 | – |
| `mealie-recipes__mealie` | `8dbb92808d5a` | 0 | – |
| `freqtrade__freqtrade` | `7fcb509a434c` | 0 | – |
| `paperless-ngx__paperless-ngx` | `faa6f09f289b` | 0 | – |
| `HumanSignal__label-studio` | `2f42caecbf6c` | 0 | – |
| `saleor__saleor` | `e1ccf4d58002` | 0 | – |
| `fastapi__full-stack-fastapi-template` | `77e550a73bb4` | 0 | – |
| `fastapi-users__fastapi-users` | `e9c57ff7ff2f` | 0 | – |

Examples of real matches:

- superset `tests/unit_tests/tasks/test_slack.py:30`: `app.config["SLACK_CACHE_TIMEOUT"] = 123` on the
  module-scoped `app`.
- superset `tests/unit_tests/commands/report/execute_test.py:2631`: `app.config.update({...})`, with no restore.
- superset `tests/unit_tests/mcp_service/test_multi_auth_client_id.py:101`: the function-scoped `guest_app`
  fixture updates the shared `app.config`.
- jupyterhub `jupyterhub/tests/test_named_servers.py:603`: `app.user_redirect_hook = dummy_redirect`.
- jupyterhub `jupyterhub/tests/test_spawner.py:686`: `app.load_groups = {...}`.
- jupyterhub `jupyterhub/tests/test_roles.py:917`: `app.services.append(service)`.

Shapes the rule now leaves alone, found while tuning on the corpus:

- jupyterhub's `app.db.add(row)` (21).
- letta's SDK calls `client.agents.update(agent_id=…)` (49).
- jupyterhub's finalizer `request.addfinalizer(lambda: setattr(app, …))` and restores by hand.
- superset's collector fixture (`dao/session_cleanup_test.py`).
- superset's defensive `app.config.pop(k, None)` (4).
- superset's `del app.extensions[k]` after `monkeypatch.setitem` (2).
- mealie's `.delete(...)` HTTP calls, which the mining counted as 156 raw hits and the rule never matched.
