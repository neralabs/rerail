# 0121: Node order keys by granule run, and where the rest of the order cost lies

Status: decided and built (2026-11-12): §2 (`facts/vtab/syntax/granules.rs`, `order`). §4 is open.
It builds on 0094 §2–2a (node ids per definition, and comparisons of nodes rewritten to read `node_order`).

## Intro

**What `node_order` is.** Node ids are numbered per definition (0094 §2), so an edit inside one function
renumbers that function's nodes and no other's. But ids so numbered are not in document order, and some rules
ask which of two nodes comes first. Those comparisons are rewritten to read an order key instead (0094 §2a):

```text
// The rule as written: a later binding of the same name in the same scope replaces an earlier one.
shadowed(st) :- sbinds(s, x, st), sbinds(s, x, st2), st2 > st, !ovlstmt(st2).

// As the engine runs it: each node's key, compared.
shadowed(st) :- sbinds(s, x, st), sbinds(s, x, st2),
                node_order(st2, k2), node_order(st, k1), k2 > k1, !ovlstmt(st2).
```

**The problem.** Until now the key was the node's preorder index. An edit that adds or removes nodes moves the
index of every node after it in the file, so the edited file's `node_order` rows nearly all change. They are
deleted and inserted again, and every rule that reads `node_order` sees that delta.

## 1. What the delta costs (measured)

On the O125 replay (steps 06–08, the incremental session; work = rows read and written by the joins):

- **One rule reads it.** In the O125 program (`calls.rr` with its includes) `order_nodes` rewrites only
  `shadowed`'s two rules. Its other comparisons (`i < j`, `s <= k`) compare numbers, not nodes.
- **Its share is small.** The `shadowed` strata cost 0.114 / 0.148 / 0.117 M work of the steps'
  0.85 / 3.49 / 6.11 M. That is 13% / 4% / 2%.
- **It is the upper bound.** A probe (`RERAIL_ORDER_RAW=1`, measurement only) compares the ids
  themselves, which no edit outside a granule moves but which are not in order. That cuts the `shadowed`
  strata to 0.000 / 0.034 / 0.004 M. The other derive-agains stay: 2.88 M at step 08. They are priced
  choices fed by the edits' own deltas (`callee`, `pv`, `attr_of`), not by `node_order`.

## 2. The key: a granule run's rank, then the index in the granule

A *run* is a stretch of nodes consecutive in preorder that all belong to one granule. A granule with a nested
definition has two runs, its nodes before the nested one and its nodes after:

```text
def f():              ← run 1: f's own nodes (its header, `a = 1`)
    a = 1
    def g():          ← run 2: granule f/g's nodes
        return a
    return g          ← run 3: f's own nodes again
```

The key's low 32 bits are the run's rank (14 bits), then the node's index among its granule's nodes (18 bits):

```text
k = document << 32 | rank << 18 | index in granule
```

- **In order.** Runs follow one another in preorder, and a granule's nodes are in preorder in it, so the
  keys ascend in preorder.
- **Stable inside a granule.** An edit inside a granule changes neither the runs nor any other granule's
  indexes, so no other node's key moves.
- **Moved by a new granule.** An edit that adds or removes a granule (an import, a function, a module-level
  statement) moves the ranks after it.
- **Content only.** The key is a function of the document's content, like the ids, so a full run and a
  session give the same keys, and the memo's entries (one per content, 0052) stay right.
- **Overflow.** A document with more runs, or a granule with more nodes, than the bits hold falls back to the
  preorder index. There is one scheme per document, so its keys still compare.

Measured over the replay's own edits (`node_order_keys_census`, rows deleted plus inserted):

| | step 06 | step 07 | step 08 |
|---|---|---|---|
| Preorder index (before) | 58,729 | 34,979 | 95,761 |
| Run rank, index in granule (now) | 12,159 | 11,381 | 31,635 |
| Floor: the nodes whose ids change | 247 | 271 | 2,823 |

Files whose edit stays inside its granules reach the floor exactly (e.g. 86 of 86, 4 of 4, 8 of 8). Half the
edited files add or remove a granule, and those keep most of their delta.

**End to end it does not change the cost yet.** The deltas into `shadowed` fell 58k / 35k / 98k →
12k / 11k / 34k rows. But `shadowed` has 77 rows. Any delta past θ (0.25 × 77) has it derived again, and
deriving again (about 9k work) is priced below maintaining it at either size. Every replay step has a file that
adds a granule, so the work stays 0.85 / 3.50 / 6.11 M (same as full: true). The gain is in the common edit of
a working session, typing inside one function, where the delta is the floor.

## 3. Why not labels carried from the last version

An order-maintenance list (labels with gaps, a new node labelled between its neighbours, unchanged nodes keeping
theirs) reaches the floor. But its labels depend on the document's history, not its content:

- **The memo is by content.** Each file's rows are kept by content (0052), and an input's delta is read
  from its old and new entries (0090). The same content reached by two histories would carry two labelings.
- **Content alone cannot do it.** Order keys that are a function of content and also stable under insertion
  cannot exist. A moved definition keeps its content but must change its key. A definition inserted between
  two others must get a key between theirs whatever it contains. Each such fix is information about the
  neighbours, and that is the history this rules out.

## 4. Open: compare where the operands are syntax

What is left is the binary choice in §2: any delta past θ derives `shadowed` again. Its comparison is
syntax: statements of one scope, in order. It sits in the cross-file part only because its operands carry a
cross-file filter. `sbinds` reads `rdefn` and `rasg`, which carry `!py_dead_at`, and whether a branch is dead
can depend on another module's values. So *which* bindings are live builds on other files, while their *order*
does not.

The direction is to form the ordered pairs in the file-local part, over a superset: the bindings with the
cross-file negations dropped. The cross part would then join the pairs with the live bindings:

```text
// File-local, kept per content: the delta is the pairs that changed, not the keys.
rr_later(s, x, st, st2) :- sbinds_syntax(s, x, st), sbinds_syntax(s, x, st2), st2 > st.
// Cross-file.
shadowed(st) :- rr_later(s, x, st, st2), sbinds(s, x, st), sbinds(s, x, st2), !ovlstmt(st2).
```

This belongs in the file-local split (`typing/file_local.rs`, which already pushes `fl563_sbinds`' join into
the chunks), as a general rule for comparisons, not a rewrite of `calls.rr`. It is not built. Its prize on
this replay is at most the 0.11 M a step of §1.
