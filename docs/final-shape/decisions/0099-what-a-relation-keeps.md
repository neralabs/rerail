# 0099: What a relation keeps: recompute in the reader, restrict by demand, or store, decided per relation

Status: built (2026-11-09): items 1 to 4, with the constants from the relational lab below; item 1 prices from
observed step counts where an earlier chunk has them. The factorized store is analysed and designed, not built.

## Intro

**The question (the user).** "Some of the relations should maybe not be materialized. Something which creates a
massive expansion in an intermediate join before narrowing down should maybe be recomputed in the intermediate join
and not serialized. This could be done by analysis or explicitly in the program."

**The answer in short.**

- **It is the norm in this engine's cost, not an edge case.**
- **Mostly by analysis.** The relations that expand and then narrow are mostly ones the converter and the engine
  make (`rr_agg*`, `fl*`, `demand_*`), which no author would annotate. So the planner should decide, per derived
  relation, whether to store it, recompute it inside its readers, or restrict it by demand.
- **An annotation is an override:** Soufflé's `inline` / `no_inline`, `magic` / `no_magic`.

**How to read this record.** First what the Python pack shows. Then the options and when each wins, the decision
proposed, and what is built.

## What the Python pack shows

These are estimates from the profile taken after 0095's fixes (12.45 s of placed CPU, 139.9 M units of join work)
and the relational lab's 144 captures. No inlining experiment was run.

- **Native strata write 58.9 M rows.**
- **208 relations, 28.0 M rows, are written large and read small.** That is 47.6% of the rows and 45% of the join
  work, about 6 s. Each of them has at least 10 k rows, is read with some columns bound, and its readers keep at most
  a tenth.
  - Their readers probe them 1.44 M times and keep 273 k rows.
  - 154 keep under 1%. 150 have a single reader.
