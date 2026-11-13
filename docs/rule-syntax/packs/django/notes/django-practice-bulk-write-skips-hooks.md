# django-practice-bulk-write-skips-hooks

Invariant: a queryset bulk write on model M does not silently skip behaviour M attaches to save/delete: `update()`,
`bulk_create()`, `bulk_update()` skip `save()` overrides and pre_save/post_save receivers; queryset `delete()` skips
`delete()` overrides (it does send pre_delete/post_delete).

## Sources

- Django docs, Models, "Overridden model methods are not called on bulk operations"
  (https://docs.djangoproject.com/en/5.2/topics/db/models/#overriding-predefined-model-methods): "Note that the
  delete() method for an object is not necessarily called when deleting objects in bulk using a QuerySet or as a
  result of a cascading delete. To ensure customized delete logic gets executed, you can use pre_delete and/or
  post_delete signals. Unfortunately, there isn’t a workaround when creating or updating objects in bulk, since none
  of save(), pre_save, and post_save are called."
- Django docs, QuerySet API (https://docs.djangoproject.com/en/5.2/ref/models/querysets/):
  bulk_create: "The model’s save() method will not be called, and the pre_save and post_save signals will not be
  sent."; update(): "Finally, realize that update() does an update at the SQL level and, thus, does not call any
  save() methods on your models, nor does it emit the pre_save or post_save signals (which are a consequence of
  calling Model.save())."; delete(): "The delete() method does a bulk delete and does not call any delete() methods
  on your models. It does, however, emit the pre_delete and post_delete signals for all deleted objects (including
  cascaded deletions)."
- djangoproject.in, "Django bulk_create and bulk_update: Performance and the …"
  (https://djangoproject.in/blog/django-bulk-operations): "But bulk_create bypasses Django's save machinery
  entirely: no pre_save , no post_save , no auto_now update for existing rows. If your application depends on those
  signals for correctness — not just performance — bulk_create introduces silent data inconsistency." and "Model
  save() method overrides: Any custom logic in def save() is skipped"
- Stack Overflow, "How can i use signals in django bulk create", accepted answer
  (https://stackoverflow.com/a/30632974): "As mentioned bulk_create does not trigger these signals"

## Linters checked

Ruff (DJ, DTZ, S, ASYNC; rule list at https://docs.astral.sh/ruff/rules/), flake8-django (same DJ set),
pylint-django (README features), Bandit, django-upgrade, django-migration-linter, Django system checks: none relates a
bulk write to a model's save()/delete() override or its receivers.

## What it matches

A queryset call `update` / `bulk_create` / `bulk_update` / `delete` on a chain rooted at `M.objects` (or
`_default_manager` / `_base_manager`), M a model class of the run resolved across files (select-related-in-loop's
srl_root/srl_model). Hooks, cross-file: M's `save()`/`delete()` defined in M or any class it inherits from in the run
(abstract bases, mixins), unless its body is only `super().<same>(…)` (docstring aside); a pre_save/post_save
receiver for `sender=M` anywhere outside tests: `@receiver(sig, sender=M)` (sig maybe a list/tuple),
`sig.connect(fn, sender=M)`, signals resolved through sym_of to `django.db.models.signals`. The message lists every
skipped hook with path:line. Skipped: calls inside M's own save()/delete() or inside one of M's receivers (they write
in bulk to avoid recursion), migrations, tests.

Misses: querysets not rooted at a manager name (related managers, custom managers, `self.get_queryset()`, a queryset
held in a variable), string senders (`sender="app.Model"`), save overrides in third-party bases outside the run.

## Corpus

Full corpus (31 repos; PostHog is in SKIP): 329 matches in 19 repos (update 171, bulk_create 109, bulk_update 35,
delete 14); netbox-community__netbox timed out (900 s in run.py, then a private run stopped after 36 min) and is not judged. Read: a spread sample of 110 (4–6 per repo, every repo with matches, seeds 1 and 7), plus
every hook body behind the most frequent messages. Real: 110 of 110 (precision 100%). Every match is a bulk write
on a model of the run whose save()/delete() override or pre/post_save receiver the write skips; many are deliberate
(comments such as "QuerySet.update() bypasses Django signals — intentional", "Using update() to avoid circular
Term.save() call"), which the bar counts as real: the pattern is in the file.

Examples (real):
- sissbruecker__linkding:bookmarks/services/bookmarks.py:97 (`Bookmark…update()` skips Bookmark.save(), which
  recomputes `url_normalized`)
- pretix__pretix:src/pretix/base/models/event.py:803 (`Checkin…delete()` skips Checkin.delete(), which touches the
  order and the check-in list)
- zulip__zulip:zerver/lib/alert_words.py:66 (`AlertWord…bulk_create()` skips post_save `flush_alert_word`)

False-positive classes found and fixed:
- Test helpers under a `unittests/` directory (DefectDojo `unittests/dojo_test_case.py`, 4 matches): dj_test_doc
  does not know that directory name; the rule skips `path_part(d, "unittests")` (case `unittests/dojo_test_case.py`).
  The 4 rows are excluded from the counts above. Proposed for dj_test_doc.
- Earlier, in design: an override that only calls super() (case `Note`), code in the model's own save()/delete()
  (case `Counter`) and in its own receivers (case `tag_saved`).

## Mode

steer: 110/110 read matches are real (≥98%).

## Helpers worth a library

`bwh_root` is srl_root again: a `dj_qs_root(q, r)` / `dj_qs_model(q, mc)` in python/django would serve both.
`bwh_receiver(mq, signal, fname, f, at)` (receivers for a model sender, decorator or connect) is general Django
vocabulary (`dj_receiver`), as is `bwh_bare_super` (a method that only delegates to super).
