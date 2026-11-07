# T10 notes: where a corpus run's CPU goes, and the opportunity set (2026-11-06)

Profiled on the merged tree (`0ceda4e37`, final-rerail-shape merged in). These are the sources:

- **The run profiles:** `RERAIL_PROFILE` gives phases, spans, plans and joins by work.
- **One wall-clock sample:** macOS `sample` of a full-corpus serverplan-quality run (30 s at 1 ms intervals,
  demangled with `rustfilt`).
- **Every converted rule on s200:** 100 rules, one run each.

Marks:

- **M** = measured.
- **D** = derived from measurements: a sample share applied to a span's CPU.
- **A** = an estimate.

CPU is user + sys. The machine was shared and busy (load 15–47), so absolute times vary run to run by about 30%.
Shares and row counts don't.

## 1. One rule over the whole corpus: serverplan-quality, 9,010 files, 28.2M nodes

| Phase | CPU (load ~25) | Share | |
|---|---|---|---|
| parse (worker) | 22.9 s | 52% | M |
| batch (worker) | 8.8 s | 20% | M |
| local (the rules) | 11.8 s | 27% | M |
| total | 43.8 s | | M; 34.2 s at load 15–19 |

Peak RSS is 316 MB. The rules run per file: 269 rules file-local, 0 cross-file.

### 1a. Parse: 22.9 s, mostly ours, not tree-sitter's

| Span | CPU | Inside it (sample shares of the parse) |
|---|---|---|
| tree-sitter parse | 10.2 s M | The Python scanner's `deserialize` frees and reallocates its stacks on every call, through the system allocator, whose `free` reads the clock: 6.2% of the parse D |
| tape encode | 5.9 s M | `ts_tree_cursor_current_status` once per node, for the supertypes: 7.6%. SipHash in `TapeImage::finish`'s code map: 4.4%. Software SHA-256 of the source: 3.5% D |
| digest | 2.2 s M | SHA-256 fed one 4-byte value at a time, over the source and every fact column |
| facts from tape | 1.4 s M | Copies the tape's columns |
| tape restore | 1.4 s M | Deserializes the image encode had just serialized, and SHA-256s the source again (3.3%) D |
| read | 0.9 s M | |

The opportunities, cheapest first:

1. **Hardware SHA-256.** `sha2` 0.10 on aarch64 needs its `asm` feature, otherwise it runs `soft::compress`; the
   tree already locks 0.11 too. The source is hashed three times per file (encode, restore, digest). Hash it once,
   and hash the fact columns as byte slices. That saves most of the digest's 2.2 s plus about 0.7 s in
   encode/restore (A).
2. **The Python scanner's allocations.** Compile the grammars with `TREE_SITTER_REUSE_ALLOCATOR`, so the scanner
   uses jemalloc like the parser, or patch `deserialize` to clear its arrays instead of freeing them (the grammar
   already carries patches, `PATCHES.md`). About 1.2 s (A).
3. **FxHash for `finish`'s code map.** About 1 s (A).
4. **No serialize/restore round trip in a corpus run.** Build the `TapeDoc` from the encoder's columns. About 1.4 s
   (A).
5. **Supertypes without `current_status` per node.** Ask only for kinds that are some supertype's subtype, or cache
   by (parent production, child index). Up to 1.6 s (A).

Tree-sitter's own 8–10 s remains. Only parsing less removes it: a tape cache across runs, keyed by source digest.
That is a product decision.

### 1b. Batch: 8.8 s, three quarters of it SQLite durability

`with_nearest` → `ContentStore::intern_atoms` is 77% of the batch worker's samples (D). For each of the 104 chunks it:

- appends the chunk's new strings (640k distinct in all) to the on-disk `atoms` table in an `IMMEDIATE` transaction;
- runs on a 1 MB page cache (`cache_size = -1024`), so B-tree inserts miss (`pread`) and dirty pages spill
  mid-transaction (`pagerStress`);
- commits with a WAL checkpoint, `pwrite` and `fsync`. Checkpoint and `fsync` are about 36% and 26% of the
  worker's samples.

The opportunity: the store's in-memory mirror already allocates the ids. Write the rows once, or once per several
chunks, with a larger cache for the append and no checkpoint per chunk. Most of the 6–7 s (A). This is also most of
the run's 6.3 s of sys time.

### 1c. Rules: 11.8 s, 66.6M rows of join work

Engine overheads (main-thread samples):

