# T9 notes: Typing-support worktree (Datalog-engine spike, rdl compiler, native lattices, native call resolver)

Research track T9 for the final Rerail rule-engine shape. Read-only extraction; nothing was built or run.

Source roots (abbreviated below):
- `DE/` = `/Users/jeremy/orca/workspaces/rerail/Typing-support/spikes/datalog-engines/`
- `NC/` = `/Users/jeremy/orca/workspaces/rerail/Typing-support/native/src/code_rule_dsl/calls/`
- `MEM` = `/Users/jeremy/.claude/projects/-Users-jeremy-orca-rerail/memory/typing-approximate-inference.md`

Citations are `file:line`. Measurement marks: **M** = measured in the source; **D** = derived/computed in the source
from measurements; **A** = aspiration, estimate, projection or recommendation (not measured). "CPU" means user (+sys
where the source says so). Every number below is copied from the source, not recomputed.

Working-tree state (relevant to "what is current"): the spike's git log ends at `e031592d Write up round 7` (the
last committed RESULTS). `DE/rdl/rdl.py` and `DE/rdl/sqlite-ext/src/setfix.rs` have **uncommitted** changes: a new
`magic_adorn` (Soufflé-style adorned magic sets over any derived relation, `.demand *` / `RDL_MAGIC=all`,
`DE/rdl/rdl.py:284-467`, `:1783-1785`) and an `RDL_WAVE_TRACE` probe trace (`setfix.rs:319,357-370`). **No results for
`magic_adorn` are written anywhere** (not found in any RESULTS/FINDINGS). It is in-progress work following round 7's
conclusion (see section 6).

Rounds: README/FINDINGS "Results" = round 1; FINDINGS "Round 2", "Round 3", "Round 5", "Round 6", "Round 7". **No
"Round 4" section exists**; "round 4" is referenced at `DE/FINDINGS.md:120`, `DE/rdl/RESULTS.md:9` and
`DE/rdl/RESULTS.md:658` ("the expression typing from round 4 (call results, closures, iterator adapters)" — no
expression-typing round 4 write-up was found in the worktree's spikes). The round-3 table in FINDINGS (`:114-123`)
contains material RESULTS dates "since round 4" (ingestion statistics, per-wave probes).

---

## 0. The question and the corpus

- Question: should Rerail's rule and inference layer (facts, types, dependencies) be an off-the-shelf Datalog engine
  rather than "our own lowering to SQL plus native evaluators", and can such an engine take the transducer as its tree
  matcher? (`DE/README.md:3-5`).
- Corpus: CPython 3.13.12 `Lib/` minus `test/`, `tests/`, `idlelib/`, `site-packages/`: 574 files, 10.5 MB,
  1,218,597 named nodes (`DE/README.md:11-12`). Round 5 adds Rust: `native/src`, 331 files, 1.16M nodes, 73,258 calls
  (`DE/rdl/RESULTS.md:651`).
- Workload (reference `souffle/workload.dl`): W1 `B110` except-pass (negation, count, recursive helper); W2
  `SELF_ASSIGN` (recursion through nested sequences); W3 `NO_TIMEOUT` (import binding, recursive attribute resolution
  with string concatenation, negation; positional timeouts out of scope); W4 `type_of` (set-valued type
  approximation: scopes, name lookup with module fallback via negation, literals, assignment flow, constructor
  calls, interprocedural return types, cross-file classes/functions through imports); W4b `cv` (integer constant
  propagation over a flat lattice; `i += 1` in a loop diverges under set semantics, "the test of lattice support")
  (`DE/README.md:38-50`). Round 5 adds W5 `ASYNC_BLOCKING` and call targets (`DE/rdl/RESULTS.md:473-505`).
- Goldens (Soufflé interpreter): `finding_at` 179 (B110 29, NO_TIMEOUT 20, SELF_ASSIGN 130); `type_of` 165,373;
  `cv_const` 1,655 (`DE/README.md:52-58`). "An engine matches when the sorted outputs are identical" (`:60`).
- Protocol: every command under 5 minutes; record full corpus wall+RSS, one document (`facts-one` = `ftplib.py`,
  4,760 nodes), incremental update, expressiveness checklist, program length, license (`DE/README.md:62-73`).

---

## 1. The Datalog fragment ("Rerail's dialect", rdl)

### 1.1 Lexical syntax (`DE/rdl/rdl.py:37-54`)
- Tokens: `//` line comments; variables `$Name` (`$[A-Za-z_]\w*`); string constants `"..."` (no escapes); integer
  constants `-?\d+`; punctuation `:-`, `!=`, `<=`, `>=`, and single chars `( ) , . ; : = < > ! { } | @`; identifiers.
- Terms (`rdl.py:115-128`): `$X` variable, `"s"` / `123` constant, `_` wildcard, `top` (the lattice top constant,
  `TopT`), and computed terms `@f(t1, ..., tn)`.
- `TOP = 4611686018427387904` (2^62) is the SQL encoding of `top` (`rdl.py:25`); native `lib.rs` uses the same TOP and
  `BOT = i64::MIN` (`DE/rdl/sqlite-ext/src/lib.rs:28-29`).

### 1.2 Declarations and directives (`rdl.py:147-173`)
- `.decl name(col: type, ...)` — column types used: `node`, `int`, `sym`, `flat` (SQL types `rdl.py:949`; Rust types
  `node→u32, int→i64, sym→Sym, flat→Flat` `rdl.py:1612`).
- `.lattice name(k1: t, ...; v: flat)` — the `;` separates key columns from the lattice value; the parser records
  `prog.lattice[name] = len(cols) - 1` (the last column is the value) (`rdl.py:150-160`). Example:
  `.lattice cv(n: node; v: flat)` (`DE/rdl/workload.rdl:128`). **The only lattice type is `flat`.** There is **no
  widening, join, height or transfer-property syntax** in the parser (not found). The join is hard-coded
  (flat: equal stays, different → top) in SQL `merge_lattice` (`rdl.py:1493-1530`) and natively (`lib.rs:73`).
- `.input r1, r2, ...` / `.output r1, ...` (`rdl.py:161-164`).
- `.demand r1, r2, ...` or `.demand *` (`rdl.py:165-168`; `*` only in the uncommitted version, triggering
  `magic_adorn`, `rdl.py:1783`).
- `.kindset name = "k1" | "k2" | ...` (`rdl.py:169-172`). Examples: `scope = "module" | "function_definition" |
  "class_definition" | "lambda"` (`workload.rdl:13`); Python `fscope` (class scopes skipped), `fdef`
  (`calls.rdl:23-26`); Rust `iscope`, `modk`, `fnk`, `fdef`, `implk` (`calls_rust.rdl:19-23`).
- Facts: `head(c1, ..., cn).` with constants (`rdl.py:175-176`), e.g. `seqkind("tuple").`, `timeout_fn(...)`,
  `builtin(...)` (`workload.rdl:39-40,59-61`; `calls.rdl:416-437`).
- Rules: `head :- lit, lit, ... .` (`rdl.py:177-185`).

### 1.3 Body literals (`rdl.py:135-146`)
- Positive atom `rel(t, ...)`; negated atom `!rel(t, ...)`.
- `nearest($n, $s, kindset)` — the only tree built-in parsed: "$n's nearest strict ancestor whose kind is in `set`.
  No relation materializes it; the planner chooses a climb from $n or a descent from $s by cost"
  (`workload.rdl:2-3`; `rdl.py:10-14`; class `Nearest` `rdl.py:82-85`). Both arguments must be variables.
- Comparisons `t op t` with `= != < <= > >=`. `$x = term` assigns only when no other literal binds `$x`, else it is a
  test (`rdl.py:673-687`).
- Aggregate: `$k = count { atom }` (only `count`) (`rdl.py:143-145`, class `Agg` `:78-81`).
- **A computed term (`@f(...)`) is not allowed inside a body atom** — compile error "bind it with `$v = @f(...)`"
  (`rdl.py:181-184`). Computed terms appear in heads and on the right of `=`.
- Built-in functions:
  - `@cat(a, b)` = `a || '.' || b` (dotted-name concatenation) (SQL `rdl.py:799`; native `setfix.rs:623-632`;
    Ascent `cat(...)` `rdl.py:1624`).
  - `@flat_bin(op, a, b)`: flat-lattice arithmetic transfer: TOP if either is TOP; `+`, `-`; `*` only when
    |a|,|b| < 2^40; else TOP (`rdl.py:800-804`; `lib.rs:64-72`). No other registered transfer exists.
  - SQL-side: `flat_join` aggregate, `flat_bin` scalar (`lib.rs:5,75-80,284-288`).
- **`within` and `inside`**: named only in the recommendation ("`nearest`, `within` and `inside` are tape built-ins",
  `DE/FINDINGS.md:198`). **Not found** in the parser, the programs, the native code, or the Ascent runtime. Only
  `nearest` (climb) and its descent form (`TAPE.members` `DE/rdl/ascent-gen/src/runtime.rs:45-71`; native `Down`
  step `setfix.rs:763-765,960-969`) exist.
- **`ast(...)` atoms holding matcher patterns**: named in the recommendation (`DE/FINDINGS.md:179-181`). **Not found**
  in rdl. The only transducer-in-a-rule demo is the hand-written Ascent plug-in (section 7).

### 1.4 Stratification (`rdl.py:473-503`)
- Tarjan SCCs over head→body edges among derived relations; an edge is negative if the literal is negated or an
  aggregate (`:480`). Negation or aggregation within one SCC raises `NotStratifiable` (`:497-501`). Components come
  out dependencies-first (`:502`). RESULTS: "negation or aggregation inside a component is a compile error"
  (`DE/rdl/RESULTS.md:6`).
- `prune` drops rules of relations no output reads, run before and after magic sets (`rdl.py:1761-1776`, `:1787`);
  added in round 7 (`DE/rdl/RESULTS.md:887-889`).

### 1.5 Three-valued "unknown" handling
- No three-valued base-fact type exists in rdl (not found). Unknown is expressed as **separate relations derived by
  stratified negation of lower strata**: `vopen(n)` ("may also evaluate to something no static name denotes"),
  `call_open`, `attr_open`, `base_open`, `bopen`, `qopen`, `async_open`, `calls_open` (`DE/rdl/calls.rdl:5-6,265-365,
  372-377,386-410`); Rust `call_open`, `calls_open`, `reaches_open`, `macro_open` (`calls_rust.rdl:240-253,270-280`).
- A call's answer: unknown (`call_open`), else the definitions `call_target` names, none = no edge
  (`calls.rdl:5-6`; `DE/rdl/RESULTS.md:485-486`).
- Finding: "Three-valued answers need negation only on lower strata. `vopen` is stratified above `pv`. Every 'no
  information' case is negation of a lower stratum, not recursion through negation. The one exception, an external
  base making a lookup external, was written with `extb` ... to keep it stratifiable" (`DE/rdl/RESULTS.md:601-604`;
  `calls.rdl:291-298`).
- "Unknown is conservative but frequent (35% of calls). Most unknowns are parameters and attributes of parameters.
  Closing them needs parameter flow ..., which is sound only for functions whose callers are all known. Without
  context sensitivity, a decorator's wrapper calling `func` stays unknown" (`DE/rdl/RESULTS.md:605-608`).
- Flat lattice `top` is "not provably constant": "declared absorbing elements on transfer functions ... Their
  semantics are 'not provably constant', which is the semantics wanted anyway" (`DE/rdl/RESULTS.md:238-239`).
- Round 1 lists "three-valued base facts" as one of the distinctive parts no engine gives (`DE/FINDINGS.md:23-28`);
  every engine does it "by encoding"/"by hand" (must/may relation pairs) (`DE/egglog/RESULTS.md:41`,
  `DE/souffle/RESULTS.md:59`, `DE/nemo/RESULTS.md:46`).

### 1.6 Program sizes
- `workload.rdl` 141 lines file / "126 lines" (`DE/rdl/RESULTS.md:3`); `workload_demand.rdl` drops `type_of`/`cv_const`
  outputs, adds consumers `range_const`, `append_type` and `.demand cv, type_of, returns, lookup, has_def, fn_ref,
  class_ref` (diff vs workload.rdl; `workload_demand.rdl:138-145`); `workload_perdoc.rdl` adds `ext_class_q`,
  `ext_fn_q`, `ext_returns` inputs (`workload_perdoc.rdl:145-153`).
