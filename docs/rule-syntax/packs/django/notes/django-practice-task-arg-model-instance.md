# django-practice-task-arg-model-instance

Invariant: a Celery task receives a model's primary key and re-fetches the row; it never receives the model instance.

## Sources

- Celery docs, Tasks user guide, "State" (https://docs.celeryq.dev/en/stable/userguide/tasks.html#state):
  "Another gotcha is Django model objects. They shouldn't be passed on as arguments to tasks. It's almost always
  better to re-fetch the object from the database when the task is running instead, as using old data may lead to
  race conditions."
- Celery docs, Calling Tasks, "Serializers" (https://docs.celeryq.dev/en/stable/userguide/calling.html#serializers):
  "The primary disadvantage to JSON is that it limits you to the following data types: strings, Unicode, floats,
  Boolean, dictionaries, and lists."
- Django Forum, "Never can pass Django model objects to Celery tasks?", answer by nanorepublica, 2026-01-10
  (https://forum.djangoproject.com/t/never-can-pass-django-model-objects-to-celery-tasks/43893): "Typically you pass
  just the primary key(s) of the model. The first line then does a Model.objects.get(pk=model_pk) … The benefit of
  this approach is you get the latest version of the model over a potentially stale version passed through memory."
- OddBird, "Serializing Things for Celery" (https://www.oddbird.net/2017/03/20/serializing-things/): "Given that the
  safest way to do so is to use a serialization format that only includes primitive data types, how can we send
  custom objects around?"

## Linters checked

Ruff (DJ001–DJ013 in crates/ruff_linter/src/codes.rs; nothing Celery), flake8-django (DJ01–DJ13), pylint-django
(checkers/: auth_user, forms, models, json_response, foreign_key_strings, format_html, migrations; its Celery support
only silences false positives), Bandit, django-upgrade, django-migration-linter, Django system checks: none looks at
task arguments. Celery itself fails only at call time (EncodeError under JSON) and not at all under pickle.

## What it matches

Cross-file facts: the task def (decorated `@shared_task`, `@app.task`, `@celery_app.task(...)`, any decorator whose
name is `task`) resolved from `t.delay` through imports (`sym_of` → `qualname`), possibly in another module; model
classes resolved across the run (dj_model) for constructors and annotations. One-file facts: the argument's origin in
the calling function.

- Sends: `t.delay(...)`, `t.s(...)`, `t.si(...)` (positional and keyword arguments); `t.apply_async((x,))`,
  `apply_async(args=[x])`, `apply_async(kwargs={"k": x})`.
- Instances: `self` in a model method; `request.user` / `self.request.user`; `M(...)`; `M.objects.get/create/first/
  last/earliest/latest(...)` (not after `values()`/`values_list()`, which return dicts/tuples);
  `get_object_or_404(M, ...)`; a parameter annotated with a model class; a name assigned from one of those in the same
  function; the target of `for x in <queryset>:`.
- Not test files.

Misses: instances returned by helpers (`x = make_order()`, taiga's `get_object_or_error(...)`), unannotated
parameters (paperless's signal handler `add_or_update_document_in_llm_index(sender, document)`), related objects
(`order.customer`), instances inside lists. `proven_type` (python/types) catches `x = make_order()` when the helper
returns a constructor call; it was in the first version and dropped: reading it makes the engine run the typing
program, which took over 60 min on DefectDojo (timed out) against 2 min 20 s without it, for no corpus match the
syntactic forms miss. The cases keep `made = make_order()` as a documented miss.

## Corpus

31 repos (PostHog is in the corpus SKIP list), run with the engine directly (cache off) after run.py stalled on
per-repo locks. netbox timed out in the final run (killed after ~28 min on a loaded machine; the previous rule version,
without annotated parameters, finished there with 0). Matches 5 (4 lines), repos 2, read 5, real 5, precision 100%.

- taigaio__taiga-back:taiga/export_import/api.py:64 — `tasks.dump_project.delay(request.user, project, ...)`.
- taigaio__taiga-back:taiga/export_import/api.py:367 — `load_project_dump.delay(user, dump)`.
- taigaio__taiga-back:taiga/export_import/management/commands/dump_project_async.py:66 — `user` (from
  `User.objects.get(...)`) and `project` (from `Project.objects.get(...)`), two findings on one line.
- paperless-ngx__paperless-ngx:src/documents/tasks.py:800 — `update_document_in_llm_index.apply_async(kwargs=
  {"document": document})`, `document` from `Document.objects.select_related("owner").get(...)`.
Both projects set a pickle serializer (taiga `CELERY_TASK_SERIALIZER = "pickle"`, paperless `"signed-pickle"`), so
these run and ship snapshots: the stale-data half of the footgun.

False-positive classes found and fixed: `x = M.objects.values_list(..., flat=True).get(...)` then `t.delay([x])`
(addons-server versions/tasks.py:251, found by a pre-scan) would have counted `.get()` as an instance; tam_rows now
excludes querysets through values()/values_list(), with negative cases.

## Mode

steer: 5 of 5 read matches are real. Few matches: most projects already pass primary keys.

## Helpers worth a library

- tam_task_def / tam_task_decorator ("def is a Celery task") and tam_task_call ("call sends task t") for any Celery
  rule (e.g. enqueue-on-commit could restrict `.delay` to real tasks).
- tam_instance / tam_instance_value ("expression holds a model instance"), a superset of sync-orm-in-async's
  soa_model_instance; belongs in python/django.rr as dj_instance(e).