- **Filtered views rebuilt per stratum and chunk.** `setfix::view` is 12%: `node(_, $n, <kind>, _, _)` filters are
  recomputed by every stratum that reads them, through `Source::value` calls. The fix is a view cache per chunk, or
  the facts' kind postings.
- **Strings formatted for plan-cache keys and join specs per chunk.** `core::fmt` is about 6%.
- **Candidates re-checked against every key.** `Source::value` inside `run_join::go` is about 10%.

Rule shapes (rows, M). Most of the work is a few shapes the converter emits:

| Relation | Rows | Shape |
|---|---|---|
| m650 | 13.3M | span containment by comparisons (P1) |
| m758 | 11.4M | **read by nothing** (P5): 10 of 99 derived relations are unreachable from `violation` |
| violation ×4 | 3.3–7.3M each | "the file contains name X" through every node of the file (P3) |
| m680 | 7.2M | text test checked after a scan of every identifier, because the planner estimates `strtest` at selectivity 1 (P6) |
| violation #192 | 4.6M | `node($d, $n, _, …)` and `node(_, $n, 1223, …)`: one row, two atoms (P4) |

## 2. Every rule on s200 (200 files, 530k nodes)

Most rules take 0.36–0.73 s, and parsing is most of that (M). The exceptions are all the same shapes:

| Rule | CPU | Rows | Shape |
|---|---|---|---|
| transaction-closures | 71.3 s | 3.1 billion (`blocked110`) | P1 |
| module-level-caches | > 60 s (38.1 s on **s50**) | 500M on s50 (`m131`) | P3 + P6 |
| grammar-callable-constructors | 2.7 s, 690 MB RSS | 44.6M, cross-file (`m131`) | P2 |
| error-handling | 1.8 s | 18.9M (`m17`) | P3 |

The first two cannot run on the full corpus at all. They grow with file size squared (P1) and with the file's nodes
times its matches (P3).

### 2a. After P1 and P3 (2026-11-06, after merging final-rerail-shape)

Every converted rule (100) over s200, one measure run each, at machine load 74–108. All 100 ran, none panicked.

| | Instructions retired |
|---|---|
| The cheapest rule (the fixed cost: parse, batch, store) | 4.86e9 |
| The median rule | 4.90e9 |
| serverplan-quality, the most | 11.9e9 (0.91 s CPU) |
| module-level-caches | 9.4e9 (was over 60 s) |
| grammar-callable-constructors | 9.3e9, peak RSS 136 MB (was 690 MB) |
| transaction-closures | 4.9e9 (was 71.3 s) |
| All 100 | 553e9, of which 486e9 is the fixed cost paid 100 times |

No rule is an outlier any more: the most expensive does about 7e9 of its own work above the fixed cost. 88% of
the sweep is parsing and batching the same 200 files once per rule. The highest peak RSS is
serverplan-grammar-surface's, 182 MB.

### 2b. All rules in one run, and tapes kept between runs (2026-11-06)

Running the rules one at a time pays the fixed cost (parse, batch, store) once per rule: 88% of the s200 sweep.
Three changes remove most of it:

- **One run for every rule.** The rules are combined into one program and run together. Each program's own
  relations get a per-program prefix. The inputs and `violation` are shared, and so are kind sets with the same
  kinds (`corpus::combine`, `run_all`).
- **A parse cache.** The corpus runner used to delete its store and parse every file again on each run. It now
  keeps tapes in a content store of their own, keyed by the source's content, the grammar version and the tape
  format (`core::build_cached`). `RERAIL_TAPE_CACHE` names the store; `off` parses every file. The full corpus's
  tapes take 103 MB.
- **File-local joins pushed into the chunks.** In a combined program, `violation` is one relation. The split into
  per-chunk and cross-file parts is decided per relation, so a single cross-file rule made every rule's violation
  cross-file. Each such rule then read its base relations whole: every chunk's `within`, `node` and `child` rows
  were kept for the cross-file part. On s1000 that was 629 MB of values (`within` alone: 256 MB), and the full
  corpus didn't finish within 280 s.

  The fix is one rule in the split (`file_local::push_local_joins`). It takes a cross-file rule's file-local part:
  its positive base and file-local reads, joined by their variables, plus the comparisons and negated reads whose
  variables they bind. That part becomes a relation the chunks derive and export, and the rule reads it instead:

  ```text
  // Before: runs once over all files, holding within, node, text for every file.
  m($n) :- within($a, $n), node(_, $a, 1025, _, _), text($a, 522), !cross_file_test($a).
  // After: the chunk joins the first three and exports the few rows that pass.
  fl_m($a, $n) :- within($a, $n), node(_, $a, 1025, _, _), text($a, 522).
  m($n) :- fl_m($a, $n), !cross_file_test($a).
  ```

  288 rules are rewritten this way. The cross-file part now holds no base relation.

