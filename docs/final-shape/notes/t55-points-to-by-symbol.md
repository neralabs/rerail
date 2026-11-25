# t55. Points-to by symbol: P3's workspace values as symbols, its flows solved as a phase

A design note for decision 0677's family 2 ("cross-file types"): the largest rewrite, and the last library crossing
of fix 3 that holds most of Django's list. It is the design the build follows; each step is recorded in 0677 as it is
built.

## Intro

Points-to (`lib/python/points-to.rerail`, P3 of decision 0083) answers what an expression may evaluate to, and from
that `call_target`, `type_of` and `proven_type`. It has two sorts of value:
- **A library value is already a symbol** (`pt_lv(n, k, s)`). It is resolved through what P1 (`struct`) and P2 (`types`)
  hand on: a module's bindings (`types__binds`), a class's bases and ancestors (`types__anc`), an attribute looked up
  through the bases (`types__attr`), declared signatures and returns (`types__sig`, `types__rtype`).
- **A workspace value is a node** (`pt_pv(n, k, v)`): v is the `function_definition`, `class_definition` or module node
  of any file. Every flow between files carries such a node:
  - a module binding read by symbol (`pt_qv(m, q, k, v)`, through `pt_wres`);
  - an attribute found on a class or a base in another file (`pt_found(c, a, at)`, `pt_base`, `pt_anc`);
  - a call's value, the callee's returns (`pt_fret(f, k, v)`);
  - an instance attribute assigned in another file (`pt_iattr`);
  - a late-bound `self`, the class and each subclass anywhere (`pt_anc`);
  - a decorator's return (`pt_decfn`, `pt_decval`).

So every `pt_` relation the cross part derives holds a node of another file: Django's list holds 74 `pt_` entries.
The interface hands the same on. `call_target(c, f)` pairs a call with a def node of another file. Its readers then
walk the call graph across files by node: python/calls, and Django's sync-ORM-from-async (sora),
select-for-update (sfu), get-handler-writes (ghw) and dispatch-in-atomic (dco). They are most of the rest of Django's
list.

P1 and P2 already name every workspace definition by symbol (`m.C.run`, and `name#k` for a rebinding), and resolve
names, bases and attribute lookup by symbol. What is node-keyed is P3's own inference: what an assignment, a call,
an attribute read or a decorator yields, across files.

## The shape

Three changes, each needed by the next.

**1. A value is a symbol.** A workspace value becomes its definition's symbol as P1 names it: the def or class
`m.C.run`, the module `m`, an instance of class `m.C`. Workspace and library values are then one sort. `pt_pv` and
`pt_lv` become one relation, `pt_pv(n, k, s)`, and the library machinery (`lval`, `lbase`, generics through
`types__tmap`) serves both. The kinds stay as they are: `fn`, `dfn`, `wfn`, `cls`, `inst`, `mod`, `super`.

**2. P3's flows are a phase, `pt`, over terms.** A value that comes from another file cannot be joined at a node in
the cross part. So the flows are solved over symbols in a phase, as `py_names` solves names. Each file states its
expressions' flows as constraints between *terms*. The phase solves the terms' values to its fixed point and hands
them on by module. Each file reads them back at its own nodes.

**Built (2026-11-25), 0677 § Fix 3e: built.** The terms built are positions, not the shapes in the table below: a
definition is its symbol (`m.C.run`, `#k` for a rebinding), and any other node is its place in its nearest scope,
relative to the scope's start, with its length and how many of its ancestors share that span (`m.C.run@14+5^0`).
They come from the tree alone, need no rule per expression form, and change with the same edits, those inside the
scope. The flows were not rewritten as constraints between terms: the old cross-file rules were lifted rule for
rule, each node column made a term. The table is the plan as written.

A term names a value by the expression's shape and its scope's symbol, never by a node:

| Expression | Term |
|---|---|
| a name x read where binding k of scope s is live | `s.x#k` (P1's binding symbol: the same name a reader in another file resolves to) |
| `o.a` | `T(o).a` |
| `e(…)` | `T(e)()`; with the argument terms a returned parameter needs, `T(e)(T(a0), …)` |
| `o[i]`, `for x in o`, `with o as x` | `T(o).__getitem__()`, `….__iter__().__next__()`, `….__enter__()` |
| a literal | its class's instance, `builtins.int` and the rest (`pt_lit_cls`) |
| a lambda or a comprehension | its enclosing scope's symbol, `<lambda>` and its ordinal among that scope's lambdas |

Two expressions of one shape in one scope share a term and hold the same values. P3 is flow-insensitive apart from
binding liveness, and the binding index is in the term. A term is a function of the source's shape, so an edit
that keeps a scope's shape keeps its terms, and an edit of a body changes only that body's terms. That is the memo
behaviour 0677 asks for: a body edit misses its own file.

The constraints, each file's (the phase's per-file part, which may hold nodes):
- `flows(t, u)`: term t holds u's values (an assignment `x = e` makes `s.x#k ⊇ T(e)`, a return makes `f#ret ⊇ T(r)`,
  a conditional or `or` both branches).
