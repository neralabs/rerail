# 0035: Cross-file rounds bound file crossings, not iterations

Status: accepted (2026-11-04, list step 29; I11; O41)

## Intro

A typing program has a file-local part and a cross-file part (`typing::file_local::programs`).
03 §13 adopts bounded rounds for the cross-file part: round r reads round r − 1's interface
rows, there are at most k = 2 rounds after the first, and an unchanged digest stops them early.

The question here is what "a round" bounds in a Datalog program. Bounding the iterations of
every recursive stratum would also cut chains that never leave a file, such as a class's
attribute found on a base class in the same file. So a round bounds **file crossings**:

```text
// calls.rdl: an attribute is found on the class, or on its base, which may be in another file.
found($c, $a, $x) :- base($c, $b), !bound_in($c, $a), found($b, $a, $x).

// Same file: the fixpoint inside the round, exact.
found($c, $a, $x) :- base($c, $b), !bound_in($c, $a), found($b, $a, $x), @doc($b) = @doc($c).
// Another file: the previous round's rows.
found($c, $a, $x) :- base($c, $b), !bound_in($c, $a), found__prev($b, $a, $x), @doc($b) != @doc($c).
```

## Decision

- **A hop** is a recursive body atom whose first node column's variable is not the head's. Each
  hop gets the two cases above. A rule with n hops gets 2^n rules (n ≤ 3, otherwise left as is).
- **`@doc(n)`** is a node's document. Nodes are `d << 32 | index`, so the document can be computed
  without reading any relation. Only the SQL strata have it; native modes fall back to SQL for
  those rules.
- **Rounds:**
  - Round 0 reads no previous rows.
  - Round r reads round r − 1's rows of the hop relations from `x.<rel>__prev`.
  - The rounds stop when the digest of the hop relations' rows is unchanged (the result is then
    the fixpoint), or after round k.
  - The run reports `converged`.
- **Not chosen:** a cap on semi-naive iterations. It cuts same-file chains, and its result
  depends on the evaluation order.
- **Not chosen:** the layered wavefront of `typing::rounds` over the module dependency graph.
  That needs the graph before the program runs, and the graph is what the program derives.
  `typing::rounds` remains the reference for the per-document cutoff (O41).

## Evidence

- On the vtab golden corpus (`own_ingestion_derives_what_t_facts_do`), workload.rdl converges
  within k = 2. calls.rdl needs a third crossing (8 of 5,112 hop rows): O41.
- The split into file-local and cross-file parts follows the ledger's row 20 (partition by
  file, and keep global machinery for the rules that need it).
- The AST-index spike scoped cross-file resolution out (design index §17, "Data flow / name
  resolution / cross-file facts"), so it offers no prior mechanism to reuse.
