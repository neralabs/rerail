# t45. Where the matchers' work goes after 0390, and a plan

2026-11-18. internal_dead_code and public_api_triples at s1000 (339 files, 877,007 nodes). Metal runs are pinned
(`RERAIL_LANE_PIN=gpu`). Load was 30–50 throughout, so times are for comparing within this note only.

## Intro

After 0390 the longest label walk is 63 and 78 steps, and the next gains looked like the total of the walks'
steps. This note measures where those steps go, and what the matchers cost the run besides them. The steps turn
out to be the smaller part. Two other costs are larger, and each has a plain fix:
- the host confirming captures at roots that a join then drops;
- pattern passes for bare `$X` patterns.

Two switches made the measurements. Both are off unless set, and both are kept:
- `RERAIL_LABEL_CENSUS=<file>`: a line per label pass on the host. It gives the route (formula, packed walk,
  fallback), the label's kind and domain, its starts and members summed over the tapes, its walk's words (in
  the device table's notation, `compile_micro::describe`), a sketch of the rule it came from, and its wall.
- The profile's decision "ast captures by W at each root": per relation, its roots, its rows, its captures and the
  pool's CPU.

## 1. The device's walk steps

internal_dead_code's 47.1 M steps on Metal, by what the pass is:

| What | Steps | Share |
|---|---|---|
| A clause's roots that copy a label (`class=T capture emit`), walked from every node | 12.3 M | 26% |
| A pass launched at every node whose real starts are far fewer | 11.1 M | 24% |
| The rest | 23.7 M | 50% |

Both of the first two come from one rule in `matcher_metal.rs`: "a label domain's roots are every node". A clause
whose matcher is one `inside` reach has the program `SUPERTYPE T; CAPTURE; EMIT`. It is walked from all 877,007
nodes, 1–2.7 steps each. There are eight such clauses, for `mn_function`, `mn_dec_fn` and their kin. A targets
pass whose starts are a label of 7,270 nodes was launched at 877,009.

The largest single passes, as rules. The kinds are Python's: 108 module, 146 parameters, 158 argument_list.

| Pass | Steps | Rule | Starts now | Starts seeded from the neighbor relation | Kept |
|---|---|---|---|---|---|
| P1 | 8.5 M | top-level statements that are not definitions: `inside module` (neighbor), `not any[class, function, decorated]` | 877,007 | 10,274, the module's children | 7,270 |
| P2+P3 | 8.7 M | `raw_parameter`: `b5-common-1`, no splats, `not precedes positional_separator`, `inside parameters` (neighbor) | 300,244 | 15,362 | 14,717 |
| P4 | 4.0 M | `raw_base`: a base inside `argument_list` inside `class_definition`, both neighbor | 335,267 | 913, two levels | 913 |

The seeded starts were counted by probe relations, each a rule of the neighbor conjunct alone. In P2 that
selective conjunct is tested last, after about 12 steps of child checks. A walk ends at its first failing test,
so the order of the conjuncts sets the steps per start.

The steps' time: `label_micro_8` + `label_micro` took 57 ms of GPU time per run of internal_dead_code. The
matcher waves' GPU time is 62 ms in all, and the host waits 500 ms of wall on them. Their host stages (encode,
tapes, residency, sizes) are about 180 ms of CPU. Removing half the steps would save about 25 ms of GPU time,
which overlaps the host's work. It would barely move the run's wall (3 s).

## 2. Captures confirmed at roots the join drops

On the host, W's matcher runs again at every root of a matcher with captures, to bind them
(`ast_wave.rs` `captured_rows`). internal_dead_code on Metal:

| Relation | Rule | Roots | Rows | Host CPU |
|---|---|---|---|---|
| `rr_ast_14` | `mn_function(w, f) :- raw_module_name(w, d, _), ast(d, w, {inside function $FUNCTION, stopBy: function})` | 737,806 | 678,606 | 1,193 ms |
| `rr_ast_12` | `mn_dec_fn(w, f) :- raw_module_name(w, d, _), ast(d, w, {inside decorated_definition having function $DEC_FN, end})` | 236,434 | 218,178 | 442 ms |
| the other six | | 26 K | 26 K | 92 ms |

`raw_module_name` has 42,791 rows. The join keeps 27,849 of `mn_function`'s nodes (3.8%) and 10,917 of
`mn_dec_fn`'s (4.6%). Demand reaches the matcher by document only (`waves/policies.rs` `demand_program`): a
relation the batch derives is "unavailable", so its magic rule is widened to the document. But every
`raw_module_name` node is a root of another matcher of the same wave (`rr_ast_11`, the identifier rule), and its
roots are known before any capture is confirmed. Natively the captures are 1.99 s of CPU. public_api_triples'
captures are 33 ms.

## 3. Bare `$X` patterns are passes

The rules name a capture with `{"pattern": "$X"}`. Such a pattern holds at every named node of its kinds. It is
lowered as a pattern label and evaluated as a pattern pass on the host (the device's pattern kernel on Metal):

| Program (native) | Bare `$X` pattern labels | Their wall | All label passes' wall |
|---|---|---|---|
| internal_dead_code | 34 | 224 ms | 297 ms |
| public_api_triples | 27 | 581 ms | 1,241 ms |

A pattern label over all nodes kept 808,425 of 877,007 (92%; the named nodes, presumably, not yet checked). On the device each is a `class=` step in
the walks that read it as well.

## 4. The rest of the run

internal_dead_code on Metal, 9.1 s CPU in all:

| Work | CPU |
|---|---|
| The matchers: captures | 1.95 s |
| The matchers: the device wave's host part | 0.1 s |
| The matchers: device kernels | 0.06 s GPU |
| Parsing | 1.1 s |
| W's reference graph | 0.6–0.8 s |
| `local`, own CPU with no inner span to name it | 0.92 s (1.83 s natively) |

That last row has no span inside it, so where it goes is not yet known.

## The plan, by payoff

1. **Captures only at demanded roots.** Within a wave, a matcher whose node column a rule joins with a relation
   derived from another matcher's roots confirms only the intersection of the two root sets. That is the magic
   rule kept at node grain where its body is the same wave's matchers. The same set can be the clause's label
   domain on the device. Expected: internal_dead_code's 1.6–2.0 s of capture CPU falls to about 0.1 s, in
   both modes.
2. **A bare `$X` is a node test.** A single-metavariable pattern lowers to the named-node flag, within its
   domain. It is a formula, not a pass, so a walk needs no step for it. Expected: most of the host's pattern
   wall (224 and 581 ms) goes.
3. **A clause whose program copies a label is a word operation:** roots = the label ∩ the domain. Expected:
   internal_dead_code's 12.3 M device steps, 26%.
4. **A label-domain pass starts at the label's members** on the device, filtering its groups by the label's bits
   as it does by kind. Expected: the 2.66 M-step targets pass and the like go to their real starts. Item 5
   needs this.
5. **Starts seeded from a neighbor relation:** `inside X by neighbor` starts at X's children, `has X by neighbor`
   at X's parents. P1, P2+P3 and P4 total 21 M steps, and their starts shrink 20–370×. A smaller step that
   needs no new domain: order an `all`'s conjuncts so the cheapest selective test comes first (P2+P3 from
   about 14.5 steps a start to about 3).

Items 1 and 2 are where the run's time is. Items 3–5 cut the device's steps by roughly three quarters, which is
worth about 40 ms of GPU time a run here. They matter more as the rules grow.
Separately, `local`'s 0.9–1.8 s of unnamed CPU wants spans before anything is decided about it.
