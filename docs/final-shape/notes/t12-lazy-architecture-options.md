# t12. lazy-architecture: why it is refused, and five ways out

## Intro

`lazy-architecture` checks one rule of the data layer's design: code that
should only *plan* work (lazy and `@pure_lazy` functions) never *runs* it,
either itself or further down its call chain. It is one of the most
important checks in the pack, and it has to be fast and exact.

To do that, the check first gives every function and method a name, such
as `pkg.mod.Outer.Inner.method`. These names are the keys of its call
graph. To build the `Outer.Inner` part, it climbs from a class to the class
that holds it and joins the names with dots.

The engine refuses to run the check. Before running a rule, it proves that
every recursion which builds new strings must stop. It proves this by
showing that each step moves one way through the syntax tree. The climb
from a class to its parent cannot be proved that way, although it does
stop. So the engine rejects the whole check.

This note explains why the proof fails. It then sets out five ways to fix
it, with their cost, accuracy and speed, and the engine changes each
needs. It ends with a recommendation.

## Two errors, one already fixed

The check first failed like this:

```
Front(AggregationInRecursion { head: "la_cb", aggregated: "rr_agg76",
                               component: {"la_cb", "rr_agg76", "rr_ctx77"} })
```

The rule behind it (`lib/data-layer/lazy-analyzer.rr`, `la_cb`):

```
// A name unpacked from a tuple whose value holds a callable is a callable
// alias, unless a later plain assignment of the name clears it.
la_cb(n, e) :- la_ask_cb(n, e), kind(e, "identifier"), text(e, x),
    la_vis(n, x, s, v, "elt"), la_cb(s, v),
    count : { la_vis(n, x, s2, _, "rhs"), s2 > s } = 0.
```

The count reads only `la_vis`, which is in a lower stratum. But the
front end built the count's *context* relation (`rr_ctx77`, which holds
the `(n, x, s)` groups to count for) from every other literal of the
clause, `la_cb(s, v)` included. That put the count inside `la_cb`'s own
recursion.

This is fixed: in `logic/rr/lower.rs`, `context()` now leaves out atoms
of the clause's own head when the other literals bind the variables
already. The test is `rr::tests::a_context_leaves_out_the_clause_head`.

With the fix in place, the check stops at the next guard:

```
UnboundedValueCycle { component: {"la_cls"}, rule: "la_cls(k, d, @dot(p, x)) :- …" }
```

The rest of this note is about that second error.

## What the rule needs

The Python script collects a class when it is at module level, or when it
sits directly in the body of a class that was collected. Its qualified
name is the chain of class names from the module down:

```python
class Outer:              # collected, q = "Outer"
    class Inner:          # collected, q = "Outer.Inner"
        def run(self): …  # fact symbol "pkg.mod.Outer.Inner.run"
    if FLAG:
        class Hidden: …   # not collected: its parent is the `if`
def f():
    class Local: …        # not collected: its parent is a function
```

In `.rr` (`lazy-analyzer.rr`, lines 44–46):

```
.decl la_cls(k: node, d: number, q: symbol)
// At module level: q is the class's own name.
la_cls(k, d, x) :- kind(k, "class_definition"), ast_parent(k, m),
    kind(m, "module"), def_name(k, x), docof(k, d).
// Directly in a collected class o: q is o's name, a dot, then k's own.
la_cls(k, d, @dot(p, x)) :- kind(k, "class_definition"), ast_parent(k, o),
    la_cls(o, d, p), def_name(k, x).
```

`la_fact`, `la_cls_of`, `la_local` and `la_ctx_class` all read `la_cls`.
So every finding of the check depends on it. The relation is tiny: one
row per collected class, a few thousand rows over the whole corpus.

## Why the engine refuses it

`@dot(p, x)` makes a new string from `p`, and `p` comes from `la_cls`
itself. This is a *value cycle*. `front::cycles` accepts a value cycle
only when one node column (the key) moves strictly one way along the
tree in every recursive rule. A tree is finite, so a walk in one
direction must end.

