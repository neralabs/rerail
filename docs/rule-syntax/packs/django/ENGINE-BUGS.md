# Engine and library bugs found while writing the Django practice rules

Found while running the `django-practice-*` rules over 32 open-source Django apps with `rerail-engine run-rr`
(a release build of the engine at `91c2df8863`, 2026-11-14).
Paths are relative to the repository root. Each section is self-contained: symptom, reproduction, where it is in the
code, evidence, the workaround used, and a suggested fix. "Not confirmed" marks what was inferred and not proven.

The last column is each bug on main after the final-shape engine landed (2026-11-19): read from the code, and for 3, 4,
7, 8 and 10 run as a probe rule with its own test (`rerail-engine test-rr`). Bug 10 is new on main.

| # | Bug | Area | Status | On main, 2026-11-19 |
|---|---|---|---|---|
| 1 | Runs deadlock at 0% CPU | engine: `logic/strings.rs`, rayon | Reproduced many times; mechanism partly inferred | Fixed: `Vocabulary::rows` releases `seen` before it evaluates, and takes it again to insert |
| 2 | A huge module of string literals never finishes | engine guests / `python/names.rerail` annotations | Reproduced | Likely fixed: python/names asks for annotation guests only in annotation contexts (`ann_ctx`); not re-run on netbox |
| 3 | Module names are taken from the repository root (src layouts) | engine `doc` facts vs Python imports | Reproduced with a 3-file probe | Fixed: a module is named from its package root (probe passes); a folder without `__init__.py` keeps its path's name |
| 4 | `sym_of` drops relative imports of two or more dots | `lib/python/imports.rerail` | Reproduced; root cause found | Fixed: `impmod` resolves any number of dots through `rel_base` (probe passes) |
| 5 | One tape cache per machine: concurrent runs fail | engine `tape_cache_path` | Reproduced | Partly: the default store is per build and working directory; runs started from one directory still share it |
| 6 | Very large repositories do not finish | engine, scale | Observed once (PostHog) | Not re-measured |
| 7 | `sym_of` has no row for an attribute of a class of the same module | `lib/python/names.rerail` | Reproduced with a probe; cause found | Present (probe fails as described) |
| 8 | A call to a def under an outside decorator has no call target | `lib/python/calls.rerail` | Reproduced with a probe | Present (probe fails as described) |
| 9 | Rules that read the call graph take hours on large repositories | `python/calls` typing program, scale | Observed; not profiled | Not re-measured; the typing program is unchanged |
| 10 | A negated tree axis is read wrong below a node a sym_of relation found | engine: lowering or demand of `!inside`/`within_stop` | Reproduced with a probe; bisected to a merge | Present; fixed at its root in its own pull request. Until then three rules fail their cases |

---

## 1. Runs deadlock at 0% CPU

**Symptom.** A `run-rr` stops making progress: CPU time stays at 0, it never exits, and it holds its tape cache. It
happens at random, more often on larger repositories, under the default thread count and with `RERAIL_THREADS=4`.
During the corpus runs it hit netbox, openedx, cvat, DefectDojo, healthchecks, authentik, readthedocs, taiga-back
and LibrePhotos at different times; one run sat at 0 s CPU for 100 minutes, three others for 10 to 14 hours.

**Reproduce.** Run any rule that includes `python/names.rerail` over a mid-sized repository a few times in parallel, e.g.
`RERAIL_TAPE_CACHE=off rerail-engine run-rr docs/rule-syntax/packs/django/django-practice-count-not-len.rerail <repo> <out>`
on netbox or openedx. Not every run hangs; it showed up a few times per full pass of a rule over the 31-app corpus.

**Where.** `native/src/code_rule_dsl/logic/strings.rs`, `Vocabulary::rows` (line 242): it takes the `seen` mutex
(`let mut seen = self.seen.lock().unwrap();`, line 248) and keeps it while it calls
`compiled.searchers.evaluate(&terms)` (line 261). `Searchers::evaluate`
(`native/src/code_rule_dsl/text/vocabulary.rs`, line 217) runs on rayon (`terms.par_iter()`, lines 225, 245, 274,
281). `Vocabulary::rows` is called from `with_nearest` (`native/src/code_rule_dsl/facts/vtab/syntax.rs`, line 2404,
`tests.rows(&strings, &symbols)`), which runs once per chunk on the same rayon pool.