Every converted rule (100) in one run (M; load 17–55):

| Corpus | Instructions retired | CPU | Peak RSS | Findings |
|---|---|---|---|---|
| s200, separate runs (2a) | 553e9 | | 182 MB (the largest) | 4 |
| s200, one run, before the push | 57.3e9 | 4.7 s | 472 MB | 4 |
| s1000, one run, before the push (warm) | 165e9 | 14.5 s | 1,529 MB | 24 |
| s1000, one run (warm) | 115e9 | 9.7 s | 582 MB | 24 |
| full, one run, cold (fills the cache) | 970e9 | 93 s | 695 MB | 252 |
| full, one run, warm | 797e9 | 60 s | 739 MB | 252 |

On the full corpus the parse phase is 26.9 s cold and 4.3 s warm. The rest is the file-local part (49 s warm);
the cross-file part takes 0.01 s. The findings match the one rule that has any (callable-constructors, 252 when
run alone). The replay's totals are unchanged.

Encoding the tape now reads a node's field from the cursor-status call it already makes (one stack walk per node,
not two). Parity with native tree-sitter holds on s1000's 997 Python files.

### 2c. The file-local part, profiled, and memoised per file (2026-11-06)

A profile of the warm full run's file-local part (39.7 s, profiled build) had three pieces:

- joins: 21.8 s;
- opening each chunk's engine: 9.5 s. Half of it (4.8 s) was one rule's `span_within` source, built whole for each
  chunk. A source made on lookup is now probed through its index (`Source::made_on_lookup`), so it takes 0.002 s;
- per-stratum overhead: about 6 s, for 620 strata in each of 104 chunks.

That brought the file-local part from 49 s to 29 s.

Then decision 0050: each file's file-local rows are memoised under its content, its path and the run's plan. A
file that hasn't changed is neither parsed nor evaluated; its rows are replayed into the cross-file tables.

Every converted rule (100) in one run (M; load 24–52):

| Corpus | Run | Instructions retired | CPU | Peak RSS | Findings |
|---|---|---|---|---|---|
| full | tapes cached, memo empty | 744e9 | 36.8 s | 665 MB | 252 |
| full | memo full | 12.1e9 | 0.93 s | 124 MB | 252 |
| s200 | memo empty | 31.9e9 | | 287 MB | 4 |
| s200 | memo full | 6.2e9 | | 45 MB | 4 |
| s200, 3 files edited | memo on | 12.2e9 | | 74 MB | 8 |
| the same | memo off | 28.5e9 | | 289 MB | 8 |

With the memo full, the run reads 9,010 memo entries (0.25 s across the workers) and spends 0.65 s on the
cross-file statistics. The memo adds 32 MB to the full corpus's store.

`rerail-engine logic` takes a directory of `.rr` files and runs them as one program (100 rules on s200: 1.6 s,
286 MB).

Two items from §1 measured again on the full corpus, with the memo off and the tapes cached:

- **Batch: 4.1 s**, no longer SQLite durability. Atom writes are deferred (decision 0047), so interning takes
  0.08 s. What's left is nearest ancestors (1.8 s), projecting (1.3 s) and text tests (0.9 s). It is paid only for
  files the memo misses.
- **Memory doesn't creep with corpus size.** `RERAIL_PROF_CHUNKS=1` prints memory as each chunk arrives. The live
  heap is 280 MB at chunk 1 and 290 to 350 MB through chunk 104. The footprint rose to about 550 MB by chunk 30 and
  stayed there: freed pages the allocator kept. The run now returns them after each chunk, so the footprint holds
  near 380 MB. Peak RSS went from 653 MB to 572 MB, at the same instructions (748e9).

### 2d. The file-local part profiled again: fixed cost and joins (2026-11-06)

Full corpus, 100 rules, memo off, tapes cached. A `sample` of the main thread for 15 s of the local phase (3,838
samples), and the run's spans. Three fixes came out of it first:

