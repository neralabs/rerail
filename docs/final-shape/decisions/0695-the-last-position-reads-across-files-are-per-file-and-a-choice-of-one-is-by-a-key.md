# 0695. The last position reads across files are per-file, and a choice of one is by a key

Status: decided (2026-11-25), by the user's plan for the rules the position report flagged after 0665, step 4 ("E and
F, read against the originals") and step 5 ("recount, push down what's left, and make item 5 an
error"). It follows 0693, which left 16 flagged rules, and amends demand's magic rules (0145), the lowering of a
negated conjunction (PR #508's contexts), staging's record of what it refused (0675), and makes 0652 item 5 an error.

## Intro

After 0693, the position report (0649) flagged 16 rules in six programs. Each one was read against its original
(the linter or checker it ports) and fixed where the port went wrong. Three kinds came up:

- **A choice of one among several, made by position, across files.** cached-pure-lazy-allowlist kept the last
  entry of a name by `visit_index` over every file. The three Django practices took the first write, the first
  call or the first bare call by the lowest line in any file. The originals choose by an order that an edit
  elsewhere doesn't move: the linter's dict keeps the last entry it reads, and the Django checkers walk a def's
  body in order.
- **A per-file analysis held across files** by a relation declared `cross_file` (eager-resolve, lazy-architecture)
  or by a location read back from `locate` (versioned-merged-identity). The originals decide those per file and
  only compare across files.
- **Two engine gaps** that made a relation of names cross files when it didn't have to. They are §3 and §4.

## Decision

### 1. A choice of one across files is by a key, not a position

The calls library gives each node an order key that an edit to another def doesn't move:

```
order_key(n, q, i) :- nearest(n, h, fdef), qualname(h, q), visit_index(h, a), visit_index(n, b), i = b - a.
order_key(n, "", i) :- kind(n, _), !nearest(n, _, fdef), visit_index(n, i).
```

It is the qualname of the def that holds n, plus n's offset in that def. A rule that picks the first of several
nodes in different files takes the least qualname, then the least offset. The Django practices do this:
- get-handler-writes uses `ghw_first_write`, `ghw_first` and `ghw_first_new`;
- sync-orm-reached-from-async uses `sora_first` and `sora_first_new`;
- select-for-update-outside-atomic uses `sfu_first_bare`.

These replace `min` over line numbers. The order inside one def is still the def's, which is what the checkers
read.

cached-pure-lazy-allowlist's "last entry of a name" is per kind of entry:
- an entry made by a call is later than another in its own file (`earlier`);
- an `opt:` entry is later by its index in the options list;
- an item is later in its own document, and then by its path.

The linter keeps the last one its dict reads, in its file order, which is the path's order.

### 2. A per-file analysis is per-file; only its comparison crosses

- **eager-resolve:** `er_facade_site` is no longer declared. It reads one facade module's records and its own
  file's syms, so it is per-file. `er_mentions` and `er_ann` are typed by node, so the split can place them.
- **lazy-architecture:** `la_eager_delegate` reads one file's facts. It is undeclared and per-file. Its findings go
  to `la_hit`, which is placed by the document's id, not read back from `locate`. Only the budget's count over every
  file, `la_over`, is declared `cross_file`. That count is what the original compares across files.
- **versioned-merged-identity:** a finding carries its own line and document (`vmi_found`). The report rule reads
  the relocated path only by name.

### 3. A negated conjunction's `x = t` binds x

`!(doc_sphinx(s, c), k = 1)` lowered to `rr_not<k>` plus a context `rr_ctx<k>(k)`, because the lowering didn't
count `k = 1` as binding k. A context of k alone holds no location. Once the atoms beside it are per-file, it joins
documents, so the split refused to stage pack-python's resolution beside D412 ("would join documents undeclared").
Now `x = t` with t ground binds x inside a negated conjunction, as it does in a rule's body.

### 4. A magic rule for a node's demand reads what is tied to the node

A relation demanded on node columns (`unparen_bf`) is asked for nodes, and a node's demand is its own document's.
The magic rule wrote every literal before the read, in sideways order.

For example, default-called-once's `magic_unparen_bf(v)` reached v through a class name:
`xd_sym_of(e, q), dco_w161_whole(t, q)`. That joined every call's callee name to every field's class, a crossing
the split refused. Its rule test failed, before this work too.

A magic rule for such a relation now keeps only:
- the literals tied to its arguments through node columns or a document's id (`docof(n, d), modnode(d, m)`);
- atoms that only test those variables (`seqkind(kl)`), or test them and bind nothing anything else reads
  (`rifrom(s, x, m, y)` for x);
- filters over what those literals hold.

Leaving a literal out only widens the demand, so the rows are the program's. When the tied literals don't bind
the arguments, nothing changes. T's goldens (`magic::tests`) are unchanged.

The same holds for an aggregate's demand (`rr_demand_agg_…`). When every key is a place (a node, `loc` or `span`),
its guards join through places alone. public-api-triples' demand of a class's Protocol count kept
`pub_member(cq, n, f)`, a base's method in another file, joined by the class's name.

### 5. Staging records what it released

When staging releases a relation because the split makes it per-file once the others are staged, the relation is
recorded as refused, with that reason. The refusals are logged to the profile (`staging`) before the early return,
so a run that stages nothing still says why.

### 6. A position read across files is an error (0652 item 5)

With no pack or data-layer policy reading a position across files, 0652 item 5 holds. A program whose cross-file
part has a rule that reads `visit_index`, `node_order`, `line`, `col`, `end_line` or `ast_depth` is refused in
preparation. The error names each rule and the relation it reads, and says how to fix it: compare positions in the
per-file part, or choose by a key that an edit elsewhere doesn't move (`order_key`, §1). As before, an output's rule
places its finding and is exempt. `RERAIL_STRICT_POSITIONS` is gone, along with its place in the preparing
switches. The split's test is now `a_cross_file_rule_reading_a_position_is_refused` (0649's `…_is_reported`).

