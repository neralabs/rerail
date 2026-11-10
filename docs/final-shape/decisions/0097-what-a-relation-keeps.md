# 0097: What a relation keeps: recompute in the reader, restrict by demand, or store, decided per relation

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
  Every choice is a profile line under "inline or store (decision 0097)", with the numbers.
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

**Observed step counts (item 1, "the estimates are the plan's on the first run").** The native strata now record,
per rule (its head and body), the rows into and out of every step of every probe of it, and per relation its rows
when its stratum ends (`keeps::observe`); the record is kept on the thread for the corpus run, beside its plans.
The next chunk or part prices from it, per node of the corpus: a relation's rows and its build when all its rules
ran; each reader term (the reader as it is, each copy) when that rule ran. A term not observed is its estimate
scaled by observed over estimated for that reader's observed terms: unscaled, a stored reader's observed work (all
its waves) set against its copies' first estimates read as savings that were not there. A copy's variables are now
named apart by the unfolding pass, so a first-level copy has the text priced and its observation is found.
It is off by default until its regression is fixed (`RERAIL_OBSERVED_STEPS=on` turns it on). The integration's statistics kept by delta (0096) are not
read yet: they keep rows and distinct values, not step counts.

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
- **The profile** names each relation factorized, with its parts, under "factorized store (decision 0097)", and the
  rows each export handed on under "rows handed on to the cross-file part". Relations the analysis still names as
  stored flat but smaller as parts (a finding, a part of one row a key, a reader that binds nothing) are listed
  under "factorized store (decision 0097): stored flat, would shrink as parts".

**Measured: the factorized store.** On the 8 programs (486 fixture files, 2 chunks, default threads). CPU is the
profile's user + sys. Two runs of each, against the base binary and the same binary with `RERAIL_FACTORIZE=off`.
Findings are byte-identical in every run.

| Setting | Rows handed on | Moving rows, CPU s | Local CPU s | Cross CPU s | Peak RSS | Load (1-min) |
|---|---:|---:|---:|---:|---:|---|
| base binary | (not counted) | 4.28, 4.45 | 17.4, 17.6 | 3.14, 3.21 | 308, 314 MB | 24 to 50 |
| `RERAIL_FACTORIZE=off` | 986,114 | 4.51, 4.61 | 18.3, 18.5 | 3.84, 3.23 | 308, 307 MB | 35 to 37 |
| factorized | 307,743, 306,011 | 2.73, 3.08 | 14.5, 16.6 | 2.52, 2.73 | 278 MB (and one 801 MB) | 28 to 99 |

- `fl853_p4_outer`'s 771,152 rows are now 89,605 (`fz894`, `f` inside `p`) and fewer than 11 k (`fz892`). Rows
  handed on fall 69%; moving them falls about a third (1.4 to 1.6 s); local CPU falls 1 to 4 s (the product is no
  longer built), cross CPU 0.5 s. On the slice it factorizes `class_path` and the two `p4_outer` joins.
- Peak RSS varies between runs far more than with the change: the base binary also peaked at 798 MB once on the
  same slice, so the single 801 MB run is that variation, not the change.

On the pack (534 programs, the same files, one run each, base then factorized, load 26 to 131 and 89 to 97):
findings byte-identical. 115 exports are handed on factorized, among them `p207_outer`'s two (the pack's
`fl853_p4_outer`), `class_path`, and joins of the typing context (`ann_q`, `head_q`, `type_arg`, `version_cmp`,
`typevar_name`, …). Moving rows to the cross-file part went from 38.75 to 37.26 s, about 4%, within the noise:
the pack hands on 6.7 M rows of 1,843 exports, and its largest exports (`fl5237_p514_effect` 197 k, `precedes`
196 k, `fl4196_doc_opener` 186 k) are not products of parts. In all, 284.3 s against 290.4 s of CPU (local 209.3
against 211.9 s, cross 26.3 against 26.1 s), within the noise of that load; the factorizing itself costs 1.4 s of
the run's preparation, which the prepared program kept in the store (0073) pays once. Peak RSS was 1,097 MB against
1,579 MB (see below).
**Measured.** 8 programs that read `bound_in`, and PYI055 + UP035 (which include the typing context), on the 486
fixture files in 2 chunks. Each pair's findings are byte-identical in all four settings. CPU is the profile's
user + sys of the run; native work is `native.tsv`'s summed work. Load (1-minute) was 30 to 115 throughout, so
differences under about 15% are noise.

| Slice | Setting | CPU s | Native work | Load |
|---|---|---:|---:|---:|
| 8 programs | both off | 19.5 | 7.10 M | 48 |
| | equality only | 21.3 | 7.09 M | 30 |
| | observed only | 19.8 | 7.26 M | 38 |
| | both on | 18.1 | 7.25 M | 63 |
| PYI055 + UP035 | both off | 66.1 | 38.4 M | 77 |
| | equality only | 70.3 | 33.9 M | 62 |
| | observed only | 76.4 | 41.8 M | 84 |
| | both on | 78.7 | 37.4 M | 86 |

- The equality fix takes 4.4 M of work (11%) off the typing slice and changes nothing else that is measurable.
- Observed counts **add** work on the typing slice (+3.4 to +3.5 M). The cause is one relation: `p0_rr_ctx57`,
  estimated under 1,024 rows (stored without pricing), observed at 1,433 a chunk, is then priced, and its copies'
  estimates say inlining saves work; inlined, its reader `p0_rr_agg56` did 3.3 M where it did 0.2 M. The copies are
  unobserved until they run, so with 2 chunks the misestimate is never corrected. Fixing that is the next step.

**Next.**

- Correct the inline side's optimism before the copies run: carry the observed fanouts (`Stats::obs`) across chunks
  so the copies' atoms are priced by the binding patterns they read, or keep the observed choice unless the
  alternative is estimated to win by a margin. With more than 2 chunks the record already corrects it (the third
  chunk sees both sides).
- Make the factorized store pay on the pack: its largest exports are not products of parts. The next step is the
  size of each export, not its shape (projection, demand from the cross part).
- Read the observations across runs, from the kept statistics (0096), not only across a run's chunks.
- Have the converter write `inline` on the helpers it makes.
