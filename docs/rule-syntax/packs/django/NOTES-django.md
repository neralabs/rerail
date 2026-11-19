# Notes: batch django (Rerail's own Django practice rules)

27 rules, written by hand as .rerail (2,264 lines, 24-181 per rule), over one library, `lib/python/django.rerail` (499
lines). None is a port, so there is no upstream to score against: each rule is checked by its cases
(`cases/<id>/`, run by each rule file's own test, `.test cases exactly`) and by a run on the 31
pinned apps of [README.md](README.md#the-corpus), whose matches were read. What each rule matches, leaves alone and
misses, with its corpus numbers and sources, is in README.md's "Rule details".

## Dropped candidates

Written, run on the corpus and dropped (the first four were never in the pack; the last two are in its git history,
commit `72cf09fbac`):

- `django-practice-no-query-at-import`: a queryset evaluated at module or class-body level. 214 matches in 2 repos,
  all in scripts that are run, not imported (WeblateOrg/weblate `ci/migrate-scripts/*.py`, 183, fed to
  `manage.py shell`; ansible/awx `tools/scripts/ig-hotfix/*.py`, 31); with non-module file names and `scripts`
  directories left alone, 0 in 31 apps. An `AppConfig.ready()` probe found 0, as did a text search of every `apps.py`.
- `django-practice-sync-orm-in-async`: blocking ORM calls inside `async def`. 0 matches in the 30 repos its last
  version ran on; a CPython `ast` scan of every non-test file (netbox included) found 237 `async def`s and no sync ORM
  call in any of them: the corpus's async code already uses `aget`/`acreate`/`async for`/`sync_to_async`. Its
  cross-file form, `django-practice-sync-orm-reached-from-async` (Log), follows the sync functions an `async def`
  calls.
- `django-practice-access-mixin-not-leftmost`: an access mixin listed after a view base. 0 matches in 31 apps. A text
  search agrees: the only bases lists with an access-style mixin after a `…View` name are pretix's and readthedocs'
  `…ViewMixin` classes (not views) placed before `EventPermissionRequiredMixin`/`UserPassesTestMixin`, which still
  come before the view, and nautobot's `NautobotViewSetMixin(GenericViewSet, …, AccessMixin, …)`, whose `AccessMixin`
  has no dispatch check.
- `django-practice-user-password-not-hashed`: a raw `password=` on a user create or `user.password = …`. 2 matches, 1
  real; the real one, nautobot `nautobot/core/testing/__init__.py:90`, is a test helper the library's `dj_test_doc`
  now skips, and the other, WeblateOrg/weblate `weblate/auth/management/commands/importusers.py:79`, imports a
  `dumpdata` JSON whose passwords are already hashes. Every other raw-password create in the corpus is in tests.
- `django-practice-receiver-never-connected`: a `@receiver` in a module nothing imports, so the receiver never
  connects. 0 matches in 31 repos (full run, every repo finished). Projects that use `@receiver` load the module
  (imported in `ready()`, from models, or by an autodiscover helper such as authentik's `import_relative("signals")`);
  a text search of the corpus found no unloaded receiver module either.
- `django-practice-index-not-concurrent` (Log): an index built on an existing PostgreSQL table without CONCURRENTLY
  (AddIndex; AddField/AlterField with db_index, unique, a OneToOneField or ForeignKey; AddConstraint(UniqueConstraint)).
  Its last judged full run had 1,519 matches in 27 repos, 90 of 90 read real (a spread sample); its last version ran
  2,216 matches in 28 repos (AddField 1,126, AddIndex 497, AlterField 306, AddConstraint 287), not judged. Dropped
  because django-migration-linter's `CREATE_INDEX` makes the same check on `sqlmigrate` output (`CREATE (UNIQUE )?INDEX`
  without CONCURRENTLY, tables created in the same migration skipped). That linter misses ADD CONSTRAINT UNIQUE
  (`unique=True`, `AddConstraint(UniqueConstraint)`, `AlterUniqueTogether`): a possible narrow rule, not built.

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
- `select-for-update-in-atomic`, as a lexical rule: lock querysets are built in helpers outside any atomic by design
  (saleor `saleor/checkout/lock_objects.py`), so it would mostly hit those. `django-practice-select-for-update-outside-atomic`
  follows the call graph instead and reports only paths it can show to run with no transaction.
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

## Library: `lib/python/django.rerail`

What it gives the rules, by group (its header is the full interface):

- Models: `dj_model` (a `models.Model` subclass through the run's classes, or a class declaring a model field; query
  expressions with only an `output_field` are not models), `dj_field`, `dj_fk`, `dj_fk_class`, `dj_model_super` and
  `dj_has_field` (fields inherited from bases in the run, as a linear closure), `dj_attname` (a field's attribute,
  `x_id` for a foreign key), `dj_number_field`, `dj_model_method`, `dj_field_call`.
- Querysets: `dj_manager`, `dj_qs` (a chain from `objects`/`_default_manager`/`_base_manager`), `dj_qs_call`,
  `dj_qs_root`, `dj_qs_model` and `dj_qs_class` (the chain's model class of the run), `dj_qs_has`, `dj_row_method`.
- Instances: `dj_fetch_call` (a fetch of one row of a model of the run), `dj_instance_class` (a name holding an
  instance: `self` in a model method, a constructor call or a fetch).
- Transactions: `dj_atomic`, `dj_atomic_with`, `dj_atomic_def`, `dj_in_atomic`, `dj_atomic_like_with` (a wrapper
  named like atomic, Saleor's `traced_atomic_transaction()`), `dj_in_transaction`, `dj_atomic_requests` and
  `dj_atomic_requests_value`, and `dj_region_stop` (def, lambda, class: "the def this code runs in").
- Side effects: `dj_side_effect` (a task enqueued or a mail sent), `dj_task_waits_for_commit` (a task class that
  defers to on_commit), `dj_dispatch_in_failing_handler`, `dj_dispatch_result_used`: what enqueue-on-commit and
  dispatch-in-atomic-request share.
- Views: `dj_view_lib` (Django and DRF view classes, their `…Mixin` classes excluded), `dj_view_class`.
- Files: `dj_migration_doc`; `dj_test_doc` (tests/test/testing/unittests directories, test_*.py, *_tests.py,
  *_test.py, tests.py, test.py, conftest.py); `dj_django_doc` (the file imports django or rest_framework).
- Settings: `dj_settings` (django.conf.settings).
- Python tests shared by several rules: `py_truth`, `py_empty_cmp`, `py_empty_test`, `py_no_args`,
  `py_reraising_handler` (every path of an except clause ends in `raise`).

Open proposals, each written rule-locally today (checked against the current django.rerail: none is in it):

- `dj_row(o, c)`: name o holds a row of model c (`self` in its method, the latest `M.objects.get(…)` assignment, a
  `for` target over `M.objects…`). Now fk-id-not-relation's `fir_row`; the library's `dj_instance_class` does not
  follow the latest assignment or `for` targets.
- `dj_related_qs(e)`: `<expr>.<rel>` used as a related manager in a Django file. Would let exists-for-existence and
  count-not-len take `obj.rel.count() > 0` and `len(obj.rel.all())`: about 15 of 40 sampled count() misses had this
  shape, all real.
- `dj_db_error(q)`: the django.db error classes. Now atomic-around-caught-db-error's `adb_error_module`/`adb_error`.
- `dj_new_row_if(i, f)`: an `if` in save method f that holds only for a new row. Now save-ignores-update-fields'
  `siuf_new_if`/`siuf_new_test`; a rule about signals or save hooks that must skip inserts could reuse it.
- `dj_import_time(n)`: n runs when its module is imported. Now lazy-translation-at-import's `lti_at_import`.
- `dj_settings_module_doc(d)`: d is a project settings module. Now settings-module-import's `smi_doc`/`smi_resolves`.
- `dj_in_async(n)`: n's nearest def, lambda or class is an `async def`. Now async-request-user's `aru_in_async`.
- `dj_drf_q(q)`: q is a Django REST framework class (or a vendored copy's `ViewSetMixin`), through the run's classes.
  Two rules hold their own: function-decorator-on-method's `fdm_drf_q`/`fdm_drf` and get-handler-writes' `ghw_drf_q`.
- python (not django), `log_call_arg(n)`: n is in the arguments of a `<…log…>.debug/info/warning/…(…)` call. Now
  host-header-without-get-host's `hhg_logged`.
- `dj_receiver(mq, signal, fname, f, at)`: a pre_save/post_save receiver for model sender mq (`@receiver(sig,
  sender=M)`, sig maybe a list or tuple; `sig.connect(fn, sender=M)`). Now bulk-write-skips-hooks' `bwh_receiver`,
  `bwh_sig_arg`, `bwh_connected`.
- python, `py_bare_super(f)`: method f's body only calls the base's method of its name (`super().x(…)` or the old
  style `Base.x(self, …)`). Now bulk-write-skips-hooks' `bwh_bare_super`.
- `dj_counter_update(c)` and `dj_f_ref(e, x)`: an `update()` whose every value is `F("x") ± n` on its own field;
  `F("x")` resolved through imports. Now bulk-write-skips-hooks' `bwh_counter_update`/`bwh_f_of`; read-modify-write
  recommends exactly that write.
- get-or-create-needs-unique's `goc_root`/`goc_mgr` walk a chain through `all()`, `using()` and `select_for_update()`
  only (a `filter()` before get_or_create changes the lookup); a `dj_qs_root` variant limited to such steps.
- Widen `dj_instance_class` to what the rules still add on their own: `get_object_or_404(M, …)`, `self.get_object()`
  in a view class with `model`/`queryset`, a `for` target over `M.objects…`, `o, created = …get_or_create()`,
  `request.user`, a parameter annotated with a model (get-handler-writes' `ghw_instance_class`, read-modify-write's
  `rmw_model`, task-arg-model-instance's `tam_instance`).
- `dj_meta(mc, k, v)`: a model's Meta options as Django inherits them (its own Meta; an abstract base's only with no
  own Meta or `class Meta(Base.Meta)`; a concrete base's table constraints), and a manager's queryset class through
  `get_queryset()`. Now get-or-create-needs-unique's `goc_meta_opt`, `goc_meta_from`, `goc_meta_class`,
  `goc_abstract`, `goc_mgr_class`.
- python/flow, `py_block_open(b)`: block b can complete normally (if/try/with, loop `else` and `break`, `while True`,
  match totality), seeded by a demand relation. Now clean-field-returns-value's
  `cfr_open`/`cfr_s_open`/`cfr_match_total`.
- `dj_form_class(c)`, `dj_model_form_class(c)`: form classes through the run's bases, including outside
  `…Form`/`…ModelForm` classes. Now clean-field-returns-value's `cfr_form` and modelform-clean-calls-super's
  `mcs_model_form`/`mcs_formset`.
- Widen `dj_in_transaction(n)` with what select-for-update-outside-atomic adds: a with-item after an atomic item, a
  context manager of the run that yields inside a transaction, `@method_decorator(transaction.atomic)` on a def or
  (with `name=`) on its class, and a decorator of the run whose wrapper calls the function in a transaction
  (`sfu_in_atomic`, `sfu_atomic_cm`, `sfu_atomic_def`, `sfu_atomic_wrapper`, `sfu_method_atomic`). enqueue-on-commit,
  atomic-around-caught-db-error and dispatch-in-atomic-request would gain the same coverage.
- dispatch-in-atomic-request's `dar_on`/`dar_true` also accept a parameter whose default is True feeding the
  ATOMIC_REQUESTS entry (addons-server's `get_db_config(..., atomic_requests=True)`), which the library's
  `dj_atomic_requests()` does not.
- python/calls, `call_open_name(x)` and an entry-point shape (decorated, takes `request`, a method of a non-model
  class, or handed to a call outside the run): select-for-update-outside-atomic's `sfu_open_name`, `sfu_passed`,
  `sfu_framework_shape`, for any rule that asks whether a framework calls a def.

## Engine issues found

[ENGINE-BUGS.md](ENGINE-BUGS.md) has each bug with its reproduction, code location, evidence, workaround and suggested
fix: the `Vocabulary::rows` deadlock, the annotation-guest blowup on huge string modules, module names taken from the
repository root, `sym_of` dropping relative imports of two or more dots, the machine-wide tape cache, very large
repositories not finishing, `sym_of` missing attributes of a same-module class (`Base.Meta`), calls through outside
decorators (`@contextmanager`) left without a target, and the call-graph rules' run time.

Smaller, not bugs: a rule-local relation that joins two unrelated sides is evaluated as a cross product with no warning
(a `text(r, "self"), param(f, "self", 0), dj_model_method(_, f, _)` join never finished on netbox; binding through
`look(r, "self", f)` fixed it); a reachability relation over every block of the run (python/ast's `body_stmt` /
`body_len`) ran netbox to 24 GB and past 15 minutes, while the same flow seeded from the blocks a rule needs
(clean-field-returns-value's `cfr_block`) finishes in about a minute; `within_stop(n, a, stop)` does not treat `a` itself as a stop (as documented,
"strictly between", but easy to miss: a lambda passed as a default had to be excluded separately).
`docs/rule-syntax/site/check_rr.py` fails only on `lib/python/pytest-collect.rerail:342: unbound k, product, w`, a file
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
- django-migration-linter `CREATE_INDEX` (on `sqlmigrate` output, with a database configured): the same check as the
  dropped index-not-concurrent. Squawk lints raw SQL, not Django migration files.
- Ruff RET503 `implicit-return` (not in Ruff's default set) and pylint R1710 `inconsistent-return-statements`: a
  function that returns a value on some paths and falls off on others. Of clean-field-returns-value's 18 methods they
  can see the 8 that also return a value somewhere; the 10 that never return one (validate, raise, fall off) and the
  fact that Django stores the result are beyond them. Ruff RET502 flags a bare `return` only when the function also
  returns a value.
- Run-time checks Django and Celery already make, each only on the path that runs: TransactionManagementError for a
  `select_for_update()` evaluated in autocommit on PostgreSQL, MySQL and Oracle (TestCase's transaction hides it in
  tests; select-for-update-outside-atomic); SynchronousOnlyOperation for a blocking query in a coroutine
  (sync-orm-reached-from-async, async-request-user); Celery's EncodeError for a model instance under the JSON
  serializer, and nothing under pickle (task-arg-model-instance).