**Evidence.** `sample <pid>` of a hung run (`django-practice-settings-module-import` on LibrePhotos, 0.0% CPU for 18
minutes):

```text
Thread rerail-cpu 1:  with_nearest → _pthread_mutex_firstfit_lock_wait            (blocked on a mutex)
Thread rerail-cpu 4:  with_nearest → Searchers::evaluate → _pthread_cond_wait     (waiting for its par_iter)
Thread rerail-cpu 5:  with_nearest → _pthread_mutex_firstfit_lock_wait            (blocked on a mutex)
other rerail-cpu threads: idle in _pthread_cond_wait; main thread in corpus::run_observed → Builder::advance
```

One thread holds `seen` and waits for its parallel evaluation; other pool threads are inside `with_nearest` for other
chunks, blocked on the same mutex. Not confirmed: exactly which rayon job the parallel evaluation is waiting on. The
likely mechanism is the known rayon hazard of holding a lock across a parallel call: the pieces of `evaluate`'s
`par_iter` are stolen by, or queued behind, pool threads that are themselves blocked on `seen` (a std `Mutex` is not
re-entrant, so a pool thread that steals a chunk job while waiting can also block on a lock its own stack holds).

**Workaround used.** The corpus runner killed a run whose CPU time stopped growing for 60 s and retried it, up to three
times. `RERAIL_THREADS=1` avoids the hang but is impractically slow: netbox did not finish in 10 minutes single-threaded
and finished in 169 s with 4 threads.

**Suggested fix.** Do not hold `seen` across `evaluate`: under the lock, collect the fresh strings and release it;
evaluate them outside the lock; take the lock again to insert the results (two chunks may evaluate the same string
twice, which is harmless). Alternatively evaluate that step without rayon, or with `rayon::ThreadPool::install` on a
separate pool.

---

## 2. A huge module of string literals never finishes

**Symptom.** On netbox, a rule whose program reaches model classes (`dj_model`, through `class_def`/`class_base`) ran
for more than 25 minutes, peaked at 33.5 GB, and was killed. The same probe on netbox without one data file finishes in
about 52 to 66 s.

**The file.** `netbox/extras/data/un_locode.py` in netbox-community/netbox at
`cf6d3f0319fade2fa0f1c51155f1c11330e58ff3`: 6.2 MB, 111,557 lines, one module-level tuple of about 111k
`('AD-ALV', '…')` string pairs. No Django code. (`netbox/extras/data/iata.py`, 480 KB, has the same shape.)

**Reproduce.**

```text
.policy "probe-model" revision 1
.language python
.scope include "**/*.py"
#include "python/django.rerail"
.violation P(at: node) "m"
P(n) :- dj_model(c), field(c, "name", n).
```

`RERAIL_THREADS=4 RERAIL_TAPE_CACHE=off rerail-engine run-rr probe.rerail <netbox checkout> <out>`: no result after 15
minutes. Delete `netbox/extras/data/un_locode.py`: 187 findings in 66 s. Not every rule is affected: count-not-len,
which reads only `sym_of`, finished on netbox with the file present in 169 s.

**Evidence.** A `RERAIL_PROFILE` run of the probe (stopped by SIGTERM at 240 s) showed 297,707 guest parses
(`local/guests/tree-sitter parse`) and the chunk holding the file stuck in a set-fixpoint seed join after the
`tgt`/`rasg` strata; the heads that grew were python/names' annotation strata (`rr_in_annotation_*`, `ann_str`,
`parsed`). 297,707 is about the number of string literals in the file, so every string of the module is parsed as a
possible annotation guest. Not confirmed: which rule in `python/names.rerail` (`parsed_in`, lines 386-388, or the
`string_annotation` rows, line 373) asks for every string rather than only strings in annotation positions.

**Workaround used.** The file was deleted from the corpus checkout.

