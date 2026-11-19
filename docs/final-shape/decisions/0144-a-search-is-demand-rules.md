# 0144: A search is demand rules

Status: draft (2026-11-10), for review. It replaces 0083 §4's "a search is a program kind" with ordinary rules,
as 0103 did for caches.

Numbered 0107 where it was written; renumbered 0144 in the hub (2026-11-13) because another record had 0107.

## Intro

**The problem.** Some questions have a huge key space and few keys asked. The question "where is attribute `a`
of class `c` defined, looking through the bases?" is one: P3's `found` answers it for every name bound in every
class, and for each base it inherits through, though calls ask for a few thousand pairs. 0083 §4 proposed a new
program kind for such questions, `.search`, with its own batching, memo and lowering. That would be one more
mechanism beside the facts, the shape 0103 removed for caches.

**What changes.** A search is written in the `.rr` file as plain rules over a **request relation**:

- **the request rules** derive the keys some reader asks for, and the keys a step of the search asks in turn;
- **the answer rules** read the request, so an answer is derived only for a key that was asked.

The engine evaluates them as it evaluates every rule. It has no search kind, no request queue and no memo of its
own. A wave's new requests are the semi-naive delta of the request relation, so they are answered together in
the next wave: that is 0065 §2a's loop, which a fixpoint already runs.

## 1. The form

```text
// mro_ask(c, a): attribute a is looked up on class c: asked by a reader, or by the search on a base.
.decl mro_ask(c: node, a: symbol)
mro_ask(c, a) :- asks(_, c, a).
mro_ask(b, a) :- mro_ask(c, a), base(c, b), !bound_in(c, a).

// found(c, a, at): a, looked up on c, is defined on at. Only for asked pairs.
.decl found(c: node, a: symbol, at: node)
found(c, a, c) :- mro_ask(c, a), bound_in(c, a).
found(c, a, x) :- mro_ask(c, a), base(c, b), !bound_in(c, a), found(b, a, x).
```

A miss is an asked key with no answer row, and it is read as `mro_ask(c, a), !found(c, a, _)`. That is an
anti-join over a closed relation, so it needs no gate.

## 2. Why not a cache

A search's answer is derived from relations, so it is kept, maintained and invalidated by their identities and
deltas (0104, 0077), as every derived relation is. A cache (0103) is for a step outside the rules, a parser or a
native call, whose result the rules cannot track, so a person bumps its version tag. A search has no such step,
so it has no version tag and no `.cache` line.

## 3. What the engine needs

Nothing new. Two things it already does matter:

- **Recursion through the request.** `mro_ask` and `found` are one recursive component with `pv` when a request
  comes from a value (`asks` reads `pv`). The negations read `bound_in`, which is lower, so the component
  stratifies.
- **Maintenance.** A changed class's names change `bound_in`. The requests through it and the answers they gave
  are maintained by their deltas (DRed or counting), as every recursive stratum's are.

## 4. What is a search

0083 §4's table holds, read as "write it as demand rules":

| Question | Request relation | Readers |
|---|---|---|
| Attribute lookup through the MRO (`found`, `lfound`) | `mro_ask(c, a)` | `asks`, a class call's `__new__`, a metaclass's `__call__` |
| Generic instantiation (`gpar`, `gbind`, `wbind`, `lgpar`, `lcbind`) | `inst_ask(f, targs)` | a call of a generic function or method |
| "Not overridden", closed-world `self` | `sub_ask(c)` | a `self` whose class is asked |

## 5. Measured: `found` is not a search in P3

P3's `found` was written as §1's demand rules and measured against its eager form on 54 of the corpus's files
(`_internal`, `l1100_exceptions`, `l2055_lazy_result`), each run with a fresh store and no kept part, 2026-11-10:

| | eager `found` | demand (`mro_ask`) |
|---|---|---|
| answers (agree with jedi) | 4,815 | 4,815 |
| `found`'s own join work | 279 | 3,976, and `mro_ask` 10,644 |
| the `pv` component | 99 relations, 1.81 M work, 1.56 s | 105 relations, 2.05 M work, 1.64 s |

The eager `found` was never the cost: it is in `pv`'s component too, but its join work is 279 units of 1.81 M.
The demand form's requests read `asks`, which reads `pv`, and that pulls six more relations (`mro_ask`, `metacls`, and the
annotation phase's `bv`, `anc`, `uinst` and `gleaf`) into the component, each derived again on every wave: 13.5% more work there. So P3
keeps the eager `found`.

The rule this gives: **write a search only where the eager relation is the cost** (measured), and only when the
requests do not pull the answer into a larger recursion than it was in. A request that reads a value (`pv`) makes
the answer part of that value's fixpoint.

## 6. Order

1. Measure the eager relations 0083 §4 names as searches before writing any: `wbind`, `gpar`, `lgpar`, `lcbind`
   (t27: `wbind` was 2.7 M join work for 23 rows) first.
2. Write as demand rules only those that are the cost, and measure again.

Measured 2026-11-10 (step 1): P3 cold, with a profile, over the 84-file small corpus (6,410 calls agree with jedi), at
load 23 to 57. Join work by the relation a rule derives, of 30.9 M in all (43.6 s of join CPU):

| Relation | Join work | Share | CPU |
|---|---|---|---|
| `lcbind` | 315,894 | 1.02% | 0.12 s |
| `lgpar` | 281,306 | 0.91% | 0.08 s |
| `wbind` | 102,788 | 0.33% | 0.02 s |
| `gpar` | 69,258 | 0.22% | 0.02 s |
| `found` | 7,107 | 0.02% | 0.02 s |
| `lfound` | 5,183 | 0.02% | 0.02 s |
| `lwbind` | 4,328 | 0.01% | 0.01 s |
| `gbind` | 1,242 | 0.00% | 0.00 s |

Together 2.5% of the work and 0.3 s. `wbind` was 2.7 M for 23 rows at t27; the closures and the dropped columns
(0098) since took that. None is the cost, so none is written as a search: step 2 has nothing to do. The cost is
elsewhere: `py_try` (4.5 M, the stub search that requests answer), `tv` (1.9 M) and `tbv` (1.0 M, with its copy in
the annotations, 1.7 M).

`py_try`'s stubs layer joined every entry of every typeshed distribution with every wanted module (1.57 M of its
2.15 M on the 54-file set, cold, for 44 rows). With the wanted tops (`py_want_top`) and the distributions' directories
(`py_stub_dir`) as relations of their own, each top is looked up by its name: 77 k and 0.04 s CPU (was 1.31 s), the
same 4,815 calls agreeing. No search and no demand rule: a rule written so its join is the lookup.