- `calls.rdl` 437 lines file, "362 lines, 30 of them builtin names" (`DE/rdl/RESULTS.md:475`); `calls_rust.rdl` 281
  (`DE/rdl/RESULTS.md:621`). "The language-specific part is name resolution: 281 lines here against 362 for Python,
  with no engine change other than the exporter's grammar" (`DE/rdl/RESULTS.md:685-686`).
- Hand-written demand relations inside the call programs: `need_attr` (`calls.rdl:204-210`), `vpos`
  (`calls.rdl:266-280`), `wants` (`calls_rust.rdl:87-99`) (`DE/rdl/RESULTS.md:872-873`).

---

## 2. The planner

### 2.1 Statistics (built at ingestion; `DE/stats.py`)
- Tables (`stats.py:23-34`, schema `:53-64`):
  - `stat_docrange(d, lo, hi)`: document d's nodes are the preorder interval lo..hi, from the tape root
    (`t.par < 0`); checked: one root per document, interval covers all its nodes, else build fails (`:78-83`).
  - `stat_val(tbl, col, val, n, docs, sqd)`: per value of each value column: corpus postings `n`, documents holding
    it, sum over documents of per-doc count squared.
  - `stat_col(tbl, col, rows, ndv, sq, sqd)`: `sq/rows` = postings-weighted fanout of an equality probe; `sqd/rows`
    = same within one document.
  - `stat_kind(kind, n, size)` (count, sum of subtree sizes) and `stat_edge(pkind, ckind, field, n)` (efreq).
  - `stat_meta(k, v)`: version, docs, row counts per table = fingerprint (`:138-143`).