- `attr_of(t, o, a)`, `call_of(t, e)`, `arg_of(t, i, a)`, `icall_of(t, o, m)`: the structure of the compound terms.
- `iattr(c, a, u)`: `self.a = e` in a method of class c (`c.a#inst ⊇ T(e)`).
- `dec(f, g)`, `ret(f, r)`, `retparam(f, i)`: decorators and returns, by the definitions' symbols.

The solver, the phase's cross part, over symbols alone: `val(t, k, s)`. Its rules are points-to's own, rewritten
from nodes to terms one family at a time: a name's term takes its binding's values; an attribute's, the class's
binding found through `types__attr`, or an instance attribute through `types__anc`; a call's, an instance of a class
or the callee's `#ret`; `self` and `cls`, the class and each subclass (`types__anc` read the other way); `super()`, the
bases. The unknown values (`vopen`) are a term's openness, `open(t)`, solved the same way.

The export is `val(m, t, k, s)`: term t of module m's file may hold a k s. It is keyed by the module, so a file reads
its own module's rows, and a change misses only the files whose terms' values changed (0665's settings scopes).

**3. The interface by symbol.** Each file reads `pt_pv(n, k, s) :- term(n, t), pt__val(m, t, k, s)` at its own
nodes, where m is its own module. `type_of`, `proven_type` and `possible_type` already answer symbols; they keep their
shape. `call_target(c, f)` becomes `call_target(c, fq)`, the callee's symbol. A reader that needs the def node finds
it by its symbol in its own file (`qualname(f, fq)`).

## The call-graph readers

0677 lets an output keep its node: only the cross-part relations that feed it hold none. So a policy that walks the
call graph walks it over symbols in its own cross part, and places its finding at its own node:
- each file states its defs' calls by symbol: `calls(fq, gq) :- call_target(c, gq), nearest(c, f, fdef),
  qualname(f, fq)`;
- each file states what a def does, by symbol (sora: `orm_call(gq)`, from `dj_qs_call` inside g);
- the closure is symbol-only: `reaches(fq, gq)`, then `does_orm(fq)`;
- the finding reads it at the node: `Violation(f, …) :- async def f, qualname(f, fq), does_orm(fq)`.

None of these needs a phase. The phase is needed for points-to alone, because its values are read by rules
that are not outputs (`call_target`, `type_of`, every rule built on them).

## The order of the work

Each step under tests written first: the old analysis and the new one run in one program over a workspace of
several modules, with the old values mapped to symbols (`pt_cls_q`, `qualname`). They must be equal. A body edit
misses its own file.

1. **Terms and their constraints**, per file: names, attributes, calls, returns, assignments, literals. Test: each
   expression of a fixture has the term the table above gives.
2. **The phase `pt`**: the solver for those, without decorators, generics, `self` or instance attributes. Test:
   module bindings and returns across three modules equal `pt_pv`'s, mapped to symbols.
3. **Classes**: attribute lookup through `types__attr`, instances, `self`/`cls` over subclasses, `super()`, instance
   attributes.
4. **Decorators, wrappers, implicit calls, generics, overloads**: the remaining families, each moved with its tests.
5. **The interface**: `pt_pv` read back, `call_target` by symbol; python/calls and the four Django call-graph
   policies over symbols, as above. The old node-keyed relations go; their 74 entries leave the list.

Library values need no new machinery at any step: they are symbols already, and the solver takes `pt_lv`'s rules
as they are.

## Open questions, settled while building

- **Term size.** A chain `a.b().c[0].d()` makes a term of its whole text. Terms are interned symbols, and a file's
  terms number its value positions (`pt_vpos`), as `pt_pv`'s rows do now. They are measured at step 2 on the
  Django pack and python-main.
- **The phase's cost.** P3's cross part runs every file's flows now. The phase runs the same rules over terms, and
  reading back is one join per value position. Measured at step 2 against the current P3, CPU and wall, on the
  same host. (Measured once built, 0677 § Fix 3e: built: 8% fewer instructions on 143 files of posthog, peak memory
  1.18 to 1.40 GB.)
- **Data-layer's own call graph** (`pc_edge`, `plp_anycand`) is the same shape as the call-graph readers above, and
  is rewritten the same way after step 5.
