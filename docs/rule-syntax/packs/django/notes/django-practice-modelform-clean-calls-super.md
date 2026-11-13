# django-practice-modelform-clean-calls-super

Invariant: a `clean()` override on a ModelForm or model formset calls its parent's `clean()`.

## Sources

- Django docs, "Overriding the clean() method" (docs/topics/forms/modelforms.txt),
  https://docs.djangoproject.com/en/dev/topics/forms/modelforms/#overriding-the-clean-method :
  "The ``ModelForm.clean()`` method sets flags that make the model validation step validate the uniqueness of model
  fields that are marked as ``unique``, ``unique_together`` or ``unique_for_date|month|year``, as well as
  constraints. If you would like to override the ``clean()`` method and maintain this validation, you must call the
  parent class's ``clean()`` method."
- Same page, "Overriding clean() on a ModelFormSet",
  https://docs.djangoproject.com/en/dev/topics/forms/modelforms/#overriding-clean-on-a-modelformset :
  "If you want to override the ``clean()`` method on a ``ModelFormSet`` and maintain this validation, you must call
  the parent class's ``clean`` method"
- Django source (django/forms/models.py, main): `BaseModelForm.clean` is only `self._validate_unique = True;
  self._validate_constraints = True; return self.cleaned_data`, and `_post_clean` runs `validate_unique()` /
  `validate_constraints()` only when those flags are set; `BaseModelFormSet.clean` is `self.validate_unique()`.
- Stack Overflow, accepted answer by Yuji 'Tomita' Tomita to "Django unique model fields validation in form",
  https://stackoverflow.com/questions/31150929/django-unique-model-fields-validation-in-form :
  "Running the superclass `clean` method should take care of model uniqueness checks given a `ModelForm`." (The
  question's form overrides `clean()` without super and its unique check never fires.)

## Linter coverage (checked)

- pylint-django (checkers/*.py, main): forms checker has only W5104 modelform-uses-exclude; nothing on clean().
- pylint core: W0231 super-init-not-called covers `__init__` only; W0246 useless-parent-delegation is the opposite.
- flake8-django (DJ01–DJ13) and Ruff's DJ port: ModelForm checks are DJ06 exclude / DJ07 `__all__` only.
- Bandit, django-upgrade, django-migration-linter, Django system checks: nothing on form methods.

## What it matches

`def clean(self)` directly in a class that derives — through the run's classes, across files — from ModelForm,
BaseModelForm, BaseModelFormSet, BaseInlineFormSet, BaseGenericInlineFormSet, auth's UserCreationForm /
BaseUserCreationForm / UserChangeForm, flatpages' FlatpageForm, or wagtail's WagtailAdminModelForm /
WagtailAdminPageForm, or an outside class named `…ModelForm` / `…ModelFormSet` (i18nfield's I18nModelForm, parler's
TranslatableModelForm) or `django.…InlineFormSet` (re-exports), when nothing in its own body is `super().clean(...)`, `super(C, self).clean(...)` or
`X.clean(self, ...)`. Test files are skipped. Cross-file fact: the class's ModelForm ancestry through workspace bases
in other modules.

Misses: a ModelForm built by modelform_factory; a mixin's clean() mixed into a ModelForm; third-party ModelForm
bases named otherwise. A parent call only inside a def nested in clean() does not count (zero-argument super() fails
there anyway).

## Corpus

31 repos (PostHog skipped by the corpus's SKIP list), engine /tmp/djxf/rerail-engine. 30 matches in 9 repos, all
30 read: 30 real, precision 100%. Real = the override skips the parent's clean(); some models involved may have no
unique fields, which makes the skip harmless today but not tomorrow.

- mozilla__addons-server:src/olympia/devhub/forms.py:1407 (a BaseModelFormSet whose clean() is only
  `if any(self.errors): return`: formset uniqueness never checked)
- django-oscar__django-oscar:src/oscar/apps/customer/forms.py:393 (ModelForm.clean re-implements a duplicate check
  with a query instead of calling super)
- netbox-community__netbox:netbox/users/forms/model_forms.py:262 (UserForm.clean: password checks, no super)
- also DefectDojo (6), Weblate (2, through workspace bases), arches (UserChangeForm subclass, `def clean(self):
  pass`), ralph (BaseInlineFormSet re-exported by django.contrib.admin.options), pretix (6, one an i18nfield
  I18nModelForm), openedx (1).

Iteration: the first run missed ralph reports/forms.py:9 (`from django.contrib.admin.options import
BaseInlineFormSet`, a re-export) and would have missed pretix's I18nModelForm; fixed by accepting outside classes
named `…ModelForm` / `…ModelFormSet`, and `django.…InlineFormSet` (cases added). No false positives found.
Recall cross-check: an AST script over the corpus (classes with ModelForm/ModelFormSet/InlineFormSet in a base's
text, clean() without `.clean(`) found 27, all among the 30; the 3 extra come through workspace bases or
UserChangeForm.

Performance: the parent-call test starts from `.clean(...)` calls (nearest def), not from every def's descendants.

## Mode

steer: 30/30 real.

## Helpers worth a library

- mcs_model_form(c) / mcs_base: "class c is a ModelForm or model formset, through the run's classes" — a
  dj_model_form_class(c) in python/django, alongside a general dj_form_class (see
  django-practice-clean-field-returns-value's cfr_form).

