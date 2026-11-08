# 0082: A run can be dumped before it is stopped, and the profile counts memory

Status: built (2026-11-08)

## Intro

A run that is stopped by `timeout` left nothing behind: the profile (`RERAIL_PROFILE`) is written when the run ends,
so a hung or very slow run said nothing about where it was. The profile also counted only CPU, while the
1 GB budget is about memory.

This record adds two things:

- **A dump of a run that has not ended:** what has been recorded so far, what every thread has open, the SQL
  statement it is running, and every thread's native stack.
- **Memory per span:** the bytes each span allocated and freed, and which spans raised the process's peak.

```
timeout 115 ──SIGTERM──► rerail-watch thread ──► <profile dir>/interrupted/
RERAIL_DEADLINE=100 ────► rerail-deadline thread ──► <profile dir>/deadline/      profile.txt  (open spans first)
SIGUSR1 ─────────────────► rerail-watch thread ──► <profile dir>/snapshot-<n>/    stacks.txt   (sample / eu-stack / gdb)
                                                                                  spans.tsv, sql.tsv, native.tsv
```

## Decision

### 1. Every thread's open spans are readable from another thread

A thread's span stack was a `thread_local` only that thread could read. It is now an entry in a registry
(`profile::THREADS`): the thread's name, its Mach port (for its CPU, read with `thread_info`), its jemalloc counters,
its open spans, and the SQL statement it is running. SQLite's trace hook (`corpus::Statements`) sets the statement
when it starts and clears it when it ends. All of this is recorded only when profiling.

`profile::open_spans()` lists each thread's open spans, outermost first, with the CPU and bytes each has taken so
far. A running statement is shown with its wall time so far, and its query plan when one was noted.

### 2. A watcher thread takes the stop signals

`profile::watch()` is the first thing `rerail-engine` does. It blocks SIGTERM, SIGINT, SIGHUP and SIGUSR1 in every
thread, and one thread (`rerail-watch`) takes them with `sigwait`, so a dump runs as ordinary code, not in a signal
handler:

- **SIGTERM, SIGINT, SIGHUP:** dump into `<dir>/interrupted`, then exit with 128 + the signal (`timeout`'s 124 is
  unchanged).
- **SIGUSR1:** dump into `<dir>/snapshot-<n>` and go on.
- **`RERAIL_DEADLINE=<seconds>`:** a snapshot into `<dir>/deadline` at that wall time, for a run stopped by SIGKILL,
  which cannot be caught.

`<dir>` is the profile directory, else `RERAIL_DUMP`. With neither, a stop signal stops the run as before and says
how to get a dump.

A dump is the profile report of what is recorded so far: spans that closed, statements that ended, plans and the
explain section, memory. The open spans come first, since they are not in the totals yet. `stacks.txt` holds every
thread's native stack: from `/usr/bin/sample` (one second) on macOS, and `eu-stack` or `gdb` elsewhere. A tool that
does not finish in 30 s is stopped.

### 3. Spans count memory

Each span records:

- **Bytes allocated and freed by its thread** while it was open, read from jemalloc's per-thread counters
  (`thread.allocatedp`, `thread.deallocatedp`). These are two loads per span, not a sampling allocator. Own bytes
  are less the spans inside it.
- **How far the process's footprint high-water mark rose** (macOS `ri_lifetime_max_phys_footprint`; elsewhere the
  peak resident set). The spans that set the peak are the ones the 1 GB budget is about.

`spans.tsv` gets five columns. The report gets a `== Memory` section: the footprint now and at most, jemalloc's
live and resident heap, the spans by own bytes allocated with what they kept, and the spans that raised the peak.

### 4. The program is combined once, inside the profile

`run_with_options` combined the program once to ask whether it reads the file system, outside any span and before
the profile was reset, then combined it again (a memo hit). It now holds the profile from the first `combine`,
releases it before rounds of discovery (which are runs of their own), and reuses that program. The phase clock
starts before it, so `stats` still counts combining. `combine_texts` has spans of its own: parse each program,
bound each program to its documents (`hiding`), and merge.

`logic <dir>` no longer runs a file another file in the directory `#include`s (`…confusables.rr`) as a program of
its own. A declared relation that no rule derives and that is not read in has no rows, so a `match_all` over it is
empty rather than refused (decision 0081).

## Measured

The Python pack (544 rules; `b005` and `perf403` refused and left out) over `fixtures/python/calls/project`, with
`RERAIL_DEADLINE=20` and `timeout 40`, at load 48 to 74:

```
thread "main": cpu 34.19 s, allocated 58399.7 MB, kept 1391.8 MB (its lifetime)
     28.280 s   42449.9 MB  parse the program
     19.369 s   18155.1 MB    bound each program to the documents it shows (542 programs, 355782 rules; decision 0058)
== Memory: footprint 1430.3 MB now, 2155.6 MB at most; jemalloc heap 1392.5 MB live, 1457.6 MB resident
   24294.8    24294.8      621.8         1  parse the program/parse each program (542)
```

Both dumps were written, 20 s apart. `stacks.txt` (127 frames in `corpus::hiding`) agrees with the open spans.
`spans_charge_memory_and_a_dump_lists_what_is_open` checks the byte counts against 8 MB and 16 MB allocations, and
a dump's listing of another thread's open span and statement.

## What the dump found

Run on the pack, the dump named what the stacks alone could not: one program, `ruff-perf401`, open in `hide` for
35 s and 36 GB. Its rounds never ended (O111): a copy whose head holds a constant at the seeded column was found
again each round and counted as a change. That is fixed in `hiding::hide`. Errors at planning now name their rule
or fact (`plan::lower`: `rule for p274_noqa_all_tail($R): the string "" stands where an integer goes`), and
`check-rr` types the pruned program as a run does, so the six rules that fail are found in one 43 s pass, not one
pack run each. Where the pack's time goes now is O123.

## What this does not do yet

- **Other threads' CPU off macOS** reads `?`: Linux would read `/proc/self/task/<tid>/stat`.
- **A statement's VM steps so far** are not read: only its text and wall time.
- **Allocation is by thread.** Bytes freed on another thread (a buffer handed to a worker) count there.

## Prior work consulted

- **Lessons ledger 10:** measure the code that is actually running. The first `combine` was outside every span, so
  the profile had not been measuring the pack's largest cost; the dump showed it.
- **Lessons ledger 16:** the cost is repeated work over the same relations. The dump shows 355,782 rules for 542
  programs: each program carries its libraries again.
- **ast-index-spike history index T24 (DH L7673-7687, S250):** a profile has to attribute the cost to the part
  that pays it. Here that means the open span and its bytes, not only the totals at the end.
- **ast-index-spike design index L379:** a runtime timeout probe (try, then retry) is rejected, because it spends
  the budget. The dump does not retry anything: it records where the time went, once, for the next decision.
- **ast-index-spike design index L1032 (budgets, C4):** the spike's budgets were sizes, and its actuals missed them.
  The 1 GB budget here is a footprint, so the report names the spans that raise the peak, not only those that
  allocate most.
- **Decisions 0058, 0075 and 0081:** the bounded programs (`hiding`), the profiled text-test compile, and
  `match_all` over nodes.
