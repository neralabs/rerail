# 0122: Planning costs no more than running, and a first wave driven by changes

Status: decided and built (2026-11-12): §1 (`plan.rs`, `SEARCH_WORK`, `plan_rule_reused`, `drive`) and §2
(`plan.rs`, `reads_change`; `native/setfix.rs`, `Line::KeptOld`), both on by default. §4 is taken up by 0123.

## Intro

**What a maintained component is.** In a session, a cross-file stratum whose inputs changed is maintained: its
relations start from the rows the last run kept, and rules over the inputs' changes add and remove rows. The
engine evaluates a component in waves. Each wave, every rule runs once per atom of the component it reads,
with that atom reading only the rows new since the last wave (its *delta*). Each such run is a *probe*, and
each probe has its own plan, an order of the rule's atoms.

**The two problems.** On the O125 replay (an editing session over a real repository):

- **The first wave read everything.** A relation that starts from kept rows made all of them the first wave's
  delta. So each rule ran once over whole relations. The rows it derived were already kept, and the `!h__old`
  test threw them away. At step 06 the first waves read 18.7 M delta rows for about 20 thousand rows of real
  change.
- **Plans cost more than the joins.** Most probes read a handful of rows, but each was searched for an order,
  re-priced, and searched again whenever an estimate moved. At step 07 the planner spent 3.4 s against 3.1 s
  of joins. Of 9,796 plans, 6,071 joined fewer than 10 rows.

## 1. The budget: a search is made only where running would cost more

A search for an order costs about as much as 250 rows of join work. At step 07, a search took 0.16 ms and a
row of work 0.74 µs; a re-price took 32 µs, about 43 rows. So:

- **A kept order is taken as it is when its rule did little when it last ran.** Each line of the engine (a seed
  or a probe) records the work it did on its kept plan (`Kept`'s sixth field). At the next run, the plan is
  neither searched nor priced if that work, scaled by the growth of what the rule reads, is under
  `SEARCH_WORK`:

  ```text
  max(ran, 1) × Π over the atoms (rows now / rows then, at least 1)  <  250
  ```

  The product bounds how far a fixed order's work can grow. A line that did nothing counts as one row, so an
  order kept from an empty input is searched once its input fills (an order kept from 0 rows ran away when
  5,000 arrived; this is the guard).
- **A probe is searched again in a run only once its work has paid for it.** A probe whose statistics moved is
  replanned only when its work so far, plus its next delta, is at least 250 rows for each search made for it in
  this run. Until then its plan stays marked to be replanned.
- **A whole plan is made only where its delta is worth one.** A whole plan reads a relation's first delta in
  full. It is not made at open when the relation is expected to start with fewer than 250 rows. It is not
  replanned between chunks unless that delta has at least 250 rows.
- **A line that did nothing observes nothing.** Its fanouts are not recorded and its plan is not marked.

`RERAIL_PLAN_BUDGET=off` plans as before (a measurement aid). `RERAIL_PLAN_CACHE_COUNTS` prints the kept
plans taken under the budget (`kept: ran below a search`) and why the others were not.

Measured (O125 replay steps 06–08, profiled, planning without the profile's explain records):

| step | planning, before | planning, budget | joins | work, before → budget |
|---|---|---|---|---|
| 06 | 2.53 s | 1.55 s | 0.60 s | 0.50 → 0.51 M |
| 07 | 3.42 s | 1.42 s | 3.55 s | 2.98 → 3.04 M |
| 08 | 3.84 s | 1.89 s | 4.94 s | 4.10 → 4.33 M |

The orders taken without a search cost 2–5% more join work, and save 1–2 s of planning a step.

## 2. A component's first wave is driven by its inputs' changes

A relation that starts from its kept rows (`Start::Base`) now holds them as no wave's delta. Its first wave is
driven by the rules that read a lower stratum's change, run once as seeds:

```text
// A maintained reachability: reach starts as its kept rows, less the gone ones.
reach__ins(a, c) :- reach(a, b), dep__plus(b, c), !reach__old(a, c).   // seed: reads a change
reach__ins(a, c) :- reach__ins(a, b), dep(b, c), !reach__old(a, c).    // probe: by reach__ins's delta
```

- **Which rules are seeds** (`reads_change`): a rule that reads a positive atom of a lower stratum named as a
  change (`__plus`, `__minus`, `__ins`, `__del`, `__gone`, `__lost`, `__redo`, `__cand`, `__chk`, `__held`) or
  an aggregate's changed groups (`h__grpR_K`). Its relations of the component must all start from kept rows. A
  rule that reads a relation starting empty derives nothing in the first wave; its probe takes the rows as they
  come.
- **Seeds are priced as probes are.** Their component relations are priced by the kept rows' statistics
  (`prior_stats`). Unpriced, each read of one was a scan of the component, and the search took cross products
  (248 × 1,085 rows to derive nothing).
- **The spec says so** (`keptold`). setfix restamps the kept rows' runs to no wave, so a keyed delta read skips
  them as a range read does.
- `RERAIL_WAVE0=kept` restores the old first wave (a measurement aid, and the tests' oracle).

**Why kept rows were the first wave's delta.** Making them the delta was never argued for, and no test or
decision depends on it. The survivors rule `h :- h__old, !h__del` (decision 0077) ran as a seed, and a seed's rows
are the first wave's delta, so every survivor was one. When the relation was made to start from the kept one in
place (decision 0090 step 4), the stamp was kept to preserve that behaviour. Its comment said only that "each
rule's whole plan reads them once". Earlier fixes went the same way: `h__live` was never made a delta (0090).

**Why it is sound.** A kept row was derived from the old inputs, and the kept rows are closed under the rules
over them. A row that is new this run therefore has a derivation that uses something changed:

- a lower stratum's change (an input's `__plus`, `__minus`, `__gone`; a negated relation's loss; an
  aggregate's changed group): that atom's rule is a seed;
- or a row new in this component: that atom's probe reads it as a delta.

A row over-deleted and derived again from survivors alone is reached by the rederive rules, which read
`__del` (a change). One gap was found and closed while writing the tests below: the insert rule of an aggregate
inside recursion reads `h__grpR_K`, which had no change suffix.

**Tests** (`corpus/session/tests.rs`, `wave0_*`). Each edits so that a new or rederived row has a derivation in
which no atom of the stratum's own relations changed. Each runs in both modes, against an oracle that keeps
nothing (§3), and asserts that the recursive stratum was maintained and that the findings changed:

| test | the derivation only a change finds |
|---|---|
| `wave0_an_aggregate_inside_recursion` | a group's count goes 1 → 2; a kept `reach(a, b)` with an unchanged `dep(b, c)` derives `reach(a, c)` |
| `wave0_a_negation_gain_inside_recursion` | a module stops being blocked; rows past it come from kept rows and `blocked__minus` |
| `wave0_a_row_derived_again_from_survivors` | a diamond loses one path; the rows over-deleted are derived again through the other |
| `wave0_a_compacted_start` | an edit kills over an eighth of `reach`'s rows, so it starts compacted, not in place |

Without the `__grp` seed, the first test fails as it should: the session loses `a.py`'s finding.

## 3. Found while testing: the session tests' oracle was incremental too

- **The session tests had not been compiled since the crate split.** `logic::plan::corpus::session::tests` is
  hosted by the engine (`tools/cratify/test-hosts.tsv`) but was not mounted there, and the logic crate's own test
  target does not build. The tests are moved to `corpus/session/tests.rs` and mounted in the engine
  (`code_rule_dsl/mod.rs`). Two read the rule packs from the logic crate's manifest and are fixed to find them
  from either.
- **A full run is not a run from nothing.** A run with no session keeps its cross-file part in the store's
  frames (K4) and maintains it from them, as a session does. Only the first full run of a test evaluates from
  nothing. The second one in the aggregate test maintained 8 strata. It missed the same row as the session,
  and the test passed. The `wave0_*` oracle runs with `RERAIL_CROSS_K4=off`, `RERAIL_KEPT_PART=off` and
  `RERAIL_WAVE0=kept`, and asserts that it maintained nothing. The older session tests still compare against a
  plain full run.

## 4. Statistics at a session's first edit (taken up by 0123)

At step 06, the first edit of the session, statistics cost 10.85 s against 0.6 s of joins:

- 6.7 s counting the maintained relations' statistics from their postings at the end of the run, so that later
  runs patch them by delta (`kind__sup0`, `docof__sup0`: 0.8 s each);
- 4 s measuring kept relations the first time a stratum plans with them.

Under §1 most plans read no statistics, so this cost could mostly be avoided: steps 07 and 08 spend 0.5 and
0.1 s. Two things stand in the way:

- **Statistics are resolved eagerly.** A stratum resolves the statistics of everything its rules read before
  planning (`Model::resolve`), and `prior_stats` resolves every component relation. They should be resolved only
  when a plan is searched or priced.
- **The engine's own sketches are not read.** Each engine relation keeps per-column sketches as rows are pushed
  (decision 0110). The planner's statistics (`statkeep::RelStats`: exact counts, a linear sketch, per-document
  pairs) are a different form, and are counted again from postings. Making the kept relations' statistics from
  the engine's sketches would remove the count.
