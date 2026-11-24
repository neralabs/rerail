# @neralabs/rerail-pack-django

Every notable change to this rule pack, latest first. Each release says how many rules the pack ships, then lists its changes, rule by rule, under Added, Changed, Deprecated, Removed, Fixed and Security, as in [Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/); releases from before the pack's notes took that format list them without those headings. There are two differences: a heading reads `## <version> (<date>)` rather than `## [<version>] - <date>`, and changes waiting for the next release are [changesets](../../.changeset/README.md), so there is no `## Unreleased`.

Versions are numbered as in [Semantic Versioning](https://semver.org/spec/v2.0.0.html), which promises nothing about compatibility below 1.0.0. Below 1.0.0 a version's minor follows the Rerail line it needs (0.2.x needs Rerail 0.2 or newer), and a patch can add, change or remove rules, so read the notes before you update.

## 0.2.0 (2026-11-24)

27 rules, 25 of them on by default.

### Added

- `django-practice-async-request-user` (starts in Steer): Inside async def, get the user with await request.auser() instead of reading request.user.
- `django-practice-atomic-around-caught-db-error` (starts in Steer): Inside transaction.atomic, catch a database error only around a nested atomic block, so the error rolls back that savepoint and not the whole transaction.
- `django-practice-bulk-write-skips-hooks` (starts in Steer): A queryset bulk write skips the model's save()/delete() overrides and its pre_save/post_save receivers; call them or do their work.
- `django-practice-clean-field-returns-value` (starts in Steer): A Django form's clean_<field>() must return the field's cleaned value on every path that does not raise.
- `django-practice-count-not-len` (starts in Steer): Use QuerySet.count() instead of len() on a queryset whose rows the code does not otherwise use.
- `django-practice-default-called-once` (starts in Steer): Pass a callable, not the result of a call, as a model field's default when the value must differ per row.
- `django-practice-dispatch-in-atomic-request` (starts in Steer): With ATOMIC_REQUESTS on, every view runs in a transaction: enqueue tasks and send mail from views through transaction.on_commit, so they run only after the request's data is committed.
- `django-practice-enqueue-on-commit` (starts in Steer): Enqueue background tasks and send mail from transaction.on_commit, not inside transaction.atomic, so they run only after the data they depend on is committed.
- `django-practice-exists-for-existence` (starts in Steer): Use QuerySet.exists() to test whether a queryset has rows, not count(), len() or the queryset's truth.
- `django-practice-fk-id-not-relation` (starts in Steer): Read a foreign key's value from <field>_id instead of loading the related object to take its id or pk.
- `django-practice-function-decorator-on-method` (starts in Steer): Apply Django's view decorators to a class's methods through @method_decorator(...), not directly.
- `django-practice-get-or-create-needs-unique` (starts in Steer): Give get_or_create() and update_or_create() a lookup the database keeps unique, or concurrent calls insert duplicate rows.
- `django-practice-lazy-translation-at-import` (starts in Steer): Use gettext_lazy, not gettext, for text translated when the module is imported: class bodies, field options, module constants and default arguments.
- `django-practice-middleware-ignores-streaming` (starts in Steer): In middleware, check response.streaming before reading or writing response.content.
- `django-practice-model-init-override` (starts in Steer): Do not read model fields in, or change the signature of, a Django model's __init__; record loaded values in from_db or a post_init receiver, and build new objects in a classmethod or manager method.
- `django-practice-modelform-clean-calls-super` (starts in Steer): A clean() override on a Django ModelForm or model formset must call the parent class's clean().
- `django-practice-read-modify-write` (starts in Steer): Update a counter in the database with F() (or under select_for_update()), not by reading it into Python, changing it and saving it back.
- `django-practice-save-ignores-update-fields` (starts in Steer): When a model's save() override sets a field before calling super().save(), add that field to update_fields when the caller gave one.
- `django-practice-select-related-in-loop` (starts in Steer): Load the relations a loop reads with select_related() or prefetch_related() on the queryset it iterates.
- `django-practice-settings-module-import` (starts in Steer): Read settings through django.conf.settings, not by importing the project's settings module.
- `django-practice-settings-runtime-assignment` (starts in Steer): Do not change django.conf.settings at runtime; set values in the settings file, and use override_settings in tests.
- `django-practice-task-arg-model-instance` (starts in Steer): Pass a model's primary key to a Celery task and re-fetch the row in the task, never the model instance itself.
- `django-practice-bulk-not-loop` (starts in Log): Create, update or fetch model rows in bulk (bulk_create, bulk_update, in_bulk) rather than with one query per loop iteration.
- `django-practice-select-for-update-outside-atomic` (starts in Log): Evaluate a select_for_update() queryset inside transaction.atomic(), in the function or in every caller chain that reaches it.
- `django-practice-sync-orm-reached-from-async` (starts in Log): An async def must not call a sync function that queries the database; call it through sync_to_async.
- `django-practice-get-handler-writes` (starts off): A view that answers GET or HEAD must not write to the database: safe methods are side-effect free and CSRF protection skips them.
- `django-practice-host-header-without-get-host` (starts off): Read the request's host with request.get_host(), which checks it against ALLOWED_HOSTS, not from the Host header.
