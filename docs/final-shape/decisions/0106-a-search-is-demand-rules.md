# 0106: A search is demand rules

Status: draft (2026-11-10), for review. It replaces 0083 §4's "a search is a program kind" with ordinary rules,
as 0103 did for caches.

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

## 5. Order

1. `found` in P3 (t32 7.2): the agree count stays 6,410; `found`'s rows and its strata's work measured.
2. `lfound`, then `inst` for the library generics (t32 7.3).