| Change | Instructions |
|---|---|
| before | 748e9 |
| Demand (decision 0046) drops only the candidates whose split fails. One component closing a cycle through a negation (`p65_m41`) had left all 100 rules without demand. Now 33 relations are demanded. Joins 18.3 to 13.6 s. | 690e9 |
| A stratum borrows the relations run so far instead of cloning them (quadratic in strata: 3%), and a store path is canonicalised once (`realpath` per stratum: 2.5%) | 669e9 |

The replay and the 252 findings are unchanged.

Where the main thread goes now:

| Part | Share | What |
|---|---|---|
| Joins (`run_join`) | 43% | finding candidates 23% (input lookups and values: 12% and up), inserting results (`Rel::push`) 11%, of which hash-table growth is 7% |
| Engine open | 18% | input views 9% (about 800 filters per chunk, each built once: 1.3 s a run), index builds 2%, SQLite reads 1% |
| Join specs as text | 4% | built, hex-encoded and parsed per stratum per chunk |
| Planning (reused plans) | 3% | |
| The rest | about 10% | statistics, step counts, allocation |

**The fixed cost** is about a third of the main thread: 645 to 780 strata, each opened, planned, specified as text
and closed in each of 104 chunks. Without changing the shape, the remaining levers are:

- keep a stratum's spec between chunks when its plan is reused, instead of rebuilding the text;
- size a derived relation's table from the planner's estimate, so it doesn't grow by rehashing.

Running fewer strata per chunk (merging strata that read nothing from each other) would be a larger change.

**The joins are mostly large relations built whole, then read in one place where the reader already binds the
key:**

| Stratum | Rows built | Read by | Join CPU |
|---|---|---|---|
| `p33_m147` | 18.3M: every node not in `m146` | one rule | 1.35 s |
| `p55_m131` | 11.4M (`inside_ks5` of 3,144 nodes) | `pre_1020`, `pre_1021`, which bind `n` from `node(…, 1223, …)` | 0.98 s |
| `p55_m106` | 7.2M: `name(n, v), strtest_217(v)` | only `!p55_m106(n)` | 1.19 s |
| `p33_m131` | 6.0M | `p33_m132(n) :- p33_m126(n), p33_m131(n)`: 242k by 6.0M gives 238k | 1.66 s |
| `p33_m132`, `p33_m16` | 6.0M each | `m17(n) :- m126(n), m16(n)` and similar | 1.82 s |
| `p55_m676` | 6.9M: all nodes of kind 1025 or 1228 | `p55_m678(n) :- p55_m676(n), name(n, v), strtest_221(v)` | 0.41 s |

Together that's about 7.4 s of 17.6 s of join CPU. Decision 0046 demands only relations that are read solely by
negations and whose every rule starts from every node. Two extensions would cover these:

1. **Positive reads:** cut a positive read of a candidate when the reader's other literals bind its variables, as
   0046 does for negations. The demanded relation is then built for the reader's keys only.
2. **Rules that start from a filtered base scan** (`node(_, n, 1025, …)`, `field(n, 65555, _)`, `name(n, v)`) as
   well as from every node.

The structural guard has to be restated for positive reads. A negation only filters, so the reader's remaining
body is no larger than its own work. A positive read can connect the body, so cutting it can make `pre_<i>` larger
than the reader. The candidate rule: cut a positive read only when a kept positive atom that is derived or
constant-filtered binds all its variables. This is for a decision (an extension of 0046), not done here.

### 2e. The roofline: every span against what its work needs (2026-11-06)

The profile now has a roofline section (`logic/profile/roofline.rs`). Each span is charged with its work in units
of one kind. SQL statements charge their VM steps and native joins their rows (into every step, plus the rows the
last step makes). The rest are charged where the work is done: bytes read, hashed or stored; nodes built; values
measured; cells solved; candidates priced. When the profile is written, short loops measure this machine's best
time per unit: the roof. For each span the report then gives:

- the roof time (units times the roof);
- the excess (own CPU less the roof time);
- for a join, the amplification (rows of work per row it made);
- the CPU that no counter explains.

Tree-sitter has no roof: it is the roof, and only a memo hit avoids it. Planning has none either; the report shows
its rate.

Runs: the full corpus (9,010 files, 28.2M nodes), the 94 converted rules of the checks with no hand-written rule,
one combined run. Store as in decision 0051. The cold run is a fresh store. The warm run is the same tree again.
The incremental run comes after 20 files each gained a function. Instructions are user and system, and the load
was 18–27.

