# 0714. A profile names the rules that make rows they drop, why, and the rewrite

Status: decided (2026-11-26), by the user's ask after 0691's fix 19 was found by reading a profile by hand: "I also
want useful diagnostics that will detect incorrect / inefficient rules like this and provide a good means to fix
them." It extends the profile's structural analysis (`profile/analysis.rs`) and the plan record (`StepPlan`,
`RulePlan`).

## Intro

0691's fix 19 (ported here in the commit before this one) was found by reading a profile. `pt_scope_tm`'s recursive
rules joined each round's new scope terms to every node under the scope, and only then kept the definitions. Over the
PostHog sample's 291 files that was 4,460 rows grown to 560,364 and cut to 4,168, in each of two plans, every round.
The profile's EXPLAIN ANALYZE section had those numbers, in one of about 5,000 plans.

The profile's structural analysis did flag rules like it (`WASTEFUL_RULE`, `JOIN_FANOUT`, `LATE_REJECTION`). But it
read the native joins' plan text and row counts alone. So it named a rule and gave one action for every case: "Read
the rule's body in its .rerail file: a literal that binds nothing the next one reads, a test that could be dropped or
bound first, or a plan made before its relation had rows." It did not say which, nor where the rule was written, nor
what to write instead. And it flagged a fan-out whose rows are all kept (`pt_tm`'s term for every node) as it flagged
one whose rows are thrown away.

The planner records more than that analysis read (`State::plans`): each step's literal, its access, its source (a
stratum's own delta or relation, an input, an earlier stratum), the planner's estimate and its basis, the joins
passed over and their price, and each walk's actual rows and CPU per step. With each step's variables and the head's
added, that is enough to tell why rows were dropped and to write the rewrite.

## Decision

### 1. The record

`StepPlan` holds its literal's variables as it shows them (`$x`), and `RulePlan` the head as shown and the variables
it reads. Both are made where the plan search makes the rest of the record (`plan_search`'s explain record).

### 2. A drop, its source, and the CPU lost

Over each measured plan (`profile/dropped.rs`, a pure function of the recorded plans):

- A **drop** is a step that keeps at most one in 10 of the rows that reach it, with 20,000 rows in or more, summed
  over the run.
- Its **source** is the last earlier step, after the first, whose rows grew: the rows the drop throws away were made
  there. The first step's rows are a scan's (a delta, a seed's relation). A drop with no later source narrows a scan,
  which is not waste: `magic_pt_tm_same_bf`'s delta, 926,766 rows of which `!child(_, n, _)` keeps 582, reads what it
  must.
- A source's rows are one finding, at the first step that drops them. `pt_rbound`'s walk is dropped by its text test,
  then by `pt_tgt`, and joining `pt_tgt` first fixes both.
- The **CPU lost** is the share of rows the drop throws away, times the CPU of the steps from the source to the drop.

A rule's plans add up over strata, chunks and loops, per cause. A rule is reported when it lost 20 ms or more, and
the costliest of its plans is shown, its source and drop marked.

### 3. The causes, and the fix each writes

The first that applies:

1. **`INVARIANT_IN_RECURSION`**: a plan led by its stratum's delta, run more than one loop, whose source and drop
   read nothing of the stratum. Every round joins the same rows again. The fix written out is a helper relation of
   the literals from the source on that read nothing of the stratum, keyed by the variables the rest of the rule and
   the head read, and the rule rewritten to read it. For `pt_scope_tm` that is fix 19's `pt_tm_child(f, s, x)`.
2. **`DEAD_VARIABLE_REJOINED`**: a variable bound before the source that no step from the source on reads, nor the
   head. Each of its values joins the source again. The fix is a helper over the live variables, made from the
   steps before the source. For `pt_type_miss`: `pt_type_miss_n_u(n, u) :- type_of(n, t0), pt_tsub(n, t0, u).`
3. **`MISPREDICTED_ORDER`**: the source made 10 times or more the rows a loop that the planner expected. The finding
   gives the estimate, its basis, and the narrowest join passed over at that step. The fault is the planner's.
4. **`TEST_AFTER_WALK`**: the drop tests a value computed per row (`$x = @f(…)`), which no index can look up. The
   fix names the narrowest join the planner passed over at the source.
5. **`ROWS_DROPPED`**: none of these. Where the dropping join shares a variable with the rows before the source, the
   fix says it could be joined there, with the planner's price for the source and what it made. For `pt_tm_same`:
   `child(p, n, _)` shares `p` with the rows before `node(rr_shown1, n, _, b, e)`, priced at 49,231 rows a loop, which
   made 252,025.

And **`CROSS_PRODUCT`**: a positive join after the first step that shares no variable with the rows before it, and
grows them past 20,000. It is reported whatever its CPU, because it is nearly always the rule's mistake (0692's
`pha_latest`, `visit_index(v, k)` with `v` unbound, was one).

A rewrite is in the plan's own literals, so it names the lowered forms (`rr_nearest_pt_scope(f, s)` for
`nearest(f, s, pt_scope)`), with the split's id bounds (`$n < 6917529027641081856`) left out. The finding names the
head's authored rules by file and line where the run's sources are known (`author_joins::source_name`).

### 4. Where it is read

The findings are in the profile's structural analysis, at the top of `profile.txt`, ranked with the others by CPU,
each with a `Fix:` block; and in `analysis.json`, with a `fix` field.

A head whose plans this analysis read raises no `JOIN_FANOUT`, `LATE_REJECTION` or `WASTEFUL_RULE`: those read the
native joins' row counts alone, and say less of the same rows. So a fan-out whose rows are all kept (`pt_tm`, a term
for every node) is no longer reported.