The checker has a single measure: **a node's preorder index**. It treats
"up" as "to an earlier node" and "down" as "to a later node". Each
relation gets a guarantee from its rules. That guarantee is the weakest
move that holds in *every* rule.

`ast_parent` (`lib/python/ast.rr`) has three rules:

```
// 1. The parent in the tree, when it is not a wrapper or a decorator.
ast_parent(n, p) :- child(p, n, _), !wrapper_node(p), !kind(p, "decorator"), …
// 2. A decorator's expression belongs to the def it decorates.
ast_parent(e, f) :- decorator(f, e).
// 3. Past a wrapper (block, decorated_definition, parentheses, …).
ast_parent(n, q) :- child(p, n, _), wrapper_node(p), …, ast_parent(p, q).
```

Rules 1 and 3 always move to an earlier node. Rule 2 moves to a **later**
node, because tree-sitter puts the decorators before the definition:

```
decorated_definition        preorder 10
  decorator                 11
    call  <- e              12
  function_definition <- f  14     ast_parent(e, f): from 12 to 14, "down"
```

So `ast_parent` as a whole has no preorder guarantee. `la_cls` climbs it,
so `la_cls` has no key, and the component is refused.

Two facts make the refusal a false alarm:

- In `la_cls`, `k` is always a `class_definition`. Rule 2 only applies to
  a decorator's expression, which is never a class. The checker cannot
  see this: it does not know which kinds a relation's columns can hold.
- Every rule of `ast_parent` moves strictly up **in depth**. Rule 2 goes
  from depth `D+2` (the expression under the `decorator`) to `D+1` (the
  definition). The checker has no depth measure, so it cannot use this.

## The options

### A. Climb along tree edges in `la_cls` (rule change, one clause)

Write the step from a class to the class that holds it with base tree
edges, rather than `ast_parent`. In this one place, `ast_parent` follows
exactly two shapes: the class is in the holder's body block, or it is
in a `decorated_definition` inside that block.

```
// Directly in collected class o's body, decorated or not.
la_cls(k, d, @dot(p, x)) :- kind(k, "class_definition"), la_in_class(k, o),
    la_cls(o, d, p), def_name(k, x).
.decl la_in_class(k: node, o: node)   // not recursive: tree edges only
la_in_class(k, o) :- kind(o, "class_definition"), field(o, "body", b), child(b, k, _).
la_in_class(k, o) :- kind(o, "class_definition"), field(o, "body", b), child(b, dd, _),
    kind(dd, "decorated_definition"), field(dd, "definition", k).
```

- **Accuracy.** For a `class_definition`, this is the same as
  `ast_parent(k, o), kind(o, "class_definition")`. A block's parent is
  the class it belongs to. A `decorated_definition` is a wrapper whose
  parent is the block. Classes under `if`, `try` or a def are still left
  out, as before.
- **Speed.** It uses three base atoms, each indexed on its key, in place
  of a recursive relation. It is no slower, and probably a little
  faster.
- **Engine changes.** None. `child` and `field` are already strict tree
  moves in `cycles::base()`. Every step from `k` to `o` is "up", so the
  key is proved.
- **Cost.** One clause and one helper in `lazy-analyzer.rr`.
- **Limits.** It fixes only this rule. The next rule that climbs
  `ast_parent` and builds a value will hit the same refusal.

### B. A climb without the decorator edge in `python/ast.rr` (library change)

The library can offer a parent relation that only ever goes up, and keep
`ast_parent` as `ast`'s parent built on top of it:

