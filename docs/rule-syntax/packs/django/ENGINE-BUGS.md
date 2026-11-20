# Engine and library bugs found while writing the Django practice rules

Found while running the `django-practice-*` rules over 32 open-source Django apps with `rerail-engine run-rr`
(a release build of the engine at `91c2df8863`, 2026-11-14).
Paths are relative to the repository root. Each section is self-contained: symptom, reproduction, where it is in the
code, evidence, the workaround used, and a suggested fix. "Not confirmed" marks what was inferred and not proven.

The last column is each bug on main after the final-shape engine landed (2026-11-19): read from the code, and for 3, 4,
7, 8 and 10 run as a probe rule with its own test (`rerail-engine test-rr`), and for 2, 6 and 9 measured with release
builds on the apps they name (each section's "On main"). Bug 10 appeared with the final-shape engine and is fixed on
main by #862.

| # | Bug | Area | Status | On main, 2026-11-19 |
|---|---|---|---|---|
| 1 | Runs deadlock at 0% CPU | engine: `logic/strings.rs`, rayon | Reproduced many times; mechanism partly inferred | Fixed: `Vocabulary::rows` releases `seen` before it evaluates, and takes it again to insert |
| 2 | A huge module of string literals never finishes | engine guests / `python/names.rerail` annotations | Reproduced | Fixed: with `un_locode.py`, the probe finishes on netbox in 15 s and the whole pack in 164 s (measured) |
| 3 | Module names are taken from the repository root (src layouts) | engine `doc` facts vs Python imports | Reproduced with a 3-file probe | Fixed: a module is named from its package root (probe passes); a folder without `__init__.py` keeps its path's name |
| 4 | `sym_of` drops relative imports of two or more dots | `lib/python/imports.rerail` | Reproduced; root cause found | Fixed: `impmod` resolves any number of dots through `rel_base` (probe passes) |
| 5 | One tape cache per machine: concurrent runs fail | engine `tape_cache_path` | Reproduced | Partly: the default store is per build and working directory; runs started from one directory still share it |
| 6 | Very large repositories do not finish | engine, scale | Observed once (PostHog); two causes found | Fixed by #878 and #882: the whole pack finishes on PostHog in 100 min (main: killed out of memory at 376 s); its 57 GiB peak is open |
| 7 | `sym_of` has no row for an attribute of a class of the same module | `lib/python/names.rerail` | Reproduced with a probe; cause found | Fixed by #898 (probe passes); workaround removed |
| 8 | A call to a def under an outside decorator has no call target | `lib/python/calls.rerail` | Reproduced with a probe | Fixed by #875: the call targets the def and stays open (probe passes); workaround removed |
| 9 | Rules that read the call graph take hours on large repositories | `python/calls` typing program, scale | Measured; cause found | Minutes on main (107-380 s a rule on the three largest apps); #878 cuts each rule's time to 39-73% of that (table in §9) |
| 10 | A negated tree axis is read wrong below a node a sym_of relation found | engine: lowering or demand of `!inside`/`within_stop` | Reproduced with a probe; bisected to a merge | Fixed by #862; the three rules it broke pass their cases |

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

**On main, 2026-11-19: fixed.** With `un_locode.py` in place, a release build of main (`c219472639`), 16 threads: the
probe above finishes in 15 s (38 s CPU, 0.8 GiB, 203 findings), and the whole pack in one `run-policies` call in 164 s
(564 s CPU, 4.5 GiB). python/names now asks for an annotation guest only in an annotation context (`ann_ctx`).

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

**Workarounds used (rule-local), removed now that main names modules from their package roots.** python/django matched
a task or task class by name suffix (`dj_same_name`), and `django-practice-exists-for-existence` accepted a manager
whose class resolved to nothing in the run; both are gone, and the rules' cases pass without them
(exists-for-existence's src-layout case is a model in `src/inventory/`). `django-practice-settings-module-import` keeps its source-root
matching (`smi_resolves`, `smi_mod`): that is Python's semantics, not a workaround, since a folder below a source root
with no `__init__.py` (a namespace package) keeps its path's name in the engine.

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

**Workaround used, removed now that main resolves any number of dots.** python/django's `dj_task_ref` accepted any
import-bound name (`binding(_, _, _, "import")`), and select-for-update-outside-atomic's `sfu_cm_call` matched an
unresolved call to a decorated def by name; both are gone, and the four-dot case of enqueue-on-commit passes through
`sym_of`.

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

**On main, 2026-11-19: two causes found and fixed (#878, #882); PostHog finishes.** Release builds, 16 threads of a
32-core Linux machine with 61 GiB. count-not-len alone, the run above, finishes on main in 314 s (717 s CPU,
6.5 GiB). The whole pack in one `run-policies` call:

| Engine | Result | Wall | CPU | Peak |
|---|---|---|---|---|
| main (`c219472639`) | killed by the out-of-memory killer | 376 s | 2,370 s | 51.2 GiB |
| with #878 | stopped, on one thread in `held_sorted` | 1,856 s | 5,196 s | 18.4 GiB |
| with #878 and #882 | finishes, 1,264 findings | 5,998 s | 21,955 s | 57.3 GiB |

#878 is bug 9's cause (every builtin reference times every document, here 29,771 documents). #882: a tagged order
walked the span of rows sharing a leading value to its end for each new row a fixpoint wave checked (`tag_span` in
`Run::held_sorted`), 92% of the samples of a 30 s `perf` record. What remains is a question for the engine's owner, not
a bug found: the run's 57.3 GiB peak, at the limit of the machine, and its length. The typing program is done by 900 s;
after it, a `perf` record at 13 minutes put 46% of the samples in `Run::find_at` and 25% in `first_print` (whole-key
probes scanning a span's key prints), and the hot relations at 900 s were select-for-update-outside-atomic's own
`sfu_built_for_atomic` (454k rows to 128M in one step, 88 s CPU) and `proven_type`. Not attributed further: the
profile a combined run writes covers its last part only.

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

**Where.** `docs/rule-syntax/lib/python/names.rerail`, `sym_of`. Rule 1 goes through `qn`, which extends a name along
an attribute chain (`qn(n, @dot(q, a)) :- attr_of(n, o, a), qn(o, q)`) but starts only from an import binding
(`ibind`). Rule 2, a local def or class, matched only the bare name (`look(n, x, s), defn(s, x, f), qualname(f, q)`),
with no attribute step.

**Workaround used (removed).** get-or-create-needs-unique resolved the Meta base through its object:
`attr_of(e, o, "Meta"), sym_of(o, bq)`. It now reads the Meta's `class_base` and matches it to the base's own Meta.

**Fix (#898).** `sym_of(n, @dot(q, a)) :- attr_of(n, o, a), sym_of(o, q), class_def(_, q)`: an attribute of a class
of the run is the class's name and the attribute's, along a chain (`Base.Meta.abstract`), as `qn` extends an imported
name; `class_base` follows. An attribute of a def or of anything not a class of the run still has no row.

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

**Where.** `native/src/code_rule_dsl/typing/programs/calls.rdl` (and `calls.rerail`, its translation), the typing
program `python/calls` reads: a decorator that is neither transparent nor a function of the run made the decorated
name's value unknown, so the call was left open with no target.

**Workaround used (removed).** `django-practice-select-for-update-outside-atomic` fell back, for an open or unresolved
call, to the def its callee's name resolves to by import (`sfu_target`, `sfu_cm_call`). The rule now reads
`call_target` alone.

**Fix.** #875: a call of a def whose decorators are imported from outside the run targets the def and stays
`call_open`, since the wrapper is code the program does not read. The probe's `held()` and `cached()` now target
their defs.

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

**On main, 2026-11-19: minutes, not hours; one cause found and fixed (#878).** Release builds, 16 threads of a
32-core Linux machine, nothing else on those cores. Each call-graph rule alone, wall s (peak GiB), on main
(`c219472639`) and with #878; the findings are the same in every pair:

| Rule | edx-platform (4,459 files) | saleor (4,385) | DefectDojo (2,549) |
|---|---|---|---|
| `call_target` alone (probe) | 172 (11.1) → 66 (2.0) | 202 (14.0) → 61 (2.0) | 85 (5.3) → 39 (1.4) |
| get-handler-writes | 243 → 128 | 233 → 91 | 112 → 60 |
| dispatch-in-atomic-request | 229 → 111 | 264 → 108 | 110 → 65 |
| select-for-update-outside-atomic | 280 → 170 | 380 → 256 | 168 → 122 |
| sync-orm-reached-from-async | 252 → 146 | 237 → 94 | 107 → 64 |
| the four in one `run-policies` | 383 → 275 | 453 → 298 | 207 → 166 |

The cause (#878): the demand split of decision 0048 cut `docof(n, d)` out of the typing program's `builtin_ref`, and
its guard against a cross product skipped `exists_0(d)` (simplify's `modnode(d, m), !star(m)`), an atom of documents
only. `pre_168(d, n, x)` was every builtin reference times every document: 260M rows of work and `demand_docof` 129M
on edx-platform, 55 of the typing program's 100 s of join CPU; 88 of 127 s on saleor. The rows grow as the files
squared. The hours of the first runs are not reproduced: they were on a machine at load 15 to 50, with restarts after
bug 1. Running the four rules in one call shares the typing program: 275 s against 555 s one by one (edx-platform).

The `proven_type` version of task-arg-model-instance on DefectDojo: 104 s on main (5.3 GiB), 60 s with #878 and #882
(1.6 GiB), against 21 s without it; same findings. The rule reads `proven_type` again.

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
alone, so the commit inside the hub is not found. The cause, found by #862: of two hub changes, one copies a kind set
for the nodes that may be annotations, and the other (`fold_own`) folds those copies back and deletes the set, but
redirects only `nearest`, so the negated axis is left reading a set that no longer exists.

**Workaround used.** None: the rules keep `within_stop` as written. Before #862, atomic-around-caught-db-error,
enqueue-on-commit and select-for-update-outside-atomic failed their cases (`.test cases`); with it, all 27 rules pass.

**Fix.** #862.
