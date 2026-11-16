# 0174: A node's source attributes are answered on request, by rules

Status: draft (2026-11-13), for review; step 1 built (§7). It applies decision 0145's shape ("Building on it": a relation answered by a
native step) to the per-node relations the projection makes eagerly today, and replaces the Rust demand analysis
of decisions 0109 and 0108 §1 with rules.

## Intro

**The problem.** Some per-node relations hold a node's own source text, or a value read from it:

- `node_text(n, s)`: the node's exact text, at any length;
- `text(n, s)` past its 256-byte inline bound;
- `value(n, v)`: W's value of a node (`facts::core::value_reader`).

The projection makes their rows in Rust, before the waves. Every value is interned as an atom, and atoms are kept in
the store. Rows were made for every node a rule could read, and a node's text holds all of its descendants' text.
So a module's text, each class's in it and each function's in those are interned once each.

Measured over `impl/` of the data-layer repository (3,217 files, 69 MB of source), each rule alone on a fresh store,
strict SQL:

| Rule | Atoms left in the store | Run | Peak memory |
|---|---|---|---|
| `authentication-plan-purity`, every node's `node_text` | 468 MB | 32 s | 1.5 GB |
| the same, `node_text` narrowed by the Rust demand below | 56 MB | 11 s | 0.8 GB |
| `schema-introspection-data-paths`, narrowed | 468 MB, from `value` (every node) | 25 s | 1.2 GB |

Past 256 MB of atoms the next run cannot read them whole before its first wave (0171), and looks them up inside the
waves, which strict mode refuses.

**How it is narrowed today, in Rust.**

- `text_kind_demand` (`facts/vtab/syntax.rs`) walks the rules' bodies. It finds the kinds a `text(v, …)` reads from
  `kind(v, "k")`, T's `node(d, v, "k", …)`, a matcher's root, derived columns to a fixpoint, the fields `v` sits in,
  and (330d9cc2c) the kinds of the node `v` is a child of.
- Text past the bound comes from twin rules reading `@rr_node_text(v)` (0109), with `rr_long_text_node` to seed
  them, and 0167 to keep the call after the joins that narrow it.
- 330d9cc2c gave `node_text` the same analysis.

That is a second, hand-written evaluator of the rules' bodies. It approximates by kind: every decorator child's
text where a rule wants a few. Each new way a rule binds a node needs a new case. `value` has no case yet.

**What changes.** These relations are read through a **request relation**, as 0145 reads the file system's:

- the rewrite derives, from each reader's own literals, the nodes it asks for;
- one rule per relation answers the requests, with a generic native function over the node's document.

No demand analysis in Rust, no twins, no kinds.

## 1. The form

For each per-node source relation, the lowering (`rr::lower`, beside `WORLD`) writes a request relation. A rule
reading the relation with its node bound by other literals asks for those nodes:

```text
// the rule as written
introspection_text_doc(d) :- node(d, m, "module", _, _), node_text(m, s), introspection_trigger(x), contains(x, s).

// what the rewrite adds: the literals that bind m and read nothing node_text gives (world_demand's `context`)
rr_node_text_req(m) :- node(_, m, "module", _, _).

// the answer, one rule for the relation
node_text(n, s) :- rr_node_text_req(n), s = @rr_node_text(n).
```

The reader's rule is unchanged: it reads `node_text`, which is now derived.

**A node nothing binds asks for every node.** `world_demand` asks for nothing where no other literal binds the key.
Here that would be unsound, since `node_text` has rows for every node. The request is then every node:

```text
rr_node_text_req(n) :- node(_, n, _, _, _).
```

That is today's behaviour, for the rules that need it.

The same holds for `value` (`rr_value_req`, `@rr_node_value(n)`) and for `text` past the bound
(`rr_long_text_req`, `@rr_node_text(n)`).

## 2. What backs it in Rust: one generic function per attribute

The answer is a computed value over a request row, `s = @rr_node_text(n)`: evaluated by the engine as every function
is. Behind it is one generic mechanism, not per-relation code:

- **A node's document, by its key.** 0109's registry (`TEXTS`, by document key, the latest batch holding each
  document) already finds a node's document from its id. It is used for every attribute, not only long text.
- **The attribute.** `@rr_node_text(n)` returns the node's text at any length (today it returns only text past the
  bound). `@rr_node_value(n)` calls the registered value reader on the node, and returns nothing where the node has
  no value.

Only requested nodes are read and interned, so the atoms a run leaves are the texts its rules can read.

## 3. Why this is sound, and how it is maintained

- **The answers are right.** This is 0145's magic-sets argument. A request is derived from literals that hold in
  the original program, so every row a reader can join is answered. Asking for too much is sound, asking for too
  little is not, and an unbound key asks for every node.
- **Strata.** A request reads base relations (`node`, `field`, `child`) or derived ones lower than its reader. It
  does not read the attribute it requests, so it stratifies below the reader: requests, then answers, then the
  reader, in ordinary order. A request that reads a derived value makes the answer part of that value's stratum.
  0144 §5 measured that this can cost more than it saves, so such a case is measured before it is kept.
- **Maintenance.** Requests and answers are derived relations of the file-local part. They are memoised and kept
  by the per-file memo (frames, 0146), and an edit's delta maintains them as any derived relation's (0077).

## 4. What goes

Once all three relations are answered on request:

- `text_kind_demand`, `TextDemand`, `TextNodes` and their filters in the projection (`Extra::nodes`, core_rr's
  `text` rows);
- 0109's twin rules and `rr_long_text_node`;
- 330d9cc2c's `node_text` demand;
- for these relations, the reason for 0167's reordering. 0167 stays for other computed values.

`text` within the bound stays eager. It is read for most identifiers in most rules, and one request row per node
would cost more than the row it asks for (§6).

## 5. How it is checked

- **An oracle** (a switch to add): `RERAIL_NODE_SOURCES=eager` makes the three relations as today, for every node. A run with it and
  one without must give the same findings.
- **Rules with findings.** On rules with nonzero findings over the data-layer's `impl/` and over rerail's `.rr`
  files. Today's checks of `node_text` compared 0 rows with 0 rows.
- **The atoms.** The atoms a cold run leaves are counted (`atom_segment`, by length), against the table in the
  Intro.
- Each step runs in seconds on `impl/` or smaller, never the whole repository.

## 6. Open, to measure

- **Request rows.** One row per requested node. A module request is one row a file, but an unbound reader asks for
  every node, which is today's cost plus the requests. Measured on the rules above before `text` within the bound is
  considered.
