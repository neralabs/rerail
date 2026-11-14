# 0197: Warm mode: a truly warm run writes nothing, and any write fails it

Status: built (2026-11-14). A warm batch over a corpus that is its own repository now writes nothing; what a warm run with the cutoff off still writes is collection's bookkeeping, open below. Follows decision 0183 (a warm
batch recomputes nothing) and uses 0104's statement trace (`RERAIL_SQL_STRICT`).

## Intro

A run is truly warm when its store already holds everything it computes, and its rules and inputs have not changed
since. Such a run should only read. Measuring warm runs (0183) shows how much they cost, but not why: a warm run
that quietly computes and writes something again still gives the same findings, so nothing fails.

`RERAIL_WARM` is a development flag that turns each such recomputation into a failure:

```
# Warm the store, then check that a second run only reads.
rerail-engine run-policies dl1 out-1 policies...              # cold, or warm already
RERAIL_WARM=1 rerail-engine run-policies dl1 out-2 policies...   # aborts at the first write, with its stack
RERAIL_WARM=list rerail-engine run-policies dl1 out-2 policies...  # reports them all, exits with status 3
```

It is only for runs known to be warm. Over a changed tree it fails, as it should.

## What fails a warm run

| Fault | Where it is caught |
|---|---|
| A SQLite write committed, on any connection: the store, its `.files` observer store, the cost samples, an in-memory database | A commit hook on every connection (`profile::warm_commit`). SQLite calls it only for a transaction that wrote or took the write lock. The store's atoms set their own hook, which calls it too. |
| A write queued for the store's writer | `ContentStore::write_back` and the like (`write_back::queue`), so the stack is the caller's, not the writer thread's. |
| The whole-run cutoff asked for and not applying | `reuse::miss`: a rule file, a document or a read changed, or no record. A run that did not ask (a session's) is checked by its deltas instead. |
| A changed document, or a non-empty delta | `input_delta::changed` and `delta`; a maintained part's derived `__minus`, `__plus`, `__del`, `__ins`, `__gone` relations with rows. |

The mode is `profile::warm_mode()`, beside `strict()`. `RERAIL_WARM` is one of `reuse::NOT_OUTPUTS`: it changes how
a run is checked, not what it outputs. If it were in the run's key, no checked run would ever find the record of
the run that warmed the store.

`list` prints each kind's first fault with its stack and each distinct fault once. When the process exits it
prints the counts and exits with status 3.

## What only the check reads

The mode changes no computation and no write. Two things hold that, and `scripts/check/check-warm-mode.py` checks
both before every commit (the commit hook runs it beside the cache and thread-state checks):

- **Who reads it.** Only `profile.rs` (the mode itself), the maintained part's delta check in `corpus.rs` (it reads
  deltas held in memory and faults) and `reuse::NOT_OUTPUTS` (left out of the cutoff's key) may read
  `RERAIL_WARM`, call `warm()` or `warm_mode()`, or name `Warm::`. Each place has an approved count of reads. A
  new reader fails the check. So does another read in a listed place. Functions also called `warm` that warm GPU
  pipelines are not the mode. A fault's detail closure only formats what the caller already holds.
- **What a warm run may still write.** `profile::WARM_WRITES_ALLOWED` lists them, each with its reason. The check
  holds a copy, so the list cannot grow without the user's approval:

  | Write (the writer's job) | Why |
  |---|---|
  | the run's hold on its plans | A lock: a lease on the run's plans against a collection in another process (0053) |
  | the run's hold released | The lease's release |
  | a source's pin | A lock: a lease on a source the run reads, until its manifest names it (O95) |
  | the combined program's pins released | The release of the combined program's leases |
  | the run's manifest | Timestamps, only for a manifest identical to one stored: its last run and time move, by which collection keeps a root's last two. A new manifest is a fault (`a new manifest recorded`) |
  | the journal's aged cursor | A timestamp: a root's journal cursor, or a cutoff record's (`git-files:`, `cone-rounds:`), stored again once it is `observe::CURSOR_AGE` (2^20) events behind with nothing named since. Added with the user's approval (2026-11-14) |

  Neither the queued job nor its commit is a fault, and only when every job in the writer's transaction is
  listed. The writer opens a scope around each transaction (`profile::warm_scope`), each job marks it
  (`warm_job`), and the commit hook passes only a scope of listed jobs. A wait for the writer queues an empty job,
  which writes nothing and is not a fault.

  Not listed, and still faults: the frames' used marks, the run's atoms, a source the manifest names, and the
  combined program's text. They are recency and content, not locks or timestamps.

One name collided. The Metal segment benchmark (`parser_tests::metal_tests`) read `RERAIL_WARM` as its count of
warm-up blocks, so turning the check on changed that benchmark. It is now `RERAIL_SEG_WARM`.

## First findings: the `dl1` batch

These are from 44 data-layer policies over one Python file (`dl1`, decision 0183), `run-policies`, CPU, strict
SQL, debug build. A rebuild changes the engine's id, which is in the run's key, so each check came after one cold
run and one warming run.

The whole-run cutoff applied in every warm run: no policy was planned or evaluated. Findings were the same in
every run. Every write came from setup around the run, not from evaluation:

| Found | Writes a warm batch | Status |
|---|---|---|
| The store's schema check applied all 124 `CREATE` statements of `SCHEMA` to a scratch in-memory database, once a process, to learn the object names it expects | 124 | **Fixed**: the names are read from `SCHEMA`'s text (`schema_objects`). The test `schema_objects_are_what_schema_creates` applies the schema and compares the two. |
| The observer bumped a stored `epoch` counter on every open, to rotate which 1/64 of trusted files it hashes again | 1 | **Fixed**: the epoch is the second the process opened in. Consecutive processes still take consecutive slices. Lost: the counter's guarantee that 64 processes in a row cover every file once. Two processes in the same second check the same slice. |
| The observer moved and stored each watched root's journal cursor on every run, though the journal named nothing under it | 13 | **Fixed** (below) |
| One canonical path form written again | 1 | Not seen again after the store's first warm run; `the observer resolved a canonical form it did not hold` names it if it returns |

### Second pass: the cutoff off

With the cutoff off (`run-rr` with `{"reuse": false}`), a warm run plans and evaluates, and maintains from what the
last run kept. Its deltas were all empty, the cross-file part ran none of its 45 strata, and its outputs matched a run
without a store (`dl10`, `layer-violations.rr`, 8 `internal_import` rows). These were found and fixed:

| Found | Fix |
|---|---|
| A chunk of memo hits alone built no document, yet its local program was planned and run over a projection of nothing (15 components, ~12 strata), its tree counts measured from it and written (3 jobs), and a cost sample written to `~/.cache/rerail/costs.sqlite` | A chunk with no built document is skipped as one with no sources. |
| `the memo's entries` queued with no entry | Not queued. |
| `the files' stats` queued with no file | Not queued. |
| A known file read in the waves (where the store is not asked) has its content named by its digest, so the stat the index already held was recorded again (4 files of `dl10`, every run) | Not recorded. A known file found to be another stored source (a stat that lied) still is. |
| The observer recorded again an answer it already held and trusted (a clean Git file named by its blob, every run) | `remember` keeps an unchanged, trusted answer as it is. |

### The journal cursor

The cursor is stored only when the journal names a path under the root. When it names nothing, memory's cursor moves
on, for this process's later asks, and the store's stays where it was. The next process replays from there, which
gives a superset of the changes, so no change is missed.

Two measurements decided it:

- **An old cursor is not slow to replay.** A warm batch was run over copies of one store, one with its cursors set
  2 million events back (about two hours of this machine). The journal's asks took 0.23 to 1.14 s over 13 either way,
  at load 50 to 95. Each ask's time is the wait for its cookie's event, not the replay's length.
- **Keeping the cursor wherever nothing was forgotten was wrong.** A root whose subtree churns (the work tree, with
  the run's outputs and store in it) then named every path changed since, growing each run: 971 paths in one ask. `forget`
  matches each against every answer kept, and the warm batch went from 8 to 53 G instructions. Where paths are named,
  the cursor moves as before.

### What the harness was

`dl1` sat inside this repository's work tree, so Git's discovery read the work tree's `.git`, and the observer watched
the whole work tree. Its replay then named the harness's own writes: the store, the outputs, the logs. A corpus that
is its own repository (`dl1g`, `dl1` with `git init`) is what a real corpus is. Over it the warm batch writes nothing,
and takes 6.7 G instructions and 0.58 s user, against 8.0 G and 0.72 s over `dl1`.

## Open

- **Collection's bookkeeping, with the cutoff off.** A warm run that plans still writes:
  - its manifest, identical to its root's newest: an identical manifest only moves its `last_run`, the recency by which
    collection protects a root's last two;
  - its hold on its plans and the hold's release, a source's pins, and the combined program's pins and their release:
    leases against a collection pass in another process while the run reads;
  - the frames' used marks (`frames`, `the frames' parts used`), by which relations unused for `RUNS` runs are
    forgotten;
  - the run's atoms, if any were made (`flush_atoms`, `the pending atoms`), and the writer waits.

  The warm batch with the cutoff applied writes none of these, and collection still keeps what it needs, so skipping
  them when they would change nothing is consistent with that path. Not bumping recency only costs a recomputation
  later, never a wrong result. Dropping the hold and pins changes how a run is guarded against another process's
  collection (decision 0053), so it is left for that design.
- **`forget` was quadratic**: it matched each named path against every answer kept, so a root that churns cost
  paths × answers on every ask. Fixed: the answers and canonical forms are sorted by path (`BTreeMap`), and what
  lies under a named path is one range from it (`forget_takes_what_is_under_a_path_and_no_sibling`).
- **The widest stored root stays** (`watch_above`). The deepest was measured with a temporary switch, on the warm
  44-policy batch, at load 64 to 95:

  | Corpus | Root | Asks | Their wait | Paths named | Instructions |
  |---|---|---|---|---|---|
  | `dl10`, inside this work tree | widest | 13 | 3.86 s | 354 | 9.7 G |
  | | deepest | 17 | 5.48 s | 314 | 9.9 G |
  | `dl1g`, its own repository | widest | 22 | 6.02 s | 0 | 8.2 G |
  | | deepest | 22 | 7.00 s | 0 | 8.2 G |

  The deepest root names 11% fewer paths where the work tree is a root, and asks 30% more often. Each ask costs a
  wait for its cookie, and now that `forget` is a range a path, a path named costs little. Where the corpus is its
  own repository, the roots do not nest and the two are the same.
- **The journal's asks were the warm batch's wall time**: 22 asks, about 6 s of waits for 0.8 s of CPU over `dl1g`.
  Each cause is fixed:

  | Seen | Fix |
  |---|---|
  | One directory asked as two roots, `/var/folders/…` and `/private/var/folders/…` (phase 0's root) | Phase 0's root is canonical, and not watched up front: it is an empty placeholder, and a read under it is watched as it comes |
  | A root kept as `native/crates/logic/../../src/…` | `rr::native_dir()` names the source tree without `..`; the library, the typing programs and phase 1's program are found through it |
  | Each root asked again after every run inside the run (phase 0's, a policy's) | Only the outermost run's end (`Observer::flush` with no run left open) ends the roots' answers and settles the roots the journal could not speak for |
  | Each root asked again, with a cookie's wait, at each policy of a batch | A batch is one run for the observer (`corpus::ObservedBatch`, in `run_policies`): its runs read the tree as of its first ask; a change meanwhile is named to the next batch |
  | `dl1g/.git` asked beside `dl1g` | A cutoff's check (`changed_since`) skips a root under another it holds with the same cursor: one ask gave both |
  | Phase 0's root timed out twice, so it was not trusted | Not asked up front (above) |

  After all of them: 6 asks and 0.51 s of waits, 1.1 s wall for the warm batch (from about 10 s), 6.9 G instructions,
  no warm fault, the same findings (load 47 to 87). Before the last, 12 asks waited 1.5 s, mostly the first cookie
  of each library stream (0.6 to 0.9 s): they replay from cursors 7 million events old (below).
- **An old cursor is slow to replay after all, at 7 million events.** It was not at 2 million (above). The first ask
  of a stream started from a cursor that old waited 0.6 to 2 s, against 0.04 to 0.17 s for the same stream's later
  asks; a cutoff record's check from 4 million events back timed out, and the record was made again. A cursor stayed
  where it was while the journal named nothing under its root, so a root or a record that never changes aged without
  bound. Fixed, with the user's approval of the write:
  - a root's stored cursor `CURSOR_AGE` (2^20) events behind, with nothing named, is stored again (`forget`);
  - so is a cutoff record's cursor line, where its check (`changed_since`) finds nothing named under its roots: it
    holds as of the journal's cursors now;
  - a stream started as the process opens (`journal::prepare`) starts from its outermost root's cursor (the newest,
    for two names of one directory), no longer from the oldest of the roots under it. Those held cursors no run asks
    any more (`lib` under `rule-syntax`, `/var/…` beside `/private/var/…`). A root asked with an older cursor than its
    stream's starts its own, as before.

  After: no stream starts from far back, the warm batch over `dl1g` writes nothing it may not, and its 6 asks waited
  0.57 to 1.23 s at load 115.
- **The observer's store kept everything, and every process read it all.** Nothing was ever dropped from it, and
  each process loads all of it as it opens (`read_all`). In the `dl1g` store: 3 of 7 root cursors were of names no
  run asks any more, 7 of 86 answers and 8 of 81 canonical forms were under them, and 54 whole-run cutoff records
  (975 KB) were mostly of earlier builds (the engine's id is in their key). Its database file was 4 KB, with
  everything in a 2.9 MB WAL that was never folded in. Fixed:
  - `observe::collect`, in the content store's collection pass (behind the run, never in a warm run), drops:
    - a root's cursor whose name has a `.` or `..`, is gone, lies under another root kept, or is another name of a
      kept root's directory with an older cursor;
    - an answer, a canonical form or a Git file list under no root kept (every path read is under a root);
    - a cutoff record (now keyed `cutoff:…`) none of whose plans is live in the content store, and one from before
      records named their plans.

    Then it folds the WAL in and truncates it. `constants:` and `cone-rounds:` records stay.
  - The outermost run's end folds the WAL in (`wal_checkpoint(PASSIVE)`: it waits for no reader, commits nothing).
  - The journal's cookies are written in `<store>.cookies`, a directory holding nothing else. Beside the store, each
    stream was told every write there: 2,596 events in one warm batch's first ask, the harness's outputs and the
    store's own writes, all replayed and dropped.
- **Loading the observer's store at open** (`read_all`) rather than by key as asked is left: a lookup in the waves is a
  statement there, which the strict mode refuses, so it needs the keys known before the first wave.

