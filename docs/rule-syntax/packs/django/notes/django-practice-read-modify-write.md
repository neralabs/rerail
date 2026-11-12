# django-practice-read-modify-write

Invariant: a numeric model field is changed relative to its stored value in the database (`F()`, `update()`), or
under a row lock (`select_for_update()`), never read into Python, changed and saved back.

## Sources

- Django docs, Query Expressions, "Avoiding race conditions using F()"
  (https://docs.djangoproject.com/en/stable/ref/models/expressions/#avoiding-race-conditions-using-f, source
  https://github.com/django/django/blob/main/docs/ref/models/expressions.txt): "If two Python threads execute the
  code in the first example above, one thread could retrieve, increment, and save a field's value after the other has
  retrieved it from the database. The value that the second thread saves will be based on the original value; the
  work of the first thread will be lost."
- freeCodeCamp, "How to Prevent Race Conditions in Django"
  (https://www.freecodecamp.org/news/how-to-prevent-race-conditions-in-django): "Imagine two requests try to increase
  a post’s view count from 100. Both read 100 before either saves a change. Each adds one and writes 101. The counter
  should have reached 102, so one update disappears. This is a lost update, …".

## Linter coverage checked

- Ruff 0.17.5 (`uvx ruff rule --all`, 977 rules): no rule mentions F() expressions, select_for_update, lost updates or
  race conditions on model fields.
- flake8-django 1.4, pylint-django 2.8.0, Bandit 1.8.6 (sources downloaded and grepped for `F(`,
  `select_for_update`, `save`): no such check.
- django-upgrade, django-migration-linter, Django system checks: none read function bodies for this.

## What it matches

In one function (outside test files): `o.x += n`, `o.x -= n`, `o.x = o.x ± n` or `o.x = n + o.x`, and later in the
same function (not a nested def) `o.save(...)`, where o is a plain name, x is a numeric field (Integer, SmallInteger,
BigInteger, PositiveInteger/SmallInteger/BigInteger, Decimal, Float, Duration) that o's model declares in its body or
a base class of the run (cross-file), and o's model is known:
- `self` in a method of model M (dj_model_method);
- `o = M.objects....get()/first()/last()/latest()/earliest()`, `o, created = M.objects...get_or_create(...)` (or
  update_or_create), `o = get_object_or_404(M or M.objects..., ...)`, `for o in M.objects...:` in the function, M
  resolved across files.
Not reported: a function that calls `select_for_update()` anywhere; `o.x = F("x") + n`; a fresh `o = M(...)` (an
insert, no lost update).

type_of (python/types) was tried and dropped: on the cases it types only `self` and constructor-bound names
(`w = Wallet(...)`), which the rule already covers or deliberately excludes; querysets' results are not typed.

Misses: o as an attribute chain (`self.object.hits += 1` in a DetailView), a parameter whose model is known only at
call sites, a row locked by the caller (a model method called under select_for_update() is still reported: the
method itself holds the pattern), increments followed by `save()` in a helper.

## Corpus

31 repos (PostHog in SKIP); netbox-community__netbox timed out (killed) and is not counted. 18 matches in 8 repos,
all read: 18 real, precision 100%. Model methods (`self.f += n; self.save()`) are 14 of them and are real by the bar:
the method holds the pattern whatever the caller locks.
Examples: django-oscar__django-oscar:src/oscar/apps/voucher/abstract_models.py:255 (voucher num_orders),
healthchecks__healthchecks:hc/api/models.py:1454 (its own comment: "Race condition: two concurrent authorize calls can
overwrite each other's changes"), allegro__ralph:src/ralph/accessories/models.py:163 (after get_or_create),
openedx__edx-platform:common/djangoapps/student/models/user.py:1038 (login failure counter),
inventree__InvenTree:src/backend/InvenTree/stock/models.py:2294 (stock quantity).
No false positive found. Recall is low by design (o must be a plain name with a known model; `self.object.f += 1`
in views is missed); pretix and saleor, which lock with select_for_update, have none.
The get_or_create tuple binding was added after the first pass (it found ralph and healthchecks).

## Mode

steer (100% of 18 read matches real).

## Helpers worth a library

`rmw_qs_model` / `rmw_root` (a queryset chain's model, the same as srl_model in select-related-in-loop),
`rmw_model` (the model of a name bound from a queryset in a function), `rmw_numeric` (numeric fields through bases).
