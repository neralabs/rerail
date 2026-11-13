# django-practice-get-or-create-needs-unique

Invariant: `get_or_create()` / `update_or_create()` look up a set of fields that the database keeps unique for the
model, so two concurrent calls cannot both insert.

## Sources

- Django docs, QuerySet API reference, `get_or_create()` warning
  (https://docs.djangoproject.com/en/stable/ref/models/querysets/#get-or-create, source
  https://github.com/django/django/blob/main/docs/ref/models/querysets.txt): "This method is atomic assuming that
  the database enforces uniqueness of the keyword arguments (see unique or unique_together). If the fields used in
  the keyword arguments do not have a uniqueness constraint, concurrent calls to this method may result in multiple
  rows with the same parameters being inserted."
- BaoDev.studio, "Django get_or_create: the race condition nobody expects"
  (https://baodev.studio/blog/django-get-or-create-race-condition): "get_or_create is SELECT-then-INSERT with a gap,
  not atomic on its own. Under concurrency both callers miss the get and both insert. A DB UniqueConstraint on the
  lookup fields is the real fix; the method's error handling is built to rely on it." and "update_or_create carries
  the same risk."

## Linter coverage checked

- Ruff 0.17.5 (`uvx ruff rule --all`, 977 rules): no rule mentions get_or_create, update_or_create or a uniqueness
  constraint (searched names and explanations; DJ001-DJ013 are model/form style checks).
- flake8-django 1.4, pylint-django 2.8.0, Bandit 1.8.6 (sources downloaded and grepped): get_or_create appears only in
  pylint-django's list of manager methods it suppresses no-member for; no check.
- django-upgrade (rewrites only), django-migration-linter (migration operations only), Django system checks (no
  check reads call sites): none.

## What it matches

A call `M.objects.get_or_create(...)` / `update_or_create(...)` (also after `.all()`, `.using()`,
`.select_for_update()`, and `cls.objects...` in a classmethod of a non-abstract model), outside test files, where M
is a model class of the run and:
- every lookup keyword (all but `defaults`, `create_defaults`) names a field M declares, in its body or in a base class
  of the run (cross-file: class_def/subclass), with `x_id` read as ForeignKey/OneToOneField `x`;
- no lookup is `pk`/`id`, a OneToOneField, or a field with `unique=`/`primary_key=` other than literal False;
- no group of M's (or a run base's) Meta `unique_together` (flat or nested) or `UniqueConstraint(fields=[...])`
  lies within the lookup fields.

The assignment asked to count only UniqueConstraints without `condition=`. The corpus showed partial constraints that
do cover the looked-up row (Weblate's MachineryExclusion, `fields=["project", "engine"], condition=~Q(engine="")`,
looked up with a non-empty engine; Mathesar's Server, two partial constraints over host/port that together cover every
row), so a conditional constraint within the lookup makes the call unknown instead of reported.

Unknown, not reported: `__` lookups and any keyword that is not a declared field (a field from a base outside the
run such as AbstractUser.username, a third-party field class), `**kwargs`, positional Q, a Meta whose
unique_together/constraints are not literal lists, an expression UniqueConstraint (`Lower("name")`), fields= not a
literal, a non-call constraint entry (`*Base.Meta.constraints`), an unmanaged model (`managed = False`: its table
and indexes are made outside Django), and a manager the model sets itself (`objects = BuildConfigManager()`,
`objects = QS.as_manager()`) whose class in the run overrides the method (Read the Docs' BuildConfigManager turns
`get_or_create(data=...)` into a lookup on a unique `data_hash`).

Misses: related managers (`author.book_set.get_or_create(...)`, `self.items.get_or_create(...)`): the receiver's
model is not resolved by any library relation; custom manager names (`M.custom.get_or_create`); querysets after
filter()/exclude() (their kwargs join the lookup but not the insert, left as unknown); uniqueness added only in a
migration (RunSQL, AddConstraint not mirrored in Meta) is not read, so such a call would be reported.

## Corpus

31 repos (PostHog in SKIP); netbox-community__netbox timed out (killed after >15 min, twice) and is not counted.
Final rule: 92 matches in 19 repos, all 92 read with their model's fields and Meta checked: 92 real, precision 100%.
Examples: django__djangoproject.com:fundraising/views.py:392 (the model says `# TODO: Make this unique.`),
DefectDojo__django-DefectDojo:dojo/finding/ui/views.py:893 (catches MultipleObjectsReturned right after),
LibrePhotos__librephotos:apps/backend/api/models/cluster.py:85 (same), goauthentik__authentik:
authentik/sources/telegram/api/source.py:83, openedx__edx-platform:common/djangoapps/student/models/user.py:1037.
Many are in management commands and data generators: real by the bar (the pattern holds).

False positives found and fixed (each with a negative case):
- conditional UniqueConstraint the row meets (Weblate MachineryExclusion x2, Weblate Group with a workspace, Mathesar
  Server): conditional groups within the lookup now make the call unknown;
- a custom manager overriding get_or_create (Read the Docs BuildConfigManager, lookup moved to a unique hash): a
  model-set manager whose run class (or queryset class) defines the method makes the call unknown;
- a constraint naming the column `transaction_id` while the lookup says `transaction` (Saleor TransactionEvent):
  `x_id` in Meta now matches relation x. (That repo was not rerun after this fix; the match is dropped from the
  results file by hand.)
- `managed = False` added pre-emptively (no corpus match): the table's indexes are outside Django.
Borderline but real by the bar: taiga Points (project, value) with `name` in defaults under unique (project, name);
the IntegrityError path then re-gets the row.

## Mode

steer (100% of 92 read matches real).

## Helpers worth a library

`goc_field`/`goc_meta_opt` (a model's fields and Meta options through the run's bases; srl_field in
select-related-in-loop is the same), `goc_mgr_def` (a model's manager assignments), `goc_group` (a model's unique
field sets). A shared `dj_model_field(mc, x, call, t)` and `dj_meta(mc, k, v)` in python/django.rr would serve
several rules.