```
// ast_up(n, p): ast_parent without the decorator edge: always to an earlier node.
.decl ast_up(n: node, p: node)
ast_up(n, p) :- child(p, n, _), !wrapper_node(p), !kind(p, "decorator"), !kind(n, "comment").
ast_up(n, q) :- child(p, n, _), wrapper_node(p), !kind(n, "decorator"), !kind(n, "comment"),
    ast_up(p, q).
// ast_parent(n, p): as before; a decorator's expression belongs to the def.
ast_parent(n, p) :- ast_up(n, p).
ast_parent(e, f) :- decorator(f, e).
```

Then `la_cls` uses `ast_up(k, o)`. (`la_ctx_class` climbs the same way but makes no
values, so it may keep `ast_parent`.)

- **Accuracy.** For any node that is not a decorator's expression,
  `ast_up` and `ast_parent` are the same. Classes and defs are never
  decorator expressions, so `la_cls` is unchanged.
- **Speed.** It is the same as today. The rows of `ast_parent` are now
  the union of `ast_up` and `decorator`, which magic sets and demand
  handle as before.
- **Engine changes.** None. The checker's summary fixpoint gives
  `ast_up` a strict "up" guarantee from its rules.
- **Cost.** Two relations change in `python/ast.rr` and two atoms in
  `lazy-analyzer.rr`. Any rule that climbs and builds values gets a
  relation it can be proved on.
- **Limits.** A rule author has to know to use `ast_up` in a recursion.
  The error should say so (see "Engine follow-ups").

### C. Build the chain first, then join the names (stratify the string)

Split the work into a recursion that makes no values and a single
`join` over its result:

```
// The collected classes: a recursion over nodes only, so it is no value cycle.
.decl la_ccls(k: node)
la_ccls(k) :- kind(k, "class_definition"), ast_parent(k, m), kind(m, "module").
la_ccls(k) :- kind(k, "class_definition"), ast_parent(k, o), la_ccls(o).
// la_chain(k, a): a is k or a collected class that holds k.
.decl la_chain(k: node, a: node)
la_chain(k, k) :- la_ccls(k).
la_chain(k, a) :- la_ccls(k), ast_parent(k, o), la_chain(o, a).
// The qualified name: the names along the chain, outermost first. A holder
// comes before what it holds in preorder, so its visit index is smaller.
la_cls(k, d, q) :- la_ccls(k), docof(k, d),
    q = join x by i sep "." : { la_chain(k, a), def_name(a, x), visit_index(a, i) }.
```

- **Accuracy.** The result is the same. Outer classes have a smaller
  preorder index than the classes they hold, so ordering by visit index
  gives `Outer.Inner`.
- **Speed.** `la_chain` holds one row per class per nesting level, which
  is a few thousand rows. The `join` is one grouped aggregate in a lower
  stratum. It costs about the same as A.
- **Engine changes.** None, if `join … by … sep` over a helper body
  lowers as it does in `pyflakes.rr` and `compare.rr`.
- **Cost.** Three relations in place of one. It also reads less like
  the script than A or B do.
- **Limits.** It changes how the rule is written, not what the engine
  can prove. It is useful as a general pattern: any "path name" over a
  recursion can be written as a closure plus `join`.

### D. A depth measure in `front::cycles` (engine change, rule as written)

Give the checker a second measure next to preorder: **depth**. Each tree
edge changes depth by exactly one (`child`, `field`, `tape`'s parent
column). `nearest` moves at least one up, and a sibling move keeps the
same depth. Derived relations get a depth guarantee from the same
greatest-fixpoint summary they use today. A component is accepted when
**one** measure (preorder or depth) proves every one of its rules.

Unlike preorder, depth needs counts rather than just a direction,
because `decorator(f, e)` is up two edges and down one. The domain is an
interval of depth change per pair of node columns, for example
`[-1, -1]`, `(-inf, -1]` or `[0, 0]`. Moves along a path in a rule body
are added together, and a recursive relation is widened to infinity
after a few rounds.

