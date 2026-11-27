# pytest pack: independent review

This review audits the six `pytest-practice-*` rules, the library `rules/lib/pytest.rerail`, the cases, the README's
"Rule details" and `NOTES-pytest.md`. Nothing in the rules, cases or `pack.json` was changed: the verdicts are the
output. The counterexamples are in `/tmp/review-pytest/` (not committed). Commands are listed with each result and in
section 4.

Two precision figures per rule:

- **Code precision**: the matched code does what the message says (the value outlives the test; the patch misses the
  copy; the mock goes to the other parameter).
- **Actionable precision**: a reviewer of that repository would ask for the change. A code-real match is not
  actionable when it resets a cache, singleton or registry at the start of a test for isolation, or when it is the
  cleanup itself.

The BRIEF's `"on": true` bar (90% of read matches real) is applied to the actionable figure.

## 1. Summary

Verdicts: **3 keep, 3 rework, 0 cut.**

| Rule | Verdict | Proposed mode, on | Corpus matches (repos) | Read | Code precision | Actionable | Reason |
|---|---|---|--:|--:|--:|--:|---|
| `patch-misses-from-import` | keep | steer, on | 14 (3) | 14 | 14/14 | 14/14 | Cross-file, precise; finds vacuous `assert_not_called()` (saleor ×3); misses patches in conftest fixtures |
| `patch-decorator-order` | keep | steer, on | 4 (2) | 4 | 4/4 | 4/4 | Silent wrong result, zero noise; an independent scan of 2,558 stacked-patch tests found the same 4; camelCase targets never match |
| `scoped-fixture-mutated` | rework | log until fixed, then steer, on | 88 (2) | 88 | 63/88 (72%) | 63/88 (72%) | `pop(k, None)` in a `finally`/after `yield` is not seen as the undo: 25 superset false positives; accepts a restore by hand its siblings report |
| `django-settings-in-test` | keep (move to the `django` pack) | steer, on | 24 (4) | 24 | 24/24 | 24/24 | Precise; 15 of its 24 matches are in DefectDojo, which the pytest pack's `match` never installs in |
| `environ-without-monkeypatch` | rework | steer, on | 14 (8 of 19; dify not run) | 14 | 13/14 | 13/14 (93%) | Reports any def in a test document: runner scripts (saleor), a base `tearDown` that is the undo, context-manager helpers that restore by key |
| `module-state-without-monkeypatch` | rework | log, off | 185 (10) | 110 | 91/110 | 69/110 (63%) | Real leaks (superset `security_manager.on_* = Mock()`), but cache resets, FastAPI `dependency_overrides` popped in `finally`, and cleanups are reported; README section unfinished |

