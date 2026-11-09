# T13: counting pytest's tests from the source

## Intro

The data-layer repository caps its number of conformance tests. A
script runs `pytest --collect-only` over `conform/**/test_*.py`,
reads the number pytest prints, and fails when it exceeds a signed
ceiling. The script also fails when the number drops by more than 10%
at once, which usually means collection broke.

The rerail port of that check, `test-budget`, cannot run today. It
wants pytest's number from a file that a CI step writes, and the
engine has no reader for such files yet:

    collected_total: revision and stamped-artifact readers are not
    implemented

The rule also computes its own floor from the source: test functions
at the top of each file, plus the test methods of `Test*` classes.
On the corpus that floor is about half of the real number. It misses
every `@pytest.mark.parametrize` and every fixture with `params=`.

This note asks a narrower question. Rerail now resolves names across
files and infers some types. Is that enough to imitate pytest's
collection phase in rules and get close to its number, with no
pytest run?

The answer is yes, on this repository. A static model, measured
against real collection runs, is 5.8% under in total. 1,131 of 1,143
test files are exact. Nearly all of the gap comes from three files.
They build their case lists by running code, such as loops that
append to a list or library introspection. The model reads none of
the code it counts and runs none of it. The rules would need three
small engine pieces that do not exist yet (§6).

## 1. What pytest does, and what of it we model

pytest finds the tests of one file this way:

| Step | pytest | Static model |
|---|---|---|
| Files | The paths it is given that match `python_files` (`test_*.py` here, from `[tool.pytest.ini_options]`) | Glob over the path. The patterns come from pyproject.toml |
| Names | Walks the module's namespace. A function matching `python_functions` (`test_*`) is a test. A class matching `python_classes` (`Test*`) is searched for test methods | Each top-level def and class. Also each **imported** name, because importing a test function into a test module collects it again |
| Classes | Skipped if the class or any base defines `__init__` or `__new__`, or if `__test__` is false. Methods come from the whole MRO. Nested `Test*` classes are collected too | Bases are resolved across files. Inherited methods count once per name. `setattr(C, "__test__", False)` and `C.__test__ = False` are honoured |
| Not tests | A fixture method named `test_*`. A `Test*` "class" that a decorator has turned into a function | Fixture decorators are recognised. So is a workspace decorator whose return is a nested `def` |
| Parametrize | Each `@pytest.mark.parametrize` multiplies the items by `len(argvalues)`. Stacked marks multiply. So do class marks and `pytestmark`. An empty list still gives one (skipped) item | `len(argvalues)`, explained in §2 |
| Fixture params | Every fixture with `params=` in the test's fixture closure multiplies the items: its arguments, autouse fixtures, `usefixtures`, and each fixture's own arguments in turn. The nearest definition wins (class, module, deepest conftest). A name that parametrize supplies directly hides the fixture of that name | The same closure, over conftest scopes from the root to the file's directory |
| Hooks | `pytest_generate_tests` and `pytest_collection_modifyitems` can add, drop or deselect items | Not modelled. Here the generate hook applies only under `tests/`, not `conform/`. The modify hook deselects only under an environment variable |
| Skips | Skip marks do not change the count. Skipped items are still collected | Nothing to do |

### Lengths of `argvalues`

The length of `argvalues` resolves in this order:

1. A literal list, tuple, set or dict: count its elements. A spread
   `*xs` adds the length of `xs`.
2. A name or a dotted name: follow it to a module-level constant, in
   this file or another one (`from .scenarios import ALL_SCENARIOS`).
   An enum class counts its members.
3. `a + b`, `xs * 3`, `list(xs)`, `tuple(xs)`, `sorted(xs)`,
   `range(a, b)`, `zip`, `itertools.product`, `d.keys()`. Also a local
   helper `def cases(): return [...]`.
4. A comprehension without `if`: the product of its iterables'
   lengths.
5. Approximations, counted separately in the results. `X if c else Y`
   takes the larger branch; here `c` is a WebAssembly check that is
   false in CI. A filtered comprehension takes the length of its
   iterable, which is an upper bound.

Anything else is unknown. It contributes 1 and marks the test's count
as a floor.

## 2. Accuracy

