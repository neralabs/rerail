# t31. Making a one-file edit cost what it changed: a workplan

## Intro

After an edit to one file, a run should cost about what that file's change reaches: close to nothing. Today it
costs a fifth of a cold run. This note measures where that time goes, finds the cause of each part, and orders
the work that removes it.

The workload is the Python pack (main's 318 default rules less B005, run as one `logic` program) over 300 files
of the data-layer repository: 3.8 MB of source, 645,519 nodes. It is from the release build of 7e85463a4 with the
rule fixes of 913cb3f7a. CPU is user + sys, at load averages 50–100, so only large differences count.

| Run | CPU | Peak memory | What it does |
|---|---:|---:|---|
| cold (new store) | 123–146 s | 5.6 GB | parse and evaluate every file, then the cross-file part |
| warm (nothing changed) | 0.6 s | 196 MB | phase 0 checks every read and keeps the outputs |
| **one file edited** | **29.2 s** | **5.7 GB** | 299 files from the memo, one evaluated, the cross-file part in full |
| one rule's message edited | 38.6 s | 5.7 GB | the same, with the program compiled again |
| main 0.1.3, any of the above | 20 s | 1.0 GB | main's headless scan reuses nothing |

The edit appends one small function to one file:

```python
def _bench_edit(x):
    assert x, "edited"     # one new B101 finding; nothing else changes
    return x
```

**Target.** A one-file edit under 1 s of CPU here, and under 3 s on the full 9,010-file repository, within the
1 GB budget, with the same findings as a cold run. A comment edit costs what a warm run costs.

## Where the 29 s go

From `RERAIL_PROFILE` on the edit run (own CPU of each span):

| Part | CPU | Allocated | What it is |
|---|---:|---:|---|
| memo replay | 10.4 s | 11.9 GB | every unchanged file's rows decoded, turned into columns, added to the cross-file inputs, measured |
| memo lookup (worker) | 1.9 s | 8.3 GB | 301 lookups, each decoding the whole entry |
| one index, in F823's `outer` | 6.6 s | 6.1 GB | the cross-file part's hash index on a base input of 15 M rows per large file |
| the other 949 cross-file strata | 5.7 s | | every stratum runs, from empty |
| statistics phase, parse, other | 4.6 s | | |

The cross-file part has 950 strata. All of them ran after the edit, and 25 took more than 50 ms.

## Why each part is linear in the repository

### 1. A process keeps nothing of the cross-file part

A `logic` run is one process. Between processes, only the store survives, and the store holds each file's
file-local rows (the per-file memo, decision 0052). The cross-file part has no kept state, so a run rebuilds its
inputs from every file's memo entry and then derives every stratum from empty.

The kept cross-file state of decisions 0063, 0077 and 0090 lives in a session, inside one process. A one-shot
run never has one. So the 11–15 s of memo restore and the 12–14 s cross-file part come from one cause: the
cross-file fixpoint is not kept between runs.

### 2. The memo is 400 times the source

The memo holds 1,450 MB of rows (87 MB compressed) for 3.8 MB of source. The largest entry is 291 MB, for a
291 KB `conftest.py`. One relation is 55% of the memo, 801 MB:

```text
fl3436_p137_outer    15,401,090 rows x 2 columns, in conftest.py alone (235 MB)
```

It is the file-local half of F823's `outer`, which the split pushed into the chunks without its filter:

```text
// F823 (ruff-f823-undefined-local.rr) as it was:
outer(f, x) :- read(_, x, f, _), inside(f, p, fscope), bound_in(p, x).
//             ^ the filter      ^ pushed file-local and exported on its own:
//                                 every function x every name bound in every scope around it
```

The cross-file part then indexes that export: the 6.6 s, 6 GB index above. The same shape cost 263 M rows in
pyflakes's `unbound_ref` (fixed in 913cb3f7a).

### 3. Invalidation is by whole input

Even inside a session, decision 0077's first stage reruns a stratum when any file's rows of an input it reads
change. An edit changes rows of almost every export, so almost every stratum runs. Decision 0090 replaces this
with deltas, in steps not yet built.

### 4. Per-component memo of the cross-file rows is off for the pack

Decision 0076 memoises cross-file rows per import component, so an edit reruns only its component and the
components that import it. A program asks for it with `.pragma "cross-memo"`. The Python libraries and the pack
do not, so the run reports it "off".

## The workplan

Six streams. A and C1 are small and can land at once. B and D are the architecture: they need the kept
cross-file state of decision 0090. Each step is measured before the next, on this workload and on the full
repository. Each must keep findings equal to a cold run.

### Stream 0: the measurement harness (first)

| Step | Change | Done when |
|---|---|---|
| 0.1 | A script that runs cold, warm and five edits (append a function, change a function body, change a comment, change an import, change one rule's message), reporting CPU, peak memory, memo bytes, strata run, and the findings' diff against a cold run | its numbers match this note's |
| 0.2 | A way to run several edits in one process, so a session can be measured from the CLI: `logic --session-script <file>` or a stdin loop, using decision 0063's session | the same edits measured one-shot and in a session |
| 0.3 | Profile lines per edit: entries decoded, bytes decoded, strata run and their delta sizes | the edit run's 29 s is fully accounted for |

### Stream A: memoise less

| Step | Change | Expected | Done when |
|---|---|---|---|
| A1 | Engine, in the file-local split (decision 0051): never push down and export a join whose filter stays in the cross-file part. Either keep the join cross-file, probing the filter first, or push the filter's file-local half down with it as a semi-join. A rewrite of the rule alone does not help (the trial below) | memo 1,450 MB → about 650 MB; the 6.6 s index goes | F823's `outer` exports no `fl*_p137_outer` rows beyond its filter |
| A2 | The profile reports any pushed join that exports more rows than a bound per node of its document, with the rule it came from | the next such rule is found by the profile, not by hand | a test with `outer`'s shape is reported |
| A3 | Memo entries hold only what the cross-file part reads: exports it reads and base relations it reads, projected to the columns it reads | entries smaller again | memo bytes per source byte under 20 |
| A4 | Entries stored as columns: replay is a slice, not a decode and transpose of each value | replay of 299 files under 1 s | measured |

### Stream B: restore only what changed

An unchanged file's entry should never be decoded. That needs the cross-file inputs kept between runs. There are
two places to keep them, and both are needed:

| Step | Change | Done when |
|---|---|---|
| B1 | **In a session**, decision 0090 step 2: the cross-file part's input relations stay engine relations (`setfix::InRel`, with their indexes), not rows copied out and back in | a session edit makes no `memo replay` of unchanged files |
| B2 | Decision 0090 step 4: an edited file's base delta is its old memo entry against its new one. Only those two entries are decoded | an edit decodes 2 entries, whatever the repository's size |
| B3 | Statistics moved by the delta's counts (decision 0096's keeper), never measured from every row | no `measure the base rows held` on an edit |
| B4 | **Between processes**: the kept relations are written to SQLite after a run (durable, may lag; memory may be ahead, as decided), as column blocks per relation and document. The next run maps them in and applies its delta | a one-shot edit's restore costs the blocks' bytes read, not their decoding; under 1 s here |

B1–B3 make the session path near zero. B4 brings the one-shot CLI and CI path close to it. Its remaining cost
is reading the kept relations, which is proportional to the bytes kept, so streams A and D shrink it.

### Stream C: open inputs and build indexes

| Step | Change | Done when |
|---|---|---|
| C1 | A1 removes the 6.6 s index of a base input | measured after A1 |
| C2 | Kept relations carry their indexes (B1). A delta updates postings in place | an edit builds no index of a kept relation |
| C3 | Within a run, a derived relation's indexes are shared by every stratum that reads it. Today `setfix`'s cache is per relation object, and each stratum gets a new one (14,113 builds on a cold run, 12 s) | builds per relation and column set: one |
| C4 | Large base inputs use their source's sorted or leading-column path, not a full hash table | no index of a base input over 1 s |

### Stream D: the cross-file part runs only what the edit reaches

| Step | Change | Done when |
|---|---|---|
| D1 | The census of decision 0093 over this pack (`RERAIL_SPLIT_DUMP`): each of the 950 cross-file strata classed as same-document, union of per-file rows, lookup by value, and so on | a table of the pack's cross-file strata by class |
| D2 | Same-document strata move to the file-local part, so the per-file memo covers them. F823's `outer` is per function, except where `bound_in` reads another file's bindings: that narrow relation stays cross-file | the cross-file strata cut to those that really read other files |
| D3 | Decision 0076 for the pack: `.pragma "cross-memo"` on the relations that are a function of the import closure. 0076 measured "correct, and no faster" on typing, so measure before relying on it | an edit to a leaf module reruns its own component only |
| D4 | Decision 0090 steps 3 and 5: in-place survivors, a stratum's net delta, and an empty delta stops propagation. Changed strata run on their delta only | an edit runs only the strata its delta reaches, each costing its delta |
| D5 | Per-definition node ids (decision 0083 step 5a), so an edit in the middle of a file does not change the ids of every node after it, and a comment changes no exported row | a comment edit runs no stratum |

### Stream E: the run's fixed costs after an edit

| Step | Change | Done when |
|---|---|---|
| E1 | Statistics phase: 1.4 s on an edit with the program read from the store. Find what is redone for a prepared program | under 0.1 s |
| E2 | A rule edit recompiles one rule. Each policy's relations are already namespaced (`pNNN_`), so a changed rule reruns its own strata and a message-only change reruns only the output | a message edit costs a warm run; a body edit costs that rule |
| E3 | Peak memory on an edit, 5.7 GB, falls with A, B and D. Track it against the 1 GB budget at each step | under 1 GB |

### Stream F: other problems this measurement found

These are not on the edit path. They were found on the way.

| Item | What | Fix |
|---|---|---|
| F1 | Pack rules run inside annotation guests. About 157 of B002's findings are in guest documents (`guestNNN:line`), from strings like `"--host"` | finding rules read host documents only, unless they ask for guests |
| F2 | A parse request is built only from the literals that name the string, dropping disjunctions and indirect guards, so `parsed(s, "annotation", r)` asked for every string (612,872 guests for 300 files). 913cb3f7a works around it with `str_node`, `ann_ctx` and `frag_str` | the request lowering keeps those literals; the workarounds can go |
| F3 | The demand split runs per chunk: 9.6 s over 28 chunks on the 300 files. It reads the chunk's statistics, so it cannot simply be cached | decide once per run from the run's statistics, or key it by the decisions it makes |
| F4 | A cold run peaks at 5.6 GB against a 1 GB budget | stream A's memo cut, then the cross-file part's held bases |
| F5 | B005 does not compile in the combined program | an engine fix; the rule then joins the set |
| F6 | Moving rows to the cross-file part: 17–21 s of a cold run | the same column blocks as B4 |

## Order and expected effect

| Order | Steps | One-file edit, expected |
|---|---|---|
| now | 0.1, A1, A2, C1 | 29 s → about 20 s (index and half the memo gone) |
| next | A3, A4, C3, E1 | about 10 s one-shot; less memory everywhere |
| architecture | B1–B3, D4, C2 (in a session) | under 1 s in a session |
| then | B4 (one-shot), D1–D3, D5, E2 | under 1 s one-shot; a comment edit at the cost of a warm run |

Stream B and D4 are decision 0090's own steps 2–4. They should be built where 0090 is being built, not as a
second mechanism. This plan adds the measurement, stream A, and the pack-side work (D1–D3) around them.

## The rule-level trial, and why A1 is in the engine

F823's three `outer` rules were rewritten to start from `outer_cand(f, x) :- read(_, x, f, _), local_in(f, x)`.
Same workload, same build:

| Run | Before | With `outer_cand` |
|---|---:|---:|
| cold CPU | 146 s (load 100) | 161 s (load 120) |
| cold peak | 5.6 GB | 4.4 GB |
| edit CPU | 29.2 s (load 65) | 37.8 s (load 72) |
| memo replay on the edit | 10.4 s, 11.9 GB | 13.5 s, 11.9 GB |
| the `outer` base-input index | 6.6 s, 6.1 GB | 7.6 s, 6.1 GB |
| memo, uncompressed | 1,450 MB | 1,528 MB |
| findings | 615, 616 after the edit | the same |

Nothing moved but the noise. `outer_cand` itself is small (27,191 rows), but it is a cross-file relation: one of
the relations it reads is cross-file. So the split still pushes `inside(f, p, fscope), bound_in(p, x)` into the
chunks on its own and exports it (now as `fl3437_p137_outer`, still 15.4 M rows in `conftest.py`). A rule cannot
steer that; the split must (A1).