All six rules' own `.test` blocks pass (`nice -n 19 node dist/cli/cli.js rules test packs/pytest/rules/<rule>.rerail`:
1 passed, 0 failed each), and the pack's validation contract passes (`nice -n 19 npx vitest run
test/engine/catalog/packs.test.ts -t "pack pytest"`: 15 passed, 1418 skipped). Every match count was re-measured on the
BRIEF's 20 repositories (section 4), except `environ-without-monkeypatch` on dify, whose run never got a slot; the
README's numbers were not copied.

**Reading order.** `pack.json` lists the rules alphabetically. Read them by what they prevent and how sure a match is:

1. `patch-misses-from-import`, then `patch-decorator-order`: tests that pass without testing anything (a silent wrong
   result), precise; the first is cross-file.
2. `scoped-fixture-mutated`: shared fixture state, cross-file through the conftest chain.
3. `django-settings-in-test`, then `environ-without-monkeypatch`: global state with a one-line fix.
4. `module-state-without-monkeypatch`: the widest and noisiest of the leak rules (log).

## 2. Rules

### 2.1 `pytest-practice-patch-misses-from-import`

**Value.** A `from pkg.mod import f` copy is not replaced by `patch("pkg.mod.f")`; the code under test keeps the real
object and `assert_not_called()` passes vacuously. Sources checked: the unittest.mock docs' "Where to patch" ("patch
where an object is looked up"; the README's quoted wording "patch the name used by the system under test" is not on the
page, the meaning is), Ned Batchelder's "Why your mock doesn't work" (`from mod import val` is an assignment). Both hold
up. On the corpus the bug is live: saleor's `test_increase_stock_with_back_in_stock_webhook_triggered_without_allocation`
(`saleor/warehouse/tests/test_stock_management.py:837`) patches
`saleor.warehouse.webhooks.stock_events.trigger_product_variant_back_in_stock`, `saleor/warehouse/management.py:42`
imports it by name, and the test's `assert_not_called()` can never fail.

**Correctness.** The logic is careful: workspace-only targets, re-export chains, function-scope imports, any
`mod.name` read, Django management commands, `importlib` reloads and imports inside the patching test all silence it.

| # | Counterexample (`/tmp/review-pytest/pmi`) | Expected | Result |
|---|---|---|---|
| 1 | autouse conftest fixture `with mock.patch("shop.mail.send_email")`, test files import `shop.orders` | report | silent: the conftest imports no importer (miss) |
| 2 | `mock.patch.object(mail, "send_email")` after `from shop import mail` in the test body | report | reported `tests/test_cx.py:11` |
| 3 | `patch("shop.clock.now")`, `shop.billing` copies it, `shop.sched` reads `clock.now` | report | silent (documented gap: any `mod.name` read silences every importer) |
| 4 | `mocker.patch("shop.mail.send_email")` | report | reported `tests/test_cx.py:25` |

Further gaps read in the logic: `create=True` / `raising=False` silence the rule even when the attribute exists (the
patch still misses the copy); the fixture-in-conftest shape (row 1) is the common way teams install a
"no real e-mail" guard, and it is missed.

**Corpus.** 14 matches in 3 of the 20 repositories (freqtrade 1, superset 8, saleor 5); every match read,
14/14 real, 14/14 actionable. Saleor's 5 are new against the README, which did not run saleor:
`saleor/warehouse/tests/test_stock_management.py:837`, `:1249`, `:1270` (vacuous `assert_not_called()` on the
back-in-stock webhook), `saleor/plugins/webhook/tests/test_webhook.py:1962` (`assert not mocked_clear_delivery.called`,
vacuous: `asynchronous/transport.py:50` copies `clear_successful_delivery`),
`saleor/graphql/webhook/tests/mutations/test_webhook_trigger.py:130`. Superset's 8 and freqtrade's
`tests/rpc/test_rpc_emc.py:298` are the README's. No false positive.

**Overlap.** None: no linter resolves patch target strings against the importers' bindings (Ruff PT008 is about
`lambda` as `new`; PGH005 about mock assertions).

**Text.** Description: one imperative sentence, 143 characters; it doesn't name the patch API it is about ("Patch a
function, class or value where..."): fine. Message (185 characters) names the importer and the fix. It does not say when
to leave it (criterion 2): a patch whose test never reaches the importer is dead rather than misdirected. Add "or remove
the patch if this test never runs that module."

**Verdict: keep**, steer, on. Optional follow-up: read patches in conftest fixtures against the test files under that
conftest (row 1), and drop the `create=True`/`raising=False` exemption when the attribute exists.

### 2.2 `pytest-practice-patch-decorator-order`

**Value.** Stacked patch decorators pass mocks bottom-up; parameters named top-down hold each other's mocks, so
set-ups and asserts hit the wrong mock. Sources checked: unittest.mock "Nesting patch decorators" ("This means from the
bottom up"), Ned Batchelder's "Why your mock still doesn't work" ("The first decorator corresponds to the last
argument"), Stack Overflow 47042196 (score 55, accepted answer quotes the docs). All hold up.

**Correctness.**

| # | Counterexample (`/tmp/review-pytest/pdo`) | Expected | Result |
|---|---|---|---|
| 1 | `requests.get` / `requests.post`, parameters `mock_requests_get, mock_requests_post` | report | reported `shop/tests/test_cx.py:8` |
| 2 | `send` / `send_email`, parameters `mock_send, mock_send_email` (swapped) | report | silent (documented: a name holding both targets) |
| 3 | short targets `uuid4` / `get_id` with unrelated abbreviations | silent | silent |
| 4 | class-level `@patch` plus two method patches named top-down | report | reported `:31` (the method's own mocks are read; correct) |
| 5 | `patch.object(TestCase, "assertTrue")` + `patch("...redis")`, `mock_assert_true, mock_redis` (swapped) | report | silent: `asserttrue` is not a substring of `mock_assert_true` |
| 6 | parametrize mark between patches, bottom-up names | silent | silent |
| 7 | three patches, parameters in a rotation | — | silent (neither order) |

Row 5 is a real gap: camelCase attribute and class targets (`EmailClient` vs `mock_email_client`) never match. An
independent `ast` scan of the corpus (`/tmp/review-pytest/probe.py`, 2,558 tests with two or more mock-passing patches,
827 of them with names that follow one of the two orders) found the same 4 swaps, and none more when underscores are
dropped from both sides, so the gap costs nothing on this corpus.

**Corpus.** 4 matches in 2 repositories (addons-server 2, saleor 2), the README's 4; 4/4 real. About 0.5% of the stacks
whose names follow either order are swapped.

**Overlap.** None (flake8-pytest-style, Ruff, pylint have no parameter-order check).

**Text.** Description 152 characters, imperative, says why. Message 352 characters, over criterion 6's 200; it names
the change and tells the agent to rerun the test because its asserts were wrong: the right steer (renaming can turn a
passing test red, which is the point). Trim to: "`{x}` receives the mock of `{real}`, not `{claimed}`: stacked
patches pass mocks bottom-up. Reorder the mock parameters bottom-up and rerun the test."

**Verdict: keep**, steer, on. Optional: compare names with underscores removed on both sides (row 5).

### 2.3 `pytest-practice-scoped-fixture-mutated`

**Value.** A change to a class/module/package/session-scoped fixture's object reaches every later test of the scope.
Sources: pytest fixtures docs (scope: "the fixture is destroyed during teardown of the last test in the module"),
"Flaky tests", monkeypatch docs. They hold up; the lookup through conftest files is cross-file value no linter has.

**Correctness.**

| # | Counterexample (`/tmp/review-pytest/sfm`) | Expected | Result |
|---|---|---|---|
| 1 | two plain stores to the same key (`X = 1` then `X = 2`) | report | silent (documented: taken as a restore) |
| 2 | restore by hand after a failing assert, no `finally` | report (the sibling rules report it) | silent |
| 3 | alias `cfg = app["config"]; cfg["Y"] = 1` | report | silent (documented) |
| 4 | item store through a session fixture named `client` | report | reported `tests/test_cx.py:26` |
| 5 | helper `add(seen)` mutates a session list | report | silent (documented) |
| 6 | session list a test appends to on purpose | message lets it be | reported `:40` |
| 7 | `request.getfixturevalue("app")` then a write | report | silent |
| 8 | class-scoped fixture in a test class | report | reported `:56` |
| 9 | `app["config"]["RESOLVER"] = x; try: ... finally: app["config"].pop("RESOLVER", None)` | silent | **reported** `tests/test_cx2.py:3` |
| 10 | the same with `del app["config"]["RESOLVER2"]` in the `finally` | silent | silent |

Row 9 is the corpus's main false positive. `sfm_not_collection` drops `pop(key, default)` as a "defensive removal",
so the same call in a `finally` or after a `yield` never counts as the undo either.

**Corpus.** 88 matches in 2 repositories (jupyterhub 16, superset 72); the README's superset count is a placeholder
(`SUPERSET_N`). All 16 jupyterhub matches read: real (`app.services.append`, `app.load_groups = ...`,
`app.authenticator.auth_state = ...` on the module-scoped `app`). All 72 superset matches triaged with a script that
prints each match's line and its next 40 lines (`/tmp/review-pytest/sfm-sup-list.txt`), 15 read in full: 25 are row 9
(`app.config["MCP_..."] = v` with `app.config.pop("MCP_...", None)` in a `finally` or after a `yield`, e.g.
`tests/unit_tests/mcp_service/test_auth_user_resolution.py:124`, `test_mcp_multi_issuer_identity_binding.py:75`,
`test_native_tool_surface.py:492`; the keys are absent from the app's config, so the pop restores it). The other 47
are real: `commands/report/execute_test.py` (20 `app.config` writes with no restore),
`datasets/partition_mapping_preview_test.py` (7), `tasks/test_slack.py:30`, and
`mcp_service/test_auth_api_key.py:71`/`:81`, whose teardown pops `FAB_API_KEY_ENABLED`, a key `superset/config.py`
defines: it deletes the default. Precision 63/88 (72%) code and actionable.

**Overlap.** None (no Ruff, pylint or flake8-pytest-style rule reads fixture scope through conftest files).

**Text.** Description 129 characters, fine. Message 345 characters (over 200); it says "This test changes" for a
function-scoped fixture too (`conftest.py:47` in the cases). The escape clause ("leave it if the fixture is meant to
accumulate state across tests") is good.

**Verdict: rework**, then steer, on. Count `pop(k, ...)`, `del` and `popitem` on the same target in undo code (finally,
after yield, cleanup callback) as the undo (row 9: 25 of 25 false positives); report a restore by hand without `finally`
at the first change, as the three sibling rules do (row 2), and stop taking two plain stores for a restore (row 1).
With row 9 fixed the corpus reads 63/63. Until then: log.

### 2.4 `django-settings-in-test` (moved to the django pack as `django-practice-settings-in-test`)

**Value.** Django's docs say it outright: "Don't manipulate django.conf.settings directly as Django won't restore the
original values" (checked on the 5.2 page), and pytest-django's `settings` fixture docs say it "will automatically revert
any changes made to the settings (modifications, additions and deletions)". Both hold up. Teams hit it: 24 matches in
4 of 10 Django repositories.

**Correctness.**

| # | Counterexample (`/tmp/review-pytest/dst`) | Expected | Result |
|---|---|---|---|
| 1 | `o = override_settings(SITE_ID=1); o.enable(); settings.SITE_ID = 3; ... finally: o.disable()` | silent | reported `tests/test_cx.py:12` (rare shape) |
| 2 | restore in a base class's `tearDown` (`shop/testutils/base.py`) | silent | reported `:20` (documented gap) |
| 3 | `@override_settings` on the class, write in `setUp` | silent | silent |
| 4 | `setUpClass` write, no `tearDownClass` | report | reported `:38` |
| 5 | alias `apps = settings.INSTALLED_APPS; apps.append(x)` | report | silent (documented) |
| 6 | `def test_x(settings)` fixture | silent | silent |
| 7 | helper in a test module that a test calls | report | reported `:54` |
| 8 | module-level write in a test module | (the environ rule leaves this shape alone) | reported `tests/test_module_level.py:4` |
| 9 | helper in `shop/testing.py` (a file, not a `testing/` directory) | report | silent: not a test document |
| 10 | `tests/runtests.py`'s `main()` | silent | reported `tests/runtests.py:8` |

Row 10 is the same false positive the environ rule has on saleor (2.5). Row 8 contradicts
`environ-without-monkeypatch`, which leaves module-level writes alone with the argument that they run once at import
before any test; the same argument holds here. Pick one rule for both.

**Corpus.** Run on the 10 Django repositories: 24 matches in 4 (label-studio 5, InvenTree 2, pretix 2, DefectDojo 15),
equal to the writer's numbers for these repositories. All 24 read: 24/24 real and actionable. Three DefectDojo writes
(`unittests/test_generic_meta_import.py:23`, `unittests/test_tags.py:418`, `:435`) set `SECURE_SSL_REDIRECT = False`,
the value the test settings already have: a leak with no effect today, still worth `override_settings`. Seven DefectDojo
`DEDUPE_ALGO_ENDPOINT_FIELDS` writes are restored by hand at the end of the test, which leaks only when the test fails
(the rule reports these on purpose). Real examples:
`label_studio/tests/test_upload_svg.py:13` (`SVG_SECURITY_CLEANUP` left on for later tests),
`src/backend/InvenTree/common/test_tasks.py:35` (`USE_TZ = True` in `setUp`, `tearDown` restores another setting),
`src/tests/base/test_mail.py:691` (pretix, a forgotten `settings` fixture parameter),
`unittests/test_cleanup_alerts.py:24` (DefectDojo, never restored).

**Overlap.** `django-practice-settings-runtime-assignment` covers the same write outside tests; both read the same
test-document definition (`pt_test_doc` and `dj_test_doc` are textually identical), so no write is reported twice. No
Ruff, pylint, flake8-django or pylint-django check.

**Text.** Description 116 characters, fine. Message 405 characters (over 200), and its escape clause is useful. It
names pytest-django's fixture first; in a `unittest`/Django `TestCase` (most of DefectDojo's 15) the fix is
`override_settings`/`self.settings()`, which the message also gives.

**Verdict: keep**, steer, on, with two small changes: leave `main()`/`if __name__ == "__main__"` code of runner scripts
alone (row 10), and agree with the environ rule on module-level writes (row 8). Pack placement: see section 3.

### 2.5 `pytest-practice-environ-without-monkeypatch`

**Value.** The environment is process-global; `monkeypatch.setenv`/`delenv` and `patch.dict(os.environ)` undo their
changes. Sources: pytest monkeypatch docs ("All modifications will be undone after the requesting test function or
fixture has finished"), unittest.mock `patch.dict` ("ensure the dictionary is restored when the test ends"), pytest
"Flaky tests", and flakytest.dev ("os.environ["KEY"] = ... stays for the rest of the run"). All hold up.

**Correctness.**

| # | Counterexample (`/tmp/review-pytest/env`) | Expected | Result |
|---|---|---|---|
| 1 | `@contextmanager def env(**v)`: `os.environ.update(v)`, `finally` restores each key in a loop | silent | **reported** `tests/helpers.py:9` |
| 2 | `tests/runtests.py` `main()` with `os.environ.setdefault("DJANGO_SETTINGS_MODULE", ...)` | silent | **reported** `tests/runtests.py:7` |
| 3 | `setUpClass` write, no restore | report | reported `tests/test_cx.py:14` |
| 4 | `os.environ |= {"A": "1"}` | report | silent (miss) |
| 5 | `os.environ.copy()` written and passed to a subprocess | silent | silent |
| 6 | the test using helper 1 | silent | silent |
| 7 | non-autouse session fixture | report | reported `:38` |
| 8 | base class `tearDown` doing `os.environ.pop("TOKEN", None)`, child test sets `TOKEN` | silent | **both reported**: the test (`:49`, documented gap) and the base `tearDown` itself (`:44`) |
| 9 | `clear(); update(saved)` in a `finally` | silent | silent |

`env_cand` takes a write in any def of a test document (`env_unit` is any `def_frame`), not only tests, fixtures and
set-up hooks, and `pt_undo` only recognises a restore that matches an earlier change of the same key. So undo code
that stands alone (row 8's base `tearDown`), a context manager that restores by computed key (row 1), and scripts that
live in a test directory (row 2) are all reported. Row 2 is on the corpus: saleor
`saleor/asgi/tests/checks/check_uvicorn_starts.py:31`.

**Corpus.** 14 matches in 8 of 19 repositories (jupyterhub 1, linkding 2, freqtrade 1, InvenTree 1, label-studio 4,
polar 3, superset 1, saleor 1); dify's run of this rule had not started when the report was written (section 4). All
read: 13 real, 1 false positive (saleor's script). Real examples:
`server/tests/observability/observability/test_http_middleware.py:21` and
`:33` (polar: `PROMETHEUS_MULTIPROC_DIR` left pointing at a deleted temporary directory), `label_studio/tests/conftest.py:91`
(moto's AWS credential variables in a function fixture), `jupyterhub/tests/test_roles.py:1418`,
`src/backend/InvenTree/InvenTree/test_tasks.py:671`. The README's corpus section is a placeholder (`CORPUS_TABLE`).

**Overlap.** Ruff B003 reports only rebinding `os.environ`; SIM112 is about names. Nothing else.

**Text.** Description 128 characters, fine. Message 406 characters (over 200); it names both fixes and the
`pytest.MonkeyPatch.context()` escape for broad-scoped fixtures, which is the right advice.

**Verdict: rework**, steer, on. Report only writes in tests, fixtures, set-up hooks and the helpers they call
(through the call graph), as `module-state` does with `pt_test_code`; never report a write that sits in undo position
(a teardown hook, a `finally`, after a fixture's `yield`, a cleanup callback); treat a computed-key write in undo code
over the keys of a dict the same def changed as a restore of `*`. Rows 1, 2 and 8 then pass.

### 2.6 `pytest-practice-module-state-without-monkeypatch`

**Value.** Module globals and class attributes outlive a test. Sources: pytest "Flaky tests", the monkeypatch docs,
asottile/detect-test-pollution (a bisecting tool for exactly this; 208 stars). The hazard is real and the matches show
harmful leaks: superset's `security_manager.on_view_menu_after_insert = Mock()`
(`tests/integration_tests/security_tests.py:172`), `SQLGLOT_DIALECTS["firebolt"] = FireboltOld`
(`tests/unit_tests/sql/parse_tests.py:3008`), saleor wrapping `transport.trigger_webhook_sync_promise` once more in every
test (`saleor/webhook/tests/circuit_breaker/test_breaker_board_decorator.py:24`), freqtrade's
`Exchange._ccxt_params = {"hello": "world"}` (`tests/exchange/test_exchange.py:252`, never restored).

**Correctness.**

| # | Counterexample (`/tmp/review-pytest/ms`) | Expected | Result |
|---|---|---|---|
| 1 | fixture sets `app.dependency_overrides[get_db]`, `.clear()` after the `yield` | silent | **reported** `tests/conftest.py:9` |
| 2 | the same, `app.dependency_overrides = {}` after the `yield` | silent | silent |
| 3 | test module's own spy list `CALLS.clear()` | silent | silent |
| 4 | pydantic settings instance `settings.debug = True` | report | reported `tests/test_cx.py:21` |
| 5 | `app.dependency_overrides[...]` in a test, never cleared | report | reported `:26` |
| 6 | alias | report | silent (documented) |
| 7 | write in a helper the test calls | report | silent (documented) |
| 8 | test registers through a decorator, then `REGISTRY.pop("tmp", None)` at the end | silent | **reported** `tests/test_cx2.py:12` (the cleanup itself) |
| 9 | `_CACHE.clear()` before a cold-cache assertion | silent | **reported** `:17` |
| 10 | `tasks.REGISTRY["fake"] = ...` left behind | report | reported `:23` |

Row 1: a mutator call counts as a write (or an undo) only when its receiver is a module-level container literal
(`ms_container`), so `.clear()`/`.pop()` on `app.dependency_overrides`, a FastAPI instance attribute, is neither and
the restore is not seen. Row 8: the decorator registered the entry through code the rule does not read, so the only
change it sees is the test's own `pop(k, None)` cleanup, which it reports; the message then says the test "changes
`REGISTRY.pop`". Row 9: clearing a cache is reported.

**Corpus.** 185 matches in 10 repositories (superset 80, freqtrade 38, dify 28, polar 21, saleor 6, pretix 4,
InvenTree 4, addons-server 2, mealie 1, DefectDojo 1). The README has no corpus section for this rule. Classified: 110,
covering every repository: all matches of polar, saleor, pretix, InvenTree, addons-server, mealie, DefectDojo and dify
(dify: 10 read in context, the other 18 are the same assignments in sibling tests of the same files), freqtrade 22 of 38
and superset 21 of 80 (spread over every pattern the superset labels show). Actionable 69/110 (63%):

| Shape | Read | Real |
|---|--:|--:|
| class attribute or module global replaced and never put back (superset `results_backend`, `security_manager.on_*`, `BaseScreenshot.cache`; dify `ToolManager._hardcoded_providers`, `ext_redis.redis_client.get = MagicMock(...)` in a module fixture; saleor wrappers; pretix `security._geoip`; freqtrade `Exchange._ccxt_params`, `PairLock.session = MagicMock()`) | 57 | 57 |
| restored by hand without `finally` (freqtrade `Trade.use_db`, pretix `checkoutflow.DEFAULT_FLOW`, superset `app.config = config`) | 10 | 10 |
| writes the value the attribute already has (freqtrade `Trade.use_db = True`, `tests/rpc/test_rpc_apiserver.py:1003`) | 2 | 2 |
| FastAPI `dependency_overrides` set and popped in a `finally` / after the `yield` (polar backoffice ×11, `server/tests/oauth2/conftest.py:95`) | 12 | 0 |
| cache, singleton or registry reset at the start of a test for isolation (`_ENGINE_CACHE.clear()`, `_queue_url_cache.clear()`, `policy._cache = None`, `_encrypter = None`, `registry.errors = {}`, `ApiBG.exchanges = {}`, `TaskRegistry._tasks.clear()`, `FakeRunScheduler.created.clear()`) | 21 | 0 |
| cleanup at the end of a test (`TaskRegistry._tasks.pop(k, None)`, `del FxaNotificationView.fxa_verifying_keys`) | 2 | 0 |
| restored by `self.addCleanup(self._restore_resolver)`, a bound method (DefectDojo `unittests/test_locations_feature_flag.py:21`) | 1 | 0 |
| inside superset's own `@with_config` override (`tests/integration_tests/charts/data/api_tests.py:318`) | 1 | 0 |
| `app.config["X"]: lambda ...`, an annotation, not a write (`tests/integration_tests/db_engine_specs/hive_tests.py:187`) | 1 | 0 |
| attribute of a `MagicMock` the `finally` replaces (`tests/unit_tests/views/test_soft_delete_filter.py:314`) | 1 | 0 |
| flask's per-request `session` proxy read through a module (dify `test_init_validate.py:239`) | 1 | 0 |
| a test fake's class state its own fixture resets (dify `FakeCryptographyClient.disabled_versions.add`) | 1 | 0 |

Polar alone reads 1/21 (`server/tests/fixtures/base.py:48`, where the teardown pops one of five keys).

**Overlap.** None in Ruff/pylint/flake8. pytest-randomly and detect-test-pollution find the same leaks at run time,
only when an order exposes them; the rule's value is finding them before.

**Text.** Description 154 characters, fine. Message 385 characters (over 200). For a mutator the label is the method
(`REGISTRY.pop`, `_ENGINE_CACHE.clear`), so the message says the test "changes `_ENGINE_CACHE.clear`"; name the
receiver. The README's rule-details section is the `.rerail` header comment pasted as prose: the left-alone examples
lost their code formatting and the section has no sources list, overlap or corpus numbers.

**Verdict: rework**, log, off. Changes: (1) count a mutator call on a shared object's attribute
(`app.dependency_overrides.clear()`/`.pop(k)`) as the undo when it sits in undo code (12 matches); (2) leave `.clear()`
and `pop(k, default)`/`del` alone when they are the only change of that target in the def: a reset or a cleanup (12);
(3) accept `addCleanup(self.<method>)` whose method writes the target, in the library so every leak rule gets it (1);
(4) skip annotated assignments without a value (1). That removes 26 of the 41 non-actionable matches read and lifts the
sample to 69/84 (82%); the rest (resets by assignment, `= None` on a singleton, test fakes) need judgement a static
rule lacks. Log is right after the fixes too; steer only if a re-measure reaches 90%.

## 3. Pack-level findings

**The four leak rules disagree with each other.** They check one hazard (state a test changes and does not put back)
on four kinds of state, and a reader would expect one policy. They have three:

| Question | django-settings | environ | module-state | scoped-fixture |
|---|---|---|---|---|
| Restore by hand at the end, no `finally` | reported | reported | reported | **accepted as a restore** |
| Module-level write in a test module | **reported** | left alone | left alone | n/a |
| Write in a helper the test calls | reported | reported (any def in a test document, including scripts) | **not read** | not read |
| `pop(k, default)` / `.clear()` in undo code | n/a (setting writes) | counted | **not counted** (non-literal containers) | **not counted** |

The pack should pick one answer per row and put it in `lib/pytest.rerail` (an "undo code" predicate that does not
depend on an earlier matching write, and one "runs inside a test" predicate), so the rules cannot drift again.

**Library (`rules/lib/pytest.rerail`).** Good: a documented interface, fixture resolution by pytest's own lookup
order through the conftest chain (`pt_resolves`), `pt_undo` asked on demand so it finishes on superset and dify, imports
resolved through `sym_of`. Gaps that cost every leak rule: `addCleanup(self._restore)` (a bound method) is not a
cleanup callback (DefectDojo `unittests/test_locations_feature_flag.py:21`); a teardown in a base class or a
conftest fixture the test requests is not seen; `pt_test_doc` takes every file under a `tests/` directory, so runner
scripts (`saleor/asgi/tests/checks/check_uvicorn_starts.py:31`) count as test code; `request.getfixturevalue(...)` is
not modelled.

**Rule that belongs in another pack: `django-settings-in-test` → `django`.** It is Django's rule (Django's own docs,
`override_settings`, `self.settings()`, `setting_changed`), it is the test-side half of
`django-practice-settings-runtime-assignment`, and its matches are mostly in Django test-runner suites the pytest pack
never installs in: DefectDojo has no `conftest.py` and no pytest dependency, so the pack's `match` (a `pytest`
dependency or a `conftest.py`) skips it, and with it 15 of the rule's 24 corpus matches. In the django pack it reaches
every Django project, and the pair reports each settings write exactly once (their test-document definitions are
identical).

**Missing rules.** The lead's own build list (`NOTES-pytest.md` section 3) has two more rules that were not built:

- `yield-fixture-unsafe-setup` (rank 7, log): pytest's fixtures page says "If a yield fixture raises an exception before
  yielding, pytest won't try to run the teardown code" (checked); the mining found 85 shapes in 12 repositories. It fits
  this pack's isolation theme and should be built in log as the notes planned.
- `sleep-in-test` (rank 8, log): 218 sleeps in 13 repositories, 112 of them `sleep(1)`; waits on real threads and
  processes are legitimate, and no corpus evidence separates the harmful ones. I would not build it.

No other gap the corpus shows: the notes' drop decisions (PGH005, PT012, PT017, pytest's own return warning, the
`python_classes` config dependency) hold.

**README.** Three sections are unfinished: `environ-without-monkeypatch`'s corpus line is the placeholder
`CORPUS_TABLE`, `scoped-fixture-mutated`'s superset row is `SUPERSET_N`/`SUPERSET_REAL`, and
`module-state-without-monkeypatch`'s section is the rule's header comment pasted in, with no overlap, sources or corpus
numbers. `patch-misses-from-import`'s table leaves saleor out (5 real matches). `NOTES-pytest.md` S1 quotes "patch the
name used by the system under test", which is not on the page (the page says "patch where an object is looked up").
Every rule's message is longer than the 200-character criterion except `patch-misses-from-import` (185); the django
pack's model rules run long too (20 of 34 over 200), so this is house style, but the criterion is the contract.

**Can the pack be pitched on its own?** Not yet. Its strongest two rules are precise and cross-file but rare (14 and
4 matches across 20 repositories); the leak rules are where the volume is, and two of them read 72% and 63% today.
After the reworks in section 2 and the move of the Django rule, it is a coherent "test isolation and mock sanity" pack of
five rules that installs beside the Python team bundle; on its own it is thin and every rule is `test-quality`, which a
team ranks below the data-safety and availability packs.

## 4. Corpus and commands

All 20 repositories of the BRIEF's corpus, at the pinned commits of `~/rerail-bench/python-arch/corpus/MANIFEST.tsv`.
No repository was added. Each was run with all six rules in one run
(`~/rerail-bench/python-arch/bin/corpus-run <repo> review-pytest-all-<repo> packs/pytest/rules/*.rerail`, outputs in
`~/rerail-bench/python-arch/runs/review-pytest-all-<repo>/`). dify's six-rule run stopped on the engine's content-store
limit ("content store hard limit rejects new payloads"), and so did a three-rule rerun (`review-pytest-dify-a`: environ,
decorator-order, patch-misses); dify was then run one rule at a time (`review-pytest-dify-b`: module-state, `-c`:
scoped-fixture, `review-pytest-dify-<rule>` for the other three). dify is not a Django project, so
`django-settings-in-test` was not run there ("n/r"). The one-rule dify run of `environ-without-monkeypatch`
(`review-pytest-dify-environ-without-monkeypatch`) had waited over two hours for one of the machine's large-repository
slots, which other reviewers' runs held, and had not started when this report was written ("queued"). The `ast` scan
found 16 environment writes inside defs of dify's test files (`api/tests/pytest_dify.py:50`-`59`, a helper of dify's
own pytest plugin; `dify-agent/tests/local/dify_agent/server/test_observability_isolation.py:141`-`148`, `:203`), so
the rule's dify number is not known.

| Repository | Commit | .py files | settings | environ | module-state | decorator-order | patch-misses | scoped-fixture |
|---|---|--:|--:|--:|--:|--:|--:|--:|
| `fastapi__full-stack-fastapi-template` | `77e550a73bb4` | 43 | 0 | 0 | 0 | 0 | 0 | 0 |
| `fastapi-users__fastapi-users` | `e9c57ff7ff2f` | 88 | 0 | 0 | 0 | 0 | 0 | 0 |
| `jupyterhub__jupyterhub` | `3ca37442a18e` | 158 | 0 | 1 | 0 | 0 | 0 | 16 |
| `sissbruecker__linkding` | `27b7303baf41` | 229 | 0 | 2 | 0 | 0 | 0 | 0 |
| `open-webui__open-webui` | `0666e59d26a4` | 330 | 0 | 0 | 0 | 0 | 0 | 0 |
| `paperless-ngx__paperless-ngx` | `faa6f09f289b` | 424 | 0 | 0 | 0 | 0 | 0 | 0 |
| `freqtrade__freqtrade` | `7fcb509a434c` | 487 | 0 | 1 | 38 | 0 | 1 | 0 |
| `healthchecks__healthchecks` | `c1039ad23395` | 658 | 0 | 0 | 0 | 0 | 0 | 0 |
| `Netflix__dispatch` | `dd2837e82a0b` | 717 | 0 | 0 | 0 | 0 | 0 | 0 |
| `mealie-recipes__mealie` | `8dbb92808d5a` | 723 | 0 | 0 | 1 | 0 | 0 | 0 |
| `inventree__InvenTree` | `939df605f43d` | 731 | 2 | 1 | 4 | 0 | 0 | 0 |
| `HumanSignal__label-studio` | `2f42caecbf6c` | 775 | 5 | 4 | 0 | 0 | 0 | 0 |
| `makeplane__plane` | `723bb178f97b` | 784 | 0 | 0 | 0 | 0 | 0 | 0 |
| `pretix__pretix` | `40ea9097d0b2` | 1,228 | 2 | 0 | 4 | 0 | 0 | 0 |
| `mozilla__addons-server` | `7e3374fe5383` | 1,426 | 0 | 0 | 2 | 2 | 0 | 0 |
| `polarsource__polar` | `872272fbfc6e` | 2,522 | 0 | 3 | 21 | 0 | 0 | 0 |
| `DefectDojo__django-DefectDojo` | `cd0a4483a224` | 2,601 | 15 | 0 | 1 | 0 | 0 | 0 |
| `apache__superset` | `fdf61721d056` | 3,191 | 0 | 1 | 80 | 0 | 8 | 72 |
| `saleor__saleor` | `e1ccf4d58002` | 4,385 | 0 | 1 | 6 | 2 | 5 | 0 |
| `langgenius__dify` | `724ac15121ee` | 5,191 | n/r | queued | 28 | 0 | 0 | 0 |
| **Total** | | | **24** | **14** | **185** | **4** | **14** | **88** |

Other commands:

- Each rule's cases: `nice -n 19 node dist/cli/cli.js rules test packs/pytest/rules/<rule>.rerail` (6 × "1 passed").
- Pack contract: `nice -n 19 npx vitest run test/engine/catalog/packs.test.ts -t "pack pytest"` (15 passed).
- Counterexamples: `~/rerail-bench/python-arch/bin/corpus-run /tmp/review-pytest/<dir> review-pytest-<rule>-cx
  packs/pytest/rules/<rule>.rerail` for `dst`, `env`, `ms`, `pdo`, `pmi`, `sfm`, then `review-pytest-ms-cx2` and
  `review-pytest-sfm-cx2` for the cleanup shapes.
- Independent scan: `python3 /tmp/review-pytest/probe.py ~/rerail-bench/python-arch/corpus <repo>` (stacked patch
  decorators, environment and settings writes in test files).
- Sources were opened with `curl` and the reader (`/tmp/review-pytest/src/`): the Django 5.2 testing-tools page, the
  pytest-django helpers page, the unittest.mock page, both Batchelder posts, the Stack Overflow question, the pytest
  flaky, monkeypatch and fixtures pages, flakytest.dev's isolation guide and detect-test-pollution's repository.

## Applied

Changes made after this review (the pack fixer), per rule:

- **Library (`rules/lib/pytest.rerail`).** One definition of a change and of its undo, used by all four leak rules:
  `pt_loc` (every write read as a path below each expression of its chain: stores, `del`, augmented assignment,
  setattr/delattr, list/dict/set mutators on any receiver), `pt_covers` (a restore of the path, of a container it
  runs through, or a whole-container form), `pt_undo` (asked on demand; no earlier matching write needed: after the
  yield of a fixture or `@contextmanager`, `finally`, teardown hooks and `__exit__` of the class or a base class in
  any document, `teardown_module`, cleanup callbacks including bound methods `self._restore`, defs that undo code
  calls, the teardown of a requested function-scoped fixture or a covering autouse one, the caller's undo for a
  helper), `pt_guarded` (monkeypatch/patch active where the change runs), `pt_runs_in_test` (tests, fixtures, set-up
  hooks, and test-document helpers reached through the call graph; not module-level code, not session set-up, not
  defs no test calls such as runner scripts), `pt_session_def`. A computed key in a loop in undo code restores every
  key. `request.getfixturevalue("x")` is modelled. Bug fixed: the expression of `yield f(...)` counted as teardown.
- **`patch-misses-from-import`.** Message names when to drop the patch (183 characters). The `create=True` /
  `raising=False` exemption is gone (the rule reads only names the module binds); cases added. Conftest-fixture
  patches (row 1) not done.
- **`patch-decorator-order`.** Names compared with every underscore removed (row 5: `mock_assert_true` ↔
  `assertTrue`, `mock_email_client` ↔ `EmailClient`); cases added. Message trimmed to the review's text (149);
  description reworded without code.
- **`scoped-fixture-mutated`.** Rebuilt on the library: `pop(k, None)`, `del`, `clear()` in undo code count as the
  undo (row 9); restore by hand and two plain stores are reported at the first change (rows 1, 2);
  `getfixturevalue` (row 7) and plain test-document helpers the fixture is passed to (row 5) are read. Message 186
  characters, no longer says "This test" for fixtures. Mode steer, on.
- **`django-settings-in-test`.** Rebuilt on the library: runner scripts and defs no test calls are left alone (row
  10); module-level writes are left alone, as for the environment (row 8); base-class tearDown anywhere (row 2) and
  `override_settings(...).enable()` (row 1) are seen; helpers are read only when a test calls them. Still reads test
  documents only, so it never overlaps `django-practice-settings-runtime-assignment` (rows 9's `shop/testing.py` is
  that rule's). Not moved to the django pack (coordinator's step). Message 180 characters.
- **`environ-without-monkeypatch`.** Rebuilt on the library: only writes inside a test (rows 2, saleor's check
  script); undo code is never reported and a base tearDown restores (row 8); a context manager restoring by key in a
  loop restores (row 1); `os.environ |= {...}` is caught (row 4). Message 179 characters.
- **`module-state-without-monkeypatch`.** Rebuilt on the library: a mutator on any object in undo code is the undo
  (`dependency_overrides.clear()`/`pop`, change 1); resets and cleanups (`clear()`, `discard()`, `pop(k, default)`,
  `del`) are never reported (change 2); `addCleanup(self._restore)` restores (change 3); bare annotations are no
  write (change 4); helpers a test calls are read; the message names the receiver. Mode log, off.
- **Text, README, pack.json.** Every description is one sentence without code; every message is at most 200
  characters. README rewritten (no placeholders or pasted header; a section on the shared leak policy), rules in the
  reading order of section 1 in both the README and `pack.json`, which now carries `on`. `NOTES-pytest.md` S1 quotes
  the page's own words.
- **Not built** (out of scope): `yield-fixture-unsafe-setup`, `sleep-in-test`.
- **Re-measured** (Junction `corpus-run`, all six rules, the 13 repositories under 1,000 `.py` files; the 7 larger ones
  were refused by the fleet boxes and keep this review's numbers in the README, marked not re-run): patch-misses 1
  (freqtrade, real); decorator-order 0; scoped-fixture 19 (jupyterhub, 18 real: +2 restores by hand now reported, 1
  false positive `conftest.py:354`, a slice store `app.services[:] = []` not read as a restore); django-settings 7
  (label-studio 5, InvenTree 2, all real, unchanged); environ 9 (unchanged, all real); module-state 43 (freqtrade 38:
  36 real, 2 resets by assignment; InvenTree 4, mealie 1, unchanged). Counterexample directories of section 2 re-run:
  every row the verdicts asked for now behaves as expected, except the documented gaps (aliases, the `mod.name` read
  in patch-misses row 3, the hidden swap in decorator-order row 2).
