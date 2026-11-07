# 0063: A long-lived run keeps the program compiled and each file's rows; an edit reruns what it changed

Status: accepted (2026-11-07; built and measured in the explorer)

## Intro

The rule explorer runs a rule over the files on its page each time the reader types. Each run used to
be a whole engine process: a new WebAssembly instance, the rule read and lowered, every file parsed and
evaluated, then the cross-file part. On one 20-line file that was about 190 ms in the engine and
235 ms from a click to the findings. The page waited 450 ms after the last keystroke before starting,
so the findings appeared most of a second after typing stopped.

Almost none of that work depends on the keystroke:

- The rule did not change, yet it was combined, simplified, lowered and split again (the "stats"
  phase, plus the flatten before it).
- The other files did not change, yet they were parsed and evaluated again.
- About a hundred `CREATE TABLE` statements on the cross-file scratch database each wrote a journal
  through the page's filesystem shim.

This record keeps what does not depend on the edit, so a run after an edit costs what the edit changed.

```text
// one instance of the engine for the page's life
logic rule.rr src/ out/     // first run: compile the program, evaluate both files
logic rule.rr src/ out/     // after an edit to a.py: the program as compiled,
                            //   b.py's rows from the memo, a.py evaluated, then the cross-file part
```

## Decision

### 1. A session

A process that runs the same program again and again starts a session (`corpus::session`,
`RERAIL_SESSION=<dir>`). The session keeps three things:

- **Its store**, opened once in `<dir>` and never deleted. Its atoms are never given out again
  (decision 0049), so they mean the same thing in every run. Each run's SQL connection opens the
  same file, so it shares those atoms.
- **The last program**, combined (`combine`, keyed by its texts) and compiled (`Compiled`, keyed by
  those texts, the inputs the corpus settled through `external_inputs`, and the dialect). A run with
  T's facts or typing is not a session's.
- **A per-file memo in memory.** These are decision 0052's entries under the file's content digest,
  path and file-local program digest. They are cleared when the program changes, or when they pass
  4096 entries.

A reused compile states the decisions it made when it ran (`profile::decisions_since` and
`add_decisions`). The explorer's Profile view is therefore the same on every run.

A run that should show every step runs outside the session: `logic --full` (`session::full`) combines,
compiles and evaluates every file as a process without a session would, in a store of its own, and
leaves the session as it was. The explorer's **Full run** button, beside the output tabs, does this,
so Join plans, SQL and Profile show the whole run; the runs an edit starts stay incremental, and the
explain views say which kind of run they show.

### 2. What compiling is

`Compiled::of` is the part of `run_with_options` that depends on the program alone:

- simplify (0038), demand for negated tests (0048) and text tests as relations (0037);
- lower, the `nearest` sets, file-local joins pushed into the chunks (0051), `locate`, and the
  file-local / cross-file split;
- the memo program digest, the relations read, the projection (0046) and the constants `Own`
  measures.

Everything that depends on the files or the connection stays in the run: documents, statistics, the
`x` tables and the bindings. A run without a session compiles and runs in one go, as before.

### 3. Base relations are memoised too

