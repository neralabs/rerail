# django-practice-clean-field-returns-value

Invariant: a form's `clean_<field>()` returns the field's cleaned value on every path that does not raise.

## Sources

- Django docs, "Form and field validation" (docs/ref/forms/validation.txt),
  https://docs.djangoproject.com/en/dev/ref/forms/validation/ :
  "If no ``ValidationError`` is raised, the method should return the cleaned (normalized) data as a Python
  object." and, on `clean_<fieldname>()`: "The return value of this method replaces the existing value in
  ``cleaned_data``, so it must be the field's value from ``cleaned_data`` (even if this method didn't change it) or
  a new cleaned value." Its example ends: "# Always return a value to use as the new cleaned data, even if
  # this method didn't change it."
- Django source (django/forms/forms.py, main, `_clean_fields`): `value = getattr(self, "clean_%s" % name)()` then
  `self.cleaned_data[name] = value`, unconditionally.
- MDN Learn, "Django Tutorial Part 9: Working with forms",
  https://developer.mozilla.org/en-US/docs/Learn_web_development/Extensions/Server-side/Django/Forms :
  "# Remember to always return the cleaned data." and "we return this data whether or not we change it at the end
  of the function."

## Linter coverage (checked)

- pylint-django (checkers/*.py, main): its forms checker has only W5104 modelform-uses-exclude.
- flake8-django DJ01–DJ13 and Ruff's DJ port: no form-method checks.
- Generic overlap: Ruff RET503 (implicit-return, not in Ruff's default set) and pylint R1710
  (inconsistent-return-statements) flag a function that returns a value on some paths and falls off on others.
  Neither flags a clean_<field> with no `return <value>` at all (the commonest shape below: validate-and-raise
  then fall off), nor knows that Django stores the result. Ruff RET502 flags a bare `return` only when the
  function also returns a value.
- Bandit, django-upgrade, django-migration-linter, Django system checks: nothing on form methods.

## What it matches

`def clean_<x>(self)` (no other parameters) directly in a form class — a subclass, through the run's classes in any
file, of a django.forms `*Form` class or another package's `<pkg>.forms.*Form` class outside the run — when the body
can fall off its end (reported at the def name) and at each bare `return` (reported at the return). Flow: the last
statement of each block; if/elif/else, try/except/else/finally, with, `while True` without break and match are
followed; return and raise end a path. Left alone: `return None` (written on purpose), test files, helpers (the
class calls `self.clean_<x>()` itself, or defines a method/property x). Cross-file fact: the form ancestry through
workspace bases in other modules.

Misses: a form whose base is an attribute (`UserAdmin.form`), modelform_factory forms, a call that never returns
(`sys.exit`) as the last statement (would be reported).

## Corpus

31 repos (PostHog skipped by the corpus's SKIP list), engine /tmp/djxf/rerail-engine. 21 matches in 6 repos
(18 methods), all 21 read: 21 real, precision 100%. A match is real when the method can hand Django None for the
field; some are harmless in context (Weblate's captcha fields, whose value nothing reads).

- WeblateOrg__weblate:weblate/accounts/forms.py:700 (`clean_captcha() -> None`: bare `return`, then falls off)
- mozilla__addons-server:src/olympia/devhub/forms.py:557 (`clean_slug` validates and falls off: slug becomes None)
- nautobot__nautobot:nautobot/ipam/forms.py:534 (`clean_namespace` pops the key, then Django writes it back as None)
- also django-oscar customer/forms.py:81 (`clean_redirect_url` returns None for a disallowed URL), openedx (4),
  pretix (6).

Only 8 of the 18 methods also return a value somewhere (the part Ruff RET503 / pylint R1710 could see); 10 never
return a value, which no linter flags.

False-positive classes found and fixed (negative cases added):
- readthedocs projects/forms.py (6 matches): `clean_prevalidation()`, a hook its base `PrevalidatedForm` calls itself
  (`self.clean_prevalidation()`), not a field. Fix: a clean_<x> that the class or a base in the run calls on `self`
  is a helper (cfr_helper).
- Misago posting/forms/post.py `clean_attachments_limit` (seen in an AST cross-check before the first run): a helper
  called from clean(), and `attachments_limit` is a property of the form. Same fix, plus "x is a method/property of
  the class".

Recall cross-check: an AST script (classes with `Form` in a base's text, the same flow) found the same methods plus
ralph accounts/admin.py:85, whose base `UserAdmin.form` is an attribute the engine does not resolve.

Performance: a first version computed reachability over every block in the run through python/ast's body_stmt /
body_len; on netbox that ran out to 24 GB and >15 min. The flow is now demand-scoped to clean_<x> bodies
(cfr_block / cfr_stmt) and the last statement is found with a `max` aggregate; netbox runs in about a minute.

## Mode

steer: 21/21 real.

## Helpers worth a library

- cfr_form / cfr_form_base: "class c is a Django form, through the run's classes" (a dj_form_class(c) beside
  dj_view_class in python/django). django-practice-modelform-clean-calls-super's mcs_model_form is its ModelForm
  subset.
- cfr_open / cfr_s_open: "this block can complete normally" (return/raise end a path; if/try/with/`while True`/match
  followed), demand-scoped by a seed relation. Fits python/flow beside always_raises; RET503-style rules could use it.
- python/ast's body_stmt counts earlier siblings per statement over every block and module in the run: quadratic in
  long modules and costly on netbox; a demand-bound or `max`-based "last statement" helper would avoid it.

