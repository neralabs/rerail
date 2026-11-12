# 0118. Each test has its own store, copied from a template

## Intro

The engine keeps what a run learns in a SQLite store: rule files' tapes, plans, planning choices, kept frames. The
engine's tests used one store per test process, shared by every test the process ran. So a test read what other
tests had left there. A plan, a choice or a statistic another test's run kept could change what this test's run did,
and two tests running at once waited on each other's leases.

Now each test has a store of its own, made when it first asks for one and removed when it ends. A new store would
make each test repeat the work every run starts with: loading the rule library (phase 0) and planning the core
program. So each test's store starts as a copy of a template that already holds that work. The template is made once
per build of the engine.

The prior art is the data-layer repository's SQLite test fixtures (`conform/conftest.py`,
`data_layer_testing/helpers/sqlite_*.py`): a template per schema key, built under a file lock into a staging file,
published with a rename and a ready mark; each test clones it (APFS `clonefile`, else a copy); dead processes' files
are swept.

## The decision

1. **Which store a test gets** (`logic::plan::corpus::tape_cache_path`, in a test build):
   - a store the test names itself: `TEST_TAPE_CACHE` (tests that need an empty store or several), or
     `RERAIL_TAPE_CACHE`;
   - otherwise its own copy of the template, made on the first ask from the test's thread
     (`corpus::test_store::own`) and removed with the thread (a thread-local's drop).

   A thread the harness did not name after a test (no `::` in its name) asks for the process's store, and says so
   once. In the runs of `logic::rr::`, `logic::plan::tests::` and `logic::plan::corpus::tests::`, none did.

2. **The template** (`corpus::test_store::template`) holds phase 0 over every library module and the core program,
   then a run of the core program over an empty corpus. It is keyed by the build identities (`build_ids::engine`,
   `build_ids::grammar`), so a new build makes a new one. It lives in `$TMPDIR/rerail-store-templates/<key>.sqlite`.
   It is made by the first process that needs it:
   - under a file lock (`<key>.lock`), into a staging store;
   - then drained (pending atoms, queued writes) and written whole with `VACUUM INTO`;
   - then renamed into place, after which `<key>.ready` names the key.

   The processes that come after find it ready. Another build's template unused for an hour is removed, and so is
   staging left by a dead process. A binary that registers no language (a lower crate's) cannot run phase 0: its
   template is an empty store, and it says so.

3. **A copy is a new store, and a copy is only a clone.** The template is published in WAL mode and with no
   identity (`store_identity` empty): it is no store until copied. At its first open a copy takes an identity of its
   own (`ContentStore::initialize`, on a writer's thread, as a new store's schema is written), so nothing the process
   keys by identity takes it for another test's copy. The copies are in `$TMPDIR/rerail-test-stores/`, so sweeping
   them reads only theirs.

4. **A store removed while the process goes on is forgotten** (`ContentStore::forget`, `observe::forget`): the frames
   the process read of it, its identity, and its observer. Each run's end waits on every observer the process holds,
   so an observer kept past its store's life costs every later run.

## What this found

A store handle that closed with atoms pending and a writer running queued the atoms' write and returned before
closing. That handle stayed counted, so the store's last close never came, and every connection to the store stayed
open for the life of the process. With one store per process this went unseen. With a store per test, each finished
test's store stayed open: four connections and its observer, 69 stores after `logic::rr::`. `ContentStore::drop` now
closes on both paths.

## Measured

On the same test binary, the old shape is reproduced by naming a new store per process (`RERAIL_TAPE_CACHE`).
Instructions retired, peak memory and the load are from `/usr/bin/time -l`.

| Tests | One store per process | A store per test |
|---|---|---|
| `logic::rr::` (102) | 668 G, 686 MB (load 20) | 740 G, 784 MB (load 21) |
| `logic::plan::tests::` (42) | 148 G, 240 MB (load 41) | 179 G, 329 MB (load 51) |
| `logic::plan::corpus::tests::` (9) | 185 G, 206 MB (load 69) | 194 G, 240 MB (load 76) |

| `logic::plan::tests::`, repeated | One store per process | A store per test |
|---|---|---|
| `closures_of_one_level_launch_in_one_wave` fails | 2 of 10 | 0 of 20 |
| "phase 0: no statistics for `p0_file_t_shape`" | 4 of 10 | 0 of 20 |
| failing tests in all | 14 in 10 runs | 0 in 20 runs |

- **The template's content is not the cost.** A template of the core program alone (1,091 atoms, against 22,615)
  costs about the same: 731 G for `logic::rr::`. It does save memory: 669 MB. One store per process prefilled from
  the full template costs 647 G, the old shape's figure.
- **The rest is per test, about 0.65 G instructions.** A test's first run on a new store opens it, makes its observer
  store, and finds the process's caches of that store cold. One shared store used to carry that warmth from test to
  test. That warmth is the cross-test sharing this decision removes, so the cost is intrinsic, not overhead.
- **Making the template** costs about 2 s of CPU, once per build.
- **A copy** (`benchmark_test_store_copy`, `benchmark_test_store_copy_steps`): 318 µs and 3.3 M instructions with
  its removal, of which the clone is 66–104 µs. Writing the identity through a connection of its own cost 1,056 µs
  and 8.5 M instructions (SQLite parsing the store's schema a second time); it is now part of the first open, which
  every test pays anyway (2.7 ms, 14.2 M). In all, about 3 ms and 17.5 M instructions a test, against 3.6 ms and
  24 M before: about 3% of what a test's first run on a new store costs.

## Why the template cannot make a test's start free

Three tests, each run alone on the same binary, in instructions retired:

| Test | A: its own copy | C: the template as the process's store, no copy | B: a store that already ran this test |
|---|---|---|---|
| `native_python_syntax_verdict_keeps_raw_source_available` | 19.1 G | 18.2 G | 14.2 G |
| `rules_test_names_paths_and_lines_without_regexes` | 9.9 G | 9.5 G | 5.9 G |
| `library_relations_are_shared_and_each_policy_reads_its_own_scope` | 59.3 G | 58.8 G | 49.6 G |

- **A against C, the copy: 2 to 5%.** The template holds what it can.
- **A against B, 4 to 10 G a test, is the test's own work.** A stack-sample diff of A and B for the third test
  shows where it goes:
  - phase 0 for the test's program, 860 samples. Phase 0's results are keyed by the whole set of modules a
    program reads (O147), so the template's phase 0 holds only for the template's own set. Phase 0 kept per
    module would let every test reuse the template's.
  - the test's own program: its prepared and combined forms, plans, and the run itself (`run_atoms_native_over`,
    `prepared::remember`, `combine_stored`), about 1,000 samples. These are keyed by the test's program, and the
    template can only hold them by running the test, which is the test.
  - the process, not the store: the GPU device's start (`gpu::shared::get`, 324 samples) and reading the
    program's settings texts (244). No store can hold these.
- **So the remaining lever is O147**, phase 0 per module, not more in the template. A template made from a
  previous run's stores would help only a rerun of the same build, since every stored key carries the engine's
  build identity.