The memo used to be off whenever the cross-file part read base relations (0052's "Open"), because
those rows came only from evaluated chunks. Most single pack rules do this.

An entry now also holds the file's rows of each base relation the cross-file part reads, in the
columns it holds. A row is attributed by its first column, a node or a document key:

- `node(d, …)`, `doc(d, …)` and `docof(n, d)` give a document;
- `within(n, a)`, `field(p, f, c)` and the others give a node.

A row that names no document of its chunk turns the memo off for that chunk, as for exports.
`docof`'s second column is now a location (`split_located`), so a relation `locate` gives a document
column through `docof` has one.

A memoised run measures these relations once, over every file's held rows, before the cross-file
part plans. A column the part never names is stated as keys, and a relation no file has rows of is
held empty. The memo's schema is 4.

### 4. A run's scratch has no journal

`x` (`cross.sqlite`) is deleted at the start of every run, so a run that stops leaves nothing to
recover. It runs with `journal_mode = OFF` and `synchronous = OFF`, and temp tables are in memory. In
the browser build `x` is `:memory:`, so its pages skip the filesystem shim's calls.

### 5. The browser's engine is a reactor

The CLI's dispatch is `tools::cli`. The binary calls it once per process. The library, built for
`wasm32-wasip1` with `crt1-reactor.o` (native/wasm/README.md, "A long-lived engine"), exports
`rerail_command`, which calls it once per call, on one instance and one in-memory filesystem.

The explorer instantiates the engine once and mounts the files in place for each run. It flattens
the rule only when the rule's files change, and starts a run 16 ms after an edit lands. Edits made
during a run are taken together by the next one. After a trap it starts another instance.

## What the prior work says

- **"Key memoized analysis by what it is a function of, never by object identity"** (history index
  §2.16(d); S238). The session keys the program by its texts and settled inputs, and the memo by
  content, path and the file-local program. Nothing is keyed by run. *Agrees.*
- **"Incremental indexing allows no corpus-wide statistic"** (design index §10; S124). The cross-file
  statistics of a memoised run are measured over every file's held rows on each run: work in
  proportion to those rows, not a stored corpus-wide figure. *Agrees.* That pass is the cost a large
  corpus would see (Open).
- **Restoring a cached tape is 20–26x faster than reparsing** (lessons ledger #25). A memo hit goes
  further: a hit file is neither restored nor evaluated. *Agrees, and extends.*
- **Decision 0014:** a warm run evaluates only changed documents, there in the slice's waves. The same
  shape now holds for the corpus runner a page drives. Decision 0044 §4 keeps a lowered program per
  process while the files it read hash the same; the session extends that from parsing to compiling.

## Measured (2026-11-07)

The explorer in the Orca browser (Chromium), served locally, load average 68–84 on 18 cores. The
browser has no CPU clock, so these are wall times on the worker's thread.

| Run | Before | After |
|---|---|---|
| The weak-hashes scenario again, unchanged: engine | 190–200 ms | 30–40 ms |
| The same, click to findings shown | 230–240 ms | 39–43 ms |
| An edit to its one file: keystroke to findings | 450 ms wait + ~235 ms | 60–110 ms |
| An edit to one of pickle-modules' two files: engine | (not measured) | 90 ms; 1 of 2 files from the memo |

The page also draws the Program, Join plans, SQL and Profile views only when shown, and the program
only when its text changed.

The 56 scenario rules were each run twice natively over one tape cache. The second pass read 52 of
them from the memo, and every rule's findings were the same in both passes and the same as before the
change. A session test (`a_session_reruns_what_an_edit_changed`) runs three times: the second run reads
both files from the memo, and the third, after an edit, evaluates the edited file only. Each run's
findings are a fresh run's.

## As merged into the final shape (2026-11-07)

This record was written as 0056 on the explorer's branch. On the final shape that number was already taken (bounded
rounds), so it is 0063 here. The session merged onto decision 0060, which keeps a run's fixed costs for the process,
so two parts of §1 and §2 are built differently:

- **The program is not the session's.** `Compiled` is not kept. The session holds its store, its path and its
  in-memory memo (`session::Session`). The program comes from 0060's caches, which every process has: `combine` by
  its texts, and the prepared program (`Prepared`) by its texts, additions, stage and dialect. That program's atoms
  are checked against the run's store, here the session's. An edit to a file still neither parses nor lowers the rule
  again.
- **What a reused program states.** A reused prepared program states its demand, pushed-join and merge decisions
  again from what it kept, not through `profile::decisions_since`. `logic --full` runs outside the session but still
  in the process, so it reuses the prepared program when its store's atoms agree.
- **Unchanged from §1:** the in-memory memo (`memo::Memo::Held`, by content digest, path and file-local program) and
  the base rows a memoised file hands the cross-file part (§3). The stored memo (`memo::Memo::Stored`) keeps 0053's
  stat index, held plan and manifest. A session's run records no manifest, because its sources are not in the tape
  cache.

## Open

- **The cross-file part runs in full on each run.** It creates its views and tables and re-plans
  every stratum: 20–30 ms of an edit. Keeping its connection, tables and plans across a session's
  runs, and evaluating it incrementally, would remove most of that.
- **An edited file's evaluation** (the file-local strata over one file) is the other 30–50 ms. That
  cost is per stratum, not per node.
- **Statistics on a large corpus.** A memoised run measures the base relations the cross-file part
  reads over every file on each run (§3).
- **Memory.** The session's memo and the instance's memory grow while the page stays open. The memo
  starts again empty at 4096 entries, but the instance's memory does not shrink.
