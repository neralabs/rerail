# django-practice-receiver-never-connected

Invariant: a `@receiver` function lives in a module that the running project imports (the signals submodule imported
in its app's `AppConfig.ready()`), else it is never connected and never runs.

## Sources

- Django docs, Signals, "Where should this code live?"
  (https://docs.djangoproject.com/en/5.2/topics/signals/#connecting-receiver-functions): "In practice, signal
  handlers are usually defined in a signals submodule of the application they relate to. Signal receivers are
  connected in the ready() method of your application configuration class. If you’re using the receiver()
  decorator, import the signals submodule inside ready(), this will implicitly connect signal handlers"
- Vitor Freitas, Simple is Better Than Complex, "How to Create Django Signals"
  (https://simpleisbetterthancomplex.com/tutorial/2016/07/28/how-to-create-django-signals.html): "In the example
  above, just importing the signals module inside the ready() method will work because I’m using the @receiver()
  decorator."

## Linters checked

Ruff's flake8-django rules (DJ001–DJ013, from https://docs.astral.sh/ruff/rules/: only DJ013 is about `@receiver`,
and it checks decorator order), flake8-django (the same set), pylint-django (README feature list: model attributes,
ModelForm exclude, migrations plugin), Bandit, django-upgrade, django-migration-linter, Django system checks
(signals.E001 is about lazy model senders). None checks whether a receiver's module is ever imported.

## What it matches

A `@receiver(...)` (django.dispatch.receiver, also through `import django.dispatch`) in module m, outside tests, when
nothing loads m. Cross-file facts: every import statement of the run (absolute, `from pkg import m`, relative of
any depth, an import of a module below m), string literals naming m or a module below it, `autodiscover_modules`
calls, Celery `autodiscover_tasks` calls. Module names compare by dotted tail too (`app.signals` loads
`backend.app.signals`): a run's module names start at the repository root, which may sit above the import root.
Auto-loaded modules: `models` (and anything under a `models` package), `admin`, `apps`, `tasks` when the run calls
`autodiscover_tasks()`, x for `autodiscover_modules("x")`. Autodiscover-style string loading, as the corpus does it:
an apps.py (or a module with a `…AppConfig` base) holding x as a string loads every module named x (authentik's
`ManagedAppConfig.import_related` runs `import_relative("signals")`, i.e. `import_module(f"{self.name}.signals")`;
ralph's `get_load_modules_when_ready()` returns `["publishers"]`); a string ending in `.x` around a placeholder
(`f"{app}.signals"`, `"%s.signals"`) loads every module named x. A `@receiver` on a def nested in a function is not
reported: it connects when that function runs.

Misses: modules only reachable from a module that is itself never imported (no transitive reachability), dynamic
imports whose name is computed without a `.x` string piece.

## Corpus

Full corpus (31 repos; PostHog is in SKIP): 1 match, allegro__ralph:src/ralph/domains/publishers.py:47, a false
positive: ralph's `Domains(RalphAppConfig).get_load_modules_when_ready()` returns `["publishers"]`, which
`RalphAppConfig.ready()` imports. The first fix matched AppConfig subclasses through the run's classes, which fails
there: the run names the module `src.ralph.apps` while the code imports `ralph.apps`, so the base did not resolve.
Fixed by matching the string in any apps.py or module with a `…AppConfig`-named base (case
`src/ralph/domains/`); rerun on ralph: 0. netbox-community__netbox timed out (900 s in run.py, then a private run stopped after 36 min) and is not judged.

Cross-check by grep over the corpus (every non-test module with `@receiver` whose `parent.last` name no non-test file
mentions and that no sibling imports relatively): authentik (22 `signals.py` modules, loaded by
`import_relative("signals")`), ralph `domains/publishers.py` (above), pretix `base/datasync/utils.py` (its receiver
is nested in a function). All three are loaded or connected; the rule reports none of them.

Real matches: 0. Read: 1 of 1.

## Mode

DROP: no real match in the corpus. Kept as `.mode log`. Projects that use @receiver load the module (by import in
ready(), by importing the module from models, or through an autodiscover helper); the failure this rule catches is
likely caught by the project's own tests before it lands.

## Helpers worth a library

`rnc_tail` / `rnc_loaded` (is module m imported by the run) would serve any "module never imported" rule
(`python/modules`): e.g. a Celery tasks module, a templatetags module, an admin module outside autodiscovery.