Ground truth is a real `pytest --collect-only -q` with the script's
arguments (`--backend-config=conform/backends.sqlite.yaml`), over all
`conform/**/test_*.py` files in four shards, as the script runs it.
It was run twice. **corpus** is the 9,010-file data-layer corpus that
the gate uses, with 1,143 test files. **head** is the repository's
main branch at `0ca5b57` (2026-11-02), with 1,256 test files. The
static model is a `python3 -I` AST script, `collect_est.py` in the
spike's scratchpad. Each level adds to the one before it.

| Level | What it adds | corpus (truth 14,449) | head (truth 15,361) | files exact (corpus) |
|---|---|---|---|---|
| 0 | The rule's current floor (`static_tests`) | 7,603 (−47.4%) | 8,220 (−46.5%) | 726 / 1,143 |
| 1 | pytest's globs, nested classes, MRO, imported tests, `__init__`, fixture methods skipped | 7,590 (−47.5%) | 8,215 (−46.5%) | 733 |
| 2 | Parametrize with literal argvalues, stacked, class marks, `pytestmark` | 9,064 (−37.3%) | 9,831 (−36.0%) | 896 |
| 3 | Argvalues through names across files, and simple expressions | 9,264 (−35.9%) | 10,060 (−34.5%) | 918 |
| 4 | Fixtures with `params=`, through the fixture closure | 13,457 (−6.9%) | 14,301 (−6.9%) | 1,114 |
| 5 | Conditional and filtered argvalues, taken as upper bounds | 13,772 (−4.7%) | 14,616 (−4.8%) | 1,121 |
| 6 | `__test__ = False` by `setattr`; classes that a decorator turns into functions | **13,610 (−5.8%)** | **14,454 (−5.9%)** | **1,131** |

