# 0649. Positions stay in the per-file part: the split keeps a document's rules there

Status: decided (2026-11-23), by the user ("capture this, then go ahead"). It changes no maintenance decision:
0328 and 0364 (R1–R10) stand as they are. It fixes two places where the file-local split (0051, 0281) sent a
document's own rules to the cross-file part, and adds a check that would have caught them.

## Intro

**The question (the user).** After a minor edit, how much work does a re-run do, and can the edit's wave be kept
from spreading? Could node ids be kept to one derivation, and is the real question about visit order just "is x
visited before y"?

**What a re-run does.** On the data layer's 334-file history (`conditional-architecture`, the session of steps
00–03, M3), an edit of 3–4 files takes 0.6–1.5 s of CPU against 8–9 s for the first run. Most of that edit work goes to
strata derived again over all 334 files, because the edit's delta into them is past 0090 §4's threshold:

| Stratum (step 02) | Changed rows read | of | Why |
|---|---|---|---|
| `rr_agg46` | 96,360 | 2,581,766 | `visit_index` 96,069 of them |
| `ca_alias_at` | 97,746 | 2,655,594 | the same `visit_index` rows |
| `rr_demand_agg_rr_agg46` | 1,806 | 2,662,477 | priced above a derive (Q352) |

`visit_index(n, i)` is a node's place in the visit order. An edit moves the place of every node after it, so most
of an edited file's rows change.

## What was ruled out, and why

- **A third maintenance path, "derive again for the changed documents".** That is 0328's *partitions derived
  again*, measured and not chosen on 2026-11-17 (0364 §7), and it breaks R9 (skip, mode, or derive again past θ;
  nothing else).
- **`earlier(x, y)` as a comparison read in place, with no rows.** Unsound under the maintain mode: a removal join
  is evaluated over the new tape, so a row whose order flipped is never found and never removed. Order is an input,
  and something has to carry its change.
- **Order keys that an edit leaves alone.** 0121 made `node_order` a key by granule run. Commit 03563ebd65 does
  the same for `visit_index`, in the visit order. On this history it changes nothing (96,303 → 96,069 rows on step
  02; 55,020 on step 03): every edit here adds or removes a granule (a module-level statement is keyed by its
  text), which moves the ranks after it. Keys carried from the last version are ruled out by 0121 §3 (the memo is
  by content).
- **Tape-order node ids.** With them, the edit deltas into id-keyed relations grow (`child` 1,359 → 62,845 rows
  on step 02), and the edits cost the same CPU, because the strata above are derived again either way. Granule
  ids stay.

## What the split does with this policy

`RERAIL_SPLIT_DUMP` on step 00 shows the cross part reading `visit_index`, `docof`, `node` and `child` whole. Every
such read is `ca_alias_at`'s and its aggregate's (`rr_agg46`, `rr_ctx50/51`, `rr_cover50`,
`rr_demand_agg_rr_agg46`), a rule about one document:

```text
ca_alias_at(at, h, q) :- ca_want(e, at), ca_dotted(e, r), ca_split(r, h, _), docof(at, d),
    j = max v : { ca_alias(i, h, _), docof(i, d), earlier(i, at), visit_index(i, v) },
    ca_alias(i2, h, q), docof(i2, d), visit_index(i2, j).
```

0051 keeps the tree in the chunks: an edited file's chunk is derived again, and only its changed outputs cross.
This rule should be there. The split sends it across for two reasons, one behind the other:

1. **"reads ca_split with no location".** `ca_split(r, h, t) :- ca_dotted(_, r), match("(?<h>[^.]*)(?<t>.*)", r)`
   is a string split written as a relation over every document's names. `InlineFilters` inlines "a computed view
   where its reader reads its domain already", which this is (each reader holds `ca_dotted(e, r)`). It is not
   inlined because:
   - `computed` takes a single rule only with call terms in its head and atoms alone in its body; `ca_split`
     defines `h` and `t` by `@rx_group` comparisons over its domain;
   - a read with a wildcard (`ca_split($r, $h, _)`) takes a relation out of the candidates.

   Inlined by hand (a measurement copy, restored), the findings are the same and the cause moves to the next one.