```
// decorator(f, e) :- kind(dd, "decorated_definition"), field(dd, "definition", f),
//                    child(dd, dc, _), kind(dc, "decorator"), child(dc, e, 0).
//   e -> dc -> dd: -1 -1 ; dd -> f: +1          => decorator: e to f is [-1, -1]
// ast_parent rule 1: [-1, -1]; rule 2: [-1, -1]; rule 3: -1 + ast_parent
//   fixpoint: ast_parent n to p is (-inf, -1]: strictly up in depth
// la_cls: k to o through ast_parent: strictly up  => key proved, accepted
```

- **Accuracy.** The rules run exactly as written. The proof stays
  sound: each tree edge is an exact constraint on depth, so adding them
  along any path gives a valid bound.
- **Speed.** It costs nothing when rules run. The check runs once per
  program, over small rule bodies.
- **Engine changes.** `logic/front/cycles.rs`. A `Depth` interval
  type goes beside `Step`. `base()` gains depth deltas, and `edges()`
  and `reach()` carry them (a shortest and longest sum, or the first
  path found, since any path gives a sound bound). `summary()`,
  `meet()` and `summaries()` get an interval meet with widening. The
  key search tries depth when preorder fails. Tests go in
  `logic/front/tests.rs`: `ast_parent` climbing with `@cat` is accepted,
  and a climb over a graph edge is still refused. Decision 0046's
  "value-cycle bound" paragraph gets one sentence.
- **Cost.** About 150 lines and their tests. Two measures must not be
  mixed inside one rule's proof. Keep them as separate passes so the
  existing preorder proofs do not change.
- **Limits.** It proves climbs and descents. It does not prove moves
  that go sideways and up, such as preorder `$a < $b` comparisons,
  which have no depth meaning.

### E. A native qualified-name fact (engine change, broad)

The transducer already keeps a scope stack as it walks each tree. It
could emit `def_qualname(n, q)` for every class and def, in Python's
`__qualname__` form (`Outer.Inner`, `f.<locals>.Local`). Then:

```
// Collected: every holder up to the module is a class directly holding it.
// The recursion is over nodes only, and q comes from the native fact.
la_cls(k, d, q) :- la_ccls(k), def_qualname(k, q), docof(k, d).
```

- **Accuracy.** It is exact for collected classes. Python's qualname of
  `Hidden` (inside `if FLAG:` in a class body) is also `Outer.Hidden`,
  so `la_ccls` is still needed to leave it out. This is the same
  node-only recursion as in C.
- **Speed.** It is the fastest of the five: one pass during the tree
  walk that already happens, and no string building in Datalog.
- **Engine changes.** A new view in `facts/vtab/syntax.rs`, plus its
  declaration in `core.rr` or `python/names.rr`, its column list in
  `logic/sql/select.rs` and `logic/plan.rs`, and vtab tests. Text is
  interned per document, so the memory cost is one symbol per def.
- **Cost.** The highest of the five, and it adds new surface to
  maintain. It pays off beyond this rule: approximate typing, call-graph
  keys and other qualified-name checks could read the same fact rather
  than each rebuilding it.
- **Limits.** It is Python only, and qualname rules differ for other
  languages. On its own, it does not remove the need for A, B or D for
  other value-building climbs.

### Not recommended

- **A nesting counter with a cap:** `la_cls(k, d, q, lvl)` with
  `lvl = @add(l0, 1), l0 < 64`. The checker accepts this today, but it
  adds a fake column and a magic limit to an exact rule.
