# t25. Phase 0 as a run of `bootstrap.rr`

## Intro

Phase 0 loads the rule files a run needs. Until t24 the engine did this in Rust: it read each file, followed its
`#include` lines, and pasted the modules into one text. What was asked for phase 0 (2026-11-08) is different:

> We should use the same mechanism for loading of our rerail source files and the source being analyzed: add a
> path, and have that turn into facts, follow included modules, and eventually get to a fixed point where all the
> libraries are loaded. That needs to be phase 0.

Also asked: Git for whole trees, `stat` (inode, mtime, size) with an occasional validating scan, and no
environment variable to choose the provider. Watchman is for later.

This note gives what is now built on `investigate-bend-vs-datalog`. Phase 0 is a corpus run of a rule program,
`docs/rule-syntax/lib/bootstrap.rr`, over `.rr` documents. The note also covers how a program is keyed, how much is
reused when something changes, and what it costs.

## What phase 0 does now

The engine starts phase 0 from a few builtin facts and runs `bootstrap.rr`. The run ends at a fixed point where
every module that some program includes is loaded.

```
// builtin facts (from the engine)
env("rerail.lib", "/…/docs/rule-syntax/lib")     // the library: a root the loader may read
env("rerail.program_dir", "/…/typing/programs")  // a program file's directory: also readable
p0_file("/…/typing/programs/calls_cx.rr")        // a program file the engine was given
p0_text("text:3"). p0_ask("text:3", 0, "python/ast.rr")   // a program given as text, and what it includes
```

`bootstrap.rr` then does the rest:

```
// A rule file is loaded like a source: the loader answers, the file's tree becomes facts.
load(f, f, "rr") :- p0_file(f).
// Its #include lines are facts of that tree (`include` nodes) ...
p0_names(i, f, at, n) :- kind(i, "include"), field(i, "path", p), str(p, n), docof(i, d), doc(d, f, _), node(d, i, _, at, _).
// ... each name has candidates: the library first, then the includer's own directory ...
p0_cand(f, at, n, 1, c) :- p0_from(f, at, n), p0_lib(l), c = cat(l, "/", n).
p0_cand(f, at, n, 2, c) :- p0_from(f, at, n), !p0_text(f), f != "engine", match("(?<dir>.*)/[^/]*", f), c = cat(dir, "/", n).
// ... the first candidate that exists is loaded, which brings in its own includes, to a fixed point.
load(c, n, "rr") :- p0_resolved(_, _, n, c).
```