2. **"joins 2 documents: {a_f1 e} {at d i2}".** `ca_param(f, x, ann)` (from `sig_param`, from `param_parts`) fills
   `ann` with a node (`field(p, "type", a)`) or with `unset(a)`, -1. A column filled by `unset` in some rule is
   not one of the relation's same-document columns, so the split cannot see that `at` and the annotation's `e`
   are of one document. But -1 is no node: a location column that is a node of the row's document, or none, is a
   same-document column all the same. A join through -1 matches no node.

The same run shows what crosses cheaply and what does not. `rr_ctx50` reads `visit_index(i2, _)`, existence only:
127 changed rows. `rr_agg46` reads its values: 96,069. Under granule ids, node ids cross cheaply; positions do not.

## Decision

1. **Maintenance is unchanged.** 0328 and 0364, R1–R10.
2. **`InlineFilters` inlines a computed view whose columns are defined by comparisons over its domain.** That is
   one rule, positive atoms of other relations as its domain, and comparisons that test or define head variables
   from what the domain binds. A read may leave a column as `_`. It is inlined only where the reader reads its
   domain already, as before, so the copy adds the function and nothing else.
3. **A column filled in every rule by a location or by `unset` is a nullable location** (`FileLocal::nullable`).
   A read's nullable column joins its row's document only where a location column of the same rule places its
   variable too: there it is no -1. Two rows joined by nullable columns alone may both be -1, of two documents, so
   such a rule stays cross-file. It is kept apart from `locations` and `same_doc`, not added to them, because a row's
   document is read off those (located rows, retained groups, cone digests), and -1 has none. A nullable column
   carries through a derived relation's rules as a location column does.
4. **Preparation reports position reads in the cross part.** Each read of `visit_index`, `node_order`, `line`,
   `col`, `end_line` or `ast_depth` by a cross-file rule, other than one placing a finding, is reported with the
   relation that reads it. `RERAIL_STRICT_POSITIONS=1` refuses the program instead, as warm mode refuses work.
   Node ids are not limited: under granule ids an edit changes the ids of its own granules only.
5. **Q352 stays open beside this.** Strata such as `rr_ctx50` are derived again on 0.07% deltas because their mode
   lines are priced above a derive (375,662 against 50,027).

## Tests

- `a_view_computed_by_comparisons_is_inlined_where_its_domain_is_read` (tests_logic): inlined into a reader that
  reads its domain, with a wildcard read; not inlined where another reader does not; the same rows either way.
- `a_location_or_unset_column_keeps_a_rule_through_it_on_one_document` (the split's tests): the column is nullable,
  a rule joining through it with a node column stays file-local, and a rule joining two rows by it alone does not.
- `a_cross_file_rule_reading_a_position_is_reported` (the split's tests): the cross rule reading `visit_index` is
  reported, a finding's `line` and a file-local rule's are not, and `positions_checked` refuses under strict.
- `conditional-architecture`'s session under `RERAIL_STRICT_POSITIONS=1` runs, with the same findings (below).

The switch is in `PREPARING`: it decides whether a program is prepared, so a program prepared without it is not
reused under it.

## Measured

Commit 4420df69e9 (steps 2 and 3), the same 334-file session, M3, two runs each, CPU:

| | first run (00) | edit 01 | edit 02 | edit 03 |
|---|---|---|---|---|
| before | 8.0–8.9 s | 0.61–0.66 s | 1.37–1.53 s | 0.93–1.36 s |
| after | 7.20–7.62 s | 0.43–0.44 s | 1.00–1.01 s | 0.71 s |

- `ca_alias_at`, `ca_split`'s readers and the aggregate's helpers are file-local. The split dump's cross part reads
  `docof` (6 reads, placing findings) and no position relation, `node` or `child`.
- After an edit no stratum is derived again. Every cross-file stratum the edits reach is maintained on deltas of
  2–14 rows (`document`, `ca_base`, `ca_conditional`, `ca_valid`, `CONDA005`).
- Findings are identical to the earlier builds' (masked of ids and paths).
- Under `RERAIL_STRICT_POSITIONS=1` the session runs with no report and the same findings. That run's CPU was about
  20% above the pair's on every step, the first run included, which the change does not touch: the M3's load.

Q352 does not arise on this history any more: no mode line is priced against a derive after these edits.
