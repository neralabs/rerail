# 0131. A run's store is its project's, one per engine build

Numbered 0120 where it was written (More-ergonomic-rule-syntax); 0131 in the hub, where 0120 is an edit reaching strata by their deltas.

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
   store of its own per build (`$TMPDIR/rerail-test-rr-stores/`, decision 0130 §7), so a test never reads what a
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

## Amendment (2026-11-19): a store is one process's at a time

Runs started at once from one directory shared the project's store, and so its atoms lease (decision 0049), which a
run keeps across its waves: eight concurrent `run-rr` of a Django rule over netbox, from one directory, ended three
to five of eight with `content store writer is busy`, each after waiting `LEASE_WAIT` (60 s) in `Atoms::acquire`
(Django rules' bug 5; decision 0160 measured the lease deadlocking). Concurrent `test-rr` processes shared testing
mode's store the same way, from any directory.

The engine now keeps the product's discipline: a store belongs to one process, as the machine-wide one belongs to the
main daemon (`main-instance.lock`). A process owns a store by an exclusive OS lock on `<store>-owner.lock`, held for
its life and released by the OS however it ends; `<store>-owner.json` names the holder for messages
(`corpus::owner`).

- The project's store and testing mode's are the first of their slots no other process owns: `<name>.sqlite`, then
  `<name>-2.sqlite`, `<name>-3.sqlite`, … A run alone, or runs one after another, keep the first and what it caches;
  runs at once each own one. No process waits for a slot.
- A store `RERAIL_TAPE_CACHE` names is waited for: the second process says whose it is, and runs once that process
  ends, or fails after 600 s with an error that names it.
- The cleanup of other builds' stores leaves a store a live process owns.

Sharing one store between concurrent processes stays unsupported: that is the lease's design question (decisions
0049, 0160, 0170), not the store's choice.

## Amended (2026-11-24): another build's store goes with its folders

The cleanup removed an unused store's files with `remove_file`, which fails on a folder, so a store's folders stayed:
its objects, kept facts, atoms and memo, most of its bytes. On this Mac `$TMPDIR/rerail-images` held 6.9 GB, and 474
of its 607 folders were stores whose file was long gone. `remove_store` now removes a folder whole
(`remove_dir_all`), and the next new build's first run removes those left over. Test:
`a_cleanup_leaves_a_store_another_process_owns`, now with each store's folders.