- **Metal.** A stratum with a computed term is declined by the Metal path today ("computed term: `@rr_node_text` in
  a comparison"). The answer rule runs on the CPU until the function has a kernel, and by the rule that the Metal
  path does no host work, that is listed as remaining host work.
- **Where the requests are derived.** In the file-local part, the requests and answers are per file. A reader in
  the cross-file part that binds a node of another file asks across files. The cross-file part then needs that
  document's batch, which 0109's registry keeps only while a batch holds it.

## 7. Step 1, `node_text`: as built (2026-11-13)

`rr::lower`'s `node_text_on_request`, after the program is lowered. Two departures from §1, both found by running it:

- **Only the literals that read the text are left out.** `world_demand` also drops every literal joined to them, so
  `runtime_node(…) :- runtime_protected(lb, p, x), runtime_gate(d, p), node(d, n, k, _, _), field(n, "name", nm),
  node_text(nm, x)` lost everything that binds `nm`. A request keeps the literals that do not read the text (or
  what is computed from it), are joined to the node, and are no negation or aggregate. Dropping any literal only
  widens a request.
- **A binder that reads `node_text` is read through a widened copy.** In `cached_err(f, …) :- cached_sel(_, f, _),
  node_text(f, t), …`, `cached_sel` reads `node_text` itself. Read as it is, the request is recursive with the texts.
  In `cached-pure-lazy-allowlist` that is not stratifiable (`cached_import_alias` negates `cpl_hasalias` in its own
  component). So the request reads `rr_wide_cached_sel`: `cached_sel`'s rules with the same literals left out, on
  the columns they still bind (a fixpoint over the relations). It holds every row of `cached_sel` and reads no text,
  so requests stratify below every reader. Copies are made only for the relations the requests reach.

`@rr_node_text_any(n)` answers. It reads the node's document through 0109's registry, and also a comment no node is
(a span `Extra` gives `node_text` rows), by its number. A reader that negates `node_text`, reads it inside an
aggregate, or names its node by a constant keeps the relation an input, as `RERAIL_NODE_SOURCES=eager` does. The
profile says which (`node_text on request`). 330d9cc2c's Rust demand for `node_text` is removed; its `text` half stays
until step 3.

Checked against the oracle (`RERAIL_NODE_SOURCES=eager`, every node), each on a cold store, strict SQL:

| Rule, corpus | Findings | Atoms | Peak memory |
|---|---|---|---|
| `serverplan-target-architecture`, `impl/` | 7 = 7 | 467.5 → 54.8 MB | 1.54 → 1.10 GB |
| `authentication-plan-purity`, `impl/` | 0 = 0 | 466.9 → 54.7 MB | 1.74 → 0.91 GB |
| `serverplan-target-architecture`, `impl/sqlite` | 7 = 7 | 23.2 → 3.8 MB | |
| `versioned-merged-identity`, `impl/sqlite` (13 widened relations) | 1 = 1 | 23.1 → 3.7 MB | |
| `cached-pure-lazy-allowlist`, `impl/sqlite` (26 widened relations) | 8 = 8 | 59.2 = 59.2 MB | |

`cached-pure-lazy-allowlist` reads the text of every node of the kinds it canonicalizes (`cpl_raw`, about 250 k
requests), so its requests are as many as the nodes: over all of `impl/` that is 718 MB of atoms and 2.2 GB at peak
with requests, and over 3 GB (killed) without.
Test: `node_text_answered_on_request`.

**The analysis in the profile.** The rewrite is made whether or not it is applied. The profile's decisions say,
per attribute (`node_text on request`), whether it is answered on request or why not, and with
`RERAIL_NODE_SOURCES=eager` whether it could be. Per reader (`node_text on request: readers`) they give the request
it asks and each literal it leaves out, with the reason (reads the value, a negation, an aggregate, not joined to
the node, reads what nothing kept binds). They also give each widened copy and its columns (`on request: widened
copies`). An attribute left eager is a structural finding (`EAGER_NODE_ATTRIBUTE`), with its reader and reason.

**What the 55 MB left were.** 40 MB were every distinct physical line of the corpus. They were not rule rows:
any program reading `text` made a `text` row for every span within the 256-byte bound, the lines included,
though `span_text_demand` already knew no rule read a line's text (`wants_line_text`). The source and its lines
now have text only where a rule reads it (`RERAIL_LINE_TEXT=all`: as before). `authentication-plan-purity` over
`impl/` then leaves 18.1 MB. That is 9.2 MB of identifiers and dotted names (6 MB qualified Python names, import
resolution), 4.1 MB of other one-line strings (decorator and string texts rules read, docstrings, relative module
names), 2.5 MB of multi-line texts and 1.9 MB of paths. The 11 rules of the pack that read `src_line` give the same
findings either way over `impl/sqlite`.

**What the widened copies cost.** Measured cold, by the native joins' work. `versioned-merged-identity` over
`impl/sqlite` (13 copies): 646,886 against 645,841 eager (+0.2%). `cached-pure-lazy-allowlist` over `impl/` (26
copies): the copies, requests and answers are 133,658 of 11.7 M (1.1%). A large planner figure in its profile
(`rr_wide_cached_gate`: "readers would add 950543476 work") is the price of inlining the copy into its readers,
which is why it is stored. It is not work done.

## 8. Step 2, `value` and `token`: as built (2026-11-13)

`rr::lower`'s `sources_on_request` runs over a table of attributes (`ON_REQUEST`), in two rounds, with one widening
for all of them (copies `rr_wide_<attribute>_<relation>`):

- **Round 1, `token`.** A request `rr_token_req(p)` asks for a parent's tokens. Its answer is a rule, not a
  function: `token(p, i, t, s, e) :- rr_token_req(p), rr_token_named(p, i, c, s, e), node_text(c, t)`, over
  `rr_token_named` (the named children that are nodes, by position and span). Their text is `node_text`, so it
  becomes a reader that round 2 answers on request in turn. The rest of the tokens (anonymous ones: their kind is
  their text; named ones without a node) stay an eager input, `rr_token_anon`, joined in by a rule
  `token(…) :- rr_token_anon(…)`. Those texts are a grammar's few keywords and punctuation, not the source.
- **Round 2, `node_text` and `value`.** `value` is answered by `@rr_node_value(n)`, which reads the node's literal
  value from the tape through the same registry (per document, once). The literal values a rule names by constant
  stay eager as `rr_value_lit`.
- **A negation asks like a positive read.** `out(s) :- node(_, n, "call", _, _), !node_text(n, "x")` asks
  `rr_node_text_req(n) :- node(_, n, "call", _, _)`: the answer for each node asked is complete, so the negation
  over it is the same. Only a constant node, an aggregate, or a node nothing binds keeps the attribute eager.
- **Literal strings only where read.** `str` and the literal values were interned for every literal whenever
  any literal relation was read. They are now interned only where `str` or `value` is itself read.

Checked against the oracle (`RERAIL_NODE_SOURCES=eager`), cold stores, strict SQL:

| Rule, corpus | Findings | Atoms | Peak memory | Wall |
|---|---|---|---|---|
| `schema-introspection-data-paths`, `impl/` | 0 = 0 | 433.6 → 112.7 MB | 1.40 → 0.97 GB | 20.0 → 12.5 s |
| `schema-introspection-data-paths`, `impl/sqlite` | 0 = 0 | 21.1 → 6.0 MB | 0.42 → 0.34 GB | |
| `serverplan-target-architecture`, `impl/sqlite` | 7 = 7 | 21.0 → 1.5 MB | | |
| `versioned-merged-identity`, `impl/sqlite` | 1 = 1 | 20.9 → 1.5 MB | | |
| `cached-pure-lazy-allowlist`, `impl/sqlite` | 8 = 8 | 57.0 → 38.2 MB | | |
| `classvar-check` (reads `token`), `impl/` | 0 = 0 | 432.8 → 145.7 MB | 1.59 → 0.93 GB | 12.0 → 10.2 s |
| `graphql-plan-ownership` (reads `token`), `impl/` | 0 = 0 | 433.9 → 85.6 MB | 1.62 → 1.01 GB | 17.8 → 14.1 s |

Walls are measured with the oracle run first; the first run of a pair is slower (the corpus's pages). What
`schema-introspection-data-paths` keeps over `impl/` is mostly the text of modules, which it reads.
`error-handling` reads every module's text, so its atoms stay 21 MB over `impl/sqlite`.

## 8b. With a library cone: found on the Python pack (2026-11-15)

A program that adds to a library relation takes the library's readers of it, its cone (0089 §3, `rr::with_cone`).
The cone was taken over the library with this rewrite in it. From a relation the program adds to, it reached the
widened copies and requests of that relation's readers, the attribute's answer, and every reader of the attribute.
Five of the pack's 317 programs (those that add to library relations) took 1,300 to 1,500 of the library's rules
each, and combining kept them apart (`p239_rr_agg`, `q8_truth_of`): the combined program went from 5,611 rules to
10,261 before its own rewrite. A cold run of one file, debug: 679 G instructions with `RERAIL_NODE_SOURCES=eager`,
930 G on request.

Now the cone is taken without the rewrite's rules (`lower::on_request_rule`: answers, requests, widened copies), and
`Text::lower_in` undoes the program's rewrite before adding the cone and makes it again over both. The combined
program is 5,611 rules again (5,722 after its rewrite), the outputs are unchanged, and the run is 826 G. Test:
`a_cone_is_taken_without_the_attributes_requests`.

What stays is the rewrite's own cost on this run: 147 G (debug, one file), in more statistics measured, more hiding
rounds and more cross-file strata. It is the trade §7 and §8 measured for single rules over 3,217 files; see the
measured rows below for the pack. `undo_on_request` is not exact either: it drops a `node_text` declaration the
program had (core's), and moves `value` to the end of the inputs. Combining is unaffected (the parts' sizes match
eager's), but printing and converting read the undone program (open).

### An answer placed in the cross-file part answered nothing

§6's third point, found as wrong findings. On the Python pack, `__all__`'s and `exports`' readers ask for `value`
across modules, so the split put `value`'s answer rule (`value(n, v) :- rr_value_req(n), v = @rr_node_value(n)`)
in the cross-file part. That part holds no documents: `@rr_node_value` reads the node's document from the chunk's
batch, found none, and answered nothing, without an error. Every reader of `value`, the file-local ones too, then
saw no value. On 50 files the pack reported 5 outputs apart from eager: RUF034 flagged `1 if a else 0` (with no
values on either side, compare.rr's `same_node` takes the branches for the same), PT014 flagged `[1, 5, 25]`, and
RUF015 missed one. The smallest pack that shows it is RUF034 with C414 (their `cmp_same_ask` additions bring
`dunder_all`'s readers in), and a one-rule `vals` reader gives no rows beside them, two files' worth alone.

Now, until a cross-file answer reads its documents:

- Combining (`answers_where_documents_are`) checks each attribute answered by a function of the document against
  the split, and an attribute whose answer is not file-local is eager again (its round undone: `lower::undo_round`).
  The profile's decisions say so ("value on request: no: its answer is not file-local …").
- Preparation refuses a program whose function answer is still cross-file, so this can no longer answer nothing.
- A program with a round undone still has core.rr's shape. `core_names` reads core.rr with the rewrite, so it
  holds the requests, `rr_value_lit` and the like, and the projection took a program without them for T's shape:
  it made no token leaves, and the planner found no statistics for `last_token`. The shape is now checked against
  `rr::core_shape`, the core names less those the rewrite adds (`lower::made_on_request`).

### Measured (2026-11-15)

The Python pack's 317 rules, release, one thread, fresh project store, no bundle unless named, M3 Max:

| Build | One file, on request | One file, eager | 50 files, on / eager (peak) |
|---|---|---|---|
| Before the hub merge | 38.9 G | | 273 G, eager's outputs |
| The merge | 49.5 G | | |
| With the cone fix | 41.7 G | 39.6 G | 300.8 G / 273.1 G (1,088 / 1,004 MB); 5 outputs apart |
| With the cross-file answer and shape fixes | 40.2 G | 39.9 G | 273.6 G / 272.5 G (1,010 / 998 MB); 0 apart |
| The same, with the bundle (0264) | 26.1 G | | |

The 50-file outputs now match the build before the merge, on request and eager, and so do the one-file runs and the
bundle's. On this pack `value` is eager (its cross-file readers), so on request costs what `node_text` and `token`
save less the analysis: 0.3 G on one file.

Test: `an_attribute_a_cross_file_reader_asks_for_is_eager` (a cross-file reader and a file-local one of `value`,
over two files; it fails, refused, without the first point).

## 9. Order

1. `node_text` on request, checked by §5. Remove 330d9cc2c's demand.
2. `value` (and `token`) on request, measured on `schema-introspection-data-paths`. Done, §8.
3. `text` past the bound on request. Remove the twins, `rr_long_text_node` and `text_kind_demand`'s long half.
4. Measure `text` within the bound (§6), and keep it eager unless requests are cheaper.