| Run | Instructions | CPU (phases) | Peak RSS | Chunks |
|---|---|---|---|---|
| cold | 804e9 | 56.8 s | 447 MB | 104 |
| warm | 20.9e9 | 1.40 s | 175 MB | 0 |
| incremental (20 files) | 28.0e9 | 1.55 s | 229 MB | 1 |

Cold, by kind of work. A span counts under its main kind; "of roof" is roof time over own CPU:

| Kind | Units | Own s | ns/unit | Roof ns | Of roof | Excess s |
|---|---|---|---|---|---|---|
| join row | 121M | 13.5 | 111 | 5.4 | 4.8% | 12.9 |
| node | 74.9M | 6.8 | 91 | 4.7 | 5.1% | 6.5 |
| byte | 797M | 6.0 | 7.5 | 0.34 | 4.6% | 5.7 |
| candidate | 1.10M | 4.3 | 3,943 | – | – | 4.3 |
| VM step | 131M | 2.3 | 17 | 3.2 | 19% | 1.8 |
| value | 13.4M | 1.2 | 91 | 4.0 | 4.4% | 1.2 |
| cell | 534M | 1.0 | 1.9 | 0.07 | 3.9% | 1.0 |
| (tree-sitter) | 138 MB | 8.4 | 61/byte | – | – | – |
| (no counter) | | 12.9 | | | | |

Where the cold run's excess is, with its cause where the profile names it:

1. **Node sets copied per program, 5–6 s.** The converter's matcher relations make kind sets and their complements
   for every node, and each program has its own copies:
   - `p33_m147(n) :- node(_, n, _, _, _), !p33_m146(n)` holds 18.4M rows;
   - `p53_m131`, a subtree under nodes of one kind, holds 11.4M rows;
   - `p53_m676` and `p61_m107` each copy all 6.0M identifiers.

   These cost about 100 ns a row against a 5 ns probe. A kind set belongs to the batch's kind index, a complement
   to an anti-join where it is read (decision 0046), and a subtree to `within` (decision 0048).
2. **Tape work, 7.6 s.** Encoding the tape costs 140 ns a node against a 4.7 ns roof (3.8 s). Storing it costs 2.1 s
   for 246 MB. Facts from the tape and the tape restore cost 1.8 s more.
3. **Content keys, 3.0 s.** Each source is read (0.9 s), hashed and stored for the memo (1.9 s, 14 ns a byte against
   0.34 for SHA-256), then hashed again for the tape cache (0.28 s). One key per source would do.
4. **Planning, 4.3 s.** 1.1M candidate orders are priced at 3.9 µs each, over 104 chunks.
5. **Batch, 4.6 s.** Project and intern: 92 ns a node. Text tests: 1.1M strings at 1 µs each. Nearest: 1.9 ns a
   cell, against 0.07 sequential; 1.6 s more of it has no counter.
6. **Recursion through `child`, 0.7 s.** `p33_m25(n) :- p33_u19(v), child(n, v, _)` runs 911 waves for 64k rows,
   11 µs a row. A climb is a tree axis (`core.rr`: tape arithmetic, never Datalog recursion).
7. **No counter: 12.9 s.** The largest pieces are engine open (load inputs, build indexes, about 3 s over many
   strata), statistics and plan (1.0 s), memo remember (0.4 s), tape parse less tree-sitter (0.6 s) and digest
   (0.3 s).

Warm and incremental runs are small: 1.0–1.3 s of own CPU. Reading every source to key it is 0.4 s (2.9 ns a byte).
The memo lookup is 0.2 s. Preparing 94 programs and creating 560 export tables (`stats`) is 0.35 s. The 20 edited
files cost 0.15 s beyond a warm run. Reading by size and mtime first would take away most of the warm cost.

**The hand-written rules** (`docs/rule-syntax/data-layer`, `RERAIL_MEASURE_HAND=only`) cannot run on the full corpus
yet. Six of them lower and run together. Three read `input_error` (O80), and one does not lower. On s200 (200 files,
530k nodes) the six cost:

- 39 s of CPU, against 2.1 s for the 94 converted rules on the same files;
- 1.8 GB of peak RSS, over the 1 GB budget;
- 1.59M SQL statements.

The profile names three causes:

