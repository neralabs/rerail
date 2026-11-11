# 0104: Everything kept is facts, on the same fact path

Status: decided (2026-11-10). The checks are built; the moves are in the order below.

## Intro

**The problem.** The engine keeps results from one run, chunk or process to the next in many
places of their own. There are 68 store tables and 24 in-process maps. Each cache picks its own key
(a digest of what its author remembered) and has its own test of whether it is still right:

```rust
// plan/choices.rs: a digest of the program, the build and three switches,
// then "a scale at most GROWTH times theirs".
let key = choices::key(&prog);
// plan/corpus/memo.rs: a digest of the build, the grammar, the node-id
// scheme and the program; a file's rows by (source, path, that digest).
let plan = memo::plan_digest(&prepared);
// plan/corpus/kept_strata.rs: each input's digest, compared in Rust.
if kept.input(rel) == kept.was(rel) { /* the stratum is clean */ }
```

A key that leaves out an input gives a stale hit, and nothing reports it. Each validity test is Rust
code, beside the engine that keeps every other fact right.

**The rule.** A result kept for later use is facts, and it runs through the same fact path as every
other fact:

- one rule makes the **key fact** from what decides validity (a file's content, a program's facts,
  the engine's build);
- one rule makes the **result** from the key, by the expensive native step;
- the engine keeps both like any fact. An unchanged key is no delta, so the native step does not run
  again. A changed key retracts the old result and asks for the new one.

So validity is automatic: it is the engine's maintenance, not code a cache owns. No cache has a
key, a table, a map or a validity test of its own.

## Prior work consulted

- **ast-index-spike design index, the linter survey (LA 6, F3).** The Python linters' fact store
  was content-hash cached, and suffered "all-or-nothing cache invalidation".
  - **Agrees.** Here a key is facts, so an edit retracts only the results whose keys it changed.
- **Lessons ledger row 10.** A stale fact store showed 0, then 9, then 16 findings for one input.
  - **Agrees.** A result kept under a key that misses an input is that failure. Facts derived by the
    engine's maintenance cannot keep a row whose support is gone.
- **Ledger row 14.** Each file owns its rows, so an edit rewrites only that file's rows.
  - **Agrees.** A document's facts are keyed by the document's content facts. The program they come
    from is facts too, so a program edit retracts exactly the rows of the rules it changed.
- **Ledger row 25.** Restoring a stored tape is 20–26x faster than parsing again.
  - **Agrees.** A tape stays a stored result. Its key becomes a fact (the content and the grammar),
    and the tape is the answer to it.
- **Decisions.**
  - 03 §1 already says it: "There is no separate 'file layer' or 'cache layer'; a cache entry is a
    fact with a version." 0092 adds: "the key must also be facts in our own system".
  - 0065 §2a is the native step as a rule. A request relation's rows are answered inside the
    fixpoint by a native answerer, whose rows enter as produced facts.
  - 0083 and 0094: phases are namespaced fact spaces run by the same engine. A phase's state is
    facts of its space.
  - 0090: a session maintains facts by counting. That is what makes an unchanged key no delta.
  - 0102: the writer that moves facts to SQLite behind the run. It is the only transport. Its jobs
    carry facts, never an encoding of a cache's own.
  - A first attempt at this put a cache layer beside the facts: cache tables, a freshness program
    and an observer step. It was withdrawn for that reason, and nothing of it is built on here.

## The pattern

A cache is two ordinary rules, and nothing else: no directive, no table, no read and no fact type
of its own.

```text
// The key: what decides whether the result is still right.
choose(p, b) :- program(p), engine_build(b).

// The result, from the key, by the native step (here a request answered
// natively, 0065 §2a). An unchanged key is no delta, so it is not asked again.
chosen(p, b, rel, keep) :- choose(p, b), choices_of(p, b, rel, keep).
```

Both are relations like every other. They are stored, held in memory, written back and read the
way every relation is. A relation's identity names its rules and what they read, so a rule edit
makes it new. A version, when one is wanted, is a column written in the rule.

1. **Inputs are facts.** A file's content digest, a directory's listing, the engine's build and a
   setting are base facts. Phase 0's observer states them, and it reads only what changed.
2. **A key is a rule.** It joins exactly what decides validity. A key that leaves something out is
   a wrong rule, and it can be read and tested like any rule.
3. **A result is an answer.** A native step makes it from its key. Its rows are produced facts. A
   result that is not rows (a tape, a compiled automaton) is a value in a fact's column, a stored
   object named by its id.
4. **Kept is maintained.** Within a session, the engine keeps the program's facts from run to run
   and maintains them from the inputs' delta (0090). Across processes, the store holds the same facts
   as relation rows. A new process loads them and maintains them from the delta of its observations.
5. **Memory is ahead of SQLite.** The session's facts are the resident copy. The 0102 writer writes
   them behind the run.

## No wave waits for SQL

SQL is in a run's data path only before its first wave and after its last. A wave that runs, reads
or waits on SQL either waits for SQLite or reads rows it was not given. So what a run reads from the
store is read before the first wave. What it writes goes to the writer (0102). Meanwhile, memory
holds it.

The profile reports every statement that a wave's thread runs, and every wait for the writer, under
"Forces SQL waits (hazards)". Each is a hazard to remove.

## The tapes are facts

A document's syntax is two ordinary rules too. They are phase 1's (`typing/programs/sources.rr`):

```text
// The key: a source file's content, its grammar and the tape's version.
doc_key(p, c, g, "tape-1") :- observer__source_file(p, g, c).
// Each key asks for its file as a document, answered inside the fixed point.
load(p, p, g) :- doc_key(p, _, g, _).
// The result: the loaded document's tape.
doc_tape(p, t) :- doc_key(p, _, g, _), loaded(f, p, g), doc(d, f, p), tape_of(d, t).
.cache doc_tape by doc_key
```

The run's phases:

| Phase | What it does |
|---|---|
| 0 | loads the rule files and the rule libraries (`bootstrap.rr`) |
| 1 | loads every source file, finds the libraries, and runs the imports to their fixed point |
| 2 | runs the typing to its fixed point |
| 3 | the analysis |

Then the findings are written. Every later phase takes its documents from phase 1: it does
not list, load or parse a file itself.

A library's tape is the same two rules. Its key names the file an import resolves to, by
its content (`file_digest(path, c)`, the file's SHA-256, read through the observer):

```text
// The key: the file module q resolves to, by its content.
library_key(q, f, c, "tape-1") :- py_resolves(q, f, _), file_digest(f, c).
// The result: the tape of the document its load gave.
library_tape(q, t) :- library_key(q, _, _, _), loaded(f, q, "python"), doc(d, f, q), tape_of(d, t).
.cache library_tape by library_key
```

What `parse_of` answers, and what the file-local part reads of it, are a document's facts. They
are stored where a document's facts are stored, as every relation is, and read with them.

- **No row ids.** A document's content is its digest. A row id that SQLite assigns is never a value
  that a wave holds, because it would be known only after a write.

## The work

Each step is checked on the slice by the profile's hazards (count and time), the findings (cold,
warm and after an edit), CPU (user + sys, with the load) and the pins of `facts_only`.

| # | What | Done when |
|---|---|---|
| 1 | The tapes as the two rules above, for a run's files and its guests; contents by digest; the tape and source writes are the writer's | **built** (below): phase 1 loads every source file by its key; no tape or source statement of a file or a guest among the hazards, and the 4.2 s wait is gone |
| 2 | The memo's entries read before the first wave; a lookup writes nothing | **built**: no memo statement among the hazards of a cold run or of a run after an edit (below) |
| 3 | The constants, the statistics and the choices read before the first wave, written by the writer | no store statement among the hazards |
| 4 | A program's stated facts and `rdl_int` bound in the engine, not loaded through SQL tables; inputs read from where they are held | **built**: on the slice, one statement is left on the run's connection (below) |
| 5 | A test runs a small corpus with the profile on and fails on any hazard | the test passes with none |

**Step 1, as built (the reads and writes).** A wave runs no tape or source SQL:

- before the first wave, the run reads the tapes of the files the memo has no entry for, as
  stored frames, in one read. Each is decompressed in its file's wave;
- in a wave, a file's tape is that frame, or one this process has queued, else the file is
  parsed. Its source and tape are handed to the store's writer, and memory holds the tape
  until it is written;
- a content the store does not hold yet is named by its digest. The memo's entries, the stat
  index and the kept statistics name it so, and the writer finds its source, written before
  them, when it writes them. The next run finds it by digest;
- a guest's tape is made the same way.

On the slice, cold, run side by side with the run before this step: the hazards went from
11,076 statement runs and 4,065 ms to 204 runs and 27 ms; CPU (user + sys) from 22.3 s to
17.7 s, and wall from 25.2 s to 16.2 s, at a load of 50 to 100. The findings are the same, cold,
warm and after an edit, and on a second program over a cache the first one filled (its tapes
hit).

**Step 1, as built (phase 1).** Every run that is not itself a phase runs phase 1 first:

- the observer states each source file of the run's grammar, with its content's digest. It
  reads a file only where its journal, stat or Git index cannot vouch for it;
- phase 1's run is given no file. It loads each one by its key. Its rows are kept as any
  run's: its per-file memo, and its cone of requests in the frames;
- its export, `document(path, grammar, content)`, is the run's list of documents. A file it
  made no document of is listed as before, so that the run reports it.

A memo entry's key is taken under the earlier phases' rows only where a rule it depends on
reads them. Phase 1's keys feed no entry, so an edit misses only the edited file's entry.

On the slice, the findings are the same as before phase 1, cold, warm, after an edit and on
a fresh cache. After an edit, both phases read 485 of 486 files from their memos, and only
the edited file is parsed. A warm run is still the whole-run cutoff (0.37 s CPU); phase 1's
program is among the rule files its record checks. Cold, CPU is within the load's noise of
the run before (21.5 s, user + sys, at a load of 15 to 45).

Phase 1 still asks for every key's file on every run. An unchanged file's answer is its memo
entry, not a parse or a build. Asking only for the keys that changed needs the cone to be
maintained from its delta, not kept as a whole stratum.

A run inside another (phase 0's, phase 1's, the constants') leaves the store's collection to
the outer run. The collection's thread works behind the run, as the writer does: the waves
read nothing it writes, and its statements are not among the hazards.

**The imports, in phase 1.** Phase 1 holds the import fixpoint (`python/modules.rr`). It
loads the run's own files under their modules, and states them as the workspace's
(`py_own`): an import of one loads nothing. A library an import resolves to is loaded inside
the fixed point, and its imports are resolved in turn. The phase ends when the import
frontier is closed. It runs first, before the phases a program reads. Its result is held
for the run (`PhaseOne`):

- every run inside the run, a phase's included, takes its documents from it;
- a program that reads `sources__library(path, module, grammar, content)` (the structure
  phase, `struct.rr`) takes those library documents as its own loaded documents, named by
  their modules. It makes no request and loads nothing;
- a whole-run record holds what phase 1 consulted: the directories it listed, the files it
  read, the libraries it loaded and the paths it found missing. A change to any of them
  runs the program again.

A rule of a file's own part that joins an earlier phase's rows puts all of those rows under
every file's memo key. Phase 1 takes a file's module from a derived relation
(`doc_module`), so its per-file part joins no observer row.

On the slice (no typeshed, so no library), the findings are the same with phase 1 on and
off, cold, warm, after an edit and on a fresh cache. Cold, CPU is 33.1 s with phase 1 and
28.7 s without (user + sys, at a load of 60 to 75). Phase 1's own run is 1.35 s of it. Each
file's facts are built twice, once in each phase (977 builds, not 486), for 0.2 s. Holding
phase 1's built documents for the later phases would keep about 16 MB per 500 files for the
run. After an edit, phase 1 costs 0.46 s of 16.2 s: it asks for all 486 files again, each
answered by its memo entry.

Two source statements are left among the hazards. The planning choices and the measured
constants are kept under a small text stored as a source. They are step 3's.

**Step 2, as built.** Before the first wave, the run looks up every file of the corpus:

- it stats each file. A file whose stat the store recorded is that source, and is not read;
- it reads and hashes the others, in parallel, and finds their sources in one read;
- it finds the entries of all of them in one or two reads, as stored frames. Each is
  decompressed and decoded in its file's wave, so memory holds compressed entries, not
  decoded ones;
- for the statistics keeper, it also finds the entries of the files changed or gone since
  their counts were kept.

A lookup writes nothing:

- a source found is pinned by the store's writer, behind the run;
- a new source is not stored by the lookup. It is stored with its tape, in the tape's
  transaction, which already held it;
- a file read again in its wave is checked against its stat. If it changed since it was
  looked up, its content is new, and it is stored with its tape.

A load's files are known only in the waves, so they are still looked up there. On the
slice, nothing is loaded. A session's memo is in memory, and reads no SQL.

On the slice, the lookups' 488 transactions on the parse worker are gone. After one file
is edited, 485 of the 486 files are memo hits, and the run's hazards are 212 statement runs
and 37 ms, against 248 and 225 ms before. The findings equal a cold run's of the edited
files. Run side by side on the same machine, the run before and after this step take the
same CPU and wall.

What is left inside the waves is the tape store (step 1), and the choices and statistics
read from the store (step 3).

**Step 4, as built.** Within a native run, the run's connection makes nothing in SQL that SQL
does not read:

- a relation of stated facts alone is held in the engine. A relation with rules and stated facts
  has its facts apart (`rel__stated`), read by one rule;
- a native run makes no views. A projection of an input is a stratum like any, run from the input
  where it is held;
- a range read computes a document's node-id interval from its key, when the run's documents are
  numbered by key (the corpus runner's). The guests and the loaded documents write no intervals;
- `rdl_int` is a series in the statement that reads it;
- a connection's state (what its runs left in SQL, its settings, how its documents are numbered)
  is kept with the connection and dropped when it closes. A run asks SQL nothing it knows.

On the slice, the run's connection went from 208 table statements, 91 inserts, 11 input reads and
one rule run in SQLite, to one `PRAGMA` on the first run of a guests' connection.

How relations are stored and read back is shared work. rr-align's frames (one stored form, each
relation by its identity, written only by the writer) are that form. As each of the stores above
moves onto it, its hazards here go with it. Nothing on this list adds a store or a read of its own.

## What is checked

`logic::facts_only` reads the engine's source:

- **Every table** that the store or the logic engine makes is listed. It is the fact store, a
  scratch table of one statement, or a cache not yet moved.
- **Every map** behind a lock or a cell in the engine's run path is listed. It is state (registries,
  queues, switches, profiling, the engine's facts within one run) or a cache not yet moved.
- A table or map in no list fails, and so does a listed one that is gone.
- The not-yet-moved lists may only shrink. Their sizes are pinned (68 tables, 24 maps), and each move
  lowers its pin.

## The moves

Each cache is listed with its step in `facts_only.rs`.

| Step | What moves | Becomes | Where |
|---|---|---|---|
| 1 | the run's constants, the whole-run cutoff, the observer's record, the derivation tables (7 tables, 2 maps) | the observer's base facts and rules over them | final-rerail-shape-rr-align, which owns it |
| 2 | rule files and programs: parsed files, libraries, phase 0's graph, the combined and prepared programs, the planning choices (7 maps) | phase 0's facts. A rule file is loaded on a `load` request. The combined program, the prepared program and the choices are answers keyed by the program's facts (0074) and the build | this branch |
| 3 | a document's facts: the per-file memo, manifests, the cross-file memo, the first slice's kept matches, guest parses, the session's held rows (21 tables, 2 maps) | a document's facts in its phase's space, keyed by its content facts and the program's facts | this branch. The per-file memo is done: frames in its plan's space, keyed by content and path (0108; t32 5.1–5.2 measured) |
| 4 | the kept strata (2 tables, 1 map) | the cross program's own relations, kept and maintained from the delta of its input facts | final-rerail-shape-rr-align: the 2 tables are moved to the store's frames (notes/t33, K1); K4 is built (fe23a4986): a new process loads the session's retained state from the frames, maintains it from the delta of its inputs, and writes back what changed. The cone of requests still reruns as whole strata |
| 5 | the phase store (5 tables) | a phase's exports as its space's facts | the typing branch (investigate-bend-vs-datalog), as its plan already says |
| 6 | planning: kept plans, derived values, observed steps, the tables a chunk builds ahead, compiled plans, the statistics (14 tables, 7 maps) | planner facts. Statistics are maintained aggregates over the base facts. A plan is an answer keyed by its rule's facts and the statistics it read | this branch |
| 7 | text patterns and indexes: stored automata, compiled matchers, the prefilter and vocabulary indexes (10 tables, 5 maps) | answers keyed by the pattern's facts. An index is a derived relation over tapes | this branch |
| 8 | the workspace pipeline's result pages and check results (9 tables) | moved with that pipeline, or retired with it | that pipeline's owner |

Step 2 comes first here, because every other step reads programs. A program as facts is the key
that steps 3, 4, 6 and 7 join.

## Compatibility

- **rr-align** keeps step 1, which includes the observer, `World::still`, `inputs::still` and
  `git::still`. Its withdrawn cache layer is not rebuilt. Its step 1 states the base facts that the
  other steps' keys join.
- **The typing branch** keeps step 5 (its plan's stage 2). Its plan's principle 2 (write-back carries
  facts only) is this decision's pattern rule 5.
- **Write-back (0102)** stays the transport. As each cache moves, its job stops encoding a payload of
  its own and carries relation rows.

## Open

- **Loading across processes.** Rule 4 needs a stored fact space to be loaded and maintained from the
  delta of its inputs. That is 0083 §5's stored fixpoint (O131). Steps 3 and 4 build it. Until then, a
  new process derives what step 2 and later steps keep.