**Suggested fix.** Parse annotation guests only for strings that sit in an annotation position (a parameter, return or
variable annotation, or the arguments of `TypeVar`, `cast`, `NewType`), never for every string literal of a module;
and bound guest work per document so one file cannot take a run down.

---

## 3. Module names are taken from the repository root (src layouts)

**Symptom.** In repositories whose packages sit below the root (`src/`, `backend/`, `apps/api/`, or a project directory
of the same name), an imported name never resolves to its definition, so everything that joins an import to a class
of the run fails: `subclass`, `mro_method`, `workspace_module`/`resolve_export`, and the Django library's model and
view closures.

**Reproduce.** Three files:

```text
src/shop/__init__.py   (empty)
src/shop/base.py       class Base: pass
src/shop/child.py      from shop.base import Base
                       class Child(Base): pass
```

with a rule printing `doc`, `class_def` and `class_base`:

```text
src/shop/base.py   doc module    src.shop.base
src/shop/base.py   class_def     src.shop.base.Base
src/shop/child.py  class_base    shop.base.Base        ← never equals src.shop.base.Base
```

**Where.** The engine's `doc(d, path, module)` facts (`lib/core.rerail`, line 31) derive the module from the path relative
to the scanned root. Python resolves `shop.base` from the source root (`src/`), so the names disagree.

**Evidence from the corpus.** pretix (`src/pretix/...` is module `src.pretix...`, imports say `pretix...`), InvenTree
(`src/backend/InvenTree/common/settings.py` is `src.backend.InvenTree.common.settings`), plane
(`apps/api/plane/db/models` is `apps.api.plane.db.models`), netbox (`netbox/netbox/settings.py` is
`netbox.netbox.settings`). In pretix, Celery task classes defined in the repository were invisible to
`django-practice-enqueue-on-commit` until it matched by name suffix. In allegro/ralph (`src/ralph/...`), an
`AppConfig` subclass's base `ralph.apps.RalphAppConfig` did not resolve to the run's `src.ralph.apps.RalphAppConfig`,
which broke a first version of the dropped `django-practice-receiver-never-connected`.

**Workarounds used (rule-local).** `django-practice-enqueue-on-commit` matches a class by name suffix;
`django-practice-settings-module-import` (`smi_resolves`) tries the imported name under each package prefix of the
importer's module; `django-practice-exists-for-existence` accepts a manager whose class resolves to nothing in the
run.

**Suggested fix.** Name documents from their source roots: the highest directory that holds `__init__.py` (or has no
`__init__.py` above it), or configured roots; or let `doc` carry every importable name of a document so `sym_of` and
`qualname` meet.

---

## 4. `sym_of` drops relative imports of two or more dots

**Symptom.** `from ..x import two` binds `two`, but `two` has no `sym_of` row, so a rule that identifies a function,
task or class by its qualified name misses it. Found through saleor's
`from .....order.tasks import recalculate_orders_task`, which `django-practice-enqueue-on-commit` lost when it
required `sym_of`.

**Reproduce.** In `a/b/c/d/e/m.py`:

```python
from .x import one
from ..x import two
from ...x import three
from ....x import four
one(); two(); three(); four()
```

A rule reporting `sym_of` of each callee reports only `a.b.c.d.e.x.one`. A rule reporting `import_alias` gives all
four, correctly resolved (`a.b.c.d.x`, `a.b.c.x`, `a.b.x`).

**Where.** `docs/rule-syntax/lib/python/imports.rerail`. `impmod` (lines 26-35), which feeds `rifrom` → `ribind` →
python/names' `ibind` → `qn` → `sym_of`, handles only single-dot relative imports (its comment says so, line 28).
Two or more dots fall to `irel_open` (lines 51-57) and bind the name as an unknown target (`rotgt`). Yet `rel_base`
(lines 66-71) already resolves any number of dots with `@pkg_up`, and `module_text`/`import_alias` use it.

The same holds for `from .. import X` (no module after the dots). Probe, in `a/b/c/views.py` with `BaseAPIView`
defined in `a/b/base.py` and re-exported by `a/b/__init__.py`:

```python
from .. import BaseAPIView            # import_alias: a.b BaseAPIView
from ..base import BaseAPIView as B2  # import_alias: a.b.base BaseAPIView
class V(BaseAPIView): pass            # no sym_of, no class_base row
class W(B2): pass                     # no sym_of, no class_base row
```

So a class whose base is imported this way is neither a view nor a model to `python/django`: makeplane/plane's
`from .. import BaseAPIView` views are invisible to `django-practice-get-handler-writes` and
`django-practice-sync-orm-reached-from-async`.

(An earlier note blamed tree-sitter reading `...` as an ellipsis. That is wrong: the module name node's text is
`..x`, `...x`, `....x`, and `import_alias` resolves them.)

**Workaround used.** enqueue-on-commit accepts any import-bound name (`binding(_, _, _, "import")`), with a four-dot
case.

**Suggested fix.** Add the multi-dot forms to `impmod` from `rel_base`, as `module_text` does (rules 2 and 3 of
`module_text`, lines 77-78), so `irel_open` keeps only imports outside a package.

---

## 5. One tape cache per machine: concurrent runs fail

**Symptom.** A second `run-rr` started while another is running fails at once with
`rerail-engine: phase 0: content store writer is busy` (or `content store writer is busy`).

**Reproduce.** Start two `run-rr` processes at the same time with no `RERAIL_TAPE_CACHE` set. During this work, an
eight-way parallel corpus run failed this way on three repositories, and a cases check failed while a corpus run was
going.

**Where.** `native/src/code_rule_dsl/logic/plan/corpus.rs`, `tape_cache_path` (lines 1434-1443): with no
`RERAIL_TAPE_CACHE`, every process uses `std::env::temp_dir().join("rerail-tape-cache.sqlite")`. The store refuses a
second writer (`StoreError::Busy`, `native/src/content_store/mod.rs`, line 292) instead of waiting.

**Workaround used.** `RERAIL_TAPE_CACHE=off`, or one cache file per repository (`RERAIL_TAPE_CACHE=<dir>/<repo>.sqlite`)
behind a file lock.

**Suggested fix.** Wait for the writer lock with a timeout instead of failing, or key the default cache by the scanned
root.

**Related, not reproduced.** Authors reported that after a `run-rr` was killed, the next run into the same output
directory failed with "content store writer is busy" even with `RERAIL_TAPE_CACHE=off`, with
`store.sqlite-atoms.lock` left in the output directory. Three tries (kill -9 after 1, 6 and 12 s, then a new run into
the same directory) did not show the error; the lock file is left behind each time. The third re-run, on a repository
that normally takes about 5 s, hung until a 120 s timeout, which may be bug 1.

---

## 6. Very large repositories do not finish

**Symptom.** On PostHog/posthog (`37c895112cd9830ba6161037c34c431a2a6d7d0c`, 29,885 Python files) a single
`run-rr` of count-not-len did not finish in 50 minutes (killed at 3,000 s). Netbox (1,295 Python files) takes 30 to 70 s
for most rules, so the cost is far from linear in the number of files.

**Reproduce.** `RERAIL_THREADS=4 rerail-engine run-rr docs/rule-syntax/packs/django/django-practice-count-not-len.rerail
<posthog checkout> <out>`. Not profiled; bug 2 (a few very large generated files such as `posthog/schema.py`, 1.7 MB)
and bug 1 may account for it.

**Workaround used.** PostHog is left out of the corpus.

**Suggested fix.** Profile with `RERAIL_PROFILE=<dir>` first. Note that `RERAIL_PROFILE=1` writes the profile into a
directory named `1` under the working directory.

---

## 7. `sym_of` has no row for an attribute of a class of the same module

**Symptom.** `Base.Meta`, `Base.f` or `Base.X`, where `Base` is a class defined in the same module, has no `sym_of`
row, while the same expression with `Base` imported from another module resolves. A rule that follows a model's
`class Meta(Base.Meta)` to the base's Meta options (`django-practice-get-or-create-needs-unique`) lost it when the base
sat in the same file.

**Reproduce.** `pkg/m.py`:

