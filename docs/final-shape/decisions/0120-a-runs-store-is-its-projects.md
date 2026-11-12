# 0120. A run's store is its project's, one per engine build

Status: built (2026-11-12).

## Intro

The engine keeps what a run learns in a SQLite store: tapes, plans, planning choices, the memo, kept frames
(decision 0104). A run that names no store (`RERAIL_TAPE_CACHE` unset) used one store for the whole machine,
`$TMPDIR/rerail-tape-cache.sqlite`. Every worktree, build and corpus on the machine wrote to it.

That store fitted badly:

- **Most of it was dead.** Every stored key carries the engine's build (decision 0118), so only a rerun of the
  same build could use what it held. On 2026-11-12 it was 4.4 GB.
- **Nothing left it.** Its atoms only grew: 4.9 M atoms, 421 MB of text, about 20 times the full corpus's. Past
  64 MB, a run looks atoms up inside its waves (decision 0104), so `RERAIL_SQL_STRICT=1` stopped every run on it
  (O-store-atoms-grow).
- **Projects shared it.** A run read the atoms and plans unrelated runs had left, and waited behind their writes
  (O-test-rr-busy).

## The decision

1. **The default store is the project's, for this build**: `$TMPDIR/rerail-stores/<build>-<project>.sqlite`
   (`corpus::default_store`). `<build>` is the first 16 characters of the engine's build identity
   (`build_ids::engine`); `<project>` is a hash of the canonical working directory, which stands for the project
   until a run's store can follow its corpus root.
2. **Old builds' stores go.** When a process first asks for the default store, files of another build in that
   directory that have not changed for a day are removed, with their WAL, shared-memory and lock files
   (`corpus::build_store`). A store is never removed while its build is the one running.
3. **Unchanged:** `RERAIL_TAPE_CACHE=<path>` names a store, and `=off` runs with none. Testing mode keeps a
   store of its own per build (`$TMPDIR/rerail-test-rr-stores/`, decision 0119 §7), so a test never reads what a
   project's runs left. The engine's tests keep their own copies of a template (decision 0118).

The addon (`CodeRuleDslRuntime`) chooses its store the same way, so its project is its process's working
directory.

## Measured

`run-rr` of `packs/python/bandit-b101-assert-used.rr` over a one-file corpus, `RERAIL_SQL_STRICT=1` (load 19 to 45):

| Run | Instructions retired | Wall | CPU (user + sys) |
|---|---|---|---|
| First (makes the store) | 10.6 G | 9.1 s | 1.0 s |
| Second | 5.3 G | 0.9 s | 0.5 s |

Both report B101; strict mode finds no SQL inside the waves. The store is 1.4 MB. `test-rr docs/rule-syntax`
passes all 8 tests under strict.

## What is open

- **The project is the working directory.** A run over a corpus elsewhere shares the store of the directory it
  was started from. Keying by the corpus root needs the root where the store is chosen (`tape_cache_path`
  takes none).
- **The old store** `$TMPDIR/rerail-tape-cache.sqlite` is no longer opened, and nothing removes it.