- **How their readers read them:**

  | Read | Relations | Rows |
  |---|---:|---:|
  | only inside `count {…} = 0` or `min {…}` (the converter's `rr_agg*`) | 38 | 13.0 M |
  | positive lookups | 95 | 8.7 M |
  | file-local pieces (`fl*`) | 36 | 3.0 M |
  | demand's own relations (`demand_*`) | 17 | 2.2 M |
  | negation | 16 | 0.8 M |

- **The largest cases:**
  - **`rr_agg77`**: 3.9 M rows, and as many again in its annotation copy. It joins `rr_inside_ks11` with
    `bound_in`, and its one reader asks `count {rr_agg77(_, s, x)} = 0` of 55,314 pairs and keeps 405. In a capture
    its inputs hold 63 k and 2.6 k rows, and their join 617 k. Recomputed per pair, it costs at most about 8 probes.
  - **`module_of`**: 954 k rows of node, document, module. Its seven readers bind the node and keep 17.5 k.
  - **`fl4683_p207_outer`**: 777 k rows, probed 2,548 times, keeping 647.
  - **Nearly all waste:** `fl5245_p514_effect` (363 k rows, probed 7 times, keeps none) and `p358_final_alias`
    (232 k rows of a cross product, negated by 3).
  - **Demand making the blow-up:** `demand_tp_name` holds 338 k rows to answer 155.
- **Inside one rule:** 50 plans pass a step of 100 k rows or more that is at least 10× their output, 16% of the
  work. The worst are an equality applied after a cross product: `fl4317_typing_root` goes from 14 k rows to 2.86 M
  to 603. That is a planner bug, not a storage question.

**Why demand (0048) misses them.**

- It drops a relation read inside an aggregate, and `count {…} = 0` is how the converter writes "no such row".
- It drops a relation whose rule has a variable of unknown type.
- It is off in sessions. A relation computed for some keys only can miss a key asked later (O125).
- `inline` on a `.decl` is refused ("not yet").

## The options

| Option | Wins when | Costs |
|---|---|---|
| **Store it** (today) | Many readers, or many probes per key; recursive; read unbound; an output | Rows written, indexed, kept per chunk and maintained across edits |
| **Recompute in the reader** (inline) | Not recursive; every reader binds a key from which the relation's body is cheap; probes × that cost ≪ its size. Existence reads gain most: they stop at the first match. | The reader is copied once per rule of the relation. Work repeats if many probes hit one key. A count, sum or string join needs the distinct rows, so only existence aggregates inline freely. |
| **Restrict by demand** (magic sets, 0048) | Many probes share few keys | Extra strata. A demand relation can itself be wide. Unsound in a session unless demand is maintained too. |
| **Project it** (`count only`, 0098 §1) | Readers ignore some columns | None: the rows are fewer and the same to every reader |
| **Store it factorized** | It must cross the chunk boundary, or is read only by key or to count: `rr_agg77`'s two inputs (66 k rows) instead of their join (617 k) | A second layout for readers and kernels |
| **Better join order** | The blow-up is inside one rule | Nothing for a relation whose own output is large; only the reader's binding narrows that |

Two consequences beyond CPU:

- **Maintenance across edits (0090, 0092).** A kept relation is paid for per row on every run, and a wide one widens
  over-deletion. An inlined relation is maintained through its inputs, which are maintained anyway. So inlining
  shrinks the kept state and stays exact.
- **Device waves (0095 §5, 0098).** Inlining is kernel fusion: no intermediate to size, keep resident or spill.
  Storing pays on the device only when several waves read the rows.

Memory matters as much as CPU: the pack's peak is 1.39 GB against the 1 GB budget, and the chunk size is set by
memory.

## Decision (proposed)

1. **The planner decides per derived relation, after `simplify` and before demand:** store, inline, or demand. Its
   inputs are the relation's size and the work to build it, each reader's probes into it (the plan's rows into the
   step), its body's cost per probe given the reader's bound columns (to the first match for an existence read), the
   distinct probe keys, and a per-row price to store, index and maintain:

   ```text
   inline       Σ probes × fan-out < α (work + m·|R|), every reader binds a key the body reaches from,
                at most K rules, and every aggregate read is an existence test
   demand       Σ keys × fan-out ≪ work, and probes ≫ keys (corpus runs, until demand is maintained)
   store        otherwise; always when recursive, an output, read unbound, or read by readers whose probes
                sum past |R|; factorized when it crosses the chunk boundary
   ```

   The estimates are the plan's on the first run, and the observed step counts afterwards (`native.tsv` has both).
   The profile names every choice and why, as demand's do.
2. **An existence count is a negation.** `count {R(…)} = 0` is read as `!R(…)` by demand and by inlining, as the
   projection already does (0098). That alone brings in the 13 M rows read only inside such counts.
3. **The program may override.** `.decl r(…) inline` and `no_inline`, `magic` and `no_magic`, as Soufflé spells
   them. The converter writes `inline` on the helpers it makes when the analysis would, so the choice is visible in
   the program.
4. **Separately:** an equality after a cross product inside one rule is a planner fix. Worst-case-optimal joins are
   for cyclic bodies only.

## Built

- **Projection (`count only`), 0098 §1:** the cheapest of the options, and the first.
- **An existence count is a negation (item 2).** Before anything else in `simplify`, a count compared only with 0 is
  the atom it counts: `count : { r(…) } = 0` is `!r(…)`, and `> 0` is `r(…)`. The count's own variables are `_`
  already, so the atom is a plain read. Demand, inlining and projection then treat it as any other read.
  `RERAIL_EXISTENCE_COUNTS=off` keeps the counts.
- **The program's override (item 3).** `.decl r(…) inline`, `no_inline`, `magic` and `no_magic` are read as Soufflé
  spells them; two that disagree are an error. `no_magic` keeps a relation from demand; `magic` demands it past the
  size test. An `inline` that cannot be honoured (recursive, an output, a negated read of a join) is an error, not a
  quiet store.
- **The choice per relation (item 1).** In the planner, after `simplify` and before demand (`plan/keeps.rs`). For
  each derived relation it prices two things with the plan's own estimates:
  - stored: the work to build it, plus `M` per row;
  - inlined: the work its readers add once each reads the relation's rules in place of its rows.

  It inlines when the added work is under `α` times the stored cost, the relation has at most `K` rules, and
  inlining makes at most 8 reader copies. A positive read becomes one reader copy per rule. A negated read is
  unfolded only into a negation of plain literals: a single atom whose own variables appear once (`!b(t, _)`), or
  literals over the head's variables alone (their disjunction, one copy per literal). Anything else stays stored.
  A head that names a variable twice, or has a non-variable term, adds an equality to the copy. Only the head's side
  of that equality is renamed apart; the read's side is the reader's own terms. Renaming both sides once turned
  `expr_name(t, n, x)`, read against `expr_name(n, n, x)`, into `t = t`, which left `n` free. B007 then reported
  15,289 rows on the fixtures instead of 82. The fix is tested by `a_head_variable_named_twice_equates_the_readers_terms`.
  Every choice is a profile line under "inline or store (decision 0099)", with the numbers.
  `RERAIL_INLINE=off` stores everything. `RERAIL_INLINE_ONLY=r,…` inlines only those, for the lab.

**The lab experiment.** These numbers come from 8 programs that read `bound_in`, on the 486 fixture files, in 2
chunks. In that program set the pack's three relations carry other numbers: `rr_agg77` is `rr_agg59`, and
`fl4683_p207_outer` is `fl858_p4_outer`. Strata were captured with everything stored, and again with `module_of`
inlined and the existence counts read as negations. Each capture was replayed alone. CPU is the replay thread's
user + sys, summed over both chunks. Load was 100 to 140 throughout.

| Relation | Stored: build + readers | Changed: readers | Rows not written |
|---|---:|---:|---:|
| `module_of` (inlined; 4 of its 7 readers captured) | 235 + 696 ms | 392 ms | 129,313 |
| `rr_agg59` (now demanded: its read is a negation) | 1,099 + 1,233 ms | 49 + 1 + 24 ms | 772,613 |
| `fl858_p4_outer` (stays stored: it crosses the chunk boundary) | 1,108 + 555 ms | 545 ms | none |

- **`module_of`** costs less inlined in every reader. Its readers did 303 ms less work without its stored rows, so
  the rows cost that much beyond their build: there are 129 k of them, and its build runs at 0.6 µs per unit of
  work. That is about 4 units a row, so **`M` = 4**.
- **`rr_agg59` cannot be inlined.** Its read is a negation of a join with a variable of its own (`!∃p. inside(s, p),
  bound_in(p, x)`), and no single literal says that. Read as a negation, though, demand (0048) takes it: 31 times
  less CPU, 1 row in place of 773 k.
- **`fl858_p4_outer` is an export.** The cross-file part reads it, so it must be stored. Only the factorized store
  shrinks it.
- **`α` = 1.** Both sides of the test are in the planner's units, so 1 is break-even, and no case measured was near
  it. `module_of`'s readers were estimated to save work; the relations kept stored had readers estimated to add 1.5
  to 300 times their stored cost.
- **`K` = 4 rules**, with at most 8 reader copies. `module_of` has 1 rule and 7 readers. The relations refused for
  size have 3 to 5 rules and 9 to 30 copies.

**The gate.** Findings with everything on are identical to those with `RERAIL_INLINE=off RERAIL_EXISTENCE_COUNTS=off`.
This held on the 8 programs, and on 12 that read counts (5 more programs). On the 8, the analysis inlined 18 to 19
relations per part (`module_of`, `scope_node`, `tp_decl`, `exists_11`, …), and read 34 counts as plain reads.
Demand then took 34 relations in the largest part, against 28 with the counts kept. Local CPU was
14.6 + 0.4 s on against 16.5 + 0.3 s off, at load 125 to 145. On the 12 it was 22.3 + 0.5 s against
22.9 + 0.4 s, at load 50 to 100.

**The planner fix (item 4).** An equality of a bound variable (or a constant) with an unbound one, `$s = $a` with
`$s` bound, could only test: only an unbound *left* side was assigned. The typing-context library's string
annotation, `typing_root(s) :- ann_root(a), (s = a ; within(s, a)), parsed(s, "annotation", _)`, lowers to a rule
with `$s = $a`. Its file-local piece (`fl4317_typing_root` in the pack; `fl860_p0_typing_root` with PYI055 and UP035
alone) was ordered `parsed($s, …), ann_root($a), $s = $a`: a cross product narrowed after. Now the equality assigns
its right side (`binds_right`, read by the planner's `ready` and the SQL emitter; the native spec already put the
bound side right), so it runs between the joins and `ann_root($a)` is a lookup:

| `fl860_p0_typing_root`, first chunk | Steps (rows out) | Work |
|---|---|---:|
| before | 12,256 → 12,256 → 12,256 → **2,463,456** → 603 | 2,505,544 |
| after | 12,256 → 12,256 → 12,256 → 12,256 → 603 | 49,028 |

`RERAIL_EQUALITY_BINDS=off` restores the old reading. The test is
`plan::tests::an_equality_binds_its_right_side_before_the_next_join`.

**Observed step counts (item 1, "the estimates are the plan's on the first run").** The native strata record, per
rule (its head and body), the rows into and out of every step of every probe of it; per step pattern (below), the
same rows pooled over every rule; and per relation its rows and, per column, its distinct values and fanout when its
stratum ends (`keeps::observe`, `observe_steps`). The record is kept on the thread for the corpus run, beside its
plans, and read per node of the corpus. It is on by default; `RERAIL_OBSERVED_STEPS=off` keeps none, as the
comparison.

*The regression, profiled.* With observations first built, PYI055 + UP035 in 2 chunks did 29.4 M of native work
against 26.0 M from estimates alone. All of it was `rr_agg56`, the one reader of `rr_ctx57(n, s, x) :- load_ref(n,
x), look(n, x, s), rr_nearest_ks13(n, s), x != 8173, x != 8176`: inlined, its copies did 3.3 M a run where the stored
reader did 0.2 M. `rr_ctx57` is planned under 1,024 rows, so the first parts of chunk 1 store it unpriced; it measured
1,433 rows a chunk, so later parts price it, and the copies, which never ran, were priced by estimates that were
wrong in opposite directions on the two sides:

| Term | Step | Binding pattern | Estimated | Observed |
|---|---|---|---:|---:|
| stored reader | `binding($s, $x, $b, _)` after `rr_ctx57` | scope and name bound | 205 a probe | 3.0 a probe |
| copy | `binding($s, $x, $b, _)` | scanned first | 3,288 rows | 2,870 a part |
| copy | `$x != 8173`, `$x != 8176` | a name bound | 0.5 each | 1.0 each |
| copy | `look($n, $x, $s)` | name and scope bound | 0.15 a probe | 4.1 a probe |
| copy | `load_ref($n, $x)` | name bound | 1 a probe | many (a name is loaded in many places) |

The first two rows are the cause. `binding` and `load_ref` are derived, and their columns' distinct values were
guessed: the least of the body columns that bind each (4 for `binding`'s scope and name, from a column the typing
library fills with few values), or the rows when none does (5,533 names in 5,533 `load_ref` rows). So the stored
reader was priced 14 times high (450 k against 32 k observed) and the copies, ordered by a scan of `binding`, about
100 times low. The old pricing then scaled the copies by the stored reader's observed over estimated (0.07), which
made them 170 times low: inlining "saved" 100 k and cost 3.3 M. The multi-column probe (scope and name divided by
each column's distinct values, as if independent) and the comparison's fixed one half are the planner's estimates
everywhere; inlining only exposes them, because a copy's order is the planner's choice while a stored relation's
reader is ordered by the relation it scans.

*The scheduler.* Four parts, each a guard against one way the estimates miss:

- **Measured columns.** A relation an earlier chunk or part stored keeps its measured distinct values and fanout
  per column in the estimates (`estimate`), not guessed ones. `binding`'s scope and name then carry 1,003 and 734
  distinct values, and fanouts of 21 and 75.
- **Observed selectivities, per step pattern.** Every plan the choice prices (reader, copy, build) is re-priced step
  by step in the native units (`calibrated`): the planner's order and first rows, and for each later step the share
  of its bindings it kept, observed where an earlier chunk or part ran a step of that pattern (in that rule first,
  else in any rule), else the planner's estimate. A pattern is an atom's relation, negation and bound columns
  (constants by value, as the planner's `Stats::obs`), a comparison's operator, constants and bound sides, or a
  `nearest`'s kinds and direction. A copy never ran, but its steps' patterns mostly did, in other rules.
- **The side that ran is believed** (`weigh`). A side is observed when every term of it ran: the readers as they are
  (stored), or the rules unfolded from each reader (inlined; the unfolding records each copy's reader and the
  relations unfolded into it, so a copy's work is found though deeper inlining and its renaming changed its text).
  Both observed: both are priced by what they did. Neither: both by the estimates. One: it is priced by what it did,
  and the other by its estimate times the estimates' error on the side that ran (observed over estimated, or its
  inverse, the larger).
- **The estimates' choice stands unless the observations are evidence against it** (`settle`). Each candidate is
  priced twice, by the estimates alone and with the observations. The observations change the choice in two cases
  only: both sides ran, and the one that did less is taken (an inlining that did worse is taken back); or the
  estimates inline a relation whose stored side ran, and its copies, priced as above and doubted, would cost more
  than the stored side did (a veto). An alternative that never ran is never taken on estimates the estimates alone
  would not take: with 2 chunks it would not run before the last chunk, and a wrong guess costs that chunk. Only the
  relations whose stored side ran are priced the second time.

The profile line says which side was observed and why it chose: `rr_ctx57: stored, readers would add 269299 work >=
1 x (build 55263 + 4 x 1433 rows); stored observed (estimated 118882, did 161291), inlined estimated 276639 x 1.4`,
or `p0_rr_ctx33: stored, …; stored observed (estimated 9590, did 31985), inlined estimated 12786 x 3.3; the estimates
would inline it`. Calibrated, the stored reader is priced within 1.4 times of what it did (it was 14 times off) and
the copies at 277 k (they were 10 k). Tests: `plan::keeps::tests::observed_selectivities_price_a_copy_that_never_ran`
(the regression's shape: inlined by estimates, stored once the stored side ran and the copy's patterns were seen) and
`plan::keeps::tests::the_side_that_ran_is_believed`.

*The choice's own cost.* It estimated every derived relation, planning each of their rules (10.7 k rules on the
pack's program), though it prices only candidates: now it estimates the cone its pricing reads (each candidate, what
its readers read, and upstream of those every relation their rules read), plans each reader once however many of
the relations it reads are priced, takes each build from that estimate, and gathers its refusal tests (outputs,
inputs, demand, answers, recursion) once rather than scanning the program's lists per relation. Its span is
"inline or store".

The integration's statistics kept by delta (0096) are not read yet: they keep rows and distinct values, not step
counts.

**The factorized store across the chunk boundary (built).** The chunks hand the cross-file part each export's
rows; moving them (holding them, attributing them to their files, writing the per-file memo) was 39 s of the
pack's 284 s of CPU. Some exports are a product: `fl853_p4_outer(f, x) :- rr_inside_ks5(f, p), bound_in(p, x)`
(every name bound in every scope a node is inside) held 771 k rows over the fixture files, though its two parts,
joined only at the scope `p` the head drops, hold about 100 k. Such an export is now handed on as its parts, and
the cross-file rules that read it join the parts.

- **What changes.** After the file-local joins are pushed into the chunks, and before their rows are located
  (`file_local::factorize_exports`, called from the corpus run's preparation). For each export whose every rule
  splits at a variable its head projects away into parts that each name a head variable (`keeps::factors`; each
  part keeps the comparisons and negations over its own variables, and a literal that reads two parts keeps them
  one part), each part becomes a file-local relation of its head variables and the key, `fz892(x, p) :-
  bound_in(p, x), …` and `fz894(f, p) :- rr_inside_ks5(f, p), …`. Each is reduced by the others' keys
  (`fzk895(p) :- rr_inside_ks5(f, p), …` read in `fz892`'s rule), so every row stored joins and no part holds more
  rows than the product it replaces. The export's rules go, and each cross-file read of it becomes its parts
  joined at a fresh key: `p4_outer(f, x) :- p4_read(_, x, f, _), fz894(f, k), fz892(x, k)`. The parts' key is a
  node, so they are located (0052) as the product was. If the split does not find a part file-local, that export
  is left whole.
- **When.** Only where the parts can be smaller and the readers stay cheap:
  - at least two parts may hold many rows per key. A part holds one row per key when the key determines its head
    variables through keys of the relations it reads. The keys are inferred from the program (`relation_keys`): a
    node's attributes by the node, the tree's edges by the child and by the parent with the field or index, a
    lattice by its key, and a derived relation by a column that determines the head in each of its rules, found to
    a fixpoint. So `bound(p, x) :- …, field(a, "left", l), text(l, x)` split at the assignment `a` is left whole: its
    second part is one row per key, and splitting it would double the rows. UP003's `p6_primitive(a, t)` is refused
    the same way;
  - every read of the export is positive, in a cross-file rule that binds a column of it (a constant, or a variable
    another positive atom names), so the reader's join starts from a bound part and goes through the key;
  - at most 4 rules and 8 reader copies.

  An inferred key that is wrong costs rows, never findings: the test only decides whether a relation is split.
- **The per-file memo.** The memo keys each file's entry by the plan, a digest of the file-local part as it was
  lowered (`memo::program_digest`: its rules, declarations and outputs, which include the exports). Factorizing
  changes that part (the parts are its exports in place of the product), so a run whose choice differs from the one
  a warm memo was written under has another key: every file misses and is evaluated again. That is a miss, not a
  wrong answer. For the miss to happen only when the choice changes, the choice is made from the program alone,
  before the first chunk, never from a chunk's rows: the program's shape and the keys it shows. The prepared
  program, which is kept across processes under the program text, the engine build and the run's additions
  (decision 0073), now also names the setting: `RERAIL_FACTORIZE=off` (or a run's `factorize: off`) is another
  prepared program, not a stale one. The statistics kept by delta (0096) are kept under the same digest, so they
  follow the choice too. The test `corpus::tests::an_export_handed_on_as_its_parts_finds_the_same_and_a_changed_choice_misses_the_memo`
  runs one program four times over one store, off, off, on, on and back to off: the findings are identical every
  time; each change is a miss for every file and each repeat a hit; the memo's entries hold the two parts, reduced
  (every row joins), and no product, and the parts' join equals the product's rows the off run stored.
  `file_local::tests::an_export_joined_at_a_projected_key_is_handed_on_as_its_parts` checks the rewrite, and that
  a join whose second part is a node's attribute is left whole.
- **The reader.** Nothing new runs in the cross part: the parts are inputs, and the planner orders the reader's
  join. On the 8 programs it joins per key from the bound side: `p4_outer` reads `fz894` by `f` (899 bindings, 912
  rows) and checks `fz892` by `(x, p)` (148 rows), where it checked the product by `(f, x)` (899 to 148).
  `p7_names_class` reads `class_path`'s parts by the bound `p`, through the key, to its 22,659 rows, the same rows
  it read from the product.
- **The profile** names each relation factorized, with its parts, under "factorized store (decision 0099)", and the
  rows each export handed on under "rows handed on to the cross-file part". Relations the analysis still names as
  stored flat but smaller as parts (a finding, a part of one row a key, a reader that binds nothing) are listed
  under "factorized store (decision 0099): stored flat, would shrink as parts".

**Measured: the factorized store.** On the 8 programs (486 fixture files, 2 chunks, default threads), three
paired runs of one binary, `RERAIL_FACTORIZE=off` then on, alternating. Instructions are the process's retired
instructions (steadier than CPU time at this load); CPU is the profile's user + sys. Findings are byte-identical
to the base binary's in every run.

| Setting | Rows handed on | Instructions | Moving rows, CPU s | Local CPU s | Cross CPU s | Peak RSS | Load (1-min) |
|---|---:|---:|---:|---:|---:|---:|---|
| off | 986,114 | 237.3, 232.4 G (and 295.5 G) | 3.90, 4.02 (4.08) | 15.0, 15.3 (15.3) | 3.14, 3.09 (3.17) | 292, 294 MB (800 MB) | 35 to 45 |
| factorized | 306,011 | 208.6, 208.4, 207.9 G | 2.66, 2.62, 2.65 | 13.6, 13.6, 13.8 | 2.63, 2.58, 2.57 | 257, 258, 256 MB | 34 to 40 |

- `fl853_p4_outer`'s 771,152 rows are now 89,605 (`fz894`, `f` inside `p`) and fewer than 11 k (`fz892`). Rows
  handed on fall 69%; moving them falls a third (1.3 s), local CPU 1.5 s (the product is no longer built), cross
  CPU 0.5 s; 11 to 12% fewer instructions, 35 MB less peak RSS. On the slice it factorizes `class_path` and the two
  `p4_outer` joins.
- The run in parentheses is another mode of the engine, seen with and without the change (the base binary's first
  run on the slice too: 329 G instructions, 798 MB): about 60 G more instructions and 500 MB more RSS. It is not
  the factorized store's, and is not explained here.

On the pack (534 programs, the same files; one run each: base binary, factorized, then the same binary with
`RERAIL_FACTORIZE=off`; load 26 to 131, 89 to 97 and 117 to 145): findings byte-identical in all three. 115
exports are handed on factorized, among them `p207_outer`'s two (the pack's `fl853_p4_outer`), `class_path`, and
joins of the typing context (`ann_q`, `head_q`, `type_arg`, `version_cmp`, `typevar_name`, …).

| Pack | Rows handed on | Moving rows, CPU s | Local CPU s | Cross CPU s | All CPU s | Instructions | Peak RSS |
|---|---:|---:|---:|---:|---:|---:|---:|
| base binary | (not counted) | 38.75 | 209.3 | 26.3 | 284.3 | 3,017 G | 1,097 MB |
| off | 6,966,788 | 38.09 | 211.5 | 26.5 | 288.6 | 3,010 G | 1,547 MB |
| factorized | 6,731,499 | 37.26 | 211.9 | 26.1 | 290.4 | 3,078 G | 1,579 MB |

- Of the 39 s spent moving rows, the factorized store takes about 1 s: `p207_outer`'s 771 k rows go, but the pack
  hands on 7 M rows of 1,769 exports, and its other large exports (`fl5237_p514_effect` 197 k, `precedes` 196 k,
  `fl4196_doc_opener` 186 k) are not products of parts.
- Instructions rose 2.3% (68 G) with it on, against falling 12% on the slice. One run each, and the slice shows a
  mode worth 25% of its instructions, so this is not settled; the factorizing itself costs 1.4 s CPU of the run's
  preparation (inferring keys and two splits over 13 k rules), which a prepared program kept in the store (0073)
  pays once, and every run here started from an empty store.
- Peak RSS is that of the setting's run, not the setting's: off and on peaked alike (1,547 and 1,579 MB); the base
  run, at lower load, at 1,097 MB.

**Measured.** 8 programs that read `bound_in` (slice8), and PYI055 + UP035 (which include the typing context), on
the 486 fixture files, in 2 chunks (the default) and in 6 (`--chunk 20000`), estimates only
(`RERAIL_OBSERVED_STEPS=off`, a comparison switch) against observations (the default). Findings are byte-identical
to the base binary's in all eight runs. Native work is `native.tsv`'s summed work; CPU is the profile's user +
system over all spans; instructions and cycles are the process's (retired, from the run's report). The runs were
one at a time, off then observed, with the 1-minute load at start and end as shown: 26 to 144, so CPU differences
under about 10% are noise; instructions are steadier.

| Slice | Chunks | Setting | Native work | CPU s | G instructions | G cycles | Choose s | Load |
|---|---:|---|---:|---:|---:|---:|---:|---|
| slice8 | 2 | estimates | 7,073,676 | 23.0 | 232.09 | 72.7 | 0.52 | 88 to 141 |
| | | observed | 7,064,858 | 22.0 | 231.90 | 70.8 | 0.49 | 141 to 109 |
| PYI055 + UP035 | 2 | estimates | 25,959,388 | 56.4 | 527.17 | 175.1 | 1.97 | 109 to 144 |
| | | observed | 25,951,656 | 56.0 | 527.72 | 174.1 | 1.99 | 144 to 134 |
| slice8 | 6 | estimates | 6,960,993 | 23.0 | 239.35 | 77.4 | 1.47 | 134 to 118 |
| | | observed | 6,954,967 | 20.5 | 240.23 | 74.4 | 1.36 | 118 to 95 |
| PYI055 + UP035 | 6 | estimates | 25,705,589 | 51.4 | 590.31 | 184.3 | 4.81 | 95 to 46 |
| | | observed | 25,705,470 | 49.5 | 592.46 | 182.6 | 4.64 | 46 to 26 |

- Observed is never worse in native work: it does less in all four (by up to 0.12%), and the regression is gone
  (29.4 M on the pair before the fix, 25.95 M now; estimates 25.96 M). The gains are small because the observations
  change few choices: 2 vetoes on slice8 (`p0_rr_ctx33`, `rr_in_annotation_0_asg`), 6 on the pair (`exists_11` and
  `rr_in_annotation_0_asg` in each part), 3 with 6 chunks on the pair; with 6 chunks on slice8 both sides of
  `rr_in_annotation_0_target_form` ran and the inlining was confirmed (no revert anywhere).
- Instructions are within 0.4% either way (observed 0.1% lower on slice8 in 2 chunks, 0.1 to 0.4% higher on the
  others); cycles and CPU are lower in all four. Run-to-run spread of the instruction count on this loaded machine
  was about 2% between batches (spin waits), so neither side is measurably ahead on instructions.
- The choice's own cost (span "inline or store", summed over its calls): with observations it was 0.93 s on slice8
  and 3.63 s on the pair before the cone, the overlay and the narrowed second pass; it is now 0.49 s and 1.99 s,
  level with estimates only. The equality fix (above) is unchanged by any of this.
- The whole pack (536 programs, 2 chunks), estimates then observed, findings byte-identical between the two:
  native work 114.13 M against 113.12 M (-0.9%); 2,658.3 against 2,654.5 G instructions (-0.1%); 803.6 against
  761.1 G cycles; CPU 222.8 against 192.9 s, with the load falling from 80 to 10 over the second run, so most of
  the CPU difference is load. The observations vetoed 15 inlinings and confirmed 13 where both sides ran; none
  were reverted. The choice took 8.9 s against 8.5 s of the 10.9 and 10.4 s planning spans ("demand"); its cost
  before the cone was not measured on the pack.

**Next.**

- Let an alternative that never ran win when the evidence for it is strong, not only veto or revert: today an
  inlining the estimates would not choose is never tried, however the observations price it. A bounded trial (one
  part inlined, kept if it did less) would find those; it needs a part small enough that a wrong trial costs little.
- Use observed selectivities in the planner's own ordering, not only in the choice's pricing: a copy's order is still
  the planner's from estimates, so a copy can be priced right and still run in a poor order.
- Make the factorized store pay on the pack: its largest exports are not products of parts. The next step is the
  size of each export, not its shape (projection, demand from the cross part).
- Read the observations across runs, from the kept statistics (0096), not only across a run's chunks.
- Have the converter write `inline` on the helpers it makes.