```python
class Base:
    X = 1
    class Meta:
        abstract = True
    def f(self):
        pass

a = Base.Meta     # no sym_of
b = Base.f        # no sym_of
c = Base.X        # no sym_of
d = Base          # sym_of pkg.m.Base
```

A rule reporting `sym_of` of each right-hand side gives only `pkg.m.Base`; `qualname` has `pkg.m.Base.Meta` and
`pkg.m.Base.f`. With `class Other` in `shop/base.py` and `from shop.base import Other` in `shop/models.py`,
`class Meta(Other.Meta)` gives `sym_of` `shop.base.Other.Meta` and the `class_base` row `shop.base.Other.Meta`; the
same-module `class Meta(Base.Meta)` gives neither.

**Where.** `docs/rule-syntax/lib/python/names.rerail`, `sym_of` (lines 359-362). Rule 1 goes through `qn` (lines 273-275),
which extends a name along an attribute chain (`qn(n, @dot(q, a)) :- attr_of(n, o, a), qn(o, q)`) but starts only
from an import binding (`ibind`). Rule 2, a local def or class, matches only the bare name (`look(n, x, s), defn(s, x,
f), qualname(f, q)`), with no attribute step.

**Workaround used.** get-or-create-needs-unique resolves the Meta base through its object:
`attr_of(e, o, "Meta"), sym_of(o, bq)`.

**Suggested fix.** Extend rule 2 along attributes as `qn` does, for a name that resolves to a class of the run:
`sym_of(n, @dot(q, a)) :- attr_of(n, o, a), sym_of(o, q), class_def(_, q)` (then `class_base` follows).

---

## 8. A call to a def under an outside decorator has no call target

**Symptom.** In `python/calls`, a call to a def of the run decorated by a function from outside the run is
`call_open`, with no `call_target`, even for decorators that keep the def's code on the call path
(`@contextlib.contextmanager`, `@functools.lru_cache`). A rule walking callers or callees through such a def stops
there.