At level 6 on the corpus, 72 of 76 directory groups are exact (the
script's `by_directory` keys). The error has two signs:

- **Under, by 872.** This is almost all three files whose case lists
  are computed by running code:
  - `l5250_matching/test_phase1_case_matrix.py`: 819 real, 11
    estimated. Its cases are `_case_matrix_module().MATRIX
    .enumerate_all()`, a method on an object built at import time.
  - `l7000_versioned/test_speculative_publication_scenarios.py`: 66
    real, 13 estimated. Its cases are `_build_scenarios()`, nested
    `for` loops that `append`, then `extend`.
  - `l2060_query/test_predicate_provenance_behavior.py` (head only):
    73 real, 5 estimated. Its cases come from discovering the
    library's public functions by introspection.
- **Over, by 33.** These are the level-5 upper bounds. Several
  GraphQL suites take `[s for s in ALL_SCENARIOS if s.supports_x]` at
  the full length of `ALL_SCENARIOS`.

Without those three files the model is within 0.3% of pytest on both
trees. Fixtures with `params=` are the largest single piece, about
4,200 tests. A root conftest fixture `read_permission_scenario` has 7
params, `permission_session_id` has 2, and they reach tests through
other fixtures. Of the 591 parametrize sites, 4 stay unknown. Of the
fixture params, 33 stay unknown before level 5 and none after.

Two things this measurement settles:

- **The recorded baseline is not ground truth.**
  `.test-budget-baseline.json`'s `by_directory` sums to 12,546. That
  is the count from 2026-09-20. The corpus is later and collects
  14,449. A comparison needs a real collection of the same tree.
- **The current floor is not a floor file by file.** At level 0, 137
  tests are over-counted. One cause is the `test` prefix where pytest's
  `test_*` glob applies. Others are fixture methods and classes
  switched off by `setattr`. In total the floor stays under, so the
  first `budget_exceeded` rule is still sound in practice. It is not
  sound by construction.

Cost: each level-6 run took about 25 s of CPU in Python over about
1,150 test files plus the modules they import. That is about 6,000
file parses, nearly all of them repeated parse time. In rerail the
syntax facts and `sym_of` already exist, so the new work is only
the rules below.

## 3. What existing facts already express

Most of the model maps onto relations the Python library already
has. The cross-file pieces come from the names and imports layer:

| Piece | Existing relations |
|---|---|
| File discovery, directory group | `doc`, `file_name`, `path_part`, `glob` |
| Defs, classes, names | `body_stmt`, `def_name`, `kind`, `field`, `stmt_def` (test-budget.rr) |
| Imported test names, constants across files | `binding`, `sym_of`, `qualname`, `binding_value`, `site_only` (names.rr) |
| Bases, inherited methods, `__init__` anywhere in the MRO | `class_def`, `class_base`, `subclass`, `method`, `mro_method` (names.rr) |
| Decorators, marks, fixtures | `pytest_deco`, `pytest_mark`, `pytest_fixture` (pytest.rr), `call_arg` (calls.rr) |
| Decorator replaces the class | `call_target` over the decorator, then its return (calls.rr's typing program follows workspace decorators through their returns) |
| Literal lengths | `child` counts (aggregate over syntax only) |
| Names a parametrize supplies | `str` on the first argument, with `word(x, s)` to match a fixture name inside `"a, b"` |
| Totals, per-directory totals | `sum`, `count` |

Three shapes need care, but no new piece:

- **Lengths that recurse** (spread, `+`, a name that leads to
  another name). An aggregate inside a recursive cycle does not
  stratify. The draft folds over child order instead: `len_upto(l, i,
  k)` adds 1 for a plain element and `seq_len(x)` for a spread `*x`.
  That is recursion with addition and no aggregate. The final `count`
  of children reads only syntax, which sits in a lower stratum.
- **Nearest fixture definition.** `max` over the depth of the scopes
  that define the name and enclose the test is an aggregate over a
  non-recursive relation. The closure `needs(t, x)` is then plain
  recursion.
- **Mutable module lists.** The native value decoder drops the length
  of a list it follows, because the list may be mutated. For counting
  tests, module-level case lists are not mutated in practice. The rule
  reads its own `seq_len`, not `value`.

## 4. The draft rules

Here are the pieces that carry the idea. The full draft is
`pytest-collect.draft.rr` in the spike's scratchpad. It is not
compiled.

```
// Files: pytest's python_files globs, read from pyproject.toml
// (needs the TOML reader, §6); pytest's defaults otherwise.
.decl collect_file(d: number)
collect_file(d) :- doc(d, p, _), prefix("conform/", p),
    file_name(d, x), files_pat(g), glob(g, x).

// A module's namespace: its own defs, and the names it imports.
// `from .base import test_shared` collects test_shared again.
ns_entry(d, x, f) :- collect_file(d), modnode(d, m),
    binding(m, x, site, "import"), sym_of(site, q), qualname(f, q).

// A Test* class is skipped when it or a base, in any file,
// defines __init__ or __new__.
has_init(q) :- method(q, x, _), (x = "__init__" ; x = "__new__").
has_init(q) :- subclass(q, b), has_init(b).

// Templates: module-level tests, and methods along the MRO.
test_fn(d, "", f) :- ns_entry(d, x, f), kind(f, "function_definition"),
    test_name(x), !pytest_fixture(f, _, _).
test_fn(d, q, f) :- test_class(d, q), mro_method(q, x, f),
    test_name(x), !pytest_fixture(f, _, _).

// Length of a literal, folded over its children, so that a spread
// can add the length of what it names without an aggregate in the
// recursion.
len_upto(l, 0, 0) :- kind(l, k), seq_kind(k).
len_upto(l, i + 1, k + 1) :- len_upto(l, i, k), elem(l, i, e),
    !kind(e, "list_splat").
len_upto(l, i + 1, k + w) :- len_upto(l, i, k), elem(l, i, e),
    kind(e, "list_splat"), child(e, x, 0), seq_len(x, w).
seq_len(l, k) :- len_upto(l, n, k), n = count : { child(l, _, _) }.

// A name: a module-level constant here or in another module.
const_of(q, v) :- modnode(_, m), binding(m, x, site, "assign"),
    site_only(site), binding_value(site, v), modname(m, mq),
    q = @dot(mq, x).
seq_len(e, k) :- kind(e, "identifier"), sym_of(e, q),
    const_of(q, v), seq_len(v, k).

// One factor per parametrize mark. An empty list still yields one
// item.
pz_len(e, k) :- call_arg(e, 1, "", v), seq_len(v, n), k = max(n, 1).
factor(d, o, f, e, k) :- test_fn(d, o, f), pz(f, e), pz_len(e, k).

// The count: a product per template, then a sum (needs product, §6).
collect_item(d, o, f, k) :- test_fn(d, o, f),
    k = product w : { factor(d, o, f, _, w) ; fx_factor(d, o, f, _, w) }.
collected(n) :- n = sum k : { collect_item(_, _, _, k) }.
```

In `test-budget.rr`, `collected` would take the place of the
artifact's `collected_total`. The rule then becomes three ratchets:

- Over the ceiling by the estimate: report, labelled as an estimate.
- Over the ceiling by the floor (`collected` minus every
  `collect_open` contribution): report as definite.
- The 10% drop rule stays with the artifact. A static count cannot
  see that collection broke, and that is the point of the rule.

## 5. Accuracy to expect elsewhere

| Repository shape | Expected error | Why |
|---|---|---|
| Literal or constant parametrize, conftest fixtures (this repository) | Under 1% after the computed-list files | Measured above |
| Heavy `pytest_generate_tests`, `pytest-cases`, Hypothesis | Large, and under | Hooks and plugins create items in code |
| unittest `TestCase` suites | Exact in principle | Methods `test*` (unittest's prefix, not pytest's glob). No parametrize. `subTest` does not add items. None in this corpus |
| Case lists built by loops | Under, by the size of those lists | Needs a symbolic length for `append` in loops of known length. Possible but not drafted |
| Module-level `pytest.skip(allow_module_level=True)`, `importorskip` | Over, by the file | The file collects nothing. 5 occurrences here, none in the error list |

## 6. Missing engine pieces

1. **A `product` aggregate.** It is `sum` with `*`: 1 over no rows,
   i64, saturating or flagged on overflow. `AggOp` has Count, Min,
   Max, Sum and Join. Without it, a ranked fold over the factors
   (ranks from `count` of the earlier ones) does the same job in .rr,
   at the cost of one more relation per template.
2. **A TOML reader** (`.input … IO=toml, filename=…, path=…`), for
   `[tool.pytest.ini_options]` `python_files`, `python_classes`,
   `python_functions` and `testpaths`. Today the readers are file,
   json, jsonl, lines and yaml. `pytest.ini`, `setup.cfg` and
   `tox.ini` need an INI reader as well. Until then the rule can pin
   the patterns, with a gate that reads the file through `lines` and
   fails when the patterns differ.
3. **Scope wider than `conform/`.** `test-budget.rr` has
   `.scope dirs "conform"`, but constants and bases come from
   `data_layer_testing/` and the root `conftest.py`. The rule needs
   "report in conform, resolve over the workspace". Either the scope
   form gains an inputs side, or the rule drops the scope and filters
   on `collect_file`.

Small and optional: a `split(s, sep, i, part)` relation for
parametrize argnames such as `"a, b"`. `word(x, s)` is enough for the
one use, matching a fixture name.

Not needed: a type checker, a Python evaluator, or CPython. The level-6
decorator case uses the call-target answers that the typing program
already produces.

## 7. Plan

1. Add the `product` aggregate (front end, lowering, SQL executor),
   with a test of 1 over no rows.
2. Write `python/pytest-collect.rr` from the draft. Check it with
   rrlab over the s50 and s200 captures against the script's per-file
   counts (`est-full-6.json`), then once over the full corpus. The
   target is the per-file table of §2: 1,131 of 1,143 exact.
3. Change `test-budget.rr`: `collected` replaces the static floor; the
   floor and estimate rules are as in §4; `collected_total` stays,
   optional, for the drop rule. Use a pinned pattern gate until the
   TOML reader lands.
4. Optional: lengths for `append` inside loops of known length. That
   closes `test_speculative_publication_scenarios.py`. The case-matrix
   object (819 tests) stays out of reach without evaluation, and should
   be reported as `collect_open` with its file.

## Sources

- The rule: `gate/data-layer/packs/data-layer/candidates/rr/test-budget.rr`
- The original check: `scripts/check_test_budget.py` in the corpus
  (sharded `pytest --collect-only -q`, `by_directory` keyed by
  `conform/<dir>/`)
- Libraries: `docs/rule-syntax/lib/python/{names,imports,calls,pytest}.rr`,
  `docs/rule-syntax/lib/core.rr`. Aggregates: `logic/rr/lower.rs`
  (`AggOp`)
- Scratchpad (spike session): `collect_est.py` (the model), plus
  `truth-full.json` / `truth-head.json` (real collection per file) and
  `est-full-*.json` / `est-head-*.json` (per level)
