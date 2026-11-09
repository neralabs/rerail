# t26. Pruning facts that are never read, working backwards

## Intro

A cold run of `calls_cx` on the 84 files derives about 4.0 M rows across 748 documents. It makes 130 M join
probes to answer 6,299 calls. Most of those rows are about typeshed's stubs, and no answer reads most of them.
The engine evaluates bottom-up: a rule fires for every binding of its body, whether or not anything downstream
asks for its head.

This note is a starting point for thinking about the opposite direction. The idea is to work back from what is
read (the outputs, or a phase's exports) and mark the facts that no reader can reach, then not derive them, not
store them, and not maintain them. Datalog's classic tool for this is magic sets. The engine already has three
narrow forms of it. The question is how to generalize them without making a second evaluator.

## What "never read" means

A fact is **live** if some derivation of a live fact uses it. Output rows are live, and so are a phase's export
rows. Everything else is **dead**: deleting it changes no output.

Liveness flows backwards through rule instances:

```text
// forward                                   // backward: a body fact is live if a live head used it
calls(c, f) :- call(c, x), look(c, x, s), bv(s, f).
                                             live_look(c, x, s) :- live_calls(c, f), call(c, x), look(c, x, s), bv(s, f).
                                             live_bv(s, f)      :- live_calls(c, f), call(c, x), look(c, x, s), bv(s, f).
```

Negation needs care. `!R(k)` reads R at the key k. The facts of R that match k are the ones that block a
derivation, and they are live. The absence of a fact is read too, which matters for maintenance (§ The hard
cases).

## Two ways to use it

1. **Before deriving (magic sets).** Rewrite each rule so that its head is derived only for keys some reader
   asks for:

   ```text
   need_bv(s)    :- need_calls(c), call(c, x), look(c, x, s).   // the reader's prefix binds s
   bv(s, f)      :- need_bv(s), <bv's body>.
   ```

   This is exact when the binding is selective: a call site needs one callee, and the callee needs its own
   returns. It loses when the demand is nearly everything, because the `need_` relations cost what the pruning
   saves.

2. **After deriving (learned liveness).** Compute liveness over a run's fixpoint, store it, and use it next run:
   - dead rows are not stored (the 4.0 M rows the cross-file memo stores);
   - dead relations, columns or documents are not derived next time, speculatively, with a check.

   In the incremental setting (0083 §2), liveness is a relation like any other, maintained by deltas. An edit
   that makes a dead fact live is a delta to `live_R`, like any other.

## Granularity

The cost of tracking liveness falls with granularity, and so does the precision. From coarsest to finest:

| level | what is marked | where it already exists |
|---|---|---|
| relation | a relation no output reaches | `prune, simplify, demand` (lowering) |
| column | a column no reader joins or returns | partly: per-chunk projection |
| document / import component | a document whose facts no checked file reaches | not yet; 0083 §5 says a library document runs P1 only |
| key | a value of a bound column some reader asks for | 0048 (negated relations), 0065 §2a requests, 0083 §4 searches |

**Document level** is the cheap, large case here. P3 answers only for checked files. A stub's facts are live
only if a checked file's name resolution reaches that stub's module, class or function. Demand by document is a
few thousand rows, not 4.0 M, and it is exactly what "typeshed is its own phase and hands on only its exports"
gives structurally (point 4 of the 2026-11-08 discussion).

## What exists, and how this would generalize it

- **0048, demand for negated relations.** A non-recursive relation that rules only negate, and that starts from
  every node, is computed for `demand_R(args)`. The reader's positive prefix is split into `pre_<i>`. This is
  magic sets for one adornment, with the stratification problem avoided by putting the demand in a lower
  stratum.
- **0065 §2a, requests.** `load`, `dir_entry` and the other world requests are demand relations answered inside
  the fixpoint. Gated negations wait for a key's requests to settle.
- **0083 §4, searches.** A program kind that is demand-driven and memoised per key, for the relations where
  eager evaluation is hopeless: instantiation, method-resolution lookups, protocol matching.

A magic-set rewrite chosen by analysis is the same thing as deciding which relations are searches. So the
generalization is not a new mechanism. It is a **pass that chooses, per relation and binding pattern, eager
or demanded**, the way the planner chooses join orders, and lowers the demanded ones as 0048 and 0083 §4
already do. The same engine runs both forms (memory: phases, not new evaluators).

## The hard cases

- **Negation.** Magic rules can make a stratified program unstratified: demand for `R` can depend on a
  relation that negates `R`. There are known answers:
  - 0048's split, which puts the demand in the stratum below;
  - 0065's per-key gates;
  - magic sets for stratified programs (Balbin and Ramamohanarao; Ross), which add the demand relations as
    their own strata.
- **Aggregates.** `count : { … }` needs its whole group. Demand passes through the group's key, and the
  body is evaluated in full for each demanded key.
- **Recursion.** Fine in principle, and the standard case for magic sets. The cost is the `need_` relation's
  own recursion. A transitive closure demanded from a few roots wins; one demanded from every node loses.
- **Memo and maintenance.** A demanded relation is partial: R is complete only for the keys it was asked. The
  per-file and cross-file memos would have to key entries by demand, or store "R complete for k" as the search
  memo of 0083 §4 does. Maintenance must track demand deltas: a new reader key makes a key live and starts its
  derivation.
- **Choosing.** Demand pays when it is selective. The choice needs the same statistics the planner uses,
  with an eager fallback. A wrong guess costs time, never answers.

## A first experiment, before building anything

To measure what pruning can save, without guessing:

1. Generate the backward program mechanically from a forward program. For each rule
   `H :- B1, …, Bn`, emit one `live_Bi(x̄) :- live_H(h̄), B1, …, Bn` per positive body literal, and seed
   `live_out` from every output. A negated literal adds live rows of its relation at the probed key.
2. Run it over a finished run's fixpoint, in a phase that reads the forward run's relations. The engine and
   the joins are the same.
3. Report, per relation: rows, live rows, and the live fraction. Also report by document: the live fraction of
   typeshed's documents against the workspace's.

The result is an upper bound on what any pruning can save, relation by relation. It also names the relations
where key-level demand would pay (a small live fraction and a selective binding), and those where only
document-level demand is worth it.

## Open questions

- Whether liveness should be exact (every derivation) or one derivation per live fact. One is enough to keep
  a fact, and it is cheaper.
- How demand and the import components of 0076 interact. A component is the unit of the cross-file memo, and
  demand would make its rows partial.
- Whether column-level liveness (projecting away columns no reader reads) is a separate cheap win. It shrinks
  distinct rows, so it shrinks probes.
