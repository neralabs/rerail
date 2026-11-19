# Django practice rules

The 27 `django-practice-*` rules are Rerail's own; none is a port of a linter rule. Each is advice that the Django docs
give, or that Django maintainers give again and again in code review, and that no linter checks. A rule was kept when
at least two independent sources gave the advice, one of them the Django docs or a Django core developer or Fellow;
when public apps held real violations of it; and when no existing lint rule makes the same check. The tools checked
were Ruff (its DJ, DTZ, INT, S and ASYNC rules among them), flake8-django, pylint-django, Bandit, django-upgrade,
django-migration-linter and Django's system checks; the Semgrep registry and orm-lens were read too, and their partial
overlaps are listed in [NOTES-django.md](NOTES-django.md).

Each rule reads the repository's Python files through the shared library
[`python/django.rerail`](../../lib/python/django.rerail). Most rules read one file at a time; many also follow names into the
repository's other files. The model rules follow a model's bases, so fields, Meta options and save()/delete()
overrides declared on a base in another module count (fk-id-not-relation, select-related-in-loop, model-init-override,
save-ignores-update-fields, bulk-write-skips-hooks, read-modify-write, get-or-create-needs-unique, which also reads the
model's managers); bulk-write-skips-hooks also finds a model's pre_save/post_save receivers in any file.
function-decorator-on-method follows view bases; clean-field-returns-value and modelform-clean-calls-super follow
form bases; default-called-once follows the project's field classes derived from Django's; enqueue-on-commit and
task-arg-model-instance follow Celery tasks and task classes into the modules that define them; settings-module-import
finds the project's settings modules among all its files; dispatch-in-atomic-request and
select-for-update-outside-atomic read the ATOMIC_REQUESTS setting wherever the project sets it. Four rules follow calls across
files through the call graph of `python/calls`: get-handler-writes (from a GET handler to the writes it reaches),
dispatch-in-atomic-request (from a view to the dispatches it reaches), select-for-update-outside-atomic (from a lock up
to the callers that run it outside a transaction) and sync-orm-reached-from-async (from an `async def` to the queries
it reaches). These four are the slowest: on the largest apps a run takes hours. Each rule's details below list what it
misses.

Each rule is proven three ways: by its cases in `cases/<id>/`, where a line ending in `# caught` must be reported and no
other line may be, run by the rule file's own test (`.test cases exactly`, which reads the case directory as one
repository and expects each `# caught` line; `rerail rules test <file>.rerail`); by a run on 31 open-source Django apps
at pinned commits ([the corpus](#the-corpus)), whose matches were read and judged real or not (every match where a rule
had at most 80, otherwise a spread sample of at least 80 with every matching repository represented); and by an
independent review of each rule that tried counterexamples against it, which were added to its cases.

| Rule | Mode | Catches | Matches (repos) on 31 apps | Precision of read matches |
|---|---|---|---|---|
| `django-practice-exists-for-existence` | Steer | an inline queryset counted, loaded or truth-tested only to learn whether it has rows | 42 (17) | 100% (42 of 42) |
| `django-practice-count-not-len` | Steer | `len()` of an inline queryset whose rows are thrown away | 2 (2) | 100% (2 of 2) |
| `django-practice-fk-id-not-relation` | Steer | `<obj>.<fk>.id` or `.pk` where the row already holds `<fk>_id` | 149 (25) | 99.3% (148 of 149) |
| `django-practice-select-related-in-loop` | Steer | a loop over an inline queryset that reads a relation of every row | 46 (17) | 100% (46 of 46) |
| `django-practice-bulk-not-loop` | Log | one create, save or get query per loop iteration | 1,533 (31) | 98.9% (89 of 90 read, a spread sample) |
| `django-practice-bulk-write-skips-hooks` | Steer | a queryset bulk write on a model whose save()/delete() override or pre/post_save receiver it skips | 340 (20) | 100% (89 of 89 read, a spread sample) |
| `django-practice-read-modify-write` | Steer | a numeric field read into Python, changed and saved back instead of updated with `F()` | 18 (8) | 100% (18 of 18) |
| `django-practice-get-or-create-needs-unique` | Steer | `get_or_create()`/`update_or_create()` whose lookup no unique field or constraint covers | 92 (19) | 100% (82 of 82 read, a spread sample) |
| `django-practice-atomic-around-caught-db-error` | Steer | a database error caught inside atomic, around queries with no savepoint of their own | 5 (4) | 100% (5 of 5) |
| `django-practice-select-for-update-outside-atomic` | Log | a `select_for_update()` in a function that a caller chain or a framework runs with no transaction open | 2 (2) | 100% (2 of 2) |
| `django-practice-enqueue-on-commit` | Steer | a task enqueued or a mail sent inside `transaction.atomic` | 30 (6) | 100% (30 of 30) |
| `django-practice-dispatch-in-atomic-request` | Steer | a task enqueued or a mail sent from a view, or code it calls, while ATOMIC_REQUESTS wraps the request in a transaction | 22 (2) | 100% (22 of 22) |
| `django-practice-task-arg-model-instance` | Steer | a model instance passed as an argument to a Celery task | 5 (2) | 100% (5 of 5) |
| `django-practice-default-called-once` | Steer | a model field default that calls uuid4, random, now and the like once, at import | 1 (1) | 100% (1 of 1) |
| `django-practice-model-init-override` | Steer | a model `__init__` that reads a field or changes the calling signature | 12 (10) | 100% (12 of 12) |
| `django-practice-save-ignores-update-fields` | Steer | a model `save()` that sets a field and forwards `**kwargs` without handling `update_fields` | 155 (20) | 100% (155 of 155) |
| `django-practice-lazy-translation-at-import` | Steer | non-lazy `gettext` called when the module is imported | 1,699 (14) | 100% (82 of 82 read, a spread sample) |
| `django-practice-clean-field-returns-value` | Steer | a form `clean_<field>()` that can end without returning the cleaned value | 21 (6) | 100% (21 of 21) |
| `django-practice-modelform-clean-calls-super` | Steer | a `clean()` override on a ModelForm or model formset that never calls the parent's `clean()` | 30 (9) | 100% (30 of 30) |
| `django-practice-function-decorator-on-method` | Steer | a view decorator put on a method without `method_decorator` | 8 (2) | 100% (8 of 8) |
| `django-practice-get-handler-writes` | Log | a GET/HEAD handler that writes rows, itself or through what it calls | 122 (12) | 72% (61 of 85 read, a spread sample) |
| `django-practice-host-header-without-get-host` | Log | the Host or X-Forwarded-Host header read directly instead of `request.get_host()` | 12 (5) | 83% (10 of 12) |
| `django-practice-middleware-ignores-streaming` | Steer | middleware reading `response.content` without handling streaming responses | 1 (1) | 100% (1 of 1) |
| `django-practice-settings-runtime-assignment` | Steer | a runtime write to `django.conf.settings` | 92 (11) | 100% (92 of 92) |
| `django-practice-settings-module-import` | Steer | an import of the project's settings module instead of `django.conf.settings` | 49 (11) | 100% (49 of 49) |
| `django-practice-async-request-user` | Steer | `request.user` read inside `async def` instead of `await request.auser()` | 45 (1) | 100% (45 of 45) |
| `django-practice-sync-orm-reached-from-async` | Log | a blocking ORM query in an `async def`, written there or reached through the sync functions it calls | 12 (1) | 100% (12 of 12) |

Matches are the engine's findings; a line that holds two matched shapes counts once per shape. In distinct lines:
lazy-translation-at-import 1,671, fk-id-not-relation 147, settings-module-import 48, select-related-in-loop 43,
exists-for-existence 41, get-handler-writes 121 and task-arg-model-instance 4; every other rule's matches are on
distinct lines.

A match is real when the file holds the pattern, even if code elsewhere (a caller, a middleware, a settings wrapper,
a task class) makes it harmless there; a repository that disagrees turns the rule off. A rule steers where nearly
every read match was real in that sense. It logs where many matches do not hold the pattern, or where the pattern
holds but the advised rewrite is often unsafe in ways one file cannot show; each Log rule's details give the reason.

## Rule details

What each rule matches beyond its one-line description: exact matching, what it leaves alone, gaps and sources.

### `django-practice-exists-for-existence`

Rerail's own rule, not a port. It matches, outside test files, a queryset written inline as a chain from a manager
(`Book.objects…`, `_default_manager`, `_base_manager`, through QuerySet methods that return a queryset), not a `raw()`
one, in three shapes: `<qs>.count()` with no arguments, compared with 0 or 1 as an emptiness test (`> 0`, `!= 0`,
`>= 1`, `== 0`, `< 1`, `<= 0`, and the mirrored `0 < …`, `1 > …`, …) or read for its truth; `len(<qs>)` in the same
positions; and `<qs>` itself read for its truth (`if Book.objects.filter(…):`, a `while` or `assert` test, `not …`,
`bool(…)`, a conditional expression's or comprehension's `if`, an `and`/`or` operand that is itself a test). The
queryset must be Django's: in a file that imports Django or Django REST framework, or starting at a model class of the
run (in a src layout too: `src/inventory/models.py` is the module `inventory.models`). The message names which of the
three shapes it is.

It leaves alone: a real count (`> 5`, `== 1`); chained comparisons (`0 < qs.count() < 10`, a range); querysets in
variables (the docs' "Don't overuse exists()" case: `if books: for b in books`); `if Book.objects:` (a manager is
always true, a different bug); `raw()` querysets (no `exists()`); `qs or other` used as a value; `count(x)` with
arguments; test files; a mongoengine `Document.objects.filter(…)`, the Document a class of the run, in a file that
does not import Django.

**Gaps:** querysets held in a name or returned by a method: a wider probe (any zero-argument `.count()` in a file
importing django) found 130 more matches, e.g. DefectDojo/django-DefectDojo `dojo/jira/views.py:286`
`existing_notes.count() == 0`, inventree/InvenTree `src/backend/InvenTree/order/models.py:1112`
`self.pending_line_items().count() == 0`; some are reused afterwards, so they are not taken. Related managers
(`item.sales_order_allocations.count() > 0`, InvenTree `stock/serializers.py:1562`;
`to_wishlist.lines.filter(…).count() > 0`, django-oscar `customer/wishlists/views.py:427`): about 15 of 40 sampled
were this shape and all were real, but a chain from a manager does not reach them. `first() is None`, and counts
stored in a variable then tested (`server_db_count = len(…)`; mathesar-foundation/mathesar
`mathesar/rpc/databases/configured.py:172`, which count-not-len reports instead). Harmless in context: nine matches
are data migrations on historical models that run once (e.g. inventree/InvenTree
`part/migrations/0144_auto_20251203_1045.py:45`, umap-project/umap `umap/migrations/0003_add_tilelayer.py:9`,
ansible/awx `awx/main/migrations/_galaxy.py:18`). It could match wrongly on a custom manager named `objects` whose
`filter()` returns no queryset; none appeared. A mongoengine `Document` imported from a module the engine cannot
resolve is still taken as Django's. orm-lens DOL001/DOL002 (a VS Code extension, regex) cover the `count()`
comparisons; nothing covers the `len()` or truth shapes.

**Sources:**
- Django docs, [Database access optimization: Use QuerySet.exists()](https://docs.djangoproject.com/en/5.2/topics/db/optimization/#use-queryset-exists): "Use `QuerySet.exists()` …if you only want to find out if at least one result exists, rather than `if queryset`."
- Django docs, [When QuerySets are evaluated](https://docs.djangoproject.com/en/5.2/ref/models/querysets/#when-querysets-are-evaluated): "Note: If you only want to determine if at least one result exists (and don't need the actual objects), it's more efficient to use `exists()`."
- NyanKiyoshi (Saleor maintainer), [saleor#19476](https://github.com/saleor/saleor/pull/19476#discussion_r3578344387), on `assert new_checkout.lines.count() == 0`: "Slightly faster (`COUNT(1)` is more expensive):"
- IKarbowiak (Saleor maintainer), [saleor#11129](https://github.com/saleor/saleor/pull/11129#discussion_r1015280098), on `len(TransactionEvent.objects.filter(...)) == 0`: "You could use `exists` here:"
- jacobtylerwalls (Django Fellow), [pylint-django#382](https://github.com/pylint-dev/pylint-django/issues/382): "it warns not to overuse exists(). So this check would really only be useful if it could tell with confidence that the queryset is not evaluated afterward." (Why only inline chains.)

### `django-practice-count-not-len`

Rerail's own rule, not a port. It matches `len(<qs>)` with one argument, `len` the builtin (not shadowed), and `<qs>`
an inline chain from a manager, not `raw()`, outside test files, unless the `len()` is an emptiness test: compared
with 0 or 1 as in `django-practice-exists-for-existence`, or read for its truth. Those are exists-for-existence's (fix:
`exists()`), so each shape is reported by exactly one rule.

It leaves alone: `len(<name>)` (the rows may be iterated after, where `len()` fills the cache usefully); `len()` of
anything not a manager chain; `raw()` querysets; a shadowed `len`; test files.

**Gaps:** related managers (`len(obj.rel.all())`) and querysets built in helpers. Both corpus matches are real:
mathesar-foundation/mathesar `mathesar/rpc/databases/configured.py:172` (stored, then tested `== 0`; `exists()`
would be better still) and nautobot/nautobot
`nautobot/core/management/commands/generate_performance_test_endpoints.py:88` (`model_class.objects.all()`, a
variable model).

**Sources:**
- Django docs, [Use QuerySet.count()](https://docs.djangoproject.com/en/5.2/topics/db/optimization/#use-queryset-count): "Use `QuerySet.count()` …if you only want the count, rather than doing `len(queryset)`."
- Django docs, [count()](https://docs.djangoproject.com/en/5.2/ref/models/querysets/#count): "A `count()` call performs a `SELECT COUNT(*)` behind the scenes, so you should always use `count()` rather than loading all of the record into Python objects and calling `len()` on the result (unless you need to load the objects into memory anyway, in which case `len()` will be faster)."
- raphaelm (pretix maintainer), [pretix#2003](https://github.com/pretix/pretix/pull/2003#discussion_r626892952), on `return len(ScheduledMail.objects.filter(rule=self))`: "1. Prefer .count() over len(): https://stackoverflow.com/a/14327086"

### `django-practice-fk-id-not-relation`

Rerail's own rule, not a port. It matches `<obj>.<fk>.id` and `<obj>.<fk>.pk`, read (not assigned), outside test
files, where `<fk>` is a ForeignKey, OneToOneField or ParentalKey without `to_field=` that `<obj>`'s model declares,
itself or through a base class of the run. The model is known three ways: `self` in a method of the model (92
matches); a name whose latest assignment in its scope is `M.objects….get|first|last|latest|earliest|create(…)` with M
a model class of the run, when nothing else in that scope binds the name (a `for` or comprehension target,
`with … as`, a tuple target, `except … as`, a walrus or `+=`; parameters are allowed, as they bind before the
assignment); and the target of `for x in M.objects…:` inside the loop body, when the body binds the name in no other
way (57 matches for these two).

It leaves alone: a key read only for its truth or compared with None (`if not self.owner.id:`, `bool(self.author.pk)`,
`self.tax_rule.pk is None`): these ask "is the related object saved yet?", which `<fk>_id` does not answer for an
unsaved object just assigned (healthchecks/healthchecks `hc/api/models.py:1422`, django-oscar/django-oscar
`catalogue/reviews/abstract_models.py:229`, pretix/pretix `base/models/orders.py:2482`); `x.<fk>.pk = None` (a
store); `to_field=` relations; other attributes of the related object; parameters and any object whose model is not
known; non-model classes reusing a field name.

**Gaps:** one wrong match: readthedocs/readthedocs.org `readthedocs/organizations/models.py:239`
`self.stripe_customer.id`: the target, dj-stripe's `Customer`, keeps the Stripe id in `id` and its primary key in
`djstripe_id`, so `stripe_customer_id` is not the same value. The rule assumes `.id` is the target's primary key; for
a target outside the run it cannot check that. The broader form (any `x.<fk>.id|pk` with `<fk>` a FK name of some
model in the run, x not `self`/`request`, fk not `user`) gave 867 more matches in 28 repos; a sample of 30 had about
22 real; the wrong ones are non-model wrappers sharing the attribute name (saleor/saleor
`saleor/checkout/webhooks/calculate_taxes.py:46` `line_info.line.pk`; zulip/zulip `corporate/views/upgrade.py:164`
`billing_session.remote_server.id`; rafalp/Misago `misago/posting/views/start.py:204` `state.thread.id`;
WeblateOrg/weblate `weblate/trans/tasks.py:1711` `project_language.language.id`), so reads in views, serializers and
helpers through parameters stay a gap. Harmless in context: some matches also read other fields of the same related
object, so the row is loaded anyway and `_id` saves nothing there (archesproject/arches
`arches/app/models/models.py:1580`; pretix/pretix `src/pretix/base/models/waitinglist.py:215`); and `select_related`
may have cached it. `_id` is still the right read.

**Sources:**
- Django docs, [Use foreign key values directly](https://docs.djangoproject.com/en/5.2/topics/db/optimization/#use-foreign-key-values-directly): "If you only need a foreign key value, use the foreign key value that is already on the object you've got, rather than getting the whole related object and taking its primary key. i.e. do: `entry.blog_id` instead of: `entry.blog.id`"
- raphaelm (pretix maintainer), [pretix#4735](https://github.com/pretix/pretix/pull/4735#discussion_r1948869482), on `order.customer.id == self.request.customer.id`: "More than a style change: saves us a database query here :)"
- raphaelm, [pretix#2965](https://github.com/pretix/pretix/pull/2965#discussion_r1044466434): "property_id does not require the property object to be loaded from the database. We can save a SQL query here by using property_id and removing the prefetch related :+1:"
- maarcingebala (Saleor), [saleor#11239](https://github.com/saleor/saleor/pull/11239#discussion_r1044302502): "Doesn't `webhook.app.id` make a database query? Could be `webhook.app_id` instead I guess."
- leeandher (Sentry maintainer), [sentry#120908](https://github.com/getsentry/sentry/pull/120908#discussion_r3715820538): "`.organization` does a join via Django. Either way we only use the ID, so switching it with `project.organization_id` avoids this whole thing"

### `django-practice-select-related-in-loop`

Rerail's own rule, not a port. It matches a `for x in <qs>:` statement or a comprehension's `for x in <qs>`, where
`<qs>` is an inline chain from a manager without `values()`, `values_list()` or `raw()`, or `<qs>.iterator(…)` (`<qs>`
may be the manager there), reported at the queryset, when the code run per row (outside nested defs, lambdas and
classes) does one of:
- reads a related manager as `prefetch_related()` would cache it: `x.<rel>.all()` with nothing chained after it
  (iterated, listed), `x.<rel>.count()` or `x.<rel>.exists()`, and the chain has no `prefetch_related` (32 matches).
  When the model is a class of the run, `<rel>` must be one of its many-to-many fields or a name it (and its bases in
  the run) does not define: a field of another kind, or a method or `@property`/`@cached_property` returning a
  queryset, is not prefetchable by that name (`prefetch_related('active_tags')` raises). Over `.iterator()` without
  `chunk_size`, which prefetches nothing (Django 5.0 raises with prefetch_related), this shape is left alone.
- reads `x.<fk>.<attr>` (not `.id`/`.pk`) with `<fk>` a ForeignKey/OneToOneField of the iterated model, which must be
  a model class of the run (`Book.objects…`, declared directly or through a base), and the chain has neither
  `select_related` nor `prefetch_related` (14 matches).

The message names the relation and the fix (`.prefetch_related('<rel>')` / `.select_related('<fk>')`), and says to
leave it when the loop stops after a row or two. Loops over `.iterator()` are matched too: ansible/awx
`awx/main/management/commands/regenerate_secret_key.py:62` reads `credential.credential_type` over
`Credential.objects.iterator()`; rafalp/Misago `misago/users/management/commands/synchronizeusers.py:32` reads
`user.followed_by.count()` and `user.follows.count()` over `User.objects.iterator(chunk_size=50)`.

It leaves alone: `x.<rel>.filter(…)`, `.exclude()`, `.order_by()`, `.all().update()`, `.all().values_list()`: these
query on every call even after a prefetch, so prefetch_related is not their fix (e.g. LibrePhotos/librephotos
`apps/backend/api/albums/auto_albums.py:280` `album_thing.photos.filter(hidden=False).count()`, pretix/pretix
`src/pretix/base/migrations/0069_invoice_prefix.py:11` `e.invoices.all().update(…)`); `x.<fk>_id`, `x.<fk>.pk`,
`x.<fk>.id` (fk-id-not-relation's); querysets in variables (they may be prefetched where built); unknown models for
the FK shape; test files.

**Gaps:** querysets from `get_queryset()`, names or helpers; template and serializer loops; the FK shape when the
model is not a class of the run; any `select_related(…)` in the chain silences the FK shape even if it names another
relation. 18 of the 46 matches are data migrations, where the loop runs once (e.g. pretix/pretix
`src/pretix/base/migrations/0084_questionoption_position.py:10`, saleor/saleor
`saleor/product/migrations/0074_auto_20181010_0730.py:27`); they are real, and Saleor's reviewers ask for bulk work
there too. When the model is not a class of the run (a variable model, a model of another package), a property
returning a queryset cannot be told from a related manager and is reported as one, as is any attribute with a
zero-argument `.exists()`/`.count()`; none appeared. orm-lens DOL007 and `orm-lens nplusone` detect N+1 queries too.

**Sources:**
- Django docs, [Retrieve everything at once if you know you will need it](https://docs.djangoproject.com/en/5.2/topics/db/optimization/#retrieve-everything-at-once-if-you-know-you-will-need-it): "This is particularly important if you have a query that is executed in a loop, and could therefore end up doing many database queries, when only one was needed." Then: "Use `QuerySet.select_related()` and `prefetch_related()`".
- laymonage (Wagtail core), [wagtail#13155](https://github.com/wagtail/wagtail/pull/13155#discussion_r2140588959): "Add `select_related` for `site`? … Otherwise this would add one query for every site and setting that the group has permission for."
- ababic (Wagtail), [wagtail#10850](https://github.com/wagtail/wagtail/pull/10850#discussion_r1399730106), on `[(site.root_page.path, str(site)) for site in Site.objects.all()]`: "Little optimisation opportunity here..."
- sahil839 (Zulip), [zulip#38770](https://github.com/zulip/zulip/pull/38770#discussion_r3039004820), on `for field_value in CustomProfileFieldValue.objects.filter(field=field):`: "This will not be efficient if large number of users have set the field value as this does DB query in a loop."

### `django-practice-bulk-not-loop`

Rerail's own rule, not a port. In the body of a `for` statement or a comprehension (not in a def or lambda nested in
it), it matches three shapes. (1) `<qs>.create(...)` on a manager chain (`X.objects.create`,
`X.objects.using(db).create`) whose result is dropped (a statement of its own) or only passed to `.append(...)` →
`bulk_create()`. (2) `<v>.save(...)` where `<v>` is the loop's own target (alone or in a tuple target) and the call has
no positional argument (Model.save() takes keywords only) → `bulk_update()` or `QuerySet.update()`. (3)
`X.objects.get(...)`, or `.first()`/`.last()` at the end of a manager chain, whose arguments or chain read the loop
variable (keyword names, attribute names, a `.using(alias)` argument and a `**{field: value}` key are not reads; a
`.first()`/`.last()` after `order_by(...)` is left out) → `in_bulk()` or one `filter(..__in=...)` query. The message
names the shape and says that bulk methods skip save() and the pre_save/post_save signals.

It leaves alone: a created row used at once (`book = X.objects.create(...)`); `.save()` on another object or with a
positional argument (Misago's `form.save(state)`); a lookup that does not depend on the loop; a manager whose model is
the loop variable or a name the loop body assigns (`for model in models: model.objects.get(...)`, awx's
`object_cls = apps.get_model(...)`), because each iteration hits another table; chains with `select_for_update()`
(one row lock per iteration is the point: label-studio `next_task.py`, InvenTree `build/tasks.py`, Weblate
`accounts/tasks.py`); a loop over databases (`for alias in connections: X.objects.using(alias).get(pk=pk)`) or over
lookup columns (`filter(**{field: value})`), which no bulk query joins; `filter(k=v).order_by(...).first()` per key,
the newest row per group, which needs a Subquery or window query rather than in_bulk (8 corpus lines, e.g. zulip
`zerver/migrations/0509_fix_emoji_metadata.py`, kiwitcms `tcms/testcases/migrations/0006_merge_text_field_into_testcase_model.py`);
a `.save()` whose loop moves a number field of the target from its old value (`v.hits -= 1`, `v.hits = v.hits + n`,
with a number literal step or a field some model of the run declares numeric): bulk_update would write the
Python-computed values and keep the lost update, and `django-practice-read-modify-write` gives the
`update(hits=F("hits") - 1)` fix (1 corpus line, openedx `lms/djangoapps/lti_provider/signals.py`; a string built on
its old value, `v.name = v.name + suffix`, stays reported); test code (including `test.py`, `testing/` and `unittests/`).

**Gaps:** the commonest bulk_create shape, `book = Book(...); book.save()` (only the loop's own target's save() is
read); loops in helpers, `while` loops, `.update()`/`.delete()` per row, `get_or_create` per row, related-manager
`.create`, and loops written through `map`. It can match wrongly on a custom manager method named `create` with
another signature: the one not-real match read, mozilla/addons-server `src/olympia/addons/serializers.py`
(`ActivityLog.objects.create(amo.LOG.MAX_APPVERSION_UPDATED, …)`). Harmless or deliberate in context (the pattern
holds, but the rewrite would change behaviour): allegro/ralph `src/ralph/networks/models/networks.py`
(`network.save(update_subnetworks_parent=False)`, a custom save), netbox `netbox/dcim/models/racks.py`
(`rack.snapshot(); rack.save()` for the change log), cvat `cvat/apps/dataset_manager/project.py` (commented "We need
label_id here, so we can't use bulk_create here"), umap `umap/management/commands/migrate_to_S3.py`
(`datalayer.save(force_update=True)` writes the file), DefectDojo `dojo/api_v2/views.py` (per-item `create` inside
`try`). 628 of the 1,533 matches are in migrations. The precision is from a spread sample of 90 matches, 2-3 per
repository, every repository represented.

**Sources:**
- Django docs, [Database access optimization: Create in bulk](https://docs.djangoproject.com/en/5.2/topics/db/optimization/#create-in-bulk): "When creating objects, where possible, use the `bulk_create()` method to reduce the number of SQL queries." Same page, Update in bulk: "When updating objects, where possible, use the `bulk_update()` method to reduce the number of SQL queries."
- Django docs, [bulk_create()](https://docs.djangoproject.com/en/5.2/ref/models/querysets/#bulk-create): "The model's `save()` method will not be called, and the `pre_save` and `post_save` signals will not be sent."
- IKarbowiak (Saleor maintainer), [saleor#8851](https://github.com/saleor/saleor/pull/8851#discussion_r787514348): "I'm not sure about this save in the for loop. For big databases, it's gonna take forever. IMO we should prepare the list of stock objects to update and then use `bulk_update` with `batch_size` defined."
- IKarbowiak, [saleor#9526](https://github.com/saleor/saleor/pull/9526#discussion_r923079835): "We could use `bulk_update` instead of saving every line separately."
- jeremystretch (NetBox lead), [netbox#20611](https://github.com/netbox-community/netbox/pull/20611#discussion_r2469425637): "have you considered setting the updated value on each VLANRange instance in the queryset, and then calling `bulk_update()` (with perhaps `batch=100`) to update the database more efficiently?"
- laymonage (Wagtail core), [wagtail#12564](https://github.com/wagtail/wagtail/pull/12564#discussion_r1838085969): "Can we use `bulk_create` here to speed it up".
- felixxm (Django Fellow), [django#18105](https://github.com/django/django/pull/18105#discussion_r1580866309), the caveat: "We have to remember that `bulk_create()` doesn't call `save()` and will not emit signals, so this change can be backward incompatible for some projects. … Not sure it's worth changing."

Why Log: the pattern holds in nearly every match, but the advised rewrite is safe only where save() overrides, signals
and per-row side effects do not matter, which one file cannot tell. Of the 90 read, about one in eight (11) sit on a
custom save, signal-driven change logging, a per-row `try`, or code that needs each new id at once, where a bulk call
would change behaviour; felixxm's caveat says the same. Many loops are also small and bounded (a handful of settings
rows in a migration). The rule records where per-row queries are; a Steer variant could be limited to migrations
using `apps.get_model` (historical models have no custom save()), which hold 628 of the 1,533 matches.

### `django-practice-bulk-write-skips-hooks`

Rerail's own rule, not a port. It matches a queryset bulk write, `update()`, `bulk_create()` or `bulk_update()`, on a
chain that starts at `M.objects` (or `_default_manager`, `_base_manager`), when M or a base class of M in the run
(abstract bases and mixins in other modules included) overrides `save()` with more than a bare base-class call, or a
`pre_save`/`post_save` receiver for `sender=M` is connected in any non-test file of the run
(`@receiver(sig, sender=M)`, sig maybe a list or tuple; `sig.connect(fn, sender=M)`); and a queryset `delete()` when M
or a base overrides `delete()` (a bulk delete still sends pre_delete/post_delete). Signals and senders resolve through
imports to `django.db.models.signals`. The message names every hook the write skips, with its path and line, and says
to do their work at the call site or save the objects one by one, with `F()` for a counter.

It leaves alone: an `update()` whose every value is `F("x") ± n` on its own field x, an atomic counter bump, the write
Django documents for counters and `django-practice-read-modify-write` recommends (pretix `orders.py`, zulip
`stream_subscription.py`); an override whose body, docstring aside, is only `super().save(…)` or the old style
`models.Model.save(self, …)`; a write inside M's own save()/delete() or inside one of M's receivers, decorated or
named by `.connect(fn, …)` (they write in bulk on purpose, to avoid recursion); migrations; test code.

**Gaps:** querysets not rooted at a manager name (related managers, custom manager names, `self.get_queryset()`, a
queryset held in a variable); string senders (`sender="app.Model"`); hooks in base classes outside the run; a counter
bump wrapped in a function (pretix `Greatest(0, F('redeemed') - usage_count)` is reported); a counter bump whose
model's hooks do depend on that counter is left alone. Many matches are deliberate (comments such as "QuerySet.update()
bypasses Django signals — intentional"); they hold the pattern. Examples: sissbruecker/linkding
`bookmarks/services/bookmarks.py` (`Bookmark…update()` skips Bookmark.save(), which recomputes `url_normalized`),
pretix/pretix `src/pretix/base/models/event.py` (`Checkin…delete()` skips Checkin.delete(), which touches the order and
the check-in list), zulip/zulip `zerver/lib/alert_words.py` (`AlertWord…bulk_create()` skips the post_save receiver
`flush_alert_word`), ansible/awx `awx/main/scheduler/task_manager.py` (`bulk_update()` skips
CreatedModifiedModel.save(), which maintains `modified`). By method: update 177, bulk_create 109, bulk_update 40,
delete 14. The precision is from a spread sample of 89 matches, 4-5 per repository, every repository represented.

**Sources:**
- Django docs, [Overriding predefined model methods](https://docs.djangoproject.com/en/5.2/topics/db/models/#overriding-predefined-model-methods): "Note that the delete() method for an object is not necessarily called when deleting objects in bulk using a QuerySet or as a result of a cascading delete. To ensure customized delete logic gets executed, you can use pre_delete and/or post_delete signals. Unfortunately, there isn’t a workaround when creating or updating objects in bulk, since none of save(), pre_save, and post_save are called."
- Django docs, [QuerySet API reference](https://docs.djangoproject.com/en/5.2/ref/models/querysets/): bulk_create: "The model’s save() method will not be called, and the pre_save and post_save signals will not be sent."; update(): "Finally, realize that update() does an update at the SQL level and, thus, does not call any save() methods on your models, nor does it emit the pre_save or post_save signals (which are a consequence of calling Model.save())."; delete(): "The delete() method does a bulk delete and does not call any delete() methods on your models. It does, however, emit the pre_delete and post_delete signals for all deleted objects (including cascaded deletions)."
- djangoproject.in, [Django bulk_create and bulk_update: Performance and the …](https://djangoproject.in/blog/django-bulk-operations): "But bulk_create bypasses Django's save machinery entirely: no pre_save , no post_save , no auto_now update for existing rows. If your application depends on those signals for correctness — not just performance — bulk_create introduces silent data inconsistency."
- Stack Overflow, [How can i use signals in django bulk create](https://stackoverflow.com/a/30632974), accepted answer: "As mentioned bulk_create does not trigger these signals"

### `django-practice-read-modify-write`

Rerail's own rule, not a port. In one function (outside test code), it matches `o.x += n`, `o.x -= n`,
`o.x = o.x ± n` or `o.x = n + o.x`, followed later in the same function (not a nested def) by `o.save(...)`, where o
is a plain name, x is a numeric field (IntegerField and its small, big and positive variants, DecimalField,
FloatField, DurationField) that o's model declares in its body or in a base class of the run, and o's model is
known: `self` in a method of model M; or o bound in the function by `M.objects….get()/first()/last()/latest()/
earliest()`, `o, created = M.objects….get_or_create(...)` (or update_or_create), `get_object_or_404(M or
M.objects…, ...)`, or `for o in M.objects…:`, M resolved across files. The message offers both F() forms:
`o.x = F("x") + n` before save(), which still runs save() and its signals, and
`M.objects.filter(pk=o.pk).update(x=F("x") + n)`, which skips them (`django-practice-bulk-write-skips-hooks` leaves
such a counter update alone); or a row lock with select_for_update() inside transaction.atomic().

It leaves alone: a function that calls `select_for_update()` anywhere; `o.x = F("x") + n`; a new instance
(`o = M(...)`), which is inserted, not updated; a non-numeric field (`a.title = a.title + "!"`); a change with no
later save() of the same name; test code.

**Gaps:** o as an attribute chain (`self.object.hits += 1` in a DetailView); a parameter whose model only callers
know; a save() in a helper; a row the caller locks: a model method is reported even when every caller holds
select_for_update(), since the method itself holds the pattern (14 of the 18 matches are model methods). Examples:
django-oscar/django-oscar `src/oscar/apps/voucher/abstract_models.py` (`self.num_orders += 1; self.save()`),
healthchecks/healthchecks `hc/api/models.py` (token bucket `obj.tokens -= …`, under the file's own comment "Race
condition: two concurrent authorize calls can overwrite each other's changes"), allegro/ralph
`src/ralph/accessories/models.py` (after get_or_create), openedx/edx-platform
`common/djangoapps/student/models/user.py` (login failure counter), inventree/InvenTree
`src/backend/InvenTree/stock/models.py` (stock quantity). pretix and saleor, which lock with select_for_update(), have
no match.

**Sources:**
- Django docs, [Avoiding race conditions using F()](https://docs.djangoproject.com/en/stable/ref/models/expressions/#avoiding-race-conditions-using-f): "If two Python threads execute the code in the first example above, one thread could retrieve, increment, and save a field's value after the other has retrieved it from the database. The value that the second thread saves will be based on the original value; the work of the first thread will be lost."
- freeCodeCamp, [How to Prevent Race Conditions in Django](https://www.freecodecamp.org/news/how-to-prevent-race-conditions-in-django): "Imagine two requests try to increase a post’s view count from 100. Both read 100 before either saves a change. Each adds one and writes 101. The counter should have reached 102, so one update disappears. This is a lost update, …".

### `django-practice-get-or-create-needs-unique`

Rerail's own rule, not a port. It matches `M.objects.get_or_create(...)` or `update_or_create(...)` (also after
`.all()`, `.using()` or `.select_for_update()`, and `cls.objects…` in a method of a model whose own Meta is not
abstract), outside test code, where M is a model class of the run, every lookup keyword (all but `defaults` and
`create_defaults`) names a field M declares in its body or a base class of the run (`x_id` read as relation x), and
the database keeps none of them unique: no lookup is `pk` or `id`, a OneToOneField, or a field with `unique=` or
`primary_key=` other than False, and no `unique_together` group (flat or nested) or `UniqueConstraint(fields=[...])`
without a condition lies within the lookup. M's Meta options are read as Django inherits them: M's own Meta; an
abstract base's when M declares no Meta or its Meta subclasses the base's (`class Meta(Base.Meta)`); and a concrete
base's (multi-table inheritance or a proxy: that table's constraints hold for M's rows too). The message names the
lookup and suggests the UniqueConstraint to add.

It leaves alone, as unknown: a `__` lookup or any keyword that is not a field the run declares (a field from a base
outside the run such as AbstractUser.username), `**kwargs`, a positional Q; a conditional UniqueConstraint within
the lookup (the row may meet the condition: Weblate's MachineryExclusion, Mathesar's Server); a Meta whose
unique_together or constraints are not literal lists, an expression UniqueConstraint (`Lower("name")`), `fields=`
not a literal; an unmanaged model (`managed = False`: its table and indexes are made outside Django); a manager the
model sets itself whose class, queryset class (`as_manager()`, `from_queryset()`) or the class its `get_queryset()`
returns overrides the method in the run (Read the Docs' BuildConfigManager turns `get_or_create(data=...)` into a
lookup on a unique `data_hash`). Related managers (`author.book_set.get_or_create(...)`) and querysets after
`filter()`/`exclude()` are not followed.

**Gaps:** related managers and custom manager names; uniqueness added only in a migration (RunSQL, an AddConstraint
not mirrored in Meta) or enforced only by `validate_unique()` (nautobot Device names) is not read, so such a call is
reported. Many matches are in management commands and data generators; they hold the pattern. Examples:
django/djangoproject.com `fundraising/views.py` (the model says `# TODO: Make this unique.`), DefectDojo/django-DefectDojo
`dojo/finding/ui/views.py` (catches MultipleObjectsReturned right after), LibrePhotos/librephotos
`apps/backend/api/models/cluster.py` (same), goauthentik/authentik `authentik/sources/telegram/api/source.py`,
openedx/edx-platform `common/djangoapps/student/models/user.py`. Borderline but real: taigaio/taiga-back Points looked
up by (project, value) under unique (project, name), whose IntegrityError path re-gets the row. The precision is from
a spread sample of 82 matches, every repository represented, each checked against its model's fields and Meta.

**Sources:**
- Django docs, [get_or_create()](https://docs.djangoproject.com/en/stable/ref/models/querysets/#get-or-create): "This method is atomic assuming that the database enforces uniqueness of the keyword arguments (see unique or unique_together). If the fields used in the keyword arguments do not have a uniqueness constraint, concurrent calls to this method may result in multiple rows with the same parameters being inserted."
- BaoDev.studio, [Django get_or_create: the race condition nobody expects](https://baodev.studio/blog/django-get-or-create-race-condition): "get_or_create is SELECT-then-INSERT with a gap, not atomic on its own. Under concurrency both callers miss the get and both insert. A DB UniqueConstraint on the lookup fields is the real fix; the method's error handling is built to rely on it." and "update_or_create carries the same risk."

### `django-practice-atomic-around-caught-db-error`

Rerail's own rule, not a port. It matches an `except` clause of a `try` that runs inside an atomic region (a
`with transaction.atomic():` body, `with atomic() as sid:`, or an `@transaction.atomic`/`@atomic(...)` function, with
no nested def, lambda or class between), when the clause catches `DatabaseError`, `IntegrityError`,
`OperationalError`, `DataError`, `ProgrammingError` or `InternalError` from `django.db` or `django.db.utils` (alone, in
a tuple, or `as e`), and the try body (not a nested def) holds a database call outside any `with atomic()` nested in
the try body (an `atomic(savepoint=False)` there does not count: it makes no savepoint): `.save()` on any receiver but
Django's cache, `.delete()` with no positional argument (not `cache.delete(key)`), `.update()` on a queryset chain
(`X.objects.filter(...)`, `obj.rel.filter(...)`; not a dict's update), `.create()`, `.bulk_create()`,
`.bulk_update()`, `.execute()`/`.executemany()` on any receiver, or a queryset method that evaluates (one that returns
no new queryset). The report sits on the `except` line. The message says the try body "runs a database query": it
also fires on read-only try bodies, which break the transaction the same way.

It leaves alone: a try body whose writes sit in their own `with transaction.atomic():` (the docs' fix); handlers whose
every path ends in `raise` (last statement `raise`, or an if/elif/else every branch of which ends in `raise`,
recursively), since the error still leaves the block and rolls it back; handlers that call
`transaction.savepoint_rollback(...)` or `transaction.set_rollback(True)`; `get_or_create`/`update_or_create` (they
wrap their insert in a savepoint), and queryset chain steps like `select_for_update()` (they run nothing);
`except ValueError` and other non-database errors; bare `except:`/`except Exception` (no source covers them, and they
usually log and re-raise); test files. On the corpus this leaves alone archesproject/arches
`arches/app/models/graph.py` (both branches raise) and `arches/app/views/api/workflow.py`
(`select_for_update().update_or_create(...)`).

**Gaps:** `ATOMIC_REQUESTS` and an atomic in a caller, helpers that write, and wrappers such as Saleor's
`traced_atomic_transaction()` (not treated as atomic here, unlike enqueue-on-commit).
Handlers that re-raise inside a nested `try`/`with`/`match` count as not re-raising. Matches (all real):
archesproject/arches `arches/management/commands/packages.py` (map layer `save()` in a loop, IntegrityError printed,
loop continues), goauthentik/authentik `authentik/core/api/users.py` (`service_account`: creates inside
`with atomic()`, IntegrityError turned into a 400 response, and the block then exits normally), pretix/pretix
`src/pretix/base/services/cart.py` (`@transaction.atomic(durable=True)` `_perform_operations`:
`op.position.save(force_update=True)` swallowed with `pass` and more queries follow, the exact bug of ticket 21134),
pretix/pretix `src/pretix/base/datasync/datasync.py` (`select_for_update(nowait=True).get()` and `save()`,
`DatabaseError` → `continue`; harmless in context because nothing else runs in that block), saleor/saleor
`saleor/graphql/product/utils.py` (`update_ordered_media`: per-media `save()` in a loop, DatabaseError logged, loop
continues).

**Sources:**
- Django docs, [Controlling transactions explicitly](https://docs.djangoproject.com/en/5.2/topics/db/transactions/#controlling-transactions-explicitly): "If you catch and handle exceptions inside an `atomic` block, you may hide from Django the fact that a problem has happened. This can result in unexpected behavior. This is mostly a concern for `DatabaseError` and its subclasses such as `IntegrityError`. After such an error, the transaction is broken and Django will perform a rollback at the end of the `atomic` block. … The correct way to catch database errors is around an `atomic` block as shown above. If necessary, add an extra `atomic` block for this purpose."
- RichardOfWard, [Django ticket #21134](https://code.djangoproject.com/ticket/21134): "Catching a database error inside transaction.atomic and then subsequently issuing more database commands before the end of the atomic block leads to a silent implicit rollback of the subsequent commands in Atomic's __exit__, even when there is no current exception."
- Anssi Kääriäinen (core), same ticket, comment 3, on `with transaction.atomic(): … try: r2.save(force_insert=True) except IntegrityError: r2.save(force_update=True)`: "The last assert fails."
- Aymeric Augustin (core, author of `atomic`), same ticket, comment 4, giving the inner `with transaction.atomic():` fix: "it's unrealistic to hope that everyone will do that without complaining".

### `django-practice-select-for-update-outside-atomic`

Rerail's own rule, not a port. It matches `<qs>.select_for_update(…)` in a def f outside test files, not inside an
atomic region, when f may run with no transaction open: f is an entry point, or a function that may run with no
transaction open calls f (through the cross-file call graph) at a call outside any atomic region. An entry point is a
def nothing outside tests calls, by a known target, by a call of unknown or unresolved target spelled with its name,
or by being handed as an argument to a function of the run; that has a shape a framework calls (decorated, taking
`request`, a method of a class that is neither a model nor a model's base, or handed to a call outside the run such
as `signal.connect(f)`); and that is not a view of a project with ATOMIC_REQUESTS (dict entry, subscript assignment
or keyword). An atomic region is a `with transaction.atomic()` body; a with-item after an atomic item
(`with transaction.atomic(), self.lock_rows(qs):`); the body of a with whose item calls a wrapper named like atomic
(HumanSignal/label-studio's `conditional_atomic(...)`) or a context manager of the run that yields inside a
transaction (WeblateOrg/weblate's `source_operation(component)`); an `@transaction.atomic` or
`@method_decorator(transaction.atomic)` def, a method of a class decorated
`@method_decorator(transaction.atomic, name="dispatch")` (or its own name), a def under a decorator of the run whose
wrapper calls it in a transaction (Weblate's `@source_operation_method`), or a def decorated with a context manager that
opens one (saleor/saleor's `@traced_atomic_transaction()`). A context manager the call graph cannot resolve (one
imported by a relative import of two or more dots) is matched by name against the run's context managers (saleor's
`transaction_with_commit_on_errors()`). The message says how f is reached
(`` `g` calls it outside a transaction at p:l `` or "no code in the project calls it").

It leaves alone: a lock switched on by a parameter (`if lock: qs = qs.select_for_update()`, zulip's `lock_message`)
or by a test on atomicity (`if self.is_atomic:`, `… if connection.in_atomic_block else …`); a function that exits
early on atomicity first (`if not connection.in_atomic_block: raise RuntimeError(...)`, pretix's `lock_objects`;
netbox's `cache_presave_scope_fields` returns); `qs = qs.select_for_update(…)` that the same function evaluates
inside its own atomic block (goauthentik/authentik's `delete_in_batches`); a def handed to a function of the run
(mozilla/pontoon's `self.run_import(import_uploaded_file, …)`); uncalled plain functions and model methods (helpers
whose callers hold the transaction); `@property`/`@cached_property` defs, which are read, not called (saleor's
`Reordering.ordered_node_map`); callers in test code (TestCase holds a transaction); test files.

**Gaps:** querysets returned by a helper and evaluated by the caller (the caller's call site is not judged); class
attribute querysets; transactions opened through `set_autocommit(False)`; the call graph is context-insensitive, so a
function reached both inside and outside a transaction is reported for the bare path. Both matches are real by that
path and harmless in context: inventree/InvenTree `src/backend/InvenTree/order/api.py:707` (`check_received`, reached
from `CommonBulkDeleteMixin._delete` through `validate_delete`; the one viewset using it wraps `bulk_delete` in
atomic) and WeblateOrg/weblate `weblate/trans/models/component.py:7664` (`update_enforced_checks`, called from
`Component.after_save`, which `save()` runs with no transaction of its own). On SQLite, `select_for_update()` is
ignored.

**Sources:**
- Django docs, [select_for_update()](https://docs.djangoproject.com/en/dev/ref/models/querysets/#select-for-update): "Evaluating a queryset with ``select_for_update()`` in autocommit mode on backends which support ``SELECT ... FOR UPDATE`` is a :exc:`~django.db.transaction.TransactionManagementError` error because the rows are not locked in that case. If allowed, this would facilitate data corruption and could easily be caused by calling code that expects to be run in a transaction outside of one."
- Django docs, same section: "Although ``select_for_update()`` normally fails in autocommit mode, since :class:`~django.test.TestCase` automatically wraps each test in a transaction, calling ``select_for_update()`` in a ``TestCase`` even outside an :func:`~django.db.transaction.atomic` block will (perhaps unexpectedly) pass without raising a ``TransactionManagementError``."
- Aymeric Augustin (core), [ticket #22491](https://code.djangoproject.com/ticket/22491): "Otherwise we'll have to document that code involving select_for_update should be tested in a TransactionTestCase."
- Adam Johnson, [select_for_update and transactions (bad dev UX in a feature)](https://www.mail-archive.com/django-developers%40googlegroups.com/msg56292.html): "FWIW I always recommend enabling ATOMIC_REQUESTS and ensuring transactions are used by default in other code paths such as background tasks."

Why Log: Django raises TransactionManagementError the first time an unguarded path runs on PostgreSQL, so real
projects reach these sites only from transactions the call graph cannot see per path; both corpus matches are of that
kind. Most `select_for_update()` calls written outside a lexical atomic block are locking helpers whose callers hold
the transaction (293 such calls in non-test code of the 32 cloned apps, 155 of them PostHog's); the rule reports only
the ones it can show to run bare, and the bare path it shows may not be one production takes.

### `django-practice-enqueue-on-commit`

Rerail's own rule, not a port. It matches, inside an atomic region (a `with transaction.atomic():` body or an
`@transaction.atomic` function, or the body of a `with` whose item calls a function whose name contains `atomic`, e.g.
Saleor's `traced_atomic_transaction()`; no nested def, lambda or class in between): `<x>.delay(...)`,
`<x>.apply_async(...)`, `<x>.send_task(...)`, `<x>.enqueue(...)`/`<x>.aenqueue(...)` (Django 6 tasks; also
`task.using(...).enqueue`), django-q `async_task(...)`, `django.core.mail.send_mail`/`send_mass_mail`/`mail_admins`/
`mail_managers`, and `.send()` on an `EmailMessage`/`EmailMultiAlternatives` built inline or bound to a name in the
same function. The message names the call. A task method needs a receiver that can be a task: a name bound by an
import (resolved or not) or an attribute of one, other than a plain undecorated function of the run, or a call on one
(`t.s(...)`, `t.using(...)`).

It leaves alone: calls inside a `transaction.on_commit(...)` argument (`partial(...)`, a lambda, `task.s(...).delay`);
`.delay_on_commit()`/`apply_async_on_commit()`; calls after the atomic block; nested defs; signal `.send(...)`; test
files; and tasks that already wait for the commit: a task declared `@app.task(base=X)` where X's own or inherited
`apply_async`/`delay` calls `transaction.on_commit` (pretix's `TransactionAwareTask`,
`TransactionAwareProfiledEventTask`), and every `.delay`/`.apply_async` in a run whose app is
`Celery(..., task_cls=X)` for such an X (mozilla/addons-server's `AMOTask`). Also left alone: calls in an except
handler whose every path re-raises (the block rolls back, so an on_commit callback would never run: a failure mail
belongs there); `r = t.apply_async(...)` / `r = t.delay(...)` whose result the function reads afterwards (Weblate
`weblate/trans/views/create.py`, which stores `task.id` for a progress page: on_commit returns nothing, so the call
cannot simply move); `.delay()` on a parameter or local (`client.delay(2.0)`). Cache writes and HTTP calls inside
atomic are not matched ([NOTES-django.md](NOTES-django.md)).

**Gaps:** `ATOMIC_REQUESTS` (`django-practice-dispatch-in-atomic-request` takes those views) and a caller's atomic
(the saleor#18906 source below: the atomic wraps the whole `perform_mutation`); signal receivers that run inside
`save()`; tasks called through helpers; a task kept in a local before `.delay()`; `task_cls` given as a string
(`Celery("saleor", task_cls="saleor.core.tasks:RestrictWriterDBTask")` is not resolved; there it would not matter, as
that class does not defer). Harmless in context: saleor/saleor
`saleor/webhook/transport/asynchronous/transport.py` (the atomic is
`transaction.atomic(using=<replica>, savepoint=False)`, a read-only snapshot on a replica, so there is nothing to
commit first) and ansible/awx `awx/main/models/inventory.py` (`delete_inventory.delay` comes from
`dispatcherd.publish.task`, not Celery; whether its broker waits for the commit is not visible from the repo). The
rest: WeblateOrg/weblate (5, `@transaction.atomic` views and `billing/tasks.py`; its `OriginTask` offers
`delay_on_commit` but these calls do not use it), makeplane/plane (2, batch reschedules inside
`with transaction.atomic()`), mozilla/pontoon (2), pretix/pretix (16: `tickets.invalidate_cache` on `EventTask`,
`send_webhook` on `ProfiledTask`, scheduled exports; none of these task classes defers), saleor/saleor (3 inside
`traced_atomic_transaction()`).

**Sources:**
- Django docs, [Performing actions after commit](https://docs.djangoproject.com/en/5.2/topics/db/transactions/#performing-actions-after-commit): "Sometimes you need to perform an action related to the current database transaction, but only if the transaction successfully commits. Examples might include a background task, an email notification, or a cache invalidation."
- Django 6.0 docs, [Tasks](https://docs.djangoproject.com/en/6.0/topics/tasks/) (Transactions): "When using a transaction, without waiting for it to commit, workers could start to process a Task which uses objects it can't access yet."
- Celery docs, [First steps with Django](https://docs.celeryq.dev/en/stable/django/first-steps-with-django.html): "A common pitfall with Django is triggering a task immediately and not wait until the end of the database transaction, which means that the Celery task may run before all changes are persisted to the database."
- Simon Charette (core), [django#18627](https://github.com/django/django/pull/18627#discussion_r2353820975): "encourage usage of `transaction.on_commit(partial(tasks.enqueue, ...), alias)` instead of providing `ENQUEUE_ON_COMMIT` at first?"
- korycins, [saleor#18906](https://github.com/saleor/saleor/pull/18906#discussion_r2894325732): "You're inside transaction here. This needs to be called as `on_commit` action. (till end of the transaction block, created for the whole `perform_mutation`, your changes are isolated and not visible by other queries.)"
- Adam Johnson, [Common Celery issues on Django projects](https://adamj.eu/tech/2020/02/03/common-celery-issues-on-django-projects/): "If you enqueue a task from within a database transaction, it might execute before the database commits the transaction. … The solution is to enqueue your tasks after the transaction commits, using Django's transaction.on_commit."

### `django-practice-dispatch-in-atomic-request`

Rerail's own rule, not a port. It reports nothing unless a non-test file of the run (in practice a settings module)
turns ATOMIC_REQUESTS on with the literal True: a dict entry `"ATOMIC_REQUESTS": True`,
`DATABASES[...]["ATOMIC_REQUESTS"] = True`, `ATOMIC_REQUESTS=True` as a keyword (`dict(...)`, `.update(...)`), or the
entry's value a parameter whose default is True (mozilla/addons-server's `get_db_config(..., atomic_requests=True)`).
Then it matches a dispatch in a view, or in a function a view reaches through the cross-file call graph (not through
a call written inside `on_commit(...)`). A view is a def whose first parameter is `request` and that is not a method,
nor the inner `def middleware(request)` of a function-style middleware `def m(get_response)` (Django wraps only the
resolved view in the transaction), or any method of a Django or DRF view class; not an async def, and not a view
marked non-atomic: `@non_atomic_requests` on the def or the class (`method_decorator` included),
`non_atomic_requests(...)` anywhere in the class body (on a method, or allegro/ralph's and addons-server's `as_view`
overrides), or a urls-level `non_atomic_requests(views.f)`. A dispatch is what `django-practice-enqueue-on-commit`
matches: `.delay()`, `.apply_async()`, `.send_task()`, `.enqueue()`/`.aenqueue()` on a receiver that can be a task,
django-q `async_task()`, `send_mail`/`send_mass_mail`/`mail_admins`/`mail_managers`, and `.send()` on an
`EmailMessage`. The message names the call.

It leaves alone: calls inside `transaction.on_commit(...)` or `<connection>.on_commit(...)`; `.delay_on_commit()`;
calls inside an atomic block (enqueue-on-commit reports those, so no line is reported twice); and, as
enqueue-on-commit does, tasks that already wait for the commit (`@app.task(base=X)` where X's `apply_async`/`delay`
calls `transaction.on_commit`, every send of a run whose app is `Celery(..., task_cls=X)` for such an X, as
addons-server's `AMOTask`), calls in an except handler whose every path re-raises (the request rolls back, so an
on_commit callback would drop the failure mail), `r = t.apply_async(...)` whose result the function reads afterwards,
and `.delay()` on a parameter or local (`client.delay(2.0)`); env-driven settings (`env.bool(...)`); a top-level
`ATOMIC_REQUESTS = True`, which Django never reads (readthedocs' settings class and taigaio/taiga-back set it so, and
run in autocommit); test files.

**Gaps:** views whose first parameter is not named `request`; dispatches reached only through framework code (DRF
calling a serializer's `create` is found only when a view calls it), signals, or calls of unknown target; the
ATOMIC_REQUESTS fact is global to the run, so a monorepo holding an atomic and a non-atomic project applies it to
both. Only 5 of the 31 apps turn it on (ansible/awx, openedx/edx-platform, allegro/ralph, django-oscar,
mozilla/addons-server); ralph, oscar and addons-server have no match. The 22 are ansible/awx (6:
`awx/conf/views.py:133` and `:151` send `reconfigure_rsyslog` right after wrapping the neighbouring send in
`connection.on_commit`; `awx/api/views/__init__.py:551`, `:2565`, `:2591`; `awx/api/serializers.py:2361`, reached
from a view) and openedx/edx-platform (16: grading views' `update_credit_course_requirements.delay`,
`rerun_course_task.delay` after `CourseRerunState.objects.initiated(...)` writes the row the task updates,
`reset_student_course.delay` after `CourseResetAudit.objects.create(...)`, activation and verification mails).
Possibly harmless in context, still real: awx's tasks go through dispatcherd's pg_notify broker; if it publishes on
Django's connection, PostgreSQL delivers the NOTIFY only at commit.

**Sources:**
- Django docs, [Tying transactions to HTTP requests](https://docs.djangoproject.com/en/stable/topics/db/transactions/#tying-transactions-to-http-requests): "It works like this. Before calling a view function, Django starts a transaction. If the response is produced without problems, Django commits the transaction. If the view produces an exception, Django rolls back the transaction."
- Django docs, [Performing actions after commit](https://docs.djangoproject.com/en/stable/topics/db/transactions/#performing-actions-after-commit): "Sometimes you need to perform an action related to the current database transaction, but only if the transaction successfully commits. Examples might include a background task, an email notification, or a cache invalidation."
- Celery docs, [Tasks: Database transactions](https://docs.celeryq.dev/en/stable/userguide/tasks.html#database-transactions): "There is a race condition because transactions are atomic. This means the article object is not persisted to the database until after the view function returns a response. If the asynchronous task starts executing before the transaction is committed, it may attempt to query the article object before it exists."
- TestDriven.io, [Working with Celery and Django Database Transactions](https://testdriven.io/blog/celery-database-transactions/): "Since the task executes immediately, user = User.objects.get(pk=user_pk) fails as the user is not in the database because the transaction in Django has not yet been committed."

### `django-practice-task-arg-model-instance`

Rerail's own rule, not a port. It matches an argument of a Celery send, `t.delay(...)`, `t.s(...)`, `t.si(...)`
(positional and keyword), `t.apply_async((x,))`, `t.apply_async(args=[x])` or `t.apply_async(kwargs={"k": x})`,
where `t` resolves through imports, possibly to another module, to a def decorated `@shared_task`, `@app.task(...)`
or any decorator named `task`, and the argument holds a model instance: `self` in a model's method; `request.user` or
`self.request.user`; a model constructor `M(...)`; `M.objects.get/create/first/last/earliest/latest(...)` not after
`values()`/`values_list()`; `get_object_or_404(...)`; a parameter annotated with a model class; a name the same
function assigns from one of those; the target of `for x in <queryset>:` over a queryset not through
`values()`/`values_list()`. Model classes resolve across the run. Test files are skipped.

It leaves alone: `x.pk`, ids and other plain values; `.get()` after `values()`/`values_list()` and loops over them
(dicts and tuples, the shape the rule asks for); sends of plain functions that are not tasks (`send_report.delay(x)`
on an undecorated def); test files.

**Gaps:** instances returned by helpers (`x = make_order()`, taiga's `get_object_or_error(...)`), unannotated
parameters (paperless-ngx's signal handler `add_or_update_document_in_llm_index(sender, document)`), related objects
(`order.customer`), instances inside lists. The type program's `proven_type` (python/types) would catch `x = make_order()` when
the helper returns a constructor call, but reading it makes the engine run the typing program (over 60 minutes on
DefectDojo, against 2 min 20 s without it) for no corpus match the syntactic forms miss. The 5 matches are in taigaio/taiga-back
(`taiga/export_import/api.py:64`, `:367`, and two on `taiga/export_import/management/commands/dump_project_async.py:66`)
and paperless-ngx/paperless-ngx (`src/documents/tasks.py:800`). Both projects set a pickle serializer (taiga
`CELERY_TASK_SERIALIZER = "pickle"`, paperless `"signed-pickle"`), so the sends work and ship snapshots that may be
stale when the worker runs: the stale-data half of the advice.

**Sources:**
- Celery docs, [Tasks: State](https://docs.celeryq.dev/en/stable/userguide/tasks.html#state): "Another gotcha is Django model objects. They shouldn't be passed on as arguments to tasks. It's almost always better to re-fetch the object from the database when the task is running instead, as using old data may lead to race conditions."
- Celery docs, [Calling Tasks: Serializers](https://docs.celeryq.dev/en/stable/userguide/calling.html#serializers): "The primary disadvantage to JSON is that it limits you to the following data types: strings, Unicode, floats, Boolean, dictionaries, and lists."
- nanorepublica, [Never can pass Django model objects to Celery tasks?](https://forum.djangoproject.com/t/never-can-pass-django-model-objects-to-celery-tasks/43893) (Django Forum): "Typically you pass just the primary key(s) of the model. The first line then does a Model.objects.get(pk=model_pk) … The benefit of this approach is you get the latest version of the model over a potentially stale version passed through memory."
- OddBird, [Serializing Things for Celery](https://www.oddbird.net/2017/03/20/serializing-things/): "Given that the safest way to do so is to use a serialization format that only includes primitive data types, how can we send custom objects around?"

### `django-practice-default-called-once`

Rerail's own rule, not a port. It matches, inside the `default=` or `limit_choices_to=` argument of a Django model
field call (a `…Field` or relation class of `django.db.models`, `django.contrib.postgres.fields`,
`django.contrib.gis.db.models` or `modelcluster.fields`, or a class of the run derived from one, such as
`class ShortUUIDField(models.CharField)`), a call whose callee resolves (through `import uuid`, `from uuid import
uuid4`, aliases) to `uuid.uuid1/4/6/7`, any `secrets.*` or `random.*`, `os.urandom`,
`django.utils.crypto.get_random_string`, `time.time`, `datetime.datetime.now/utcnow/today`, `datetime.date.today` or
`django.utils.timezone.now/localtime/localdate`, anywhere in the value (`str(uuid4())`, `uuid4().hex`,
`timezone.now() + timedelta(days=7)`, `now().isoformat()`, `{"date__lte": date.today()}`) but not inside a lambda. It
reports the varying call, naming it.

It leaves alone: a callable (`default=uuid.uuid4`, a module-level function, a lambda); deterministic calls
(`uuid.uuid5(NAMESPACE, "x")`); the defaults Django's check fields.W161 reports. W161 compares the value with the
current UTC time (±10 s), only on DateField, DateTimeField and TimeField, so the rule leaves alone exactly these whole
defaults: `timezone.now()`, `timezone.localtime()` or `datetime.utcnow()` on a DateTimeField, their `.date()` on a
DateField and `timezone.now().time()`/`utcnow().time()` on a TimeField, and `date.today()`/`timezone.localdate()` on
a DateField. A naive `datetime.now()`/`datetime.today()` is local time, which W161 reports only when TIME_ZONE is UTC
(verified with Django 5.2), so it is reported, as are arithmetic, `.year`, `.isoformat()` and other field classes. It
also leaves alone migrations (a one-off `AddField` default is computed once on purpose), test code, and `default=` of
anything that is not a model field (argparse, pydantic, `dict.get`, cache `get_or_set`).

**Gaps:** third-party field classes outside the modules above; a callee no import resolves
(`__import__("datetime").datetime.now()`); project wrappers (`default=make_token()` where `make_token` returns a
random value); form-field `initial=` and DRF serializer `default=`, which have the same flaw but are not model fields.
On the corpus the only match is HumanSignal/label-studio `label_studio/users/models.py`
(`year = models.IntegerField(..., default=datetime.datetime.now().year)`, a module-level field the process freezes at
the year it started). A text search of the corpus for the same calls in any `default=` found no other model field
outside migrations: the rest are argparse/pydantic defaults and archesproject/arches migrations
(`AddField(... default=datetime.datetime.now(), preserve_default=False)`), which the rule skips.

**Sources:**
- Django docs, [Model field reference: `default`](https://docs.djangoproject.com/en/5.2/ref/models/fields/#default): "The default value for the field. This can be a value or a callable object. If callable it will be called every time a new object is created."
- Django docs, [UUIDField](https://docs.djangoproject.com/en/5.2/ref/models/fields/#uuidfield): "Note that a callable (with the parentheses omitted) is passed to `default`, not an instance of `UUID`."
- Claude Paroz (Django core developer), [ticket #24631 comment 1](https://code.djangoproject.com/ticket/24631#comment:1), to a reporter with `default=uuid.uuid4()`: "Re-read the documentation: https://docs.djangoproject.com/en/1.8/ref/models/fields/#uuidfield Note that a callable (with the parentheses omitted) is passed to default, not an instance of UUID."
- Markus Holtermann (Django core, migrations), [ticket #22571 comment 2](https://code.djangoproject.com/ticket/22571#comment:2): "Your solution is not a solution at all, because your default on the DateTimeField is evaluated only once at model loading. You can use Django's django.utils.timezone.now (w/o the trailing ())."

### `django-practice-model-init-override`

Rerail's own rule, not a port. It matches the name of a `def __init__` written directly in a Django model class (a
`models.Model` subclass through the run's classes, or a class declaring a model field) when its own body (not a
nested def, lambda or class) loads `self.<x>` for a field `x` the model or a model base in the run declares (a foreign
key also as `x_id`), in load context (tuple targets such as `self.a, self.b = …` are writes) or as an
augmented-assignment target (`self.hits += 1` reads first); when the parameter after `self` is a named positional one
before `*args` (`def __init__(self, number, *args, **kwargs)`); or when it takes no `*args` at all
(`def __init__(self, **kwargs)`; an annotated `*args: Any` counts as `*args`): `Model.from_db` calls `cls(*values)`, so
every queryset load fails. The message names the fields read or the signature problem.

It leaves alone: an `__init__` that only sets other attributes or rewrites kwargs before `super().__init__` (about 40
of the 54 overrides on the corpus: oscar's basket `_lines` cache, Weblate's `self.stats = …Stats(self)`), the
documented advice but not the documented hazard; reads of the primary key (never deferred, e.g. archesproject/arches
`CardModel` normalising `self.cardid`); reads through `self.__dict__.get(...)` (WeblateOrg/weblate `Project`,
`Workspace`), the fix Simon Charette gives; field reads in an `__init__` that calls `get_deferred_fields()`
(pretix/pretix `Order`, HumanSignal/label-studio `Project`); query-expression classes (`Func`/`Expression`
subclasses declaring `output_field = models.…Field()`); forms, managers, test and migration code.

**Gaps:** fields read through a helper the `__init__` calls (`self._get_fields_snapshot()` in ansible/awx
`PrimordialModel`, `self.get_old_component_settings()` in WeblateOrg/weblate `Component`); fields of a project field
class (allegro/ralph `Network.address` is a `ralph` `IPNetwork` field, so `self._old_address = self.address` is not
reported); bases outside the run. Harmless in context, still the pattern: WeblateOrg/weblate
`weblate/trans/models/change.py` and django-oscar/django-oscar `src/oscar/apps/payment/abstract_models.py` read fields
only when the instance is new (`not args and self.pk is None`, `self.id is None`); nautobot/nautobot
`dcim/models/devices.py` reads `u_height`/images only `if self.present_in_database`, which still loads deferred
fields of every row a queryset returns. The other matches are the tracking pattern the docs warn about:
HumanSignal/label-studio `ml/models.py` (`__original_title = self.title`), WeblateOrg/weblate `billing/models.py`,
goauthentik/authentik `django_dramatiq_postgres/models.py`, inventree/InvenTree `plugin/models.py`,
mozilla/addons-server `users/models.py`, nautobot/nautobot `ControllerManagedDeviceGroup`, pretix/pretix `SubEvent`,
readthedocs/readthedocs.org `api/v3/models.py`, taigaio/taiga-back `attachments/models.py`. No signature change
appeared in the corpus.

**Sources:**
- Django docs, [Model instance reference: Creating objects](https://docs.djangoproject.com/en/5.2/ref/models/instances/#creating-objects): "You may be tempted to customize the model by overriding the `__init__` method. If you do so, however, take care not to change the calling signature as any change may prevent the model instance from being saved. Additionally, referring to model fields within `__init__` may potentially result in infinite recursion errors in some circumstances. Rather than overriding `__init__`, try using one of these approaches:"
- Simon Charette (Django Steering Council), [ticket #31435 comment 2](https://code.djangoproject.com/ticket/31435#comment:2): "In all cases your code was broken because you are accessing a deferred field in your TimeSeries.__init__ model which is not allowed. You can test it out but on any version of Django doing TimeSeries.objects.defer('frequency') would cause N + 1 queries to be performed where N = TimeSeries.objects.count(). You want to do self._old_frequency = self.__dict__.get('frequency') in there."
- Simon Charette, [ticket #36713 comment 1](https://code.djangoproject.com/ticket/36713#comment:1): "Please refer to the documentation about Model.__init__ and the note about overriding it ... You can refer to #31435 for some discussion on the subject but the TL;DR is that your __init__ override must account for some fields being potentially deferred."
- Tim Graham (Django Fellow), [ticket #22858 comment 1](https://code.djangoproject.com/ticket/22858#comment:1): "If you access a field in __init__(), you should include it in all of your only() calls to avoid hitting the database again for each item in the queryset."

### `django-practice-save-ignores-update-fields`

Rerail's own rule, not a port. It matches the name of a `save` method written directly in a Django model that calls
`super().save(…)` (or `super(C, self).save(…)`) with a `**kwargs` splat, when before that call, in the method's own
body, it assigns (`=` or augmented) `self.<x>` for a field `x` the model or a model base in the run declares, or a
foreign key's `<fk>_id`, and the method never mentions `update_fields` (identifier or string) and never rebinds its
`**kwargs` parameter. The message lists the fields set, and says how to leave a deliberate new-row-only default alone.

It leaves alone: assignments, and forwarding calls, that run only for a new row: in the `elif`/`else` branches of an
`if` testing that the row exists (`if self.pk:`, `if self.pk is not None:`, `if not self._state.adding:`,
`if self.present_in_database:`), after such an `if` whose body returns, and under an `if` or `elif` whose test is true
only for a new row: `not self.pk`/`not self.id`/`not bool(self.pk)`, `self.pk is None`, `self._state.adding`,
`not self.present_in_database` (nautobot), `kwargs.get("force_insert")`, `not self.pk or kwargs.get("force_insert")`,
alone or before `and`, or a local set to one of these (`is_new = not bool(self.pk)` … `if is_new and …`). An insert
writes every field whatever update_fields says; on the corpus these shapes cover django-oscar/django-oscar
`wishlists/abstract_models.py`, mozilla/addons-server `files/models.py:687`, nautobot/nautobot
`device_components.py:2124` and `users/models.py:299`. It also leaves alone a method that rebinds kwargs
(archesproject/arches `kwargs = add_to_update_fields(kwargs, "…")`), a parameter named `update_fields`, assignments
after the write, `save()` without `**kwargs`, non-model classes, tests and migrations.

**Gaps:** fields set in a helper the save calls (`self.set_sla_expiration_date()`, `self._assign_parent()`,
`self.calculate_numeric_value()`), or with `setattr(self, name, value)` (allegro/ralph `assets/models/assets.py` for
`barcode`/`sn`); fields declared through project field classes or in bases outside the run; new-row tests in other
spellings (`if created`, `self._state.db is None`). It can name a field set only when it was empty
(`if not self.slug: self.slug = …`), which in practice runs mostly on insert. These count as real: an update that
clears the field then gives update_fields still skips it. Harmless in context: apps that never pass `update_fields`
or call `update_or_create` on that model; the pattern is still the docs' latent bug. Typical matches: derived values
on every save (django/djangoproject.com `blog/models.py` `summary_html`/`body_html`, makeplane/plane
`description_stripped` in five models, netbox-community/netbox `_abs_weight`/`_abs_distance` mixins,
taigaio/taiga-back `modified_date` in eleven models), normalisation (WeblateOrg/weblate `vcs/github.py` `hostname`,
readthedocs/readthedocs.org `projects/models.py` `domain`), state recomputation (nautobot/nautobot
`extras/models/approvals.py` `current_state`/`decision_date`, mozilla/addons-server `scanners/models.py`
`has_matches`), and a foreign key's key (DefectDojo/django-DefectDojo `dojo/location/models.py`:
`self.test_id = self.finding.test_id`).

**Sources:**
- Django docs, [Overriding predefined model methods](https://docs.djangoproject.com/en/5.2/topics/db/models/#overriding-predefined-model-methods): "If you wish to update a field value in the `save()` method, you may also want to have this field added to the `update_fields` keyword argument. This will ensure the field is saved when `update_fields` is specified."
- Django 4.2 release notes (`docs/releases/4.2.txt`): "`QuerySet.update_or_create` now passes `update_fields` to the `Model.save()` calls. As a consequence, any fields modified in the custom `save()` methods should be added to the `update_fields` keyword argument before calling `super()`."
- Simon Charette, [forum "update_or_create Behavior" #4](https://forum.djangoproject.com/t/update-or-create-behavior/25944/4): "Aren't such derived fields broken by design for any usage of Model.save(update_fields)? Why can't the save method of such models be adapted to augment update_fields with the dependant fields instead?"
- Jacob Walls (Django Fellow), [forum "Hook for extending update_fields in save() overrides" #4](https://forum.djangoproject.com/t/hook-for-extending-update-fields-in-save-overrides/26040/4), labelling this shape "Hypothetical current code": "class MyModel(models.Model): def save(self, *args, **kwargs): if altering_field_one: self.field_one = 1 ... super().save(*args, **kwargs)"

### `django-practice-lazy-translation-at-import`

Rerail's own rule, not a port. It matches a call whose callee resolves to `django.utils.translation.gettext`,
`ngettext`, `pgettext`, `npgettext`, `ugettext` or `ungettext` (bare, through
`from django.utils.translation import gettext as _`, or `translation.gettext(...)`) when no def or lambda holds it: a
module or class body (model field `verbose_name`/`help_text`/`choices`, `class Meta`, form and filter `label=`,
serializer `default_error_messages`, admin `short_description`, management command `help`), a decorator
(`@admin.display(description=_(…))`), a module constant; or when it is a parameter default of a def that is itself
defined at import (`def __init__(self, message=_("…"))` in a class body). A module-level alias in the same file
(`__ = gettext`) is followed.

There are two messages. Where the result is turned into a str at import (an operand of `%` or `+`, the object of a
method call such as `.format()`/`.upper()`, an f-string interpolation, a dict key, inside `str()`/`json.dumps()`),
`gettext_lazy` would be forced there too (and `json.dumps` of a lazy proxy raises TypeError), so the message says to
build the text in the function that uses it or with `format_lazy()`; elsewhere it names the lazy form.

It leaves alone: calls inside def and lambda bodies, which run per call in the active language; a call that is a
statement of its own (`_("Scrum")`, a marker for makemessages whose result nothing reads, e.g. 51 lines of
taigaio/taiga-back `taiga/projects/translations.py`); a class defined inside a function; the lazy forms and
`gettext_noop`; test code and migrations.

**Gaps:** a project re-export of gettext (`from core.i18n import _`), or an alias assigned in another module, which
import resolution does not follow. A lazy string forced at import (`gettext_lazy("x") % y`, `"…" + _lazy("…")`,
f-strings) is not matched; other ways of forcing a non-lazy result (passing it to a function that formats it) get the
lazy-form advice. Matches are real wherever read: zulip/zulip `zerver/lib/typed_endpoint.py` `ERROR_TEMPLATES`
(module-level `_("{var_name} is not an integer")`) is read straight into error responses, so clients get the language
the server started in. The counts are dominated by apps that import `gettext as _` in model, form and filter modules:
netbox-community/netbox 950 (filterset labels, form labels, model `verbose_name`), DefectDojo/django-DefectDojo 537
(model `help_text`/`verbose_name`), openedx/edx-platform 127 (module constants, management command `help`, admin
`short_description`). No match was in a Django settings module or an `apps.py`, where a non-lazy call at import would
raise AppRegistryNotReady (pretix/pretix `src/pretix/base/settings.py` is pretix's own hierarkey defaults, a regular
module). The precision is from a spread sample of 82 matches, 5-12 per repository across files, every matching
repository represented; the 1,699 matches fall on 1,671 lines.

**Sources:**
- Django docs, [Lazy translation](https://docs.djangoproject.com/en/5.2/topics/i18n/translation/#lazy-translation): "This is essential when calls to these functions are located in code paths that are executed at module load time. This is something that can easily happen when defining models, forms and model forms, because Django implements these such that their fields are actually class-level attributes. For that reason, make sure to use lazy translations in the following cases:"
- Mark Walker (django-cms org member, django-filer maintainer), [django-filer#1377 comment](https://github.com/django-cms/django-filer/issues/1377#issuecomment-1625405457): "`gettext_lazy()` should be used for anything loaded at runtime: * `models.py` (fields, `verbose_name`, `help_text`, function `short_description`); * `forms.py` (`labels`, `help_text`, `empty_label`); * `apps.py` (`verbose_name`). `gettext()`should be used for any modules that are executed during the request process, such as `views.py`"
- dchukhin (Lincoln Loop), [wagtail-localize-dashboard#18 review](https://github.com/lincolnloop/wagtail-localize-dashboard/pull/18#discussion_r4527805628), on a view-class attribute using `gettext`: "It looks like this string shows as untranslated for browsers in a non-English language, but using `gettext_lazy` here makes it appear translated."

### `django-practice-clean-field-returns-value`

Rerail's own rule, not a port. It matches a method `clean_<x>(self)` (no other parameter) written directly in a form
class, outside test code: a subclass, through the run's classes in any file, of a django.forms form class or of a
`*Form` class from another package's `forms` module (django.contrib.auth.forms.UserCreationForm, allauth, wagtail). It
reports at the method's name when its body can reach its end, and at each bare `return`. Django's `_clean_fields`
stores the method's return value as `cleaned_data[<x>]`, so either path turns the field's value into None. Return and
raise end a path; if/elif/else, try/except/else/finally and with are followed; a `for` or `while` loop can end unless
it has an `else` that cannot and no `break` reaches it (`while True` ends only by a break); a `match` can end when a
case body can, or when no case catches every value (no `case _:` or bare capture name without a guard).

It leaves alone: `return None`, written on purpose; clean_<x> methods taking more arguments (Django calls them with
none); helpers: a clean_<x> the class or a base in the run calls on `self` (readthedocs' `clean_prevalidation()`,
which its `PrevalidatedForm` base runs itself), or whose x is a method or property of the class (Misago's
`clean_attachments_limit`); classes that are not forms; test code.

**Gaps:** a form whose base is an attribute (`UserAdmin.form`: allegro/ralph `accounts/admin.py`), modelform_factory
forms; a call that never returns (`sys.exit`) as the last statement is reported. Harmless in context: WeblateOrg/weblate
`weblate/accounts/forms.py` (captcha fields whose value nothing reads). Examples: mozilla/addons-server
`src/olympia/devhub/forms.py` (`clean_slug` validates and falls off: the slug becomes None), nautobot/nautobot
`nautobot/ipam/forms.py` (`clean_namespace` pops the key, then Django writes it back as None), django-oscar/django-oscar
`src/oscar/apps/customer/forms.py` (`clean_redirect_url` returns None for a disallowed URL), openedx/edx-platform (4),
pretix/pretix (6). The 21 matches sit in 18 methods; only 8 of them also return a value somewhere, the part Ruff
RET503 and pylint R1710 can see; 10 never return one.

**Sources:**
- Django docs, [Form and field validation](https://docs.djangoproject.com/en/dev/ref/forms/validation/): "If no ``ValidationError`` is raised, the method should return the cleaned (normalized) data as a Python object." and, on `clean_<fieldname>()`: "The return value of this method replaces the existing value in ``cleaned_data``, so it must be the field's value from ``cleaned_data`` (even if this method didn't change it) or a new cleaned value."
- MDN Learn, [Django Tutorial Part 9: Working with forms](https://developer.mozilla.org/en-US/docs/Learn_web_development/Extensions/Server-side/Django/Forms): "# Remember to always return the cleaned data." and "we return this data whether or not we change it at the end of the function."

### `django-practice-modelform-clean-calls-super`

Rerail's own rule, not a port. It matches the name of a `def clean(self)` written directly in a class that derives,
through the run's classes in any file, from `ModelForm`, `BaseModelForm`, `BaseModelFormSet`, `BaseInlineFormSet`,
`BaseGenericInlineFormSet`, auth's `UserCreationForm`/`BaseUserCreationForm`/`UserChangeForm`, flatpages'
`FlatpageForm`, wagtail's `WagtailAdminModelForm`/`WagtailAdminPageForm`, an outside class named `…ModelForm` or
`…ModelFormSet` (i18nfield's `I18nModelForm`, parler's `TranslatableModelForm`), or a `django.…InlineFormSet`
re-export (`django.contrib.admin.options.BaseInlineFormSet`), when its own body (not a def nested in it) has no
`super().clean(...)`, `super(C, self).clean(...)` or `X.clean(self, ...)`. In a formset, `self.validate_unique()`,
all that `BaseModelFormSet.clean()` does, counts as the parent call. The message names what is skipped: the model's
unique, unique_together and constraint validation for a form; the uniqueness check across the forms for a formset.

It leaves alone: plain `Form` and formset classes (no model validation to skip); `clean_<field>()` methods; a parent
call anywhere in clean()'s own body, inside a `try` or a lambda included; test files. In a form,
`self.validate_unique()` alone is still reported: the constraint checks that `ModelForm.clean()` turns on stay off.

**Gaps:** a ModelForm built by `modelform_factory`; a mixin's `clean()` mixed into a ModelForm; third-party bases named
otherwise; a parent call only in a def nested in `clean()`. Reported though the validation runs: a `clean()` that
reaches the parent through a helper method (`return self._parent_clean()`) or sets `self._validate_unique = True`
itself (none in the corpus). Some matched models have no unique field or constraint, which makes the skip harmless
today and not after the next migration. Examples: mozilla/addons-server `src/olympia/devhub/forms.py:1407` (a
`BaseModelFormSet` whose `clean()` only returns on errors, so the formset never checks uniqueness),
django-oscar `src/oscar/apps/customer/forms.py:393` (re-implements a duplicate check with a query instead of calling
super), netbox `netbox/users/forms/model_forms.py:262`, DefectDojo (6), pretix (6, one on an i18nfield
`I18nModelForm`), allegro/ralph `src/ralph/reports/forms.py:9` (through the admin re-export of `BaseInlineFormSet`).

**Sources:**
- Django docs, [Overriding the clean() method](https://docs.djangoproject.com/en/dev/topics/forms/modelforms/#overriding-the-clean-method): "The ``ModelForm.clean()`` method sets flags that make the model validation step validate the uniqueness of model fields that are marked as ``unique``, ``unique_together`` or ``unique_for_date|month|year``, as well as constraints. If you would like to override the ``clean()`` method and maintain this validation, you must call the parent class's ``clean()`` method."
- Django docs, [Overriding clean() on a ModelFormSet](https://docs.djangoproject.com/en/dev/topics/forms/modelforms/#overriding-clean-on-a-modelformset): "If you want to override the ``clean()`` method on a ``ModelFormSet`` and maintain this validation, you must call the parent class's ``clean`` method"
- Yuji 'Tomita' Tomita, accepted answer to [Django unique model fields validation in form](https://stackoverflow.com/questions/31150929/django-unique-model-fields-validation-in-form): "Running the superclass `clean` method should take care of model uniqueness checks given a `ModelForm`."

### `django-practice-function-decorator-on-method`

Rerail's own rule, not a port. It matches, on a method (first parameter `self`) defined directly in a class body, a
decorator that resolves by import to one of: `login_required`, `permission_required`, `user_passes_test`,
`staff_member_required`; `require_http_methods`, `require_GET`, `require_POST`, `require_safe`, `condition`, `etag`,
`last_modified`; `csrf_protect`, `ensure_csrf_cookie`, `requires_csrf_token`; `cache_page`; `never_cache`,
`cache_control`, `sensitive_post_parameters` (these fail on every request: they take `self` for the request), and
`gzip_page` (fails on every response it would compress; GZipMiddleware returns early for short or already-encoded
responses), bare or called. `csrf_exempt`, `login_not_required` and `no_append_slash` only mark the function, and
`as_view()` copies the marks from `dispatch()` alone, so those three are matched only on a method other than
`dispatch()` of a class-based view (Django or DRF). On a Django REST framework view (a run class deriving from a
`rest_framework.*` view), `csrf_exempt` gets its own message: it is redundant, since DRF's `as_view()` already exempts
the view and DRF's CSRF check comes from `SessionAuthentication`; the fix is to remove it (or use other authentication
classes), not to wrap it. Every other case says how it fails and gives the `method_decorator` form.

It leaves alone: `@method_decorator(...)`; `csrf_exempt`/`login_not_required` on `dispatch()`;
`csrf_exempt`/`login_not_required` on methods of non-view classes, where a bound method passed as a view carries the
mark (edx-platform `common/djangoapps/third_party_auth/admin.py`, `admin_view(self.upload_csv)`; zulip
`zerver/lib/integrations.py`, `Integration.view`, which even asserts the mark); functions without `self` (arches
`arches/app/views/auth.py` `ExternalOauth.callback(request)`, used as a plain function in urls.py);
`staticmethod`/`classmethod`; harmless decorators (`vary_on_*`, `xframe_options_*`, `sensitive_variables`); test
files.

**Gaps:** project decorators wrapping these (`course_author_access_required` and the like); aliases re-exported from
another module outside the run. A view class that defines its own `.user` or `.method` would hide the crash (still
wrong). All 8 matches are `@csrf_exempt` on DRF view methods (ansible/awx `awx/api/views/webhooks.py`;
openedx/edx-platform `cms/djangoapps/contentstore/rest_api/v0/views/{assets,authoring_videos,transcripts,xblock}.py`):
dead code that suggests the view is exempt, reported with the "remove it" message.

**Sources:**
- Django docs, [Decorating the class](https://docs.djangoproject.com/en/5.2/topics/class-based-views/intro/#decorating-the-class): "A method on a class isn't quite the same as a standalone function, so you can't just apply a function decorator to the method – you need to transform it into a method decorator first. The `method_decorator` decorator transforms a function decorator into a method decorator so that it can be used on an instance method."
- Luke Plant (core developer), [ticket #15794](https://code.djangoproject.com/ticket/15794#comment:1): "I agree that we should fix the documentation to say that you should decorate the dispatch method. In some cases, it may be possible to decorate the get/post/etc methods etc. but that depends on the nature of the decorator."
- Django fix commit [16056] for that ticket: "Corrected an error in the docs which indicated applying decorators to any of the view-like methods would work when it will only work reliably with dispatch."

### `django-practice-get-handler-writes`

Rerail's own rule, not a port. The handlers it reads, outside test code: for each view class K (Django's View and
generic views, DRF views and viewsets, through the run's classes), the method K resolves `get` or `head` to, its own
or inherited, and for a DRF class (a `rest_framework` base, or a vendored copy's `ViewSetMixin` such as taiga's
`taiga.base.api`) also `list` and `retrieve`; and a function decorated `@require_GET`, `@require_safe`,
`@require_http_methods([...])` or DRF's `@api_view([...])` listing only GET, HEAD or OPTIONS (any case), or a bare
`@api_view()`. It matches such a handler when it, or a function it reaches through the call graph (python/calls,
across files), runs a QuerySet write (`create`, `update`, `delete`, `get_or_create`, `update_or_create`,
`bulk_create`, `bulk_update`, or their async twins) or `save()`/`delete()` on a model instance: `self` in a model
method, a name bound to `M(...)`, to a fetch (`M.objects….get()/first()/…`, `get_object_or_404(M, …)`), to
`self.get_object()` in a view class with `model = M` or `queryset = M.objects…`, a for-loop target over
`M.objects…`, or a receiver the type program proves is a model; where the call graph has no target for `obj.m()` on
such an instance, it runs M's method m. `self.m()` in K's methods goes to what K resolves m to, so a base view's get
is judged with the overrides of the class that inherits it. One finding per handler, naming one write: at the def for
a view's own method or a function view, at `class K` when K inherits the handler and only K's overrides make it
write.

It leaves alone: handlers of unsafe methods (`post`, `@require_POST`, `@require_http_methods(["GET", "POST"])`);
`list`/`retrieve` on a plain Django View, which never routes to them; a `get` on a class that is not a view; test
code.

**Gaps:** writes through related managers (`obj.tags.add`), custom managers, `form.save()` or raw SQL; handlers chosen
by URLconf or `http_method_names`; calls the call graph cannot resolve; views whose base only a two-dot relative
import names (`from .. import BaseAPIView`, makeplane/plane: [ENGINE-BUGS.md](ENGINE-BUGS.md) #4). A call resolved
late-bound, to whichever class runs at run time, may lead to a write this request never runs: goauthentik/authentik's
flow views reach every flow stage's writes through `plan.to_redirect()` and `executor.stage_ok()`/`stage_invalid()`
(18 of its 41 matches are unclear for that reason, e.g. `authentik/sources/telegram/views.py`,
`authentik/stages/consent/stage.py`). The write the message names is any one of those reachable, not the nearest:
zulip/zulip `zerver/views/auth.py` (`api_get_server_settings`) names a push-device registration reached through its
queue workers, while its real write is `process_client`'s lazy `get_client` row. Real examples: python-discord/site
`pydis_site/apps/home/views.py` (`get` refreshes cached repository metadata with bulk_create), taigaio/taiga-back
`taiga/projects/settings/api.py` (`list` lazily creates per-user settings), archesproject/arches
`arches/app/views/api/search.py` (`get` saves a SearchExportHistory row), inventree/InvenTree
`src/backend/InvenTree/users/api.py` (`get` deletes and creates an API token), WeblateOrg/weblate
`weblate/accounts/views.py` (an AuditLog row on viewing recovery codes; a TOTP device on GET), openedx/edx-platform
`lms/djangoapps/user_tours/v1/views.py` (bulk_create on GET) and its lazy writes on read: anonymous user ids, cohort
assignment, course grades, user activity (59 matches). The precision is from 85 matches read: all 63 outside
openedx/edx-platform and a random 22 of its 59, every repository represented; the 24 not counted are unclear, not
wrong. The 122 matches fall on 121 lines: taigaio/taiga-back `taiga/projects/notifications/api.py` reports one class for
both its inherited list and retrieve.

**Sources:**
- Django docs, [Cross Site Request Forgery protection](https://docs.djangoproject.com/en/stable/ref/csrf/): "The first defense against CSRF attacks is to ensure that GET requests (and other 'safe' methods, as defined by :rfc:`9110#section-9.2.1`) are side effect free." and "It deliberately ignores GET and QUERY requests (and other requests that are defined as 'safe' by :rfc:`9110#section-9.2.1`). These requests ought never to have any potentially dangerous side effects, and so a CSRF attack with a GET request ought to be harmless."
- RFC 9110, [§9.2.1 Safe Methods](https://www.rfc-editor.org/rfc/rfc9110#section-9.2.1): "Request methods are considered "safe" if their defined semantics are essentially read-only; i.e., the client does not request, and does not expect, any state change on the origin server as a result of applying a safe method to a target resource." The same section allows incidental effects: "most servers append request information to access log files at the completion of every response, regardless of the method, and that is considered safe".

Why Log: the handler-to-write chains go through the call graph, which follows late-bound calls to every class that
may run; in plugin-style code (authentik's flow stages, zulip's queue workers) that reaches writes a given request
never makes, and 24 of the 85 read could not be shown to write. Many real matches are deliberate lazy writes on read
(openedx's anonymous ids, cohort assignment and grade caching; read tracking in Misago; audit rows in Weblate), which
RFC 9110's allowance for incidental effects covers in spirit and a team may keep. The rule records where GET
handlers write.

### `django-practice-host-header-without-get-host`

Rerail's own rule, not a port. It matches a load of `<x>.META["HTTP_HOST"]`, `<x>.META.get("HTTP_HOST")`,
`<x>.headers["host"]`, `<x>.headers.get("Host")` and the X-Forwarded-Host spellings (`HTTP_X_FORWARDED_HOST`,
`x-forwarded-host`, `x_forwarded_host`), the key a string literal in any case. django-upgrade's `headers` fixer
rewrites the META form to the headers form, so both are matched.

It leaves alone: stores and `del` (`request.META["HTTP_HOST"] = ...`, test and RequestFactory setup); reads in the
arguments of a logger call (`<…log…>.warning/info/error/…(…)`), such as pretix `src/pretix/control/views/auth.py`
lines 140 and 177, which log the raw header in an origin-mismatch warning; test files.

**Gaps:** keys built dynamically; WSGI `environ["HTTP_HOST"]`; `"HTTP_HOST" in request.META` (only a presence test).
Not real: pretix `src/pretix/multidomain/middlewares.py:144` shows the raw X-Forwarded-Host on a 400 "unknown host"
page, and taiga-back `taiga/feedback/api.py:36` puts the raw header in a feedback email's metadata: both diagnostics.
Harmless in context but real: pretix `src/pretix/multidomain/middlewares.py:75,77`, which resolves custom domains
itself and raises DisallowedHost for unknown ones. Real: DefectDojo `dojo/reports/ui/views.py` (URL built from the
header), edx-platform `common/djangoapps/util/cache.py` (cache key), `lms/djangoapps/branding/views.py`,
`xmodule/util/xmodule_django.py`, Misago `misago/core/utils.py` (four reads in the referrer check).

**Sources:**
- Django docs, [Host header validation](https://docs.djangoproject.com/en/5.2/topics/security/#host-header-validation): "Django validates Host headers against the ALLOWED_HOSTS setting in the django.http.HttpRequest.get_host() method. This validation only applies via get_host(); if your code accesses the Host header directly from request.META you are bypassing this security protection."
- David Cramer (Sentry), [commit b5c36ac "Use request.get_host instead of HTTP_HOST"](https://github.com/getsentry/sentry/commit/b5c36accdb6448ecb9493ca4ae2a20fb3bef05b4): `member.get_dsn(request.META['HTTP_HOST'], …)` replaced by `member.get_dsn(request.get_host(), …)`.

Why Log: 2 of 12 matches are diagnostics that show the raw header on purpose (an error page, a feedback email), which
one file cannot tell from a host used to build links or keys; the message says to leave the raw read for logging or
diagnostics.

### `django-practice-middleware-ignores-streaming`

Rerail's own rule, not a port. It matches `<r>.content` where r is the response of a middleware's response phase: the
third parameter of a `process_response` method, or a name the function assigns from `get_response(…)` /
`self.get_response(…)` (awaited or not); one finding per function, at its first such read.

It leaves alone: functions that mention `.streaming` or `.streaming_content`, name `StreamingHttpResponse` or
`FileResponse`, call `isinstance(r, …)`, or call `hasattr`/`getattr(r, "content"|"streaming"|"streaming_content")`;
a read inside a `try` whose handler catches `AttributeError`, `Exception` or `BaseException` (streamed responses are
handled there; a handler for another error guards nothing); `.content` of other objects; test files. The guards keep
two corpus middlewares quiet that handle streaming correctly: label-studio `label_studio/core/middleware.py`
(`hasattr(response, 'content')`) and zulip `zerver/middleware.py` (`isinstance(response, HttpResponse)`). An
`hasattr(r, "data")` is no guard.

**Gaps:** helpers that receive the response (the debug-toolbar bug was in a panel helper); responses reached another
way (`super().__call__`, `process_template_response`); a guard on Content-Type only, or a guard that tests a different
object, still silences nothing and is matched (none in the corpus). The one match, pretix
`src/pretix/api/middleware.py:84`, is an idempotency middleware that stores `resp.content` (seven reads,
`isinstance(resp.content, …)`) for any successful API call: a streamed or file response there raises AttributeError.
It steers on a small sample: the one function matched holds the pattern, and the guards cover every correct
middleware in the corpus.

**Sources:**
- Django docs, [Dealing with streaming responses](https://docs.djangoproject.com/en/5.2/topics/http/middleware/#dealing-with-streaming-responses): "Unlike HttpResponse, StreamingHttpResponse does not have a content attribute. As a result, middleware can no longer assume that all responses will have a content attribute. If they need access to the content, they must test for streaming responses and adjust their behavior accordingly".
- Matthias Kestenholz (django-debug-toolbar maintainer), [django-debug-toolbar#1945](https://github.com/django-commons/django-debug-toolbar/issues/1945): "Thanks for the report. The new alerts panel should have a check for streaming responses."
- Anders Kaseorg (Zulip maintainer), [zulip#26047](https://github.com/zulip/zulip/pull/26047): "`streaming_content` is an iterator. Consuming it within middleware prevents it from being sent to the browser."

### `django-practice-settings-runtime-assignment`

Rerail's own rule, not a port. It matches, outside test code, an assignment, augmented assignment or `del` whose
target is an upper-case attribute of django.conf.settings (resolved through imports, so
`from django.conf import settings as s` and `django.conf.settings.X` count), or an item of one at any depth
(`settings.CACHES["default"]["LOCATION"] = …`), and `setattr`/`delattr(settings, "X", …)` with a string literal name.

It leaves alone: reads; lower-case attributes (`settings._wrapped`: Django's own machinery, not a setting); `setattr`
with a computed name; a `settings` that is a parameter (pytest-django's `settings` fixture, Open edX's
`plugin_settings(settings)` hooks, which build the settings module); test code (tests/, test_*.py, conftest.py, which
also covers `pytest_configure` hooks).

**Gaps:** in-place mutation through a method (`settings.INSTALLED_APPS.append(x)`, `settings.FEATURES.update(…)`); a
settings object reached through another name (`s = settings; s.X = 1`); e2e helpers and build scripts outside test
directories are matched (paperless-ngx/paperless-ngx `src-ui/e2e/backend.py`, pretix/pretix `src/pretix/_build.py`):
the pattern is there, the harm is small. Harmless in context: ansible/awx (40 of the matches, e.g.
`awx/api/views/root.py`, `awx/main/utils/candlepin/__init__.py`) replaces django.conf.settings with
`awx.conf.settings.SettingsWrapper`, whose `__setattr__` saves registered settings to the database; the message says
to leave such deliberate wrappers alone. goauthentik/authentik `authentik/enterprise/audit/apps.py` and label-studio
`core/middleware.py` rewrite `MIDDLEWARE` while the app loads.

**Sources:**
- Django docs, [Settings: Altering settings at runtime](https://docs.djangoproject.com/en/5.2/topics/settings/#altering-settings-at-runtime): "You shouldn't alter settings in your applications at runtime. For example, don't do this in a view: ... `settings.DEBUG = True  # Don't do this!` The only place you should assign to settings is in a settings file."
- Adam Johnson (Django contributor), [Django settings patterns to avoid: Avoid direct setting changes](https://adamj.eu/tech/2022/11/24/django-settings-patterns-to-avoid/): "it's possible to change a setting at runtime by directly setting it. But you should avoid doing so, since this does not trigger the django.test.signals.setting_changed signal, and any signal receivers will not be notified. Django's documentation warns against this pattern. For example, tests should not directly change settings"; and "making direct changes to pytest-django's settings fixture is okay."

### `django-practice-settings-module-import`

Rerail's own rule, not a port. It recognises the project settings modules from the run's files: a module that assigns
`INSTALLED_APPS` at module level (`=` or `+=`), a module that `DJANGO_SETTINGS_MODULE` names
(`os.environ.setdefault("DJANGO_SETTINGS_MODULE", "x")` or `os.environ["DJANGO_SETTINGS_MODULE"] = "x"`, in
manage.py/wsgi.py/asgi.py or anywhere), and, closed under `from … import *`, a module that star-imports a settings
module (`settings/__init__.py: from .base import *`, `production_settings.py`) or that a settings module star-imports
(`from .local import *`). Imported module names are matched under the importing file's source root, because the
engine names modules from the repository root (`netbox/netbox/settings.py` is `netbox.netbox.settings`, which
`netbox/utilities/x.py` imports as `netbox.settings`): the name resolves to a run module `r.n` where `r` is a package
prefix of the importer's own module name. It then matches, per imported name: an upper-case name from a settings
module (`from pontoon.settings.base import SITE_URL`, relative `from ...settings import WEBHOOK_SYNC_TIMEOUT`), or the
settings module itself as a module object (`from pydis_site import settings`, `from . import settings`,
`from pontoon.settings import base`, `import x.settings [as y]`).

It leaves alone: `django.conf.settings`; app defaults modules that happen to be named `settings.py`
(`rest_framework.settings`, `common.settings`, `chunked_upload.settings`, `pretix.base.settings`): no INSTALLED_APPS,
not named by DJANGO_SETTINGS_MODULE; lower-case names (helpers such as `env_bool`, `get_global_setting`,
`_parse_caches`) and CapWords classes (`plane.settings.storage.S3Storage`); files that are themselves settings
modules, any file under a `settings` directory or named `*settings*.py`, manage.py, wsgi.py, asgi.py; test code (Adam
Johnson's exception: tests of the settings module). `global_settings` imports are not matched (see
[NOTES-django.md](NOTES-django.md)).

**Gaps:** settings modules outside the run, and project settings that neither assign INSTALLED_APPS, nor are named by
a DJANGO_SETTINGS_MODULE literal, nor are star-linked to one (a settings object built by a class, as readthedocs's
`CommunityBaseSettings`); `import settings` resolved through `sys.path` games when the run has several `settings`
modules at different roots (any of them can match). Harmless-in-context matches: zulip/zulip
`scripts/setup/generate_secrets.py` (three imports of `zproject.settings` in a setup script that runs before Django is
configured), django/djangoproject.com `scripts/sync_stripe_products.py` and label-studio
`label_studio/core/argparser.py` (CLI code before setup); archesproject/arches
`arches/app/models/migrations/8838_descriptors_update.py` reads the module in a migration. Most matches are pontoon
(23 of 49: `pontoon/insights/utils.py` imports 13 settings from `pontoon.settings.base`). The 49 matches are 48 lines:
netbox `netbox/utilities/templatetags/helpers.py:11` imports two settings on one line and is reported once per name.

**Sources:**
- Django docs, [Settings: Using settings in Python code](https://docs.djangoproject.com/en/5.2/topics/settings/#using-settings-in-python-code): "Also note that your code should *not* import from either `global_settings` or your own settings file. `django.conf.settings` abstracts the concepts of default settings and site-specific settings; it presents a single interface. It also decouples the code that uses settings from the location of your settings."
- Adam Johnson, [Django settings patterns to avoid: Don't import your project settings module](https://adamj.eu/tech/2022/11/24/django-settings-patterns-to-avoid/): "It's possible to directly import your settings module and read from it directly, bypassing Django's settings machinery ... You should avoid doing so though. If settings are loaded from a different place, or overridden with override_settings etc., they won't be reflected in the original module. You should always import and use Django's settings object"; "It can occasionally be legitimate to import your settings modules directly, such as when testing them".

### `django-practice-async-request-user`

Rerail's own rule, not a port. It matches, outside test code, a load of the attribute `user` on `request` or
`self.request`, and `getattr(request, "user", …)`, where `request` is a name whose lookup lands on a function
parameter named `request` (any position, so closures over an outer view's `request` count), and the read's nearest
def, lambda or class is an `async def`.

It leaves alone: files that import neither Django nor DRF (a Starlette or FastAPI `async def me(request)` handler is
not a Django view); `await request.auser()`; writes (`request.user = …` in middleware); reads inside a nested sync def
or lambda (code handed to `sync_to_async`); `request.user` passed whole as an argument of `sync_to_async(f)(…)` /
`database_sync_to_async(f)(…)`, where only the lazy object crosses into the thread; a `request` that is not a
parameter (a module global); test code.

**Gaps:** a request passed to a helper coroutine under another name (`req.user`); templates rendered from async views
that read `user`; `request.user` passed whole to an ordinary call (`await service.asearch(unit, request.user)`) is
matched although passing it does not evaluate the lazy object (the callee then uses it in async code, which is the
same problem one call later). All 45 matches are in WeblateOrg/weblate (`weblate/vcs/views.py`,
`weblate/accounts/views.py`, `weblate/screenshots/views.py`, …), whose own
`weblate.accounts.middleware.AuthenticationMiddleware` resolves the user in a thread
(`sync_to_async(self.prepare_request)`) and stores a loaded user in `request.user`, so these reads are harmless in
context; chained relations on it (`request.user.profile.watched`, `request.user.subscription_set`) still query
lazily. The message names this case.

**Sources:**
- Django docs, [Authentication in web requests](https://docs.djangoproject.com/en/5.2/topics/auth/default/#authentication-in-web-requests): "These provide a request.user attribute and a request.auser async method on every request which represents the current user." followed by "Or in an asynchronous view: `user = await request.auser()`".
- Django docs, [HttpRequest.auser()](https://docs.djangoproject.com/en/5.2/ref/request-response/#django.http.HttpRequest.auser): "This is similar to the user attribute but it works in async contexts."
- Carlton Gibson (former Django fellow), [Django Forum: Workaround for SynchronousOnlyOperation when using AuthenticationMiddleware](https://forum.djangoproject.com/t/workaround-for-synchronousonlyoperation-when-using-authenticationmiddleware/22889) (post #5): "I'm going to assume — but guessing — it's this issue that you're hitting: https://code.djangoproject.com/ticket/31920 Unwrapping request.user in an async view… So is there a workaround? — You can use one of the examples from the report (Cases 4 and 5 from the description there.)"

### `django-practice-sync-orm-reached-from-async`

Rerail's own rule, not a port. It reports, in an `async def`'s own frame (not a nested def or lambda) and outside test
code: (1) a blocking query written there: a blocking QuerySet method (`get`, `create`, `count`, `exists`, `first`,
`update`, `delete`, `iterator`, … on a chain from a manager), a sync `for` over a queryset, `with transaction.atomic`,
or `save()`/`delete()`/`refresh_from_db()` on a model instance (`self` in a model method, a name assigned a
fetched or constructed instance, or a receiver the type program proves is a model); (2) a call the cross-file call
graph resolves, with no unknown target, to a sync function of the run that runs such a query in its own frame or
reaches one through further sync functions. A call written in a nested def or lambda of those functions runs later
and is not followed. Where the call graph has no target for `obj.m()` on a model instance whose class the rule knows,
it follows the model's method `m`. `self.m()` in a class goes to that class's `m`; a subclass whose override queries
is reported at `class Sub` (inherited `async def`). The message names the callee as written
(`request.user.has_perm`), so `sync_to_async(...)` wraps something callable.

It leaves alone: `await sync_to_async(f)()`, `asyncio.to_thread(f)`, `loop.run_in_executor(None, f)`; defs decorated
`@sync_to_async`/`@database_sync_to_async`; chains through an async def (calling one only makes a coroutine); calls
of unknown target; the async ORM (`aget()`, `acount()`, `async for`, `asave()`); test files.

**Gaps:** chains with a link of unknown target (receivers of unknown type, getattr dispatch); querysets from custom
managers' methods and related managers; the chain is followed through every branch, so a call is reported when any
branch of the reached code queries. All 12 matches are WeblateOrg/weblate permission checks after
`await user.aprepare_permissions()` (`weblate/trans/views/labels.py:84`, `weblate/utils/views.py:459`, `:467`,
`:471`, `:572`, `weblate/vcs/views.py:109`, `:196`, `:443`, `weblate/accounts/views.py:2417`, …): `has_perm` reaches
`Project.objects.in_bulk(...)` (`weblate/auth/permissions.py:192`) on the language-scope branch of
`check_permission`, which these calls (on a Project, Component or Workspace) do not take, and the rest reads caches
the coroutine primes first. Real in the sense above, harmless in context. No async def in the corpus writes a sync
query itself.

**Sources:**
- Django docs, [Async safety](https://docs.djangoproject.com/en/stable/topics/async/#async-safety): "Note that you don't have to be inside an async function directly to have this error occur. If you have called a sync function directly from an async function, without using :func:`sync_to_async` or similar, then it can also occur. This is because your code is still running in a thread with an active event loop, even though it may not be declared as async code."
- Channels docs, [Database Access](https://channels.readthedocs.io/en/latest/topics/databases.html): "If you are writing asynchronous code, however, you will need to call database methods in a safe, synchronous context, using ``database_sync_to_async`` or by using the asynchronous methods prefixed with ``a`` like ``Model.objects.aget()``."

Why Log: the call graph follows every branch of the reached code, and every corpus match reaches its query only on a
branch its arguments do not take, behind caches the coroutine fills first (Weblate's `aprepare_permissions`); one rule
cannot tell a cached path from a querying one, so offloading is often unneeded there.

## The corpus

31 open-source Django apps, cloned shallow at these commits. PostHog/posthog
(`37c895112cd9830ba6161037c34c431a2a6d7d0c`, MIT) was left out: with 29,885 Python files, no run finished. In
netbox-community/netbox, `netbox/extras/data/un_locode.py` (6.2 MB, one tuple of about 111k string pairs, no Django
code) was removed from the working tree, because of an engine issue described in [ENGINE-BUGS.md](ENGINE-BUGS.md)
(bug 2).

| Repository | Commit | Licence |
|---|---|---|
| [allegro/ralph](https://github.com/allegro/ralph) | `b80ca56bf00302a6a5afc7e83e6b3a11155eba37` | Apache-2.0 |
| [ansible/awx](https://github.com/ansible/awx) | `b970bf20255b6782fc4daccf81e7ee6d4a4dd174` | Apache-2.0 |
| [archesproject/arches](https://github.com/archesproject/arches) | `95805146622eb9ed957cd614390b67c6bb9c6aab` | AGPL-3.0 |
| [cvat-ai/cvat](https://github.com/cvat-ai/cvat) | `873008dead495e215a51dbea5c705bc19b61016d` | MIT |
| [DefectDojo/django-DefectDojo](https://github.com/DefectDojo/django-DefectDojo) | `4b6bac70af552f0d09c5285377c1d6c03b051568` | BSD-3-Clause |
| [django-oscar/django-oscar](https://github.com/django-oscar/django-oscar) | `a5ed3c9c17d4cf6ee18aa9795b493172416ab68b` | BSD-3-Clause |
| [django/djangoproject.com](https://github.com/django/djangoproject.com) | `83a027bb6aac5bb36b9ef48dad8cb4ff45a2e84f` | BSD-3-Clause |
| [goauthentik/authentik](https://github.com/goauthentik/authentik) | `937d44569da711539555bc1090c6d368a6acbc69` | MIT (authentik/enterprise/ under a separate EE licence; website/ CC BY-SA 4.0) |
| [healthchecks/healthchecks](https://github.com/healthchecks/healthchecks) | `255075ce65ee0997fd9d3b8dd04e4829a87fad5d` | BSD-3-Clause |
| [HumanSignal/label-studio](https://github.com/HumanSignal/label-studio) | `0d5ecf589ce57dc9bccb30b8afb9f26a02efee60` | Apache-2.0 |
| [inventree/InvenTree](https://github.com/inventree/InvenTree) | `7e57196d76f4560caebbff9508ea5e230341fc08` | MIT |
| [kiwitcms/Kiwi](https://github.com/kiwitcms/Kiwi) | `3728b829ba6c2ce5bd926b860133d54b5d1edb5b` | GPL-2.0 |
| [LibrePhotos/librephotos](https://github.com/LibrePhotos/librephotos) | `5a81eb3d66ea3b552e5325ced4e1e17b8b5b4267` | MIT |
| [makeplane/plane](https://github.com/makeplane/plane) | `9cefae892c4c28ee60400abca12e4ababe74cf96` | AGPL-3.0 |
| [mathesar-foundation/mathesar](https://github.com/mathesar-foundation/mathesar) | `3833c3ae5714f164dd6870f4d28865e78d60a4cc` | GPL-3.0 |
| [mozilla/addons-server](https://github.com/mozilla/addons-server) | `bba7d9c89ecba317bc465c3dadd82ec964d3c1e9` | BSD-3-Clause |
| [mozilla/pontoon](https://github.com/mozilla/pontoon) | `3370f51ed39cd3822b921838b98baea7035b07fc` | BSD-3-Clause |
| [nautobot/nautobot](https://github.com/nautobot/nautobot) | `7dd9ae667ce569bf6bbf4443962b244abf1ecf3c` | Apache-2.0 |
| [netbox-community/netbox](https://github.com/netbox-community/netbox) | `cf6d3f0319fade2fa0f1c51155f1c11330e58ff3` | Apache-2.0 |
| [openedx/edx-platform](https://github.com/openedx/edx-platform) | `d3a86602dcf70bbf4f7925727bcc50ff10e51109` | AGPL-3.0 |
| [paperless-ngx/paperless-ngx](https://github.com/paperless-ngx/paperless-ngx) | `a1d76f3a19c9bf7067491de6f0a12672676ae9f1` | GPL-3.0 |
| [pretix/pretix](https://github.com/pretix/pretix) | `137059dd4e7a7f4643199323b69a85824bffc28e` | AGPL-3.0 (with additional terms) |
| [python-discord/site](https://github.com/python-discord/site) | `d513a424afe47a30be0828ee9d3e865e4d88ebc5` | MIT |
| [rafalp/Misago](https://github.com/rafalp/Misago) | `4a37d19d3d9f74064c75cdda6856350b2daea5df` | GPL-2.0 |
| [readthedocs/readthedocs.org](https://github.com/readthedocs/readthedocs.org) | `516881b08da007391b560db8f2f64d79f8c3e00c` | MIT |
| [saleor/saleor](https://github.com/saleor/saleor) | `6f714faf25f3dbdd6ee2cbc2427b40cc890990c5` | BSD-3-Clause |
| [sissbruecker/linkding](https://github.com/sissbruecker/linkding) | `27b7303baf41bb28babc610ac8eaa486e1ddfab5` | MIT |
| [taigaio/taiga-back](https://github.com/taigaio/taiga-back) | `8ab6a4e3b5f6fbbfcbf108a6449011f07b7308aa` | MPL-2.0 |
| [umap-project/umap](https://github.com/umap-project/umap) | `8c20f55ba00e0d63bd46a013e46e7f812a44e9ca` | AGPL-3.0 |
| [WeblateOrg/weblate](https://github.com/WeblateOrg/weblate) | `0c855eecb8f9a43cec69d0f52b452f1192f17714` | GPL-3.0 |
| [zulip/zulip](https://github.com/zulip/zulip) | `39bd5f2f40a647af06b1cc0267cddd34f88ed2e9` | Apache-2.0 |

## Files

| Path | What it is |
|---|---|
| `django-practice-<id>.rerail` | The 27 rules, written by hand. Each opens with a comment on what it matches and leaves alone, with an example of each |
| `cases/<id>/` | Each rule's cases, a small repository: a line ending in `# caught` must be reported, no other line may be (the rule file's `.test cases`) |
| [`../../lib/python/django.rerail`](../../lib/python/django.rerail) | `python/django`, the shared vocabulary: models and their fields, querysets, transactions, views, test and migration files, settings |
| [`NOTES-django.md`](NOTES-django.md) | The batch notes: dropped candidates, open library proposals, overlaps with other tools |
| [`ENGINE-BUGS.md`](ENGINE-BUGS.md) | Engine and library bugs found while writing and measuring these rules |
