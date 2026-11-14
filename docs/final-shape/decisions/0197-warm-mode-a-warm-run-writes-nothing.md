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

`list` prints each fault as a line as it happens, the first 60 with their stacks (`sort | uniq -c` groups them). When
the process exits it prints the count and exits with status 3. (A map of distinct faults held by the process was
refused by the no-new-caches check, decision 0104: the lines are the report.)

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
- **Choosing the widest stored root** for a path (`watch_above`) means one ask covers many reads, so there are fewer
  cookie waits. But a read near the top (a work tree's `.git`) makes the whole tree a root. The deepest root would name
  less, at the cost of more asks. Not measured.