## Tests, written first, one per requirement

| Requirement | Test (`profile::dropped::tests`) |
|---|---|
| An invariant join inside a recursion is reported with fix 19's helper; fix 19's rule is not | `an_invariant_join_inside_a_recursion_is_made_once_outside_it` |
| A dead variable that multiplies rows is projected away first | `a_dead_variable_that_multiplies_rows_is_projected_away_first` |
| A test after a walk names the narrower join passed over | `a_test_after_a_walk_names_the_narrower_join_passed_over` |
| A misprediction names the estimate and its basis | `a_misprediction_names_the_estimate_and_its_basis` |
| A fan-out whose rows are kept is not reported | `a_fan_out_whose_rows_are_kept_is_not_reported` |
| A cross product is reported whatever its cost | `a_cross_product_is_reported_whatever_its_cost` |
| A drop that could be joined before its source says so | `a_drop_that_could_be_joined_before_its_source_says_so` |
| A scan narrowed by a test is not reported | `a_scan_narrowed_by_a_test_is_not_reported` |
| A head the plans cover raises no count-only finding | `profile::analysis::tests::a_head_the_plans_cover_raises_no_count_only_finding` |

Each test's plan is one this run recorded, its rows and CPU as measured.

## Results

The Django pack's three points_to policies over the PostHog sample's 291 files, with fix 19 in (one run, the CLI's
debug build, 41.8 s user CPU). Before, the analysis listed 17 findings of rules' joins: 12 `JOIN_FANOUT`, 3
`LATE_REJECTION` and 2 `WASTEFUL_RULE`, one generic action for each code, no cause and no rewrite. Now it lists 11,
each with its cause and its fix:

| Finding | Rule | CPU lost |
|---|---|---|
| `TEST_AFTER_WALK` | `pt_rbound` (python/points-to-file:110-113) | 1.80 s |
| `DEAD_VARIABLE_REJOINED` | `pt_type_miss` (python/points-to-solve) | 0.56 s |
| `ROWS_DROPPED` (3 plans) | `pt_tm_same` (python/points-to-file:844-847) | 0.38 s |
| `INVARIANT_IN_RECURSION` (2 plans) | `qualname` (python/scopes:370-371) | 0.09 s |
| `MISPREDICTED_ORDER` | `cls_early` (python/scopes:235-236) | 0.04 s |
| `ROWS_DROPPED` | `bind_only` (python/scopes), `module_of` | 0.05 s |
| `CROSS_PRODUCT` | `res` | 0.6 ms |

`qualname` is fix 19's shape in a second library: each round walks every node under its new scopes (1,980 rows to
225,593) to keep the 29 definitions, and the report writes the helper,
`qualname_f_s_x(f, s, x) :- rr_nearest_ks8(f, s), def_name(f, x).`

`pt_type_miss` is written out as its helper over `(n, u)`. `pt_rbound` names `pt_tgt` (2,050 rows a binding) as
the join passed over. `res` joins `defined(_, q)` with nothing bound, then tests `q = @cat(c, a)`.

## Open

- Two causes have mechanical rewrites (a dead variable, an invariant join in a recursion). The engine could make the
  helper itself; that is the user's to decide, with the report's counts across the packs.