**Reproduce.** In one module: `@contextmanager def held(): yield 1`, `@functools.lru_cache def cached(): ...`, a
decorator of the run `def mine(f)` that returns a `functools.wraps(f)` wrapper, `@mine def wrapped(): ...`, and
`def plain(): ...`; then `with held(): pass; wrapped(); cached(); plain()`. A rule reporting `call_target`/`call_open`
per call gives: `held()` open, `cached()` open, `wrapped()` → `w` (the run's wrapper), `plain()` → `plain`.

**Where.** `docs/rule-syntax/lib/python/calls.rerail`: a decorator outside the run makes the decorated name's value
unknown, so the call is left open. That is sound for an arbitrary decorator; for the standard ones above it loses
edges a rule needs.

**Workaround used.** `django-practice-select-for-update-outside-atomic` (`sfu_target`, `sfu_cm_call`) falls back, for
an open call, to the def its callee's name resolves to by import.

**Suggested fix.** Treat a short list of standard decorators that keep the def's code on the call path
(`contextlib.contextmanager`, `contextlib.asynccontextmanager`, `functools.lru_cache`, `functools.cache`) as passing
the def through, so the call targets the decorated def. (A decorator of the run, such as `mine` above, already
resolves to its wrapper.)

---

## 9. Rules that read the call graph take hours on large repositories

**Symptom.** The four rules that include `python/calls` and read `call_target` (get-handler-writes,
dispatch-in-atomic-request, select-for-update-outside-atomic, sync-orm-reached-from-async) are far slower than the
others. get-handler-writes took 2.8 h on WeblateOrg/weblate, 3.1 h on openedx/edx-platform and 3.8 h on saleor/saleor
(with restarts after bug 1 stalls), on a machine at load 15 to 50; dispatch-in-atomic-request timed out on openedx
(4,459 Python files) at 900 s and 3,600 s in early runs.

**Evidence.** A probe rule that reads only `call_target` costs most of a reach rule's time: on paperless-ngx 88 s of
user time against 101 s for sync-orm-reached-from-async, on umap 16 s against 25 s. So the typing program behind the
call graph, not the rules' own relations, dominates. The same program runs for any rule that reads `proven_type`
(python/types): a version of task-arg-model-instance that read it ran over 60 minutes on DefectDojo/django-DefectDojo
(timed out) against 2 min 20 s without it. Not profiled.

**Workaround used.** Long per-repository timeouts (up to 5,400 s) and re-running stalled repositories alone.

**Suggested fix.** Profile the typing program on one large repository (`RERAIL_PROFILE=<dir>`); running several
call-graph rules in one `run-policies` call may share its cost (not measured).

---

## 10. A negated tree axis is read wrong below a node a sym_of relation found

**Symptom.** `within_stop(n, b, S)` holds although a node of S sits between n and b, and `!inside(n, _, S)` holds
although n has an ancestor in S, when b comes from a relation that reads `sym_of` and the rule also reads
`within(n, b)`. The same b found without `sym_of` gives the right answer. On the engine of main, three rules' cases
failed: atomic-around-caught-db-error and enqueue-on-commit reported code in a def nested inside an atomic block, and
select-for-update-outside-atomic missed a lock in an `on_commit` callback defined inside one. The rules passed their
cases on the engine they were written on (`91c2df8863`).

**Reproduce.** `rerail-engine test-rr probe.rerail`:

````text
.policy "probe-negated-axis" revision 1
.language python
.scope include "**/*.py"
#include "python/names.rerail"
.violation Unused(at: node) code "U" "unused"
Unused(n) :- kind(n, "nothing_has_this_kind").
.test nested_def
    .fixture "a.py" ```
        from django.db import transaction


        def nested_def(order):
            with transaction.atomic():
                def later():
                    try:
                        order.save()
                    except E:
                        pass
                return later
        ```
    .decl bt(b: node)
    bt(b) :- kind(w, "with_statement"), field(w, "body", b), has(w, c, "call"), field(c, "function", f),
        text(f, "transaction.atomic").
    .decl bs(b: node)
    bs(b) :- kind(w, "with_statement"), field(w, "body", b), has(w, c, "call"), field(c, "function", f),
        sym_of(f, "django.db.transaction.atomic").
    .decl x_text(l: number)
    x_text(l) :- kind(t, "try_statement"), bt(b), within(t, b), !inside(t, _, "function_definition"), line(t, l).
    .decl x_sym(l: number)
    x_sym(l) :- kind(t, "try_statement"), bs(b), within(t, b), !inside(t, _, "function_definition"), line(t, l).
    .expect count x_text = 0
    .expect count x_sym = 0
.end
````

`x_text` has no row; `x_sym` has (7), though the `try` on line 7 is inside the defs of lines 4 and 6
(`inside(t, f, "function_definition")` in the same rule shape gives both). `bt` and `bs` hold the same node. Without
`within(t, b)`, `x_sym` is right. `within_stop(t, b, "function_definition")` over `bs` is wrong the same way: its
lowering (`logic/rr/lower.rs`, `within`) is `within(n, a)` and either `!rr_inside_<S>(n, _)` or the nearest stop not
below a.

**When.** The engine at `91c2df8863` gives no `x_sym` row. Along the first parents of the final-shape engine's
history, `7529228ce3` is right and the merge after it, `bddab770c8` (the serverplan-quality hub), is wrong; the six
commits after that merge do not build or do not run the probe on their own, and the hub's own commits do not build
alone, so the commit inside the hub is not found. Not confirmed: whether the negated axis atom gets no rows for an n
bound late (a demand or negation split that never asks for n's ancestors) or is lowered without them.

**Workaround used.** None: the rules keep `within_stop` as written, and the engine is fixed at its root in its own
pull request. Until it merges, atomic-around-caught-db-error, enqueue-on-commit and select-for-update-outside-atomic
fail their cases (`.test cases`). Writing the region without a negation, `within(n, b), nearest(n, s, S'),
within(b, s)` with S' the def, lambda and class kinds and the module, gives the right answer on today's engine.

**Suggested fix.** Find which pass drops the negated axis's rows for a late-bound node: run the probe with the
lowered program and plan printed (`rerail-engine dump --rule probe.rerail`) on both engines and compare how
`!rr_inside_function_definition(t, _)` is evaluated.