- **Teaching the checker which kinds a column can hold,** so it can
  ignore `ast_parent` rule 2 when `k` is a class. This would need
  grammar knowledge (which kinds a decorator's expression can be) inside
  the front end. That is too much machinery for one edge.

## Comparison

| | rule change | engine change | accuracy | run cost | reach |
|---|---|---|---|---|---|
| A tree edges | 1 clause + helper | none | same | same or less | this rule |
| B `ast_up` | `ast.rr` + 2 atoms | none | same | same | every climb |
| C closure + join | 3 relations | none | same | about the same | pattern |
| D depth measure | none | `cycles.rs` | same | none | every climb, as written |
| E native qualname | 1 clause | vtab + decls | same | least | qualname users |

## Recommendation

1. The context fix is in place (above).
2. **Unblock now with B.** Add `ast_up` to `lib/python/ast.rr` and use it
   in `la_cls` (line 46) of `lib/data-layer/lazy-analyzer.rr`. It needs no engine change, keeps
   the same answers, and has the same speed. It also names the right
   tool for future climbs. A is the smallest fallback if the library
   should not change.
3. **Then D as the engine's general answer.** A depth measure in
   `logic/front/cycles.rs` lets the `ast_parent` climbs that rule
   authors naturally write pass as written, while staying sound. When it
   lands, the refusal message can say which measure failed, and that
   `ast_up` or a depth climb would pass.
4. **Keep E for typing.** A native `def_qualname` is worth doing when
   approximate typing needs qualified names. It is not needed to unblock
   this rule.

Once `la_cls` passes, the check may stop at a later guard: demand,
stratification or memory. This was not run here, because no builds were
allowed while this note was written. The first run after B should be
the pack on s50 with this rule alone, comparing its violations against
the script's.

## Engine follow-ups

- `front.rs`, `Error::UnboundedValueCycle`'s message: name the relation
  whose guarantee was missing (here `ast_parent`) and the move that broke
  it. Today it names only the rule.
- `cycles.rs` module doc: note that preorder is not a measure for
  `ast_parent`, because of decorators, and say what to use instead.

## Outcome (2026-11-07)

D was built: `front::cycles` now has a depth measure beside preorder.
Each tree edge is an exact depth step, derived relations carry depth
intervals (a least fixpoint, widened after eight rounds), and the key
search tries "shallower" and "deeper" after the preorder ways. `la_cls`
passes as written, with no rule or library change. The tests are in
`front::tests::named_errors`: a climb through a decorator-style edge is
accepted, and a sibling move is still refused.

Running the rule then showed three more problems, now fixed:

- **`.scope` was not applied.** The engine kept `.scope` as settings and
  read nothing from them, so findings came from test files and from
  directories outside the policy's. A finding at a document outside the
  scope is now dropped (`rr::lower`, `scope_filter`). The replay marks
  its events' files as given (`.scope given`), because the scripts scan
  a test's files with no discovery.
- **Two file-local pushes made huge relations.** `within(n, b)` widened
  by n's start was a copy of `within` (2.8 M rows on 200 files). A join
  of a file's identifiers with its imports was a per-file product
  (3.9 M rows). Neither is pushed now (`typing::file_local`).
- **`la_ann_q` crossed files.** It matched every identifier against
  every same-named import of the repository. It now ties the head to its
  document, as its one reader already did.

| files | CPU before | CPU after | findings |
|---|---|---|---|
| 50 | 3.0 s | 2.6 s | 0 |
| 200 | 19.0 s | 9.5 s | 0 |
| 500 | not run | 22.6 s | 1 (LA-000, see below) |
| 1000 | over 110 s | 52.5 s | 1 (LA-000) |

The Python linter reports 0 sites on the full 9,010-file tree. Rerail
gives 0 too, except LA-000 on the slices. LA-000 is right there: the
slices leave out the budget file, `.ratchet-baselines/`. The full tree
was not run to the end: it takes more than the 10-minute cap at about
52 ms a file.

Still open:

- **Speed.** About 52 ms a file at 1,000 files, spread over many small
  strata. The biggest are `fl633_la_native` in SQL (3.4 s, a seven-way
  join that runs in SQLite rather than natively) and the exports to the
  cross part (`fl655_la_scan`, 1.6 s).
- **Memory.** In the 66-rule pack on 200 files, this rule adds about
  300 MB (1,163 MB without it, 1,463 MB with it).