- **The included library runs once per program.** Each rule includes `python/names.rr`, `imports.rr` and
  `calls.rr`, and `combine` prefixes every relation that is not `core.rr`'s. So 18.4 s of the 26.5 s in strata is in
  69 strata repeated up to six times. The value cycles (`base`, `pv`, `found`) alone are 10.2 s over five copies.
  Sharing relations whose definitions are equal, recursive ones included, would remove about 15 s.
- **The cross-file rounds** (`typing::cross_rounds`, k = 2) run the whole cross-file program again each round. They
  write the hop relations one row per statement (`INSERT OR IGNORE INTO x.*__prev VALUES (?)`): the 1.59M statements.
- **Per-node exports.** Twelve file-local exports (`fl…_violation`, `fl…_wstart`) hold about 349k rows on 200 files,
  most of a row per node. Each chunk copies them into SQLite at 20–28 ns a VM step.

### 2f. Namespaces, climbs, content keys, planning and counters (2026-11-06)

This section follows up §2e. It covers six items: shared relations across rules, climbs written as recursion,
the counters, content keys, planning, and the cross-file rounds.

**Rules in their own namespace, shared facts shared.** Each rule's relations carry its prefix (`p{i}_`). A
congruence merge (`simplify::merge_identical`) then finds relations whose definitions are equal, recursive ones
included. It is the greatest congruence, found by partition refinement. Such a class is named once in a shared
namespace (`s_<name>`). The libraries' outputs keep their names as copy rules. On the hand-written rules (s200),
this took CPU from 39 s to 11.7 s and peak RSS from 1.8 GB to 785 MB. Violations were unchanged.

Relations that are equal up to their constants are only reported, not merged. There are 285 such relations in 57
groups among the converted rules on s200 (`exists_0 ~ exists_83`), and 10 in 5 groups among the hand-written ones.
Merging them would need the constant as a column.

**Climbs as an axis.** `simplify::climbs_as_axes` finds a recursive component of unary node relations whose rules
are bases, copies, or one step through `child` in one direction. It rewrites the component as its bases plus a
`within` read. Two components of the pack qualify: `p33_m25/p33_u19` and `p33_m63/p33_u57`.

**Content keys.** A source was hashed three times: by the worker, by the memo, and by the tape cache. Now it is
hashed once (`put_source_hashed`), and the source id goes to the tape cache (`build_cached_as`).

A stat index (`corpus_stats`) keeps each file's size, mtime and inode with its source, as git's index does. A warm
run trusts an entry when the stat matches and the mtime is over 2 s before the run that recorded it (git's racy
rule). It then reads and hashes nothing.

| Run (full corpus, 94 rules) | Instructions before | After |
|---|---|---|
| cold | 804e9 | 755e9 |
| warm | 20.9e9 | 15.4e9 (no file read) |
| incremental (20 files) | 28.0e9 | 23.6e9 |

What remains of cold content-key work is compression and one transaction per file: 1.7 s for sources and 2.0 s
for tapes, under load. Hashing is 0.05 s of it at the SHA-256 roof. Batching the writes per chunk is the next step.

**Planning.** The 4.3 s that §2e put on planning was mostly not planning:

- Rebuilding EXPLAIN on every re-price, a profile-only cost, took 0.75 s. A kept plan now carries its EXPLAIN.
- Candidate work was charged after the plan span closed. It landed on the stratum span and took that span's other
  time with it. It is now charged where the orders are priced.

Planning is now 1.7M candidates at 0.5–0.8 µs each (re-pricings included), and 1.8 s of plan spans when profiled.
"Statistics and plan" splits into statistics (0.06 s), the demand split (0.6–0.9 s) and strata (0.16 s).

Keying kept plans by rule address does not work. The demand split makes a new program each chunk, so no address
survives, and every plan was searched again (28 s). The key stays text. Making the split once per run would make
an address key possible and save the split's own 0.6 s.

**The counters.** The rule is that every span doing work charges the units of that work, and every unit has a roof:
a short loop on this machine doing the least that work needs. Added here:

| Where | Unit | Roof |
|---|---|---|
| engine open: rows loaded from SQLite; Csr, Rel and sorted indexes built | index entry | a row id entered in a hash table, not presized (30 ns) |
| engine open: SQLite reads | VM step | as before |
| filtered input views | value | a hash-set insert |
| digest, memo remember | byte | SHA-256 |
| spec, engine close, profile record | spans only | none yet |

A span with no counter is either fixed cost or a missing counter. The profile lists them by own CPU. On the cold run
they went from 17.8 s (31% of own CPU) to 9.7 s (22%). Sampling the process (`sample`) found what the spans could
not:

- a filtered view scanned rows through `value()`, searching for each row's document. It now scans document by
  document (`Source::filter_eq`);
- a rule-number map was rebuilt with SipHash for every stratum.

Cold instructions, unprofiled, went from 725e9 to 683e9.

Still without a counter: `nearest` outside its CPU solve (1.3–2 s), `spec` (2.5 s: native join specs rebuilt per
stratum per chunk, cacheable while the orders are unchanged), the demand split, tape parse less tree-sitter, and
the main thread's wait.

**All of it, measured again** (full corpus, 94 rules, profiled, a fresh clone; load 16–33):

| Run | §2e | Now | Peak RSS |
|---|---|---|---|
| cold | 804e9 | 702e9 | 407 MB |
| warm | 20.9e9 | 16.2e9 | 181 MB |
| incremental (20 files) | 28.0e9 | 24.3e9 | 222 MB |

Violations are 252 in every run.

**Cross-file rounds.** See §2g.

### 2g. Cross-file rounds: what they are, and why they should go

`typing::cross_rounds` comes from typing's summaries (03 §13, I11, list step 29). Its idea is that a file's types
are computed from the previous round's summaries of the files it imports, with k = 2 rounds and a cutoff when the
summaries stop changing. In the corpus runner it is applied to the cross-file part of a program:

- every recursive atom whose node may lie in another document (a *hop*) becomes two rules;
- one rule is within the document (exact);
- the other is across documents, reading `<rel>__prev`: the previous round's rows.

Each round then reruns the whole cross-file program. The rounds stop when the hop relations' digest is unchanged, or
after round K = 2.

Why it is wrong here:

1. **It can miss findings.** A chain crossing more than K files is cut. The result is an under-approximation,
   reported as "not converged". On s200 the hand-written rules do not converge in 3 rounds: the hop rows grow
   152,539 → 152,757 → 152,792.
2. **It repeats work.** Each round reruns every rule of the cross-file part, not only those a change reaches.
3. **It writes rows one at a time.** The previous rows go into `x.<rel>__prev` with one INSERT per row: most of
   the 1.59M statements of §2e.

The cross-file part already runs in one process, over one in-memory database of every file's exports. That is
exactly where semi-naive evaluation computes the full fixpoint, cross-file chains included. Each wave joins only the
new rows, so a chain through n files costs n waves of only the new rows, not n reruns of everything.

The rounds pay for a separation (one file at a time, from summaries) that this part of the engine does not have.

The proposal, not done here: run the cross-file part as one fixpoint and drop the rounds from the corpus runner. Keep
bounded rounds only where summaries really are per file, in typing over files that are not loaded together. The
incremental case is then the memo's job: unchanged files replay their exports, and the fixpoint reruns over them.

## 3. The shapes

The fix for each is one engine rule, not one per check:

- **P1. Span containment written as comparisons.** For example,
  `node(D, A, _, sa, ea), node(D, B, _, sb, eb), sb <= sa, ea <= eb`. The engine joins every node of the file, then
  filters. Spans form a laminar family, so the nodes containing a span are one chain: A's ancestors, plus
  same-span descendants and zero-width edge cases, read off the tape. **Billions of rows; the largest.**
- **P2. An atom whose other variables are used nowhere else.** For example, `text($v122, $s)` beside
  `text($v129, $s)` is implied, but the implied-atom rule (0038) only sees `_`. Treating singleton variables as `_`
  drops it.
- **P3. An existential part joined only through a document.** For example,
  `node($d, $y), name($y, $s), strtest($s)` joined to the rest only by `$d`. Project that part to
  `aux($d)` (distinct docs) before the join: a semi-join, Yannakakis-style.
- **P4. Two atoms of a relation keyed by its node.** For example, `node(_, $n, …)` twice. Merge them into one by
  the key, `node`'s column 1.
- **P5. Relations no output reads.** Drop them before planning.
- **P6. Text-test sizes unknown to the planner.** The vocabulary pass knows each `strtest_*` exactly before the run.
  Feed those sizes to the statistics.

P2, P4 and P5 are simplifier rules (decision 0038's family). P3 and P6 are planning. P1 is a structural join on the
tape. A converter fix can also remove P1 and P3 at the source (`inside`, `has` on the file), but the engine must
not depend on it (decision 0046's point: imperfect conversions).
