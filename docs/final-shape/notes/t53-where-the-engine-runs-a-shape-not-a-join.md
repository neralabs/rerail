# t53: Where the engine runs a shape, not a join

Status: a note (2026-11-22). It answers a question asked with t50's goal: where should the engine make specialized
execution shapes? A connectedness question, for example, has far better representations than the dense reachability
matrix naive facts become. And at the limit, can the engine find the `.rerail` shapes that were translated from Rust,
and run the original Rust for them?

## Intro

A rule says what holds, not how to find it. The engine runs every rule the same general way: a join program over
relations, with the rows it derives kept as rows. For most rules that is the right shape. For some it is the wrong
data structure, not just a slow plan:

- A transitive closure held as rows is the dense reachability matrix. On the 54-file Python corpus, `rr_cx_reach`
  grew to 6.3 M pairs, 220 MB, so that every reader could ask one membership (0119).
- "The first `X` of a group, by an order" held as rows joins the group with itself to prove nothing comes earlier
  (`!later`): quadratic in the group, where W keeps a sorted vector (t50 §2, "First by an order").
- "The binding in force at `r`" (`max t : { B(k, t), t < r }`) as rows is an aggregate over every earlier binding,
  where W does a binary search (T5).

W, the Rust these rules were translated from, used the right structure in each case: hash maps by name, sorted
vectors, a visitor's stack, C3 by memo. The rules lost the structure and kept the meaning. Each specialized shape
puts the structure back, under the same meaning.

**The answer.** Make a specialized shape where a rule's *form* fixes a better structure than rows, recognize that
form by structure (never by a relation's name), and run it as an operator that the planner prices against the
general join. The operator reads blocks where they lie and is written once, for the CPU and Metal. W's Rust is
the oracle for it, not its implementation.

## 1. What already exists

| Shape | The form | The structure | Where |
|---|---|---|---|
| Reach | `r(x, z) :- r(x, y), edge(y, z)` and seeds, read only as membership | one compressed bitmap per start; a strongly connected component's starts share one | 0119, `logic/reach_form.rs`, `native/reach.rs` |
| Nearest ancestor | `nearest(n, a, set)` | one pass in preorder (CPU), pointer jumping (Metal) | 0038, 0040, `typing/file_local.rs` |
| Tree axes | `within`, `child`, `field` read from the tape | the tape's preorder intervals, no rows made | 0143, 0353 |
| First of a group | `X, !later(X)` under an order | a group argmin | t50 step 2, 0515 |
| Aggregate bounded by its probe | `max t : { B(k, t), t < r }` | a search in an ordered run | t50 step 3, 0511 |
| Walk as a keyed closure | `walk`/`look` | one wave of a keyed closure | t50 step 5, 0570 |
| Matchers | `ast(...)` patterns | the transducer's matcher wave | 0106, 0459 |

Each recognizer reads the body's shape, rewrites the relation into an operator node (`Native::Reach { seed, edge }`
and its kind), and leaves the rest of the program as rules. Each operator's rows were checked equal to the join's
before it replaced the join.

## 2. Where to make one next

The rule for choosing: a shape is worth an operator when the general join's *representation* is wrong, so that
the rows themselves grow faster than the answer: quadratic pairs, a group joined with itself, every node times every
scope. A plan that is wrong under the right representation is a planner fix, not a shape. The `pre_401` case below
is one.

In the order the measurements give them (t50 §2–3, and this session's product run):

1. **Connectedness and reach beyond 0119's form.** 0119 takes only the left-linear closure read as membership.
   Not yet taken:
   - an undirected component question (`same_component(a, b)`), as union-find;
   - a closure whose readers join its pairs rather than test them, as an interval labelling of a DAG
     (a spanning tree's preorder intervals plus a few non-tree edges' exceptions), with a component's condensation
     first;
   - a closure over the tree itself (`within` written as a recursive rule), as the tape's own intervals.

   The test for each: the operator's rows equal the rules'. Work and memory must be proportional to the answer's
   readers, not to the pairs.
2. **Name to definition (C1)** and **bases in order (C2)**: W's per-name hash maps and C3 by memo. In the rules,
   `rf_hit` and `rf_target` make symbols and routes per step. A name forest with keyed nearest (t50 step 7) is the
   shape.
3. **The scope table (T1, T4)**: about 40 relations over `kind`, `field`, `child` and `text`, which W builds in one
   visitor. Matcher captures in the one matcher pass (t50 step 6) are the shape: tree patterns are what the
   matcher wave already runs.

Not shapes, so not operators:

- `pre_401` and `pre_402` (`def_outside`). The plan expands a scope to every node under it, then narrows by
  `nameref`. Each node has one nearest scope, and the parameters' subtree is small. That is the pricing of
  `within` from a part (t50 step 4), a plan fix.
- `rf_ref`'s 4.3 M-row file-local fragment. It was a split decision (`text` was not a node's attribute), fixed in
  the push pass.

## 3. Running W's Rust for a translated shape

The question's limit: since the rules were translated from W's Rust, could the engine recognize the translated
rules and run W's code? My recommendation is no, as a mechanism, and yes, as an oracle. The reasons:

- **W's code reads W's structures.** Its input is its own AST visitor's state, `HashMap<name, Vec<Binding>>` and
  per-module tables. To call it, the engine would first build those structures from the blocks: a second structure
  the engine keeps, which readers read instead. That is the copy 0353 forbids, and it is the cost W's run pays that
  the engine set out not to.
- **It would run on one backend.** An operator in the engine's IR runs on the CPU and Metal, and in the
  per-chunk and cross parts alike. W's Rust is a CPU program over a whole repository.
- **Recognition would key on names or on one translation.** A translated rule set changes with every edit to the
  rules. Matching it to a Rust function needs either the relation names (which the structural-rewrite rule
  forbids) or a fingerprint of one version of the rules, which an edit breaks silently.
- **It does not compose.** The rules around a shape read its rows, and the planner prices its probes. A call into W
  is opaque to both: no statistics, no demand, no maintenance by delta.

What does carry over from W is the *algorithm*. Each operator in §1–2 is W's structure, taken to blocks: W's
sorted vector is the group argmin, its binary search the bounded aggregate, its stack the keyed nearest. So the
practical form of "run W's code" is:

1. Name the shape by structure: its rule form, as 0119 does.
2. Write the operator once, from W's algorithm, in the engine's IR, over blocks.
3. Keep W's Rust beside it as the test oracle. The operator's rows equal the rules' rows and W's, on the fixtures
   and on `all`, as `reference_edge_is_ws_reference_graph_on_*` holds the graph today.

W stays the reference that says what fast looks like (51 s of CPU on `all` against the head's 183 s, t50), and
each operator is measured against it.
