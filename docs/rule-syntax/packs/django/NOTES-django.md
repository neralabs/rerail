# Notes: batch django (Rerail's own Django practice rules)

17 rules, written by hand as .rr (1,210 lines, 24-142 per rule), over one library, `lib/python/django.rr` (293
lines). None is a port, so there is no upstream to score against: each rule is checked by its cases
(`cases/<id>/`, run by the Rust test `django_practice_rules_report_exactly_their_caught_lines`) and by a run on the 31
pinned apps of [README.md](README.md#the-corpus), whose matches were read. What each rule matches, leaves alone and
misses, with its corpus numbers and sources, is in README.md's "Rule details".

## Dropped candidates

Written, run on the corpus and dropped for want of real matches (rule and cases kept outside the pack for a later
corpus):

- `django-practice-no-query-at-import`: a queryset evaluated at module or class-body level. 214 matches in 2 repos,
  all in scripts that are run, not imported (WeblateOrg/weblate `ci/migrate-scripts/*.py`, 183, fed to
  `manage.py shell`; ansible/awx `tools/scripts/ig-hotfix/*.py`, 31); with non-module file names and `scripts`
  directories left alone, 0 in 31 apps. An `AppConfig.ready()` probe found 0, as did a text search of every `apps.py`.
- `django-practice-sync-orm-in-async`: blocking ORM calls inside `async def`. 0 matches in the 30 repos its last
  version ran on; a CPython `ast` scan of every non-test file (netbox included) found 237 `async def`s and no sync ORM
  call in any of them: the corpus's async code already uses `aget`/`acreate`/`async for`/`sync_to_async`.
- `django-practice-access-mixin-not-leftmost`: an access mixin listed after a view base. 0 matches in 31 apps. A text
  search agrees: the only bases lists with an access-style mixin after a `…View` name are pretix's and readthedocs'
  `…ViewMixin` classes (not views) placed before `EventPermissionRequiredMixin`/`UserPassesTestMixin`, which still
  come before the view, and nautobot's `NautobotViewSetMixin(GenericViewSet, …, AccessMixin, …)`, whose `AccessMixin`
  has no dispatch check.
- `django-practice-user-password-not-hashed`: a raw `password=` on a user create or `user.password = …`. 2 matches, 1
  real; the real one, nautobot `nautobot/core/testing/__init__.py:90`, is a test helper the library's `dj_test_doc`
  now skips, and the other, WeblateOrg/weblate `weblate/auth/management/commands/importusers.py:79`, imports a
  `dumpdata` JSON whose passwords are already hashes. Every other raw-password create in the corpus is in tests.

Probes and tiers not built:

- Cache writes and HTTP calls inside atomic (a Log tier of enqueue-on-commit): 5 matches in 4 repos, 4 of them
  `cache.delete`/`delete_many` (the weak case, a stale refill race) and 1 `requests.post` (pretix
  `api/webhooks.py`). Too few.
- `global_settings` imports (a Log sibling of settings-module-import): 2 outside tests, both in settings files
  extending a default (readthedocs `readthedocs/settings/base.py`, saleor `saleor/settings.py`). No app code to steer.
- fk-id-not-relation on any object (`x.<fk>.id|pk` with `<fk>` the name of some model's foreign key): 867 more matches
  in 28 repos, about 22 of 30 sampled real; the rest are non-model wrappers sharing the attribute name.
- exists-for-existence on any zero-argument `.count()` in a Django file: 130 more matches, mixed; some querysets are
  reused afterwards.
- model-init-override on every model `__init__`: 54 matches, about 40 of which only set non-field attributes or
  rewrite kwargs (the docs' advice, not the docs' hazard), and 3 query expressions.

Research verdicts that never became rules:

- `user-model-direct-reference` (a relation to `auth.User` instead of `AUTH_USER_MODEL`): pylint-django E5142
  `imported-auth-user` and E5141 `hard-coded-auth-user` cover it.
- `migration-imports-model` (a migration importing app models for RunPython): django-migration-linter
  RUNPYTHON_MODEL_IMPORT makes the same check.
- `runpython-without-reverse`: django-migration-linter RUNPYTHON_REVERSIBLE and pylint-django W5197
  `missing-backwards-migration-callable`.
- `select-for-update-in-atomic`: the runtime raises TransactionManagementError at first execution, and lock
  querysets are built in helpers outside any atomic by design (saleor `saleor/checkout/lock_objects.py`), so a
  lexical rule would mostly hit those.
- `open-redirect`: the docs give no guidance (`url_has_allowed_host_and_scheme` appears only in release notes),
  Semgrep's `open-redirect.yaml` covers it, and it needs taint flow.
- `request-data-to-model`: no crisp shape; the one crisp form, `X.objects.create(**request.POST)`, is Semgrep's
  `mass-assignment.yaml`, and Ruff DJ006/DJ007 cover the adjacent ModelForm advice.
- `save-override-signature`: the signature half is pylint W0221 `arguments-differ`; the "never calls super" half has
  no second source (Semgrep's `django-db-model-save-super` covers it).
- `stacked-userpassestest-mixins`: stacked mixins are project mixins in other files.
- `on_commit(lambda: f(loopvar))` in a loop: Ruff B023 `function-uses-loop-variable`.
- Left unsure for want of a second source or a docs statement: `float-for-money`, `post-save-saves-instance`,
  `get-without-404`, `migration-mixes-runpython-and-schema`, `signal-local-receiver-weak`, `multiple-view-bases`.

## Library: `lib/python/django.rr`

What it gives the rules, by group (its header is the full interface):

- Models: `dj_model` (a `models.Model` subclass through the run's classes, or a class declaring a model field; query
  expressions with only an `output_field` are not models), `dj_field`, `dj_fk`, `dj_fk_class`, `dj_model_super` and
  `dj_has_field` (fields inherited from bases in the run, as a linear closure), `dj_attname` (a field's attribute,
  `x_id` for a foreign key), `dj_model_method`, `dj_field_call`.
- Querysets: `dj_manager`, `dj_qs` (a chain from `objects`/`_default_manager`/`_base_manager`), `dj_qs_call`,
  `dj_qs_root`, `dj_qs_model` (the chain's model class of the run).
- Transactions: `dj_atomic`, `dj_atomic_with`, `dj_atomic_def`, `dj_in_atomic`, and `dj_region_stop` (def, lambda,
  class: "the def this code runs in").
- Views: `dj_view_lib` (Django and DRF view classes, their `…Mixin` classes excluded), `dj_view_class`.
- Files: `dj_migration_doc`; `dj_test_doc` (tests/test/testing/unittests directories, test_*.py, *_tests.py,
  *_test.py, tests.py, test.py, conftest.py); `dj_django_doc` (the file imports django or rest_framework).
- Settings: `dj_settings` (django.conf.settings).
- Python tests shared by several rules: `py_truth`, `py_empty_cmp`, `py_empty_test`, `py_no_args`.

Open proposals, each written rule-locally today (checked against the current django.rr: none is in it). They were not
moved because fewer than two kept rules use them.

- `dj_row(o, c)`: name o holds a row of model c (`self` in its method, the latest `M.objects.get(…)` assignment, a
  `for` target over `M.objects…`). Now fk-id-not-relation's `fir_row`.
- `dj_related_qs(e)`: `<expr>.<rel>` used as a related manager in a Django file. Would let exists-for-existence and
  count-not-len take `obj.rel.count() > 0` and `len(obj.rel.all())`: about 15 of 40 sampled count() misses had this
  shape, all real.
- `dj_atomic_like_with(w, b)`: a with-statement calling a wrapper named like atomic (`traced_atomic_transaction()`).
  Now in enqueue-on-commit's `eoc_region`; atomic-around-caught-db-error would close a gap with it.
- `dj_db_error(q)`: the django.db error classes. Now atomic-around-caught-db-error's `adb_error_module`/`adb_error`.
- `dj_handler_reraises(x)`: every path of except handler x ends in `raise`. atomic-around-caught-db-error
  (`adb_raise_block`, `adb_branch`, `adb_has_else`, `adb_may_end`) and enqueue-on-commit (`eoc_…`, the same four)
  hold identical copies; this one has two users and should move.
- `dj_new_row_if(i, f)`: an `if` in save method f that holds only for a new row. Now save-ignores-update-fields'
  `siuf_new_if`/`siuf_new_test`; a rule about signals or save hooks that must skip inserts could reuse it.
- `dj_import_time(n)`: n runs when its module is imported. Now lazy-translation-at-import's `lti_at_import`.
- `dj_settings_module_doc(d)`: d is a project settings module. Now settings-module-import's `smi_doc`/`smi_resolves`.
- `dj_in_async(n)`: n's nearest def, lambda or class is an `async def`. Now async-request-user's `aru_in_async`.
- `dj_drf_view_class(c)`: c derives from a DRF view. Now function-decorator-on-method's `fdm_drf`.
- python (not django), `log_call_arg(n)`: n is in the arguments of a `<…log…>.debug/info/warning/…(…)` call. Now
  host-header-without-get-host's `hhg_logged`.

## Engine issues found

[ENGINE-BUGS.md](ENGINE-BUGS.md) has each bug with its reproduction, code location, evidence, workaround and suggested
fix: the `Vocabulary::rows` deadlock, the annotation-guest blowup on huge string modules, module names taken from the
repository root, `sym_of` dropping relative imports of two or more dots, the machine-wide tape cache, and very large
repositories not finishing.

Smaller, not bugs: a rule-local relation that joins two unrelated sides is evaluated as a cross product with no warning
(a `text(r, "self"), param(f, "self", 0), dj_model_method(_, f, _)` join never finished on netbox; binding through
`look(r, "self", f)` fixed it); `within_stop(n, a, stop)` does not treat `a` itself as a stop (as documented,
"strictly between", but easy to miss: a lambda passed as a default had to be excluded separately).
`docs/rule-syntax/site/check_rr.py` fails only on `lib/python/pytest-collect.rr:342: unbound k, product, w`, a file
outside this batch.

## Overlap with existing tools (partial)

- orm-lens DOL001/DOL002 (a VS Code extension, regex editor rules): `.count() > 0`/`>= 1`/`== 0` → `exists()`, the
  count-comparison shape of exists-for-existence; not its `len()` or truth shapes. DOL003/DOL004 (`first() is None`)
  match a shape no rule here takes.
- orm-lens DOL007 and the `orm-lens nplusone` CLI (an AST N+1 analyzer with one-hop helper resolution): N+1 queries in
  loops, as select-related-in-loop.
- Semgrep `python/django/performance/access-foreign-keys`: `$X.user.id` only, `request.user.id` excluded;
  fk-id-not-relation takes any foreign key, and `.pk` too.
- Semgrep `use-count-method`: looks for a `.len()` method, which does not exist, so it never fires on `len(qs)`
  (count-not-len).
- Semgrep `django-db-model-save-super` and `use-decimalfield-for-money`: the save-without-super and float-for-money
  candidates, not written.
- Semgrep `open-redirect.yaml` and `mass-assignment.yaml`: why open-redirect and request-data-to-model were dropped.
- Ruff PLC1802 `len-test`: flags `if len(x):`, but its advice (`if x:`) still fetches every row; not
  exists-for-existence's check.
- Ruff TID251 / flake8-tidy-imports `banned-modules`: catches a settings-module import only when a user lists the
  module by hand; no default check.
- Django's check fields.W161: a `timezone.now()`-like default on a date or time field; default-called-once leaves
  exactly those to it.
- django-upgrade's `headers` fixer rewrites `request.META["HTTP_HOST"]` to `request.headers["host"]`, which still
  bypasses `get_host()`; host-header-without-get-host matches both forms.
