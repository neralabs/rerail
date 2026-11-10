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

A document's syntax is two ordinary rules too:

```text
// The key: a document's content and its grammar.
doc_key(d, c, g) :- doc(d, p, _), file(p, c), grammar_of(p, g).
// The result: the parser's answer for the key.
doc_tape(d, t) :- doc_key(d, c, g), parse_of(c, g, t).
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
| 1 | The tapes as the two rules above, for a run's files and its guests; contents by digest; the tape and source writes are the writer's | **the writes and reads are built** (below): no tape or source statement of a file or a guest among the hazards, and the 4.2 s wait is gone. The two rules are not built yet |
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
| 3 | a document's facts: the per-file memo, manifests, the cross-file memo, the first slice's kept matches, guest parses, the session's held rows (21 tables, 2 maps) | a document's facts in its phase's space, keyed by its content facts and the program's facts | this branch |
| 4 | the kept strata (2 tables, 1 map) | the cross program's own relations, kept and maintained from the delta of its input facts | final-rerail-shape-rr-align: the 2 tables are moved to the store's frames (notes/t33, K1); maintaining from the delta is K4, next there |
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