- Document-local columns (node ids, a node's doc, byte offsets) are grouped per document so corpus sums stay exact
  under incremental merges (`stats.py:42-51,87-92`; `DE/rdl/RESULTS.md:463-467`). `start`/`end_` as value columns
  gave 230k value rows, 5.4 B/node; making them document-local is "the fix the check caught" (`DE/rdl/RESULTS.md:465-467`).
- `merge <+|-> <docs>` adds/removes a batch's contribution from its rows, touching only changed keys; replacing a
  document = `-` old rows then `+` new (`stats.py:11-14,72-131`). `check`: three batches + five removed and re-added
  equals one-shot build on the full corpus; it failed on a real bug before the fix (`stats.py:174-193`;
  `DE/rdl/RESULTS.md:454`).
- Size/time (**M**): 26,767 value rows; 0.73 B/node, 0.9 MB vs facts' 175 MB; full build 7.4–8.7 s; `facts-one`
  0.04 s (`DE/rdl/RESULTS.md:459-460`; README 8.7 s `DE/README.md:33-34`; round 6 "stats.py takes 5.4 s"
  `DE/rdl/RESULTS.md:737`).
- Read in one statement with fingerprint check (`rdl.py:507-553`; `Stats.__init__` UNION ALL of all `stat_*` plus
  constants' postings plus input row counts `:514-524`; mismatch → refuse with rebuild command `:527-531`).
  Plan-time statistics 0.03–0.05 s vs 0.7–1.4 s before; identical plans (**M**, `DE/rdl/RESULTS.md:461-462`).
- `ANALYZE` stays in the exporter for SQLite's own index choices, but the planner does not read `sqlite_stat1`: "Its
  averages are exactly the uniform card/NDV estimate that misplanned `resolves`" (`DE/rdl/RESULTS.md:470-471`).
- Mapping to ast-index-spike (`DE/rdl/RESULTS.md:449-457`): `t_kfreq`/`t_efreq` (S178, S277) → `stat_kind`/`stat_edge`
  (equal the exporter's Rust-computed totals: 113/113 kinds, 872/872 edges); `npost` (S229) → `stat_val`; process-
  memory statistics planned differently per process (S174, S178) → persisted `stat_col`; `_merge_kind_freq` one-shot
  = incremental (DH L8706-8708) → `stats.py merge/check`; `_prime_corpus_constants` (S259) → one statement; attach
  refuses on fingerprint mismatch (S178) → `stat_meta`. "Kind postings from the positional FTS index (`fts5vocab`),
  the index *is* the statistic" → here each aggregate is read off an index the facts already have; "The facts have
  B-tree postings (`node_kind`, `text_s`), not a tape blob, so no FTS index is added" (`:457`).
- Derived relations: row and per-column distinct counts measured as each stratum completes (`rdl.py:1031-1033`;
  `DE/rdl/RESULTS.md:468-469`).
- Live statistics from the native store between chunks of waves: rows, per-column distinct count (hash index key
  count), per-column sum of squared posting lengths, and per-value postings for columns of ≤ `FREQ_NDV = 64` values
  (`setfix.rs:452-478,688-689`; consumed by `live_stats` `rdl.py:1147-1162`).

### 2.2 Cost model (`plan_rule`, `rdl.py:603-747`)
- **Work** = Σ over steps of (rows produced + rows entering) for an atom step (`step` returns `out + rows`,
  `rdl.py:691-699`).
- **Atom output rows** `atom_rows(a, bound)` (`rdl.py:617-650`), in priority order:
  1. **observed fanout** of the binding pattern if ≥ `OBS_MIN = 32` bindings seen: `out/in`, floor 0.01
     (`:618-619`, `:584`; signature = relation + bound columns, constants by value, `atom_sig` `:586-589`);
  2. `kind($v, "k")` with `$v` bound → 1.0; unbound → `kfreq[k]` (or 0.5) (`:620-622`);
  3. nothing bound on a relation of the component being solved → `max(card, nodes)` ("priced as a pass over the
     corpus at least ... a probe that scans it per delta row is quadratic") (`:624-627`);
  4. nothing bound → `card(rel)` (`node` = total nodes; unknown relation default 1000) (`:613-615,628`);
  5. `RDL_PLAN0`: uniform `card / Π NDV(bound cols)` (`:629-632`);
  6. otherwise: the most selective bound column's expected postings — exact `stat_val` count for a constant on an
     input; `dfreq` per-value count for a constant on a derived tag column; else `fan = sq/rows` (postings-weighted)
     or `card/ndv` — then divide by NDV of each other bound column (`:633-646`);
  7. every column bound on a non-lattice relation → `min(rows, 1)` (membership test) (`:647-649`); floor 0.01.
- **Document range** (`doc_range`, `rdl.py:652-671`): if the body has `docof($v, $d)` (any projection of
  `node(d, n)`) with `$d` bound and `$v` unbound, `$v` is the atom's column, and an input index starts with
  [bound column, `$v` column], the atom can be read as a range in the document's node-id interval, priced at
  `sqd/rows` (per-document weighted fanout), or the constant's count if smaller; used if cheaper (`:695-698`).
- **`nearest`** (`rdl.py:700-704`): climb from bound `$n`: `rows × steps(kind($n), kset)`, rows unchanged; descent from
  bound `$s`: `rows × mean subtree size of $s's kind` (default `nodes/1000`), rows multiplied by the same.
- **`steps(kind, kset)`** (`rdl.py:565-580`): expected parent reads to the nearest ancestor in `kset`, an absorbing
  Markov chain over efreq parent-kind probabilities `parents[c][p] = n / Σn` (`:536-539`), iterated 40 times ("tree
  depth bound; converges well before"): `est[c] = 1 + Σ_p pr(p|c) × (0 if p ∈ kset else est[p])`. Unknown kind:
  `1 + mean(est)`. "No constant is fitted to a corpus" (`rdl.py:13-14`). Example: 8.3 reads from `identifier` to the
  nearest scope (`DE/rdl/RESULTS.md:35`). Replaces S057's fitted `_CLIMB_LEAF_RATIO = 10`, as `SQL_ONLY_RECOVERY_ANALYSIS`
  Step 4c asked (`DE/rdl/RESULTS.md:39-40`).
- **Filters** (negation, comparison, aggregate) run as soon as their variables are bound; each non-`=`, non-aggregate
  filter multiplies rows by 0.5 (`close`, `rdl.py:706-716`). "The planner's cost model is crude: independent
  per-column selectivity and a filter factor of 0.5" (`DE/rdl/RESULTS.md:80`).

### 2.3 Search: Selinger DP (`rdl.py:718-747`)
- DP over subsets of the join literals (positive atoms and `nearest`), keeping the cheapest order per subset, with
  filters closed as early as possible; the first literal (anchor) must be an atom; with `first=` (a semi-naive delta
  or probe), only orders anchored on that atom (`:733-734`). "Bodies here have at most ten join literals" (`:719`).
- The anchor = "ast-index-spike's rarest mandatory concrete variable (DESIGN S049–S050), generalised to joins"
  (`DE/rdl/RESULTS.md:31-32`). Greedy order was myopic: `lookup` 92 s, plan 9.7M → subset DP (`DE/rdl/RESULTS.md:64`).
- Chosen plans round 2 (`DE/rdl/RESULTS.md:44-54`): `lookup` local anchored on `text($n,$x)`, climb, est. 3.6M;
  fallback on `modnode`, 72k; `assigns` on `field($a,"right",$r)`, 57k; `defines_fn/class` on `kind`, 43k/6.6k;
  `augdef` 4k; `returns` on `type_of`, 2.1k. "The descent never won here: every rule binds the inner node first"
  (`:53-54`).

### 2.4 Climb/descent model, native
- Climb = `tape_nearest(n, 'k1,k2')` scalar over the in-memory tape (`lib.rs:263-283`), or native `Near` step
  (`setfix.rs:952-959`). Descent walks `[s+1, s+size]`, skipping each nested set member's subtree with
  `cur += size + 1` (S050 `_dc_cte`) (`DE/rdl/RESULTS.md:36-37`; SQL CTE `rdl.py:885-891`; native `setfix.rs:960-969`).
- Recommendation: "Natively, a nearest-scope column shared across rules is one preorder pass. It is worth it when the
  summed climb work exceeds a pass over the document's nodes, which is again a comparison of costs" (**A**,
  `DE/FINDINGS.md:200-201`). The lattice spike's `anc_P` pass: 4 ms for 1.2M nodes (**M**, `DE/lattice/RESULTS.md:19-22`).

### 2.5 Postings-weighted fanout (round 3/4)
- Cause: `resolves($n,$q) :- binds($d,$s,$q), text($n,$s), docof($n,$d), …` anchored on `binds` (3,094 rows) probing
  `text` by symbol corpus-wide; card/NDV said 15.5 rows; Σf²/Σf = **5,750** ("names like `self` and `os` occur
  everywhere") (**M**, `DE/rdl/RESULTS.md:328-331`). "Lessons ledger L2: estimate selected postings, not kinds or
  distinct values" (`:339-341`).
- Three generic changes (`DE/rdl/RESULTS.md:337-349`): postings-weighted fanout; documents as node-id intervals
  (S012, ledger L21); materialize a view over an input when no input index serves a probe on it (`ensure_indexes`
  `rdl.py:1455-1472`).
- Results (**M**, load 3–14): demand eval 1.94–2.54 → 0.92–1.11 s; full 2.53–3.49 → 1.68–2.13 s
  (`DE/rdl/RESULTS.md:353-364`). At load 12–25: demand 2.98–3.24 → 1.07–1.14 → 0.90–1.08 s (with native set fixpoint);
  full 4.25–5.76 → 2.38–4.24 → 2.26–3.53 s (`:401-412`).
- "The largest set-valued cost was estimation under skew, not recursion" (`DE/rdl/RESULTS.md:433-434`).
- Ascent replanned with the weighted statistics: reorders 16 rule bodies; 1.72–1.84 s vs 1.73–1.89 s — "No
  difference: Ascent's hash joins don't fan out over the corpus the way SQLite's nested loops did, so the skew
  estimate matters to SQL, not here" (**M**, `DE/rdl/RESULTS.md:423-430`).

### 2.6 Observed fanouts and replanning (rounds 5–6)
- Probes in the native set fixpoint are planned before their relations exist; "each column counts as 1, so a join on
  it looks unselective" — `nm($n,$y)` scanned before `member`, one probe 5.6 s (`DE/rdl/RESULTS.md:667-671`).
- Fix: run in chunks of 2, 4, 8, ... waves (`RDL_REPLAN`, default 2, doubling, `rdl.py:1267,1291,1415,1437`); between
  chunks read live stats, replan each probe with the same `plan_rule`, swap those whose order changed
  (`set_fixpoint_reprobe`) (`rdl.py:1282-1291,1424-1437`; `setfix.rs:491-509`). Rust: 7 of 9 probes replanned; `res`
  component 5.3 → 1.5 s solve; Python replans 17 probes, 13.3 → 11.1 s; round-3 workloads replan none
  (**M**, `DE/rdl/RESULTS.md:672-676`). Rows written to tables only at the fixpoint (`setfix.rs:303-305,427-429`).
- Round 6 "estimates for derived joins": correlation between columns (`pv($o,"inst",$c)` with `$c` bound; instances of
  a few classes dominate) made one probe 1.1 s, then another 8.6 s. Three changes from the native store's own indexes:
  per-column sum of squared postings; value frequencies for columns of ≤64 values; **observed fanouts** — every join
  step counts bindings in and passed on; replanning uses the observed fanout per binding pattern once ≥32 bindings;
  "Observations carry over to later strata" (`DE/rdl/RESULTS.md:769-778`; `observe` `rdl.py:591-601`; step counters
  `setfix.rs:770-773,895-898`). "With feedback, no Python probe takes more than 67 ms, against 1–8.6 s before.
  Component relations also keep composite indexes for multi-column probes" (**M**, `:779-780`).
- Planning in Python costs 0.1–0.36 s (Selinger DP per probe per replan); "Plans could be cached by program and
  statistics digest ... Or the DP could run natively" (**A**, `DE/rdl/RESULTS.md:807-808`).

### 2.7 Plan variants per statistics class (round 3)
- 21 documents log-spaced 1–24,808 nodes, each planned from its own statistics: every one a distinct plan, differing
  in 10–34 of 66 rules (`DE/rdl/RESULTS.md:186-188`).
- Four variants compiled into one binary, 4 threads, min of 2 runs (**M**, `:193-199`): corpus plan best in every size
  class: <100 nodes 5.2 ms (vs 5.5/5.6/5.7); 100–1,000 163 ms; 1,000–5,000 694 ms (vs 816–862); ≥5,000 860 ms (vs
  972–1,075); all 1,722 ms vs 1,967–2,112. Best-variant-per-doc after the fact would save 7% (flattered); `--variant
  auto` (closest size) worse: 2.5–3.7 s vs 2.0–2.5 s (`:203-206`).
- "one document's statistics are too small a sample ... a plan fitted to noise loses. So: plan engine programs at
  build time from merged statistics per language ... with no variants. Statistics per file still matter for the
  run-time SQL planner, whose relations are repository-sized" (`DE/rdl/RESULTS.md:207-210`).

### 2.8 Compiler-check lessons (round 2)
- Planned order enforced (SQLite reorders otherwise; D S069) — `CROSS JOIN` (`rdl.py:898`); seed columns threaded through
  walks (S050 `_path_threaded`, `rdl.py:873-874`); anchor by cost search not greedy (S050); typed derived columns or
  indexes go unused (L11, `rdl.py:947-950`). "Each cost 10–90 s until fixed. They belong as tests and checks in the
  compiler" (`DE/FINDINGS.md:70-76`; table `DE/rdl/RESULTS.md:58-65`: `assigns` 62.8 s quadratic join-back; `binds`
  15 s, `augdef` 19.7 s unindexed projections + reorder; `lookup` 92 s greedy; `lookup` 83 s untyped columns).
- From first version to round 2: 290 s and timing out → 26.9 s (`DE/rdl/RESULTS.md:58`).

---

## 3. Backends

### 3.1 SQL emission (`rdl.py:791-1076`)
- Facts attached read-only; `RDL_MMAP` (default on): `mmap_size` 4 GiB, page caches 1 GiB, `temp_store = MEMORY`
  (`rdl.py:903-915`).
- One `SELECT DISTINCT` per planned body (`sql_select`, `rdl.py:811-901`): FROM in planned order joined by `CROSS JOIN`
  (enforces order), equality conditions per bound variable, `NOT EXISTS` for negation, correlated `count(*)` subquery
  for `count`, `stat_docrange` + `BETWEEN lo AND hi` for a document range (`:824-832`).
- `nearest`: with `RDL_NATIVE=nearest`, `tape_nearest(n, 'kinds')` as a column expression (`:855-859`); else a seed CTE
  of everything joined so far, a recursive walk CTE threading the seed's columns, and a `near` CTE (climb via `par`
  while kind ∉ set; descent over `[s+1, s+size]` skipping set members) (`:860-896`).
- Projections over one input atom become temp VIEWs (`rdl.py:927-944`); `docof`-shaped projections of `node` are
  recorded for document ranges (`:940-944`).
- Derived tables: typed columns; `RDL_CLUSTER` (default on) `WITHOUT ROWID` with whole row (or lattice key) as primary
  key (`rdl.py:945-962`). Composite index per bound-column pattern a plan probes (`ensure_indexes` `:1455-1479`).
- Non-recursive stratum: one `INSERT OR IGNORE ... SELECT` per rule (`rdl.py:999-1008,1481-1485`).
- Recursive component, SQL semi-naive (`semi_naive_round`, `rdl.py:1559-1608`): `delta_r` tables; first round all
  rules; later rounds one variant per recursive atom reading `delta_`; `UNION ALL` into `cand`; `EXCEPT` against the
  relation into `new_r`; insert; rename to delta.
- Lattice merge (`merge_lattice`, `rdl.py:1493-1530`): candidates → `GROUP BY` keys (`count(DISTINCT v) > 1 OR max(v)
  = TOP → TOP`) → compare with old into `chg` → old different constant → TOP → `INSERT OR REPLACE`. UPSERT variant with
  triggers (`:1532-1557`) measured slower (14.5 s vs 13.2 s user; "row triggers cost more") (`DE/rdl/RESULTS.md:243-244`).
- Clustered derived tables + covering fact indexes (exporter `child(c,p,idx)`, `child(p,idx,c)`, `field(p,f,c)`,
  `field(c,p,f)`, `exporter/src/main.rs:160-163`): Python plain SQL 17.5 → 14.0 s; with native pieces 6.1 → 5.0 s;
  Rust 3–8% (**M**, round 7, `DE/rdl/RESULTS.md:837-847`). Cause: B-tree descent 45% of plain SQL, top symbol
  `sqlite3BtreeTableMoveto` (second descent index → table) (`:837-838`).
- Page cache: page-miss work 36–46% of Rust plain-SQL samples; `RDL_MMAP`: Rust 12.8 → 8.5 s plain, 7.2 → 4.9 s
  native pieces; Python 19.2 → 17.6 s, 9.8 → 8.9 s (**M**, `DE/rdl/RESULTS.md:744-747`). After mmap, page-cache 6–9%;
  `temp_store=MEMORY` removed spills but "CPU did not move" (`:833-835`).

### 3.2 Native pieces (SQLite loadable extension, `DE/rdl/sqlite-ext/`)
- Inventory (`lib.rs:1-14`, registration `:261-337`): `tape_load(schema)` (reads `node(n, kind)` and `tape(n, par, size)`
  into `Tape { par: Vec<i32>, kind: Vec<u16>, size: Vec<u32>, kinds }` `:32-62`); `tape_nearest`; `flat_join`;
  `flat_bin`; `lattice_fixpoint(p[, target])`; `lattice_result` vtab; `set_fixpoint_open/run/close/stats/steps/
  reprobe/materialize/loads`; `set_rel` vtab.
- `RDL_NATIVE` options: `nearest, fixpoint, kseed, direct, vtab, setfix, upsert`; `all` = nearest, fixpoint, kseed,
  vtab, setfix, join (not direct/upsert, "which measured slower"); `program` adds nearest, setfix, join
  (`rdl.py:26-33`).
- **`lattice_fixpoint`** (`lib.rs:82-188`; compiler side `native_fixpoint` `rdl.py:1078-1145`): applies to a
  one-relation flat-lattice component whose rules are each a **seed** (no recursive atom), **copy**
  (`cv($n,$v) :- …, cv($x,$v)`), **binary transfer** (`cv($n, @flat_bin($o,$a,$b)) :- …, cv($l,$a), cv($r,$b)`), or
  **kseed** (per-kind default `R($n, v) :- kind($n, $k), <tests of $k>` evaluated over distinct kinds — 109 rows — and
  expanded natively over the tape's kind column) (`DE/rdl/RESULTS.md:254-264`). SQL evaluates the non-recursive rest
  once into seed/copy/bin/kseed tables; native: CSR in-edges/out-edges, semi-naive waves with a per-wave stamp
  (`lib.rs:118-164`). `RDL_ABSORB`: TOP absorbs in the binary transfer before the other input arrives (`lib.rs:131-141`).
- `lattice_result` vtab: result kept in memory; scans skip bottom; `c0 = ?` O(1) via `xBestIndex`; no rows written
  (`lib.rs:190-252`; `DE/rdl/RESULTS.md:265-267`).
- **`set_fixpoint`** (round 3/4; `setfix.rs:1-29`; compiler `native_set_fixpoint` `rdl.py:1303-1453`): one recursive
  component of set-valued relations. Seeds run once in SQL. Each recursive rule is, by cost:
  - **Edge**: linear rule whose non-recursive rest SQL evaluates once into an edge table keyed by the variables it
    shares with the recursive atom; waves join the atom's delta with edges natively; head terms variables,
    constants or `@cat`.
  - **Probe**: everything else (several recursive atoms, or rest not worth an edge table); originally its SQL ran
    once per wave per non-empty delta over `set_rel` vtabs (`full`, `delta`, hash index on every column); since
    round 6 a **native join probe** (`join` spec) where every literal has a native step (`join_spec`
    `rdl.py:1166-1219`; steps `setfix.rs:607-621,754-769`: pos/neg atoms over component, input (`t:`), or lower-stratum
    native (`r:`) relations; `cat`; `near`; `down`; `eq`/`ne`/let). No native step for: aggregate, document range,
    descent with `$n` bound, `top`, computed terms other than `@cat` (`rdl.py:1167-1168,1191,1208-1210`).
  - Edge gates (`rdl.py:1355-1363`; `DE/rdl/RESULTS.md:380-388`): (1) a rest costing more than one pass over the tape
    (`work > stats.nodes`) is never an edge table (`mirror`'s rest est. 1.9×10^15; demand-guarded `lookup` rest 4×10^6);
    (2) an edge table must cost less than one delta pass at the estimated seed sizes (`resolves` would need 71k
    attribute edges for 12k derived rows → probe; lowered to edges it measured 0.18 → 0.25 s, slower).
  - Native side dedups and schedules: "no DDL, EXCEPT or count per round, and a probe whose delta is empty does not
    run" (`DE/rdl/RESULTS.md:378-379`).
- vtab index fix (round 5): `best_index` used the first usable equality; probes are prepared while the relation is
  empty, so `pv($o, "inst", $c)` used the constant `"inst"`: 52.5 s for one probe. Now every equality is passed and
  `xFilter` picks the shortest posting list for the actual value, checking the others natively
  (`setfix.rs:529-557,576-594`; `DE/rdl/RESULTS.md:589-594`). Same rule inside native atoms: "shortest posting chain
  among its key columns" (`setfix.rs:926-934`).

### 3.3 Program mode (round 6, `RDL_NATIVE=...,program`)
- `native_stratum` (`rdl.py:1226-1301`): every set-valued stratum whose literals all have native steps runs on the
  native set engine as seed joins (once) + join probes (one per recursive atom, delta first), planned by `plan_rule`,
  replanned between chunks; `nowrite`: relations stay in the native store (`DERIVED`, `setfix.rs:685-686`) and later
  strata read them directly (`r:` sources). Written to SQLite only when SQL reads them or as outputs
  (`materialize`, `rdl.py:965-968,982,1048`; `setfix.rs:691-705`). Returns False for any stratum containing a lattice
  relation (`rdl.py:1237`), which then takes the SQL path (with `lattice_fixpoint` if enabled).
- Coverage: 76 of 76 Python strata and 45 of 45 Rust strata (call programs) (**M**, `DE/rdl/RESULTS.md:762-763`).
- "This removes the per-stratum `count(DISTINCT)` statistics, index builds, write-backs and SQL bodies"
  (`DE/rdl/RESULTS.md:768`).
- Native run phase = Ascent eval: 1.91 vs 1.9–2.0 s Python; 1.01 vs 1.0–1.2 s Rust (round 6, **M**,
  `DE/rdl/RESULTS.md:782-784`). Rest of "exec": Python planning 0.1/0.36 s, lower strata index builds 0.14–0.3 s,
  orchestration, output materialization.
- Profile of program mode, round 6 (**M**, `DE/rdl/RESULTS.md:788-796`): malloc/free/memmove 33% / 33%; tuple dedup and
  index upkeep 32% / 27%; join interpreter 20% / 9%; SQLite (fact loads, output writes) 6% / 11% (Python / Rust).
- Codegen: "worth at most the interpreter's share (9–20%). The plans are already specialized per program ... What is
  left of the interpretation is a `match` per step. Codegen would make it Ascent-like, but it needs a compile step per
  program, while the interpreter takes replanned orders between chunks of waves, which compiled code cannot"
  (`DE/rdl/RESULTS.md:800-804`).

### 3.4 Arena store layout (round 7, `setfix.rs`)
- **Rows**: flat `data: Vec<i64>` per relation, `k` columns each, insertion order (`setfix.rs:104-116`). (RESULTS:
  "Node ids also fit in u32" — proposed, not done; rows are i64 `DE/rdl/RESULTS.md:799`.)
- **Dedup**: `set: HashTable<u32>` of row ids, hashing and comparing the row in place; nothing cloned
  (`setfix.rs:117-139`; `DE/rdl/RESULTS.md:854-855`).
- **Indexes**: one `Index` per column, plus composite indexes per key-column set a join probe reads
  (`Rel::add_multi` `:121-126`). `Index { cols, table: HashTable<Post>, next: Vec<u32> }`;
  `Post { head, len, dlen, wave }` = newest row, total count, count in wave `wave` (`:69-77`). A key's rows are
  **chained newest-first through `next`** (one u32 slot per row): "A posting is a slot, not an allocation"
  (`:74-96`; `DE/rdl/RESULTS.md:856-858`).
- **Delta** needs no index: a wave's new rows are the newest, so a key's delta rows are the prefix of its chain down
  to `lo` (`postings`, `:145-154`; `Cands::Chain{stop}` `:157-176`; `DE/rdl/RESULTS.md:859-860`). `lo..hi` is the
  previous wave's delta (`set_delta` `:140-144`).
- **Inputs and lower-stratum relations** are immutable → **CSR**: `Csr { cols, table: HashTable<(rep, start, len)>,
  ids: Vec<u32> }`, built in two passes with no per-key allocation; one CSR per key-column bitmask, built lazily on
  first use (`InRel`, `:651-684,706-724`; `DE/rdl/RESULTS.md:861-862`).
- Input loading: an input atom loads with its constant columns as SQL filters and projects them away
  (`field($n,"path",$p)` loads only `path` edges) (`rdl.py:1175-1187`; `DE/rdl/RESULTS.md:759-760`); cached per process
  (`INPUTS`, `setfix.rs:725-752`). Load CPU is accounted as **construction** for base facts and execution for lower
  strata (`LOADS`, `setfix.rs:641-649`).
- Symbols: one global interner for every component and input relation, so symbol columns join as integers
  (`setfix.rs:45-58`); `@cat` reuses a scratch string (`:623-632`).
- Results (**M**, round 7): malloc 33% → 5–7% of samples; memory halved ("an index's posting is now 4 bytes plus a
  shared table entry"); construction 2.2 → 1.4 s Python (CSR inputs); program mode about 1.7× faster; native run
  phase 1.19 s Python, 0.59 s Rust, below Ascent's 1.8 / 1.0 s eval (`DE/rdl/RESULTS.md:866-868,829`; `DE/FINDINGS.md:172`).

### 3.5 What stays in SQLite
- Base facts (`facts.sqlite`, attached read-only) and the persisted statistics (sections 2.1, 3.1).
- In SQL+native modes: seed and edge SQL ("What stays in SQL is what SQL is good at: planned joins for the edges";
  `DE/rdl/RESULTS.md:289`), and probes with no native step.
- In program mode: strata with a literal lacking a native step (aggregates, document ranges, ...), lattice strata
  (SQL path + `lattice_fixpoint`), materialization of relations SQL reads, and outputs (`rdl.py:973-982,1048`).
  SQLite = 6% (Python) / 11% (Rust) of program-mode samples (round 6) (`DE/rdl/RESULTS.md:796`).
- Round 1 recommendation: repository strata (cross-file joins, demand, derivations) lower to SQLite, "the smallest in
  memory (161 MB) and the system of record" (**A**, `DE/FINDINGS.md:182-184`). Later rounds do not restate this; see
  Synthesis on contradictions.

### 3.6 Ascent backend (`gen_ascent`, `rdl.py:1628-1737`; `DE/rdl/ascent-gen/`)
- One `ascent!` block; each body emitted in planned order ("Ascent iterates the first atom and probes indexes for the
  rest, so the order is the plan"); `nearest` → `TAPE.nearest` (climb) or `for n in TAPE.members(...)` (descent);
  lattice relations → `lattice` with `Flat`; `count` → `ascent::aggregators::count()` (`rdl.py:1629-1716`).
- Plan comes from `plan.json` written by the SQL run (stats feed both backends) (`DE/rdl/RESULTS.md:13-15`;
  `DE/FINDINGS.md:81-82`). Document ranges are SQL-only (`rdl.py:1635`).
- One generator fix in round 5: a `nearest` climb from a value-bound variable dereferenced it (`DE/rdl/RESULTS.md:586`).
- `ascent-doc/` runs the generated program per document with interface rounds (`ascent-doc/src/main.rs:1-4`; section 6).

---

## 4. Lattices (`DE/lattice/`, plus SQL/native lattice results in rdl)

### 4.1 Graphs (lattice spike, hand-built from facts, `lattice/src/main.rs:723-1070`)
- The lattice spike builds the W4/W4b value-flow graphs in Rust from the facts (not from rdl) (`lattice/RESULTS.md:3-4`).
- Scopes: `anc[v] = scope(par v) ? par v : anc[par v]`, one preorder pass: **4 ms for 1.2M nodes** vs 4.5 s (SQLite),
  5.3 s (Soufflé) recursive `enclosing`; "a tape column, not a fixpoint" (**M**, `lattice/RESULTS.md:19-22`).
- Graph build 128–234 ms including binds, resolves, lookup, return slots (**M**, `:23`).
- `type_of`: 1,232,942 nodes (incl. 14,345 return slots), 161,465 copy edges (1,995 cross files), 146 cyclic SCCs,
  173 nodes in cycles, largest SCC 5. `cv`: 1,232,942 nodes, 152,187 copy edges + 11,176 binary transfers, none across
  files, 1,031 cyclic SCCs, 2,994 nodes in cycles, largest 49. Tarjan over the corpus 54–73 ms (**M**, `:25-31`).

### 4.2 Lattice types (`lattice/src/main.rs:206-316`)
- `Cv` flat: Bot < C(v) < Top; `bin` returns Top as soon as either operand is Top ("workload.dl derives TOP from either
  TOP operand before it needs the other one"), `*` bounded by 2^40 (`:214-253`). Height 2.
- `Ty` exact set union (no fixed height; largest set on corpus 5 members) (`:255-289`; `RESULTS.md:47-50`).
- `TyK` capped union, `K = 4`: more than 4 members → TOP (widening); height 5 (`:291-316`; `RESULTS.md:49`).

### 4.3 Strategies, whole corpus, one thread (**M**, load 40–50 on 18 cores, single runs from the last of three
consistent runs; `lattice/RESULTS.md:14-15,35-45`)

| Strategy | Lattice | ms | Rounds/waves | Visits | Max changes/node |
|---|---|---|---|---|---|
| Naive rounds (Jacobi) | exact types | 282–404 | 16 | 19,727,072 | 2 |
| Semi-naive frontier waves | exact types | 10–11 | 15 | 36,265 | 2 |
| RPO priority worklist | exact types | 13 | — | 35,390 | 1 |
| SCC condensation | exact types | 18–27 | 3 (largest local fixpoint) | 1,232,974 | 1 |
| Naive rounds | cv | 58–100 | 10 | 12,329,420 | 2 |
| Semi-naive frontier waves | cv | 6–8 | 9 | 111,702 | 2 |
| RPO worklist | cv | 13–31 | — | 101,522 | 2 |
| SCC condensation | cv | 9 | 6 | 1,236,658 | 2 |
| any of the four | capped K=4 | same shape | 15 waves | same | ≤ 2 |

- Widening K=4: exactly the 41 nodes with >4 types become TOP; none widened through propagation; 163,980 of 164,021
  typed nodes identical to exact union: "On this corpus the cap costs nothing" (**M**, `:54-56`).
- Naive = 150–550× the visits of semi-naive; "never an option"; the SQLite baseline's 19 s loop was this (`:59-60`).
- Semi-naive waves: work ∝ affected nodes; right for deltas, SQL (one delta table per wave, ≤15 rounds) and Metal
  (`:61-63`). RPO worklist: fewest visits, heap overhead, sequential (`:64-65`). SCC condensation: every node once plus
  local fixpoints in cycles; "right form for a from-scratch evaluation of a whole document. The SCC heads are also
  exactly where a widening operator is needed for an infinite-height lattice ... Acyclic nodes never need widening"
  (`:66-68`). **Note:** the code applies the K cap inside `join` everywhere, not only at SCC heads (`main.rs:298-316`);
  widening-at-heads is a recommendation (**A**), not implemented.

### 4.4 Parallelism (**M**, `lattice/RESULTS.md:72-92`)
- Wave-parallel atomics, corpus, cv, 18 threads: 4.7–8.8 ms, 8 waves; capped types: 12.6–14.6 ms, 15 waves.
- Per-document rayon cv (Tarjan + SCC per doc): 18.5–21.8 ms. Per-doc rayon exact types, doc-local part (R0):
  8.7–14.4 ms; one thread 26.8–38.1 ms.
- Largest document (`tkinter/__init__.py`, 25k nodes): SCC cv 1 thread 0.2 ms (2 waves, 25,413 visits); semi-naive cv
  0.3 ms (4 waves, 1,568 visits); wave-parallel cv 18 threads 1.4–2.0 ms; wave-parallel capped types 1.0–2.3 ms.
- Encodings: flat cv in one word with CAS: `0 = Bot, 1 = Top, v<<2|2`, `i<<2|3` interned big constant; capped type set
  = four 16-bit type ids in one word (`:84-87`; code uses `AtomicU64`, `main.rs:589-688`).
- Waves per document (doc-local, semi-naive): cv p50 3, p99 6, max 9; capped types p50 3, p99 7, max 15 (`:88`).
- "Within-document parallelism doesn't pay on the CPU. Frontiers are at most 1.3k nodes ... 18 threads are 4–7× slower
  than one" (`:89-90`). "Parallelism should come from documents ... and from the modules within one cross-file layer"
  (`:91-92`).

### 4.5 Cross-file layering with interface-digest cutoff (**M**, `lattice/RESULTS.md:109-130`; code `main.rs:1351-1535`)
- Dependency: D depends on M when a return slot of M flows into D, or D constructs a class of M ("resolved
  references, not raw imports") (`:111-112`; `main.rs:1352-1361`). 1,052 edges; 423 of 574 modules depend on another;
  2 cyclic components hold 65 modules; condensation 13 layers; widest 151 modules (`:113-115`).
- Layered run: components after dependencies, components of a layer in parallel, cyclic component iterates until its
  interface digests stop changing: **46–71 ms, 701 document evaluations** (574 + 127 extra iterations), identical to
  goldens (`:116-119`).
- Interface digest = hash of the module's top-level function return types and class names (`:120`;
  `main.rs:1412-1419`).
- Edits to `codecs.py` (122 direct dependents, 408 transitive): A body change reaching no return slot → **1 module**
  re-derived, 0 digests changed (vs 408 without cutoff); B an exported function used at 19 cross-file call sites gains
  a return type → **123** (itself + 122 direct dependents), 1 digest changed (vs 408); B agrees with full layered
  recompute (`:122-128`). Edit times 9–10 ms "dominated by the harness cloning the 1.2M-slot state. One document
  re-derives in about 0.05 ms on average (26.8 ms for 574 documents on one thread)" (`:129-130`).

### 4.6 Metal estimate (**A**, not measured; `lattice/RESULTS.md:94-107`)
- From `prior/research/metal-enqueue.md`: a batch of every document needs max-over-documents waves: 9 (cv) + 15 (types)
  ≈ 24 waves; 1.2–1.6 µs per wave with the hybrid scheduler (almost every frontier < 1,024 nodes → threadgroup-loop
  mode) ≈ 30–40 µs scheduling in one command buffer; work ≈ 150k node evaluations, small next to the measured
  5.4M-node, 27-wave CPython fixpoint at 1.57 ms → "expect well under a millisecond". FINDINGS: "about 24 waves ×
  1.2–1.6 µs of scheduling, plus about 150k node evaluations, so under 1 ms" (`DE/FINDINGS.md:97`).
- "The same 15 waves cover the cross-file edges if the corpus is one graph. Layering exists for caching, not for the
  GPU" (`:100-101`).
- Bound: "each node enters once per stratum" becomes "each node enters at most h times"; arenas statically sized at
  h × n; frontier ≤ n by per-wave stamp; exact union has no h, so Metal needs declared widening (K cap) (`:102-104`).
- Encoding "(to verify)": Metal guarantees 32-bit atomic CAS, 64-bit restricted → GPU lattice value should be a 32-bit
  word (tag + interned constant or set id), "also the representation that suits SQL" (`:105-107`). (The CPU spike used
  64-bit words; `main.rs:589-660`.)

### 4.7 Options matrix and recommendation (`lattice/RESULTS.md:132-158`)
- Matrix rows (strategy → lattice needs / per-doc R0 / cross-file / SQL / Metal): naive (no everywhere); semi-naive
  waves (monotone join, finite height or widening; yes for deltas; yes cross-file "frontier is the set of modules whose
  digest changed"; SQL yes ≤15 waves; Metal yes); RPO (sequential only); SCC (widen only at heads; per-doc from scratch;
  module-level layers; SQL "plan-time structure: strata per SCC"; Metal plan-time only); wave-parallel atomics (GPU
  form); per-document parallel (yes; "batch = documents"); interface digests (deterministic digest; 1 vs 408, 123 vs
  408; "the derivation's output hash").
- Recommendation (**A**): tree structure is a column, never a fixpoint (anc_P, parents from the tape in O(n) at ingest);
  lattices declared with finite height or widening (flat + capped union enough for R0 types; widen only at SCC heads);
  per-doc R0 = SCC condensation from scratch, semi-naive waves for deltas, both <0.3 ms on the largest doc; cross-file
  R1..Rk = module condensation layers, parallel within a layer, early cutoff on interface digests ("the derivation
  mechanism's job"; dependency graph from resolved references); SQL lattices = semi-naive delta waves, values interned
  as 32-bit ids, bounded by h × longest flow path (≤15 rounds here); Metal = same waves per `metal-enqueue.md`
  (`:146-158`).

### 4.8 Lattices in rdl SQL / native (rounds 2–3)
- SQL semi-naive merge: `cv` 12.0 s, 26 iterations vs 19.4 s hand-written naive loop; native 6–8 ms, 9 waves. "The
  component seeds all 1.2M nodes ... and each round pays a temp-table merge" (**M**, `DE/rdl/RESULTS.md:69-76`).
- Profile, no demand, load 40 (**M**, `DE/rdl/RESULTS.md:214-244`): rule bodies into `cand` 11.1 s; GROUP BY join and
  compare 9.8 s; INSERT OR REPLACE 0.52; delta copy 0.23; UPDATE to top 0.12; create cand 0.04 (summed over 26
  rounds). Round 1 has 1,118,742 candidates, mostly the per-kind default `cv($n, top) :- kind($n,$k), !const_kind($k),
  !is_lit($k)`; later rounds 88k, 6k, 2k, <1k. "26 rounds is not a SQL artefact": native on the same edges takes 25
  waves; the lattice spike's 9 waves come from TOP absorbing in the binary transfer; with `RDL_ABSORB=1`: 10 waves,
  identical outputs, `cv` gains 650 top rows (keys whose other input never gets a value): "declared absorbing elements
  on transfer functions are worth 2.5× in rounds" (`:232-239`).
- Native lattice matrix, full workload no demand (**M**, `DE/rdl/RESULTS.md:273-278`): none 17.3 s eval / 13.2 s user /
  214 MB, `cv` 8.0 s 26 rounds, `lookup` 4.3 s; `nearest` 13.7/10.0/222, `lookup` 0.96 s; `fixpoint,kseed,vtab`
  15.8/8.4/177, `cv` 1.7 s (incl. `tape_load`), `lookup` 7.4; `all` **5.6 s / 4.6 s / 179 MB**, `cv` 0.50–0.65 s,
  `lookup` 1.06 s.
- `cv` breakdown (`:282-287`): SQL semi-naive 8–22 s; `fixpoint` seed/edge SQL 1.1–1.6 s (1.12M seed rows), solve
  0.25–0.34 s, writes ~1 s + 0.4–0.8 s copy → 2.5–3.8 s; `+kseed` 0.4–0.6 s (291k seed rows + 109 kind rows), 0.08 s,
  1.5 s writes → 2.2 s; `+vtab` 0.37 s, 0.09 s, none → **0.50 s**.
- "With demand, native adds nothing": eval 4.7 → 4.1 s, user 4.0 → 3.8 s; lattice 580 rows, 16 ms natively, 7 waves;
  RSS 101 MB (tape loaded); one document 0.11 s vs 0.10 s (**M**, `:294-297`).
- Opportunities by measured value (`:299-315`): (1) native fixpoint behind a vtab for seed/copy/transfer components —
  "This is what type inference is: values flowing along edges, with registered transfer functions"; (2) `tape_nearest`
  + `tape_members`; (3) kseed; (4) declared transfer properties (absorbing, strictness), "They need a native
  evaluator, because a SQL join can't fire on one input"; (5) not worth it: UPSERT, native `flat_join` (not measured).

---

## 5. Fact schema and construction costs

### 5.1 Schema (`DE/exporter/src/main.rs`)
- Parses `.py` with tree-sitter-python 0.25.0 and `.rs` with tree-sitter-rust 0.24.2 (`DE/README.md:13`;
  `DE/rdl/RESULTS.md:621-623`; `main.rs:54-57`); skips dirs `test`, `tests`, `site-packages`, `idlelib`, `__pycache__`,
  `lib2to3`, `target` (`main.rs:11`).
- Named nodes only, preorder ids assigned on entry, **global across the corpus** ("no rule needs the document column to
  join", `DE/README.md:27`); comments dropped (`main.rs:70-117`).
- Relations / SQLite tables (`main.rs:139-149`; README table `DE/README.md:16-25`):
  - `doc(d, path, module)` 574; module from path; Rust module path from crate layout (`src/lib.rs`/`main.rs` = `crate`,
    `src/a.rs`, `src/a/mod.rs` = `crate.a`) (`main.rs:20-31`).
  - `node(d, n PK, kind, start, end_)` 1,218,597.
  - `child(p, c, idx)` named children only, 1,218,023.
  - `field(p, f, c)` 561,597.
  - `text(n PK, s)` 350,865 — README says identifier and dotted_name nodes; exporter now also `import_prefix`,
    `type_identifier`, `field_identifier`, `crate`, `self`, `super`; skipped if containing tab/newline/backslash
    (`main.rs:91-94`).
  - `intval(n PK, v)` 16,645 (integer literals fitting i64, `_` removed) (`main.rs:95-98`).
  - `op(n PK, s)`: operator of binary/augmented/comparison/boolean/unary; `async` for `async def` and Rust async
    `function_item` (`main.rs:99-105`).
  - `tape(n PK, par, size, depth)`: `par = -1` at root; size = descendants, one reverse pass (`main.rs:119-129`).
  - `kfreq(d, kind, n)`, `efreq(d, pkind, ckind, field, n)` per file (`main.rs:130-131`).
  - `pkg(d, p)` = `__package__` for relative imports (`main.rs:65-68`).
- Indexes: `child_c(c,p,idx)`, `child_p(p,idx,c)`, `field_p(p,f,c)`, `field_c(c,p,f)`, `node_kind(kind)`, `text_s(s)`,
  then `ANALYZE` (`main.rs:160-163`).
- `subset.py` cuts named documents into a new facts dir; `tsv2sqlite.py` rebuilds facts.sqlite with the same schema and
  indexes and runs `stats.build` (`DE/README.md:28-30`; `DE/tsv2sqlite.py:1-30`).

### 5.2 Construction costs (**M**)
- Exporter: 28–31 s, ~20 s of it system time from one unbuffered `write` per fact line; buffered (`BufWriter` 1 MiB per
  relation, `main.rs:33-41,53`): 5–6 s, byte-identical facts (`DE/rdl/RESULTS.md:735-737`; `DE/FINDINGS.md:155`).
- `stats.py`: 5.4 s (round 6, `DE/rdl/RESULTS.md:737`); 7.4–8.7 s full build (`:460`; README 8.7 s); `facts-one` 0.04 s.
- Facts file 175 MB (`DE/rdl/RESULTS.md:459`) / 177 MB (`:745`).
- Native resolver parses in 0.6 s (`DE/rdl/RESULTS.md:737`).
- rdl construction (base-fact loads + hash indexes) reported apart from exec, "as the anchor-index decision requires"
  (`DE/rdl/RESULTS.md:721-722`): round 6 SQL+native 1.0 s (Py) / 0.2 s (Rust); program mode 2.2 s / 1.6 s; round 7
  program mode 1.4–1.5 s / 1.2 s (`:728-729,826`). Ascent load 1.4–1.5 s / 1.3 s (round 6), 1.3–1.4 / 1.3 s (round 7).
- `ascent-doc` fact loader alone 0.97 GB ("every TSV row is held as `String`s ... nothing to do with the engine")
  (`DE/rdl/RESULTS.md:169-170`).

### 5.3 What construction should come from in the product (**A**)
- "Construction should come from the tape and anchor index" (`DE/FINDINGS.md:153`).
- "Construction (1.6–2.2 s) is a copy of the facts into hash indexes. In the product the tape and anchor index hold
  them; `kind`, `child`, `docof` and the per-field `field` filters are tape and posting lookups, not copies"
  (`DE/rdl/RESULTS.md:805-806`). setfix comment: "Base-fact relations and their indexes are construction (the
  product's tape and anchor index hold them)" (`setfix.rs:644-645`).
- Tree structure (enclosing scope, ancestors, intervals) "comes from the tape or the transducer as base relations. It
  is never Datalog recursion" (`DE/FINDINGS.md:192-193`); `anc_P` and parents from the tape in O(n) at ingest
  (`lattice/RESULTS.md:146-147`); "Structural relations ... must come from the transducer or the tape, not from Datalog
  recursion. This is the strongest argument for the transducer staying the tree layer" (`DE/FINDINGS.md:41-44`).
- Statistics: built at ingestion, persisted, merged per document batch, fingerprinted (`DE/FINDINGS.md:227-228`);
  per-kind defaults declared over kinds and expanded from the tape, never a row per node (`:233`).

---

## 6. Per-document vs whole-program; magic sets; per-edit

### 6.1 Per document is "the unit that matters" (round 1)
- Per-document costs (**M**): Ascent ~5–6 ms, Soufflé embedded 6.4 ms, SQLite 20 ms, egglog 60 ms, Nemo 216 ms, CodeQL
  ~20 s (`DE/FINDINGS.md:31-39`). "Full-corpus memory (Ascent 1.6 GB, egglog 2.3 GB) mostly stops mattering once
  evaluation is partitioned by document" (`:39`).
- Soufflé: all 574 documents one at a time in-process 3.7 s total vs 5.9 s corpus eval, "because the deep tree
  recursion iterates over small relations" (`DE/souffle/RESULTS.md:23-25`).

### 6.2 Per-document Ascent with rounds over the interface (round 3, `ascent-doc/`)
- `workload_perdoc.rdl`: cross-file part = `ext_class_q`, `ext_fn_q`, `ext_returns`; each run reads own facts + the
  interface rows for the qualified names its `resolves` produced, exports its own classes, functions, return types;
  rerun only when its imported rows change: "a derivation keyed by (facts, imported rows), with early cutoff on
  exports" (`DE/rdl/RESULTS.md:144-153`).
- Results (**M**, `DE/rdl/RESULTS.md:160-166`): whole-corpus Ascent 2.64 s eval, 2.7 GB; per document 18 threads
  0.39–0.44 s wall (sum of runs 3.6–4.6 s), 1.31 GB; 1 thread 3.40 s wall (sum 2.78 s), 1.14 GB; 1,035 runs in 3 rounds
  (574, 458, 3); per run p50 1.1 ms, p99 33–47 ms, max 66–104 ms (`tkinter/__init__.py`); 1-thread p50 0.65 ms; exact.
- Per-document programs add ~170 MB peak (1 thread) / ~340 MB (18 threads) vs ~1.7 GB for whole-program derived
  relations (`:171-172`).
- Round 1 reruns 458 docs because `class_q`/`fn_q` are known only after round 0; running interface strata first would
  make a from-scratch run "about 574 runs plus the documents with cross-file return types" (**A**, `:173-175`).
- Edits (**M**, `:176-180`): `codecs.py` body edit → 1 run, 3 ms; interface edit → 123 dependents, 111 ms in parallel
  (same 123 as the lattice spike); `tkinter/__init__.py` 31 ms + 9 dependents (30 ms).

### 6.3 Demand / magic sets
- **Round 2 transform** (`magic_sets`, `rdl.py:190-282`; `DE/rdl/RESULTS.md:83-115`): `.demand rel` restricts each
  listed relation to the keys (**first column**) its consumers ask for; each rule for a demanded relation gets
  `magic_rel(key)` first; each use with key bound adds `magic_rel(key) :- <literals before it>`, including negated uses
  (first version missed this and lost 3 of 166 rows); magic rules leave out negations and atoms of the head's own
  recursive component (over-approximates, keeps stratifiable); an unbound demanded key is a compile error.
- Results (**M**, `DE/rdl/RESULTS.md:100-106`): SQL eval/user/RSS 16.3 s / 13.4 s / 199 MB → **4.2 s / 3.9 s / 51 MB**;
  `cv` 1,217,464 rows, 26 iterations, 7.3 s → **580 rows**, 8 iterations; `lookup`/`type_of`/`has_def` 200,578 /
  165,373 / 87,733 → 1,541 / 1,860 / 83; Ascent 4.2 → 3.9 s, 2.7 GB both; outputs identical (179 findings, 166
  `range_const`, 1,206 `append_type`). "Demand is what makes lattices viable in SQL ... Ascent gains little ... Natively
  the bigger lever is per-document evaluation, at 26 MB per document" (`:108-111`).
- **Round 3 finer demand** (`DE/rdl/RESULTS.md:119-140`): extras (positive atoms of own recursive component, bound
  negations) kept greedily only if still stratifiable; demand that would close a negative cycle → that relation
  computed in full (`prog.demand_dropped`). "On this workload, no extra survives" (`cv($l,$a)` before `cv($r,$b)` would
  put `magic_cv` in `cv`'s component, which contains `!has_def($n)` whose demand comes from `magic_cv`). Demand already
  tight: `magic_cv` 586 keys for 580 rows; `magic_type_of` 2,572 keys for 1,860 typed nodes. Widening `.demand` to
  `resolves, assigns, augdef, bound_in`: drops `bound_in` (negative cycle), drops `assigns` (unbound key), demands
  `resolves` for 91,060 keys and pulls it into the 23-round magic component: eval 4.2 → 5.2 s. "Demand pays only when
  the demanded keys are few relative to the relation. It should not merge a cheap non-recursive stratum into a large
  recursive one" (`:139-140`). Recommendation: "Demand is chosen by cost per relation, not demanded everywhere. It is
  dropped automatically where it would break stratification" (`DE/FINDINGS.md:234-235`).
- **Round 7, magic sets for a one-file query** (`DE/rdl/RESULTS.md:869-907`): whole-corpus outputs demand everything,
  so `.demand` only adds magic relations. Query `q_target(c,f) :- qdoc(d), docof(c,d), call_target(c,f)` + `q_open` for
  `rlcompleter.py` (44 calls; 10 targets, 6 unknowns) (**M**): no demand plain SQL 12.7 s, native program 1.6 s;
  `.demand call_target, call_open, pv, bv, vopen` plain SQL **168 s**, native program **> 250 s (stopped)**; answers
  identical; 13× slower. `magic_call_target` had 53,061 keys (every call); `pv`'s magic component 153 s over 26 rounds.
  Causes: (1) dead consumers seeded demand (fixed by `prune`); (2) relations computed whole demand everything they read
  (`need_attr` asks `pv` of every receiver; also `base`, `iattr`, `qv`) — "That demand is the hand-written demand: rules
  written forward from every use"; (3) demand binds only the first column (`found(c,a,at)`, `attr_open(c,a)` need both).
- What a query-specialized program needs (**A**, `:897-904`): adornments over any bound columns (`found^bbf`,
  `attr_open^bb`); rules without hand-written demand (`need_attr`, `vpos`, `wants` become what `magic_sets`
  generates); demand components run natively ("Demand turns non-recursive strata into a recursive component that grows
  one hop per round (26 rounds here). Per-round cost is what the native set engine removed").
- "That is how the native resolver works: per file, from summaries, asking only what a call needs. Until then, one
  file's query costs about the whole program (1.6 s in program mode, against 1.5 s for every file). For incremental
  analysis the alternative is to keep the whole-program result and maintain it per changed file" (`:905-907`).
- **Uncommitted follow-up** `magic_adorn` (`rdl.py:284-467`): Soufflé magic-set transformation applied to every derived
  relation; differences from Soufflé: one relation per predicate (adornments guard copies of R's rules into R); custom
  sideways order (bound key first, connected atom before unconnected, constants alone never pick an atom); **"Key
  adornments only. An adornment whose first argument is free is all free"** (`:298,336-342` — i.e., it still requires
  the first column bound, reasons given: demand on later columns alone "restricts little and reverses the rules'
  direction, which is how it spreads"); negation kept; aggregating/aggregated relations computed in full; views and
  lattice values never adorned; relations demanded all-free anywhere computed in full (`:411-418`). **No measurement
  recorded.**

### 6.4 Whole-program vs per-file for call resolution
- "Whole-program, not per file. This program is evaluated over the whole repository. The native resolver links per-file
  summaries and re-links on an edit. The per-document Ascent split (round 3) applies to the file-local part (`walk`,
  `look`, `defn`, `bound_in`), and the cross-file part (`pv`, `found`, `qv`) is the interface fixpoint. That split was
  not built for this program" (`DE/rdl/RESULTS.md:609-612`).
- Native resolver "is still faster end to end (1.7–2.0 s with parsing), because it is per file and summary-based: it
  never materializes a whole-repository relation" (`DE/rdl/RESULTS.md:811-814`).
- Recommendation (round 3): "Per-document evaluation is the unit for engine programs. A document reruns when its facts
  or its imported interface rows change, and stops at its exports when they don't" (**A**, `DE/FINDINGS.md:237-238`).

### 6.5 The native resolver's per-file design (`NC/`, structure)
- `mod.rs:1-5`: "Cross-file `calls`: ... Per-file call summaries (`summary`) are linked into one call graph
  (`closure`); each `calls` term is answered by a backward walk from the definitions its inner rule matches, computed
  when first read and kept until an edit can change it. The inner rule runs per file as the term's seed query; the
  callee's subtree is never walked at query time."
- `summary.rs:1-17`: one Python file's summary = definitions, what each calls **by qualified name**, module/class-level
  names bound, classes' bases; "Rule-independent and file-local: callees are names, not definitions, so a summary
  changes only when its own file does." A callee is named where statically decided, else **open (unknown), never
  absent**; late binding `pkg.C.m^`; builtins/literal methods no edge; flags `ASYNC, CLASS, OPEN, DECORATED, SHADOWED`
  (`:31-39`); `Summary` fields `module, names, defs, edges, targets, aliases, opaque, stars, supers, open, open_sites,
  bytes` (`:58-90`); `EXTRACTOR_VERSION = 9` (`:28-29`); encode/decode (`:163,216`).
- `closure.rs:1-4`: "summaries linked by name, and per `calls` term the definitions that reach its seed. A term is a
  sparse reachability question: a backward walk over callers from its seed definitions, whose cost is the region it
  reaches, not the graph." `Graph` holds files by slot, `by_name`, `aliases`, `opaque`, `modules`, `supers`,
  `children`, `subclasses`, `orphans`, memoized `links`, `link_roots` relink index (`:429-466`). `Graph::link(files)`
  (`:528-540`) = `update` from empty; `Graph::update(changes)` replaces/adds/removes files and returns
  `GraphUpdate{slots, reshaped, touched, dropped}` (`:257-262,541`); per-language linkers `link_python` (`:1587`),
  `link_rust` (`:1557`), `link_php` (`:1372`); `MAX_HOPS = 16` alias/star hops before open (`:19-20`).
- `CallsView::build/update` (`mod.rs:495-560`): update "keeping every answer an edit cannot change"; returns files whose
  stamp may have changed. `stamp` = SHA-256 of every callee name's verdict and chain for the file's terms: "A file whose
  stamp is unchanged gets the same `calls` verdicts and chains at every call" (`mod.rs:685-709`). `silent` (`:670-683`).
- Rust resolver (`NC/rust.rs:1-7`): "from syntax and lexical scopes alone ... What needs types (other method calls,
  trait dispatch, closures and function pointers) is `Open`. A macro's tokens are not parsed"; a `#[path]` module makes
  the whole crate opaque (`DE/rdl/RESULTS.md:708-710`).
- `spike_bench.rs` (spike-only ignored test, "revert before merge") measures parse / summarize / `Graph::link` /
  `callee_at`+`link_of` (`NC/spike_bench.rs:1-4`; `DE/rdl/RESULTS.md:695-701`).

### 6.6 Other per-edit / incremental data points
- Statistics merge per document batch, one-shot = incremental (section 2.1).
- egglog: additions only; adding one file and re-saturating 4.3 s = 31% of a full eval for 0.4% of nodes; no
  retraction (`DE/egglog/RESULTS.md:21,40`). CodeQL: no supported incremental mode; experimental overlay 10.3 s build,
  11.2 s query wall (`DE/codeql/RESULTS.md:44-53`). Soufflé, Nemo, Ascent: none (`DE/souffle/RESULTS.md:16`,
  `DE/nemo/RESULTS.md:15`, `DE/ascent/RESULTS.md:15`). "Only Differential Dataflow is incremental" (not tested;
  `DE/FINDINGS.md:30`).

---

## 7. Engine comparisons (all marked; machine load noted)

### 7.1 Round 1, workload W1–W4b (`DE/FINDINGS.md:6-16`; load average 50–99 throughout, "compare CPU time" `:3-4`)

| Engine | Full corpus | Peak RSS | One document | Goldens | Lattice | Transducer as relation | Runtime rules | Rust embed | License |
|---|---|---|---|---|---|---|---|---|---|
| SQLite hand-lowered | 9.7 s CPU (~9 s w/o W4b) **M** | 161 MB | 20 ms | exact | host loop (naive, 19 s) | vtab (designed, not built) | yes | yes | public domain |
| Soufflé 2.5 interp | 7.8 s CPU | 338 MB | 140 ms (process) | exact | subsumption, sound for flat | functor returning a list; materialized, no pushdown | yes | no interpreter API | UPL |
| Soufflé compiled | 5.8 s CPU (+19 s C++ build) | 356 MB | 6.4 ms embedded | exact | as above | as above | no | via C shim | UPL |
| Ascent 0.8.1 | 4.1 s CPU | 1.61 GB | 5.2 ms | exact | native `lattice` | yes, demonstrated with Rerail's matcher | no (macro, 25 s build) | native | MIT/Apache |
| egglog 3.0.0 | 9.7 s CPU | 2.29 GB | 60 ms | exact | native `:merge` | partial (`rust_rule`) | yes | yes | MIT |
| Nemo 0.10.1 | 22.4 s CPU | 413 MB | 216 ms | 148 constants off | none (bounded) | no | yes | git crate, async | MIT/Apache |
| CodeQL 2.27.3 | 52.6 s DB + 35 s eval | 1.25–3.9 GB | ~20 s | exact W1–W3, W4 by AST | none (bounded) | external CSV only | yes (8–48 s compile) | no (JVM CLI) | no embedding, no private code |

Per-engine details (all **M** unless noted):
- **SQLite** (`DE/sqlite/RESULTS.md`; load not stated): 28.6 s wall, 9.7 s user, 161 MB; excluding W4b ~9 s wall
  (`enclosing` 4.5 s, `lookup` 1.7, `type_of` 1.3); W4b 19.4 s, 25 naive rounds; one doc 0.02 s in-process (0.06 s
  with Python start); 169 lines (`:7-14`). Hand-lowering choices a compiler must make: folding mutual recursion into
  one tagged CTE, one recursive reference per SELECT, NOT EXISTS, host loop lattice, indexes (`:16-21`).
- **Soufflé** (`DE/souffle/RESULTS.md`; shared machine, CPU reliable): interp 10.5 s wall / 7.8 s user / 338 MB;
  compiled 6.4 / 5.8 / 356 MB; load 0.6 s; C++ 1 s + 18.7 s; one doc process 0.14–0.19 s interp, 0.02 s compiled;
  embedded 6.4 ms avg over 574 docs; all 574 one at a time 3.7 s; `-j 8` no speedup (no OpenMP) (`:8-17`). Profile:
  `enclosing` 5.3 s of 7.8 s, `cv` 1.7 s, `type_of` 0.5 s (`:20-22`). Subsumption = golden; no subsumption ±1000
  > 290 s killed; ±10 27 s, 148 rows wrong (`:32-36`). Relation-valued externals: stateful functor returning cons
  list, no demand pushdown, materialized (`:55`). Interpreter ~1.6× compiled speed (`:56`). SQLite in/out via
  `.input IO=sqlite` (own connection) (`:58`). 143 lines (`:64`).
- **Ascent** (`DE/ascent/RESULTS.md`; load 70–85): load 1.36 s, eval 4.67 s, 7.1 s wall, 4.1 s user, 1.61 GB; one doc
  5.2 ms, 12.5 MB; 130 lines; ~25 s crate compile per program change; no incremental (`:8-15`). Plug-in: `for (s, e) in
  matcher.matches(path)` in a rule body with `path` bound (`:24`); 574 docs × 2 rules 3.64 s in the matcher, 3.2 ms per
  (file, rule) through JSON `evaluate` which re-parses per call; findings B110 26 (golden 29), NO_TIMEOUT 21 (golden
  20), differences from rule semantics (comment nodes counted; module-local `check_output` resolved) (`:32-40`).
  Required a feature-gated `spike_api` in `native/` (`:42-43`).
- **egglog** (`DE/egglog/RESULTS.md`; load 67–99): 1 thread load 4.0 + eval 14.8 s, 22.4 s wall, 9.7 s user, 2.29 GB;
  18 threads 9.5 s wall, 26 s user, 3.84 GB; one doc 0.06 s wall, 20 MB; incremental add 4.3 s (31%); 187 lines
  (+31%) + 3-line Rust primitive; 3 major versions in 12 months (`:15-24`). No stratified negation (emulated in 9
  places by hand), no aggregates (`:32-33`); `rust_rule` demand-first transducer call: 2,568 `ast_match` rows in 84 ms
  over 574 docs (`:36`).
- **Nemo** (`DE/nemo/RESULTS.md`; shared, wall ≈ 2.7× CPU): 49.9 s wall / 22.4 s user / 413 MB; import 3.7 /
  reasoning 45.5 / export 0.6 s wall; one doc 0.32 s wall, 216 ms engine (`:11-14`); ~3× Soufflé interp CPU, ~4×
  compiled (`:18-19`); `cv_const` 148 rows differ (bounded ±10), 0 differ from Soufflé bounded variant (`:27`); no
  lattices, no UDFs (closed enum), no relation-valued externals without a fork (`:36-40`); 119 lines (`:50`).
- **CodeQL** (`DE/codeql/RESULTS.md`; 18 cores, 128 GB, single runs; load not stated): DB create 52.6 s (97.5 CPU-s),
  1.25 GB, 44 MB; W1–W3 59.0 s (compile 29.6–39.5, eval 22.2), 3.6 GB; W4 eval 5.1 s; W4b eval 8.1 s; API-graph W3
  107 s, 3.9 GB; points-to 139 s, 3.3 GB (`:25-33`). One doc: DB 7.7–14.8 s; query 10.7 s wall (eval 0.6–2 s); "about
  20 s" per-doc derivation (`:38-42`). Overlay (experimental "do not use"): 10.3 s build, 11.2 s query wall, eval 4.3 s
  vs 25.1 s base, 30 B110 (`:44-53`). W3 API graphs: 22 rows vs 20 (2 extra need flow) (`:59-63`). W4: 144,044 agree;
  20,506 golden-only (18,870 concatenated string parts; 565 attribute/keyword names; 208 parens; 81 `global`; ~780
  positional); 2,144 CodeQL-only (`:65-78`). W4b 1,649/1,655 agree (`:80-83`). Points-to vs ours over 339,960
  Name/Call: 122,995 (36%) vs 50,138 (15%); both 26,175, agree on class in 15,904 (`:85-90`). No lattices (depth-bounded
  d ≤ 32 workaround), 32-bit int (`:97-98`). 259 lines QL vs 143 Soufflé (`:111`). License forbids embedding,
  redistribution, and use on non-open-source code (`:126-139`). Borrow: class-based typed AST predicates, API graphs as
  model for `resolves`, data extensions as model for knowledge packs (`rerail.types/v1`) (`:153-156`).

### 7.2 Round 2: rdl backends (`DE/rdl/RESULTS.md:17-24`; load 41–54)
- Compiled SQL (SQLite, Python driver): 26.9 s eval (`cv` 12.0 s, `lookup` 7.0 s), 13.1 s user, 210 MB; one doc 0.3 s;
  exact. Generated Ascent: 2.64 s eval, 3.5 s user, 2.7 GB; one doc 8 ms eval (26 MB per doc, `DE/FINDINGS.md:80`);
  exact. Comparison: hand-written SQL ~9 s + 19.4 s naive lattice, 161 MB, 20 ms; hand-written Ascent (materialized
  `enclosing`) 4.67 s, 1.6 GB, 5 ms. Generated Ascent beats hand-written "because the plan and the tape climb replace
  `enclosing`" (`DE/FINDINGS.md:81`).

### 7.3 Round 3/4: native pieces and set fixpoint (sections 2.5, 4.8; loads 40, 3–14, 12–25 as noted there)
- Full workload no demand: 17.3 s eval / 13.2 s user → **5.6 s / 4.6 s** with all native pieces (**M**,
  `DE/rdl/RESULTS.md:273-278`; FINDINGS `:123`).
- Set fixpoint on `returns`/`type_of`: 3 edge rules (155k edge rows), 1 probe rule; 15 waves; 12 probe statements run,
  3 skipped; 498k visits, 37k new rows, 79 ms solve. Demand component: 9 relations, 7 edge and 16 probe rules; 22
  waves; 211 probe statements run, 251 skipped; 0.08 s vs 0.19–0.24 s SQL semi-naive (**M**, `DE/rdl/RESULTS.md:389-396`).
- One document (`facts-one`, demand workload), min of 8 runs: PLAN0 0.090 s; plan fixes 0.100 s; all native 0.060 s;
  magic component 0.040 s SQL vs 0.010 s native (**M**, `:414-421`).
- After the vtab fix (round 5): 1.15 s (demand) and 1.99 s (full) with all native pieces (`:592-594`); round 5 machine:
  replanning on or off 2.7–3.1 s full and 1.3–1.65 s demand (`:675-676`).

### 7.4 Round 5: call resolution (`DE/rdl/RESULTS.md:507-686`; loads: SQL plain at 28–35)
- CPython runtime oracle (`fixtures/python/calls`, `sys.monitoring`; scoring = `python_tests.rs`'s): native resolver 76
  agree / 17 departures (all unknown) / 0 wrong; `calls.rdl` **84 / 9 / 0**, each departure also a native departure
  (**M**, `:513-516`). Resolved extra: methods through receiver type (3), instance `__call__`, `super()` (2), decorator's
  returned wrapper, returned closure (`:518-524`). Remaining 9: star import, two called parameters, `lru_cache`,
  subscript, `getattr` result, two lambdas, `partial()` (`:526-534`).
- Jedi 0.19.2 `goto`, 574 files (**M**, `:551-559`): 54,720 calls recorded; Jedi undecided 11,234; `calls.rdl` unknown
  where Jedi decided 10,175; decided by both 33,309; agreed **33,036 (99.18%)**; explained divergences 236 (130
  `try: from _x import f` accelerators, 67 late-bound `self.m()`, 39 decorated); other 37. Two real bugs fixed (`global`
  bindings: 18 of 20 no-edge-vs-definition calls; external-base attribute as known value) (`:561-569`).
- Corpus totals Python: 54,823 calls; 16,582 (30%) named (784 several); 19,310 (35%) unknown; 18,931 (35%) no edge;
  15,315 call-graph edges; W5 no blocking call found; 388 async calls unknown (`:571-578`).
- Cost Python (**M**, `:582-587`): SQL plain 20.8–27.2 s (load 28–35), 245 MB (points-to component 8.0 s, `vopen` 4.6
  s, `walk` 3.8 s); SQL + native set fixpoint 13.3 s, 11.1 s with replanned probes (points-to 2.7 s solve vs 90.9 s
  before the vtab fix); generated Ascent 1.9–2.0 s, 2.99 GB; fixture project (9 files) 0.2 s SQL.
- Rust rust-analyzer fixture: 37 agree / 7 departures / 0 wrong (4 trait dispatch unknown, 2 iterator chain unknown, 1
  explained) (`:638-649`). `native/src` 331 files: 18,119 (25%) named; 35,151 (48%) unknown (32,136 method calls with
  unknown receiver type); 19,988 (27%) no edge; 22 random samples hand-checked correct (`:651-658`).
- Cost Rust (**M**, `:660-665`): SQL plain 11.7–15.3 s, 143 MB; SQL + native probes planned once 11.3 s (`res` 5.3 s);
  replanned **7.4 s**, 229 MB (`res` 1.5 s); generated Ascent **1.2 s** eval (1.4 s load), 2.33 GB. "generated Ascent
  is 6–9× faster than SQL" (`:682-683`).

### 7.5 Round 6: against the native resolver (`DE/rdl/RESULTS.md:688-814`; CPU user+sys, load ~29; Python program mode
measured 2.08 s at a quieter moment, `:690-691`)

| | Python, 574 files | Rust, 331 files |
|---|---|---|
| Native resolver total | 1.74–1.92 s (parse 0.60–0.71, summarize 0.67–0.74, link 0.08–0.12, answer 0.35–0.38); 292 MB | 1.88–1.99 s (parse 0.53–0.55, summarize 0.91–0.98, link 0.01, answer 0.43–0.45); 313 MB (328 files) |
| rdl plain SQL | 17.5–17.6 s; 461 MB | 8.2–8.4 s; 308 MB |
| rdl SQL + native (`all`) | 6.1–6.4 s exec, 1.0 s construction; 1.06 GB | 3.8–4.0 s exec, 0.2 s construction; 0.50 GB |
| rdl native program (`all,program`) | 2.7–2.8 s exec (2.08 at lower load), 2.2 s construction; 1.5 GB | 1.66–1.81 s exec, 1.6 s construction; 1.0 GB |
| Generated Ascent | 1.92–2.01 s eval, 1.4–1.5 s load; 2.8 GB | 1.03–1.16 s eval, 1.3 s load; 2.2 GB |

(**M**, `:724-730`.) Call counts (`:712-717`): Python native 54,823 / 16,641 named / 17,977 unknown / 20,205 no edge vs
`calls.rdl` 16,582 / 19,310 / 18,931; Rust native (328 files) 71,517 / 12,289 / 53,067 / 6,161 vs `calls_rust.rdl` (331)
73,258 / 18,119 / 35,151 / 19,988. "rdl's whole-program execution is now at parity with Ascent and within 1–1.5× of
the native resolver's resolution phases (summarize + link + answer: 1.1–1.4 s), while resolving more calls"
(`:812-814`). Memory: program mode 1.0–1.5 GB = half of Ascent, 3–5× the native resolver's 0.3 GB (`:809-810`).

### 7.6 Round 7 (`DE/rdl/RESULTS.md:816-829`; CPU, two runs, load 18–21)

| | Python | Rust |
|---|---|---|
| Native resolver | 1.87–1.93 s; 307 MB | 2.03–2.06 s (328 files); 328 MB |
| rdl plain SQL | 14.0 s; 368 MB | 8.0–8.3 s; 288 MB |
| rdl SQL + native | 5.0–5.2 s; 662 MB | 3.5 s; 374 MB |
| rdl native program | **1.48–1.67 s** exec, 1.4–1.5 s construction; **640 MB** | **1.13–1.20 s** exec, 1.2 s construction; **450 MB** |
| Generated Ascent | 1.80–1.84 s eval, 1.3–1.4 s load; 2.8 GB | 1.01–1.09 s eval, 1.3 s load; 2.2 GB |

(**M**.) Native run phase alone 1.19 s (Py) / 0.59 s (Rust) vs Ascent 1.8 / 1.0 s eval (`:829`). Fixture scores unchanged
(Python 84/9/0, Rust 37/7/0) (`:819`).

---

## 8. The user's typing requirements (`MEM`)

- Rejected: collecting types from a full type checker (ty/pyright, tsc/tsgo, rust-analyzer): "too slow, memory hungry
  and unwieldy. Nobody will turn that on." (`MEM:10`).
- Instead: approximations refined by language-specific progressive inference programs over the AST, invalidated on
  source change like any other fact; aim for 80/20 to support better linter rules; 100% agreement is the job of a real
  type checker plus strategic annotations/concrete types in the source where inference can't work (`MEM:10`).
- Requirements added the same day (2026-10-25) (`MEM:14`):
  - rules must be able to say both **"provably true"** and **"not provably false"** (unknown counts as a finding) —
    both supported;
  - there must be **ways to tell the checker the types** so a rule can be made to pass;
  - **parameterized/generic types are first-class**.
- Why: heavy checkers break the per-edit and 1–2 s audit targets (`MEM:16`).
- How to apply: never propose embedding a checker as a producer; design inference as **bounded, per-file, cacheable
  passes with cross-file interface summaries** (`MEM:17`).
- Work only in the `Typing-support` worktree (`MEM:12`).
- Relation to spike evidence (only what sources say): the spike's `cv` lattice's `top` means "not provably constant"
  (`DE/rdl/RESULTS.md:238-239`); the call programs' `*_open` relations give the unknown side (section 1.5). **Not found**
  in the spike: any type-annotation input relation ("tell the checker the types"), any parameterized/generic type
  representation (W4 types are bare symbols like `int`, `list`, class qualified names, `workload.rdl:73-77,108`; Rust
  generics only make a receiver external/unknown, `calls_rust.rdl:224-231`), or any "provably true vs not provably
  false" rule-level operator. CodeQL's data extensions are named as the model for knowledge packs `rerail.types/v1`
  (`DE/codeql/RESULTS.md:156`).

---

## Synthesis (restating only what the sources establish)

### Recommendation as it stands after all rounds
1. **Datalog is the semantics, not someone else's runtime** (`DE/FINDINGS.md:175-177`). The same program ran on six
   engines with identical results; no engine provides SQL-visible state, per-document derivations with invalidation,
   three-valued base facts, transducer with demand pushdown, and fast+small runtime-loaded rules (each misses ≥2)
   (`:20-30`). CodeQL is the precision reference (API graphs, points-to 36% vs 15%) and is license-excluded (`:48-53`).
2. **Language**: `$X` variables, `ast(...)` matcher atoms (not built), `lattice` with widening (only `flat`, no widening
   syntax built), stratified negation and aggregates (`count` only) (`:179-181`).
3. **Tree structure is never Datalog recursion**: `nearest` (built), `within`/`inside` (named, not built) as tape
   built-ins; anchor by subset DP; climb vs descent by the efreq Markov chain / mean subtree size (`:197-201`).
4. **Lattices**: declared join + finite height or widening; per-doc SCC condensation from scratch, semi-naive waves for
   deltas, parallel across documents; cross-file module-condensation layers with interface-digest early cutoff; in SQL
   only with demand seeding, otherwise native behind a vtab; Metal = 32-bit-word waves, ≤ h × n entries (`:202-208`).
5. **Recursive components run natively** with joins in planned SQL (edge tables when cheaper than a delta pass, per-wave
   probes otherwise), recognized from the Datalog — "Rules still carry no SQL" (`:217-226`); later rounds move probes
   and whole strata into the native engine (program mode, round 6) with an arena store (round 7).
6. **Planner statistics** built at ingestion, postings-weighted, merged per batch, fingerprinted, read in one statement;
   document-local joins as tape-interval ranges (`:227-230`); plus observed fanouts and replanning between chunks of
   waves for derived relations (round 6).
7. **Transfer functions registered natively with declared properties** (absorbing element, strictness) — worth 2.5× in
   rounds; SQL joins can't express them (`:231-232`). (Only `flat_bin` exists; properties are an env flag `RDL_ABSORB`.)
8. **Per-kind defaults** declared over kinds, expanded from the tape (`:233`).
9. **Demand chosen by cost per relation**, dropped where it would break stratification (`:234-235`); for queries, needs
   adornments over any bound columns and rules without hand-written demand (round 7).
10. **Engine programs planned once from merged statistics per language** at build time, no per-document variants
    (`:236`).
11. **Per-document evaluation is the unit for engine programs**; rerun on own facts or imported interface rows change,
    stop at exports when unchanged (`:237-238`).
12. **Construction should come from the tape and anchor index**, not copies of facts into hash indexes (`:153`;
    `DE/rdl/RESULTS.md:805-806`).
13. **Compiler checks** for the four SQL-shape lessons (order enforced, walks threaded, typed columns, indexes per access
    pattern) (`DE/FINDINGS.md:209-213`).

### Key numbers to carry (all M unless marked)
- Native program mode (round 7): Python 1.48–1.67 s exec / 640 MB; Rust 1.13–1.20 s / 450 MB; run phase 1.19 / 0.59 s;
  construction 1.4–1.5 / 1.2 s; load 18–21. Native resolver: 1.87–1.93 / 2.03–2.06 s, 307 / 328 MB.
- Lattices natively: `type_of` semi-naive 10–11 ms, `cv` 6–8 ms over the whole corpus; largest doc < 0.3 ms; cross-file
  layered 46–71 ms; edit cutoff 1 vs 408 and 123 vs 408 (load 40–50).
- SQL lattice: 12 s (no demand) → 580 rows with demand; native `cv` 0.50 s via vtab.
- Per-doc Ascent: p50 1.1 ms, p99 33–47 ms; body edit 3 ms; interface edit 111 ms for 123 docs.
- Statistics: 0.73 B/node, 7.4–8.7 s build (5.4 s in round 6), 0.03–0.05 s read.
- Metal: < 1 ms (**A**, estimate from another doc).

### Contradictions and revisions between rounds (later rounds revise earlier ones)
- **Ascent codegen for engine-shipped programs** (round 1: "Engine-shipped programs get generated into Ascent at build
  time. Ascent was the fastest", `DE/FINDINGS.md:186-189`) vs rounds 6–7: native program mode equals then beats Ascent's
  eval (run phase 1.19 s vs 1.8 s Python) at a quarter of the memory; "Codegen per program is worth at most the
  interpreter's share, and would lose replanning between chunks" (`DE/FINDINGS.md:153`; `DE/rdl/RESULTS.md:800-804,829`).
  FINDINGS' Recommendation section was **not rewritten** after rounds 5–7; its last "Revised after" block is round 3.
- **Repository strata in SQLite** (round 1, `DE/FINDINGS.md:182-184`, "smallest in memory 161 MB") vs round 6/7 program
  mode keeping set-valued relations native between strata with SQLite only for fact loads, non-native strata and output
  writes. No later round restates where repository strata live.
- **Lattices in SQL**: round 1 "Add semi-naive delta evaluation for lattice strata" (`DE/FINDINGS.md:184`) → round 2
  "SQL lattices are the weak spot" (12 s semi-naive) → round 3 native `lattice_fixpoint` 0.5 s, and "With demand, native
  adds nothing" (`DE/rdl/RESULTS.md:294`).
- **Demand**: round 2 demand cut SQL 16.3 → 4.2 s; round 3 says wider demand is a cost decision (4.2 → 5.2 s); round 7
  demand for a one-file query was 13× slower (168 s) — first-column-only demand plus hand-written demand rules are the
  cause. The uncommitted `magic_adorn` still restricts to key (first-column) adornments, contrary to round 7's stated
  need for adornments over any bound columns; no results recorded.
- **"70 ms per round" fixed cost** (round 3) retracted: "Wrong ... round setup and dedup are 0.1–0.2 s over all 23–27
  rounds"; it was two bad plans inflated by load 35–70 (`DE/rdl/RESULTS.md:240-242`; `DE/FINDINGS.md:120`).
- **Set-fixpoint probes**: round 3 SQL statements over `set_rel` vtabs per wave → round 6 native join probes (pv solve
  3.1 → 0.46 s; `res` 1.5 → 0.22 s, `DE/rdl/RESULTS.md:756-761`).
- **vtab index choice**: first usable equality → shortest posting at filter time (52.5 s probe fixed, round 5).
- **Statistics source**: round 2 used the exporter's Rust `kfreq`/`efreq` and plan-time GROUP BYs; round 3/4 moved to
  ingestion-time `stats.py` (identical plans); `ANALYZE` kept but unread.
- **rdl.py size**: "about 1,000 lines" (`DE/rdl/RESULTS.md:3`) vs 1,795 lines now (including uncommitted magic_adorn).
- **README fact table** lists `text` for identifier and dotted_name only and omits `tape`/`kfreq`/`efreq`; the exporter
  now emits tape, kfreq, efreq, pkg and more text kinds (`DE/README.md:16-25` vs `exporter/src/main.rs:52,91`).
- **Hand-written Ascent** "4.1 s CPU" (`DE/FINDINGS.md:13`, user CPU) vs "4.67 s eval" (`DE/rdl/RESULTS.md:24`) — same
  run, different measures (`DE/ascent/RESULTS.md:10`: eval 4.67 s; 4.1 s user).
- **Native resolver memory**: 292/313 MB (round 6) vs 307/328 MB (round 7); FINDINGS round 6 says "0.3 GB".
- **Python program mode** round 6: 2.7–2.8 s (2.08 at lower load) → round 7 1.48–1.67 s (arena).
- **Lattice widening "only at SCC heads"** is a recommendation; the spike's K cap is applied in every join.
- **Atomic encoding**: CPU spike uses 64-bit words; Metal estimate requires 32-bit words "(to verify)".

### Stated open items (from the sources)
- Storage was "the next 2×" (round 6) — done in round 7 (arena). Node ids in u32 proposed, not done (`DE/rdl/RESULTS.md:799`).
- Plan caching by program + statistics digest, or a native DP (`DE/rdl/RESULTS.md:807-808`).
- Construction from tape and anchor index instead of hash-index copies (`DE/rdl/RESULTS.md:805-806`).
- Query-specialized magic sets: adornments over any bound columns; derive demand instead of hand-written `need_attr`,
  `vpos`, `wants`; demand components natively (`DE/rdl/RESULTS.md:897-904`). Alternative: keep the whole-program result
  and maintain it per changed file (`:906-907`).
- Per-file split of `calls.rdl` (file-local `walk`/`look`/`defn`/`bound_in` vs cross-file `pv`/`found`/`qv` interface
  fixpoint) — not built (`DE/rdl/RESULTS.md:609-612`).
- Not measured / not covered by `calls.rdl`: parameter flow, `__getattr__`, `__new__`, metaclass `__call__`, C3 MRO
  (union over bases), relative imports with ≥2 dots (unknown) (`DE/rdl/RESULTS.md:613-617`).
- Rust: method-call unknowns need "the expression typing from round 4 (call results, closures, iterator adapters)
  carried into `etype`" (`DE/rdl/RESULTS.md:657-658`); no such round-4 write-up found.
- Running interface strata first in per-doc mode to avoid the 458-doc rerun (**A**, `DE/rdl/RESULTS.md:173-175`).
- `tape_members` table-valued function for SQL descents (proposed; descent never chosen) (`DE/rdl/RESULTS.md:306-307`).
- Metal encoding (32-bit CAS) "to verify" (`DE/lattice/RESULTS.md:105-107`).
- Transducer as a vtab for SQLite: "designed, not built here" (`DE/FINDINGS.md:10`); `ast(...)` atoms and
  `within`/`inside` built-ins not built.
- Typing requirements with no spike evidence yet: type annotations as input facts, generic/parameterized types as
  first-class values, explicit "provably true" vs "not provably false" rule semantics (section 8).