### 7. What the error found

Making the check an error found two programs the report had not covered:

- **A pack of data-layer policies.** The data-layer split test runs its policies in packs. With cached-pure-lazy-allowlist,
  plan-scope-side-effects, post-op-no-internal-dispatch and public-api-triples together, post-op's count of earlier
  comparisons (`p2_rr_agg3`) read `visit_index` across files. Its `enum_ref` reads `sym_of`. Staging didn't stage
  the name resolution, because public-api-triples' `root` joined two documents undeclared: a class's root can be a
  base's method in another file. That crossing is meant, so `root` is declared `cross_file`. Its count's demand
  joined the same two files, which §4 fixes. Each policy alone was clean, so the report, which runs them alone,
  never saw it. A pack run's error now names the pack's policies.
- **The maintenance test R4's program.** Its reader `first` was a `min` over nodes, which compares `node_order`
  across files. It is now a `min` over a number from each row's name, still maintained over the re-derived
  stratum's delta.

## Tests, written first, one per requirement

| Requirement | Test |
|---|---|
| cached-pure-lazy-allowlist compares the last entry of a name as the linter does, and its cross-file part reads no position | `pack_split_tests::cached_pure_lazy_compares_the_last_of_a_name_and_reads_no_position_across_files` |
| eager-resolve finds what its linter finds, and reads no position across files | `pack_split_tests::eager_resolve_finds_what_its_linter_finds_and_reads_no_position_across_files` |
| lazy-architecture finds a delegate in another file and the budget over every file, and reads no position across files | `pack_split_tests::lazy_architecture_finds_a_delegate_in_another_file_and_reads_no_position_across_files` |
| versioned-merged-identity reads no position across files | `pack_split_tests::versioned_merged_identity_finds_what_its_checker_finds` (now strict) |
| A pack's resolution is staged beside a negated conjunction with `k = 1` | `pack_split_tests::a_packs_resolution_is_staged_beside_a_negated_conjunction` |
| The Django practices name one of several by a key, and an edit elsewhere doesn't change which | `pack_split_tests::the_django_practices_name_one_of_several_by_a_key_edits_elsewhere_do_not_move` |
| A node's magic rule reads no value join to another document | `django-practice-default-called-once.rerail::cases` (`test-rr`) |
| The magic transformation is T's on T's programs | `magic::tests::t_programs_transform_like_t` |
| A pack of data-layer policies stages its name resolution and reads no position across files | `pack_split_tests::the_data_layer_is_split_with_its_crossings_declared` |
| A maintained `min` over a stratum derived again past θ | `maintain_tests::r4_a_stratum_derived_again_past_theta_feeds_maintained_readers` |
| A cross-file rule reading a position refuses the program and names it; an output's rule and a per-file rule do not | `file_local::tests::a_cross_file_rule_reading_a_position_is_refused` |

## Results

- The position report covered every data-layer policy over the 334-file corpus and every pack over a small corpus,
  66 programs, on one Linux host. Every run completed. 16 rules were flagged after 0693, and none now. No log names
  an error or a refusal.
- On one Linux host: every rule file's own tests (201, `test-rr` over the packs, the libraries and the data layer),
  and `rerail-logic`, `rerail-typing`, `rerail-rr`, `rerail-rr-root`, `rerail-corpus-prep`, `rerail-boot` (every
  pack split test, each pack prepared under the error), `tests_logic`, `tests_corpus`, `tests_rr`, `tests_lang`,
  `tests_findings` and the device crate: 1,618 of 1,620 pass. The device crate's two GPU tests fail on a Linux host,
  as in 0686.
- default-called-once's rule test, which failed before this work, passes. T's magic goldens are unchanged.
- `gate/executor-coverage/packs.tsv`: the data layer has one fewer `max` bounded by a comparison, 8 to 7
  (cached-pure-lazy-allowlist's `pe_ord`, §1).

## Left open

- Points-to's `call_target(c, g)` and the Django walks (`mro_method`, `dj_instance_value`, `sfu_atomic_def`,
  `ghw_model_ref`) still carry nodes across files here, declared `cross_file`. 0677, on the sync-integration-hub-tape
  line, decides that no node id crosses, and has re-keyed these by symbol there. This line has none of that work
  since it split at 94e9aafd78.
- §4 and 0677's tail 3 fix the same kind of magic-rule crossing, by two functions: here, a node's demand keeps only
  the literals tied to its key; there, a literal reached only through a dropped one is dropped
  (`drop_other_documents`, RUF034). At the merge they become one function, the key's places reached through the
  kept literals, covering both cases. §3's lowering fix and 0677's rewrite of D412 as a disjunction can both stay.