The search order, the implicit `core.rr` (not for programs that declare `doc`, T's shape), and the engine's own
`components.rr` are all rules in `bootstrap.rr`. The engine reads three outputs:
- `p0_include(from, at, name, path)`: the module graph;
- `p0_missing(from, at, name)`: an include with no file, reported with the old message;
- `p0_absent(path)`: a candidate that was not there. A file appearing there would change the graph, so it is
  part of the graph's key.

The Rust reader that did this before is kept only to load `bootstrap.rr` and `core.rr`, the two files that must
exist before phase 0 can run (`rr::builtin_resolve`, used while `phase0::active()`). A test checks that phase 0
gives the same order as that reader on 40 library modules, on texts with two includes and in T's shape, and on a
missing include (`phase0::tests::bootstrap_resolves_as_the_builtin_reader_does`).

## Rule files are stored as sources are

A module that `bootstrap.rr` loads goes through the same path as a Python file that a resolution rule loads:
- the world's `load` answer;
- the observer's `stat` check;
- the chunk builder, which parses it and stores its tape in the tape cache;
- the per-file memo, which keeps its rows of the per-file relations.

The program is then assembled from those stored modules. `rr::modules_of` reads each module's statements off its
stored tape, so the text of the library is not read or parsed a second time. Nothing is kept on a separate path
for rule files.

## No environment variable

- `RERAIL_FILES` is gone. A root inside a Git work tree is listed by Git, scoped to the root's subtree: only
  HEAD's subtree at that prefix is visited, and the status call takes a pathspec. Without scoping, the 84-file
  fixture ran 3.4× slower, because each run took the status of the whole repository. A root outside a Git
  tree is walked.
- `RERAIL_RUN_REUSE` is gone. The whole-run cutoff is a run option (`"reuse": true`). The CLI and the pack
  loader set it, and phase 0 sets it for its own run.

## How a program is keyed

A program used to be keyed by its text, including its `#include` lines. Two programs with the same lines but
different library contents had the same key. An edited library module could then reuse a combined program
built from the old one within the same process.

Now the key folds in the graph phase 0 found: each module's path and digest, the edges, and the missing names
(`texts_digest` in `corpus.rs`, through `rr::graph_digest`). This one key is used by:
- the process cache of combined programs;
- the prepared-plan pointer;
- the whole-run record.

So:
- **No library file changed:** the same key, and the run is kept.
- **A module changed** (even only a comment): a new key, and its programs are prepared again. Other programs
  keep theirs.

## Partial reuse

Reuse works at four levels:

| level | what is kept | when |
|---|---|---|
| a seed's graph, in the process | `phase0::KEPT` | while each module's stat holds and each absent candidate is still absent |
| phase 0's run, across processes | the whole-run cutoff on `bootstrap.rr`'s run | nothing it read changed |
| one changed rule file | the per-file memo for every other module | phase 0 runs again; only the changed file is parsed |
| the main run after a source edit | phase 0 and the program's key, then the per-file and cross-file memos | the edited file and the import components that reach it are derived again (t22) |

What is not partial yet:
- **A rule edit prepares its program again.** Edits that lower to the same program (comments, layout) still find
  every memo (16 s of main-thread CPU, below). An edit that changes a rule misses every memo of that program,
  including the rows of rules the edit does not touch.
- **The main run after a source edit is still all or nothing below the memos.** Typeshed's rounds run again.
  Keeping them needs phase 0's outputs to be one phase's exports, maintained as a delta (0083's phased facts),
  so that a later phase sees a change rather than a new world. Decision 0094 defers how that delta is
  maintained.

## Figures

CPU is user + sys. The machine was shared: load average 18–92 during these runs.

**Phase 0 alone, the Python pack** (546 rule files, 537 texts with includes, 53 modules; one `bootstrap.rr` run
for every text; `phase0::benchmarks::benchmark_phase_0_rules`):

| run | thread CPU (read files + graphs) | process user + sys | peak RSS |
|---|---|---|---|
| cold store | 1.38 + 0.55 s | 3.4 s | 138 MB |
| warm | 0.15 + 0.14 s | 0.29 s | 70 MB |
| library and pack copied, cold | 1.30 s (files) | 2.8 s | 139 MB |
| copy, warm | 0.13 s (files) | 0.26 s | 75 MB |
| copy, comment added to `python/conditions.rr` | 0.15 s (files) | 0.29 s | 75 MB |
| copy, the same again | 0.15 s (files) | 0.30 s | 75 MB |

A comment edit to a module costs no more than a warm run here: phase 0 parses the one file again, and every
other module comes from its memo.

**The whole run, `calls_cx` on the 84 files** (with typeshed; both memos and the cutoff on):

CPU of the main thread is the run's report. The process figure is `/usr/bin/time`'s user + sys. The load average was
92 for the prime and 18–25 for the rest. t24's prime, before this change, took 75.0 s and 86.7 s at load 10–15.

| run | memos | CPU main thread | process user + sys | phase 0 | answers |
|---|---|---|---|---|---|
| prime (cold store) | 0 found, 133 derived | 91.1 s | 106.1 s | 367 stats, 273 hashed, 34 listings | 6,299 agree |
| warm, no edit: kept | none run | 0.6 s | 1.4 s | 354 stats, 341 reused | 6,299 |
| the same again | none run | 0.6 s | 1.4 s | 354 stats, 344 reused | 6,299 |
| a library copy, first run | 133 found, 0 derived | 17.9 s | 29.0 s | 380 stats, 143 hashed | 6,299 |
| the copy, warm: kept | none run | 0.4 s | 1.0 s | 354 stats, 345 reused | 6,299 |
| comment added to the copy's `python/conditions.rr` | 133 found, 0 derived | 16.1 s | 29.2 s | 389 stats, 25 hashed | 6,299 |
| the same again: kept | none run | 0.6 s | 1.4 s | 354 stats, 343 reused | 6,299 |

- **The graph is the key.** A comment in an included module changes the module's digest, so the whole-run cutoff
  misses and the program is prepared again. A run against a different library directory (the copy) misses for
  the same reason.
- **The memos still hold.** The comment lowers to the same program. The per-file and cross-file memos are keyed
  by the lowered program, so all 133 import components are found and none is derived. The edit costs what t24's
  warm run without the cutoff cost (14.5 s then), not a cold run.
- **An edit that changes a rule misses the memos.** It changes the lowered program, and a cold run (about 90 s
  here) follows. Reusing what that rule does not touch needs maintained fact spaces; see Partial reuse.
- **A kept run costs 1.0–1.4 s,** against 0.9–1.1 s in t24. The extra is the stat checks of the modules phase 0
  loaded (354 against 315 stats).

## Where the time goes: debug against optimized

All the figures above are from the dev build. It optimizes the dependencies (SQLite, tree-sitter) but not
the engine's own Rust code: the native joins, statistics, memo replay and SQLite glue. Its profiles are
dominated by the standard library's debug checks (`ub_checks`, `precondition_check`). The `profiling` profile
(Cargo.toml: opt-level 3, incremental, no LTO) builds the same code optimized. Same corpus and program
(calls_cx, both memos), with every answer the same (6,299 agree):

| run | dev build, user + sys | `profiling` build, user + sys |
|---|---|---|
| phase 0 alone (`bootstrap.rr` for calls_cx.rr), new store | 0.53 s | 0.14 s |
| phase 0 alone, warm | 0.07 s | 0.03 s |
| calls_cx, new store | 117 s | 19.1 s |
| calls_cx, warm, the cutoff off (both memos found) | 23.6 s | 4.4 s |
| calls_cx, kept by the cutoff | 1.4 s | 0.25 s |
| struct.rr as its own phase (P1, typeshed loaded in it), new store | 16.4 s | 3.2 s |
| struct.rr, unchanged / a comment in `exceptions.py` | 0.1 s / 8.7 s | 0.1 s / 1.4 s |

Where the optimized cold run's 16.1 s of main-thread CPU goes:
- the cross-file part, 6.8 s: the 53-relation stratum of calls, values and bindings is 2.8 s (75.7 M probes for
  2.8 M rows);
- the file-local part, 5.7 s, of which 2.3 s moves the chunks' exported rows to the cross-file part;
- statistics, 2.1 s;
- storing the import components' rows, 0.53 s (13.9 s in the dev build, as a row per insert);
- typeshed's load requests, 0.9 s in 21 pauses of the fixpoint;
- parsing the 267 files, 0.21 s.

The warm run's 3.4 s: memo replay into SQLite 1.5 s, typeshed's load requests 0.9 s, statistics 0.8 s.

### The first fixes, in the dev build

Cold runs of calls_cx on the 84 files, profiled. The load average was 70–157, so CPU differs by up to a quarter
between runs of the same code. Every run agreed on all 6,299 answers.

| part | before | after | what changed |
|---|---|---|---|
| moving the chunks' rows to the cross-file part | 10.5 s | 7.9 s | each run of one document's rows finds its owners once, and rows are gathered by file and handed on once (before: a list allocated and a relation's name copied for each row) |
| storing the import components' rows | 3.5 s | 3.0 s | one sort of the rows tagged by component; a column's last checked document is kept |
| statistics | 11.9 s | 7.1 s | the prepared plan's facts are stored as they are, not parsed back from their own text (3.6 s) |
| the file-local split, within statistics | 1.84 s | 1.73 s | a rule is checked again only when a relation it reads was found cross-file |

What remains:
- **Statistics, own time, 4.4 s.** Not yet attributed.
- **Pushing the file-local joins, 1.7 s.** It runs the split five times: twice for each of the two pushes, and
  once to check whether the cross-file part reads base relations.
- **The 53-relation stratum.** With no component found, its 1,986 `!rr_cross_doc` guards are now dropped
  (`cross_memo::drop_guards`). This halves its join work (72.9 M to 39.6 M units). Two cold runs side by side, one
  binary keeping the guards and one dropping them, under the same load: 20.5 s against 18.4 s for the stratum's joins,
  and 109.2 s against 104.6 s for the process (user + sys). The stratum's 12.7 s before the change was at a lower
  load. What remains in it is the joins' own lookups (`RowRef::at`, `Stored::rows_of`, index sorts) and the debug
  build's checks.

## Open items

- O144: a relation the request cone derives is written empty as an output. `bootstrap.rr` outputs copies.
- O145: a per-file relation with no node column is lost for documents that `load` brings in. `p0_names` carries
  its node.
- O146: tests against the shared default tape cache wait on its atoms lease.
- O147: each phase-0 run costs about 0.25 s, so callers batch their seeds.

One fault was found and fixed while measuring. A profiled run with the cross-file memo deadlocked: the outer run
held the profile (`profile::one_run`), and `cross_memo::add` then started a phase-0 run for `components.rr`,
which waited for the same lock. Now a phase-0 run neither holds nor resets the profile, and the main run
resolves `components.rr` together with its texts.
